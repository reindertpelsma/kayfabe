//! ★ L1-M1 stage 3 — the **plan / execute / commit** verb seam, R1's teeth at the
//! trait boundary, the bounded worker pool, and the R5 staleness canaries
//! (`l1_concurrency.md` §3.3 R1/R5, §3.5 the #37 intra-proc invariant, §7.2/§7.3 the
//! pool, §11 B5/B6).
//!
//! What this file pins, per test:
//!
//! - **`r1_is_asserted_at_the_host_verb_itself_not_at_a_wrapper`** — the §12.6 gap,
//!   closed: invoking a host RM verb through the ONE door (`Worker::execute`) with
//!   ANY ranked lock held panics naming R1, and the legal shape (checked-out worker,
//!   no guards) runs. This is the assert covering the thing it names.
//! - **★ `progress_under_pending_verb_intra_proc`** — the #37 invariant itself. ONE
//!   proc, TWO threads: thread A's verb is held pending by the mock while thread B of
//!   the **same proc** completes an independent op end to end. Asserted as
//!   **progress**, never wall-clock — no sleeps, no thresholds.
//! - **`poll_and_delivery_need_no_worker_at_full_saturation`** — §3.5 guarantee 3:
//!   with EVERY worker of the pool held pending, a completion observation, a
//!   completion poll and a pump all still make progress.
//! - **`r5_canary_proc_retired_in_the_gap`** / **`..._channel_torn_down_in_the_gap`**
//!   / **`..._apply_rewrote_routing_in_the_gap`** — one canary per §8.4 case: the
//!   script mutates the world *while the verb is held pending*; on release the commit
//!   must refuse loudly and mutate nothing.
//! - **`pool_full_is_backpressure_not_a_hang`** — a saturated pool makes the next
//!   requester wait (holding zero locks — `BlockingSection` inside the wait asserts
//!   it) and complete after a release.
//! - **`worker_death_retires_the_proc_loudly_and_never_resurrects`** — a worker HUP
//!   dispatches through the reactor, kills the slot, retires the proc and CONDEMNS its
//!   component (§12.13); its completions die with it, and a later `refresh` does not
//!   bring it back.
//! - **`single_in_flight_per_worker_is_structural`** — one `&mut`-owned handle per
//!   slot: a checked-out worker cannot be checked out again, and concurrency comes
//!   from channel COUNT (§11 B6) — there is no shared in-flight slot table to grow.
//!
//! Section 9 (`commit_*_proc_guard[s]_refuse_on_either_term_alone`,
//! `plan_publish_refuses_when_either_half_of_the_target_is_missing`,
//! `a_refused_commit_releases_its_orphans_on_the_single_threaded_path`,
//! `a_refused_op_returns_its_worker_to_the_pool`,
//! `the_shell_ring_gate_refuses_an_unpublished_working_set`,
//! `worker_death_kills_its_own_pool_slot_not_merely_the_proc`) splits the same R5 /
//! disposition machinery **term by term**, driving the plan/commit pair directly so
//! exactly one clause of each guard is true per case. Every one of them closes a
//! survivor of the first L1 mutation campaign — see `core_mutation_gate.md`
//! §L1 baseline, and the per-test doc comments, which name the mutant they kill.
//!
//! Every test arms a [`watchdog`] (the `concurrency_stress.rs` bounded-termination
//! rule) and joins every thread it spawns.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant as WallInstant};

use kayfabe_arch::ids::{ControlCmd, GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_completion::OsEventRef;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::reactor::SourceKind;
use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
use kayfabe_core::{ProcAnchor, ProcId};
use kayfabe_fwd::{FwdFault, Stale};
use kayfabe_isolate::{
    DEFAULT_POOL_WORKERS, HostHandle, IsolateFactory, IsolateId, VerbPlan, WorkerId,
};
use kayfabe_mocks::{
    HoldSpec, MockArch, MockIsolateFactory, RmVerb, SharedRecorder, VerbHold, VerbKind,
};
use kayfabe_rt::device::{LockMode, SharedDevice, SignalOutcome};
use kayfabe_rt::executor::{Effect, Executor};
use kayfabe_rt::inbox::{CoreEvent, inbox};
use kayfabe_rt::lock::{LockRank, RankedMutex};
use kayfabe_tests::{Scenario, identical_handles};
use kayfabe_util::Instant;

// ---------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------

/// Abort the process loudly if the guard is not dropped within `limit` — bounded
/// termination, so a regression that wedges this suite fails fast instead of eating
/// the CI timeout.
///
/// `KAYFABE_STRESS_WATCHDOG_SECS` overrides `limit`, as in `rt_shell`/`l1_mean`: under
/// ThreadSanitizer (§8.2's race ceiling) everything runs 10-20x slower, so the normal
/// limits would abort on the sanitizer's overhead rather than on a real wedge.
#[must_use]
fn watchdog(test: &'static str, limit: Duration) -> WatchdogGuard {
    let limit = match std::env::var("KAYFABE_STRESS_WATCHDOG_SECS") {
        Ok(s) => Duration::from_secs(s.parse().expect("KAYFABE_STRESS_WATCHDOG_SECS: seconds")),
        Err(_) => limit,
    };
    let done = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&done);
    thread::spawn(move || {
        let deadline = WallInstant::now() + limit;
        while WallInstant::now() < deadline {
            if flag.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        if !flag.load(Ordering::Relaxed) {
            eprintln!("WATCHDOG: {test} still running after {limit:?} — aborting the process");
            std::process::abort();
        }
    });
    WatchdogGuard(done)
}

struct WatchdogGuard(Arc<AtomicBool>);
impl Drop for WatchdogGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

const GPU: GpuId = GpuId::ZERO;
const CLIENT: HClient = HClient(0xA0);
const CLIENT2: HClient = HClient(0xA1);
const PDB: Pdb = Pdb(0x3400_0000);
const PDB2: Pdb = Pdb(0x3500_0000);
const GR: VChid = VChid(0x100);
const CE: VChid = VChid(0x200);
const GR2: VChid = VChid(0x101);
const CE2: VChid = VChid(0x201);
const MEM: HObject = HObject(0x6000_0000);
const VA: GpuVa = GpuVa(0x2_0020_0000);

/// One guest proc (optionally two) on GPU0, with `pool` workers per isolate.
fn device_with(
    procs: usize,
    pool: usize,
    mode: LockMode,
) -> (Arc<SharedDevice>, Vec<ProcId>, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::with_pool_size(pool);
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");

    let mut s = Scenario::new();
    for i in 0..procs {
        let (client, pdb, gr, ce) = if i == 0 {
            (CLIENT, PDB, GR, CE)
        } else {
            (CLIENT2, PDB2, GR2, CE2)
        };
        s.compute_process_on_gpu(client, pdb, identical_handles(gr.0, ce.0), None);
        s.memory(
            client,
            HObject(0x5c00_0001),
            MEM,
            0x9_0000_0000 + (i as u64) * 0x1000_0000,
        );
    }
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    let pids: Vec<ProcId> = (0..procs)
        .map(|i| gpu.spine.by_pdb[&(GPU, if i == 0 { PDB } else { PDB2 })])
        .collect();
    (Arc::new(SharedDevice::new(gpu, mode)), pids, recorder)
}

/// Arm a one-shot hold on `verb` of `(proc, worker)`'s isolate.
fn hold(rec: &SharedRecorder, pid: ProcId, worker: u32, verb: VerbKind) -> Arc<VerbHold> {
    rec.lock().expect("recorder").hold(HoldSpec::exact(
        IsolateId(pid.0),
        GPU,
        WorkerId(worker),
        verb,
    ))
}

// ---------------------------------------------------------------------------------
// 1 — R1 at the boundary (the §12.6 gap, closed)
// ---------------------------------------------------------------------------------

/// ★ Invoking a host RM verb through the ONE door with a ranked lock held panics
/// **naming R1** — the assert now covers the trait boundary itself, not a wrapper a
/// caller must remember to use. Reverting stage 3 (calling the backend from inside a
/// locked act phase) reproduces exactly this panic.
#[test]
#[should_panic(expected = "R1 no-blocking-under-lock violation")]
fn r1_is_asserted_at_the_host_verb_itself_not_at_a_wrapper() {
    let (mut factory, _rec) = MockIsolateFactory::new();
    let mut iso = factory.spawn(IsolateId(1), GPU);
    let mut worker = iso.checkout().expect("fresh pool");
    let proc_lock = RankedMutex::new(LockRank::Proc, ());

    let _guard = proc_lock.lock(); // a locked act phase, exactly as stage 2 had it
    let _ = worker.execute(&VerbPlan::Publish {
        host_vas: None,
        len: 0x1000,
    });
}

/// R1's success polarity, and the *ownership* half of the enforcement: a worker is
/// obtained by being **moved out** of the pool, so the natural call site holds no
/// guard — and the verb chain then runs clean.
#[test]
fn r1_legal_path_checked_out_worker_with_no_guards_runs() {
    let _wd = watchdog("r1_legal_path", Duration::from_secs(30));
    let (mut factory, rec) = MockIsolateFactory::new();
    let mut iso = factory.spawn(IsolateId(1), GPU);
    let mut worker = iso.checkout().expect("fresh pool");
    assert_eq!(kayfabe_rt::lock::held_depth(), 0, "no guard is alive here");

    let reply = worker
        .execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
        })
        .expect("the chain runs");
    match reply {
        kayfabe_isolate::VerbReply::Published { host_vas, .. } => {
            assert!(
                host_vas.is_some(),
                "the chain allocated the host VAS itself"
            );
        }
        other => panic!("wrong reply: {other:?}"),
    }
    // The chain threaded its OWN intermediates: vas → mem → map, no core access.
    let log = rec.lock().expect("recorder");
    let kinds: Vec<&'static str> = log
        .log
        .iter()
        .map(|(_, v)| match v {
            kayfabe_mocks::RmVerb::AllocVaSpace { .. } => "vas",
            kayfabe_mocks::RmVerb::AllocSysmem { .. } => "mem",
            kayfabe_mocks::RmVerb::MapGpuVa { .. } => "map",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["vas", "mem", "map"]);
}

