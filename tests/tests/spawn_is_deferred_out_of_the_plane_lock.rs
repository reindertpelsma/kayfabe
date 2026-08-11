//! ★★★★★ **w239 — the isolate spawn no longer runs beneath the register plane's mutex.**
//!
//! `[measured 2026-08-11, §16.88, `traces/boots/w237/`]` the guest's first `GSP_RM_ALLOC`
//! reached `ObjectModel::apply` **fifteen frames and six crates** inside `RegPlane::write`,
//! which holds `LockRank::Plane` (rank 0), and `clone`+`execveat`ed a sandboxed child there.
//! QEMU aborted on the R1 assert.
//!
//! # ⊘ The fix is a PULL, and these tests are about the two halves of it
//!
//! [`SharedDevice::apply_deferring`] decides the spawn and **leaves it latched in the spine**;
//! [`SharedDevice::materialize_pending`] runs it. The shim calls the second one from
//! `Regs::write`, after `RegPlane::write` has returned and the plane's guard is a dropped
//! local. ⊘ Nothing travels through `CommandPolicy`, so no `kayfabe-core` vocabulary crosses
//! the `kayfabe-gsp` port (§16.90's blocker, dissolved rather than paid).

use kayfabe_arch::ids::{HClient, HObject, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
use kayfabe_mocks::{MockArch, MockIsolateFactory, mock_classes as mc};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_rt::lock::{LockRank, RankedRwLock};
use kayfabe_tests::Scenario;

const CLIENT: HClient = HClient(0xC0B_0000);
const PDB0: Pdb = Pdb(0x4002_0000);

/// A device plus the events that make the spine decide an isolate is needed.
fn device_and_events() -> (SharedDevice, Vec<RmEvent>) {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");

    let root = HObject(0xC0B_0000);
    let dev = HObject(0xC0B_0001);
    let vas = HObject(0xC0B_0010);
    let mut s = Scenario::new();
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: root,
        handle: root,
        class: mc::CLIENT,
        facts: kayfabe_tests::user_client(CLIENT),
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: root,
        handle: dev,
        class: mc::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: dev,
        handle: vas,
        class: mc::VASPACE,
        facts: AllocFacts::default(),
    });
    s.push(RmEvent::SetPageDir {
        client: CLIENT,
        vaspace: vas,
        pdb: PDB0,
    });
    (SharedDevice::new(gpu, LockMode::Sharded), s.events)
}

/// ★★★★★ **THE FALSIFIER — `apply_deferring` under an outer lock must NOT spawn.**
///
/// A rank-0 guard is held for the whole call, exactly as `RegPlane::write` holds the plane's.
/// ⊘ Before this rung the equivalent call was `apply`, and it spawned: on the bench that is
/// the `R1 … rank(s) [0]` abort, and here it is a panic inside this closure.
///
/// ★ The assertion is that the call **completes**, which is only meaningful because
/// `IsolateBox::new` asserts lock-freedom and would panic if a spawn happened. ⊘ Stated
/// explicitly because "it returned Ok" is otherwise a weak claim: the strength comes from the
/// production assert being armed underneath, not from the `Ok`.
#[test]
fn apply_deferring_does_not_spawn_while_an_outer_lock_is_held() {
    let (dev, events) = device_and_events();
    let outer = RankedRwLock::new(LockRank::Plane, 0u32);

    let _held = outer.read();
    for ev in events {
        dev.apply_deferring(ev)
            .expect("the scenario applies cleanly");
    }
    // Still holding rank 0 here: reaching this line at all is the measurement.
    assert_ne!(
        kayfabe_util::lockwitness::held_mask(),
        0,
        "★ the outer lock must STILL be held, or this test proves nothing about spawning \
         beneath one — a guard dropped early would make the whole body vacuous"
    );
}

/// ★★★ **THE KNOWN-POSITIVE — the drain, lock-free, actually runs the spawn.**
///
/// ⊘ Without this, `apply_deferring` could simply have lost the spawn and the falsifier above
/// would still pass. A latch nobody drains is a hang, which is the failure
/// `FwdFault::IsolatePending` exists to prevent — so "did not spawn under the lock" is only
/// half a fix, and this is the other half.
#[test]
fn the_lock_free_drain_runs_what_was_latched() {
    let (dev, events) = device_and_events();
    {
        let outer = RankedRwLock::new(LockRank::Plane, 0u32);
        let _held = outer.read();
        for ev in events {
            dev.apply_deferring(ev).expect("applies");
        }
    } // ← every guard dropped, as `Regs::write` drops the plane's before draining

    assert_eq!(
        kayfabe_util::lockwitness::held_mask(),
        0,
        "★ the drain's precondition, asserted rather than assumed"
    );
    dev.materialize_pending();
    let census = dev.isolate_census();
    assert!(
        census.materialized >= 1,
        "★★★ the drain must actually MATERIALIZE — a fix that only stops spawning under the \
         lock, and never spawns at all, turns an abort into a hang. Census: {census:?}"
    );
}

/// ★★ **`materialize_pending` is idempotent and cheap when nothing is latched.**
///
/// ⊘ It runs on **every** register write, including the overwhelming majority that decide no
/// spawn, so "costs nothing when empty" is a property the hot path depends on — and a drain
/// that spawned something on an empty latch would be worse than the bug.
#[test]
fn draining_an_empty_latch_is_a_no_op() {
    let (dev, _events) = device_and_events();
    let before = dev.isolate_census().materialized;
    dev.materialize_pending();
    dev.materialize_pending();
    assert_eq!(
        dev.isolate_census().materialized,
        before,
        "★ an empty drain must not conjure an isolate"
    );
}
