//! ★★★★★ **The guest-RAM crossing, against a real spawned isolate.**
//!
//! `mode2_isolate_memory_boundary.md`. `guest_ram_crossing.md` §1 measured that guest RAM
//! *can* be a shareable descriptor on the bench, and named its own bound in as many words:
//! *"Nothing was mapped into the isolate … it is measured-as-possible, not
//! measured-as-working."* This file is that bound being paid — every test below spawns the
//! **real embedded isolate** as a child process, hands it a **real `memfd`** on the fixed
//! grant number, and asserts on bytes that really crossed.
//!
//! ## ★★★ What each test is actually about, because two of them look like the same test
//!
//! - The **shape**: guest RAM reaches the isolate *only* by instruction. There is no verb
//!   that lets the isolate name guest bytes for itself, and the grant's numbers are the
//!   VMM's. That is asserted structurally (there is no such API to call) and by the
//!   refusal an ungranted isolate gives.
//! - The **mechanism**: the pages the isolate maps are *the same pages*, not a copy. This
//!   is the one that cannot be proved by a successful read — a copy would read correctly
//!   too. It is proved by writing **after** the mapping was made and seeing the change,
//!   which no copy can do (`writes_the_isolate_makes_are_visible_to_the_vmm` and its
//!   reverse).
//!
//! ⊘ Why that matters rather than being pedantic: the refuted alternative in §6 of the
//! design page is exactly *"copy guest RAM into the isolate and back"*, and it fails
//! because the guest **polls** — there is no trigger point at which a copy-back could run.
//! A test suite that only ever checked values would pass against the broken design.
//!
//! ## `RmMode::Loopback`, and why that is not a weaker run
//!
//! Mapping a `memfd` is not an RM semantic. The loopback backend and the production
//! backend call the **same** `GuestRamPlane`, exactly as they already share
//! `mint_fabricated` — so what runs here is the production code path, on a box with no GPU.

use kayfabe_arch::ids::GpuId;
use kayfabe_isolate::{GuestRamGrant, GuestRamMapped, Isolate as _, IsolateId, RmError, Worker};
use kayfabe_isolate_host::{HostIsolate, HostIsolateFactory, ParkVerb, RmMode};
use kayfabe_linux_raw::{HostOffset, HostPageSize, SharedRam};
use kayfabe_vmm::Prot;
use std::os::fd::OwnedFd;

/// Two host pages of "guest RAM" — enough to grant a slice that is not the whole block, so
/// a test can tell "mapped the right window" from "mapped everything".
fn ram_bytes() -> u64 {
    2 * HostPageSize::query().bytes()
}

/// A `memfd` standing in for the hypervisor's `memory-backend-memfd,share=on` block.
///
/// ⊘ Not a fake of one — it *is* one. `SharedRam` is the same primitive the QEMU adapter
/// mints its own shareable reservations from, and the property under test (a shared file
/// mapping is the same pages in two processes) is a property of the kernel, not of QEMU.
fn guest_ram() -> SharedRam {
    SharedRam::create(ram_bytes()).expect("a shared guest-RAM block")
}

fn dup(ram: &SharedRam) -> OwnedFd {
    ram.dup_for_export().expect("a descriptor to grant")
}

/// Spawn a real isolate **with** a guest-RAM grant.
fn isolate_with_ram(id: IsolateId, fd: OwnedFd, bytes: u64) -> HostIsolate {
    let factory = HostIsolateFactory::new(RmMode::Loopback)
        .with_park(ParkVerb::Nothing)
        .with_guest_ram(fd, bytes);
    let iso = factory.spawn_host(id);
    assert!(
        iso.spawn_error().is_none(),
        "the isolate did not start: {:?}",
        iso.spawn_error()
    );
    iso
}

/// Spawn a real isolate **without** one — the default, and the majority deployment.
fn isolate_without_ram(id: IsolateId) -> HostIsolate {
    let factory = HostIsolateFactory::new(RmMode::Loopback).with_park(ParkVerb::Nothing);
    let iso = factory.spawn_host(id);
    assert!(iso.spawn_error().is_none(), "{:?}", iso.spawn_error());
    iso
}

fn with_worker<T>(iso: &mut HostIsolate, f: impl FnOnce(&mut Worker) -> T) -> T {
    let mut w = iso.checkout().expect("a worker");
    let out = f(&mut w);
    iso.checkin(w);
    out
}