// ---------------------------------------------------------------------------------
// 2 — ★ THE #37 INVARIANT: progress under a pending verb, INTRA-PROC
// ---------------------------------------------------------------------------------

/// ★ **The invariant this whole stage exists for** (`l1_concurrency.md` §3.5): a
/// blocking GPU-work verb issued by guest thread A must not stall guest thread B *of
/// the same process*.
///
/// ONE proc (a multi-threaded guest process is ONE `Proc`), TWO threads. Thread A's
/// `alloc_sysmem` is held pending by the mock. While it is held — proved by
/// `VerbHold::is_pending()`, an **edge**, not a clock — thread B of the same proc
/// completes an independent publish AND a doorbell AND a completion poll, end to end.
/// Then A is released and its commit lands correctly.
///
/// If R1 regressed (the verb ran under the proc lock), B's very first op would block
/// on that lock and this test would hang into its watchdog. If the pool regressed to
/// one worker per isolate (#34's shape), B's verb would queue behind A's on the wire
/// and the same thing would happen.
#[test]
fn progress_under_pending_verb_intra_proc() {
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let _wd = watchdog(
            "progress_under_pending_verb_intra_proc",
            Duration::from_secs(60),
        );
        let (device, pids, rec) = device_with(1, DEFAULT_POOL_WORKERS, mode);
        let pid = pids[0];

        // A's verb parks inside the backend on worker 0 (the first checkout).
        let held = hold(&rec, pid, 0, VerbKind::AllocSysmem);

        let a_device = Arc::clone(&device);
        let a = thread::spawn(move || a_device.publish_backing(GPU, PDB, VA, 0x1000));

        // PROGRESS EDGE, not a sleep: block until A's verb genuinely entered.
        held.wait_until_pending();
        assert!(held.is_pending(), "A's verb is parked in the host backend");

        // ---- B: the SAME proc, a different thread, entirely independent work ----
        let b_device = Arc::clone(&device);
        let b = thread::spawn(move || {
            let published = b_device
                .publish_backing(GPU, PDB, GpuVa(VA.0 + 0x10_0000), 0x1000)
                .expect("B publishes while A's verb is pending");
            let rung = b_device
                .doorbell(GPU, MockArch::token_for(CE), &[])
                .expect("B rings while A's verb is pending");
            // The poll path needs no worker at all (§3.5 guarantee 3).
            b_device.completion_poll(GPU, pid, Instant(1));
            (published, rung)
        });
        let (b_pub, b_ring) = b.join().expect("B made progress");

        // ★ The assertion: B finished WHILE A was still pending. No wall clock.
        assert!(
            held.is_pending(),
            "B completed only after A's verb was released — the #37 invariant is violated"
        );
        assert_eq!(b_ring.proc, pid, "B's doorbell demuxed to its own proc");
        assert!(b_pub.host_va != 0);

        // ---- release A and check its commit landed ----
        held.release();
        let a_pub = a
            .join()
            .expect("A's thread joins")
            .expect("A's publish commits");
        let (binding, off) = device
            .resolve(GPU, PDB, GpuVa(VA.0 + 0x40))
            .expect("A's VA resolves after its commit");
        assert_eq!(
            (binding.phys, binding.host_va(), off),
            (a_pub.gpa, Some(a_pub.host_va), 0x40),
            "({mode:?}) A's commit wrote the binding it computed, not B's"
        );
        // Two publishes, two DISTINCT host VAs in the SAME per-Vas host VAS.
        assert_ne!(a_pub.host_va, b_pub.host_va);
        assert_ne!(a_pub.gpa, b_pub.gpa);
    }
}

// ---------------------------------------------------------------------------------
// 3 — the poll/completion path needs no worker (§3.5 guarantee 3)
// ---------------------------------------------------------------------------------

