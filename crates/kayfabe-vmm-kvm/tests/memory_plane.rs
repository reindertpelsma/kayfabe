//! ★★ The real memory plane, end to end — `l1_os_shell.md` §6.1/§6.2/§6.7, stage M2-c.
//!
//! Every test here drives the **real** thing: a real `/dev/kvm` descriptor, real
//! `KVM_SET_USER_MEMORY_REGION` memslots, real `mmap`ed windows and real `MAP_FIXED`
//! placements. The arms they assert — `EEXIST` for an overlapping guest-physical range,
//! `ENOMEM` for a window the address space cannot hold, a real memslot ceiling — are arms
//! `MockVmm` does not have and cannot be given: its `map_guest` is a `BTreeMap::insert`.
//! That is gap **O1**, and closing it is what this stage is for.
//!
//! The composed mean run lives in `tests/tests/l1_mean.rs`, where the doctrine says it
//! belongs; these are the focused siblings that name one property each.

use core::time::Duration;

use kayfabe_linux_raw::HostPageSize;
use kayfabe_util::lockwitness;
use kayfabe_vmm::{
    BarId, CoreEvent, CoreEventKind, HostRegion, IrqSpec, Prot, TrapMode, Vmm, VmmError,
};
use kayfabe_vmm_kvm::{BarPlacement, KvmMachine, MachineConfig, leaf};

const GPA_RAM: u64 = 0x1000_0000;
const GPA_BAR0: u64 = 0x7000_0000;
const BAR0_LEN: u64 = 0x1_0000;

fn page() -> HostPageSize {
    HostPageSize::query()
}

/// A machine with one BAR declared, as a realized device would have.
fn machine() -> KvmMachine {
    KvmMachine::realize(MachineConfig {
        shareable_ram: true,
        bars: vec![BarPlacement {
            bar: BarId::Bar0,
            base: GPA_BAR0,
            len: BAR0_LEN,
        }],
    })
    .expect(
        "/dev/kvm must be present and permitted for the KVM-direct harness (§10, decision \
         #48) — a deployment fact no code gate can observe, so it refuses loudly here",
    )
}

// =================================================================================
// The plane's ordinary life
// =================================================================================

/// ★ A window is a **memslot plus a mapping**, and a placement inside it is neither.
/// This is §6.7's frequency rule as an observation rather than a rule: ten publications
/// into one window perform **one** memslot install.
#[test]
fn ten_publications_into_one_window_perform_exactly_one_memslot_install() {
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    let region = m
        .install_ram_window(GPA_RAM, 16 * p.bytes())
        .expect("a 16-page window installs");

    let backing = m.register_backing(p.bytes()).expect("a host backing");
    let mut slots = Vec::new();
    for i in 0..10u64 {
        slots.push(
            v.map_guest(GPA_RAM + i * p.bytes(), p.bytes(), backing, Prot::ReadWrite)
                .expect("a publication into an installed window"),
        );
    }

    let a = m.audit();
    assert_eq!(
        (a.memslot_installs, a.placements_made),
        (1, 10),
        "★ THE MEMSLOT-FREQUENCY GATE (§9.3): installs must scale with WINDOWS and \
         placements with PUBLICATIONS. A per-object memslot would read (11, 10) here — \
         and that is the C artifact's measured regression (>1500 slots for one \
         cuCtxCreate), caught structurally and without a clock"
    );
    assert_eq!(
        (a.live_windows, a.live_memslots, a.live_placements),
        (1, 1, 10)
    );

    for s in slots {
        v.unmap_guest(s).expect("restore");
    }
    m.remove_window(region).expect("remove");
    let a = m.audit();
    assert_eq!(
        (
            a.live_windows,
            a.live_memslots,
            a.live_placements,
            a.window_bytes
        ),
        (0, 0, 0, 0),
        "the conservation ledger must balance after teardown"
    );
    assert_eq!(
        (a.peak_windows, a.peak_memslots, a.peak_placements),
        (1, 1, 10),
        "★ NON-VACUITY: the ledger's peaks prove it ever counted anything. Without this, \
         a ledger that was never incremented balances perfectly"
    );
}

