//! ★★★★★ **§16.96 — the engine-object forward no longer issues a host RM verb beneath the
//! register plane's mutex.**
//!
//! `[measured 2026-08-11, §16.91, `traces/boots/w239/`]` the guest's first engine-object
//! `GSP_RM_ALLOC` reaches `ObjectModel::forward_engine_object` **six crates inside
//! `RegPlane::write`**, which holds `LockRank::Plane` (rank 0), and issues a host RM ioctl
//! there. QEMU aborts on the R1 assert:
//!
//! ```text
//! R1 no-blocking-under-lock violation: issuing a host RM verb while holding rank(s) [0]
//! ```
//!
//! # ⊘ Why this is a LATCH and not the relocation §16.91 said it had to be
//!
//! §16.91 read the signature — `forward_engine_object` returns
//! `Result<EngineObjectForwarded, FwdFault>` — and concluded *"its result IS the answer, so
//! it can only be relocated, never latched"*. ⊘ **Refuted at the call site.**
//! `kayfabe_rmrpc::Bridge::deliver` **discards** that result on purpose and says so:
//! *"the guest's answer does NOT change, and that is a decision, not an oversight"* —
//! turning a host refusal into an alloc failure would fail `cuCtxCreate` outright, and a
//! boot measuring the forward would silently be measuring that instead.
//!
//! ⇒ **nothing in the guest's reply depends on this verb**, so §16.91's own rule — *work
//! decided under a lock can be deferred only if nothing in the response depends on it* —
//! **admits** it. ★ A signature bounds what a function *can* return; only the call site says
//! what is *read*.
//!
//! # The two halves, and both are tested here
//!
//! [`SharedDevice::forward_engine_object_deferring`] admits the request and **latches** it;
//! [`SharedDevice::run_pending_engine_forwards`] runs it. The shim calls the second from
//! `Regs::write`, after `RegPlane::write` has returned and the plane's guard is a dropped
//! local — the same frame, in the same position, as §16.91's spawn drain.

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_arch::ids::{ClassId, EngineKind, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_mocks::{MockArch, MockIsolateFactory, RmVerb, SharedRecorder, mock_classes as mc};
use kayfabe_rt::ForwardAdmission;
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_rt::lock::{LockRank, RankedRwLock};
use kayfabe_tests::{Scenario, identical_handles};

const CLIENT: HClient = HClient(0xAA);
const PDB: Pdb = Pdb(0x3401_000);
const GR_VCHID: VChid = VChid(0x10);
const CE_VCHID: VChid = VChid(0x11);

/// The GR channel `Scenario::compute_process` declares — the `hParent` an engine-object
/// alloc carries.
const GR_CHANNEL: HObject = HObject(0x5c00_0019);

/// ★ The guest's own params bytes, distinctive so that "the host got *something*" and "the
/// host got **these**" are different assertions.
const PARAMS: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04];

/// A device with a full compute process declared and its isolate materialized, plus the
/// recorder that sees every host verb.
///
/// ⊘ The graph is applied through `apply_deferring` + `materialize_pending`, i.e. exactly the
/// §16.91 path the shim uses, so this fixture cannot pass by taking a route production does
/// not take.
fn armed_device() -> (SharedDevice, SharedRecorder) {
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("the device realizes");
    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB, identical_handles(GR_VCHID.0, CE_VCHID.0));

    let dev = SharedDevice::new(gpu, LockMode::Sharded);
    for ev in s.events {
        dev.apply_deferring(ev)
            .expect("the scenario applies cleanly");
    }
    dev.materialize_pending();
    (dev, recorder)
}

/// Every `AllocEngineObject` the host was asked for, as `(class, params)`.
fn host_engine_objects(rec: &SharedRecorder) -> Vec<(ClassId, Vec<u8>)> {
    rec.lock()
        .expect("recorder")
        .log
        .iter()
        .filter_map(|(_iso, v)| match v {
            RmVerb::AllocEngineObject { class, params, .. } => Some((*class, params.clone())),
            _ => None,
        })
        .collect()
}

// =================================================================================
// ★★★★★ THE FALSIFIER
// =================================================================================

