//! ★ L1-M1 stage 2 — the `kayfabe-rt` threaded shell, integration-tested
//! (`l1_concurrency.md` §3.3 R1/R3, §3.4 the lock swap, §8.2 both lock
//! configurations from day one).
//!
//! What this file pins, per test:
//!
//! - **`lock_mode_differential_scripted_sequence_yields_identical_state`** — one
//!   scripted, non-trivial op sequence (3 procs, 2 GPUs, applies + doorbells +
//!   publishes + polls + a retire + executor traffic) runs under `Degenerate` and
//!   `Sharded` and produces the **identical** device end-state AND the identical
//!   op-by-op outcome log. This is the test that makes the late granularity flip
//!   (the #14 gate) safe: the untested mode does not exist. (Technique borrowed
//!   from `determinism.rs`'s whole-`Gpu` snapshot; this one is EXACT — same op
//!   order both runs, so even minted host handles must match bit-for-bit — where
//!   determinism.rs's is deliberately order-canonicalized/masked. Different
//!   purpose, so the helper is local, not lifted.)
//! - **`spine_ops_acquire_no_proc_lock_via_get_mut`** — the ★ mechanic: a spine op
//!   under the device write guard drives every proc through `Mutex::get_mut` and
//!   the per-thread rank-1 acquisition counter proves rank 1 was NEVER entered.
//! - **`executor_source_signal_lands_in_owning_procs_queue_only`** /
//!   **`executor_signal_for_retired_procs_source_faults_and_mutates_nothing`** —
//!   the dispatch path: an os-event signal observes into the OWNING proc's queue
//!   and no other; a retired proc's source resolves to a loud `SourceFault` and
//!   mutates nothing (the F4 use-after-retire species, designed out).
//! - **`threads_smoke_hammers_both_lock_modes_bounded`** — the T2 shape: real
//!   threads hammering doorbells/publishes/polls/applies/signals across BOTH lock
//!   modes with the R1/R3 asserts running hot, bounded iteration counts sized to
//!   seconds, every thread joined, and a watchdog that aborts loudly on a wedge
//!   (the `concurrency_stress.rs` bounded-termination lesson, kept).

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant as WallInstant};