/// ★ Guest memory that really is memory: written through the port, read back through the
/// port, at an offset deep inside the window.
#[test]
fn guest_memory_round_trips_through_a_real_mapping() {
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    m.install_ram_window(GPA_RAM, 8 * p.bytes())
        .expect("window");

    let payload: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
    // Deep inside the window, deliberately unaligned: a copy is a copy, and only the
    // MAPPING geometry is page-granular.
    let at = GPA_RAM + 3 * p.bytes() + 8;
    v.gpa_write(at, &payload).expect("write");
    let mut got = vec![0u8; payload.len()];
    v.gpa_read(at, &mut got).expect("read");
    assert_eq!(got, payload, "a real mapping must round-trip its bytes");

    // The whole window is readable before anything was published into it — a live memslot
    // names the range whether or not we filled it.
    let mut zeroes = [0xFFu8; 32];
    v.gpa_read(GPA_RAM, &mut zeroes).expect("read the head");
    assert_eq!(zeroes, [0u8; 32]);
}

// =================================================================================
// ★★ The region map versus the kernel — a consistency the mock cannot express
// =================================================================================

/// ★★★ On KVM a **device region IS the absence of a memslot**. So the guest-steerable
/// hazard (§10.1 item 6) is not modelled here, it is physical: an access to the BAR has
/// nowhere to land, and the port refuses it by the name that says so.
#[test]
fn a_device_region_is_the_absence_of_a_memslot_and_refuses_by_name() {
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    m.install_ram_window(GPA_RAM, 2 * p.bytes())
        .expect("window");

    let mut buf = [0u8; 16];
    assert_eq!(
        v.gpa_read(GPA_BAR0, &mut buf),
        Err(VmmError::NonRamGpa { gpa: GPA_BAR0 }),
        "our own trapped BAR is a device window: serving it would DMA into our own MMIO \
         dispatch from inside a locked section"
    );
    assert_eq!(
        v.gpa_write(GPA_BAR0 + 0x800, &[1, 2, 3, 4]),
        Err(VmmError::NonRamGpa {
            gpa: GPA_BAR0 + 0x800
        }),
        "and the write direction is the sharper one — a stray write into a device \
         register window is a side effect on hardware"
    );
    assert_eq!(
        v.gpa_read(0xDEAD_0000, &mut buf),
        Err(VmmError::BadGpa { gpa: 0xDEAD_0000 }),
        "a hole is the NEAR NEIGHBOUR and must never start reporting as a device"
    );
    // The straddling shape: starts in real RAM, runs into nothing.
    assert_eq!(
        v.gpa_read(GPA_RAM + 2 * p.bytes() - 8, &mut buf),
        Err(VmmError::BadGpa {
            gpa: GPA_RAM + 2 * p.bytes()
        }),
        "a range that starts in RAM and leaves its region must report the BOUNDARY byte, \
         not its own start — a start-address-only check is not a check"
    );

    assert_eq!(
        m.assert_map_matches_the_kernel(),
        1,
        "★ NON-VACUITY: the map/kernel consistency check must have had a region to check"
    );
}

/// ★ The window's teardown makes the range stop resolving **before** the mapping goes
/// away, so a reader either resolves and completes or refuses. Never a stale byte.
#[test]
fn a_removed_window_refuses_as_unbacked_and_never_as_stale_bytes() {
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    let region = m.install_ram_window(GPA_RAM, p.bytes()).expect("window");
    v.gpa_write(GPA_RAM, &[0xA5; 8]).expect("write");
    let mut got = [0u8; 8];
    v.gpa_read(GPA_RAM, &mut got).expect("read");
    assert_eq!(got, [0xA5; 8], "served before the teardown");

    m.remove_window(region).expect("remove");
    assert_eq!(
        v.gpa_read(GPA_RAM, &mut got),
        Err(VmmError::BadGpa { gpa: GPA_RAM }),
        "a torn-down window must refuse as UNBACKED — not as a device window, and never \
         as the bytes that used to be there"
    );
    // ★ And it was UNDECLARED, not merely disconnected. A bite-check found these two
    // states indistinguishable through `gpa_read` — both arrive as `BadGpa` — so the
    // region map is asserted directly. A stale RAM declaration is a guest-physical range
    // the map promises is memory and the kernel has no memslot for.
    assert_eq!(
        m.resolve_region(GPA_RAM, 8),
        Err(VmmError::BadGpa { gpa: GPA_RAM }),
        "the REGION was undeclared, not just its window forgotten"
    );
    assert_eq!(
        m.remove_window(region),
        Err(VmmError::BadSlot(kayfabe_vmm::SlotId(region.0))),
        "and removing it twice is a named refusal, not a second DELETE"
    );
}