/// ★★★★★ **THE FALSIFIER — admitting a forward under an outer rank-0 lock must issue NO
/// host verb.**
///
/// A rank-0 guard is held for the whole call, exactly as `RegPlane::write` holds the plane's.
/// ⊘ Before this rung the equivalent call was `forward_engine_object_by_parent`, and it went
/// straight to `Worker::execute`: on the bench that is the `R1 … rank(s) [0]` abort, and here
/// it is a panic inside this test.
///
/// ★★ **The recorder is the measurement, not the return value.** *"It returned `Latched`"* is
/// a claim about our own bookkeeping; *"the host was asked for nothing"* is a claim about the
/// boundary, and only the second one is what the bench aborted over
/// (`measure_at_the_boundary_not_inside`).
#[test]
fn admitting_a_forward_under_the_plane_lock_issues_no_host_verb() {
    let (dev, rec) = armed_device();
    let before = host_engine_objects(&rec).len();

    let outer = RankedRwLock::new(LockRank::Plane, 0u32);
    let _held = outer.read();

    let admitted = dev.forward_engine_object_deferring(CLIENT, GR_CHANNEL, mc::COMPUTE, PARAMS);
    assert_eq!(
        admitted,
        ForwardAdmission::Latched { pending: 1 },
        "★ the request must be ADMITTED and latched, not served and not dropped"
    );
    // Still holding rank 0: reaching this line at all is half the measurement, because
    // `Worker::execute` asserts lock-freedom and would have panicked above.
    assert_ne!(
        kayfabe_util::lockwitness::held_mask(),
        0,
        "★ the outer lock must STILL be held, or this test proves nothing about issuing a \
         verb beneath one — a guard dropped early would make the whole body vacuous"
    );
    assert_eq!(
        host_engine_objects(&rec).len(),
        before,
        "★★★ the HOST was asked for an engine object while rank 0 was held. That is the \
         bench abort, and the return value above cannot see it."
    );
}

// =================================================================================
// ★★★ THE KNOWN-POSITIVE — without it, "never forwards at all" also passes
// =================================================================================

/// ★★★ **THE KNOWN-POSITIVE — the drain, lock-free, actually runs the latched forward.**
///
/// ⊘ Without this, `forward_engine_object_deferring` could simply have *lost* the request and
/// the falsifier above would still pass. A latch nobody drains is not a fix: it turns a loud
/// abort into a host context that silently never exists, which is precisely the *"capability
/// that exists, is tested, and is never called"* shape this tree keeps paying for.
#[test]
fn the_lock_free_drain_runs_what_was_latched() {
    let (dev, rec) = armed_device();
    {
        let outer = RankedRwLock::new(LockRank::Plane, 0u32);
        let _held = outer.read();
        assert_eq!(
            dev.forward_engine_object_deferring(CLIENT, GR_CHANNEL, mc::COMPUTE, PARAMS),
            ForwardAdmission::Latched { pending: 1 },
        );
    } // ← every guard dropped, as `Regs::write` drops the plane's before draining

    assert_eq!(
        kayfabe_util::lockwitness::held_mask(),
        0,
        "★ the drain's precondition, asserted rather than assumed"
    );
    let runs = dev.run_pending_engine_forwards(&[]);
    assert_eq!(runs.len(), 1, "★ one request in, one row out");
    let forwarded = runs[0]
        .out
        .as_ref()
        .expect("★★★ the drain must FORWARD, not merely dequeue");
    assert_eq!(forwarded.engine, EngineKind::GrCompute);
    assert!(
        forwarded.materialized_channel,
        "the first forward materializes the host channel"
    );

    // ★★ And the boundary, again: what the HOST was actually asked for.
    let objs = host_engine_objects(&rec);
    assert_eq!(
        objs,
        vec![(mc::COMPUTE, PARAMS.to_vec())],
        "★★★ the host must see exactly one engine-object alloc, of the guest's class, \
         carrying THE GUEST'S OWN PARAMS BYTES. ⊘ A latch that copied the request but not \
         its bytes would hand the host an empty slice and every other assertion here would \
         still pass — `the_hosts_params_are_the_guests_own_bytes_not_an_empty_slice`, one \
         deferral later."
    );
}

// =================================================================================
// ⊘ THE NEGATIVE CONTROLS — each watched to FAIL if the mechanism were louder
// =================================================================================

