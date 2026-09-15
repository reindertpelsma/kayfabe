//! ★★★★★ **§3's SINGLE STORE — what `DeviceFb` says, and what it refuses to say.**
//!
//! `SINGLE_STORE_PLAN.md` §3, cut A. The store names every framebuffer page as an address in
//! the one reserved device-local object and refuses every host-side access **by name**.
//!
//! # ⊘⊘ The property these tests exist to pin, and it is a REFUSAL
//!
//! The tempting bug is a fallback: serve host reads out of host memory *"just for the
//! walkers"*. That is two memories for one address — a value that reads back correctly and is
//! in the wrong memory — which is exactly what §18 and §22 exist to delete and what
//! `two_worlds_split::a_framebuffer_page_written_through_bar1_is_the_page_bar2_reads` is the
//! falsifier for. ⇒ the refusal is the feature, and it is asserted **by its sentence**, not by
//! "something was refused".

use kayfabe_device::{
    DEVICE_HOST_READ_UNBUILT, DEVICE_HOST_WRITE_UNBUILT, DeviceFb, FbPageBacking, FbStore,
};

const FB: u64 = 64 * 1024 * 1024;

/// ★★★★★ **THE HALF THAT WORKS** — every page names its own address in the object.
#[test]
fn every_framebuffer_page_names_its_own_address_in_the_reserved_object() {
    let mut fb = DeviceFb::new(FB);
    for phys in [0u64, 0x1000, 0x1234, 0x20_0000, FB - 0x1000] {
        assert_eq!(
            fb.page_backing(phys, true),
            FbPageBacking::Device {
                at: phys & !0xfff
            },
            "the store must answer the PAGE-ALIGNED address of {phys:#x} in the reserved \
             object. An unaligned `at` would be armed as an unaligned view and the driver \
             would refuse the mmap, which reads as `the crossing is broken`."
        );
    }
}

/// ⊘ **`materialise` is ignored, and that is not a stub.** There is nothing to materialise:
/// the object was reserved whole at realize, before the guest's first instruction. A store
/// that answered `Refused` for `materialise = false` would make the mirror's phase-3
/// re-resolve — which passes `false` — drop every slot it had just installed.
#[test]
fn the_answer_does_not_depend_on_materialise_because_there_is_nothing_to_materialise() {
    let mut fb = DeviceFb::new(FB);
    assert_eq!(
        fb.page_backing(0x8000, false),
        fb.page_backing(0x8000, true),
        "★ the mirror's phase-1 asks with `true` and its phase-3 commit check asks with \
         `false`, comparing the two keys for equality. A store whose answer moved between \
         them would drop every slot at the moment of committing it — and the symptom would \
         be `RACED-AND-DROPPED` on every page, which looks like a concurrency bug."
    );
}

/// ★★★★★ **A HOST READ IS REFUSED BY NAME, NOT SERVED FROM SOMEWHERE ELSE.**
#[test]
fn a_host_read_is_refused_by_name_and_never_falls_back_to_host_memory() {
    let mut fb = DeviceFb::new(FB);
    let mut buf = [0xAAu8; 8];
    let e = fb
        .read(0x4000, &mut buf)
        .expect_err("a host read of device memory must not succeed: there is no CPU view");
    assert_eq!(e.why, DEVICE_HOST_READ_UNBUILT, "refused, but not by this name");
    assert_eq!(
        buf, [0xAAu8; 8],
        "⊘ and the buffer must be UNTOUCHED. A refusal that had zero-filled it would be \
         indistinguishable, at every caller, from a successful read of an unwritten page — \
         which is the one answer this store must never give, because under one object there \
         is no such thing as an unwritten page."
    );
}

/// As above, for writes. ⊘ There is deliberately no success-shaped answer.
#[test]
fn a_host_write_is_refused_by_name() {
    let mut fb = DeviceFb::new(FB);
    let e = fb
        .write(0x4000, &[1, 2, 3, 4])
        .expect_err("a host write to device memory must not succeed silently");
    assert_eq!(e.why, DEVICE_HOST_WRITE_UNBUILT);
}

/// ⊘ **Outside the advertised framebuffer is a DIFFERENT refusal**, and the two must not be
/// one: *"that address was never promised to the guest"* and *"that address is real video
/// memory the host cannot reach"* have opposite fixes.
#[test]
fn outside_the_framebuffer_is_a_different_refusal_from_not_reachable() {
    let mut fb = DeviceFb::new(FB);
    let inside = fb.read(FB - 8, &mut [0u8; 8]).unwrap_err();
    let outside = fb.read(FB, &mut [0u8; 8]).unwrap_err();
    assert_ne!(
        inside.why, outside.why,
        "★ one of these is cut B's work and the other is a guest asking for memory it was \
         never told about. A single 'refused' would make the boot's census unable to tell a \
         missing mechanism from a misbehaving guest."
    );
    assert_eq!(inside.why, DEVICE_HOST_READ_UNBUILT);
}