/// With **every** worker of the pool held pending, the completion plane still makes
/// progress: an os-event source dispatches and observes, the owner's poll re-posts,
/// and the pump composes a batch. Sync progress never queues behind pool saturation
/// — which is why the guarantee holds even at pool size 1.
#[test]
fn poll_and_delivery_need_no_worker_at_full_saturation() {
    let _wd = watchdog("poll_and_delivery_need_no_worker", Duration::from_secs(60));
    const POOL: usize = 2;
    let (device, pids, rec) = device_with(1, POOL, LockMode::Sharded);
    let pid = pids[0];

    let holds: Vec<Arc<VerbHold>> = (0..POOL as u32)
        .map(|w| hold(&rec, pid, w, VerbKind::AllocSysmem))
        .collect();
    let threads: Vec<_> = (0..POOL)
        .map(|k| {
            let d = Arc::clone(&device);
            thread::spawn(move || {
                d.publish_backing(GPU, PDB, GpuVa(VA.0 + (k as u64) * 0x10_0000), 0x1000)
            })
        })
        .collect();
    for h in &holds {
        h.wait_until_pending();
    }
    assert!(
        holds.iter().all(|h| h.is_pending()),
        "every worker in the pool is parked in a host verb"
    );

    // ---- the completion plane, with ZERO workers available ----
    let (tx, rx) = inbox();
    let mut ex = Executor::new(Arc::clone(&device), rx);
    let ev = OsEventRef(0xBEEF);
    let src = device.register_source(SourceKind::OsEvent {
        proc: pid,
        gpu: GPU,
        ev,
    });
    tx.send(CoreEvent::SourceSignal(src));
    assert_eq!(
        ex.drain_all(),
        vec![Effect::Signal(SignalOutcome::Observed {
            proc: pid,
            gpu: GPU,
            ev
        })],
        "a completion is observed with the whole pool saturated"
    );
    let posted = device
        .completion_poll(GPU, pid, Instant(1))
        .expect("the owner's own poll re-posts with the pool saturated");
    assert!(
        !posted.events.is_empty(),
        "the poll delivered the completion"
    );

    for h in &holds {
        h.release();
    }
    for t in threads {
        t.join().expect("joins").expect("publishes commit");
    }
}

// ---------------------------------------------------------------------------------
// 4 — ★ THE R5 STALENESS CANARIES (§8.4, §11 B5)
// ---------------------------------------------------------------------------------

/// Run `mutate` while proc 0's `alloc_sysmem` is held pending, then release and
/// return what the publish commit did. The shared body of the three canaries.
fn canary(
    mutate: impl FnOnce(&SharedDevice, ProcId),
) -> (
    Result<kayfabe_fwd::Published, FwdFault>,
    Arc<SharedDevice>,
    ProcId,
) {
    let (device, pids, rec) = device_with(1, DEFAULT_POOL_WORKERS, LockMode::Sharded);
    let pid = pids[0];
    let held = hold(&rec, pid, 0, VerbKind::AllocSysmem);

    let d = Arc::clone(&device);
    let t = thread::spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
    held.wait_until_pending();

    mutate(&device, pid); // ← the world moves INSIDE the lock-free gap

    held.release();
    let out = t.join().expect("the publishing thread joins");
    (out, device, pid)
}

/// **Canary (a): the proc retired in the gap.** The commit must refuse — never write
/// into dead state — and the refusal must name staleness, not some incidental error.
#[test]
fn r5_canary_proc_retired_in_the_gap_refuses_loudly() {
    let _wd = watchdog("r5_canary_proc_retired", Duration::from_secs(60));
    let (out, device, pid) = canary(|device, pid| {
        assert!(
            device.retire_proc(pid),
            "the proc was live when we retired it"
        );
    });
    assert_eq!(
        out,
        Err(FwdFault::Stale(Stale::Proc(pid))),
        "a commit whose proc vanished must refuse, not finish what it started"
    );
    // Nothing was written anywhere: the proc is off the live set entirely, so even
    // the read path refuses. ★ `retire_proc` is the OUT-OF-BAND edge, so the refusal
    // is `Condemned` (§12.13): the component is dead for good, not merely absent from
    // the live set for the moment. See `worker_death_retires_the_proc_loudly_and_never
    // _resurrects` for the full statement of that rule.
    assert_eq!(
        device.resolve(GPU, PDB, VA),
        Err(FwdFault::Condemned {
            anchor: ProcAnchor(CLIENT)
        }),
        "the retired proc is off the live set; no binding was created for it"
    );
}

/// **Canary (b): the routed channel was torn down in the gap.** Freeing the channel
/// rebuilds routing without it, so the doorbell commit re-resolves to nothing and
/// refuses. (Run on the doorbell site — the channel is *its* commit target.)
#[test]
fn r5_canary_channel_torn_down_in_the_gap_refuses_loudly() {
    let _wd = watchdog("r5_canary_channel_torn_down", Duration::from_secs(60));
    let (device, pids, rec) = device_with(1, DEFAULT_POOL_WORKERS, LockMode::Sharded);
    let pid = pids[0];
    // The doorbell's first touch allocates the host channel — hold THAT verb.
    let held = hold(&rec, pid, 0, VerbKind::AllocChannel);

    let d = Arc::clone(&device);
    let t = thread::spawn(move || d.doorbell(GPU, MockArch::token_for(GR), &[]));
    held.wait_until_pending();

    // ← the guest tears the channel down while the alloc is in flight
    device
        .apply(RmEvent::Free {
            client: CLIENT,
            handle: identical_handles(GR.0, CE.0).gr_channel,
        })
        .expect("the free applies");

    held.release();
    let out = t.join().expect("joins");
    assert_eq!(
        out,
        Err(FwdFault::Stale(Stale::Route {
            gpu: GPU,
            vchid: GR
        })),
        "a commit whose channel was torn down must refuse (use-after-retire, designed out)"
    );
    // Mutation-free: the channel is gone, and its vChid routes nowhere.
    assert_eq!(
        device.doorbell(GPU, MockArch::token_for(GR), &[]),
        Err(FwdFault::UnknownVchid {
            gpu: GPU,
            vchid: GR
        }),
        "no resurrected channel, no half-written host state"
    );
    // ★ R5's DISPOSITION half (`Orphans`): the refusal above happened *after* the
    // execute phase had already allocated a host channel (and the VAS under it), so
    // those objects are orphaned — the commit adopted nothing. They must be released
    // on the same worker, still lock-free, before it is checked back in. Counted as
    // host `free` verbs, because "we refused" and "we refused and leaked" are
    // otherwise indistinguishable from the outside: the whole `Orphans::is_empty`
    // gate can be short-circuited to "nothing to do" without a single other
    // assertion in this file changing colour (the campaign found exactly that).
    let frees: Vec<_> = rec
        .lock()
        .expect("recorder")
        .verbs_of(IsolateId(pid.0))
        .into_iter()
        .filter_map(|v| match v {
            RmVerb::Free { obj } => Some(obj),
            _ => None,
        })
        .collect();
    let allocated: Vec<_> = rec
        .lock()
        .expect("recorder")
        .verbs_of(IsolateId(pid.0))
        .into_iter()
        .filter_map(|v| match v {
            RmVerb::AllocChannel { handle, .. } | RmVerb::AllocVaSpace { handle } => Some(handle),
            _ => None,
        })
        .collect();
    assert!(
        !allocated.is_empty(),
        "non-vacuity: the held verb DID allocate host state to orphan"
    );
    // Released in REVERSE allocation order — child (the channel) before the parent
    // (the VAS it was allocated on), which is the only order an RM namespace accepts.
    let expected: Vec<_> = allocated.iter().rev().copied().collect();
    assert_eq!(
        frees, expected,
        "every host object the refused commit orphaned was released — exactly once, \
         child before parent, and nothing else was freed"
    );
    assert_eq!(pid, pids[0]);
}