/// ⊘ **NEGATIVE CONTROL: a drain with nothing latched must do nothing at all.**
///
/// It runs on **every** register write, including the overwhelming majority that decide no
/// forward, so "costs nothing and conjures nothing when empty" is a property the hot path
/// depends on. ★ A drain that forwarded something here would be strictly worse than the bug.
#[test]
fn draining_an_empty_latch_is_a_no_op() {
    let (dev, rec) = armed_device();
    assert!(dev.run_pending_engine_forwards(&[]).is_empty());
    assert!(dev.run_pending_engine_forwards(&[]).is_empty());
    assert!(
        host_engine_objects(&rec).is_empty(),
        "★ an empty drain must not conjure a host engine object"
    );
}

/// ⊘ **NEGATIVE CONTROL: a non-engine class is never latched, and never runs.**
///
/// The class gate runs first, for `kayfabe_fwd::route_engine_object_by_parent`'s own
/// `[measured 2026-08-10]` reason — a client root refused as *"your parent is not a channel"*
/// is a **true sentence about the wrong question** — and here it has a second job: it is what
/// keeps the latch bounded in practice, because clients, devices, memory and VA spaces are
/// the overwhelming majority of allocs and none of them may occupy a slot.
#[test]
fn a_non_engine_class_is_refused_at_the_gate_and_never_latched() {
    let (dev, rec) = armed_device();
    let outer = RankedRwLock::new(LockRank::Plane, 0u32);
    let _held = outer.read();

    for class in [
        mc::MEMORY,
        mc::CLIENT,
        mc::DEVICE,
        mc::VASPACE,
        ClassId(0xdead),
    ] {
        assert_eq!(
            dev.forward_engine_object_deferring(CLIENT, GR_CHANNEL, class, PARAMS),
            ForwardAdmission::NotAnEngine(class),
            "★ class {class:?} is not an engine object and must exit at the gate"
        );
    }
    drop(_held);

    assert!(
        dev.run_pending_engine_forwards(&[]).is_empty(),
        "★★ the gate must consume no slot: five non-engine allocs latched NOTHING"
    );
    assert!(host_engine_objects(&rec).is_empty());
}

// =================================================================================
// ★★ THE BOUND — the latch refuses by name rather than growing
// =================================================================================

/// ★★ **The latch is BOUNDED and refuses BY NAME.**
///
/// ⊘ The observed population is **one**, because this guest is synchronous under the GPU lock
/// — and `kayfabe-gsp/src/boot.rs:1291-1294` says in as many words that this is *"a property
/// of the guest, not of the protocol"*, citing `cap1b`'s real queue-full at txn 1028 as the
/// same assumption failing elsewhere. ⇒ the bound is not defensive decoration; it is the
/// difference between a named refusal and an unbounded `Vec` fed by guest traffic.
///
/// ★ The refusal is checked to be **exact** — `pending` and `bound`, not merely "an error" —
/// because a bound that cannot say how full it was is a wall that carries no name.
#[test]
fn the_latch_refuses_by_name_at_its_bound() {
    let (dev, rec) = armed_device();
    let outer = RankedRwLock::new(LockRank::Plane, 0u32);
    let _held = outer.read();

    let bound = kayfabe_rt::MAX_PENDING_ENGINE_FORWARDS;
    for i in 1..=bound {
        assert_eq!(
            dev.forward_engine_object_deferring(CLIENT, GR_CHANNEL, mc::COMPUTE, PARAMS),
            ForwardAdmission::Latched { pending: i },
            "★ the {i}th request must be admitted and must COUNT itself"
        );
    }
    for _ in 0..3 {
        assert_eq!(
            dev.forward_engine_object_deferring(CLIENT, GR_CHANNEL, mc::COMPUTE, PARAMS),
            ForwardAdmission::LatchFull {
                pending: bound,
                bound
            },
            "★★ past the bound the latch must REFUSE BY NAME — never grow, never silently \
             drop, and never merely 'fail'"
        );
    }
    drop(_held);

    assert!(
        host_engine_objects(&rec).is_empty(),
        "★ non-vacuity in the other direction: filling the latch to its bound must still \
         have issued ZERO host verbs, because the whole point is that admission is not \
         service"
    );
    assert_eq!(
        dev.run_pending_engine_forwards(&[]).len(),
        bound,
        "★ the drain runs exactly what was admitted — the three refusals are not in it"
    );
}