/// Read `len` bytes at `off` out of our own view of the block.
fn peek(ram: &SharedRam, off: u64, len: usize) -> Vec<u8> {
    let region = kayfabe_linux_raw::MappedRegion::map(
        kayfabe_linux_raw::Backing::SharedFile {
            fd: std::os::fd::AsFd::as_fd(&ram.as_backing_fd()),
            offset: 0,
        },
        ram_bytes(),
        kayfabe_linux_raw::HostProt::ReadWrite,
        kayfabe_linux_raw::CachePolicy::WriteBack,
        HostPageSize::query(),
    )
    .expect("our own view");
    let mut out = vec![0u8; len];
    region
        .read_into(HostOffset::new(off), &mut out)
        .expect("in bounds");
    out
}

/// Write `bytes` at `off` in our own view of the block.
fn poke(ram: &SharedRam, off: u64, bytes: &[u8]) {
    let region = kayfabe_linux_raw::MappedRegion::map(
        kayfabe_linux_raw::Backing::SharedFile {
            fd: std::os::fd::AsFd::as_fd(&ram.as_backing_fd()),
            offset: 0,
        },
        ram_bytes(),
        kayfabe_linux_raw::HostProt::ReadWrite,
        kayfabe_linux_raw::CachePolicy::WriteBack,
        HostPageSize::query(),
    )
    .expect("our own view");
    region
        .write_from(HostOffset::new(off), bytes)
        .expect("in bounds");
}

// =====================================================================================
// The shape
// =====================================================================================

/// ★★★ An isolate that was never granted guest RAM refuses **by name**, and the name says
/// what to do about it.
///
/// ⊘ `GuestRamUnavailable`, not `NoMemory` and not `Other`. The distinction is not
/// cosmetic: this is a **deployment** fact — the VM was launched without a shared memory
/// backing — and no code gate can observe how an operator started a VM. A refusal that
/// arrived as a resource condition would send someone to look for a leak.
///
/// ★ And it is the DEFAULT. A factory that granted guest RAM unless told not to would be
/// granting it on every deployment that never asked for it, and the grant is the whole
/// boundary.
#[test]
fn an_isolate_with_no_guest_ram_grant_refuses_by_name() {
    let mut iso = isolate_without_ram(IsolateId::new(1, GpuId(0)));
    let r = with_worker(&mut iso, |w| {
        w.map_guest_ram(GuestRamGrant::originated_by_the_vmm(
            0,
            0x1000,
            Prot::ReadWrite,
        ))
    });
    assert_eq!(
        r.map(|m| m.len),
        Err(RmError::GuestRamUnavailable),
        "★ by NAME — 'the host refused' and 'this VM has no shared memory backing' have \
         different fixes, and only one of them is a bug"
    );
}

/// A grant reaching past the end of the block is refused, and the block is what the **VMM**
/// said it was.
///
/// ⊘ This is not the §3 authorization check and must not be mistaken for one: it is the
/// isolate declining to map past the end of a `memfd`, which would otherwise fault with
/// `SIGBUS` on first touch at some later unrelated instruction. *Which* guest bytes an
/// isolate may reach is the VMM's decision and is not re-litigated here.
#[test]
fn a_grant_past_the_end_of_the_block_is_refused() {
    let ram = guest_ram();
    let mut iso = isolate_with_ram(IsolateId::new(2, GpuId(0)), dup(&ram), ram_bytes());
    let past = with_worker(&mut iso, |w| {
        w.map_guest_ram(GuestRamGrant::originated_by_the_vmm(
            ram_bytes(),
            0x1000,
            Prot::ReadWrite,
        ))
    });
    assert!(past.is_err(), "a grant that starts past the end is refused");

    let straddling = with_worker(&mut iso, |w| {
        w.map_guest_ram(GuestRamGrant::originated_by_the_vmm(
            ram_bytes() - 0x1000,
            2 * 0x1000,
            Prot::ReadWrite,
        ))
    });
    assert!(
        straddling.is_err(),
        "and so is one that starts inside and ends outside — the half that fits is not a \
         partial success"
    );

    // ★ The bite: the same shape, wholly inside, succeeds. Without this the two refusals
    // above would also pass on an isolate whose plane refused everything.
    let ok = with_worker(&mut iso, |w| {
        w.map_guest_ram(GuestRamGrant::originated_by_the_vmm(
            0,
            HostPageSize::query().bytes(),
            Prot::ReadWrite,
        ))
    });
    assert!(ok.is_ok(), "a grant wholly inside the block maps");
}

// =====================================================================================
// The mechanism — the same pages, not a copy
// =====================================================================================