// =================================================================================
// ★★ REAL SYSCALL FAILURES — the arms a mock does not have
// =================================================================================

/// ★★ `EEXIST` from the kernel: two windows may not claim one guest-physical range. The
/// adapter does not know this rule and does not need to — the flat view is the kernel's.
#[test]
fn an_overlapping_window_is_the_kernels_eexist_and_leaks_nothing() {
    let m = machine();
    let p = page();
    let first = m
        .install_ram_window(GPA_RAM, 4 * p.bytes())
        .expect("the first window");
    let before = m.audit();

    assert_eq!(
        m.install_ram_window(GPA_RAM + 2 * p.bytes(), 4 * p.bytes()),
        Err(VmmError::HostRefused {
            what: "a memslot install",
            errno: Some(libc::EEXIST),
        }),
        "the exact errno, not `is_err()`: EEXIST (an overlap we should have known about) \
         and EINVAL (an exhausted ceiling) are operationally different and must never \
         start reporting as each other"
    );

    let after = m.audit();
    assert_eq!(
        (
            after.live_windows,
            after.live_memslots,
            after.window_bytes,
            after.host_refusals
        ),
        (
            before.live_windows,
            before.live_memslots,
            before.window_bytes,
            before.host_refusals + 1
        ),
        "★ PARTIAL FAILURE LEAKS NOTHING: the mmap succeeded and the ioctl did not, so \
         the window must have been unmapped and nothing recorded — and the refusal must \
         have been COUNTED, or this assertion is about an operation that never ran"
    );
    // Proof the address space really was returned: the same GPA installs once the first
    // window is gone.
    m.remove_window(first).expect("remove");
    m.install_ram_window(GPA_RAM + 2 * p.bytes(), 4 * p.bytes())
        .expect("the range is free again");
    assert_eq!(m.assert_map_matches_the_kernel(), 1);
}

/// ★★ **The host address space is returned too, not merely the bookkeeping.**
///
/// This test exists because a bite-check found the previous one insufficient: leaking the
/// `mmap`ed window on the memslot-install failure path moved **no counter**, because the
/// ledger is only incremented on success. So "partial failure leaks nothing" was a claim
/// about the ledger and the guest-physical flat view, and said nothing about the process's
/// own address space — which is the resource that actually runs out.
///
/// `VmSize` from `/proc/self/status` is the instrument. Each refused install `mmap`s a
/// 1 MiB window before the kernel refuses the memslot; a hundred of them is 100 MiB, two
/// orders of magnitude above the noise of an allocator warming up.
#[test]
fn a_repeated_partial_failure_returns_the_host_address_space() {
    let m = machine();
    let p = page();
    let win = 256 * p.bytes();
    m.install_ram_window(GPA_RAM, win)
        .expect("the first window");
    // Warm up, so the baseline is not measuring the first refusal's own allocations.
    for _ in 0..4 {
        assert!(m.install_ram_window(GPA_RAM + 8 * p.bytes(), win).is_err());
    }
    let before = vm_size_kib();
    for _ in 0..100 {
        assert_eq!(
            m.install_ram_window(GPA_RAM + 8 * p.bytes(), win),
            Err(VmmError::HostRefused {
                what: "a memslot install",
                errno: Some(libc::EEXIST),
            })
        );
    }
    let after = vm_size_kib();
    let leaked = after.saturating_sub(before);
    assert!(
        leaked < 8 * 1024,
        "★ a hundred refused installs grew this process's address space by {leaked} KiB. \
         Each one maps a {win_kib} KiB window BEFORE the kernel refuses the memslot; if \
         the failure path forgets to release it, the leak is ~{expect} KiB and the \
         conservation ledger — which is only incremented on SUCCESS — says nothing at all",
        win_kib = win / 1024,
        expect = 100 * win / 1024,
    );
    assert_eq!(
        m.audit().host_refusals,
        104,
        "★ NON-VACUITY: every attempt really was refused by the host"
    );
}