use kayfabe_arch::ids::{EngineKind, GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_completion::OsEventRef;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::reactor::SourceKind;
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_core::{ChanId, ProcAnchor, ProcId};
use kayfabe_isolate::HostHandle;
use kayfabe_mocks::{MockArch, MockIsolateFactory, SharedRecorder};
use kayfabe_rt::device::{LockMode, SharedDevice, SignalOutcome};
use kayfabe_rt::executor::{Effect, Executor};
use kayfabe_rt::inbox::{CoreEvent, inbox};
use kayfabe_rt::lock::{self, BlockingSection, LockRank};
use kayfabe_tests::{Scenario, identical_handles};
use kayfabe_util::Instant;
use kayfabe_vmm::CoreEventKind;

// ---------------------------------------------------------------------------------
// Harness helpers (watchdog + scenario), following concurrency_stress.rs
// ---------------------------------------------------------------------------------

/// Watchdog: abort the process loudly if the guard is not dropped within `limit`
/// (bounded termination — a future wedge fails fast, never an opaque CI timeout).
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
            thread::sleep(Duration::from_millis(200));
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

fn pdb_of(i: usize) -> Pdb {
    Pdb(0x3400_0000 + (i as u64) * 0x1_0000)
}
fn gr_vchid(i: usize) -> VChid {
    VChid(0x100 + i as u16)
}
fn ce_vchid(i: usize) -> VChid {
    VChid(0x200 + i as u16)
}
/// Proc `i`'s GPU target: alternating across two GPUs (the multi-GPU axis).
fn gpu_of(i: usize) -> GpuId {
    GpuId((i % 2) as u32)
}
fn client_of(i: usize) -> HClient {
    HClient(0xA0 + i as u32)
}
const MEM_HANDLE: HObject = HObject(0x6000_0000);

/// Build `n` guest procs with IDENTICAL guest handles (the #14 shape), DISTINCT
/// PDBs/vChids, alternating across 2 GPUs, each with a declared memory object for
/// RM-map churn. Returns the realized device plus `pids[i]` resolved via `by_pdb`
/// (never by assuming mint order).
fn rt_gpu(n: usize) -> (Gpu, Vec<ProcId>, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");

    let mut s = Scenario::new();
    for i in 0..n {
        let device_instance = match gpu_of(i) {
            GpuId::ZERO => None,
            g => Some(g.0),
        };
        s.compute_process_on_gpu(
            client_of(i),
            pdb_of(i),
            identical_handles(gr_vchid(i).0, ce_vchid(i).0),
            device_instance,
        );
        s.memory(
            client_of(i),
            HObject(0x5c00_0001),
            MEM_HANDLE,
            0x9_0000_0000 + (i as u64) * 0x1000_0000,
        );
    }
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    assert_eq!(gpu.procs.len(), n);
    let pids: Vec<ProcId> = (0..n)
        .map(|i| gpu.spine.by_pdb[&(gpu_of(i), pdb_of(i))])
        .collect();
    (gpu, pids, recorder)
}

// ---------------------------------------------------------------------------------
// The EXACT end-state snapshot (see module docs on why this is not determinism.rs's)
// ---------------------------------------------------------------------------------

/// One VAS's exact state: (host VAS, table entries as (va, len, phys, host_va)).
type VasSnap = (Option<HostHandle>, Vec<(u64, u64, u64, Option<u64>)>);
/// One channel's exact state: (gpu, vchid, vas_pdb, engine, host_channel, host_token).
type ChanSnap = (
    GpuId,
    VChid,
    Option<Pdb>,
    EngineKind,
    Option<HostHandle>,
    Option<u64>,
);

#[derive(Debug, PartialEq, Eq)]
struct ProcSnap {
    anchor: ProcAnchor,
    clients: BTreeSet<HClient>,
    vases: BTreeMap<(GpuId, Pdb), VasSnap>,
    channels: BTreeMap<ChanId, ChanSnap>,
    scheduled: BTreeSet<ChanId>,
    outstanding: usize,
    targets: BTreeSet<GpuId>,
    arenas: BTreeMap<GpuId, Range<u64>>,
    last_poll: Option<Instant>,
    last_token: Option<u64>,
    retired: bool,
}

fn snap_proc(p: &kayfabe_core::gpu::Proc) -> ProcSnap {
    ProcSnap {
        anchor: p.anchor,
        clients: p.clients.clone(),
        vases: p
            .vases
            .iter()
            .map(|(&k, v)| {
                let entries: Vec<_> = v
                    .table
                    .iter()
                    .map(|(va, len, b)| (va, len, b.phys, b.host_va))
                    .collect();
                (k, (v.host_vas, entries))
            })
            .collect(),
        channels: p
            .channels
            .iter()
            .map(|(&cid, c)| {
                (
                    cid,
                    (
                        c.gpu,
                        c.vchid,
                        c.vas_pdb,
                        c.engine,
                        c.host_channel,
                        c.host_token,
                    ),
                )
            })
            .collect(),
        scheduled: p.exec.scheduled.clone(),
        outstanding: p.completion.outstanding_len(),
        targets: p.targets.clone(),
        arenas: p
            .arenas
            .iter()
            .map(|(&g, a)| (g, a.range.clone()))
            .collect(),
        last_poll: p.poll.last_poll,
        last_token: p.poll.last_token,
        retired: p.is_retired(),
    }
}

/// The whole-device exact end-state: per-proc snaps + routing + sources + retired.
#[derive(Debug, PartialEq, Eq)]
struct RtSnapshot {
    procs: BTreeMap<ProcId, ProcSnap>,
    system: ProcSnap,
    by_pdb: BTreeMap<(GpuId, Pdb), ProcId>,
    by_vchid: BTreeMap<(GpuId, VChid), (ProcId, ChanId)>,
    retired: Vec<ProcId>,
    sources: Vec<String>,
}

fn snapshot(gpu: &Gpu) -> RtSnapshot {
    RtSnapshot {
        procs: gpu
            .procs
            .iter()
            .map(|(&id, p)| (id, snap_proc(p)))
            .collect(),
        system: snap_proc(&gpu.system),
        by_pdb: gpu.spine.by_pdb.clone(),
        by_vchid: gpu.spine.by_vchid.clone(),
        retired: gpu.spine.retired.iter().map(|p| p.id).collect(),
        sources: gpu
            .spine
            .sources
            .iter()
            .map(|(s, k)| format!("{s:?}:{k:?}"))
            .collect(),
    }
}

// ---------------------------------------------------------------------------------
// ★ Test 5 — the lock-mode differential
// ---------------------------------------------------------------------------------

/// Run the scripted sequence under `mode`, returning the op-by-op outcome log and
/// the exact end-state snapshot. Single-threaded and fully deterministic, so both
/// modes must match bit-for-bit — any divergence is a mode leaking through the API.
fn run_script(mode: LockMode) -> (Vec<String>, RtSnapshot) {
    const N: usize = 3;
    let (gpu, pids, _rec) = rt_gpu(N);
    let device = Arc::new(SharedDevice::new(gpu, mode));
    let (tx, rx) = inbox();
    let mut ex = Executor::new(Arc::clone(&device), rx);
    let mut log: Vec<String> = Vec::new();

    // 1. Publishes: 4 pages per proc, IDENTICAL guest VAs across procs (#14 shape).
    let vas: Vec<GpuVa> = (0..4).map(|k| GpuVa(0x2_0020_0000 + k * 0x1000)).collect();
    for i in 0..N {
        for &va in &vas {
            log.push(format!(
                "publish[{i}] {:?}",
                device.publish_backing(gpu_of(i), pdb_of(i), va, 0x1000)
            ));
        }
    }

    // 2. Doorbells: GR with the published working set, CE with an empty one.
    let mut gr_chans: Vec<ChanId> = Vec::new();
    for (i, &pid) in pids.iter().enumerate() {
        let out = device
            .doorbell(gpu_of(i), MockArch::token_for(gr_vchid(i)), &vas)
            .expect("GR doorbell routes and gates");
        assert_eq!(out.proc, pid, "doorbell demuxed to the wrong proc");
        gr_chans.push(out.chan);
        log.push(format!("doorbell-gr[{i}] {out:?}"));
        log.push(format!(
            "doorbell-ce[{i}] {:?}",
            device.doorbell(gpu_of(i), MockArch::token_for(ce_vchid(i)), &[])
        ));
    }

    // 3. Read-only route/act: resolve + the ring-gate query.
    for (i, (&pid, &chan)) in pids.iter().zip(&gr_chans).enumerate() {
        log.push(format!(
            "resolve[{i}] {:?}",
            device.resolve(gpu_of(i), pdb_of(i), GpuVa(vas[0].0 + 0x40))
        ));
        log.push(format!(
            "gate[{i}] {:?}",
            device.gate_working_set(pid, chan, &vas)
        ));
    }

    // 4. Completion sources + executor traffic (one os-event source per proc).
    let sources: Vec<_> = (0..N)
        .map(|i| {
            device.register_source(SourceKind::OsEvent {
                proc: pids[i],
                gpu: gpu_of(i),
                ev: OsEventRef(0xE000 + i as u64),
            })
        })
        .collect();
    for &s in &sources {
        tx.send(CoreEvent::SourceSignal(s));
    }
    for eff in ex.drain_all() {
        log.push(format!("effect {eff:?}"));
    }

    // 5. Owner polls + drains (per target), then a pump edge per target.
    for (i, &pid) in pids.iter().enumerate() {
        log.push(format!(
            "poll[{i}] {:?}",
            device.completion_poll(gpu_of(i), pid, Instant(10 + i as u64))
        ));
    }
    for g in [GpuId::ZERO, GpuId(1)] {
        device.completions_drained(g);
        log.push(format!("pump[{g:?}] {:?}", device.pump_completions(g)));
        device.completions_drained(g);
    }

    // 6. RM map churn through apply (map → unmap → map; left mapped on purpose so
    // the RPC binding shows in the snapshot).
    let map_ev = RmEvent::MapMemoryDma {
        client: client_of(0),
        vaspace: HObject(0x5c00_0010),
        memory: MEM_HANDLE,
        va: GpuVa(0x80_0000_0000),
        offset: 0,
        len: 0x2000,
    };
    log.push(format!("map {:?}", device.apply(map_ev)));
    log.push(format!(
        "unmap {:?}",
        device.apply(RmEvent::Unmap {
            client: client_of(0),
            vaspace: HObject(0x5c00_0010),
            va: GpuVa(0x80_0000_0000),
        })
    ));
    log.push(format!("re-map {:?}", device.apply(map_ev)));

    // 7. Retire proc 2 mid-flight (free its client root), then prove the world
    // refuses its identities loudly: routing gone, its source resolves to nothing.
    log.push(format!(
        "free-root[2] {:?}",
        device.apply(RmEvent::Free {
            client: client_of(2),
            handle: HObject(0x5c00_0000),
        })
    ));
    log.push(format!(
        "publish-after-retire {:?}",
        device.publish_backing(gpu_of(2), pdb_of(2), GpuVa(0x2_0020_0000), 0x1000)
    ));
    log.push(format!(
        "doorbell-after-retire {:?}",
        device.doorbell(gpu_of(2), MockArch::token_for(gr_vchid(2)), &[])
    ));
    tx.send(CoreEvent::SourceSignal(sources[2]));
    for eff in ex.drain_all() {
        log.push(format!("late-signal {eff:?}"));
    }

    // 8. The deferred edges: reap at the quiesce point, then a redeliver backstop.
    tx.send(CoreEvent::Deferred(CoreEventKind::DeferredReap));
    tx.send(CoreEvent::Deferred(CoreEventKind::CompletionRedeliver));
    for eff in ex.drain_all() {
        log.push(format!("deferred {eff:?}"));
    }
    device.completions_drained(GpuId::ZERO);

    // 9. The stage-3 wake seam: set changes happened, the latch must say so (and
    // drain exactly once).
    log.push(format!("wake {}", device.take_pending_wake()));
    log.push(format!("wake-again {}", device.take_pending_wake()));

    drop(ex); // drops its Arc so the unwrap below is sound
    let gpu = Arc::try_unwrap(device)
        .unwrap_or_else(|_| panic!("all device handles released"))
        .into_gpu();
    (log, snapshot(&gpu))
}

/// ★ Test 5: identical scripted sequence, both lock modes, identical outcome log
/// AND identical exact end-state. The mode must not be observable — this is what
/// makes flipping `Degenerate → Sharded` at the #14 gate a configuration change
/// instead of a leap of faith (§8.2 / review P5).
#[test]
fn lock_mode_differential_scripted_sequence_yields_identical_state() {
    let _wd = watchdog(
        "lock_mode_differential_scripted_sequence_yields_identical_state",
        Duration::from_secs(60),
    );
    let (log_d, snap_d) = run_script(LockMode::Degenerate);
    let (log_s, snap_s) = run_script(LockMode::Sharded);

    // Not vacuous: the script really exercised the interesting paths.
    assert!(
        log_d.iter().any(|l| l.contains("Reaped(1)")),
        "the reap ran"
    );
    assert!(
        log_d.iter().any(|l| l.contains("Fault")),
        "the retired proc's late signal faulted loudly"
    );
    assert!(
        log_d.iter().any(|l| l == "wake true"),
        "the wake latch armed"
    );
    assert!(
        log_d.iter().any(|l| l == "wake-again false"),
        "the wake latch drains once"
    );

    assert_eq!(
        log_d, log_s,
        "op-by-op outcomes diverged between lock modes"
    );
    assert_eq!(
        snap_d, snap_s,
        "device end-state diverged between lock modes"
    );
}

// ---------------------------------------------------------------------------------
// Test 6 — spine ops acquire no proc lock (the get_mut mechanic)
// ---------------------------------------------------------------------------------

/// ★ Test 6: a spine op under the sharded device's write guard drives every proc
/// via `Mutex::get_mut` — the per-thread rank-1 acquisition counter proves rank 1
/// was NEVER entered (zero proc-lock acquisitions), while a per-proc op afterwards
/// proves the counter does count (device-read + exactly one proc lock).
#[test]
fn spine_ops_acquire_no_proc_lock_via_get_mut() {
    let _wd = watchdog(
        "spine_ops_acquire_no_proc_lock_via_get_mut",
        Duration::from_secs(60),
    );
    let (gpu, pids, _rec) = rt_gpu(2);
    let device = SharedDevice::new(gpu, LockMode::Sharded);

    let proc_before = lock::acquisitions(LockRank::Proc);
    let dev_before = lock::acquisitions(LockRank::Device);

    // Spine ops, all of them: apply (graph + refresh over EVERY proc), the pump,
    // the drain, the owner poll (remove/put-back through the ProcSet), the reap,
    // and a source registration.
    device
        .apply(RmEvent::MapMemoryDma {
            client: client_of(0),
            vaspace: HObject(0x5c00_0010),
            memory: MEM_HANDLE,
            va: GpuVa(0x80_0000_0000),
            offset: 0,
            len: 0x1000,
        })
        .expect("map applies");
    device.pump_completions(GpuId::ZERO);
    device.completions_drained(GpuId::ZERO);
    device.completion_poll(GpuId::ZERO, pids[0], Instant(1));
    device.reap_retired();
    device.register_source(SourceKind::Notify);

    assert_eq!(
        lock::acquisitions(LockRank::Proc) - proc_before,
        0,
        "a spine op acquired a proc lock — the get_mut mechanic regressed"
    );
    let dev_spine = lock::acquisitions(LockRank::Device) - dev_before;
    assert_eq!(
        dev_spine, 6,
        "each spine op took the device lock exactly once"
    );

    // ★ Control: a per-proc VERB op enters rank 1 exactly TWICE — the stage-3
    // plan/execute/commit shape (`l1_concurrency.md` §7.3): one locked phase to plan
    // + check a worker OUT, the lock-free verb round trip, then one locked phase to
    // re-validate (R5), commit, and check the worker back IN. Pinning the number is
    // what makes reverting the split a test failure: the old in-lock verb call took
    // rank 1 exactly ONCE, and any future "optimization" that collapses the two
    // phases back into one is holding the proc lock across a host verb.
    let dev_mid = lock::acquisitions(LockRank::Device);
    device
        .doorbell(gpu_of(0), MockArch::token_for(gr_vchid(0)), &[])
        .expect("doorbell");
    assert_eq!(
        lock::acquisitions(LockRank::Proc) - proc_before,
        2,
        "the sharded doorbell takes its owning proc's lock once to plan+checkout and \
         once to commit+checkin — never across the verb"
    );
    assert_eq!(
        lock::acquisitions(LockRank::Device) - dev_mid,
        2,
        "and the device read lock brackets exactly those two phases"
    );
    assert_eq!(lock::held_depth(), 0, "nothing leaked");
}

// ---------------------------------------------------------------------------------
// Test 7 — executor dispatch
// ---------------------------------------------------------------------------------

/// ★ Test 7a: a `SourceSignal` for a registered os-event source lands the
/// completion in the OWNING proc's queue and in no other proc's (and not in the
/// system proc's) — per-source → per-proc dispatch by construction, in both modes.
#[test]
fn executor_source_signal_lands_in_owning_procs_queue_only() {
    let _wd = watchdog(
        "executor_source_signal_lands_in_owning_procs_queue_only",
        Duration::from_secs(60),
    );
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (gpu, pids, _rec) = rt_gpu(2);
        let device = Arc::new(SharedDevice::new(gpu, mode));
        let (tx, rx) = inbox();
        let mut ex = Executor::new(Arc::clone(&device), rx);

        let ev = OsEventRef(0xBEEF);
        let src = device.register_source(SourceKind::OsEvent {
            proc: pids[0],
            gpu: gpu_of(0),
            ev,
        });
        tx.send(CoreEvent::SourceSignal(src));
        let effects = ex.drain_all();
        assert_eq!(
            effects,
            vec![Effect::Signal(SignalOutcome::Observed {
                proc: pids[0],
                gpu: gpu_of(0),
                ev
            })],
            "({mode:?}) one signal → one typed Observed for the owner"
        );

        drop(ex);
        let gpu = Arc::try_unwrap(device)
            .unwrap_or_else(|_| panic!("all handles released"))
            .into_gpu();
        assert!(
            gpu.procs[&pids[0]].completion.has_outstanding(),
            "({mode:?}) the owner's queue holds the event"
        );
        assert!(
            !gpu.procs[&pids[1]].completion.has_outstanding(),
            "({mode:?}) the other proc's queue is untouched"
        );
        assert!(
            !gpu.system.completion.has_outstanding(),
            "({mode:?}) the system proc's queue is untouched"
        );
    }
}