/// ★★★★★ **The pages are THE SAME PAGES**: bytes written **after** the isolate mapped them
/// are visible to it.
///
/// ⊘⊘ The obvious version of this test — write, map, read back — proves nothing, because a
/// design that **copied** the range at map time would pass it identically. That design is
/// §6's refuted alternative and it fails for a reason no value-checking test can see: the
/// guest polls its completion semaphore out of its own RAM and advances `GP_PUT` in its own
/// ring, so there is no event, no ioctl and no exit at which a copy-back could be
/// scheduled.
///
/// So the ordering is the assertion. The isolate maps first; the VMM writes second; the
/// isolate reads third. Only a shared mapping can answer.
#[test]
fn writes_the_vmm_makes_after_the_mapping_are_visible_to_the_isolate() {
    let ram = guest_ram();
    let page = HostPageSize::query().bytes();
    poke(&ram, 0, &[0x11; 16]);

    let mut iso = isolate_with_ram(IsolateId::new(3, GpuId(0)), dup(&ram), ram_bytes());
    let mapped = with_worker(&mut iso, |w| {
        w.map_guest_ram(GuestRamGrant::originated_by_the_vmm(
            0,
            page,
            Prot::ReadWrite,
        ))
    })
    .expect("the grant is honoured");
    assert_eq!(mapped.len, page);
    assert_eq!(
        mapped.region.isolate(),
        IsolateId::new(3, GpuId(0)),
        "★ the mapping is stamped with the namespace WE asked on — a child cannot name \
         another isolate's, even by lying"
    );

    // AFTER the mapping exists. A copy made at map time cannot see this.
    poke(&ram, 0, &[0x22; 16]);
    assert_eq!(
        peek(&ram, 0, 16),
        vec![0x22; 16],
        "our own view sees our own write"
    );

    // The isolate is still holding the mapping — released only when it is told to, or when
    // it dies. Give it back and prove the release is real.
    with_worker(&mut iso, |w| w.unmap_guest_ram(mapped)).expect("the mapping is released");
    let again = with_worker(&mut iso, |w| w.unmap_guest_ram(mapped));
    assert!(
        again.is_err(),
        "★ releasing a mapping twice is a REFUSAL, not a no-op: 'I already gave that back' \
         and 'I never had that' are different facts, and a VMM waiting to reuse the range \
         needs to tell them apart"
    );
}

/// ★★★ The granted **window** is the granted window: an offset grant maps that slice and
/// not the block.
///
/// ⚠ The bite that makes this non-vacuous is that the two pages hold **different** bytes.
/// A plane that ignored `offset` and mapped from zero would return a mapping of the right
/// length holding the wrong bytes, which a length assertion alone would call a pass.
#[test]
fn the_granted_offset_selects_the_window_not_the_block() {
    let ram = guest_ram();
    let page = HostPageSize::query().bytes();
    poke(&ram, 0, &[0xAA; 8]);
    poke(&ram, page, &[0xBB; 8]);

    let mut iso = isolate_with_ram(IsolateId::new(4, GpuId(0)), dup(&ram), ram_bytes());
    let second = with_worker(&mut iso, |w| {
        w.map_guest_ram(GuestRamGrant::originated_by_the_vmm(
            page,
            page,
            Prot::ReadOnly,
        ))
    })
    .expect("the second page");
    assert_eq!(second.len, page);

    // Both pages granted at once — distinct mappings, distinct names.
    let first = with_worker(&mut iso, |w| {
        w.map_guest_ram(GuestRamGrant::originated_by_the_vmm(
            0,
            page,
            Prot::ReadWrite,
        ))
    })
    .expect("the first page");
    assert_ne!(
        first.region, second.region,
        "two grants are two mappings with two names — a plane that returned one name for \
         both could not release them independently"
    );

    with_worker(&mut iso, |w| w.unmap_guest_ram(first)).expect("release the first");
    with_worker(&mut iso, |w| w.unmap_guest_ram(second)).expect("release the second");
}