/// **Canary (c): an `apply` rewrote routing in the gap.** The channel is freed and
/// **re-allocated at the same vChid**, so `by_vchid` resolves again — but to a
/// DIFFERENT `ChanId`. A commit that trusted its pre-gap decision would write the
/// host channel into the wrong (or a stale) channel; re-resolving through IDs makes
/// it refuse.
#[test]
fn r5_canary_apply_rewrote_routing_in_the_gap_refuses_loudly() {
    let _wd = watchdog("r5_canary_routing_rewrite", Duration::from_secs(60));
    let (device, pids, rec) = device_with(1, DEFAULT_POOL_WORKERS, LockMode::Sharded);
    let pid = pids[0];
    let h = identical_handles(GR.0, CE.0);
    let held = hold(&rec, pid, 0, VerbKind::AllocChannel);

    let d = Arc::clone(&device);
    let t = thread::spawn(move || d.doorbell(GPU, MockArch::token_for(GR), &[]));
    held.wait_until_pending();

    // ← free + re-alloc at the SAME vChid: routing resolves, to a new identity.
    device
        .apply(RmEvent::Free {
            client: CLIENT,
            handle: h.gr_channel,
        })
        .expect("free applies");
    device
        .apply(RmEvent::Alloc {
            client: CLIENT,
            parent: h.tsg,
            handle: HObject(0x5c00_00f9),
            class: kayfabe_mocks::mock_classes::CHANNEL_GR,
            facts: AllocFacts {
                h_vaspace: Some(h.vaspace),
                userd_flags: MockArch::userd_flags_for(GR),
                ..Default::default()
            },
        })
        .expect("re-alloc applies");

    held.release();
    let out = t.join().expect("joins");
    assert_eq!(
        out,
        Err(FwdFault::Stale(Stale::Route {
            gpu: GPU,
            vchid: GR
        })),
        "the route was rewritten under the verb: the commit must re-resolve and refuse"
    );
    // The NEW channel is untouched — no host channel was written into it.
    let fresh = device
        .doorbell(GPU, MockArch::token_for(GR), &[])
        .expect("the re-allocated channel routes and materializes cleanly");
    assert_eq!(fresh.proc, pid);
    assert!(
        fresh.scheduled_now,
        "the refused commit left NO scheduled/materialized residue on the new channel"
    );
}

// ---------------------------------------------------------------------------------
// 5 — pool-full backpressure
// ---------------------------------------------------------------------------------

/// A saturated pool is **backpressure, not a hang** (§7.2): the next requester
/// releases ALL locks, waits, and completes once a worker comes back.
///
/// The "holds no lock while waiting" half is asserted by construction: the waiter
/// opens a [`kayfabe_rt::lock::BlockingSection`] before parking, which panics unless
/// the thread holds zero ranked locks — so B finishing successfully IS the proof. The
/// ordering half is progress-based, read off the verb log: B's verbs land strictly
/// after A's, on the SAME single worker.
#[test]
fn pool_full_is_backpressure_not_a_hang() {
    let _wd = watchdog("pool_full_is_backpressure", Duration::from_secs(60));
    let (device, pids, rec) = device_with(1, 1, LockMode::Sharded); // pool of ONE
    let pid = pids[0];
    let held = hold(&rec, pid, 0, VerbKind::AllocSysmem);

    let d = Arc::clone(&device);
    let a = thread::spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
    held.wait_until_pending(); // A owns the only worker

    let d = Arc::clone(&device);
    let b = thread::spawn(move || d.publish_backing(GPU, PDB, GpuVa(VA.0 + 0x10_0000), 0x1000));

    // B cannot proceed: no worker exists. Release A and require BOTH to complete.
    held.release();
    let a_out = a.join().expect("joins").expect("A commits");
    let b_out = b
        .join()
        .expect("joins")
        .expect("B commits after waiting for a worker");
    assert_ne!(a_out.host_va, b_out.host_va);

    // Progress ordering: on a one-worker pool, B's map necessarily follows A's.
    let log = rec.lock().expect("recorder");
    let maps: Vec<u64> = log
        .log
        .iter()
        .filter_map(|(_, v)| match v {
            kayfabe_mocks::RmVerb::MapGpuVa { va, .. } => Some(*va),
            _ => None,
        })
        .collect();
    assert_eq!(
        maps,
        vec![a_out.host_va, b_out.host_va],
        "the single worker served A then B — serialized on the wire, never deadlocked"
    );
}

// ---------------------------------------------------------------------------------
// 6 — worker death
// ---------------------------------------------------------------------------------

/// A worker HUP dispatches through the reactor to **retire the proc loudly**: the
/// slot is dead forever (never respawned — a worker that died mid-verb may have left
/// host state the core cannot reason about), the proc leaves the live set, its
/// completion sources stop routing, and its completions die with it.
#[test]
fn worker_death_retires_the_proc_loudly_and_never_resurrects() {
    let _wd = watchdog("worker_death_retires", Duration::from_secs(60));
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (device, pids, _rec) = device_with(2, DEFAULT_POOL_WORKERS, mode);
        let (victim, bystander) = (pids[0], pids[1]);

        // The victim has live host state and a live completion source.
        device
            .publish_backing(GPU, PDB, VA, 0x1000)
            .expect("victim publishes");
        let os = device.register_source(SourceKind::OsEvent {
            proc: victim,
            gpu: GPU,
            ev: OsEventRef(0x11),
        });
        let hup = device.register_source(SourceKind::Worker {
            proc: victim,
            gpu: GPU,
            worker: WorkerId(0),
        });

        let (tx, rx) = inbox();
        let mut ex = Executor::new(Arc::clone(&device), rx);
        tx.send(CoreEvent::SourceSignal(hup));
        assert_eq!(
            ex.drain_all(),
            vec![Effect::Signal(SignalOutcome::WorkerDied {
                proc: victim,
                gpu: GPU,
                worker: WorkerId(0)
            })],
            "({mode:?}) the HUP dispatched as a typed worker death"
        );

        // The proc is retired and its component CONDEMNED: every op on it refuses,
        // loudly, with the fault that says so (§12.13).
        let condemned = FwdFault::Condemned {
            anchor: ProcAnchor(CLIENT),
        };
        assert_eq!(
            device.publish_backing(GPU, PDB, GpuVa(VA.0 + 0x1000), 0x1000),
            Err(condemned),
            "({mode:?}) the retired proc refuses every op, loudly"
        );
        // RESOLVED (`l1_concurrency.md` §12.11 → §12.13): §12.11 recorded that an
        // out-of-band retire left `by_pdb`/`by_vchid` NAMING the dead proc until the
        // next graph `refresh`, so the two teardown routes were not fault-identical,
        // and asked whether a host-side failure should retroactively edit the guest's
        // routing truth. It should not — and does not: the guest's truth (the RM graph
        // and its projection) is untouched. What `retire_proc` edits is the host-side
        // *materialization*, moving this proc's routing into the condemned maps at the
        // instant of the failure. So the fault is the same one here as after any number
        // of later refreshes, which is what `out_of_band_retire_must_not_resurrect_the
        // _isolate` (l1_mean.rs) pins from the other end.
        assert_eq!(
            device.doorbell(GPU, MockArch::token_for(GR), &[]),
            Err(condemned),
            "({mode:?}) and its channels are condemned with it"
        );
        // Its completions die with it — the os-event source no longer resolves.
        tx.send(CoreEvent::SourceSignal(os));
        match ex.drain_all().as_slice() {
            [Effect::Signal(SignalOutcome::Fault(_))] => {}
            other => panic!("({mode:?}) a dead proc's source must fault, got {other:?}"),
        }
        // No resurrect: it reaps once and is gone.
        assert_eq!(device.reap_retired(), 1, "({mode:?}) reaped exactly once");
        assert_eq!(device.reap_retired(), 0, "({mode:?}) and never comes back");
        // ★ …and it stays gone across a `refresh` driven by ANOTHER client. This test
        // used to stop one line above, which is exactly why it stayed green through
        // §12.13's defect while the composed mean run caught it: the resurrection needed
        // an `apply` to happen, and this script never issued one. It does now.
        device
            .apply(RmEvent::MapMemoryDma {
                client: CLIENT2,
                vaspace: identical_handles(GR2.0, CE2.0).vaspace,
                memory: MEM,
                va: GpuVa(0x80_0000_0000),
                offset: 0,
                len: 0x1000,
            })
            .expect("an unrelated map applies");
        assert_eq!(
            device.publish_backing(GPU, PDB, GpuVa(VA.0 + 0x2000), 0x1000),
            Err(condemned),
            "({mode:?}) ★ a refresh must not resurrect an out-of-band-retired proc"
        );

        // The BYSTANDER proc is untouched — blast-radius containment.
        device
            .publish_backing(GPU, PDB2, VA, 0x1000)
            .expect("({mode:?}) the other proc is unaffected");
        assert_eq!(
            device
                .doorbell(GPU, MockArch::token_for(GR2), &[])
                .expect("bystander rings")
                .proc,
            bystander
        );
    }
}

