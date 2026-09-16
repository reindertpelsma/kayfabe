//! ★★★★★ **w752 CUT P1 — THE DOOR-ELIMINATION GATE, OFFLINE.**
//!
//! `[measured w742, device arm, one binary, both arms]`:
//!
//! ```text
//! VCPU-BLOCKING total=197 doors=9   worst_trap=44440us   cpu_of_that_trap=43390us
//!   [40 x receiving a descriptor across the isolate boundary]
//!   [20 x KVM_SET_USER_MEMORY_REGION (installing a memslot)]      <- door 6
//!   [20 x classifying a received descriptor]
//!   [20 x exporting a host device view to the VMM]
//!   [20 x mmap (creating a guest-physical window)]                <- door 4
//!   [20 x mmap MAP_FIXED (placing an armed device node)]
//!   [19 x KVM_SET_USER_MEMORY_REGION (dropping a memslot)]        <- door 7
//!   [19 x munmap (dropping a guest-physical window)]              <- door 8
//!   [19 x releasing a host device view]                           <- door 9
//! ```
//!
//! `20 x 7 + 19 x 3 = 197`, and **every one of them is one PRAMIN window move**. Doors 4, 6, 7
//! and 8 exist only because the move was release-and-re-arm, which `l1_os_shell.md` §6.7 rule 3
//! and constraint 16 both forbid outright: *"No slot delete/recreate to change a mapping … Re-
//! `MAP_FIXED` the window's backing instead."*
//!
//! # ⊘ What this binary can and cannot prove
//!
//! It runs against the **mock** memslot plane and a **memfd** in place of a `/dev/nvidia<N>`
//! node — the project's runner has no KVM and no GPU. So it proves the part that is decidable
//! without either, and that part is the whole of the claim being made here:
//!
//! - a re-point installs **no** memslot, clears **none**, and replaces **none**;
//! - the window shows the new backing afterwards, and the old one is unreachable through it;
//! - a re-point whose length is not the window's is **refused by name**, with the window left
//!   showing what it showed — never half.
//!
//! ⚠ What it cannot see: the `VM_IO | VM_PFNMAP` VMA a real node produces, and whether KVM
//! retires its EPT entries on the replacement. That half is the bench's, and the boot's
//! `PRAMIN-INPLACE` census line is where it is read. ⊘ `[w582-w586]`: *zero traps proves the
//! slot INTERCEPTS the access, never that it shows the same bytes.*

mod common;

use kayfabe_linux_raw::{HostOffset, SharedRam};
use kayfabe_vmm_qemu::REPOINT_LENGTH_IS_NOT_THE_WINDOWS;
use kayfabe_vmm::{RamRegionId, VmmError};

/// Where the aperture lives: inside BAR1, clear of the realize-time reservation.
fn pramin_gpa() -> u64 {
    common::BAR1_BASE + 128 * common::page()
}
fn pramin_len() -> u64 {
    4 * common::page()
}

/// ★★★★★ **THE CUT, AS A COUNT.** Twenty moves must cost the kernel's memslot plane
/// **nothing at all** beyond the one install that created the aperture.
#[test]
fn re_pointing_the_aperture_installs_no_memslot_clears_none_and_replaces_none() {
    let (m, _host, slots) = common::machine();
    let first = SharedRam::create(pramin_len()).expect("the first node");

    let region = m
        .install_device_page(pramin_gpa(), pramin_len(), first.as_backing_fd(), false)
        .expect("the aperture installs on the guest's first latch write");

    // ⊘ The baseline is taken AFTER the install, on purpose: the one-time install is not part
    // of what this test is about, and folding it in would hide a regression of exactly its size.
    let (i0, c0, r0) = (slots.installs(), slots.clears(), slots.replaces());
    assert!(i0 > 0, "the install must have installed something, or the deltas below are vacuous");

    let nodes: Vec<SharedRam> = (0..20)
        .map(|_| SharedRam::create(pramin_len()).expect("a node"))
        .collect();
    for n in &nodes {
        m.repoint_device_window(region, n.as_backing_fd(), pramin_len(), true)
            .expect("an in-place re-point");
    }

    assert_eq!(
        (slots.installs() - i0, slots.clears() - c0, slots.replaces() - r0),
        (0, 0, 0),
        "twenty in-place re-points asked the memslot plane for {} install(s), {} clear(s) and \
         {} replace(s). The correct number is ZERO for all three: the guest-physical range is \
         where it was and the host virtual range is where it was, so nothing the hypervisor \
         knows has moved. A non-zero here is the w742 shape returning — 20 installs and 19 \
         deletes on vCPU threads, each delete two `synchronize_srcu_expedited` plus a shadow \
         zap charged to EVERY vCPU.",
        slots.installs() - i0,
        slots.clears() - c0,
        slots.replaces() - r0,
    );
}