/// `VmSize` in KiB, from `/proc/self/status`.
fn vm_size_kib() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").expect("procfs");
    s.lines()
        .find_map(|l| l.strip_prefix("VmSize:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .expect("VmSize is always present on Linux")
}

/// ★ `ENOMEM` from the kernel: a window the address space cannot hold. The `mmap` fails
/// **before** any memslot exists, which is the other partial-failure shape.
#[test]
fn a_window_the_address_space_cannot_hold_is_the_kernels_enomem() {
    let m = machine();
    let p = page();
    let absurd = (1u64 << 62) & !p.mask();
    assert_eq!(
        m.install_ram_window(GPA_RAM, absurd),
        Err(VmmError::HostRefused {
            what: "a window mapping",
            errno: Some(libc::ENOMEM),
        })
    );
    let a = m.audit();
    assert_eq!(
        (a.live_windows, a.live_memslots, a.window_bytes),
        (0, 0, 0),
        "an mmap that failed must leave no window and no declaration"
    );
    assert_eq!(a.host_refusals, 1, "★ NON-VACUITY: the refusal was counted");
}

/// ★ The **memslot ceiling** is a real, kernel-imposed number, and §6.7's frequency rule
/// is what keeps a data plane away from it. The adapter refuses before the kernel does,
/// by a name that says which resource ran out.
#[test]
fn the_memslot_ceiling_is_a_real_number_and_the_refusal_names_it() {
    let m = machine();
    let ceiling = m.memslot_ceiling();
    assert!(
        ceiling >= 32,
        "the kernel reported {ceiling} memslots, which is not a real ceiling — the \
         capability query is not reaching KVM"
    );
    let p = page();
    // Walk the adapter's own slot counter up to the ceiling without paying for `ceiling`
    // mmaps: install and remove, which consumes slot NUMBERS (they are deliberately not
    // recycled — a recycled slot number is indistinguishable from a stale one in a
    // kernel log).
    for i in 0..ceiling {
        let r = m
            .install_ram_window(GPA_RAM + u64::from(i) * p.bytes() * 2, p.bytes())
            .expect("a window below the ceiling");
        m.remove_window(r).expect("remove");
    }
    assert_eq!(
        m.install_ram_window(0x9000_0000, p.bytes()),
        Err(VmmError::Unsupported(
            "the kernel's memslot ceiling — §6.7's frequency rule is what keeps a \
                     data plane away from it"
        ))
    );
    assert_eq!(
        m.audit().memslot_installs,
        u64::from(ceiling),
        "★ NON-VACUITY: exactly `ceiling` installs really happened before the refusal"
    );
}

/// ★ Alignment and emptiness are refused **before** the syscall, by name.
#[test]
fn a_misaligned_or_empty_window_is_refused_before_any_syscall() {
    let m = machine();
    let p = page();
    let expected = Err(VmmError::Unsupported(
        "a window whose base or length is not a whole number of host pages",
    ));
    assert_eq!(m.install_ram_window(GPA_RAM + 8, p.bytes()), expected);
    assert_eq!(m.install_ram_window(GPA_RAM, p.bytes() + 1), expected);
    assert_eq!(m.install_ram_window(GPA_RAM, 0), expected);
    assert_eq!(
        m.audit().memslot_installs,
        0,
        "not one ioctl may have been issued"
    );
}

// =================================================================================
// ★★★ R1 — the invariant this stage exists to test
// =================================================================================

/// ★★★ **The R1 witness.** Every syscall-shaped method must run with ZERO ranked locks,
/// and the in-lock-legal accessors must really have been entered WITH one — otherwise the
/// suite is green about a path nobody took.
///
/// Both halves are load-bearing, and the second is the one a naive version omits.
#[test]
fn no_syscall_shaped_method_ever_runs_with_a_ranked_lock_held() {
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    let backing = m.register_backing(p.bytes()).expect("backing");

    // --- syscalls, all lock-free (the ordinary path)
    let region = m
        .install_ram_window(GPA_RAM, 4 * p.bytes())
        .expect("window");
    let slot = v
        .map_guest(GPA_RAM, p.bytes(), backing, Prot::ReadWrite)
        .expect("place");
    v.export_ram(None).expect("export");
    v.unmap_guest(slot).expect("restore");

    // --- an in-lock-legal accessor, entered WITH a ranked lock, exactly as
    //     `SharedDevice::parse_pushbuffer`'s route phase enters it.
    lockwitness::note_acquired(0);
    let mut buf = [0u8; 64];
    let in_lock = v.gpa_read(GPA_RAM, &mut buf);
    lockwitness::note_released(0);
    assert_eq!(
        in_lock,
        Ok(()),
        "gpa_read is in-lock LEGAL and must be served"
    );

    // --- and one lock-free, so the span has two ends.
    v.gpa_read(GPA_RAM, &mut buf).expect("lock-free read");

    let a = m.audit();
    assert_eq!(
        a.syscall_ranked_depth,
        (0, 0),
        "★ R1: every syscall-shaped Vmm method ran with zero ranked locks held. A `min` \
         of u32::MAX would mean NO syscall ran at all, which is why the pair is asserted \
         and not just the max"
    );
    assert_eq!(
        a.accessor_ranked_depth,
        (0, 1),
        "★ and the in-lock-legal accessor really was entered with a ranked lock held \
         (max 1) AND lock-free (min 0). Without both ends this whole file could be green \
         about a path the harness never took"
    );
    assert_eq!(
        a.copy_leaf_depth_max, 0,
        "★★ THE ADAPTER HALF OF R1 (§12.43 residual 2): the memcpy runs OUTSIDE the view \
         lock, held alive by an Arc. A copy under the lock would serialise every \
         guest-memory read in the machine against every other"
    );
    assert!(
        a.view_leaf_depth_max >= 1,
        "★ NON-VACUITY for the line above: the view lock must actually have been taken. \
         With no lock at all, `copy_leaf_depth_max == 0` is true and means nothing"
    );
    m.remove_window(region).expect("remove");
}

/// ★ **The witnesses' own non-vacuity, asserted directly.** A machine on which nothing has
/// happened must report *"never observed"* and not *"observed zero"* — otherwise every
/// span assertion in this file has a lower bound that equals its own success value, which
/// is the exact defect `l1_concurrency.md` §12.43's N12 found (`Arc<AtomicU32>::default()`
/// is `0`). A bite-check on the hand-written `Default` did not bite until this test
/// existed.
#[test]
fn a_fresh_machine_reports_never_observed_and_not_observed_zero() {
    let m = machine();
    let a = m.audit();
    assert_eq!(
        (a.accessor_ranked_depth, a.syscall_ranked_depth),
        ((u32::MAX, 0), (u32::MAX, 0)),
        "★ the minima must start at u32::MAX. With a derived Default they start at 0, and \
         `syscall_ranked_depth == (0, 0)` — R1's headline assertion — becomes true of a \
         machine that never made a syscall at all"
    );
    assert_eq!(
        (a.copy_leaf_depth_max, a.view_leaf_depth_max),
        (0, 0),
        "and the leaf witnesses start at zero, so `>= 1` really is an observation"
    );
}

/// ★★ The adapter's own witness, in the polarity that matters: a syscall attempted while
/// the adapter holds one of its own locks must be **loud**, even though no ranked lock is
/// held and `lockwitness` is therefore silent.
#[test]
fn a_syscall_under_the_adapters_own_lock_is_loud_even_though_no_rank_is_held() {
    let m = machine();
    let p = page();
    let held = leaf::Held::enter();
    assert_eq!(
        lockwitness::held_depth(),
        0,
        "the RANKED witness sees nothing here — which is exactly the blind spot"
    );
    let boom = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        m.install_ram_window(GPA_RAM, p.bytes())
    }));
    drop(held);
    let err = boom.expect_err("a syscall under an adapter lock must panic");
    let msg = err
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_else(|| (*err.downcast_ref::<&str>().unwrap_or(&"")).to_string());
    assert!(
        msg.contains("R1 violation, adapter half"),
        "the panic must name R1's adapter half, not merely fail: {msg}"
    );
}

