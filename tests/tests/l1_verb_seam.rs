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
//!   dispatches through the reactor, kills the slot, retires the proc; its
//!   completions die with it.
//! - **`single_in_flight_per_worker_is_structural`** — one `&mut`-owned handle per
//!   slot: a checked-out worker cannot be checked out again, and concurrency comes
//!   from channel COUNT (§11 B6) — there is no shared in-flight slot table to grow.
//!
//! Every test arms a [`watchdog`] (the `concurrency_stress.rs` bounded-termination
//! rule) and joins every thread it spawns.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant as WallInstant};

use nvkvm_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use nvkvm_completion::OsEventRef;
use nvkvm_core::ProcId;
use nvkvm_core::gpa::GpaSpace;
use nvkvm_core::gpu::Gpu;
use nvkvm_core::reactor::SourceKind;
use nvkvm_core::rmgraph::{AllocFacts, RmEvent};
use nvkvm_fwd::{FwdFault, Stale};
use nvkvm_isolate::{DEFAULT_POOL_WORKERS, IsolateFactory, IsolateId, VerbPlan, WorkerId};
use nvkvm_mocks::{HoldSpec, MockArch, MockIsolateFactory, SharedRecorder, VerbHold, VerbKind};
use nvkvm_rt::device::{LockMode, SharedDevice, SignalOutcome};
use nvkvm_rt::executor::{Effect, Executor};
use nvkvm_rt::inbox::{CoreEvent, inbox};
use nvkvm_rt::lock::{LockRank, RankedMutex};
use nvkvm_tests::{Scenario, identical_handles};
use nvkvm_util::Instant;

// ---------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------

/// Abort the process loudly if the guard is not dropped within `limit` — bounded
/// termination, so a regression that wedges this suite fails fast instead of eating
/// the CI timeout.
#[must_use]
fn watchdog(test: &'static str, limit: Duration) -> WatchdogGuard {
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
    assert_eq!(nvkvm_rt::lock::held_depth(), 0, "no guard is alive here");

    let reply = worker
        .execute(&VerbPlan::Publish {
            host_vas: None,
            len: 0x1000,
        })
        .expect("the chain runs");
    match reply {
        nvkvm_isolate::VerbReply::Published { host_vas, .. } => {
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
            nvkvm_mocks::RmVerb::AllocVaSpace { .. } => "vas",
            nvkvm_mocks::RmVerb::AllocSysmem { .. } => "mem",
            nvkvm_mocks::RmVerb::MapGpuVa { .. } => "map",
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
            (binding.phys, binding.host_va, off),
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
    Result<nvkvm_fwd::Published, FwdFault>,
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
    // the read path refuses.
    assert_eq!(
        device.resolve(GPU, PDB, VA),
        Err(FwdFault::RetiredProc(pid)),
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
            class: nvkvm_mocks::mock_classes::CHANNEL_GR,
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
/// opens a [`nvkvm_rt::lock::BlockingSection`] before parking, which panics unless
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
            nvkvm_mocks::RmVerb::MapGpuVa { va, .. } => Some(*va),
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

        // The proc is retired: every op on it refuses, loudly.
        assert_eq!(
            device.publish_backing(GPU, PDB, GpuVa(VA.0 + 0x1000), 0x1000),
            Err(FwdFault::RetiredProc(victim)),
            "({mode:?}) the retired proc refuses every op, loudly"
        );
        // NOTE (finding, `l1_concurrency.md` §12.11): an out-of-band retire does not
        // rebuild `by_pdb`/`by_vchid` — only a graph `refresh` does — so routing still
        // NAMES the dead proc and the miss surfaces one step later, at the live-set
        // lookup. Loud and mutation-free either way; the fault just says
        // `RetiredProc` rather than `UnknownVchid`.
        assert_eq!(
            device.doorbell(GPU, MockArch::token_for(GR), &[]),
            Err(FwdFault::RetiredProc(victim)),
            "({mode:?}) and its channels reach a proc that is no longer live"
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
        .forward_engine_object(GPU, GR, nvkvm_tests::COMPUTE_CLASS, &[])
        .expect("first forward allocs");
    assert!(!first.reused);

    // Hold the ONLY worker with an unrelated op, then replay: if the replay tried to
    // check a worker out it would block here forever (and hit the watchdog).
    let held = hold(&rec, pid, 0, VerbKind::AllocSysmem);
    let d = Arc::clone(&device);
    let t = thread::spawn(move || d.publish_backing(GPU, PDB, VA, 0x1000));
    held.wait_until_pending();

    let replay = device
        .forward_engine_object(GPU, GR, nvkvm_tests::COMPUTE_CLASS, &[])
        .expect("the replay resolves from core state with the pool saturated");
    assert!(replay.reused, "a re-send is idempotent");
    assert_eq!(replay.host_object, first.host_object, "the ORIGINAL object");

    held.release();
    t.join().expect("joins").expect("publishes");
}