/// ★★★★★ **THE SENTINEL, at the verb the mirror actually calls.** Write a known value at A,
/// re-point to B, and prove the window shows B and that A is gone from it.
#[test]
fn the_aperture_shows_the_node_it_was_re_pointed_to_and_not_the_one_before_it() {
    let (m, _host, _slots) = common::machine();
    let a = SharedRam::create(pramin_len()).expect("node A");
    let b = SharedRam::create(pramin_len()).expect("node B");

    let region = m
        .install_device_page(pramin_gpa(), pramin_len(), a.as_backing_fd(), false)
        .expect("install over A");
    let w = m.device_window_handle(region).expect("the window's handle");

    w.write_from(HostOffset::ZERO, b"NODE-A!!").expect("write through the aperture");

    m.repoint_device_window(region, b.as_backing_fd(), pramin_len(), true)
        .expect("re-point to B");

    let mut got = [0u8; 8];
    w.read_into(HostOffset::ZERO, &mut got).expect("read through the aperture");
    assert_eq!(
        &got, b"\0\0\0\0\0\0\0\0",
        "the aperture still shows A after being re-pointed at B. The guest writes the BAR0 \
         window latch and reads through the aperture on its next instruction, so this is not a \
         latency bug — it is the guest reading the PREVIOUS framebuffer with no trap and no \
         error, which the trap contract calls the one failure on this path that cannot be \
         contained."
    );

    // ⚠ And the handle a caller took BEFORE the move now sees the NEW backing, which is the
    // correct semantics for a moving aperture and is exactly how the arena arm behaves today.
    w.write_from(HostOffset::ZERO, b"NODE-B!!").expect("write again");
    let a_direct = m
        .device_window_handle(region)
        .expect("the handle is still the window's");
    let mut after = [0u8; 8];
    a_direct.read_into(HostOffset::ZERO, &mut after).expect("read back");
    assert_eq!(&after, b"NODE-B!!", "the write landed in the node the aperture now shows");
}

/// ★★★★★ **THE LENGTH GATE — a short re-point is REFUSED, not truncated.**
///
/// `window_unsafe.rs`'s `a_short_re_point_leaves_the_windows_tail_showing_the_backing_it_
/// replaced` produces the defect on purpose one layer down: the head shows the new backing and
/// the tail shows the old one, one guest-physical range over two framebuffers, reported by
/// nothing. This is the refusal that keeps it unreachable from the mirror.
#[test]
fn a_re_point_that_is_not_the_windows_length_is_refused_by_name_and_places_nothing() {
    let (m, _host, slots) = common::machine();
    let a = SharedRam::create(pramin_len()).expect("node A");
    let b = SharedRam::create(pramin_len()).expect("node B");

    let region = m
        .install_device_page(pramin_gpa(), pramin_len(), a.as_backing_fd(), false)
        .expect("install over A");
    let w = m.device_window_handle(region).expect("handle");
    w.write_from(HostOffset::new(pramin_len() - common::page()), b"TAIL-A!!")
        .expect("write into the LAST page, which a short re-point would strand");

    let (i0, c0) = (slots.installs(), slots.clears());
    let short = m.repoint_device_window(region, b.as_backing_fd(), pramin_len() - common::page(), true);
    assert!(
        matches!(&short, Err(VmmError::Unsupported(why)) if *why == REPOINT_LENGTH_IS_NOT_THE_WINDOWS),
        "a short re-point must be refused BY NAME, got {short:?}"
    );

    let mut tail = [0u8; 8];
    w.read_into(HostOffset::new(pramin_len() - common::page()), &mut tail)
        .expect("read the tail");
    assert_eq!(
        &tail, b"TAIL-A!!",
        "the refusal must leave the window exactly as it was — NEVER HALF. A refusal that had \
         already placed the head would be the worst of both: refused to the caller, and a \
         two-framebuffer aperture to the guest."
    );
    assert_eq!(
        (slots.installs(), slots.clears()),
        (i0, c0),
        "and a refusal must not touch the memslot plane either"
    );

    // ⊘ Non-vacuity: a region that is not an installed window is a DIFFERENT refusal, so the
    // assertion above is about the length and not about everything failing.
    let nowhere = m.repoint_device_window(
        RamRegionId(u64::MAX),
        b.as_backing_fd(),
        pramin_len(),
        true,
    );
    assert!(
        matches!(&nowhere, Err(VmmError::Unsupported(why)) if *why != REPOINT_LENGTH_IS_NOT_THE_WINDOWS),
        "an unknown region must refuse for its own reason, got {nowhere:?}"
    );
}
