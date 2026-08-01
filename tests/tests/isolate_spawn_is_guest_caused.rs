//! ★★★ **E0b — the isolate spawns because the GUEST acted, not because the device was
//! realized** (`docs/design/execution_plane_increments.md` §3.6), and **E1 — a refusing
//! isolate is visible by NAME** (§2, row E1).
//!
//! ## Why this file exists, and what it is NOT
//!
//! E0 shipped the isolate-plane selector and proved on real hardware that
//! `KAYFABE_ISOLATES=real` spawns a capability-less child holding an RM-served
//! `/dev/nvidia0` mapping. It did **not** meet its own bar. `[measured]` rev `e10a6bf`,
//! runs `e0real2`/`e0real3` on RTX 3060 / 580.159.04 open: the child's first sighting was
//! **t+3 s** and the guest opened the device at **t+30–34 s**, because `Gpu::realize`
//! installed the system proc's isolate unconditionally. The verbs were caused by the
//! **device path**, which is weaker than "caused by the guest".
//!
//! ⊘ **Nothing in this file can settle that claim, and it does not try to.** These tests
//! observe an in-process counter written by the code under test; they can say *whether* a
//! spawn happened at a given point in the event sequence and never *why*. The attributing
//! instrument is `scripts/bench/e0_isolate_witness.sh`, which stamps host `/proc`
//! sightings against `boot_capture.sh`'s own phase lines — a timeline neither the device
//! nor this suite writes. What these tests are for is **drift**: they turn a future edit
//! that puts the spawn back at realize-time red, on every CI run, without a GPU.
//!
//! ## ★★ Multi-process, stated up front (owner ruling, 2026-08-01)
//!
//! The execution plane must be multi-process-capable **as built**. E0b does not touch that
//! seam and must not be read as having simplified it: the *system* proc is the guest
//! **kernel**'s objects and is singular by construction (`Gpu::SYSTEM_PROC`,
//! `SYSTEM_ANCHOR`, a reserved client `RmGraph::apply` refuses as guest input), while every
//! guest **process** gets its own `(Proc, GpuId)` isolate through `Spine::refresh` step 3b,
//! keyed on the projection's `ProcAnchor` — the **anchor client**, which is one of the
//! three keys `proc_is_not_a_set_of_rm_clients` measured as correct (never a raw pid: two
//! concurrent CUDA processes share one dup-DST client). `two_guest_processes_...` below
//! quantifies over that: three isolates, from three distinct events, none at realize.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped PDB literals, as `sim_14_two_process`

use kayfabe_arch::ids::{GpuId, HClient, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_isolate::RefusalKind;
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_tests::{Scenario, identical_handles};

const A_PDB: Pdb = Pdb(0x3401_000);
const B_PDB: Pdb = Pdb(0x3405_000);

/// The shipped composition root's own posture: an object model with **no forwarding
/// plane**, i.e. `StillbornIsolates`. Used where the assertion is about the *census*
/// rather than about a working isolate, because that is what master ships.
fn stillborn_gpu(why: &'static str) -> Gpu {
    Gpu::new(
        Box::new(MockArch::new()),
        Box::new(kayfabe_isolate::StillbornIsolates::new(why)),
        GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000),
    )
    .expect("device realizes")
}

fn mock_gpu() -> (
    Gpu,
    std::sync::Arc<std::sync::Mutex<kayfabe_mocks::RmRecorder>>,
) {
    let (factory, recorder) = MockIsolateFactory::new();
    let gpu = Gpu::new(
        Box::new(MockArch::new()),
        Box::new(factory),
        GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000),
    )
    .expect("device realizes");
    (gpu, recorder)
}

// =====================================================================================
// E0b
// =====================================================================================