/// ⊘ **`residency` and `is_resident` answer `None`, never `Some(false)`.**
///
/// Under one reserved object *every* page of the framebuffer exists, always. `Some(false)`
/// would be a positive claim that the guest's video memory is absent — the same error as
/// decoding an empty capture to zeros — and consumers branch on it.
#[test]
fn residency_is_unanswerable_and_says_so_rather_than_claiming_absence() {
    let fb = DeviceFb::new(FB);
    assert!(fb.residency().is_none());
    assert!(
        fb.is_resident(0x1000).is_none(),
        "`Some(false)` here would say a page the reserved object certainly contains is missing"
    );
    assert_eq!(
        fb.resident_bytes(),
        0,
        "zero is the TRUTH here and not a stub: this store holds no host memory on the \
         guest's behalf at all, which is the whole point of it"
    );
}

/// ⊘ **`install_join` is refused**, through the trait's default, and that is the deliberate
/// answer rather than an omission.
///
/// A join replaces the store's own pages with memory a second party also maps. Under one
/// object the store HAS no pages of its own and there is no second memory to join to: the
/// guest's framebuffer and the engines' framebuffer are the same object by construction. ⇒
/// `SINGLE_STORE_PLAN.md` §3 item 3's *"`install_join` answered deliberately (under one store
/// there are no two memories, so the join's whole premise changes)"*, answered.
#[test]
fn a_join_has_no_premise_under_one_store_and_is_refused() {
    let fb = DeviceFb::new(FB);
    assert!(
        fb.joined_ranges().is_empty(),
        "and it holds none, which is a real answer rather than 'cannot say'"
    );
    // ⊘ The refusal itself is the trait default (`NO_JOIN_SUPPORT`) and needs a
    // `Box<dyn FbJoined>` to exercise; the observable half — that nothing is ever joined —
    // is asserted above. The premise is named in this test's doc so the deliberateness is on
    // the record rather than inferred from an absent `impl`.
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ CUT B — THE BYTE PORT. Every one of these is a KNOWN-POSITIVE for a new counter
// or a new refusal: `SINGLE_STORE_PLAN.md`'s most expensive recurring defect is a census
// that prints `0` both when nothing happened and when the arm never ran.
// ═══════════════════════════════════════════════════════════════════════════════════════

mod fakeport;
use fakeport::{FAKE_GRAIN, FakePort};
use kayfabe_device::{DEVICE_HOST_READ_NOT_ARMED, DEVICE_HOST_WRITE_NOT_ARMED};
use std::sync::Arc;

/// ★★★★★ **THE HALF CUT B ADDS — a host read of an ARMED run is SERVED out of the object.**
///
/// ⊘ And it is served with the **bytes that were already there**, written into the object by
/// something that never went through this store. That is the shape of the real thing: the
/// guest's engines write video memory, and a CPU view arrives afterwards.
#[test]
fn a_host_read_of_an_armed_run_is_served_out_of_the_reserved_object() {
    let port = Arc::new(FakePort::new(FB));
    port.poke(0x4000, &[1, 2, 3, 4, 5, 6, 7, 8]);
    port.arm(0x4000);
    let mut fb = DeviceFb::with_port(FB, port.clone() as Arc<dyn kayfabe_device::DeviceFbPort>);
    let mut buf = [0u8; 8];
    fb.read(0x4000, &mut buf)
        .expect("an armed run must serve a host read — that is the whole of cut B item 1");
    assert_eq!(buf, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(port.counts().3, 1, "and the PORT must be what served it");
}

/// ★★★★★ **THE DEMAND SET — a miss RECORDS, and the sentence says so.**
///
/// ⊘⊘ This is cut B item 3's mechanism in one assertion. `FbRead::read_in` answers a `bool`,
/// so the address that missed cannot travel to the lock-free caller inside the refusal; it
/// survives in the port's want set instead. A store that refused **without** recording would
/// leave the retry with nothing to arm, and the boot would look exactly like cut A.
#[test]
fn a_host_read_of_an_unarmed_run_records_a_want_and_refuses_by_the_cut_b_name() {
    let port = Arc::new(FakePort::new(FB));
    let mut fb = DeviceFb::with_port(FB, port.clone() as Arc<dyn kayfabe_device::DeviceFbPort>);
    let mut buf = [0xAAu8; 8];
    let e = fb.read(0x9000, &mut buf).expect_err("nothing is armed");
    assert_eq!(
        e.why, DEVICE_HOST_READ_NOT_ARMED,
        "⊘ refused, but by cut A's `the mechanism does not exist` sentence rather than cut \
         B's `it exists and nothing armed this page yet`. The two have opposite fixes."
    );
    assert_eq!(
        buf, [0xAAu8; 8],
        "and the buffer is UNTOUCHED — a zero-fill would be indistinguishable from a \
         successful read of an unwritten page"
    );
    assert_eq!(
        port.wanted_now(),
        1,
        "★ THE KNOWN-POSITIVE: the want set must have exactly the run that missed. A refusal \
         that recorded nothing is cut A wearing cut B's sentence."
    );
    assert_eq!(port.counts().5, 1, "and it was recorded as a READ's demand");
    assert_eq!(port.counts().6, 0, "not as a write's");
}

/// ★★★★★ **DRAIN-THEN-RETRY IS WHAT TURNS THE REFUSAL INTO A READ** — the whole of cut B,
/// at the store's own seam, with the caller's two halves spelled out.
#[test]
fn a_drain_arms_what_was_wanted_and_the_second_attempt_succeeds() {
    let port = Arc::new(FakePort::new(FB));
    port.poke(0x9000, &[9; 8]);
    let mut fb = DeviceFb::with_port(FB, port.clone() as Arc<dyn kayfabe_device::DeviceFbPort>);
    let mut buf = [0u8; 8];
    assert!(fb.read(0x9000, &mut buf).is_err(), "first attempt, unarmed");

    // ★ The lock-free half. In production this is `RegPlane::arm_fb_demand`, called with the
    // plane's locks released.
    let d = kayfabe_device::FbStore::demand_port(&fb)
        .expect("the store must hand its port out, or no lock-free caller can drain it")
        .drain();
    assert_eq!(d.armed, 1, "the drain must arm exactly the run that was wanted");
    assert!(d.progressed(), "and say so, because that is what a retry branches on");
    assert!(!d.declined);

    fb.read(0x9000, &mut buf)
        .expect("second attempt, after the arm — this is cut B working");
    assert_eq!(buf, [9u8; 8]);
}

/// ⊘⊘⊘ **A DECLINED DRAIN IS NOT AN EMPTY ONE** — cut B item 5's distinction, pinned.
///
/// The shell declines when the calling thread is a vCPU or inside an MMIO trap. A caller that
/// read *"declined"* as *"nothing was wanted"* would retry forever on the one arm where
/// retrying cannot ever help, **on a vCPU**, which is the thread it must not spin on.
#[test]
fn a_declined_drain_is_distinguishable_from_one_that_had_nothing_to_do() {
    let port = Arc::new(FakePort::new(FB));
    let mut fb = DeviceFb::with_port(FB, port.clone() as Arc<dyn kayfabe_device::DeviceFbPort>);
    let _ = fb.read(0x9000, &mut [0u8; 8]);

    port.set_declining(true);
    let d = kayfabe_device::FbStore::demand_port(&fb).unwrap().drain();
    assert!(d.declined, "★ the known-positive for the decline itself");
    assert_eq!(d.armed, 0);
    assert!(!d.progressed());
    assert_eq!(
        port.wanted_now(),
        1,
        "⊘ and the demand SURVIVES a declined drain — a decline that consumed the want set \
         would lose the very page an off-vCPU caller is about to arm"
    );

    // ── and the empty drain, for contrast: it ran, and there was nothing to do ──
    port.set_declining(false);
    assert_eq!(kayfabe_device::FbStore::demand_port(&fb).unwrap().drain().armed, 1);
    let empty = kayfabe_device::FbStore::demand_port(&fb).unwrap().drain();
    assert!(!empty.declined, "★ THE CONTRAST: this one RAN");
    assert_eq!(empty.armed, 0);
}

/// ⚠ **A REFUSED ARM IS A THIRD THING** — the host BAR1 aperture being full, which no amount
/// of retrying fixes. `[measured w722]` it arrives as `NV_ERR_NO_MEMORY` with `ioctl()`
/// returning 0 and `errno == 0`, so nothing else in the system will mention it.
#[test]
fn a_refused_arm_is_reported_as_refused_and_not_as_nothing_to_do() {
    let port = Arc::new(FakePort::new(FB));
    let mut fb = DeviceFb::with_port(FB, port.clone() as Arc<dyn kayfabe_device::DeviceFbPort>);
    let _ = fb.read(0x9000, &mut [0u8; 8]);
    port.set_refusing(true);
    let d = kayfabe_device::FbStore::demand_port(&fb).unwrap().drain();
    assert_eq!(d.refused, 1, "★ the known-positive for the refusal count");
    assert_eq!(d.armed, 0);
    assert!(
        !d.progressed(),
        "and a refusal must NOT read as progress, or the retry loop spins against a full \
         aperture"
    );
}

/// ⊘ **CUT A's SENTENCE AND CUT B's ARE DIFFERENT, AND BOTH ARE REACHABLE.**
///
/// A store with no port says *"this mechanism does not exist"*; one with a port says *"it
/// exists and nothing armed this page yet"*. ⚠ One sentence for both would make a boot unable
/// to tell a missing byte port from a retry that never ran — and those have opposite fixes.
#[test]
fn the_no_port_refusal_and_the_not_armed_refusal_are_different_sentences() {
    let mut cut_a = DeviceFb::new(FB);
    let port = Arc::new(FakePort::new(FB));
    let mut cut_b = DeviceFb::with_port(FB, port as Arc<dyn kayfabe_device::DeviceFbPort>);
    let a = cut_a.read(0x9000, &mut [0u8; 8]).unwrap_err().why;
    let b = cut_b.read(0x9000, &mut [0u8; 8]).unwrap_err().why;
    assert_eq!(a, DEVICE_HOST_READ_UNBUILT);
    assert_eq!(b, DEVICE_HOST_READ_NOT_ARMED);
    assert_ne!(a, b);
    assert!(
        kayfabe_device::FbStore::demand_port(&cut_a).is_none(),
        "⊘ and cut A's store hands out no port, so `arm_fb_demand` cannot mistake it for one \
         whose drain merely armed nothing"
    );
}

/// ★★★ **THE WRITE HALF — served through an armed run, and landing NOWHERE otherwise.**
///
/// ⚠ `[measured w736]` `host_write_refused=0` against `host_read_refused=20`: nothing on the
/// `RmInitAdapter` path has ever wanted a host-side write. ⇒ this pins that a write which
/// misses changes **no byte anywhere**, which is the property that makes "no write-side retry
/// was built" a safe scoping rather than a hole.
#[test]
fn a_write_that_misses_lands_nowhere_at_all() {
    let port = Arc::new(FakePort::new(FB));
    port.poke(0x9000, &[7; 8]);
    let mut fb = DeviceFb::with_port(FB, port.clone() as Arc<dyn kayfabe_device::DeviceFbPort>);
    let e = fb.write(0x9000, &[0xFF; 8]).expect_err("nothing is armed");
    assert_eq!(e.why, DEVICE_HOST_WRITE_NOT_ARMED);
    assert_eq!(
        port.peek(0x9000, 8),
        vec![7u8; 8],
        "⊘⊘ THE BYTES MUST BE UNCHANGED. A partial or fallback write is a byte the engines \
         never see, which is the failure a single store exists to make impossible."
    );
    assert_eq!(port.counts().6, 1, "and the write's demand was recorded");

    port.arm(0x9000);
    fb.write(0x9000, &[0xFF; 8])
        .expect("an armed run serves the write");
    assert_eq!(port.peek(0x9000, 8), vec![0xFFu8; 8]);
}

/// ⊘ **A run that spans the grain boundary is all-or-nothing.** A half-filled buffer would be
/// invisible at `FbRead::read_in`, which answers a `bool`.
#[test]
fn a_read_spanning_two_runs_needs_both_and_fills_neither_until_it_has_them() {
    let port = Arc::new(FakePort::new(FB));
    let at = FAKE_GRAIN - 4;
    port.poke(at, &[1, 2, 3, 4]);
    port.poke(FAKE_GRAIN, &[5, 6, 7, 8]);
    port.arm(0);
    let mut fb = DeviceFb::with_port(FB, port.clone() as Arc<dyn kayfabe_device::DeviceFbPort>);
    let mut buf = [0xEEu8; 8];
    assert!(
        fb.read(at, &mut buf).is_err(),
        "one of the two runs is unarmed, so the read cannot be served"
    );
    assert_eq!(buf, [0xEEu8; 8], "and NOTHING was copied");
    assert_eq!(
        port.wanted_now(),
        2,
        "⊘ BOTH runs are wanted, including the one already armed — and that is correct rather \
         than sloppy: `want` runs UNDER THE PLANE LOCK, so asking `is this one already armed?` \
         there would take the port's own map on a vCPU inside an MMIO exit to save an arm the \
         drain already skips for free."
    );
    assert_eq!(
        kayfabe_device::FbStore::demand_port(&fb).unwrap().drain().armed,
        1,
        "★ THE KNOWN-POSITIVE FOR THAT BEING FREE: the drain arms exactly the ONE run that \
         was missing. A drain that re-armed the other would leak an aperture per miss."
    );
    fb.read(at, &mut buf).expect("both runs armed");
    assert_eq!(buf, [1, 2, 3, 4, 5, 6, 7, 8]);
}