// =================================================================================
// The remaining capability groups
// =================================================================================

/// ★ §6.7 item 4: KVM's read-only flag is a **slot** property, so per-object read-only
/// protection inside a shared read-write window is refused rather than approximated.
#[test]
fn per_object_read_only_protection_is_refused_because_protection_is_a_window_property() {
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    m.install_ram_window(GPA_RAM, 2 * p.bytes())
        .expect("window");
    let backing = m.register_backing(p.bytes()).expect("backing");
    assert_eq!(
        v.map_guest(GPA_RAM, p.bytes(), backing, Prot::ReadOnly),
        Err(VmmError::Unsupported(
            "per-object read-only protection inside a read-write window — \
                     protection is a WINDOW property (§6.7 item 4); place the object in \
                     a read-only window instead"
        )),
        "minting a slot to make one page read-only is a DELETE+ADD (two grace periods) \
         to protect one object, and it revokes access for every proc sharing the window"
    );
    assert_eq!(
        m.audit().placements_made,
        0,
        "and nothing was placed on the way to the refusal"
    );
}

/// ★ The read-native overlay is a **read-only memslot** — the rom-device pattern, not an
/// emulation of it — and the write-trap sub-range is rounded outward to whole host pages.
#[test]
fn a_read_native_overlay_installs_a_read_only_slot_over_the_rounded_write_trap() {
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    let backing = m.register_backing(4 * p.bytes()).expect("backing");
    // Trap one byte in the middle: the slot must widen to the whole host page containing
    // it, and split the window into three slots.
    let trap = (GPA_RAM + p.bytes() + 8)..(GPA_RAM + p.bytes() + 9);
    v.map_read_native(GPA_RAM, 4 * p.bytes(), backing, Some(trap))
        .expect("a read-native overlay installs");
    let a = m.audit();
    assert_eq!(
        (a.live_windows, a.live_memslots),
        (1, 3),
        "one window, three slots: [head RW][the trapped page RO][tail RW]. A single-byte \
         request became a whole HOST page, which on an arm64 host is 16 or 64 KiB — \
         correct, and quietly slower"
    );
    // Reads are still served natively through our own accessor.
    let mut got = [0xFFu8; 8];
    v.gpa_read(GPA_RAM + p.bytes(), &mut got)
        .expect("a read-native range is RAM to us as well");
    assert_eq!(got, [0u8; 8]);

    assert_eq!(
        v.map_read_native(GPA_RAM + 8 * p.bytes(), p.bytes(), backing, Some(0..1)),
        Err(VmmError::Unsupported(
            "a write-trap sub-range that is not inside the range it overlays"
        )),
        "a trap outside the range it overlays is a refusal, not a silent clamp"
    );
}