// ---------------------------------------------------------------------------------
// 7 — single-in-flight per worker, structurally
// ---------------------------------------------------------------------------------

/// Single-in-flight is the **borrow checker's** guarantee, N times over (§7.2/§11
/// B6): a slot's handle is `&mut`-owned by exactly one thread while it is out, so a
/// second checkout of the same slot is unrepresentable — `checkout` can only hand
/// back a *different* idle slot, and when none is left it says so.
///
/// The property this pins negatively is just as important: concurrency comes from
/// channel COUNT, never from multiplexing one channel. There is no shared in-flight
/// slot table here to grow a txn demux on — a pool of N is N independent 1-deep
/// channels, and that is the whole mechanism.
#[test]
fn single_in_flight_per_worker_is_structural() {
    let _wd = watchdog("single_in_flight_per_worker", Duration::from_secs(30));
    const POOL: usize = 3;
    let (mut factory, _rec) = MockIsolateFactory::with_pool_size(POOL);
    let mut iso = factory.spawn(IsolateId(7), GPU);
    assert_eq!(iso.pool_size(), POOL);
    assert_eq!(iso.idle_workers(), POOL);

    let mut out = Vec::new();
    for k in 0..POOL {
        let w = iso.checkout().expect("an idle slot remains");
        assert_eq!(iso.idle_workers(), POOL - 1 - k);
        out.push(w);
    }
    // Every checked-out handle is a DISTINCT slot: no slot is ever handed out twice.
    let ids: std::collections::BTreeSet<WorkerId> = out.iter().map(|w| w.id()).collect();
    assert_eq!(ids.len(), POOL, "each checkout moved a distinct slot out");
    assert!(
        iso.checkout().is_none(),
        "a saturated pool refuses rather than multiplexing an in-flight worker"
    );

    // Returning one makes exactly one available again.
    let back = out.pop().expect("held");
    let back_id = back.id();
    iso.checkin(back);
    assert_eq!(iso.idle_workers(), 1);
    assert_eq!(
        iso.checkout().expect("the returned slot").id(),
        back_id,
        "the pool hands back the slot that was returned, not a new identity"
    );

    // A retiring isolate refuses NEW checkouts (§5.4) — never a silent success.
    for w in out {
        iso.checkin(w);
    }
    iso.retire();
    assert!(
        iso.checkout().is_none(),
        "a retiring isolate refuses new work"
    );
}

// ---------------------------------------------------------------------------------
// 8 — the plan phase emits, it does not call
// ---------------------------------------------------------------------------------

/// An idempotent engine-object re-send resolves **entirely in the plan phase**: no
/// verbs, and therefore **no checkout at all**, so a replay-heavy guest cannot
/// consume pool capacity. Pins the `verbs: None` path of the seam.
#[test]
fn idempotent_replay_emits_no_verbs_and_takes_no_worker() {
    let _wd = watchdog("idempotent_replay_takes_no_worker", Duration::from_secs(30));
    let (device, pids, rec) = device_with(1, 1, LockMode::Sharded); // pool of ONE
    let pid = pids[0];
    let first = device
        .forward_engine_object(GPU, GR, kayfabe_tests::COMPUTE_CLASS, &[])
        .expect("first forward allocs");
    assert!(!first.reused);

    // Hold the ONLY worker with an unrelated op, then replay: if the replay tried to
    // check a worker out it would block here forever (and hit the watchdog).
    let held = hold(&rec, pid, 0, VerbKind::AllocSysmem);
    let d = Arc::clone(&device);
    let t = thread::spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
    held.wait_until_pending();

    let replay = device
        .forward_engine_object(GPU, GR, kayfabe_tests::COMPUTE_CLASS, &[])
        .expect("the replay resolves from core state with the pool saturated");
    assert!(replay.reused, "a re-send is idempotent");
    assert_eq!(replay.host_object, first.host_object, "the ORIGINAL object");

    held.release();
    t.join().expect("joins").expect("publishes");
}

// ---------------------------------------------------------------------------------
// 9 — ★ THE R5 GUARDS, TERM BY TERM (§11 B5, §12.10)
// ---------------------------------------------------------------------------------
//
// Section 4's canaries drive the R5 sites through the *whole* device, which proves
// the guards are wired up but exercises each `A || B` staleness test with BOTH terms
// true at once. §11 B5's warning is that a forgotten re-validation is "quieter than
// the deadlock it replaced", and the first L1 mutation campaign made the consequence
// concrete: `commit_engine_object`/`commit_control`'s `proc.is_retired() || proc.id
// != plan.proc` survived being narrowed to `&&` (mutants `lib.rs:1285:26` and
// `lib.rs:1468:26`), as did `plan_publish`'s two-target check (`lib.rs:485:40`).
//
// These tests split each guard into its independent terms and drive the plan/commit
// pair DIRECTLY, so exactly one term is true per case. Every assertion names the
// exact fault variant, never `is_err()` — §12.10's lesson, learned here once already.

/// The plain `Gpu` behind a freshly built device — the single-threaded shape the
/// term-by-term guard tests need (they hand a commit a `&mut Proc` that is
/// deliberately the WRONG one, which no threaded entry point can express).
fn plain_gpu(procs: usize) -> (Gpu, Vec<ProcId>) {
    let (device, pids, _rec) = device_with(procs, DEFAULT_POOL_WORKERS, LockMode::Degenerate);
    let gpu = Arc::into_inner(device)
        .expect("the device is not shared yet")
        .into_gpu();
    (gpu, pids)
}