/// ★★★ **THE E0b PROPERTY, in the one place a CI runner can hold it.**
///
/// Before E0b this was `materialized == 1` and `live == 1` — a host process, and under
/// `KAYFABE_ISOLATES=real` a chain of real host RM ioctls, before the guest had executed
/// one instruction. A future edit that restores the eager spawn turns this red.
#[test]
fn realizing_a_device_materializes_no_isolate_at_all() {
    let (gpu, recorder) = mock_gpu();
    let c = gpu.isolate_census();
    assert_eq!(c.materialized, 0, "realize must spawn NOTHING (E0b)");
    assert_eq!(c.live, 0, "and hold nothing");
    // ⊘ Asserted against the FACTORY's own record as well as the device's counter. The
    // counter is written by the code under test; the recorder is written by the factory,
    // which is a different object, so a counter that stopped counting cannot make this
    // pass on its own.
    assert!(
        recorder.lock().expect("recorder").client_locks.is_empty(),
        "and the factory was never asked"
    );
    // ★ The arena, by contrast, IS carved at realize — that is the half E0b deliberately
    // did NOT move, so that `GpuError::Gpa` stays a realize-time refusal.
    assert!(
        gpu.system.arenas.contains_key(&GpuId::ZERO),
        "the system proc's GPA arena is still realize-time"
    );
}

/// The first **accepted** guest RM event is what materializes the guest kernel's isolate.
#[test]
fn the_first_accepted_guest_event_materializes_the_system_isolate() {
    let (mut gpu, recorder) = mock_gpu();
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    let mut events = s.events.into_iter();
    let first = events.next().expect("a scenario has events");

    assert_eq!(gpu.isolate_census().materialized, 0);
    gpu.apply(first).expect("the first event applies");
    let c = gpu.isolate_census();
    assert_eq!(
        c.materialized, 1,
        "one accepted guest event materializes the guest kernel's isolate"
    );
    assert!(
        gpu.system.isolates.contains_key(&GpuId::ZERO),
        "and it is the SYSTEM proc's — the guest kernel's own objects"
    );
    assert_eq!(
        recorder.lock().expect("recorder").client_locks.len(),
        1,
        "the factory was asked exactly once"
    );

    // Idempotent: the remaining events of the same process add the process's OWN isolate
    // and no second system one.
    for ev in events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    assert_eq!(
        gpu.isolate_census().materialized,
        2,
        "the system proc's, plus ONE for the guest process"
    );
}

/// ⊘ **A refused event buys the guest nothing** — not even a sandbox.
///
/// `Spine::apply` is transactional: a refused event moves no state. It must not be the one
/// thing that spawns a host process either, or a guest could conjure one out of garbage.
/// ★ The non-vacuity half matters as much as the assertion: the same event applied to a
/// device that has *not* seen it refused DOES spawn, so this is not measuring an event
/// nothing ever accepts.
#[test]
fn a_refused_event_materializes_nothing() {
    let (mut gpu, _rec) = mock_gpu();
    // A `Free` of a handle that was never allocated: the graph refuses it, and the refusal
    // is raised before anything derived is touched.
    let ev = kayfabe_core::rmgraph::RmEvent::Free {
        client: HClient(0xDEAD),
        handle: kayfabe_arch::ids::HObject(0xBEEF),
    };
    let refused = gpu.apply(ev);
    assert!(refused.is_err(), "the fixture must really be refused");
    assert_eq!(
        gpu.isolate_census().materialized,
        0,
        "a refused event spawns nothing"
    );

    // Non-vacuity: an ACCEPTED event on the same device does spawn.
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    gpu.apply(s.events.into_iter().next().expect("events"))
        .expect("accepted");
    assert_eq!(gpu.isolate_census().materialized, 1);
}

/// ★★ **Two guest processes, two isolates, and neither of them realize-time.**
///
/// The owner ruling of 2026-08-01 — *"yes no rewrite for multi process"* — means the plane
/// must be multi-process-capable as built, so E0b's laziness has to hold **per process**
/// and not just once. `sim_14_two_process.rs` already asserts the two procs get distinct
/// isolate sessions; this asserts the thing E0b adds: that all three spawns are consequences
/// of applied events and the count at realize is zero.
#[test]
fn two_guest_processes_get_their_own_isolates_and_none_exists_at_realize() {
    let (mut gpu, recorder) = mock_gpu();
    assert_eq!(gpu.isolate_census().materialized, 0);

    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    s.compute_process(HClient(0xBB), B_PDB, identical_handles(0x20, 0x21));
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }

    assert_eq!(gpu.procs.len(), 2, "one Proc per guest process");
    let c = gpu.isolate_census();
    assert_eq!(
        c.materialized, 3,
        "the guest kernel's isolate plus ONE PER GUEST PROCESS — never one shared"
    );
    assert_eq!(c.live, 3);

    // ★ Distinct sessions, quantified over the procs rather than over a written-out list.
    let mut ids: Vec<_> = gpu
        .procs
        .values()
        .map(|p| p.isolates[&GpuId::ZERO].id())
        .collect();
    ids.push(gpu.system.isolates[&GpuId::ZERO].id());
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "three DISTINCT isolate sessions");
    assert_eq!(
        recorder.lock().expect("recorder").client_locks.len(),
        3,
        "and the factory — a different object from the counter — was asked three times"
    );
}

