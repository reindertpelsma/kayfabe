//! ★★★★★ **THE PUBLICATION GATE'S KEY — AND WHY IT HAS GONE CIRCULAR TWICE.**
//!
//! `Vas::publish_epoch` decides whether a publication pass runs or replays its last census.
//! A pass that is skipped publishes nothing; rows the guest declared never reach the host VA
//! space; and the GPU faults `FAULT_PDE` on the first address it is asked to translate. The
//! epoch is therefore not an optimisation detail — it is the thing standing between a
//! declared mapping and a hardware fault.
//!
//! # ⊘⊘⊘ IT HAS BEEN CIRCULAR TWICE, AND NEITHER TIME DID A TEST SAY SO
//!
//! **w406.** The epoch was `(table.generation(), guest_ram_pins.len())` — *both our own
//! state*. The sweep exists to DISCOVER guest page-table changes and fold them into our
//! table, and the gate skipped the sweep whenever our table had not changed: the guest
//! writes new PTEs ⇒ our table is unchanged ⇒ the epoch is unchanged ⇒ the sweep is skipped
//! ⇒ we never decode them ⇒ our table stays unchanged. A self-fulfilling skip. Fixed by
//! folding in `ReachShadow::witness_writes`, a GUEST-side term.
//!
//! **w785.** The guest-side term counts writes **we observed**, and the single store removed
//! both transports that could observe one — the framebuffer stopped being a trapped window,
//! and the store cannot enumerate frames for the executor witness. `[measured w784]`
//! `PT-DECODE drained=0` **x831, every window of the boot**. So the same loop returned one
//! layer down, the user's address space held four rows and no UVM mapping at all, and
//! `GR0_PBDMA0` faulted. Fixed by folding in `guest_invalidates` — the guest DECLARING the
//! change rather than us watching it.
//!
//! ⚠ **Both bugs cost a rented GPU, a guest boot and a day.** Neither needed one: every
//! statement below is about a pure function over a `Vas`, and would have gone red the moment
//! the transport went dark. That is the whole argument for this file existing.
//!
//! > **Owner, 2026-09-19:** *"It seems to me a lot of them can be found by adding tests to
//! > the project that don't need a gpu to run."*
//!
//! # ★ What is asserted, and what deliberately is NOT
//!
//! The epoch is an OPAQUE COMPARABLE. These tests assert only the two properties the gate
//! actually relies on — *it moves when the guest changes something*, and *it does not move
//! when nothing changed* — and never its representation. ⊘ An assertion on the hash's shape
//! would pin `^` and `rotate_left` and would have to be edited by the next person who adds a
//! term, which is exactly the edit that must stay cheap.

use kayfabe_arch::ids::{GpuId, HClient, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, Vas};
use kayfabe_mocks::{MockIsolateFactory, WireClassArch};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const GPU: GpuId = GpuId::ZERO;
const A_CLIENT: HClient = HClient(0xc1d0_000a);
const PDB: Pdb = Pdb(0x20_0000);