/// ★ `commit_engine_object`'s proc guard is a **disjunction**, and each term refuses
/// **alone**: (a) the plan's own proc retired in the gap, (b) the plan is committed
/// against a *live* proc that is not the one it was planned for.
///
/// (b) is the term no whole-device canary can reach — the shell always re-locks the
/// plan's own `ProcId` — yet it is the term that matters if `commit_phase` is ever
/// re-keyed. Committing proc A's plan into live proc B must name `Stale::Proc(A)`
/// and adopt NOTHING into B, or a narrowed guard would fall through to the route
/// check (which still resolves, because the route is A's and unchanged) and only
/// then trip on a channel miss — a *different*, much quieter fault.
#[test]
fn commit_engine_object_proc_guard_refuses_on_either_term_alone() {
    let _wd = watchdog("commit_engine_object_proc_guard", Duration::from_secs(30));

    // (b) wrong-but-live proc. A replay plan is used so the commit needs no reply and
    // allocates nothing — the guard is the ONLY thing under test.
    let (mut gpu, pids) = plain_gpu(2);
    let (a, b) = (pids[0], pids[1]);
    kayfabe_fwd::forward_engine_object(&mut gpu, GPU, GR, kayfabe_tests::COMPUTE_CLASS, &[])
        .expect("proc A materializes its channel + engine object");
    let route = kayfabe_fwd::route_engine_object(&gpu.spine, GPU, GR, kayfabe_tests::COMPUTE_CLASS)
        .expect("A's GR channel routes");
    assert_eq!(route.proc, a, "the scenario's first proc owns GR");
    let planned =
        kayfabe_fwd::plan_engine_object(&gpu.procs[&a], &route, kayfabe_tests::COMPUTE_CLASS, &[])
            .expect("the re-send plans");
    assert!(
        planned.plan.replay.is_some() && planned.verbs.is_none(),
        "the re-send must resolve as a replay, so this test isolates the guard"
    );

    let Gpu { spine, procs, .. } = &mut gpu;
    let refusal = kayfabe_fwd::commit_engine_object(
        spine,
        procs.get_mut(&b).expect("proc B is live"),
        &planned.plan,
        None,
    )
    .expect_err("A's plan must not commit into B");
    assert_eq!(
        refusal.fault,
        FwdFault::Stale(Stale::Proc(a)),
        "a plan committed against the wrong proc is STALENESS naming the plan's proc \
         — not a channel miss, not a route miss"
    );
    assert!(
        !refusal.retry,
        "a divergent identity mismatch is not re-plannable"
    );
    assert!(
        procs[&b].channels.values().all(|c| !c
            .host_engine_objects
            .contains_key(&kayfabe_tests::COMPUTE_CLASS)),
        "nothing of A's was adopted into B"
    );

    // (a) the plan's OWN proc, retired in the gap. Same plan, same commit, the other
    // term of the disjunction.
    let (mut gpu, pids) = plain_gpu(1);
    let a = pids[0];
    kayfabe_fwd::forward_engine_object(&mut gpu, GPU, GR, kayfabe_tests::COMPUTE_CLASS, &[])
        .expect("materializes");
    let route = kayfabe_fwd::route_engine_object(&gpu.spine, GPU, GR, kayfabe_tests::COMPUTE_CLASS)
        .expect("routes");
    let planned =
        kayfabe_fwd::plan_engine_object(&gpu.procs[&a], &route, kayfabe_tests::COMPUTE_CLASS, &[])
            .expect("plans");
    let Gpu { spine, procs, .. } = &mut gpu;
    let proc = procs.get_mut(&a).expect("live");
    proc.retire(); // ← the world moves inside the lock-free gap
    assert_eq!(
        proc.id, planned.plan.proc,
        "ONLY the retired term is true now"
    );
    let refusal = kayfabe_fwd::commit_engine_object(spine, proc, &planned.plan, None)
        .expect_err("a retired proc's commit must refuse");
    assert_eq!(
        refusal.fault,
        FwdFault::Stale(Stale::Proc(a)),
        "a retired proc refuses AS staleness (§12.10's polarity)"
    );
}

/// ★ The same disjunction in `commit_control`, with the write-back as the witness.
///
/// The control's host effect has already happened when the commit runs, so the only
/// thing this commit owns is copying the host's reply into the guest's buffer. A
/// narrowed guard therefore does not merely mis-report — it writes another proc's
/// host reply into a buffer on behalf of a proc the plan was never made for. The
/// buffer is asserted UNTOUCHED, which is the observable a bare `is_err()` would miss.
#[test]
fn commit_control_proc_guard_refuses_on_either_term_alone() {
    let _wd = watchdog("commit_control_proc_guard", Duration::from_secs(30));
    const CMD: ControlCmd = ControlCmd(0x2080_0110);
    const OBJ: HostHandle = HostHandle(0x0BEE_F000);

    let (mut gpu, pids) = plain_gpu(2);
    let (a, b) = (pids[0], pids[1]);
    let planned = kayfabe_fwd::plan_control(&gpu.procs[&a], GPU, OBJ, CMD, &[0; 4])
        .expect("A's control plans");

    // (b) wrong-but-live proc: B has an isolate on GPU, so a narrowed guard sails
    // past the target check and performs the write-back.
    let mut buf = [0u8; 4];
    let refusal = kayfabe_fwd::commit_control(
        gpu.procs.get_mut(&b).expect("proc B is live"),
        &planned.plan,
        Some(kayfabe_isolate::VerbReply::Control {
            payload: vec![0xAB; 4],
        }),
        &mut buf,
    )
    .expect_err("A's control must not write back through B");
    assert_eq!(
        refusal.fault,
        FwdFault::Stale(Stale::Proc(a)),
        "the refusal names the plan's proc"
    );
    assert_eq!(
        buf, [0u8; 4],
        "a refused control writes NOTHING back — the answer has nowhere to go"
    );

    // (a) the plan's own proc, retired in the gap.
    let proc = gpu.procs.get_mut(&a).expect("live");
    proc.retire();
    let refusal = kayfabe_fwd::commit_control(
        proc,
        &planned.plan,
        Some(kayfabe_isolate::VerbReply::Control {
            payload: vec![0xCD; 4],
        }),
        &mut buf,
    )
    .expect_err("a retired proc's control must refuse");
    assert_eq!(refusal.fault, FwdFault::Stale(Stale::Proc(a)));
    assert_eq!(buf, [0u8; 4], "still nothing written back");
}

/// ★ `plan_publish`'s target check is a disjunction over the proc's TWO per-target
/// containers (MG-5: an arena AND an isolate per `GpuId`), and **either half missing
/// alone** must refuse — before any host verb runs.
///
/// That ordering is the whole point of the check: finding the miss after the allocs
/// would mean allocating host state for a target we then refuse. So each half is
/// removed on its own and the plan must still be `NoTarget`; with both present the
/// same call plans successfully, which is what stops this test passing vacuously.
#[test]
fn plan_publish_refuses_when_either_half_of_the_target_is_missing() {
    let _wd = watchdog("plan_publish_target_halves", Duration::from_secs(30));
    let (mut gpu, pids) = plain_gpu(1);
    let pid = pids[0];
    // Materialize the target (arena + isolate) the way production does.
    let proc = gpu.procs.get_mut(&pid).expect("live");
    kayfabe_fwd::publish_backing(proc, GPU, PDB, VA, 0x1000).expect("A publishes");
    let expected = FwdFault::NoTarget {
        proc: pid,
        gpu: GPU,
    };

    let arena = proc
        .arenas
        .remove(&GPU)
        .expect("the target's arena existed");
    assert!(
        proc.isolates.contains_key(&GPU),
        "ONLY the arena half is missing"
    );
    assert_eq!(
        kayfabe_fwd::plan_publish(proc, GPU, PDB, GpuVa(VA.0 + 0x1000), 0x1000).map(|p| p.plan),
        Err(expected),
        "no arena for the target ⇒ NoTarget, with no host verb emitted"
    );
    proc.arenas.insert(GPU, arena);

    let iso = proc
        .isolates
        .remove(&GPU)
        .expect("the target's isolate existed");
    assert!(
        proc.arenas.contains_key(&GPU),
        "ONLY the isolate half is missing"
    );
    assert_eq!(
        kayfabe_fwd::plan_publish(proc, GPU, PDB, GpuVa(VA.0 + 0x1000), 0x1000).map(|p| p.plan),
        Err(expected),
        "no isolate for the target ⇒ NoTarget, with no host verb emitted"
    );
    proc.isolates.insert(GPU, iso);

    // Non-vacuity: with both halves back, the identical call plans.
    assert!(
        kayfabe_fwd::plan_publish(proc, GPU, PDB, GpuVa(VA.0 + 0x1000), 0x1000).is_ok(),
        "the refusals above were about the missing halves, not about the call"
    );
}