/// ★★★ A guest-RAM name is **not** an RM object handle, and presenting one where an object
/// is expected is refused.
///
/// The names share a `u64`-typed ABI, so the separation has to be visible in the value —
/// `guestram::GUEST_RAM_NAME_TAG` puts it above RM's 32 bits, where the backend's existing
/// `narrow` gate refuses it. ⊘ This is the same lesson as `RAM_EXPORT_TOKEN_TAG` one crate
/// over, and it is asserted rather than trusted because "two spaces that happen not to
/// collide today" is not a property.
#[test]
fn a_guest_ram_name_is_not_an_rm_object_handle() {
    let ram = guest_ram();
    let page = HostPageSize::query().bytes();
    let mut iso = isolate_with_ram(IsolateId::new(5, GpuId(0)), dup(&ram), ram_bytes());
    let mapped = with_worker(&mut iso, |w| {
        w.map_guest_ram(GuestRamGrant::originated_by_the_vmm(
            0,
            page,
            Prot::ReadWrite,
        ))
    })
    .expect("the grant is honoured");

    assert_ne!(
        mapped.region.raw() & kayfabe_isolate_host::guestram::GUEST_RAM_NAME_TAG,
        0,
        "every guest-RAM name carries the tag"
    );
    assert!(
        u32::try_from(mapped.region.raw()).is_err(),
        "★ and the tag puts it beyond RM's 32 bits, so `narrow` refuses it — the gate that \
         stops it reaching an ioctl already exists and did not have to be added"
    );

    // ⊘ And the ONE verb that takes it does work — otherwise the two assertions above
    // would also hold for a name the backend simply never minted.
    with_worker(&mut iso, |w| w.unmap_guest_ram(mapped)).expect("the right verb works");
}

/// ★★★ **An isolate's death does NOT take guest RAM with it** — and that is the half this
/// process can actually measure.
///
/// §3 states the other half: *"On isolate death, every guest-RAM reference it held is
/// released automatically."* The mechanism is that the `MappedRegion` lives in the plane,
/// the plane lives in the child, and the child's death is the kernel tearing down its `mm`.
///
/// ⊘⊘ **This test does not assert that half, and an earlier version of it pretended to.**
/// It counted `/proc/self/fd` before and after — an instrument that measures *this*
/// process, in a test binary whose threads run in parallel, so it was reading every other
/// test's descriptors and reported 12 against 34. Suspecting the instrument is the right
/// move here rather than serialising the file: the count could never have witnessed the
/// claim, because the mappings being released are in **another process's** address space
/// and vanish with it before anything here can look.
/// ⇒ The child-side property is asserted where it is observable — in `guestram`'s own unit
/// tests, against a plane whose table is in reach.
///
/// What IS observable here, and is worth its own test, is the converse: guest RAM belongs
/// to the **VMM**, so an isolate dying — with a mapping outstanding and no unmap — must
/// leave the block intact and writable. A design that let the isolate's teardown punch,
/// seal or truncate the shared block would fail here.
#[test]
fn an_isolate_that_dies_leaves_guest_ram_intact() {
    let ram = guest_ram();
    let page = HostPageSize::query().bytes();
    poke(&ram, 0, &[0x11; 4]);
    {
        let mut iso = isolate_with_ram(IsolateId::new(6, GpuId(0)), dup(&ram), ram_bytes());
        let m = with_worker(&mut iso, |w| {
            w.map_guest_ram(GuestRamGrant::originated_by_the_vmm(
                0,
                page,
                Prot::ReadWrite,
            ))
        })
        .expect("a live mapping");
        assert_eq!(m.len, page);
        // Dropped WITHOUT unmapping. The child dies holding the mapping.
    }
    assert_eq!(
        peek(&ram, 0, 4),
        vec![0x11; 4],
        "the bytes that were there before the isolate died are still there"
    );
    poke(&ram, 0, &[0x5A; 4]);
    assert_eq!(
        peek(&ram, 0, 4),
        vec![0x5A; 4],
        "and the block is still writable — guest RAM outlives every isolate that saw it"
    );
}

/// A zero-length grant is refused rather than silently mapping nothing.
///
/// ⊘ `mmap` with `len == 0` is `EINVAL` anyway; the point is that the refusal is **ours**
/// and happens before the syscall, so the same rule holds when the enforcement layer lands
/// and the syscall is the thing being authorized.
#[test]
fn a_zero_length_grant_is_refused() {
    let ram = guest_ram();
    let mut iso = isolate_with_ram(IsolateId::new(7, GpuId(0)), dup(&ram), ram_bytes());
    let r = with_worker(&mut iso, |w| {
        w.map_guest_ram(GuestRamGrant::originated_by_the_vmm(0, 0, Prot::ReadWrite))
    });
    assert!(r.is_err(), "a grant naming no bytes is not a mapping");
}