/// ★ Test 7b: a signal for a RETIRED proc's source surfaces the fault loudly and
/// mutates nothing — retire deregisters every source of the proc inside the same
/// apply critical section, and handles are never reused, so the late signal
/// resolves to nothing (the C's F4 use-after-retire, structurally gone).
#[test]
fn executor_signal_for_retired_procs_source_faults_and_mutates_nothing() {
    let _wd = watchdog(
        "executor_signal_for_retired_procs_source_faults_and_mutates_nothing",
        Duration::from_secs(60),
    );
    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (gpu, pids, _rec) = rt_gpu(2);
        let device = Arc::new(SharedDevice::new(gpu, mode));
        let (tx, rx) = inbox();
        let mut ex = Executor::new(Arc::clone(&device), rx);

        let src = device.register_source(SourceKind::OsEvent {
            proc: pids[1],
            gpu: gpu_of(1),
            ev: OsEventRef(0xDEAD),
        });
        // Retire proc 1: free its client root (retire deregisters its sources).
        device
            .apply(RmEvent::Free {
                client: client_of(1),
                handle: HObject(0x5c00_0000),
            })
            .expect("root free applies");

        tx.send(CoreEvent::SourceSignal(src));
        let effects = ex.drain_all();
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::Signal(SignalOutcome::Fault(f)) => assert_eq!(f.source, src),
            other => panic!("({mode:?}) expected a loud SourceFault, got {other:?}"),
        }

        drop(ex);
        let gpu = Arc::try_unwrap(device)
            .unwrap_or_else(|_| panic!("all handles released"))
            .into_gpu();
        // Mutated nothing: no queue anywhere holds anything, the retired proc is
        // still exactly one un-reaped entry, the survivor is untouched.
        assert!(!gpu.procs[&pids[0]].completion.has_outstanding());
        assert!(!gpu.system.completion.has_outstanding());
        assert_eq!(gpu.spine.retired.len(), 1, "({mode:?}) retired, not reaped");
        assert!(gpu.spine.retired[0].completion.outstanding_len() == 0);
        assert!(
            gpu.spine.sources.is_empty(),
            "({mode:?}) source deregistered"
        );
    }
}

