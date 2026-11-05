//! ★★★★★ **w742 — THE PUBLISH ROUTE'S DEVICE ARM**, and the mirror quiesce it exists to
//! delete.
//!
//! # ⊘⊘⊘ THE DEFECT THIS FILE IS THE FALSIFIER FOR, MEASURED
//!
//! `[measured w740, traces/w740_scrub_arming/, the `device` arm]` the publish route offered
//! every framebuffer leaf to `RegPlane::join_fb`. Under [`DeviceFb`] that refuses **correctly
//! and by name** ([`NO_JOIN_SUPPORT`]) — the store has no pages of its own. The refusal was
//! never the problem. The problem is that `join_fb`'s **first** act is to quiesce the BAR
//! mirror over the range, and a refused install puts nothing back:
//!
//! ```text
//! THE INSTALL REFUSED …                4431   (37 distinct frames, re-offered)
//! quiesce[calls=4431 removed=58175]           ⇐ the SAME 4431, one per refused install
//! BAR1-PASSTHROUGH arm=on misses=2183
//! BAR-MIRROR bar1 TRAP_FILLS=44 … refused=[ALREADY-COVERED-EARLY=2139]     (44+2139=2183)
//! ```
//!
//! ⇒ every BAR1 guest exit on that boot landed in a window a **refused** join had opened by
//! taking the memory slot away, and 2139 of them found the slot back again by the time the
//! fill ran — which is what `ALREADY-COVERED-EARLY` *beside a trap* means. ★ Constraint 1 and
//! §23: a trap on BAR1 **is** the error, so this is not "fewer traps", it is the cause of all
//! of them.
//!
//! # ★★★ WHAT IS PINNED HERE, AND WHY EACH ONE IS A KNOWN-POSITIVE
//!
//! Every assertion below fails on the **pre-w742 tree**, and the way to see that is written
//! into each test: delete [`DeviceFb`]'s `join_plan` override and the trait default answers
//! `NeedsRegion`, which is precisely the pre-w742 behaviour. ⊘ A test that only asserted the
//! new arm's *presence* would pass on a tree where the arm is wired nowhere — this tree's
//! recurring defect (`a_census_that_reports_a_zero_its_own_plumbing_guarantees`), so the
//! central test asserts the **quiesce count**, which is the thing the guest pays.