/// ★ A worker HUP kills **its own pool slot** — and the kill is observable, because
/// `retire_proc` parks the proc (isolates and all) in `spine.retired` rather than
/// dropping it.
///
/// `worker_death_retires_the_proc_loudly_and_never_resurrects` asserts everything
/// *around* this — the typed dispatch, the condemnation, the reap — but nothing that
/// distinguishes "the slot was retired, then the proc was" from "only the proc was",
/// which is why the campaign found `device.rs:868 replace SharedDevice::kill_worker
/// _slot with ()` surviving. The two steps are separate critical sections on purpose
/// (§7.3: retiring is a spine WRITE, so it cannot run under the dispatch guard), and
/// in that gap a sibling thread of the same proc can still reach the pool — so the
/// slot must already be dead when the gap opens, not merely dead by implication once
/// the proc goes. Pinned by slot count: the dead slot is gone from the pool, its
/// SIBLINGS are not.
#[test]
fn worker_death_kills_its_own_pool_slot_not_merely_the_proc() {
    let _wd = watchdog("worker_death_kills_its_slot", Duration::from_secs(30));
    const POOL: usize = 3;
    const DEAD: WorkerId = WorkerId(1);
    let (device, pids, _rec) = device_with(1, POOL, LockMode::Sharded);
    let pid = pids[0];
    device
        .publish_backing(GPU, PDB, VA, 0x1000)
        .expect("materializes the proc's isolate for GPU");

    let hup = device.register_source(SourceKind::Worker {
        proc: pid,
        gpu: GPU,
        worker: DEAD,
    });
    assert_eq!(
        device.signal_source(hup),
        SignalOutcome::WorkerDied {
            proc: pid,
            gpu: GPU,
            worker: DEAD
        },
        "the HUP dispatched as a typed worker death"
    );

    let gpu = Arc::into_inner(device)
        .expect("the device is not shared")
        .into_gpu();
    let retired = gpu
        .spine
        .retired
        .iter()
        .find(|p| p.id == pid)
        .expect("the retired proc is parked, not dropped (reaped only on demand)");
    let iso = retired
        .isolates
        .get(&GPU)
        .expect("the retired proc still carries its target isolate");
    assert_eq!(iso.pool_size(), POOL, "the pool was not resized");
    assert_eq!(
        iso.idle_workers(),
        POOL - 1,
        "the HUP'd slot is DEAD — never a respawn, and never a slot that stays \
         checkout-able for the window between the kill and the retire"
    );
}

/// ★ The SAME disjunction at the other two commit sites — `commit_publish` (the
/// data-plane materialization) and `commit_doorbell` (the ring path).
///
/// All four R5 commit guards are textually identical (`proc.is_retired() || proc.id
/// != plan.proc`), and the first ICE-free L1 campaign found the pair here surviving
/// narrowing to `&&` for the same reason as the other two: the whole-device canaries
/// only ever make BOTH terms true at once. Kept in one test because the property is
/// one property — a commit belongs to exactly one proc, and it re-checks that by
/// IDENTITY, never merely by liveness.
///
/// These two sites are the ones that hold host state at refusal time, so each case
/// also asserts the refusal hands back the orphans it could not adopt: refusing
/// *and* leaking is not refusing.
#[test]
fn commit_publish_and_doorbell_proc_guards_refuse_on_either_term_alone() {
    let _wd = watchdog("commit_publish_doorbell_guards", Duration::from_secs(30));
    const MEMORY: HostHandle = HostHandle(0x0D01_0001);
    const HOST_VA: u64 = 0x7000_0000;

    // ---- commit_publish, term (b): the plan is A's, the proc is a live B.
    let (mut gpu, pids) = plain_gpu(2);
    let (a, b) = (pids[0], pids[1]);
    {
        let proc_a = gpu.procs.get_mut(&a).expect("live");
        kayfabe_fwd::publish_backing(proc_a, GPU, PDB, VA, 0x1000).expect("A materializes");
    }
    let planned = kayfabe_fwd::plan_publish(&gpu.procs[&a], GPU, PDB, GpuVa(VA.0 + 0x1000), 0x1000)
        .expect("A plans a second publication");
    let refusal = kayfabe_fwd::commit_publish(
        gpu.procs.get_mut(&b).expect("proc B is live"),
        &planned.plan,
        Some(kayfabe_isolate::VerbReply::Published {
            host_vas: None,
            memory: MEMORY,
            host_va: HOST_VA,
        }),
    )
    .expect_err("A's publish must not commit into B");
    assert_eq!(
        refusal.fault,
        FwdFault::Stale(Stale::Proc(a)),
        "a publish committed against the wrong proc is STALENESS naming the plan's proc"
    );
    assert!(
        refusal.orphans.free.contains(&MEMORY),
        "the host memory the execute phase allocated is handed back, not leaked: {:?}",
        refusal.orphans
    );
    assert!(
        !gpu.procs[&b].vases.values().any(|v| v
            .table
            .iter()
            .any(|(_, _, bind)| bind.host_va() == Some(HOST_VA))),
        "nothing of A's was bound into B's address plane"
    );

    // ---- commit_publish, term (a): A's own plan, A retired in the gap.
    let proc_a = gpu.procs.get_mut(&a).expect("live");
    proc_a.retire();
    let refusal = kayfabe_fwd::commit_publish(
        proc_a,
        &planned.plan,
        Some(kayfabe_isolate::VerbReply::Published {
            host_vas: None,
            memory: MEMORY,
            host_va: HOST_VA,
        }),
    )
    .expect_err("a retired proc's publish must refuse");
    assert_eq!(refusal.fault, FwdFault::Stale(Stale::Proc(a)));
    assert!(refusal.orphans.free.contains(&MEMORY));

    // ---- commit_doorbell, both terms. The plan is taken AFTER a real ring, so the
    // channel is materialized and the commit needs no fresh handles.
    let (mut gpu, pids) = plain_gpu(2);
    let (a, b) = (pids[0], pids[1]);
    kayfabe_fwd::handle_doorbell(&mut gpu, GPU, MockArch::token_for(GR), &[])
        .expect("A rings once, materializing its channel");
    let route = kayfabe_fwd::route_doorbell(&gpu.spine, GPU, MockArch::token_for(GR))
        .expect("A's GR channel routes");
    assert_eq!(route.proc, a);
    let planned = kayfabe_fwd::plan_doorbell(&gpu.procs[&a], &route, &[]).expect("A plans a ring");
    let reply = || {
        Some(kayfabe_isolate::VerbReply::Doorbell {
            host_vas: None,
            channel: None,
            scheduled: false,
        })
    };

    let Gpu { spine, procs, .. } = &mut gpu;
    let refusal = kayfabe_fwd::commit_doorbell(
        spine,
        procs.get_mut(&b).expect("proc B is live"),
        &planned.plan,
        reply(),
    )
    .expect_err("A's ring must not commit into B");
    assert_eq!(
        refusal.fault,
        FwdFault::Stale(Stale::Proc(a)),
        "a ring committed against the wrong proc is STALENESS naming the plan's proc \
         — NOT the route miss a narrowed guard would fall through to"
    );

    let proc_a = procs.get_mut(&a).expect("live");
    proc_a.retire();
    let refusal = kayfabe_fwd::commit_doorbell(spine, proc_a, &planned.plan, reply())
        .expect_err("a retired proc's ring must refuse");
    assert_eq!(refusal.fault, FwdFault::Stale(Stale::Proc(a)));
}