// ---------------------------------------------------------------------------------
// Test 8 — the real-threads smoke (T2 shape), both lock modes
// ---------------------------------------------------------------------------------

/// Tiny deterministic per-thread RNG (xorshift64*), as in concurrency_stress.rs.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// ★ Test 8: N real threads hammer one `SharedDevice` — doorbells, publishes,
/// resolves, polls/pumps/drains, RM-map applies, source signals through a live
/// executor thread, and `BlockingSection`s — under BOTH lock modes, with the
/// always-on R1/R3 asserts running hot the whole time (they are the point). Every
/// count is bounded and sized to seconds; every thread is joined; the watchdog
/// aborts loudly on a wedge.
#[test]
fn threads_smoke_hammers_both_lock_modes_bounded() {
    let _wd = watchdog(
        "threads_smoke_hammers_both_lock_modes_bounded",
        Duration::from_secs(240),
    );
    const N_THREADS: usize = 8;
    const OPS_PER_THREAD: usize = 5_000;
    const N_PROCS: usize = 4;

    for mode in [LockMode::Degenerate, LockMode::Sharded] {
        let (gpu, pids, _rec) = rt_gpu(N_PROCS);
        let device = Arc::new(SharedDevice::new(gpu, mode));
        let (tx, rx) = inbox();

        // One os-event source per proc, registered up front.
        let ev_of = |i: usize| OsEventRef(0xE000 + i as u64);
        let sources: Vec<_> = (0..N_PROCS)
            .map(|i| {
                device.register_source(SourceKind::OsEvent {
                    proc: pids[i],
                    gpu: gpu_of(i),
                    ev: ev_of(i),
                })
            })
            .collect();

        let signals_sent = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        let observed = Mutex::new(0usize);

        thread::scope(|s| {
            // The executor: drains signals concurrently with the hammering. Only
            // Observed effects may occur (nothing retires in the smoke).
            let ex_device = Arc::clone(&device);
            let stop_ref = &stop;
            let observed_ref = &observed;
            let executor = s.spawn(move || {
                let mut ex = Executor::new(ex_device, rx);
                let mut seen = 0usize;
                loop {
                    match ex.drain_one() {
                        Some(Effect::Signal(SignalOutcome::Observed { .. })) => seen += 1,
                        Some(other) => panic!("unexpected executor effect: {other:?}"),
                        None => {
                            if stop_ref.load(Ordering::Acquire) {
                                break;
                            }
                            thread::yield_now();
                        }
                    }
                }
                *observed_ref.lock().unwrap() += seen;
            });

            let workers: Vec<_> = (0..N_THREADS)
                .map(|tid| {
                    let device = Arc::clone(&device);
                    let tx = tx.clone();
                    let pids = &pids;
                    let sources = &sources;
                    let signals_sent = &signals_sent;
                    s.spawn(move || {
                        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ ((tid as u64 + 1) << 32));
                        let mut publish_cnt = [0u64; N_PROCS];
                        let mut last_pub: [Option<(GpuVa, u64, u64)>; N_PROCS] = [None; N_PROCS];
                        let mut rm_mapped = [false; N_PROCS];
                        for op in 0..OPS_PER_THREAD {
                            let i = (rng.next() % N_PROCS as u64) as usize;
                            match rng.next() % 100 {
                                // Doorbells — the exec-plane demux under load.
                                0..=29 => {
                                    let vchid = if rng.next().is_multiple_of(2) {
                                        gr_vchid(i)
                                    } else {
                                        ce_vchid(i)
                                    };
                                    let out = device
                                        .doorbell(gpu_of(i), MockArch::token_for(vchid), &[])
                                        .expect("doorbell routes");
                                    assert_eq!(out.proc, pids[i], "wrong-proc demux");
                                }
                                // Publish + read back own lane.
                                30..=54 => {
                                    let va = GpuVa(
                                        0x40_0000_0000
                                            + (tid as u64) * 0x1_0000_0000
                                            + publish_cnt[i] * 0x1000,
                                    );
                                    let out = device
                                        .publish_backing(gpu_of(i), pdb_of(i), va, 0x1000)
                                        .expect("publish");
                                    publish_cnt[i] += 1;
                                    last_pub[i] = Some((va, out.gpa, out.host_va));
                                }
                                55..=64 => {
                                    if let Some((va, gpa, host_va)) = last_pub[i] {
                                        let (b, off) = device
                                            .resolve(gpu_of(i), pdb_of(i), GpuVa(va.0 + 0x40))
                                            .expect("own published VA resolves");
                                        assert_eq!(
                                            (b.phys, b.host_va, off),
                                            (gpa, Some(host_va), 0x40)
                                        );
                                    }
                                }
                                // Completion plane churn.
                                65..=74 => match rng.next() % 3 {
                                    0 => {
                                        device.completion_poll(
                                            gpu_of(i),
                                            pids[i],
                                            Instant(op as u64),
                                        );
                                    }
                                    1 => {
                                        device.pump_completions(gpu_of(i));
                                    }
                                    _ => device.completions_drained(gpu_of(i)),
                                },
                                // Source signals through the inbox (law 9: the
                                // producer holds only the sender).
                                75..=84 => {
                                    tx.send(CoreEvent::SourceSignal(sources[i]));
                                    signals_sent.fetch_add(1, Ordering::Relaxed);
                                }
                                // R1 kept hot: a legal blocking section (no locks
                                // held here — the natural call site).
                                85..=89 => {
                                    let mut section = BlockingSection::enter();
                                    section.run(|| ());
                                }
                                // RM map churn through apply.
                                _ => {
                                    let va = GpuVa(0x80_0000_0000 + (tid as u64) * 0x1_0000_0000);
                                    let ev = if rm_mapped[i] {
                                        RmEvent::Unmap {
                                            client: client_of(i),
                                            vaspace: HObject(0x5c00_0010),
                                            va,
                                        }
                                    } else {
                                        RmEvent::MapMemoryDma {
                                            client: client_of(i),
                                            vaspace: HObject(0x5c00_0010),
                                            memory: MEM_HANDLE,
                                            va,
                                            offset: 0,
                                            len: 0x1000,
                                        }
                                    };
                                    rm_mapped[i] = !rm_mapped[i];
                                    device.apply(ev).expect("map churn applies");
                                }
                            }
                            assert_eq!(lock::held_depth(), 0, "an op leaked a ranked lock guard");
                        }
                    })
                })
                .collect();

            for w in workers {
                w.join().expect("worker clean");
            }
            // All sends happened-before this store; the executor exits on the
            // first empty drain it sees after it.
            stop.store(true, Ordering::Release);
            executor.join().expect("executor clean");
        });

        // Every signal was dispatched to exactly one Observed effect.
        assert_eq!(
            *observed.lock().unwrap(),
            signals_sent.load(Ordering::Relaxed),
            "({mode:?}) signal → Observed conservation"
        );

        // Flush the completion plane and prove exact emptiness after acks.
        for g in [GpuId::ZERO, GpuId(1)] {
            loop {
                device.completions_drained(g);
                if device.pump_completions(g).is_none() {
                    break;
                }
            }
            device.completions_drained(g);
        }
        let mut gpu = Arc::try_unwrap(device)
            .unwrap_or_else(|_| panic!("all handles released"))
            .into_gpu();
        for (i, &pid) in pids.iter().enumerate() {
            let p = gpu.procs.get_mut(&pid).expect("live");
            p.completion.ack(ev_of(i));
            assert!(
                !p.completion.has_outstanding(),
                "({mode:?}) queue not empty after full ack"
            );
        }

        // Consistency sweep: per-target arenas pairwise disjoint (system included),
        // routing maps point at live procs that agree.
        let mut ranges: Vec<(GpuId, Range<u64>)> = Vec::new();
        for p in gpu.procs.values().chain([&gpu.system]) {
            for (&g, a) in &p.arenas {
                ranges.push((g, a.range.clone()));
            }
        }
        for (i, (ga, a)) in ranges.iter().enumerate() {
            for (gb, b) in ranges.iter().skip(i + 1) {
                if ga == gb {
                    assert!(
                        a.end <= b.start || b.end <= a.start,
                        "({mode:?}) arena overlap on {ga:?}: {a:?} vs {b:?}"
                    );
                }
            }
        }
        for (&(g, vchid), &(pid, cid)) in &gpu.spine.by_vchid {
            let c = &gpu.procs[&pid].channels[&cid];
            assert_eq!((c.gpu, c.vchid), (g, vchid), "routing key disagreement");
        }
    }
}