use kayfabe_device::{
    DeviceFb, DeviceFbPort, FbJoinPlan, FbJoined, FbMirrorPort, FbStore, OUTSIDE_FRAMEBUFFER,
    RefusingFb, SparseFb,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

mod fakeport;
use fakeport::{FAKE_GRAIN, FakePort};

/// The advertised framebuffer. Small; nothing here depends on the size.
const FB: u64 = 0x100_0000;
/// A leaf's base and length — the 64 KiB granule the publish route actually offers
/// (`[measured w740]` every `LEAF-HUGE len=65536`).
const AT: u64 = 0x40_0000;
const LEN: u64 = 0x1_0000;

fn plane() -> kayfabe_device::RegPlane {
    kayfabe_device::RegPlane::new(
        kayfabe_device::default_chip(),
        kayfabe_device::abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER)
            .expect("the bench driver has a table"),
        Box::new(kayfabe_device::SteppingClock::new(1)),
    )
    .expect("the shipped row is servable")
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// §1 — THE STORE'S ANSWER
// ═══════════════════════════════════════════════════════════════════════════════════════

/// ★★★★★ **The device store names the OFFSET instead of asking for a region.**
///
/// ⊘ KNOWN-POSITIVE: remove `DeviceFb::join_plan` and the trait default answers
/// `NeedsRegion`, which is the pre-w742 tree — this assertion goes red on it.
#[test]
fn the_device_store_answers_the_publish_route_with_an_offset_not_a_request_for_a_region() {
    let fb = DeviceFb::new(FB);
    for (phys, len) in [(0u64, 0x1000u64), (AT, LEN), (FB - LEN, LEN), (FB - 1, 1)] {
        assert_eq!(
            fb.join_plan(phys, len),
            FbJoinPlan::DeviceBacked { at: phys },
            "the range at {phys:#x}+{len:#x} IS the one reserved object at that offset. \
             Answering `NeedsRegion` sends the publish route off to mint a host object for \
             it — a SECOND memory for one address (§18/§22) — and, on the way, to quiesce \
             every memory slot the guest holds over the leaf."
        );
    }
}

/// ⊘ **Outside the object is a REFUSAL, not `NeedsRegion`.**
///
/// A store that answered `NeedsRegion` here would send the caller to mint a host object for
/// an address this device does not have — the same two-memories defect, for a range the store
/// would refuse to read or write anyway ([`OUTSIDE_FRAMEBUFFER`]).
#[test]
fn a_range_outside_the_framebuffer_is_refused_by_name_before_anything_is_minted() {
    let fb = DeviceFb::new(FB);
    for (phys, len) in [(FB, 0x1000u64), (FB - 0x800, 0x1000), (FB + 0x10_0000, LEN)] {
        assert_eq!(
            fb.join_plan(phys, len),
            FbJoinPlan::Refused(OUTSIDE_FRAMEBUFFER),
            "{phys:#x}+{len:#x} is not wholly inside a {FB:#x}-byte framebuffer"
        );
    }
}

/// ⊘⊘ **A leaf whose base plus length WRAPS is refused, not answered as inside.**
///
/// Both numbers are guest-authored on this path (`leaf.phys`, `leaf.len` come out of the
/// guest's own page tables), so an unchecked `phys + len` would wrap to a small number and
/// answer `DeviceBacked` for a range entirely outside the object. The structural invariant
/// is *every dereference bounds-checked*; this is the arithmetic half of it.
#[test]
fn a_leaf_whose_base_plus_length_wraps_is_refused_rather_than_answered_as_inside() {
    let fb = DeviceFb::new(FB);
    assert_eq!(
        fb.join_plan(u64::MAX - 0x100, 0x1000),
        FbJoinPlan::Refused(OUTSIDE_FRAMEBUFFER),
        "0xffff_ffff_ffff_ff00 + 0x1000 wraps. `checked_add` is what makes this a refusal \
         instead of a `DeviceBacked` answer for an address the object does not contain."
    );
}

/// ⊘ **A zero-length leaf names no installable slice and is not `DeviceBacked` by
/// arithmetic.** Every base is "inside" a range of length zero; answering `DeviceBacked`
/// would hand the caller an offset with nothing at it.
#[test]
fn a_zero_length_leaf_is_judged_on_one_byte_rather_than_on_none() {
    let fb = DeviceFb::new(FB);
    assert_eq!(fb.join_plan(AT, 0), FbJoinPlan::DeviceBacked { at: AT });
    assert_eq!(
        fb.join_plan(FB, 0),
        FbJoinPlan::Refused(OUTSIDE_FRAMEBUFFER),
        "a zero-length leaf AT THE END of the object is inside it by arithmetic and outside \
         it by any honest reading; `len.max(1)` is what makes the second one win."
    );
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// §2 — THE CONTROL. The default arm must be BYTE-IDENTICAL, proved rather than asserted.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// ★★★★★ **THE DEFAULT ARM (`KAYFABE_FB_STORE` unset) IS UNCHANGED, BY CONSTRUCTION.**
///
/// The publish route's new branch runs **only** on `DeviceBacked`. Every store but
/// [`DeviceFb`] takes the trait default and answers `NeedsRegion` for every range, in range
/// or out of it — so on the arena arm the route below the branch is entered on exactly the
/// inputs it was entered on before, and nothing it does can differ.
///
/// ⊘ This is the *"proved by a test, not asserted"* half of the brief, and it is asserted
/// over the **out-of-range** case too: a store that grew a `Refused` answer here would make
/// the arena route refuse leaves it used to attempt, which is a behaviour change on the
/// control.
#[test]
fn every_store_but_the_device_store_answers_needs_region_so_the_default_arm_is_untouched() {
    let sparse = SparseFb::new(FB);
    let refusing = RefusingFb;
    let stores: [(&str, &dyn FbStore); 2] = [("SparseFb", &sparse), ("RefusingFb", &refusing)];
    for (name, store) in stores {
        for (phys, len) in [
            (0u64, 0x1000u64),
            (AT, LEN),
            (FB - LEN, LEN),
            (FB, 0x1000),
            (FB + 0x10_0000, LEN),
            (u64::MAX - 0x100, 0x1000),
        ] {
            assert_eq!(
                store.join_plan(phys, len),
                FbJoinPlan::NeedsRegion,
                "{name} must answer `NeedsRegion` for {phys:#x}+{len:#x}. The publish route \
                 branches on this answer and NOTHING else; a second arm here is a behaviour \
                 change on the shipped default, which is the one thing w742 may not do."
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// §3 — THE POINT: the mirror quiesce, counted.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// A mirror port that does nothing but count. ⊘ The real one retires memory slots through a
/// hypervisor ioctl; what is under test is **how often the plane calls it**, which is the
/// number the guest pays in VM exits.
#[derive(Debug, Default)]
struct CountingMirror {
    quiesces: AtomicU64,
    resumes: AtomicU64,
    retire_alls: AtomicU64,
}

impl CountingMirror {
    fn quiesces(&self) -> u64 {
        self.quiesces.load(Ordering::Relaxed)
    }
}

impl FbMirrorPort for CountingMirror {
    fn quiesce(&self, _phys: u64, _len: u64) {
        self.quiesces.fetch_add(1, Ordering::Relaxed);
    }
    fn resume(&self, _phys: u64, _len: u64) {
        self.resumes.fetch_add(1, Ordering::Relaxed);
    }
    fn retire_all(&self, _why: &'static str) {
        self.retire_alls.fetch_add(1, Ordering::Relaxed);
    }
    fn revalidate_pending(&self) {}
    fn drain_fills(&self) {}
}

/// A stand-in for the isolate's mapping — the `Box<dyn FbJoined>` the old route hands over.
#[derive(Debug)]
struct Elsewhere(Vec<u8>);

impl FbJoined for Elsewhere {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read(&self, off: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        let off = usize::try_from(off).map_err(|_| "offset does not fit")?;
        let end = off.checked_add(buf.len()).ok_or("read wraps")?;
        let src = self.0.get(off..end).ok_or("read past the end")?;
        buf.copy_from_slice(src);
        Ok(())
    }
    fn write(&mut self, off: u64, bytes: &[u8]) -> Result<(), &'static str> {
        let off = usize::try_from(off).map_err(|_| "offset does not fit")?;
        let end = off.checked_add(bytes.len()).ok_or("write wraps")?;
        let dst = self.0.get_mut(off..end).ok_or("write past the end")?;
        dst.copy_from_slice(bytes);
        Ok(())
    }
}

/// ★★★★★ **THE WHOLE OF w742 IN ONE ASSERTION: the plan costs NO quiesce and the join costs
/// one that buys nothing.**
///
/// ⊘⊘ This is the test the brief demands and the one the pre-w742 tree cannot pass. It does
/// not assert that a new method exists; it asserts the **number the guest pays**. ★ And it
/// asserts both halves in one boot-shaped sequence, so a reader can see that the second is
/// what the first replaces.
///
/// # ⚠ HOW TO BREAK IT AND WATCH IT GO RED (done, and the results are in the w742 report)
///
/// * delete `DeviceFb::join_plan` ⇒ the plan answers `NeedsRegion`, the first assertion fails;
/// * make `RegPlane::fb_join_plan` quiesce "to be safe" ⇒ `quiesces() == 0` fails;
/// * make `RegPlane::join_fb` skip its quiesce ⇒ the third assertion fails, which is the
///   guard on the OTHER direction: `join_fb`'s quiesce is correct and must stay.
#[test]
fn the_plan_costs_no_mirror_quiesce_and_the_join_it_replaces_costs_one_that_buys_nothing() {
    let plane = plane();
    plane.set_fb(Box::new(DeviceFb::new(FB)));
    let mirror = Arc::new(CountingMirror::default());
    plane.set_fb_mirror(mirror.clone() as Arc<dyn FbMirrorPort>);

    // ---- THE NEW ROUTE: ask first, destroy nothing.
    assert_eq!(
        plane.fb_join_plan(AT, LEN),
        FbJoinPlan::DeviceBacked { at: AT },
        "the plane must carry the store's answer through unchanged"
    );
    assert_eq!(
        mirror.quiesces(),
        0,
        "★★★★★ THE NUMBER. Asking what a range needs must not take the guest's memory slots \
         away. `[measured w740]` 4431 of these removed 58175 slots and bought NOTHING, and \
         the guest paid 2183 VM exits into the windows they opened."
    );

    // ---- THE OLD ROUTE, on the same store: one quiesce, one refusal, nothing installed.
    let e = plane
        .join_fb(AT, Box::new(Elsewhere(vec![0u8; LEN as usize])))
        .expect_err("a join over the one reserved object would be a second memory");
    assert_eq!(
        e.why,
        kayfabe_device::DEVICE_JOIN_IS_A_SECOND_MEMORY,
        "⊘⊘ refused, but by the INHERITED sentence. `NO_JOIN_SUPPORT` is the trait DEFAULT's, \
         and it happens to be true of this store — which is exactly how an unwritten method \
         passed for a ruling through two reviews. The refusal must be this store's own."
    );
    assert_eq!(
        e.len, LEN as usize,
        "⊘ and it must say HOW MUCH was refused. The default answers `len: 0`, which is why \
         `[measured w740]` all 4431 refusals read `len=0` and the volume of the defect was \
         invisible in the boot log."
    );
    assert_eq!(
        mirror.quiesces(),
        1,
        "⊘ and this is the cost the new arm removes — NOT a defect in `join_fb`, whose \
         quiesce-before-ask is what keeps the establishment copy safe on a store that CAN \
         join. The defect is asking at all."
    );
    assert!(
        plane.joined_fb_ranges().is_empty(),
        "nothing was installed, so the slot removal above bought nothing at all"
    );
}

/// ⊘ **The control for the test above**, on the arena store: the plan answers `NeedsRegion`
/// and the join it then performs quiesces exactly once and SUCCEEDS.
///
/// ★ Without this, *"the plan costs no quiesce"* would be satisfied by a plane that never
/// quiesces for anybody — a property that would silently break every join the default arm
/// depends on.
#[test]
fn on_the_arena_arm_the_plan_says_needs_region_and_the_join_still_installs() {
    let plane = plane();
    plane.set_fb(Box::new(SparseFb::new(FB)));
    let mirror = Arc::new(CountingMirror::default());
    plane.set_fb_mirror(mirror.clone() as Arc<dyn FbMirrorPort>);

    assert_eq!(plane.fb_join_plan(AT, LEN), FbJoinPlan::NeedsRegion);
    assert_eq!(mirror.quiesces(), 0, "the plan is a question, on every arm");

    plane
        .join_fb(AT, Box::new(Elsewhere(vec![0u8; LEN as usize])))
        .expect("the arena store joins — this is the shipped default and it must not move");
    assert_eq!(
        mirror.quiesces(),
        1,
        "and the join's own quiesce is untouched"
    );
    assert_eq!(
        plane.joined_fb_ranges(),
        vec![(AT, LEN)],
        "⊘ the range is joined, which is what makes the quiesce above worth paying"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// §4 — THE INSTALL: a slice of the one reserved object, `want` here and `drain` lock-free.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// ★★★★★ **THE INSTALL HALF — `want` reaches the byte port, `drain` arms the slice, and a
/// host read of it is then SERVED out of the reserved object.**
///
/// ⊘ This is the other thing the publish route was not doing. A leaf the store answers
/// `DeviceBacked` for is already the reserved object for every GUEST access (the mirror's
/// memslots name it). The one thing not yet true of it is a **CPU view** — which is exactly
/// what `FwdFault::CpuCeFb` refuses for, `[measured w740]` at `phys: 69632` (= `0x11000`,
/// inside the most-offered leaf `0x10000`) and `phys: 1118208` (= `0x111000`, inside
/// `0x110000`, the third).
///
/// ⚠ KNOWN-POSITIVE: drop the `port.want(..)` call in `want_fb_device_slice` and the drain
/// below arms nothing, so the read stays refused and this goes red.
#[test]
fn the_want_half_reaches_the_port_and_the_drain_installs_the_slice() {
    let plane = plane();
    let port = Arc::new(FakePort::new(FB));
    port.poke(AT, &[9, 8, 7, 6]);
    plane.set_fb(Box::new(DeviceFb::with_port(
        FB,
        port.clone() as Arc<dyn DeviceFbPort>,
    )));

    assert_eq!(port.armed_now(), 0, "nothing is armed before the arm runs");
    assert!(
        plane.want_fb_device_slice(AT, LEN),
        "the single store HAS a byte port, so the want must be taken"
    );
    assert!(
        port.wanted_now() > 0,
        "⊘ and it must reach the port, not merely be counted"
    );

    let d = plane.arm_fb_demand();
    assert!(d.armed > 0, "the drain is the IPC half and it is what arms");
    assert!(port.armed_now() > 0);

    // The falsifier that the slice is REACHABLE, not merely recorded: the same host-side
    // read that was refused `DEVICE_HOST_READ_NOT_ARMED` before the install.
    let mut buf = [0u8; 4];
    plane
        .fb_peek(AT, &mut buf)
        .expect("an installed slice must serve a host read out of the reserved object");
    assert_eq!(buf, [9, 8, 7, 6]);
}

/// ⊘ **The want half answers `false` on the arena arm BY CONSTRUCTION, not by inspection.**
///
/// [`kayfabe_device::FbStore::demand_port`] is `None` for every store but the single store
/// with a port, so there is no configuration in which the publish route's install could run
/// on the control. ★ That is the same shape cut C used for its repair gate, and for the same
/// reason: a control kept true by a branch is a control one edit away from being false.
#[test]
fn the_install_half_is_unreachable_on_the_arena_arm_by_construction() {
    let plane = plane();
    plane.set_fb(Box::new(SparseFb::new(FB)));
    assert!(
        !plane.want_fb_device_slice(AT, LEN),
        "the arena store has no byte port, so there is nothing to record a want with"
    );
    let d = plane.arm_fb_demand();
    assert_eq!(
        (d.armed, d.refused, d.declined),
        (0, 0, false),
        "⊘ and the drain is a no-op rather than a decline: `no port` and `declined on a vCPU` \
         are different findings and the census must not merge them"
    );
}

/// ⊘ A want for a slice outside the object is **not** recorded: an address that cannot be
/// armed however many times it is asked for would make every later drain spend an IPC round
/// trip discovering that again. ★ The store refuses it one layer up
/// (`join_plan` ⇒ `Refused`), so this is the belt the port already wears.
#[test]
fn a_want_outside_the_reserved_object_is_not_recorded_as_demand() {
    let plane = plane();
    let port = Arc::new(FakePort::new(FB));
    plane.set_fb(Box::new(DeviceFb::with_port(
        FB,
        port.clone() as Arc<dyn DeviceFbPort>,
    )));
    plane.want_fb_device_slice(FB + FAKE_GRAIN, FAKE_GRAIN);
    assert_eq!(
        port.wanted_now(),
        0,
        "an unarmable address must not enter the demand set"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// §5 — THE CENSUS, with a known-positive that makes it fire.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// ★★★ **The counter moves when the arm runs**, and the report carries it.
///
/// ⊘ `[the w738 lesson]` two censuses reported the opposite of what the boot did, and the
/// class is *a counter that cannot tell "nothing happened" from "the arm never ran"*. So this
/// reads the counter, makes the arm run, and reads it again — a **delta**, which a counter
/// whose plumbing guarantees a zero cannot produce.
#[test]
fn the_no_join_needed_counter_moves_only_when_the_device_arm_actually_runs() {
    use kayfabe_device::fbwin::DEVICE_FB_JOIN_PLANNED_DEVICE;
    let before = DEVICE_FB_JOIN_PLANNED_DEVICE.load(Ordering::Relaxed);
    // The CONTROL first: an arena store's plan must not touch the device counter.
    let sparse = SparseFb::new(FB);
    let _ = sparse.join_plan(AT, LEN);
    assert_eq!(
        DEVICE_FB_JOIN_PLANNED_DEVICE.load(Ordering::Relaxed),
        before,
        "⊘ a `SparseFb` plan must not move a DEVICE counter. A counter that did would make \
         the control arm print a number about a mechanism it does not have."
    );
    let fb = DeviceFb::new(FB);
    let _ = fb.join_plan(AT, LEN);
    assert!(
        DEVICE_FB_JOIN_PLANNED_DEVICE.load(Ordering::Relaxed) > before,
        "the device arm ran, so the counter must have moved"
    );
    assert!(
        kayfabe_device::device_fb_report().contains("no_join_needed="),
        "and it must be REACHABLE — the w584 defect is a counter incremented correctly for \
         fifteen commits on a type nothing downstream could ask"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// §6 — THE INHERITED-DEFAULT AUDIT. ⊘ `install_join` was found by noticing that a refusal
// JUSTIFIED ITSELF; these pin the two answers that audit turned into rulings.
// ═══════════════════════════════════════════════════════════════════════════════════════

/// ★★★★★ **THE REFUSAL IS THIS STORE'S OWN, NOT THE TRAIT'S.**
///
/// ⊘⊘⊘ The hazard, stated so the next reader inherits the finding rather than the defect: a
/// trait default that **refuses** is a `_ => {}` catch-all one level up, and worse, because
/// its sentence can be a true statement about the type that inherits it. [`NO_JOIN_SUPPORT`]
/// — *"it has no pages of its own to establish from"* — **is** true of [`DeviceFb`], so an
/// unwritten method read as a design ruling and survived two reviews on the strength of its
/// own prose.
///
/// ⚠ KNOWN-POSITIVE: delete `DeviceFb::install_join` and this goes red while everything still
/// *works* — which is the whole problem it pins.
#[test]
fn the_device_stores_join_refusal_is_its_own_ruling_and_not_an_inherited_default() {
    let mut fb = DeviceFb::new(FB);
    let (e, back) = fb
        .install_join(AT, Box::new(Elsewhere(vec![0u8; LEN as usize])))
        .expect_err("a join over the one reserved object is refused");
    assert_eq!(e.why, kayfabe_device::DEVICE_JOIN_IS_A_SECOND_MEMORY);
    assert_ne!(
        e.why,
        kayfabe_device::NO_JOIN_SUPPORT,
        "⊘ the trait default's sentence is TRUE of this store, which is what made an \
         unwritten method indistinguishable from a decision"
    );
    assert_eq!(
        back.len(),
        LEN,
        "and the region must come BACK, so the caller drops it with no plane lock held — \
         `[measured w289j]` dropping it under the lock `munmap`s inside an MMIO callback and \
         aborts the whole VMM"
    );
}

/// ⊘ **The arena census is UNMEASURED on the device arm, not zero.**
///
/// [`kayfabe_device::FbStore::arena_census_all`]'s default is `(0, 0, 0, 0)` and [`DeviceFb`]
/// inherits it, so the BAR-mirror census used to print `store_refused=0 store_migrated=0
/// store_read_refused=0 store_resets=0` on the one arm where those numbers cannot exist —
/// `failed=0 IS NOT "NOTHING REFUSED"`, in the line a boot is graded from. ★ The shell says
/// so instead of printing the four zeros; this pins the store's half of that fact.
#[test]
fn the_device_store_inherits_a_four_zero_arena_census_that_must_never_be_read_as_measured() {
    let fb = DeviceFb::new(FB);
    assert_eq!(
        fb.arena_census_all(),
        (0, 0, 0, 0),
        "⊘ NOT a claim that nothing was refused — this store has no arena at all. The shell's \
         `BAR-MIRROR MECHANISM` line must SAY that rather than print these four."
    );
    let sparse = SparseFb::new(FB);
    assert_eq!(
        sparse.arena_census_all(),
        (0, 0, 0, 0),
        "★ and the control's four zeros are the SAME four numbers, which is precisely why the \
         line cannot be read without knowing which arm printed it"
    );
}