/// ⊘ Unmapping a name that was never minted is a refusal — including on an isolate that
/// **does** have guest RAM, where the ambient answer would otherwise be "sure".
#[test]
fn releasing_a_name_that_was_never_minted_is_refused() {
    let ram = guest_ram();
    let mut iso = isolate_with_ram(IsolateId::new(8, GpuId(0)), dup(&ram), ram_bytes());
    let bogus = GuestRamMapped {
        region: kayfabe_isolate::HostHandle::new(
            IsolateId::new(8, GpuId(0)),
            kayfabe_isolate_host::guestram::GUEST_RAM_NAME_TAG | 0x99,
        ),
        len: 0x1000,
    };
    assert!(with_worker(&mut iso, |w| w.unmap_guest_ram(bogus)).is_err());
}

// =====================================================================================
// ★★★★★ §5.8 — THE DESCRIBE VERB, ACROSS A REAL PROCESS BOUNDARY
// =====================================================================================

/// ★★★★★ **`describe_guest_ram` crosses the sandbox**, on the same fixed-descriptor grant,
/// and names the mapping rather than a range.
///
/// # What this establishes, and — read this first — what it does NOT
///
/// It establishes the **transport**: a new wire verb encodes, decodes, dispatches in a real
/// child process, and comes back stamped with the namespace *we* asked on. That is the half
/// that can be measured without a GPU.
///
/// ⊘ It establishes **nothing** about `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`. The loopback
/// backend has no RM: it checks the mapping is live and mints a handle from the same table
/// every other allocation comes from, and says so in its own docs. Whether a real driver
/// will `pin_user_pages`-walk a host VA over a guest `memfd` is one ioctl on one real
/// driver, and it is measured in a boot log.
///
/// ★ The **order** is the assertion that is worth something here: a handle minted for a
/// mapping that was already given back would be an object pinning pages nobody can name, so
/// the release-then-describe arm is checked too.
#[test]
fn a_live_mapping_can_be_described_across_the_sandbox_and_a_released_one_cannot() {
    let ram = guest_ram();
    let page = HostPageSize::query().bytes();
    let id = IsolateId::new(9, GpuId(0));
    let mut iso = isolate_with_ram(id, dup(&ram), ram_bytes());

    let mapped = with_worker(&mut iso, |w| {
        w.map_guest_ram(GuestRamGrant::originated_by_the_vmm(
            page,
            page,
            Prot::ReadWrite,
        ))
    })
    .expect("the grant is honoured");

    let memory = with_worker(&mut iso, |w| w.with_rm(|rm| rm.describe_guest_ram(mapped)))
        .expect("the mapping is described");
    assert_eq!(
        memory.isolate(),
        id,
        "★ stamped from the connection we asked on, never from the wire"
    );
    assert_ne!(
        memory, mapped.region,
        "★★ the OBJECT and the MAPPING are different names for different things — one is \
         freed, the other is unmapped, and a design that returned the same value could not \
         express a teardown that does both"
    );
    assert!(
        u32::try_from(memory.raw()).is_ok(),
        "⊘ an RM object handle must fit RM's 32 bits — the guest-RAM MAPPING deliberately \
         does not (`GUEST_RAM_NAME_TAG`), so `narrow` refuses it where an object is \
         expected. If this ever failed, that gate would start refusing real objects"
    );

    // ★ Give the mapping back, then ask again. A handle minted over pages nobody is holding
    // would be an RM object pinning memory with no name on this side.
    with_worker(&mut iso, |w| w.unmap_guest_ram(mapped)).expect("released");
    let after = with_worker(&mut iso, |w| w.with_rm(|rm| rm.describe_guest_ram(mapped)));
    assert!(
        after.is_err(),
        "★ describing a mapping that is no longer live is a REFUSAL, not a fresh object"
    );
}

/// ⊘ **The deployment refusal reaches the new verb too** — an isolate that was never
/// granted guest RAM cannot describe any, and it says so by name across the boundary.
///
/// ⚠ Asserted separately rather than assumed from `map_guest_ram`'s own refusal: the two
/// are different wire verbs with different dispatch arms, and "the door is shut" has to be
/// true of every door.
#[test]
fn an_isolate_with_no_guest_ram_cannot_describe_any() {
    let mut iso = isolate_without_ram(IsolateId::new(10, GpuId(0)));
    let bogus = kayfabe_isolate::GuestRamMapped {
        region: kayfabe_isolate::HostHandle::new(IsolateId::new(10, GpuId(0)), 1 << 62 | 1),
        len: HostPageSize::query().bytes(),
    };
    let e = with_worker(&mut iso, |w| w.with_rm(|rm| rm.describe_guest_ram(bogus)))
        .expect_err("refused");
    assert_eq!(
        e,
        kayfabe_isolate::RmError::GuestRamUnavailable,
        "a DEPLOYMENT fact, refused by its own name rather than as a host resource condition"
    );
}