// =====================================================================================
// E1
// =====================================================================================

/// ★★★ **E1 at the census: the shipped default is visible BY NAME, and it is not a
/// failure.**
///
/// `StillbornIsolates` is what master installs, and it must report
/// [`RefusalKind::NoPlane`] — never `SpawnFailed`. An operator who reads "spawn-failed" on
/// a build that deliberately has no forwarding plane will go and debug their host.
#[test]
fn the_shipped_default_plane_reports_no_plane_and_never_a_failure() {
    const WHY: &str = "this build has no forwarding plane: the object model accepts \
                       protocol facts and no host verb can be issued";
    let mut gpu = stillborn_gpu(WHY);
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }

    let c = gpu.isolate_census();
    assert_eq!(c.materialized, 2, "the guest caused both spawns");
    assert_eq!(c.refusing(), 2, "and every one of them refuses");
    assert_eq!(c.no_plane, 2);
    assert_eq!(
        c.spawn_failed, 0,
        "⊘ a deliberately plane-less build must never read as a host failure"
    );
    let (kind, why) = c.first.expect("a refusal carries its sentence");
    assert_eq!(kind, RefusalKind::NoPlane);
    assert_eq!(
        why, WHY,
        "verbatim — this is the composition root's sentence"
    );
}

/// A device whose isolates all work reports **nothing** to investigate — the control that
/// stops the assertion above from passing on a census that always reports something.
#[test]
fn a_working_plane_reports_no_refusal_at_all() {
    let (mut gpu, _rec) = mock_gpu();
    let mut s = Scenario::new();
    s.compute_process(HClient(0xAA), A_PDB, identical_handles(0x10, 0x11));
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    let c = gpu.isolate_census();
    assert_eq!(c.live, 2, "the isolates exist…");
    assert_eq!(c.refusing(), 0, "…and none of them refuses");
    assert!(c.first.is_none());
}

/// ★★ **`SpawnFailed` outranks `NoPlane` in the one line a report has room for**, in both
/// observation orders — a precedence that held only in the order it was written in would be
/// the kind of "passes because of the fixture" this suite is not allowed to have.
///
/// ⊘ Driven through the REAL implementors and never through `MockIsolate`, whose `refusal`
/// is `None` by construction: a mock that answered here would be the mock deciding E1's
/// question. The `SpawnFailed` producer is
/// `kayfabe_isolate_host::HostIsolate`, exercised in that crate's own unit tests
/// (`a_failed_host_isolate_is_distinguishable_from_a_deliberately_planeless_one`) because
/// its constructor is private on purpose. Here the fold's ordering rule is what is under
/// test, so it is driven with the two kinds directly.
#[test]
fn a_spawn_failure_outranks_a_missing_plane_in_either_order() {
    use kayfabe_isolate::{IsolateCensus, IsolateRefusal};

    fn folded(order: [(RefusalKind, &'static str); 2]) -> (RefusalKind, String) {
        let mut c = IsolateCensus::default();
        for (kind, why) in order {
            // A minimal stand-in for one isolate's answer: the fold's input IS
            // `Option<IsolateRefusal>`, so this drives exactly the seam.
            c.observe_refusal(Some(IsolateRefusal { kind, why }));
        }
        c.first.expect("something refused")
    }

    let a = folded([
        (RefusalKind::NoPlane, "no plane"),
        (RefusalKind::SpawnFailed, "clone: EPERM"),
    ]);
    let b = folded([
        (RefusalKind::SpawnFailed, "clone: EPERM"),
        (RefusalKind::NoPlane, "no plane"),
    ]);
    assert_eq!(a, b, "the fold must not depend on observation order");
    assert_eq!(a.0, RefusalKind::SpawnFailed);
    assert_eq!(a.1, "clone: EPERM");
}