/// A `Gpu` holding one compute process's address space, built through the **real**
/// construction path.
///
/// ⊘ Built with `Scenario` rather than by poking a `Vas` into a map: a fixture that
/// constructed the thing under test by hand would keep passing if the real path stopped
/// producing that shape, which is the `a_green_test_can_hold_a_wall_in_place` failure.
fn one_vas() -> Guarded<Gpu> {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu = Gpu::new(
        std::sync::Arc::new(WireClassArch::new()),
        Box::new(factory),
        gpa,
    )
    .expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(A_CLIENT, PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    Guarded::new("publish_epoch_gate::one_vas", gpu, rec)
}

/// The one address space, mutably. Panics rather than returning an `Option`: a fixture that
/// silently produced no VAS would make every assertion below vacuously true.
fn vas(gpu: &mut Gpu) -> &mut Vas {
    let pid = *gpu
        .spine
        .by_pdb
        .get(&(GPU, PDB))
        .expect("the scenario's process is routed by its PDB");
    gpu.procs
        .get_mut(&pid)
        .expect("the routed proc is live")
        .vas_by_pdb_mut(GPU, PDB)
        .expect("the process declared exactly one address space")
}

fn epoch(gpu: &mut Gpu) -> (u64, usize) {
    vas(gpu).publish_epoch()
}

/// ★★★ **THE w406 PROPERTY.** A witnessed guest page-table write moves the epoch even though
/// our own table has not caught up — which is what breaks the self-fulfilling skip.
#[test]
fn a_witnessed_guest_write_moves_the_epoch_before_our_table_changes() {
    let mut g = one_vas();
    let gpu = &mut *g;
    let before = epoch(gpu);
    vas(gpu).reach.witness(0x1000);
    let after = epoch(gpu);
    assert_ne!(
        before, after,
        "★ w406: the guest wrote a page table and our table has not changed. If the epoch \
         does not move here the sweep is skipped, the write is never decoded, and the table \
         never changes — the skip proves its own precondition"
    );
}

/// ★★★ **AND A RE-WRITE OF AN ALREADY-WITNESSED PAGE MUST MOVE IT TOO.**
///
/// ⊘ This is why `witness_writes` is a COUNT and not `witnessed.len()`. A page table being
/// rewritten in place is the common shape, and a set's length does not change for it — the
/// gate would skip exactly the case that matters.
#[test]
fn rewriting_an_already_witnessed_page_still_moves_the_epoch() {
    let mut g = one_vas();
    let gpu = &mut *g;
    vas(gpu).reach.witness(0x1000);
    let once = epoch(gpu);
    vas(gpu).reach.witness(0x1000);
    let twice = epoch(gpu);
    assert_ne!(
        once, twice,
        "★ the same page written twice is two changes. A `len()`-based term reports one, and \
         the second write is then invisible to every publication pass"
    );
}

/// ★★★★★ **THE w785 PROPERTY, AND THE ONE THAT WOULD HAVE SAVED THE DAY.**
///
/// With **no witness transport at all** — the single store's world, where the guest's
/// framebuffer stores do not trap and the store cannot enumerate frames — a declared
/// invalidate must still move the epoch.
///
/// ⊘ The simulation is exact rather than approximate: the test simply never calls
/// `witness`, which is precisely what `PT-DECODE drained=0 x831` means.
#[test]
fn with_no_witness_at_all_a_declared_invalidate_still_moves_the_epoch() {
    let mut g = one_vas();
    let gpu = &mut *g;
    let before = epoch(gpu);
    vas(gpu).note_guest_invalidate();
    let after = epoch(gpu);
    assert_ne!(
        before, after,
        "★★★★★ w785: THE GATE MUST NOT DEPEND ON A TRANSPORT THE ARCHITECTURE CAN REMOVE. \
         A TLB invalidate is the guest DECLARING its tables changed, not us observing it — \
         so it cannot go dark the way a witness can. With this red, the single store's \
         publication pass replays its last census forever and hardware translates nothing"
    );
}

/// ★★ **Repeated invalidates each move it.** An invalidate arriving while a pass is in
/// flight must not be swallowed by the pass that is already running.
#[test]
fn every_declared_invalidate_moves_the_epoch_again() {
    let mut g = one_vas();
    let gpu = &mut *g;
    let mut seen = vec![epoch(gpu)];
    for _ in 0..4 {
        vas(gpu).note_guest_invalidate();
        seen.push(epoch(gpu));
    }
    let mut uniq = seen.clone();
    uniq.sort_unstable();
    uniq.dedup();
    assert_eq!(
        uniq.len(),
        seen.len(),
        "★ five distinct states, or an invalidate is being lost: {seen:?}"
    );
}

/// ⊘ **THE NEGATIVE CONTROL, and without it every test above passes on a counter.**
///
/// A gate whose epoch moved on *every* read would also satisfy all four assertions above
/// while skipping nothing and republishing the whole table on every doorbell. The gate has
/// to be stable when nothing happened, or it is not a gate.
#[test]
fn an_unchanged_vas_does_not_move_the_epoch() {
    let mut g = one_vas();
    let gpu = &mut *g;
    assert_eq!(
        epoch(gpu),
        epoch(gpu),
        "⊘ nothing happened between these two reads. An epoch that moves here makes the \
         dirty gate a no-op and every pass a full republication"
    );
}

/// ★★★ **THE TWO TERMS ARE INDEPENDENT**, which is the property that makes folding both
/// worth doing rather than swapping one for the other.
///
/// A boot where the witness is dark must still gate on invalidates, and a boot where
/// invalidates are rare must still gate on the witness. If one term could mask the other,
/// the surviving signal would be silently unreachable.
#[test]
fn neither_term_can_mask_the_other() {
    let mut ga = one_vas();
    let a = &mut *ga;
    vas(a).reach.witness(0x1000);
    let witness_only = epoch(a);

    let mut gb = one_vas();
    let b = &mut *gb;
    vas(b).note_guest_invalidate();
    let invalidate_only = epoch(b);

    let mut gc = one_vas();
    let c = &mut *gc;
    vas(c).reach.witness(0x1000);
    vas(c).note_guest_invalidate();
    let both = epoch(c);

    let mut gbase = one_vas();
    let base = epoch(&mut *gbase);
    for (name, e) in [
        ("witness only", witness_only),
        ("invalidate only", invalidate_only),
        ("both", both),
    ] {
        assert_ne!(base, e, "★ {name} must be distinguishable from an idle VAS");
    }
    assert_ne!(
        witness_only, both,
        "★ an invalidate on top of a witnessed write is a further change"
    );
    assert_ne!(
        invalidate_only, both,
        "★ a witnessed write on top of an invalidate is a further change"
    );
}