/// ★ A trap over a range a live memslot serves is refused: the guest's access would never
/// leave the guest, so the trap would exist only in our bookkeeping.
#[test]
fn a_read_write_trap_over_a_live_memslot_is_refused() {
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    assert_eq!(
        v.set_trap(BarId::Bar0, 0..p.bytes(), TrapMode::ReadWrite),
        Ok(()),
        "the BAR has no memslot, so a trap over it is exactly right"
    );
    // Now put RAM under part of the BAR — the shape a guest produces by reprogramming a
    // BAR over RAM — and the trap must be refused.
    m.install_ram_window(GPA_BAR0, p.bytes())
        .expect("a window over the BAR's base");
    assert_eq!(
        v.set_trap(BarId::Bar0, 0..p.bytes(), TrapMode::ReadWrite),
        Err(VmmError::Unsupported(
            "a read-write trap over a range a live memslot already serves — \
                         the guest's access would never leave the guest"
        ))
    );
    assert_eq!(
        v.set_trap(BarId::Bar1, 0..p.bytes(), TrapMode::ReadWrite),
        Err(VmmError::Unsupported(
            "a BAR this machine was not realized with"
        ))
    );
    assert_eq!(
        v.set_trap(BarId::Bar0, 0..(BAR0_LEN + 1), TrapMode::ReadWrite),
        Err(VmmError::Unsupported("a trap range outside its BAR"))
    );
}

/// ★ §4.4.1's deployment fact: a VM created without a shareable backing refuses the
/// **first** export, loudly, rather than handing back copy-on-write pages that would make
/// an isolate's completions invisible to the guest.
#[test]
fn a_machine_without_a_shareable_backing_refuses_the_first_export() {
    let m = KvmMachine::realize(MachineConfig {
        shareable_ram: false,
        bars: Vec::new(),
    })
    .expect("realize");
    let p = page();
    let mut v = m.vmm();
    m.install_ram_window(GPA_RAM, p.bytes()).expect("window");
    // Everything else still works — which is exactly why the refusal has to be loud.
    v.gpa_write(GPA_RAM, &[7; 4]).expect("guest memory works");
    assert_eq!(
        v.export_ram(None),
        Err(VmmError::Unsupported(
            "guest RAM was not created with a shareable backing; an isolate cannot map it"
        ))
    );

    let shared = machine();
    shared
        .install_ram_window(GPA_RAM, p.bytes())
        .expect("window");
    let h = shared.vmm().export_ram(None).expect("a shareable export");
    assert_eq!(h.covers, None);
    assert_eq!(
        shared
            .vmm()
            .export_ram(Some(GPA_RAM..(GPA_RAM + p.bytes()))),
        Ok(kayfabe_vmm::RamHandle {
            token: 1,
            covers: Some(GPA_RAM..(GPA_RAM + p.bytes()))
        }),
        "a per-slice export is a distinct handle — least-privilege sharing (§4.3.4)"
    );
    assert_eq!(
        shared.vmm().export_ram(Some(0xDEAD_0000..0xDEAD_1000)),
        Err(VmmError::Unsupported(
            "no shareable window covers the requested slice"
        ))
    );
}