/// ★ R5's disposition rule on the **single-threaded** composition too: a commit that
/// refuses releases what its execute phase already allocated, on the same worker,
/// before checking it back in.
///
/// `r5_canary_channel_torn_down_in_the_gap_refuses_loudly` pins this for the threaded
/// shell (`SharedDevice::verb_op`); `round_trip` is the other, textually separate
/// call site, and the campaign found `lib.rs:403:16 delete !` surviving there — i.e.
/// "release only when there is nothing to release", the exact leak the rule exists to
/// prevent.
///
/// Driven through a **re-publication of a VA that is already bound**: the plan phase
/// legitimately cannot know the bind will collide (a sibling could have unbound it),
/// so the sysmem alloc and the host map both run first and only the commit refuses —
/// a refusal that owns host state, reached with no threads at all.
#[test]
fn a_refused_commit_releases_its_orphans_on_the_single_threaded_path() {
    let _wd = watchdog("round_trip_orphan_release", Duration::from_secs(30));
    let (device, pids, rec) = device_with(1, DEFAULT_POOL_WORKERS, LockMode::Degenerate);
    let pid = pids[0];
    let mut gpu = Arc::into_inner(device).expect("not shared").into_gpu();
    let proc = gpu.procs.get_mut(&pid).expect("live");

    kayfabe_fwd::publish_backing(proc, GPU, PDB, VA, 0x1000).expect("the first publication binds");
    assert_eq!(
        kayfabe_fwd::publish_backing(proc, GPU, PDB, VA, 0x1000),
        Err(FwdFault::Address(kayfabe_mmu::AddressFault::Overlap {
            pdb: PDB,
            va: VA
        })),
        "re-publishing a bound VA is a loud overlap — never a silent rebind"
    );

    let verbs = rec.lock().expect("recorder").verbs_of(IsolateId(pid.0));
    let allocated: Vec<_> = verbs
        .iter()
        .filter_map(|v| match v {
            RmVerb::AllocSysmem { handle, .. } => Some(*handle),
            _ => None,
        })
        .collect();
    let freed: Vec<_> = verbs
        .iter()
        .filter_map(|v| match v {
            RmVerb::Free { obj } => Some(*obj),
            _ => None,
        })
        .collect();
    assert_eq!(
        allocated.len(),
        2,
        "non-vacuity: BOTH publications allocated host memory before their commits"
    );
    assert_eq!(
        freed,
        vec![allocated[1]],
        "the refused commit released exactly the memory IT orphaned — and left the \
         first publication's, which is live, alone"
    );
    assert_eq!(
        verbs
            .iter()
            .filter(|v| matches!(v, RmVerb::UnmapGpuVa { .. }))
            .count(),
        1,
        "…unmapped from the host VAS first (unmap before free), exactly once"
    );
}

/// ★ A **refused** op returns its worker to the pool. The refusal path is the one
/// that returns the worker outside the commit critical section
/// (`SharedDevice::return_worker`), and the campaign found that whole function
/// deletable (`device.rs:541`) with the suite still green — a pool slot leaked per
/// refusal, which on a pool of one is a permanent wedge of that proc.
///
/// Driven on a pool of ONE so the leak is a hang rather than a statistic; the
/// watchdog turns that hang into a bounded failure.
#[test]
fn a_refused_op_returns_its_worker_to_the_pool() {
    let _wd = watchdog("refusal_returns_worker", Duration::from_secs(60));
    let (device, _pids, _rec) = device_with(1, 1, LockMode::Sharded); // pool of ONE
    device
        .publish_backing(GPU, PDB, VA, 0x1000)
        .expect("the first publication binds");
    assert_eq!(
        device.publish_backing(GPU, PDB, VA, 0x1000),
        Err(FwdFault::Address(kayfabe_mmu::AddressFault::Overlap {
            pdb: PDB,
            va: VA
        })),
        "the re-publication refuses in the COMMIT, i.e. with a worker checked out"
    );
    // If the refusal path had swallowed the worker, this blocks forever on an empty
    // pool and the watchdog aborts.
    device
        .publish_backing(GPU, PDB, GpuVa(VA.0 + 0x1000), 0x1000)
        .expect("the pool's ONLY worker is available again after the refusal");
}

/// ★ The #14 ring-gate as the **shell** exposes it: `SharedDevice::gate_working_set`
/// must refuse a working set that is not host-published in the channel's own VAS.
///
/// `pushbuffer_parser.rs` pins the gate on the core function (`&Gpu`); the shell's
/// wrapper is a separate route+lock composition, and the campaign found it replaceable
/// with `Ok(())` — a ring gate that says "published" for everything, which is exactly
/// the ungated cross-VAS ring #14 is about. Asserted in BOTH lock modes: the two take
/// different lock paths to the same answer.
#[test]
fn the_shell_ring_gate_refuses_an_unpublished_working_set() {
    let _wd = watchdog("shell_ring_gate", Duration::from_secs(30));
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (device, pids, _rec) = device_with(1, DEFAULT_POOL_WORKERS, mode);
        let pid = pids[0];
        let gpu = Arc::into_inner(device).expect("not shared").into_gpu();
        let (_, cid) = gpu.spine.by_vchid[&(GPU, GR)];
        let device = SharedDevice::new(gpu, mode);

        let unpublished = GpuVa(VA.0 + 0x10_0000);
        assert_eq!(
            device.gate_working_set(pid, cid, &[unpublished]),
            Err(FwdFault::Address(kayfabe_mmu::AddressFault::Miss {
                pdb: PDB,
                va: unpublished
            })),
            "({mode:?}) an unpublished VA in the working set is a loud MISS — the gate \
             never answers 'published' by default"
        );
        // Non-vacuity: once published, the SAME query passes.
        device
            .publish_backing(GPU, PDB, unpublished, 0x1000)
            .expect("publishes");
        assert_eq!(
            device.gate_working_set(pid, cid, &[unpublished]),
            Ok(()),
            "({mode:?}) and a published VA passes, so the refusal above was about \
             publication, not about the call"
        );
    }
}