/// ★ `raise_irq` is the one in-lock-legal syscall, and it really is one: a write to a
/// real notify descriptor, permitted under the ranks it declares.
#[test]
fn raise_irq_is_a_real_descriptor_write_and_is_legal_under_the_ranks_it_declares() {
    let m = machine();
    let mut v = m.vmm();
    lockwitness::note_acquired(1);
    let under_lock = v.raise_irq(IrqSpec::Msix(0));
    lockwitness::note_released(1);
    assert_eq!(
        under_lock,
        Ok(()),
        "§6.1's single named exception — an irqfd-shaped edge under the proc lock"
    );
    v.raise_irq(IrqSpec::Msix(0)).expect("and lock-free");
    assert_eq!(
        m.drain_irqs(),
        Ok(2),
        "two real edges accumulated on a real descriptor — a counter, not a Vec of enums"
    );
    assert_eq!(
        v.raise_irq(IrqSpec::IntxLevel(true)),
        Err(VmmError::Unsupported(
            "legacy INTx needs an interrupt controller this harness does not create"
        )),
        "backend-conditional by the trait's own rustdoc: a REFUSAL, never a silently \
         dropped injection"
    );
}

/// ★ The deadline queue is the port's, shared with the mock (§6.4) — so two backends
/// cannot drift on timer ordering.
#[test]
fn deferred_events_come_back_in_deadline_then_insertion_order() {
    let m = machine();
    let mut v = m.vmm();
    v.defer(
        Duration::from_millis(2),
        CoreEvent::Deferred(CoreEventKind::DeferredReap),
    );
    v.defer(
        Duration::from_millis(1),
        CoreEvent::Deferred(CoreEventKind::PollKickBudget),
    );
    v.defer(
        Duration::from_millis(1),
        CoreEvent::Deferred(CoreEventKind::CompletionRedeliver),
    );
    assert_eq!(m.advance(Duration::from_micros(500)), vec![]);
    assert_eq!(
        m.advance(Duration::from_micros(500)),
        vec![
            CoreEvent::Deferred(CoreEventKind::PollKickBudget),
            CoreEvent::Deferred(CoreEventKind::CompletionRedeliver),
        ],
        "same deadline: INSERTION order, not the payload's own Ord — otherwise delivery \
         order would depend on the event value, deterministically and wrongly"
    );
    assert_eq!(
        m.advance(Duration::from_millis(1)),
        vec![CoreEvent::Deferred(CoreEventKind::DeferredReap)]
    );
    assert_eq!(
        v.now(),
        kayfabe_util::Instant::ZERO.advanced(Duration::from_millis(2))
    );
}

/// ★ A publication naming a backing this backend never minted is refused, not served
/// from whatever happened to be at that index.
#[test]
fn a_backing_id_this_backend_never_minted_is_refused() {
    let m = machine();
    let p = page();
    let mut v = m.vmm();
    m.install_ram_window(GPA_RAM, p.bytes()).expect("window");
    assert_eq!(
        v.map_guest(
            GPA_RAM,
            p.bytes(),
            HostRegion { id: 999, offset: 0 },
            Prot::ReadWrite
        ),
        Err(VmmError::Unsupported(
            "a host backing id this backend never minted"
        ))
    );
    assert_eq!(
        v.map_guest(
            0x5000_0000,
            p.bytes(),
            m.register_backing(p.bytes()).expect("backing"),
            Prot::ReadWrite
        ),
        Err(VmmError::BadGpa { gpa: 0x5000_0000 }),
        "and a publication outside every installed window is a BadGpa — the fine tier \
         cannot conjure a window, which is §6.7's rule stated as a refusal"
    );
}
