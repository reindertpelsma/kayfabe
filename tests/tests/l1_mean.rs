//! ★★ L1-M1 stage 4 — **THE MEAN TEST**, the arbiter (`l1_concurrency.md` §8.4).
//!
//! > *"Pass = the design survived contact. Fail = the doc changes, not the assert."*
//!
//! ONE composed harsh run — multi-process × multi-thread × multi-GPU × multi-workload,
//! entirely mock-driven (no GPU, no hypervisor, no fds, no syscalls) — rather than a
//! pile of isolated cases. §8.4 is explicit that the standing T2 mean scripts (stalled
//! verb, failure mid-verb, worker HUP, staleness canaries) **compose into the same
//! run**, because the interactions between them are the part a per-case suite cannot
//! reach. It runs under **both** lock configurations (§8.2 / review P5: a late
//! granularity flip must never be the untested mode).
//!
//! ## ★★ WHAT THIS TEST FOUND, AND WHAT FIXED IT (`l1_concurrency.md` §12.13 — FIXED)
//!
//! **An out-of-band retire was undone by the very next `Gpu::apply`.** §7.3 requires a
//! worker HUP to "retire the proc loudly … no resurrect", and
//! `SignalOutcome::WorkerDied` promises "**never a respawn**". But `Spine::retire_proc`
//! only removed the proc from the live set; the guest's client root is still in the
//! graph, so the next `refresh` found a boundary with no matching live proc, minted a
//! **fresh `ProcId`**, spawned a **brand-new isolate** (a respawned sandboxed worker,
//! new handle namespace) and a fresh GPA arena, rebuilt routing onto it, and the guest
//! resumed full service — a guest that could crash its isolate worker got a clean new
//! isolate on its next RM event.
//!
//! The fix is a **condemned component** on the `Spine`, keyed on the component's
//! **client set** (exactly what `refresh` matches boundaries on, so it survives every
//! re-derivation, re-label and split the guest can provoke). A boundary that intersects
//! a condemned set gets no `Proc`, no isolate, no arena and no live route; its keys are
//! filed in the condemned routing maps instead, so its ops get the *named*
//! `FwdFault::Condemned` rather than an anonymous miss. It clears only when the guest
//! itself frees the client root. [`out_of_band_retire_must_not_resurrect_the_isolate`]
//! is the design's own invariant, no longer ignored; the sweep below pins the fixed
//! behavior, and the properties the fix introduces are pinned by the
//! `condemned_*` tests at the bottom of this file.
//!
//! ## The world (§8.4 "several guest procs, each multi-threaded, across ≥2 mock GPUs")
//!
//! Six guest procs over two mock GPUs, in three **identity lanes**. A lane's `Pdb` and
//! `VChid` values are handed to a proc on GPU0 *and* a proc on GPU1 — byte-identical
//! numbers, because `Pdb`/`VChid` are **per-GPU namespaces** (`multi_gpu_and_mig.md`,
//! MG-3) and that is legal. Any routing bug that collapses `(GpuId, ·)` to `·`
//! mis-routes here immediately. All six also share IDENTICAL guest RM handle values
//! (the #14 shape), so the graph's `(client, handle)` keying is under load too.
//!
//! | proc | GPU | lane | role in the composed script |
//! |---|---|---|---|
//! | 0 | 0 | A | ★ the **witness**: one `Proc`, FOUR threads — one parked in a held verb, three running mixed workloads (§3.5, the case per-proc sharding cannot cover) |
//! | 1 | 1 | A | the **peer**: full progress on the OTHER GPU, also multi-threaded |
//! | 2 | 0 | B | the process that **tears down mid-flight** → R5 canary (a) |
//! | 3 | 1 | B | its channel is **torn down** in the verb gap → R5 canary (b) |
//! | 4 | 0 | C | an `apply` **rewrites its routing** in the verb gap → R5 canary (c) |
//! | 5 | 1 | C | its isolate **worker dies** (HUP through the §6 reactor) mid-run |
//!
//! ## ★ The parallelism assertion is PROGRESS-UNDER-PENDING, never wall-clock
//!
//! There is no sleep, no timing threshold and no "finished within X ms" anywhere in
//! this file (§8.3 forbids the clock; the only time that exists is `Instant` values the
//! test hands the core). The mock **holds FIVE host verbs pending** — TWO of them on
//! the witness's own isolate, on two different pool workers (§7.2: concurrency comes
//! from channel COUNT) — explicit latches the script releases. While they are held the
//! test requires, as *termination*:
//!
//! - three further sibling threads of the witness's own `Proc` complete alloc/map-heavy,
//!   doorbell-heavy and poll-heavy workloads **end to end**;
//! - completion sources fire and the sync thread's own polls **observe them** —
//!   delivery ran, with zero workers involved (§3.5 guarantee 3);
//! - a second proc makes full progress **on the other GPU**;
//! - device-**WRITE** ops (`apply`, `retire_proc`, free+re-alloc) complete — the sharpest
//!   probe there is, because they prove the parked verbs hold no lock *at all*, not
//!   merely no proc lock;
//! - only THEN are the latches released, and each parked verb's commit is checked for
//!   the EXACT outcome its script earned.
//!
//! If R1 regressed (a verb held under a lock), or the pool regressed to #34's
//! one-worker-per-isolate, the sibling threads would block behind the parked verb and
//! the joins would never return — the watchdog aborts loudly instead of the box's speed
//! deciding the verdict. That is the whole point: the assertion is structural, so it
//! cannot pass because the machine was fast.
//!
//! ## The invariant asserts run hot the whole time (they are the point)
//!
//! **R3** (lock-rank) and **R1** (no blocking under a lock) are *always-on* panics in
//! `kayfabe_rt::lock` / `kayfabe_isolate::Worker::execute` — every lock acquisition and
//! every host verb in this run is checked, which is tens of thousands of checks per
//! mode. `held_depth() == 0` is additionally asserted after every op of every worker
//! thread, so a leaked guard is caught at the op that leaked it. **R5** is checked by
//! three staleness canaries, each asserting its **EXACT** `Stale` variant — the house
//! lesson from stage 3, where a canary passed for the wrong reason (`Rm(Other)` instead
//! of `Stale::Proc`) because the assert was `is_err()`-shaped.
//!
//! ## Conservation, asserted globally at the end of the run
//!
//! [`sweep_conservation`] re-assembles the device and checks the facts that must hold
//! no matter how the threads interleaved: completion conservation (nothing lost, nothing
//! duplicated, nothing in a queue that did not own it), pairwise-disjoint GPA arenas,
//! every published GPA inside its OWN proc's own-GPU arena, host-handle and host-VA
//! provenance (no handle minted for proc A is ever observed in proc B's state, and a
//! proc's GPU0 isolate never leaks into its GPU1 state), routing agreeing with the
//! graph, and reaped-once-never-reaped-again.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant as WallInstant};

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_completion::OsEventRef;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, GpuError};
use kayfabe_core::reactor::SourceKind;
use kayfabe_core::rmgraph::{AllocFacts, NodeKey, RmEvent};
use kayfabe_core::{ChanId, ProcAnchor, ProcId};
use kayfabe_fwd::{ControlRoute, DoorbellOutcome, FwdFault, Published, Stale};
use kayfabe_isolate::{HostHandle, IsolateId, RmError, WorkerId};
use kayfabe_mmu::AddressFault;
use kayfabe_mocks::{
    HoldSpec, MockArch, MockIsolateFactory, RmVerb, SharedRecorder, VerbHold, VerbKind,
    mock_classes, mock_ctrl,
};
use kayfabe_rt::device::{LockMode, SharedDevice, SignalOutcome};
use kayfabe_rt::executor::{Effect, Executor};
use kayfabe_rt::inbox::{CoreEvent, inbox};
use kayfabe_rt::lock;
use kayfabe_tests::{Scenario, identical_handles};
use kayfabe_util::Instant;

// =================================================================================
// Bounded termination (the `concurrency_stress.rs` M4b lesson, kept): fixed iteration
// counts sized to SECONDS on a 4-core box, every thread joined, and a watchdog on
// every test that aborts the process loudly on a wedge — a hang must fail fast, never
// eat the CI timeout.
// =================================================================================

/// Abort the process loudly if the guard is not dropped within `limit`.
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

/// Disarms its [`watchdog`] on drop.
struct WatchdogGuard(Arc<AtomicBool>);
impl Drop for WatchdogGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// ★ The armed latches, as a **drop guard** — harness lesson, learned the hard way on
/// this file's first run.
///
/// A held verb parks a scoped thread inside the mock backend. If an assert in the
/// window fails, the scope closure unwinds *without reaching the release*, and
/// `thread::scope`'s join-all then waits forever on threads nobody will ever wake: the
/// panic message is real but the process hangs behind it, so the failure looks like a
/// wedge instead of like the assertion it is. Releasing on `Drop` makes an unwind
/// release every latch first, so a failed mean assert **fails loudly and fast** — which
/// is the entire premise of the bounded-termination rule.
struct Latches(Vec<Arc<VerbHold>>);

impl Latches {
    /// Arm nothing yet; latches are pushed as their threads are spawned.
    fn new() -> Self {
        Latches(Vec::new())
    }
    /// Arm one latch and keep a handle to it.
    fn arm(
        &mut self,
        rec: &SharedRecorder,
        pid: ProcId,
        gpu: GpuId,
        worker: u32,
        verb: VerbKind,
    ) -> Arc<VerbHold> {
        let h = rec.lock().expect("recorder").hold(HoldSpec::exact(
            IsolateId(pid.0),
            gpu,
            WorkerId(worker),
            verb,
        ));
        self.0.push(Arc::clone(&h));
        h
    }
    /// Block until every armed verb has genuinely entered the backend — the progress
    /// EDGE that replaces a sleep.
    fn wait_all_pending(&self) {
        for h in &self.0 {
            h.wait_until_pending();
            assert!(h.is_pending());
        }
    }
    /// True while every armed verb is still parked.
    fn all_pending(&self) -> bool {
        self.0.iter().all(|h| h.is_pending())
    }
    /// Release every latch (idempotent).
    fn release_all(&self) {
        for h in &self.0 {
            h.release();
        }
    }
}

impl Drop for Latches {
    fn drop(&mut self) {
        self.release_all();
    }
}

/// Control-plane ops per alloc/map-heavy thread. Sized to SECONDS in a debug build on
/// a 4-core box — six workload threads per mode, two modes per test (measured: ≈2 s debug for the whole test). Deliberately large enough that the threads genuinely
/// interleave rather than run to completion one at a time.
const CTL_OPS: usize = 2_000;
/// Submissions per doorbell-heavy thread.
const RING_OPS: usize = 4_000;
/// Poll/pump rounds per sync thread.
const SYNC_OPS: usize = 4_000;

// =================================================================================
// The world
// =================================================================================

/// Target GPU 0.
const GPU0: GpuId = GpuId::ZERO;
/// Target GPU 1.
const GPU1: GpuId = GpuId(1);

/// ★ One **identity lane**: a `(Pdb, GR vChid, CE vChid)` triple. Each lane is handed
/// to a proc on GPU0 *and* a proc on GPU1 with byte-identical values — legal, because
/// both are per-GPU namespaces (MG-3) — so a routing map that lost its `GpuId` key
/// cross-routes the moment either proc is touched.
#[derive(Clone, Copy)]
struct Lane {
    /// The page-directory base (per-GPU namespace).
    pdb: Pdb,
    /// The GR channel's vChid (per-GPU runlist index).
    gr: VChid,
    /// The CE channel's vChid.
    ce: VChid,
}

/// The three lanes, each used TWICE (once per GPU).
const LANES: [Lane; 3] = [
    Lane {
        pdb: Pdb(0x3400_0000),
        gr: VChid(0x100),
        ce: VChid(0x200),
    },
    Lane {
        pdb: Pdb(0x3500_0000),
        gr: VChid(0x101),
        ce: VChid(0x201),
    },
    Lane {
        pdb: Pdb(0x3600_0000),
        gr: VChid(0x102),
        ce: VChid(0x202),
    },
];

/// Proc 0 — the ★ multi-threaded witness (one `Proc`, four threads) on GPU0.
const P_WITNESS: usize = 0;
/// Proc 1 — the peer making full progress on the OTHER GPU, also multi-threaded.
const P_PEER: usize = 1;
/// Proc 2 — retired mid-flight (R5 canary (a); §8.4's "tears down mid-flight").
const P_TEARDOWN: usize = 2;
/// Proc 3 — its routed channel is freed inside the verb gap (R5 canary (b)).
const P_CHANFREE: usize = 3;
/// Proc 4 — its routing is rewritten inside the verb gap (R5 canary (c)).
const P_REROUTE: usize = 4;
/// Proc 5 — its isolate worker dies out of band (§7.3 HUP through the §6 reactor).
const P_HUP: usize = 5;
/// How many guest procs the world has.
const N_PROCS: usize = 6;

/// Proc `i`'s RM client. Distinct per proc ⇒ distinct dup-components ⇒ distinct procs,
/// even though every proc reuses the same guest handle VALUES.
fn client_of(i: usize) -> HClient {
    HClient(0xA0 + i as u32)
}
/// Proc `i`'s target GPU — even procs on GPU0, odd on GPU1.
fn gpu_of(i: usize) -> GpuId {
    if i.is_multiple_of(2) { GPU0 } else { GPU1 }
}
/// Proc `i`'s identity lane — pairs `(2k, 2k+1)` share one lane across the two GPUs.
fn lane_of(i: usize) -> Lane {
    LANES[i / 2]
}

/// The VASpace handle every proc uses (identical values — the #14 shape).
const H_VASPACE: HObject = HObject(0x5c00_0010);
/// The Device handle every proc uses.
const H_DEVICE: HObject = HObject(0x5c00_0001);
/// The declared memory object every proc carries (for RM map churn through `apply`).
const MEM: HObject = HObject(0x6000_0000);
/// The handle the re-allocated GR channel of [`P_REROUTE`] takes (a *different* node
/// key at the *same* vChid — which is what makes routing resolve again, to a new
/// `ChanId`).
const H_REALLOC_GR: HObject = HObject(0x5c00_00f9);

// Disjoint guest-VA lanes, so two threads of ONE proc never collide in its address
// table (an `AddressFault::Overlap` would be a test bug, not a finding).
/// Warm-up publication (one page per proc, before the window opens).
const VA_WARM: u64 = 0x2_0000_0000;
/// The witness's HELD publication — the verb parked for the whole window.
const VA_HELD: u64 = 0x2_0010_0000;
/// A SECOND held publication by a SECOND thread of the same `Proc`, parked on a second
/// pool worker at the same time (§7.2: N workers = N independent 1-deep channels).
const VA_HELD2: u64 = 0x2_0020_0000;
/// The alloc/map-heavy thread's lane.
const VA_CTL: u64 = 0x10_0000_0000;
/// The doorbell-heavy thread's lane (it publishes what it then gates on).
const VA_RING: u64 = 0x20_0000_0000;
/// The RM-map-churn VA driven through `Gpu::apply`.
const VA_CHURN: u64 = 0x80_0000_0000;
/// A VA that is NEVER published — the #14 ring-gate's negative probe.
const VA_NEVER: GpuVa = GpuVa(0x7000_0000_0000);
/// The publication whose MAP verb is scripted to fail (the mid-chain failure script).
const VA_FAIL: u64 = 0x30_0000_0000;

/// Os-event refs the script signals through the §6 reactor. Distinct per signal, so
/// completion conservation is countable (`ack` removes by ref, so a duplicate ref would
/// make the count a lie).
const EV: [OsEventRef; 4] = [
    OsEventRef(0xE001),
    OsEventRef(0xE002),
    OsEventRef(0xE003),
    OsEventRef(0xE004),
];

/// Realize the six-proc, two-GPU world and wrap it in `mode`. Pool width is the
/// shipping default ([`kayfabe_isolate::DEFAULT_POOL_WORKERS`]) — the mean test must
/// run the configuration production runs.
fn mean_world(mode: LockMode) -> (Arc<SharedDevice>, Vec<ProcId>, SharedRecorder) {
    let (gpu, pids, recorder) = mean_gpu();
    (Arc::new(SharedDevice::new(gpu, mode)), pids, recorder)
}

/// The same six-proc, two-GPU world as a **bare [`Gpu`]**, for the §12.13 property
/// tests below: condemnation is a fact of the pure core, so the tests that pin it read
/// core state directly (arenas, client sets, the source registry) instead of through the
/// lock shell. Deterministic logic-core testing, §8.2's T1 tier.
fn mean_gpu() -> (Gpu, Vec<ProcId>, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");

    let mut s = Scenario::new();
    for i in 0..N_PROCS {
        let lane = lane_of(i);
        let instance = if gpu_of(i) == GPU0 {
            None
        } else {
            Some(gpu_of(i).0)
        };
        s.compute_process_on_gpu(
            client_of(i),
            lane.pdb,
            identical_handles(lane.gr.0, lane.ce.0),
            instance,
        );
        s.memory(
            client_of(i),
            H_DEVICE,
            MEM,
            0x9_0000_0000 + (i as u64) * 0x1000_0000,
        );
    }
    for ev in s.events {
        gpu.apply(ev).expect("the scenario applies cleanly");
    }
    assert_eq!(gpu.procs.len(), N_PROCS, "six distinct procs were derived");
    // Resolved through the routing map, never assumed from mint order.
    let pids: Vec<ProcId> = (0..N_PROCS)
        .map(|i| gpu.spine.by_pdb[&(gpu_of(i), lane_of(i).pdb)])
        .collect();
    (gpu, pids, recorder)
}

/// Arm a one-shot hold on `verb` of `(proc, gpu, worker)`'s isolate — the scripted
/// latch (§8.4). `IsolateId == ProcId` by construction (`Gpu` spawns `IsolateId(pid.0)`
/// per target).
fn hold(
    rec: &SharedRecorder,
    pid: ProcId,
    gpu: GpuId,
    worker: u32,
    verb: VerbKind,
) -> Arc<VerbHold> {
    rec.lock().expect("recorder").hold(HoldSpec::exact(
        IsolateId(pid.0),
        gpu,
        WorkerId(worker),
        verb,
    ))
}

/// How many host `free` verbs `pid`'s isolate has issued (the mid-chain leak check).
fn count_frees(rec: &SharedRecorder, pid: ProcId) -> usize {
    rec.lock()
        .expect("recorder")
        .verbs_of(IsolateId(pid.0))
        .iter()
        .filter(|v| matches!(v, RmVerb::Free { .. }))
        .count()
}

// =================================================================================
// Mock namespace provenance — how "no host handle of proc A is ever seen by proc B"
// becomes an assertion instead of a hope
// =================================================================================

/// The `(isolate lane, GPU lane)` a mock host handle was minted in. The mock namespaces
/// every handle as `((IsolateId + 1) << 32) | (GpuId << 24) | n` precisely so
/// cross-`(proc, GPU)` reach is *visible* in an assertion rather than merely refused —
/// and `IsolateId == ProcId`, so the first field reads back as `ProcId + 1`.
fn handle_lane(h: u64) -> (u32, u32) {
    ((h >> 32) as u32, ((h >> 24) & 0xff) as u32)
}

/// The `(isolate lane, GPU lane)` a mock host **work-submit token** was minted in: the
/// mock seeds each isolate's token counter at `(idlane << 20) | (GpuId << 8)`, so two
/// isolates of ONE proc on DIFFERENT GPUs mint provably-disjoint tokens (MG-5).
fn token_lane(t: u64) -> (u32, u32) {
    ((t >> 20) as u32, ((t >> 8) & 0xfff) as u32)
}

/// The `(isolate lane, GPU lane)` a mock host **GPU VA** was minted in: the mock places
/// every mapping at `0x4000_0000_0000 + (idlane << 40) + (gpu << 47) + ((vas & 0xff) <<
/// 32) + (page << 12)`, and the two lower fields provably cannot carry into bit 40
/// (`0xff << 32` plus `(2^20 - 1) << 12` is strictly under `1 << 40`), so the provenance
/// of a mapped VA is readable straight off the bits. Bit 46 is the constant base, hence
/// the `0x3f` mask rather than `0x7f`.
fn host_va_lane(va: u64) -> (u32, u32) {
    (((va >> 40) & 0x3f) as u32, ((va >> 47) & 1) as u32)
}

// =================================================================================
// What one composed run reports (the cross-mode differential)
// =================================================================================

/// The mode-INDEPENDENT facts of one composed run. `Degenerate` and `Sharded` are two
/// lock configurations of ONE design, so everything here must match; anything genuinely
/// nondeterministic under concurrency (which thread won a converging materialization
/// race, exact verb counts, minted handle values) is deliberately excluded rather than
/// fudged into a false determinism claim.
#[derive(Debug, PartialEq, Eq)]
struct MeanReport {
    /// The witness's held publish succeeded and its binding read back exactly.
    witness_committed_exactly: bool,
    /// Canary (a): the proc retired inside the gap.
    canary_teardown: Result<Published, FwdFault>,
    /// Canary (b): the routed channel was torn down inside the gap.
    canary_chanfree: Result<DoorbellOutcome, FwdFault>,
    /// Canary (c): an `apply` rewrote routing inside the gap.
    canary_reroute: Result<DoorbellOutcome, FwdFault>,
    /// The scripted mid-chain host failure.
    mid_chain_failure: Result<Published, FwdFault>,
    /// Source signals sent.
    signals: usize,
    /// `Observed` effects the executor produced (must equal `signals`).
    observed: usize,
    /// Whether each multi-threaded proc's own sync thread saw its own completion
    /// delivered while a sibling's verb was parked.
    delivery_ran_under_pending: bool,
    /// Publications committed across the workload threads (fixed: every one must
    /// succeed, so a shortfall means a thread bailed).
    publications: usize,
    /// Procs reaped at the quiesce point, then again.
    reaped: (usize, usize),
}

// =================================================================================
// The workload bodies (§8.4 "mixed concurrent workloads")
// =================================================================================

/// **The alloc/map-heavy control-plane thread.** Publishes backing, reads each
/// publication back through the address table, forwards Case-1 engine objects (and
/// re-sends them, which must be idempotent), forwards a Case-1 control and ACKs a
/// Case-2 one, and churns an RM mapping through `Gpu::apply` — a device **WRITE**,
/// which is the sharpest progress-under-pending probe there is: it can only be taken if
/// the parked verbs hold no lock at all.
fn ctl_workload(device: &SharedDevice, pids: &[ProcId], i: usize) -> usize {
    let (lane, gpu, pid) = (lane_of(i), gpu_of(i), pids[i]);
    let mut host_object: Option<HostHandle> = None;
    let mut mapped = false;
    let mut published = 0usize;
    for k in 0..CTL_OPS {
        let va = GpuVa(VA_CTL + (k as u64) * 0x1000);
        let p = device
            .publish_backing(gpu, lane.pdb, va, 0x1000)
            .expect("the control-plane thread publishes while a sibling's verb is parked");
        published += 1;
        // Read back EXACTLY what THIS thread committed: a sibling's commit must never
        // have landed in this binding (the intra-proc compare-and-swap of §12.9).
        let (binding, off) = device
            .resolve(gpu, lane.pdb, GpuVa(va.0 + 0x40))
            .expect("a just-published VA resolves");
        assert_eq!(
            (binding.phys, binding.host_va(), off),
            (p.gpa, Some(p.host_va), 0x40),
            "another thread's commit landed in this thread's binding"
        );
        assert_eq!(
            host_va_lane(p.host_va),
            (pid.0 + 1, gpu.0),
            "a host VA was minted in another proc's / another GPU's isolate"
        );

        if k.is_multiple_of(5) {
            let f = device
                .forward_engine_object(gpu, lane.gr, kayfabe_tests::COMPUTE_CLASS, &[])
                .expect("Case-1 engine-object forward");
            if let Some(prev) = host_object {
                assert_eq!(
                    f.host_object, prev,
                    "an engine-object re-send must be idempotent — the ORIGINAL host object"
                );
                assert!(f.reused, "the re-send resolved from the idempotency table");
            }
            host_object = Some(f.host_object);
            assert_eq!(
                handle_lane(f.host_object.0),
                (pid.0 + 1, gpu.0),
                "an engine object was minted in another proc's / another GPU's namespace"
            );

            let mut payload = [0u8; 16];
            assert_eq!(
                device.route_control(
                    gpu,
                    pid,
                    f.host_object,
                    mock_ctrl::FORWARDABLE,
                    &mut payload
                ),
                Ok(ControlRoute::Forwarded),
                "a Case-1 control forwards to the host"
            );
            assert_eq!(
                device.route_control(
                    gpu,
                    pid,
                    f.host_object,
                    mock_ctrl::PROMOTE_CTX,
                    &mut payload
                ),
                Ok(ControlRoute::AckOnly),
                "a Case-2 control is ACKed and never leaves the process"
            );
        }

        if k.is_multiple_of(7) {
            // ★ A device WRITE, taken while four host verbs are parked.
            let ev = if mapped {
                RmEvent::Unmap {
                    client: client_of(i),
                    vaspace: H_VASPACE,
                    va: GpuVa(VA_CHURN),
                }
            } else {
                RmEvent::MapMemoryDma {
                    client: client_of(i),
                    vaspace: H_VASPACE,
                    memory: MEM,
                    va: GpuVa(VA_CHURN),
                    offset: 0,
                    len: 0x1000,
                }
            };
            mapped = !mapped;
            device.apply(ev).expect("RM map churn applies");
        }
        assert_eq!(lock::held_depth(), 0, "the control-plane op leaked a guard");
    }
    published
}

/// **The doorbell-heavy submission thread.** Rings GR and CE, asserts the demux landed
/// on its OWN proc (never the same-lane proc on the other GPU) and that the rung host
/// token came out of its OWN `(proc, GPU)` isolate namespace, publishes and then rings
/// WITH that working set so the #14 ring-gate passes, and probes the gate negatively
/// with a VA that was never published — which must be an exact `AddressFault::Miss`,
/// never a guess and never a pass.
fn ring_workload(device: &SharedDevice, pids: &[ProcId], i: usize) -> usize {
    let (lane, gpu, pid) = (lane_of(i), gpu_of(i), pids[i]);
    let mut published = 0usize;
    let mut chan: Option<ChanId> = None;
    for k in 0..RING_OPS {
        let vchid = if k.is_multiple_of(2) {
            lane.gr
        } else {
            lane.ce
        };
        let out = device
            .doorbell(gpu, MockArch::token_for(vchid), &[])
            .expect("the submission thread rings while a sibling's verb is parked");
        assert_eq!(
            out.proc, pid,
            "the doorbell demuxed to the wrong proc — a (GpuId, VChid) routing collapse"
        );
        assert_eq!(
            token_lane(out.host_token),
            (pid.0 + 1, gpu.0),
            "the rung host token came from another proc's / another GPU's isolate"
        );

        if k.is_multiple_of(11) {
            // ★ The #14 ring-gate BITES: an unpublished VA in the working set is a loud
            // address fault, decided before any host op exists.
            assert_eq!(
                device.doorbell(gpu, MockArch::token_for(lane.ce), &[VA_NEVER]),
                Err(FwdFault::Address(AddressFault::Miss {
                    pdb: lane.pdb,
                    va: VA_NEVER
                })),
                "the ring-gate let an unpublished working-set VA through"
            );
        }
        if k.is_multiple_of(8) {
            let va = GpuVa(VA_RING + (k as u64) * 0x1000);
            device
                .publish_backing(gpu, lane.pdb, va, 0x1000)
                .expect("the submission thread publishes its own working set");
            published += 1;
            let gated = device
                .doorbell(gpu, MockArch::token_for(lane.gr), &[va])
                .expect("a published working set passes the gate");
            assert_eq!(gated.proc, pid);
            match chan {
                Some(c) => assert_eq!(c, gated.chan, "the GR channel's identity is stable"),
                None => chan = Some(gated.chan),
            }
        }
        assert_eq!(lock::held_depth(), 0, "the doorbell op leaked a guard");
    }
    published
}

/// **The poll/event-wait-heavy sync thread** — §3.5 guarantee 3: this path needs no
/// worker at all, so it must make progress no matter how saturated the pool is, and its
/// completions cannot sit behind a wedged verb.
///
/// Deliberately the SOLE driver of its GPU's completion plane (poll → drained → pump →
/// drained), so what it observes is a fact about delivery rather than a race with
/// another poller. Returns every completion it saw ride a delivered batch.
fn sync_workload(device: &SharedDevice, pids: &[ProcId], i: usize) -> BTreeSet<OsEventRef> {
    let (gpu, pid) = (gpu_of(i), pids[i]);
    let mut seen = BTreeSet::new();
    for k in 0..SYNC_OPS {
        if let Some(b) = device.completion_poll(gpu, pid, Instant(k as u64)) {
            seen.extend(b.events);
        }
        device.completions_drained(gpu);
        if let Some(b) = device.pump_completions(gpu) {
            seen.extend(b.events);
        }
        device.completions_drained(gpu);
        assert_eq!(lock::held_depth(), 0, "the completion op leaked a guard");
    }
    seen
}

// =================================================================================
// ★★ THE COMPOSED RUN
// =================================================================================

/// One complete mean run under `mode`. Deliberately ONE function read top to bottom:
/// the phase ORDER is the test, and hiding it behind helpers would hide the script.
fn mean_run(mode: LockMode) -> MeanReport {
    let (device, pids, rec) = mean_world(mode);
    let (tx, rx) = inbox();
    let mut ex = Executor::new(Arc::clone(&device), rx);
    let dev: &SharedDevice = &device;
    let pid_ref: &[ProcId] = &pids;
    let handles = identical_handles(0, 0); // the handle VALUES every proc shares

    // ---- Phase 0: warm-up. Every proc materializes its host VAS and one binding, so a
    // canary's held verb is the one the script NAMES and not an incidental first touch.
    // GR channels are left VIRGIN on the canary procs — their held verb IS the channel
    // alloc, which is the verb their staleness script needs parked.
    for i in 0..N_PROCS {
        dev.publish_backing(gpu_of(i), lane_of(i).pdb, GpuVa(VA_WARM), 0x1000)
            .expect("warm-up publish");
        dev.doorbell(gpu_of(i), MockArch::token_for(lane_of(i).ce), &[])
            .expect("warm-up CE ring");
    }

    // ---- Phase 1: register the completion sources (spine writes) up front.
    let src: Vec<_> = [
        (P_WITNESS, EV[0]),
        (P_PEER, EV[1]),
        (P_WITNESS, EV[2]),
        (P_PEER, EV[3]),
    ]
    .into_iter()
    .map(|(i, ev)| {
        dev.register_source(SourceKind::OsEvent {
            proc: pids[i],
            gpu: gpu_of(i),
            ev,
        })
    })
    .collect();
    let hup = dev.register_source(SourceKind::Worker {
        proc: pids[P_HUP],
        gpu: gpu_of(P_HUP),
        worker: WorkerId(0),
    });

    let mut signals = 0usize;
    let mut observed = 0usize;
    let mut publications = 0usize;

    let (witness, canary_teardown, canary_chanfree, canary_reroute, delivery_ran_under_pending) =
        thread::scope(|sc| {
            // ---- Phase 2: park FIVE host verbs — TWO on the witness's own isolate (two
            // of its threads, two pool workers) and one each on three other isolates
            // across both GPUs. Every thread is spawned and CONFIRMED parked before the
            // workloads start, so the slot each latch names is a fact, not a race.
            let mut latches = Latches::new();
            latches.arm(&rec, pids[P_WITNESS], GPU0, 0, VerbKind::AllocSysmem);
            let t_witness = sc.spawn(move || {
                dev.publish_backing(GPU0, lane_of(P_WITNESS).pdb, GpuVa(VA_HELD), 0x1000)
            });
            // ★ Staged on purpose: `checkout` hands out the first IDLE slot, so waiting
            // for worker 0 to be parked before arming worker 1 makes the slot each latch
            // names a FACT rather than a race — and gives the witness TWO of its own
            // threads in flight on TWO workers at once, which is precisely the §7.2
            // "concurrency comes from channel COUNT" claim under test.
            latches.wait_all_pending();
            latches.arm(&rec, pids[P_WITNESS], GPU0, 1, VerbKind::AllocSysmem);
            let t_witness2 = sc.spawn(move || {
                dev.publish_backing(GPU0, lane_of(P_WITNESS).pdb, GpuVa(VA_HELD2), 0x1000)
            });

            latches.arm(&rec, pids[P_TEARDOWN], GPU0, 0, VerbKind::AllocSysmem);
            latches.arm(&rec, pids[P_CHANFREE], GPU1, 0, VerbKind::AllocChannel);
            latches.arm(&rec, pids[P_REROUTE], GPU0, 0, VerbKind::AllocChannel);
            let t_teardown = sc.spawn(move || {
                dev.publish_backing(GPU0, lane_of(P_TEARDOWN).pdb, GpuVa(VA_HELD), 0x1000)
            });
            let t_chanfree = sc.spawn(move || {
                dev.doorbell(GPU1, MockArch::token_for(lane_of(P_CHANFREE).gr), &[])
            });
            let t_reroute = sc
                .spawn(move || dev.doorbell(GPU0, MockArch::token_for(lane_of(P_REROUTE).gr), &[]));

            latches.wait_all_pending(); // progress EDGES, never sleeps

            // ---- Phase 3: DELIVERY RAN. Two completion sources fire while all five
            // verbs are parked; the executor dispatches each into its OWNING proc's queue
            // and no other's. Needs no worker (§3.5 guarantee 3) — which is why it can be
            // asserted deterministically here, before the workloads even start.
            tx.send(CoreEvent::SourceSignal(src[0]));
            tx.send(CoreEvent::SourceSignal(src[1]));
            signals += 2;
            assert_eq!(
                ex.drain_all(),
                vec![
                    Effect::Signal(SignalOutcome::Observed {
                        proc: pids[P_WITNESS],
                        gpu: GPU0,
                        ev: EV[0]
                    }),
                    Effect::Signal(SignalOutcome::Observed {
                        proc: pids[P_PEER],
                        gpu: GPU1,
                        ev: EV[1]
                    }),
                ],
                "({mode:?}) completion delivery must run while host verbs are parked"
            );
            observed += 2;

            // ---- Phase 4: the mixed concurrent workloads. THREE sibling threads of the
            // witness's own `Proc` (§3.5 — the case per-proc sharding cannot help) plus
            // three of the peer's, on the other GPU.
            let mut workers = Vec::new();
            for i in [P_WITNESS, P_PEER] {
                workers.push(sc.spawn(move || ctl_workload(dev, pid_ref, i)));
                workers.push(sc.spawn(move || ring_workload(dev, pid_ref, i)));
            }
            let sync_w = sc.spawn(move || sync_workload(dev, pid_ref, P_WITNESS));
            let sync_p = sc.spawn(move || sync_workload(dev, pid_ref, P_PEER));

            // ---- Phase 5: the world MOVES underneath the parked verbs (R5's canaries
            // and §7.3's worker death), driven from the main thread CONCURRENTLY with
            // phase 4. Every step here is a device WRITE.
            //
            // (a) the process that tears down mid-flight.
            assert!(
                dev.retire_proc(pids[P_TEARDOWN]),
                "({mode:?}) the teardown proc was live when the script retired it"
            );
            // (b) the routed channel is torn down.
            dev.apply(RmEvent::Free {
                client: client_of(P_CHANFREE),
                handle: handles.gr_channel,
            })
            .expect("the channel free applies");
            // (c) routing is REWRITTEN: free + re-alloc at the SAME vChid, so `by_vchid`
            // resolves again but to a DIFFERENT `ChanId` — the case a commit that trusted
            // its pre-gap decision would silently write into the wrong channel.
            dev.apply(RmEvent::Free {
                client: client_of(P_REROUTE),
                handle: handles.gr_channel,
            })
            .expect("the reroute free applies");
            dev.apply(RmEvent::Alloc {
                client: client_of(P_REROUTE),
                parent: handles.tsg,
                handle: H_REALLOC_GR,
                class: mock_classes::CHANNEL_GR,
                facts: AllocFacts {
                    h_vaspace: Some(handles.vaspace),
                    userd_flags: MockArch::userd_flags_for(lane_of(P_REROUTE).gr),
                    ..Default::default()
                },
            })
            .expect("the re-alloc applies");
            // (d) a worker dies out of band → the slot is dead forever and the proc
            // retires loudly (§7.3).
            tx.send(CoreEvent::SourceSignal(hup));
            assert_eq!(
                ex.drain_all(),
                vec![Effect::Signal(SignalOutcome::WorkerDied {
                    proc: pids[P_HUP],
                    gpu: GPU1,
                    worker: WorkerId(0)
                })],
                "({mode:?}) the HUP dispatched as a typed worker death"
            );
            // ...and its sources stop routing IMMEDIATELY: a late signal on the same
            // handle resolves to nothing (the C's F4 use-after-retire, designed out).
            tx.send(CoreEvent::SourceSignal(hup));
            match ex.drain_all().as_slice() {
                [Effect::Signal(SignalOutcome::Fault(f))] => assert_eq!(f.source, hup),
                other => panic!("({mode:?}) a dead proc's source must fault, got {other:?}"),
            }
            // (e) two more completions, mid-flight.
            tx.send(CoreEvent::SourceSignal(src[2]));
            tx.send(CoreEvent::SourceSignal(src[3]));
            signals += 2;
            assert_eq!(
                ex.drain_all(),
                vec![
                    Effect::Signal(SignalOutcome::Observed {
                        proc: pids[P_WITNESS],
                        gpu: GPU0,
                        ev: EV[2]
                    }),
                    Effect::Signal(SignalOutcome::Observed {
                        proc: pids[P_PEER],
                        gpu: GPU1,
                        ev: EV[3]
                    }),
                ],
                "({mode:?}) mid-flight delivery"
            );
            observed += 2;

            // ---- Phase 6: JOIN the workloads. ★★ THE PARALLELISM ASSERTION: these joins
            // returning AT ALL, with five host verbs still parked (two of them on the
            // witness's OWN isolate), IS the #37 invariant. If a verb held any lock, or
            // the pool were #34's single worker, the sibling threads would be blocked
            // behind it and the watchdog would fire — verified by falsification: shrinking
            // the pool to one worker makes this test SIGABRT on its watchdog.
            for w in workers {
                publications += w.join().expect("a workload thread panicked");
            }
            let seen_w = sync_w.join().expect("the witness's sync thread panicked");
            let seen_p = sync_p.join().expect("the peer's sync thread panicked");

            assert!(
                latches.all_pending(),
                "({mode:?}) ★ #37 VIOLATED: the workloads only completed after a parked \
                 verb was released"
            );
            let delivery_ran = seen_w.contains(&EV[0]) && seen_p.contains(&EV[1]);
            assert!(
                delivery_ran,
                "({mode:?}) a proc's OWN poll never observed the completion delivered to \
                 it while its sibling's verb was parked — §3.5 guarantee 3 is broken"
            );

            // ---- Phase 7: release, then read each parked verb's EXACT outcome. Never
            // `is_err()`: the house lesson is that a loose assert lets a canary pass for
            // the wrong reason.
            latches.release_all();
            (
                (
                    t_witness.join().expect("the witness thread joins"),
                    t_witness2.join().expect("the witness's 2nd thread joins"),
                ),
                t_teardown.join().expect("the teardown thread joins"),
                t_chanfree.join().expect("the chanfree thread joins"),
                t_reroute.join().expect("the reroute thread joins"),
                delivery_ran,
            )
        });

    // ★ BOTH held commits landed CORRECTLY and re-validated: each binding the address
    // table holds is bit-for-bit the one THAT commit computed — the two sibling threads
    // of one `Proc` did not overwrite each other, and neither carried a stale decision
    // across its gap.
    let (w1, w2) = witness;
    let w1 = w1.expect("the witness's 1st held publish must commit");
    let w2 = w2.expect("the witness's 2nd held publish must commit");
    assert_ne!(w1.gpa, w2.gpa, "({mode:?}) two commits shared a GPA");
    assert_ne!(w1.host_va, w2.host_va, "({mode:?}) two commits shared a VA");
    let mut witness_committed_exactly = true;
    for (va, p) in [(VA_HELD, w1), (VA_HELD2, w2)] {
        let (binding, off) = dev
            .resolve(GPU0, lane_of(P_WITNESS).pdb, GpuVa(va + 0x40))
            .expect("the released verb's VA resolves");
        witness_committed_exactly &=
            (binding.phys, binding.host_va(), off) == (p.gpa, Some(p.host_va), 0x40);
    }
    assert!(
        witness_committed_exactly,
        "({mode:?}) a released verb's commit did not write the binding it computed"
    );

    // ---- Phase 8: the scripted mid-chain HOST FAILURE (§8.2's "failure mid-verb"), run
    // quiesced so the injection is deterministic: hold the MAP verb of a fresh
    // publication, arm `fail_next` while it is parked, release. The chain then fails on
    // its SECOND verb, with a host memory object already allocated — so this pins three
    // things at once: the op refuses, it mutated nothing, and `Worker::execute` unwound
    // its own intermediate (exactly one host `free`, no leak, no double-free). It also
    // pins §12.10's POLARITY: for a LIVE proc the fault is the RM error itself, and
    // staleness is claimed only when the world actually moved.
    let frees_before = count_frees(&rec, pids[P_WITNESS]);
    let mid_chain_failure = {
        let h_map = hold(&rec, pids[P_WITNESS], GPU0, 0, VerbKind::MapGpuVa);
        let out = thread::scope(|sc| {
            let t = sc.spawn(move || {
                dev.publish_backing(GPU0, lane_of(P_WITNESS).pdb, GpuVa(VA_FAIL), 0x1000)
            });
            h_map.wait_until_pending();
            rec.lock().expect("recorder").fail_next = Some(RmError::Other(0x5EED));
            h_map.release();
            t.join().expect("the failing publish thread joins")
        });
        assert_eq!(
            out,
            Err(FwdFault::Rm(RmError::Other(0x5EED))),
            "({mode:?}) a LIVE proc's host failure must surface as the RM error itself"
        );
        assert_eq!(
            dev.resolve(GPU0, lane_of(P_WITNESS).pdb, GpuVa(VA_FAIL)),
            Err(FwdFault::Address(AddressFault::Miss {
                pdb: lane_of(P_WITNESS).pdb,
                va: GpuVa(VA_FAIL)
            })),
            "({mode:?}) the failed publication mutated nothing"
        );
        assert_eq!(
            count_frees(&rec, pids[P_WITNESS]) - frees_before,
            1,
            "({mode:?}) the mid-chain failure must release the sysmem object it had \
             already allocated — exactly one host free"
        );
        out
    };

    // ★ Non-vacuity: R1 and R3 are always-on asserts, so "they ran" is only as strong as
    // the number of times the run crossed them. Every entry in this log is one
    // `Worker::execute`, whose FIRST statement is the R1 witness assert; the ranked-lock
    // acquisitions behind them (strictly more, at two per rank per verb-issuing op) each
    // ran the R3 rank check. A run that somehow stopped exercising the verb path would
    // pass every other assertion here vacuously — this is the floor that stops it.
    let verbs = rec.lock().expect("recorder").log.len();
    assert!(
        verbs > 10_000,
        "({mode:?}) the run must actually exercise the host-verb path (R1's assert fires \
         once per verb); only {verbs} verbs were issued"
    );

    // ---- Phase 9: ONE deterministic refresh, so the §12.13 property the sweep pins —
    // that a refresh does NOT resurrect an out-of-band-retired component — is a fact of
    // every run rather than a race with the workload threads' own churn.
    dev.apply(RmEvent::MapMemoryDma {
        client: client_of(P_WITNESS),
        vaspace: H_VASPACE,
        memory: MEM,
        va: GpuVa(VA_CHURN + 0x10_0000),
        offset: 0,
        len: 0x1000,
    })
    .expect("the final refresh applies");

    // ---- Phase 10: quiesce the completion plane, then reap at the declared quiesce
    // point. Exactly two procs retired during the run (the teardown and the HUP victim).
    for g in [GPU0, GPU1] {
        loop {
            dev.completions_drained(g);
            if dev.pump_completions(g).is_none() {
                break;
            }
        }
        dev.completions_drained(g);
    }
    let reaped = (dev.reap_retired(), dev.reap_retired());
    assert_eq!(
        reaped,
        (2, 0),
        "({mode:?}) exactly the two retired procs reaped, and never again"
    );
    assert_eq!(lock::held_depth(), 0, "the main thread leaked no guard");

    // ---- Phase 11: conservation.
    drop(ex);
    let mut gpu = Arc::try_unwrap(device)
        .unwrap_or_else(|_| panic!("every device handle was released"))
        .into_gpu();
    sweep_conservation(&mut gpu, &pids, mode);

    MeanReport {
        witness_committed_exactly,
        canary_teardown,
        canary_chanfree,
        canary_reroute,
        mid_chain_failure,
        signals,
        observed,
        delivery_ran_under_pending,
        publications,
        reaped,
    }
}

// =================================================================================
// The end-of-run conservation sweep
// =================================================================================

/// What a routing key resolves to — the THREE answers the device can give after
/// §12.13, made a value so the sweep can `assert_eq!` on the exact one instead of
/// `contains_key`-ing around it.
#[derive(Debug, PartialEq, Eq)]
enum Owner {
    /// A live proc owns it.
    Live(ProcId),
    /// It belongs to a **condemned** component: an out-of-band worker death killed the
    /// component for good, so there is no proc to name — only its label (§12.13).
    Condemned(ProcAnchor),
    /// Nothing declared it (or the guest freed it).
    Absent,
}

/// The owner of `(gpu, pdb)` on the data plane.
fn owner_pdb(g: &Gpu, target: GpuId, pdb: Pdb) -> Owner {
    match g.spine.by_pdb.get(&(target, pdb)) {
        Some(&pid) => Owner::Live(pid),
        None => match g.spine.condemned_pdb(target, pdb) {
            Some(a) => Owner::Condemned(a),
            None => Owner::Absent,
        },
    }
}

/// The owner of `(gpu, vchid)` on the exec plane.
fn owner_vchid(g: &Gpu, target: GpuId, vchid: VChid) -> Owner {
    match g.spine.by_vchid.get(&(target, vchid)) {
        Some(&(pid, _)) => Owner::Live(pid),
        None => match g.spine.condemned_vchid(target, vchid) {
            Some(a) => Owner::Condemned(a),
            None => Owner::Absent,
        },
    }
}

/// The owner proc `i`'s keys MUST have at the end of a run: condemned for the two procs
/// the script killed out of band, live for the four it did not. Each proc is its own
/// one-client component, so its anchor is its client.
fn expected_owner(pids: &[ProcId], i: usize) -> Owner {
    if i == P_TEARDOWN || i == P_HUP {
        Owner::Condemned(ProcAnchor(client_of(i)))
    } else {
        Owner::Live(pids[i])
    }
}

/// The facts that must hold globally, no matter how the threads interleaved. Each is a
/// property the design *claims*; asserting them here is what makes the claim falsifiable
/// by a harsh run instead of by inspection.
fn sweep_conservation(gpu: &mut Gpu, pids: &[ProcId], mode: LockMode) {
    // ---- (1) ★ Per-`(GpuId, ·)` routing NEVER collapses. Each lane's PDB and CE vChid
    // exist IDENTICALLY on both GPUs and must resolve to two DIFFERENT owners — where
    // "owner" now has three possible answers ([`Owner`]), because two of this run's six
    // procs were retired out of band and are CONDEMNED (§12.13).
    //
    // ★ That makes this a sharper multi-GPU probe than it was: lanes B and C each have
    // one condemned member and one healthy member, on OPPOSITE GPUs, carrying identical
    // `Pdb`/`VChid` VALUES. If condemnation were keyed on anything that dropped the
    // `GpuId` — or on a numeric identity rather than on the component's client set —
    // condemning proc 2 on GPU0 would take proc 3 down on GPU1, and condemning proc 5 on
    // GPU1 would take proc 4 down on GPU0. Both are asserted below, per lane.
    for (l, lane) in LANES.iter().enumerate() {
        let (a, b) = (
            owner_pdb(gpu, GPU0, lane.pdb),
            owner_pdb(gpu, GPU1, lane.pdb),
        );
        assert_ne!(
            a, b,
            "({mode:?}) identical PDBs on two GPUs collapsed onto one owner"
        );
        let (ca, cb) = (
            owner_vchid(gpu, GPU0, lane.ce),
            owner_vchid(gpu, GPU1, lane.ce),
        );
        assert_ne!(
            ca, cb,
            "({mode:?}) identical vChids on two GPUs collapsed onto one owner"
        );
        assert_eq!((ca, cb), (a, b), "({mode:?}) by_pdb and by_vchid disagree");
        // …and each half is EXACTLY the owner its script earned: live for the four procs
        // nothing happened to, condemned for the two the run killed out of band.
        for (g, i) in [(GPU0, 2 * l), (GPU1, 2 * l + 1)] {
            let want = expected_owner(pids, i);
            assert_eq!(
                owner_pdb(gpu, g, lane.pdb),
                want,
                "({mode:?}) proc {i}'s {g:?} PDB resolves to the wrong owner"
            );
            assert_eq!(
                owner_vchid(gpu, g, lane.ce),
                want,
                "({mode:?}) proc {i}'s {g:?} CE vChid resolves to the wrong owner"
            );
        }
    }

    // ---- (2) The torn-down channel is GONE from routing; the rewritten one resolves
    // again (to the re-allocated channel, whose `ChanId` the refused commit never named).
    assert!(
        !gpu.spine
            .by_vchid
            .contains_key(&(GPU1, lane_of(P_CHANFREE).gr)),
        "({mode:?}) a freed channel still routes"
    );
    assert!(
        gpu.spine
            .by_vchid
            .contains_key(&(GPU0, lane_of(P_REROUTE).gr)),
        "({mode:?}) the re-allocated channel must route again"
    );

    // ---- (3) ★★ NO RESURRECT (`l1_concurrency.md` §12.13 — FIXED; the un-ignored
    // `out_of_band_retire_must_not_resurrect_the_isolate` below is the focused proof).
    //
    // This block used to pin the DEFECT: an out-of-band-retired proc left the live set,
    // but its client root was still in the guest's graph, so the next `refresh` found a
    // boundary with no matching live proc and minted a brand-new `ProcId` with a
    // brand-new isolate (a respawned sandboxed worker) and a fresh arena. Both victims
    // were BACK, under new identities, before the sweep ran. It was pinned deliberately
    // so that fixing it would trip this test — and it did: the fix panicked here first.
    //
    // What it pins now is the fixed behavior, at the end of a run that put the two
    // out-of-band retires and thousands of `apply`s from OTHER clients in the same
    // window. The condemned components must be dead, and dead *as components* — not one
    // proc's worth of state left behind by accident.
    for &i in &[P_TEARDOWN, P_HUP] {
        assert!(
            !gpu.procs.contains_key(&pids[i]),
            "({mode:?}) the retired proc's ORIGINAL identity is gone (it was reaped)"
        );
        assert!(
            gpu.spine.is_condemned(client_of(i)),
            "★★ ({mode:?}) §12.13: proc {i} was retired out of band, so its component \
             must still be condemned after the run's applies"
        );
        // No proc, under ANY identity, holds this component's client — the resurrection
        // this run used to produce would be visible right here as a fresh `ProcId`.
        assert!(
            gpu.procs
                .values()
                .all(|p| !p.clients.contains(&client_of(i))),
            "★★ ({mode:?}) §12.13: an out-of-band-retired proc was RESURRECTED under a \
             fresh identity — §7.3 promises 'no resurrect' and `WorkerDied` promises \
             'never a respawn'"
        );
    }
    assert_eq!(
        gpu.spine.condemned_len(),
        2,
        "({mode:?}) exactly the two out-of-band retires condemned a component"
    );
    // ★ No FALSE condemnation: the four procs the run did not kill out of band are all
    // still live and none of them is condemned. A fix that condemned too eagerly (say,
    // on the graph-driven retire of `P_CHANFREE`'s channel, or on the `LateMerge` absorb
    // path) would be a self-inflicted denial of service, and it would show up here.
    assert_eq!(
        gpu.procs.len(),
        4,
        "({mode:?}) exactly the four untouched procs are live"
    );
    for i in [P_WITNESS, P_PEER, P_CHANFREE, P_REROUTE] {
        assert!(
            gpu.procs.contains_key(&pids[i]),
            "({mode:?}) proc {i} was never retired and must still be live"
        );
        assert!(
            !gpu.spine.is_condemned(client_of(i)),
            "({mode:?}) proc {i} was condemned by a host failure that was not its own"
        );
    }
    assert!(
        gpu.spine.retired.is_empty(),
        "({mode:?}) the reap left retired procs behind"
    );

    // ---- (4) Completion conservation: exactly the events each proc OWNED reached it,
    // nothing reached a proc that armed nothing, and the queues empty out on ack.
    assert!(
        !gpu.system.completion.has_outstanding(),
        "({mode:?}) a userspace completion landed in the SYSTEM proc's queue"
    );
    for i in [P_CHANFREE, P_REROUTE] {
        assert!(
            !gpu.procs[&pids[i]].completion.has_outstanding(),
            "({mode:?}) proc {i} received a completion it never armed"
        );
    }
    for i in [P_WITNESS, P_PEER] {
        let p = gpu.procs.get_mut(&pids[i]).expect("live");
        p.completion.ack(EV[i]);
        p.completion.ack(EV[i + 2]);
        assert!(
            !p.completion.has_outstanding(),
            "({mode:?}) proc {i}'s queue held something other than the two completions \
             signalled for it — a completion was lost, duplicated or misrouted"
        );
    }

    // ---- (5) GPA arenas are pairwise disjoint — GLOBALLY, not merely per GPU, because
    // each target's window is itself disjoint (MG-5 / the #80 recycle). Overlap here is
    // the #14 collision class returning.
    let mut arenas: Vec<(ProcId, GpuId, Range<u64>)> = Vec::new();
    for (&pid, p) in &gpu.procs {
        for (&g, a) in &p.arenas {
            arenas.push((pid, g, a.range.clone()));
        }
    }
    for (&g, a) in &gpu.system.arenas {
        arenas.push((Gpu::SYSTEM_PROC, g, a.range.clone()));
    }
    for (i, (pa, ga, a)) in arenas.iter().enumerate() {
        for (pb, gb, b) in arenas.iter().skip(i + 1) {
            assert!(
                a.end <= b.start || b.end <= a.start,
                "({mode:?}) GPA arenas overlap: {pa:?}/{ga:?} {a:?} vs {pb:?}/{gb:?} {b:?}"
            );
        }
    }

    // ---- (6) ★ Host-handle, host-VA and GPA provenance, per proc, per GPU. The mock
    // namespaces every minted identity by `(isolate, GPU)`, so this is a DIRECT
    // assertion that no host handle minted for proc A is ever observed inside proc B's
    // state, and that a proc's GPU0 isolate never leaks into its GPU1 state (MG-5's
    // blast-radius boundary), plus: every published GPA came from that proc's own arena
    // for that very target.
    let mut owner_of: BTreeMap<u64, ProcId> = BTreeMap::new();
    for (&pid, p) in &gpu.procs {
        let lane = pid.0 + 1;
        for (&(g, _pdb), vas) in &p.vases {
            if let Some(h) = vas.host_vas {
                assert_eq!(
                    handle_lane(h.0),
                    (lane, g.0),
                    "({mode:?}) {pid:?} holds a host VAS from another (proc, GPU)"
                );
                assert_eq!(
                    owner_of.insert(h.0, pid),
                    None,
                    "({mode:?}) one host handle is observed by two procs"
                );
            }
            for (_va, _len, b) in vas.table.iter() {
                let Some(hv) = b.host_va() else { continue };
                assert_eq!(
                    host_va_lane(hv),
                    (lane, g.0),
                    "({mode:?}) {pid:?} holds a host VA from another (proc, GPU)"
                );
                let r = p
                    .arenas
                    .get(&g)
                    .map(|a| a.range.clone())
                    .expect("a published binding implies a materialized arena");
                assert!(
                    r.contains(&b.phys),
                    "({mode:?}) {pid:?} published GPA {:#x} outside its own {g:?} arena {r:?}",
                    b.phys
                );
            }
        }
        for c in p.channels.values() {
            if let Some(h) = c.host_channel {
                assert_eq!(
                    handle_lane(h.0),
                    (lane, c.gpu.0),
                    "({mode:?}) {pid:?} holds a host channel from another (proc, GPU)"
                );
                assert_eq!(
                    owner_of.insert(h.0, pid),
                    None,
                    "({mode:?}) one host handle is observed by two procs"
                );
            }
            if let Some(t) = c.host_token {
                assert_eq!(
                    token_lane(t),
                    (lane, c.gpu.0),
                    "({mode:?}) {pid:?} holds a host token from another (proc, GPU)"
                );
            }
            for h in c.host_engine_objects.values() {
                assert_eq!(
                    handle_lane(h.0),
                    (lane, c.gpu.0),
                    "({mode:?}) {pid:?} holds an engine object from another (proc, GPU)"
                );
                assert_eq!(
                    owner_of.insert(h.0, pid),
                    None,
                    "({mode:?}) one host handle is observed by two procs"
                );
            }
        }
    }

    // ---- (7) Routing agrees with the graph: every route names a live channel, on the
    // GPU and vChid its key claims.
    for (&(g, vchid), &(pid, cid)) in &gpu.spine.by_vchid {
        let p = gpu
            .procs
            .get(&pid)
            .unwrap_or_else(|| panic!("({mode:?}) routing names a proc that is not live"));
        let c = p
            .channels
            .get(&cid)
            .unwrap_or_else(|| panic!("({mode:?}) routing names a channel that does not exist"));
        assert_eq!(
            (c.gpu, c.vchid),
            (g, vchid),
            "({mode:?}) routing key disagrees with the channel it names"
        );
    }
}

// =================================================================================
// The tests
// =================================================================================

/// ★★ **THE MEAN TEST** — the composed run, under BOTH lock configurations, with the
/// mode-independent facts required to match.
///
/// Every assertion inside [`mean_run`] and [`sweep_conservation`] fires in both modes;
/// what this outer test adds is the differential: `Degenerate` (every op write-locks the
/// device — L1-M1's shipping shape) and `Sharded` (device-read + per-proc lock — the
/// #14-gate shape) must agree on the canary faults, the conservation counts and the
/// reap, or the granularity flip is not a configuration change (§8.2, review P5).
#[test]
fn mean_multiproc_multithread_multigpu_multiworkload() {
    let _wd = watchdog(
        "mean_multiproc_multithread_multigpu_multiworkload",
        Duration::from_secs(300),
    );
    // The pids are graph-derived and identical in both worlds; resolve them once so the
    // expected `Stale::Proc` payload is derived rather than guessed.
    let (_probe, pids, _rec) = mean_world(LockMode::Degenerate);

    let degenerate = mean_run(LockMode::Degenerate);
    let sharded = mean_run(LockMode::Sharded);

    for (name, r) in [("Degenerate", &degenerate), ("Sharded", &sharded)] {
        // ---- the four parked verbs' outcomes, EXACT (§8.4's canaries).
        assert!(
            r.witness_committed_exactly,
            "({name}) the witness's held publish must commit a correct, re-validated result"
        );
        assert_eq!(
            r.canary_teardown,
            Err(FwdFault::Stale(Stale::Proc(pids[P_TEARDOWN]))),
            "({name}) R5 canary (a): a commit whose proc vanished must refuse AS \
             STALENESS — not as an incidental RM error (the stage-3 house lesson)"
        );
        assert_eq!(
            r.canary_chanfree,
            Err(FwdFault::Stale(Stale::Route {
                gpu: GPU1,
                vchid: lane_of(P_CHANFREE).gr
            })),
            "({name}) R5 canary (b): a commit whose channel was torn down must refuse, \
             naming the route it can no longer resolve"
        );
        assert_eq!(
            r.canary_reroute,
            Err(FwdFault::Stale(Stale::Route {
                gpu: GPU0,
                vchid: lane_of(P_REROUTE).gr
            })),
            "({name}) R5 canary (c): a commit whose route was rewritten must re-resolve \
             and refuse — a pre-gap decision must never be carried across the gap"
        );
        assert_eq!(
            r.mid_chain_failure,
            Err(FwdFault::Rm(RmError::Other(0x5EED))),
            "({name}) the scripted mid-chain host failure surfaces as itself"
        );

        // ---- conservation.
        assert_eq!(
            r.signals, r.observed,
            "({name}) every source signal produced exactly one Observed — no completion \
             lost, none duplicated"
        );
        assert_eq!(
            r.signals, 4,
            "({name}) the script signalled four completions"
        );
        assert_eq!(
            r.publications,
            2 * (CTL_OPS + RING_OPS.div_ceil(8)),
            "({name}) every workload publication committed — a shortfall means a thread \
             bailed out of its loop"
        );
        assert_eq!(r.reaped, (2, 0), "({name}) reaped once, never again");
        assert!(
            r.delivery_ran_under_pending,
            "({name}) delivery must have run while host verbs were parked"
        );
    }

    // ★ The differential: the lock configuration must not be observable in any of it.
    assert_eq!(
        degenerate, sharded,
        "the lock configuration is observable through the API — the late granularity \
         flip (§8.2 / P5) is NOT a configuration change"
    );
}

/// ★★ **`l1_concurrency.md` §12.13 — THE DESIGN'S OWN INVARIANT. Was `#[ignore]`d with
/// the defect named; the condemned-component fix makes it pass on its own terms.**
///
/// §7.3: *"Worker death out-of-band (crash) is a reactor source firing HUP → dispatch →
/// retire the proc loudly … either way MISS=FAULT posture, **no resurrect**."*
/// [`SignalOutcome::WorkerDied`]: *"the slot is permanently dead (**never a respawn**)"*
/// — because the guest's published data lived in host memory owned by that isolate's RM
/// client and died with it, so a resurrect would serve **zeroes** for a VA the guest
/// believes still holds its data (§7.3, "Why a condemned component is never
/// resurrected"; the recovery half is pinned by the tests at the end of this file).
///
/// What used to happen: `Spine::retire_proc` removed the proc from the live set, but
/// the guest's client root is untouched in the RM graph, so the **next `Gpu::apply`**
/// re-derived that boundary, found no matching live proc, minted a fresh `ProcId`,
/// spawned a **brand-new isolate** (new sandbox, new handle namespace — the respawn
/// §7.3 forbids), carved a fresh GPA arena, and rebuilt `by_pdb`/`by_vchid` onto it.
/// The guest then published and rang again with no refusal whatsoever. Only the dead
/// *worker slot* stayed dead; the isolate came back around it.
///
/// ★ **The one assert that changed, and why — read this before trusting the test.**
/// The two post-refresh expectations were written as `RetiredProc(victim)`, and that
/// exact value is now *unrepresentable by construction*: §12.13's own analysis said so
/// ("there is no `Proc` left to name in a `RetiredProc` fault"), and any fix that could
/// still produce it would have to keep routing pointing at a dead `ProcId` — which the
/// mean run's own conservation sweep, step (7), forbids ("routing names a proc that is
/// not live"). So the expectation is now [`FwdFault::Condemned`], carrying the
/// component's `ProcAnchor`. That is a **strengthening**, not a weakening: the old value
/// said only "the proc you named is not live" (which a *reaped* proc and a *condemned*
/// one both satisfy); the new one says "this component is condemned and will not be
/// served again", and it is derived FORWARD out of the same projection that fills
/// `by_pdb`/`by_vchid` — no reverse resolve was invented to make a prettier error. The
/// property under test — *the guest gets a refusal, not service* — is untouched, and
/// the pre-refresh assert is now the same fault as the post-refresh one, which is
/// §12.11's "the two teardown routes are not fault-identical" being paid off.
#[test]
fn out_of_band_retire_must_not_resurrect_the_isolate() {
    let _wd = watchdog(
        "out_of_band_retire_must_not_resurrect",
        Duration::from_secs(60),
    );
    let (device, pids, _rec) = mean_world(LockMode::Sharded);
    let victim = pids[P_HUP];
    let condemned = FwdFault::Condemned {
        anchor: ProcAnchor(client_of(P_HUP)),
    };

    // The victim has live host state, then its worker dies out of band.
    device
        .publish_backing(GPU1, lane_of(P_HUP).pdb, GpuVa(VA_WARM), 0x1000)
        .expect("victim publishes");
    let hup = device.register_source(SourceKind::Worker {
        proc: victim,
        gpu: GPU1,
        worker: WorkerId(0),
    });
    assert_eq!(
        device.signal_source(hup),
        SignalOutcome::WorkerDied {
            proc: victim,
            gpu: GPU1,
            worker: WorkerId(0)
        }
    );
    assert_eq!(
        device.publish_backing(GPU1, lane_of(P_HUP).pdb, GpuVa(VA_HELD), 0x1000),
        Err(condemned),
        "immediately after the HUP the proc refuses, loudly"
    );

    // ★ One unrelated RM event — anything at all, from ANY client — runs a refresh.
    device
        .apply(RmEvent::MapMemoryDma {
            client: client_of(P_WITNESS),
            vaspace: H_VASPACE,
            memory: MEM,
            va: GpuVa(VA_CHURN),
            offset: 0,
            len: 0x1000,
        })
        .expect("an unrelated map applies");

    // ★★ THE INVARIANT (§7.3): the condemned component stays dead across the refresh.
    assert_eq!(
        device.publish_backing(GPU1, lane_of(P_HUP).pdb, GpuVa(VA_HELD), 0x1000),
        Err(condemned),
        "★★ §12.13: a refresh RESURRECTED an out-of-band-retired proc on a fresh isolate \
         — §7.3 promises no resurrect and WorkerDied promises never a respawn"
    );
    assert_eq!(
        device.doorbell(GPU1, MockArch::token_for(lane_of(P_HUP).gr), &[]),
        Err(condemned),
        "★★ §12.13: and its channels serve the guest again"
    );
    // The refusal is not the whole claim: nothing was MATERIALIZED for it either. No
    // proc holds its client, so there is no isolate and no arena behind that refusal.
    let gpu = Arc::try_unwrap(device)
        .unwrap_or_else(|_| panic!("every device handle was released"))
        .into_gpu();
    assert!(
        gpu.procs
            .values()
            .all(|p| !p.clients.contains(&client_of(P_HUP))),
        "★★ §12.13: no `Proc` — under any identity — may hold a condemned client"
    );
    assert!(gpu.spine.is_condemned(client_of(P_HUP)));
}

// =================================================================================
// ★ §12.13 — the properties the condemned-component fix INTRODUCES
//
// Deliberately driven against a bare `Gpu` (`mean_gpu`) rather than through the lock
// shell: condemnation is a fact of the pure core, so these read core state directly —
// client sets, arenas, the source registry — instead of inferring it from refusals.
// `SharedDevice::retire_proc` is a thin wrapper over `Spine::retire_proc`, which is
// the ONE out-of-band retire path and therefore the one place condemnation is recorded.
// =================================================================================

/// Publish one page through the composed core path, routing the way a guest does
/// (`by_pdb` → owning proc). Returns the routing refusal verbatim.
fn core_publish(gpu: &mut Gpu, target: GpuId, pdb: Pdb, va: u64) -> Result<Published, FwdFault> {
    let pid = kayfabe_fwd::route_pdb(&gpu.spine, target, pdb)?;
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    kayfabe_fwd::publish_backing(proc, target, pdb, GpuVa(va), 0x1000)
}

/// Retire proc `pid` **out of band** on the bare core — the worker-death edge
/// (`Spine::retire_proc`), which is what condemns the component.
fn core_retire_out_of_band(gpu: &mut Gpu, pid: ProcId) -> bool {
    let Gpu { spine, procs, .. } = gpu;
    spine.retire_proc(procs, pid)
}

/// Churn one RM map/unmap through `apply` from `client` — a full `refresh`, driven by
/// a client that has nothing to do with the condemned component.
fn churn(gpu: &mut Gpu, client: HClient, k: usize) {
    let ev = if k.is_multiple_of(2) {
        RmEvent::MapMemoryDma {
            client,
            vaspace: H_VASPACE,
            memory: MEM,
            va: GpuVa(VA_CHURN),
            offset: 0,
            len: 0x1000,
        }
    } else {
        RmEvent::Unmap {
            client,
            vaspace: H_VASPACE,
            va: GpuVa(VA_CHURN),
        }
    };
    gpu.apply(ev).expect("the churn applies");
}

/// ★ **Condemnation survives an arbitrary number of intervening `apply`s from other
/// clients, and clears ONLY when the guest frees the condemned client root** — after
/// which a genuinely new process is served normally.
///
/// The defect was precisely that ONE unrelated `apply` undid the retire, so the number
/// of intervening refreshes is the load-bearing variable: this drives 64 of them, from
/// a *different* client, and requires the refusal to be identical every time. The
/// clearing half is the other side of the same rule — condemnation is not a permanent
/// poisoning of a `Pdb` value, it is the death of one component, and it ends when the
/// guest itself lets go of it (§12.13 rule 3).
#[test]
fn condemnation_survives_intervening_applies_and_clears_on_client_root_free() {
    let _wd = watchdog("condemnation_survives", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let (victim, lane) = (pids[P_HUP], lane_of(P_HUP));
    let condemned = FwdFault::Condemned {
        anchor: ProcAnchor(client_of(P_HUP)),
    };

    core_publish(&mut gpu, GPU1, lane.pdb, VA_WARM).expect("the victim publishes while alive");
    assert!(core_retire_out_of_band(&mut gpu, victim));

    for k in 0..64 {
        churn(&mut gpu, client_of(P_WITNESS), k);
        assert_eq!(
            core_publish(&mut gpu, GPU1, lane.pdb, VA_HELD + (k as u64) * 0x1000),
            Err(condemned),
            "★ refresh #{k} resurrected the condemned component"
        );
        assert_eq!(
            kayfabe_fwd::handle_doorbell(&mut gpu, GPU1, MockArch::token_for(lane.gr), &[]),
            Err(condemned),
            "★ refresh #{k} resurrected the condemned component's exec plane"
        );
        assert!(gpu.spine.is_condemned(client_of(P_HUP)));
        assert_eq!(gpu.spine.condemned_len(), 1);
        assert!(
            gpu.procs
                .values()
                .all(|p| !p.clients.contains(&client_of(P_HUP))),
            "★ refresh #{k} minted a `Proc` for the condemned component"
        );
    }

    // ---- The guest frees the client root: condemnation ends WITH the component.
    let h = identical_handles(lane.gr.0, lane.ce.0);
    gpu.apply(RmEvent::Free {
        client: client_of(P_HUP),
        handle: h.client_root,
    })
    .expect("the client-root free applies");
    assert!(
        !gpu.spine.is_condemned(client_of(P_HUP)),
        "condemnation must be dropped once no boundary holds the condemned clients"
    );
    assert_eq!(gpu.spine.condemned_len(), 0);
    assert_eq!(
        core_publish(&mut gpu, GPU1, lane.pdb, VA_HELD),
        Err(FwdFault::UnknownPdb {
            gpu: GPU1,
            pdb: lane.pdb
        }),
        "with the component gone the key is simply unknown — not condemned"
    );

    // ---- …and a genuinely NEW process (fresh client, same recycled PDB value) is
    // served normally. Condemnation must never have poisoned the VALUE.
    let fresh = HClient(0xC0);
    let mut s = Scenario::new();
    s.compute_process_on_gpu(
        fresh,
        lane.pdb,
        identical_handles(lane.gr.0, lane.ce.0),
        Some(1),
    );
    for ev in s.events {
        gpu.apply(ev).expect("the new process applies");
    }
    let p = core_publish(&mut gpu, GPU1, lane.pdb, VA_CTL).expect("a NEW process is served");
    let pid = gpu.spine.by_pdb[&(GPU1, lane.pdb)];
    assert!(gpu.procs[&pid].clients.contains(&fresh));
    assert_ne!(pid, victim, "ProcIds are never reused");
    assert!(p.host_va != 0);
}

/// ★ **A condemned component leaves NO dispatchable completion source** — and none
/// appears later.
///
/// `Spine::retire_proc` already deregisters every source of the dying proc (the C's F4
/// use-after-retire, designed out). What the fix adds is that this stays true: with no
/// `Proc` ever minted for the component again, there is nothing left to register a
/// source ON, so the registry cannot re-acquire one across any number of refreshes.
#[test]
fn a_condemned_component_leaves_no_dispatchable_completion_source() {
    let _wd = watchdog("condemned_no_sources", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let victim = pids[P_HUP];

    let os = gpu.spine.sources.register(SourceKind::OsEvent {
        proc: victim,
        gpu: GPU1,
        ev: EV[0],
    });
    let worker = gpu.spine.sources.register(SourceKind::Worker {
        proc: victim,
        gpu: GPU1,
        worker: WorkerId(0),
    });
    let bystander = gpu.spine.sources.register(SourceKind::OsEvent {
        proc: pids[P_PEER],
        gpu: GPU1,
        ev: EV[1],
    });
    assert!(gpu.spine.sources.dispatch(os.0).is_ok());

    assert!(core_retire_out_of_band(&mut gpu, victim));
    for k in 0..8 {
        churn(&mut gpu, client_of(P_WITNESS), k);
        assert!(
            gpu.spine.sources.dispatch(os.0).is_err(),
            "a condemned component's os-event source dispatched after refresh #{k}"
        );
        assert!(
            gpu.spine.sources.dispatch(worker.0).is_err(),
            "a condemned component's worker source dispatched after refresh #{k}"
        );
        assert!(
            gpu.spine
                .sources
                .iter()
                .all(|(_, kind)| kind.owner() != Some(victim)),
            "the registry still names the condemned proc after refresh #{k}"
        );
        // Blast-radius containment: the untouched peer's source is unaffected.
        assert!(gpu.spine.sources.dispatch(bystander.0).is_ok());
    }
}

/// ★ **A condemned component's GPA arena is released EXACTLY once, and is never handed
/// back to a resurrected impostor.**
///
/// The #80 recycle (`Spine::reap_retired` → `GpaSpace::release`) is what makes sequential
/// process churn sustainable, and it must keep working for an out-of-band retire — but
/// the recycled range must go to a genuinely NEW guest process, never to a re-derivation
/// of the component that lost its worker. This pins both halves: reaped once (never
/// again), and the exact released range reappears under a *different client's* proc while
/// the condemned component still holds no arena at all.
#[test]
fn a_condemned_components_arena_is_released_once_and_never_recycled_to_an_impostor() {
    let _wd = watchdog("condemned_arena", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let (victim, lane) = (pids[P_HUP], lane_of(P_HUP));

    core_publish(&mut gpu, GPU1, lane.pdb, VA_WARM).expect("the victim publishes while alive");
    let arena = gpu.procs[&victim].arenas[&GPU1].range.clone();

    assert!(core_retire_out_of_band(&mut gpu, victim));
    assert_eq!(gpu.reap_retired().len(), 1, "reaped exactly once");
    assert_eq!(gpu.reap_retired().len(), 0, "and never again");

    // No refresh may carve an arena for the condemned component — there is no proc to
    // carve one for, which is the point.
    for k in 0..16 {
        churn(&mut gpu, client_of(P_WITNESS), k);
        assert!(
            gpu.procs
                .values()
                .all(|p| !p.clients.contains(&client_of(P_HUP))),
            "refresh #{k} gave the condemned component a proc (and therefore an arena)"
        );
    }

    // The released range is recycled — to a NEW guest process, on its own fresh client.
    let fresh = HClient(0xC1);
    let mut s = Scenario::new();
    s.compute_process_on_gpu(
        fresh,
        Pdb(0x3700_0000),
        identical_handles(0x180, 0x280),
        Some(1),
    );
    for ev in s.events {
        gpu.apply(ev).expect("the new process applies");
    }
    core_publish(&mut gpu, GPU1, Pdb(0x3700_0000), VA_CTL).expect("the new process is served");
    let new_pid = gpu.spine.by_pdb[&(GPU1, Pdb(0x3700_0000))];
    assert_eq!(
        gpu.procs[&new_pid].arenas[&GPU1].range, arena,
        "the condemned proc's arena must be recycled (the #80 fix) — to a NEW process"
    );
    assert!(
        !gpu.procs[&new_pid].clients.contains(&client_of(P_HUP)),
        "the recycled arena went to an impostor of the condemned component"
    );
    // Still globally disjoint: exactly one live proc holds that range.
    assert_eq!(
        gpu.procs
            .values()
            .filter(|p| p.arenas.values().any(|a| a.range == arena))
            .count(),
        1,
        "the released arena was handed out twice"
    );
}

/// ★ **The ordinary, graph-driven retire path condemns NOTHING** — a false condemnation
/// would be a self-inflicted denial of service.
///
/// Two shapes, both of which retire a `Proc` without any host-side failure: (a) the
/// guest frees its client root (`refresh` step 3), and (b) a `DUP_OBJECT` merges two
/// untouched components, so the absorbed `Proc` is retired into the survivor
/// (`refresh`'s merge arm). Neither may leave a condemned entry behind, and in (a) the
/// guest must be able to run the very same process again.
#[test]
fn the_graph_driven_retire_paths_never_condemn() {
    let _wd = watchdog("graph_retire_never_condemns", Duration::from_secs(60));

    // ---- (a) the guest frees its client root, then starts the same process again.
    let (mut gpu, _pids, _rec) = mean_gpu();
    let lane = lane_of(P_TEARDOWN);
    let h = identical_handles(lane.gr.0, lane.ce.0);
    core_publish(&mut gpu, GPU0, lane.pdb, VA_WARM).expect("publishes while alive");
    gpu.apply(RmEvent::Free {
        client: client_of(P_TEARDOWN),
        handle: h.client_root,
    })
    .expect("the client-root free applies");
    assert_eq!(
        gpu.spine.condemned_len(),
        0,
        "★ a guest-driven teardown must NEVER condemn — that would be a self-inflicted DoS"
    );
    assert!(!gpu.spine.is_condemned(client_of(P_TEARDOWN)));
    let mut s = Scenario::new();
    s.compute_process_on_gpu(client_of(P_TEARDOWN), lane.pdb, h, None);
    for ev in s.events {
        gpu.apply(ev).expect("the same process starts again");
    }
    core_publish(&mut gpu, GPU0, lane.pdb, VA_CTL)
        .expect("★ a guest that tore itself down must be served again");

    // ---- (b) the LateMerge-absorb arm: a UVM dup folds an untouched proc into another.
    let (mut gpu, _pids, _rec) = mean_gpu();
    let compute_vas = NodeKey::new(client_of(P_WITNESS), H_VASPACE);
    let uvm = HClient(0xD0);
    let mut s = Scenario::new();
    s.uvm_dup(
        uvm,
        HObject(0x7000_0000),
        HObject(0x7000_0001),
        HObject(0x7000_0010),
        Pdb(0x3800_0000),
        HObject(0x7000_00a0),
        compute_vas,
    );
    for ev in s.events {
        gpu.apply(ev).expect("the UVM dup applies");
    }
    assert_eq!(
        gpu.spine.condemned_len(),
        0,
        "★ the merge-absorb retire must NEVER condemn"
    );
    let merged = gpu.spine.by_pdb[&(GPU0, lane_of(P_WITNESS).pdb)];
    assert!(gpu.procs[&merged].clients.contains(&uvm));
    core_publish(&mut gpu, GPU0, lane_of(P_WITNESS).pdb, VA_CTL)
        .expect("★ the merged proc keeps serving");
}

/// ★ **MG: condemnation is a COMPONENT-wide fact, and it is keyed on the component —
/// not on a number that another GPU's proc happens to share.**
///
/// The victim here spans BOTH GPUs (a UVM-style dup joins a GPU0 client and a GPU1
/// client into one dup-connected component = one `Proc` with one isolate *per target*,
/// MG-5). Killing a worker of its **GPU1** isolate must condemn the whole component,
/// GPU0 plane included — the blast radius is the proc, not the target. Meanwhile the
/// four bystander procs carrying byte-identical `Pdb`/`VChid` VALUES on both GPUs must
/// be untouched: if the condemned key had lost its `GpuId`, or if condemnation were
/// keyed on anything numeric rather than on the client set, they would die with it.
#[test]
fn condemnation_spans_a_procs_gpus_and_spares_identical_ids_on_other_gpus() {
    let _wd = watchdog("condemnation_multi_gpu", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();

    // Join P_TEARDOWN (GPU0, lane B) and P_CHANFREE (GPU1, lane B) into ONE proc, so a
    // single component spans two targets with two isolates.
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(client_of(P_TEARDOWN), H_VASPACE),
        dst: NodeKey::new(client_of(P_CHANFREE), HObject(0x7100_00a0)),
    })
    .expect("the cross-GPU dup applies");
    let spanning = gpu.spine.by_pdb[&(GPU0, lane_of(P_TEARDOWN).pdb)];
    assert_eq!(
        gpu.spine.by_pdb[&(GPU1, lane_of(P_CHANFREE).pdb)],
        spanning,
        "the dup made one proc out of the two clients"
    );
    core_publish(&mut gpu, GPU0, lane_of(P_TEARDOWN).pdb, VA_WARM).expect("its GPU0 plane works");
    core_publish(&mut gpu, GPU1, lane_of(P_CHANFREE).pdb, VA_WARM).expect("its GPU1 plane works");
    assert_eq!(
        gpu.procs[&spanning].targets,
        BTreeSet::from([GPU0, GPU1]),
        "one proc, two per-target isolates (MG-5)"
    );

    // ★ Its GPU1 worker dies. The COMPONENT is condemned — both of its GPU planes.
    assert!(core_retire_out_of_band(&mut gpu, spanning));
    churn(&mut gpu, client_of(P_WITNESS), 0);
    for (g, i) in [(GPU0, P_TEARDOWN), (GPU1, P_CHANFREE)] {
        assert_eq!(
            core_publish(&mut gpu, g, lane_of(i).pdb, VA_HELD),
            Err(FwdFault::Condemned {
                anchor: ProcAnchor(client_of(P_TEARDOWN)),
            }),
            "★ the condemned component's {g:?} plane still serves the guest"
        );
        assert!(gpu.spine.is_condemned(client_of(i)));
    }
    assert_eq!(
        gpu.spine.condemned_len(),
        1,
        "ONE component, not one per GPU"
    );

    // ★ …and the bystanders — identical numeric lanes, other procs — are untouched.
    for i in [P_WITNESS, P_PEER, P_REROUTE, P_HUP] {
        assert!(!gpu.spine.is_condemned(client_of(i)));
        core_publish(&mut gpu, gpu_of(i), lane_of(i).pdb, VA_CTL)
            .unwrap_or_else(|e| panic!("bystander proc {i} was condemned with the victim: {e:?}"));
        kayfabe_fwd::handle_doorbell(&mut gpu, gpu_of(i), MockArch::token_for(lane_of(i).gr), &[])
            .unwrap_or_else(|e| {
                panic!("bystander proc {i}'s exec plane died with the victim: {e:?}")
            });
    }
    // Lane C's PDB value lives on BOTH GPUs; lane B's now names a condemned component on
    // one GPU and nothing else anywhere. No key collapsed.
    assert_ne!(pids[P_REROUTE], pids[P_HUP]);
}

/// ★ **A `DUP_OBJECT` that would merge a LIVE proc into a condemned component is
/// refused ATOMICALLY** (`GpuError::CondemnedMerge`).
///
/// Dup-connected clients are one `Proc` by construction — one isolate, one arena, one
/// blast radius — so there is no honest way to honour this dup: absorbing the condemned
/// clients into the live proc resurrects them around a working isolate, and condemning
/// the live proc lets a guest kill a healthy process by dupping into a corpse. The third
/// answer is to refuse the *event*; `Spine::apply` is transactional, so the live proc
/// keeps serving and the condemned component stays condemned — nothing moves.
#[test]
fn a_dup_into_a_condemned_component_is_refused_atomically() {
    let _wd = watchdog("condemned_merge_refused", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let victim = pids[P_HUP];

    core_publish(&mut gpu, GPU1, lane_of(P_HUP).pdb, VA_WARM).expect("publishes while alive");
    assert!(core_retire_out_of_band(&mut gpu, victim));

    let before = gpu.procs.len();
    assert_eq!(
        gpu.apply(RmEvent::Dup {
            src: NodeKey::new(client_of(P_HUP), H_VASPACE),
            dst: NodeKey::new(client_of(P_PEER), HObject(0x7200_00a0)),
        }),
        Err(GpuError::CondemnedMerge {
            live: pids[P_PEER],
            condemned: ProcAnchor(client_of(P_HUP)),
        }),
        "★ dupping a condemned component into a live proc must be refused, loudly"
    );

    // ATOMIC: the live proc is untouched and still serving, the condemned one is still
    // condemned, and nothing was minted or merged.
    assert_eq!(
        gpu.procs.len(),
        before,
        "the refusal minted or dropped a proc"
    );
    assert!(!gpu.spine.is_condemned(client_of(P_PEER)));
    assert_eq!(gpu.spine.condemned_len(), 1);
    assert!(
        !gpu.procs[&pids[P_PEER]].clients.contains(&client_of(P_HUP)),
        "the rolled-back dup left the condemned client inside a live proc"
    );
    core_publish(&mut gpu, GPU1, lane_of(P_PEER).pdb, VA_CTL)
        .expect("★ the live proc keeps serving after the refusal");
    assert_eq!(
        core_publish(&mut gpu, GPU1, lane_of(P_HUP).pdb, VA_HELD),
        Err(FwdFault::Condemned {
            anchor: ProcAnchor(client_of(P_HUP)),
        }),
        "★ and the condemned component is still condemned"
    );
}

// =================================================================================
// ★★ §12.13 — RECOVERY: the other half of the contract, and the half a user lives in
//
// Everything above pins that condemnation STICKS. That is the half that protects the
// host. The half the *guest* experiences — that an application can come back — was an
// assumed claim until these tests, and the whole justification for never resurrecting
// rests on it:
//
//   - refusing to resurrect is only defensible if it is **sticky-fatal**, not
//     **permanently bricked**. Real hardware faults a channel and makes the context
//     sticky-fatal until the application tears it down and builds a new one; every CUDA
//     application already handles that path, because Xids exist.
//   - condemnation is keyed on the **client set**, so a re-initialising application —
//     a fresh CUDA context allocates a NEW RM client — forms a boundary with no dup edge
//     to the condemned set, is therefore a *different* component, is therefore not
//     condemned, and simply works. That is a claim about the key, and it is testable.
//   - and if the process dies instead, the **guest kernel** frees its clients on its
//     behalf, so the entry clears with no cooperation from the application at all
//     (measured on real GA106, 2026-07-25: a killed guest process produces 178 `fn=10`
//     RM FREE RPCs followed by fn-47, with the host stub still alive).
//
// If any of these failed, the design would owe the guest a resurrect — and a resurrect
// hands it a **zeroed** backing for a VA it believes still holds its data, because
// `publish_backing`'s host memory (`RmBackend::alloc_sysmem`) belonged to the dead
// isolate's RM client and the host kernel freed it. That is silent data corruption, and
// it is why the answer must be recovery rather than re-materialization.
// =================================================================================

/// The re-initialising application's fresh RM client — a genuinely NEW client handle,
/// with no `DUP_OBJECT` edge to anything condemned. This is the shape of `cuInit` +
/// `cuCtxCreate` after an Xid killed the previous context.
const R_CLIENT: HClient = HClient(0xB0);
/// The second fresh client, for the multi-GPU recovery (dup-joined to [`R_CLIENT`], so
/// the recovered component spans both targets exactly as the dead one did).
const R_CLIENT2: HClient = HClient(0xB1);
/// The fresh context's page-directory base (a new VASpace ⇒ a new PDB).
const R_PDB: Pdb = Pdb(0x3a00_0000);
/// The second fresh context's PDB.
const R_PDB2: Pdb = Pdb(0x3b00_0000);
/// The fresh context's GR vChid (E0: fresh per channel-create, no collisions).
const R_GR: VChid = VChid(0x140);
/// The fresh context's CE vChid.
const R_CE: VChid = VChid(0x240);
/// The second fresh context's GR vChid.
const R_GR2: VChid = VChid(0x141);
/// The second fresh context's CE vChid.
const R_CE2: VChid = VChid(0x241);
/// The dup handle the multi-GPU recovery aliases its sibling's VASpace under.
const R_ALIAS: HObject = HObject(0x7300_00a0);

/// Apply one fresh CUDA-context-shaped subgraph on `target` — the guest half of a
/// recovery. Deliberately the SAME builder the world itself is made of: a recovering
/// application is not a special case, it is just another process.
fn reinit(gpu: &mut Gpu, client: HClient, pdb: Pdb, gr: VChid, ce: VChid, target: GpuId) {
    let mut s = Scenario::new();
    let instance = if target == GPU0 { None } else { Some(target.0) };
    s.compute_process_on_gpu(client, pdb, identical_handles(gr.0, ce.0), instance);
    for ev in s.events {
        gpu.apply(ev)
            .expect("★ the re-initialising process's RM events apply");
    }
}

/// Every host object identity a `Proc` currently holds — host VASes, published backing
/// memory, host channels, engine objects. The set a recovered component must share
/// **nothing** with.
fn host_identities(p: &kayfabe_core::gpu::Proc) -> BTreeSet<u64> {
    let mut out = BTreeSet::new();
    for vas in p.vases.values() {
        out.extend(vas.host_vas.map(|h| h.0));
        for (_va, _len, b) in vas.table.iter() {
            out.extend(b.host_memory().map(|h| h.0));
        }
    }
    for c in p.channels.values() {
        out.extend(c.host_channel.map(|h| h.0));
        out.extend(c.host_engine_objects.values().map(|h| h.0));
    }
    out
}

/// Publish one page and ring the channel that gates on it, end to end, on a recovered
/// component — the smallest thing that proves it is genuinely SERVED and not merely
/// present in a map. The ring passes the published VA as its working set, so the #14
/// ring-gate is actually exercised rather than trivially satisfied by an empty set.
fn publish_and_ring(gpu: &mut Gpu, target: GpuId, pdb: Pdb, gr: VChid, va: u64) -> DoorbellOutcome {
    core_publish(gpu, target, pdb, va).expect("★ the recovered component publishes");
    kayfabe_fwd::handle_doorbell(gpu, target, MockArch::token_for(gr), &[GpuVa(va)])
        .expect("★ the recovered component rings")
}

/// ★★ **THE HEADLINE OF §12.13's JUSTIFICATION: an application RECOVERS by
/// re-initialising, while its condemned predecessor stays dead.**
///
/// "Never resurrect" is only defensible because it is sticky-**fatal**, not
/// sticky-**bricked**: a GPU that faults a channel does not silently hand back a fresh
/// context either, it makes the context fatal until the application builds a new one.
/// The mechanism that makes this true here is the choice of identity key — condemnation
/// is keyed on the **client set**, so a fresh CUDA context (a new RM client, no dup edge
/// to the condemned set) derives a boundary that does not intersect any condemned entry,
/// and is therefore a different component, and is therefore simply served.
///
/// This test is the executable form of that sentence: after a worker death condemns the
/// component, a new client gets a live `Proc` with a **real isolate** and its **own GPA
/// arena**, and publishes + rings end to end — while the condemned key still answers the
/// EXACT [`FwdFault::Condemned`] (never `is_err()`; §12.10's lesson, where a canary
/// passed for the wrong reason).
#[test]
fn a_fresh_client_recovers_from_its_condemned_predecessor() {
    let _wd = watchdog("recovery_fresh_client", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let (victim, lane) = (pids[P_HUP], lane_of(P_HUP));
    let condemned = FwdFault::Condemned {
        anchor: ProcAnchor(client_of(P_HUP)),
    };

    core_publish(&mut gpu, GPU1, lane.pdb, VA_WARM).expect("the victim publishes while alive");
    assert!(core_retire_out_of_band(&mut gpu, victim));
    churn(&mut gpu, client_of(P_WITNESS), 0);
    assert_eq!(
        core_publish(&mut gpu, GPU1, lane.pdb, VA_HELD),
        Err(condemned),
        "the predecessor is condemned before the recovery starts"
    );

    // ---- ★ The application re-initialises: a fresh RM client, same GPU, same process.
    reinit(&mut gpu, R_CLIENT, R_PDB, R_GR, R_CE, GPU1);

    // (a) It is a DIFFERENT COMPONENT. Nothing about the fresh client reaches the
    // condemned client set, so the condemnation pass does not even consider it.
    assert!(
        !gpu.spine.is_condemned(R_CLIENT),
        "★★ the fresh client was condemned by association — the identity key is wrong \
         and §12.13's justification does not hold"
    );
    let pid = gpu.spine.by_pdb[&(GPU1, R_PDB)];
    assert_ne!(pid, victim, "ProcIds are never reused");
    let p = &gpu.procs[&pid];
    assert!(p.clients.contains(&R_CLIENT));
    assert!(
        !p.clients.contains(&client_of(P_HUP)),
        "★ the recovery absorbed the condemned client — that IS a resurrect"
    );

    // (b) It got the real thing: a live isolate and its own arena on its own target
    // (not a stub, not a shared lane — `ensure_proc_target`'s MG-5 materialization).
    assert!(
        p.isolates.contains_key(&GPU1),
        "★ the recovered component has no isolate"
    );
    let arena = p.arenas[&GPU1].range.clone();
    assert!(!arena.is_empty());
    assert_eq!(p.targets, BTreeSet::from([GPU1]));

    // (c) And it is genuinely SERVED: publish + ring, end to end, through the ring-gate.
    let rung = publish_and_ring(&mut gpu, GPU1, R_PDB, R_GR, VA_CTL);
    assert_eq!(rung.proc, pid, "the ring routed to the recovered proc");
    assert!(
        rung.scheduled_now,
        "the recovered channel was made runnable on its first submission"
    );
    assert_eq!(
        token_lane(rung.host_token),
        (pid.0 + 1, GPU1.0),
        "★ the recovered component rang a host token minted in its OWN isolate lane"
    );
    let published = gpu.procs[&pid].vases[&(GPU1, R_PDB)]
        .table
        .resolve(R_PDB, GpuVa(VA_CTL))
        .expect("the recovered publication resolves")
        .0;
    assert!(
        arena.contains(&published.phys),
        "the recovered publication came from its own arena"
    );

    // (d) ★ MEANWHILE: the condemned component is untouched by any of it.
    assert_eq!(
        core_publish(&mut gpu, GPU1, lane.pdb, VA_HELD),
        Err(condemned),
        "★★ a successful recovery must NOT clear its predecessor's condemnation"
    );
    assert!(gpu.spine.is_condemned(client_of(P_HUP)));
    assert_eq!(gpu.spine.condemned_len(), 1);
}

/// ★ **No number of recoveries clears the condemned entry** — the recovery path is
/// additive, never a side-channel that launders the predecessor back to life.
///
/// The blunt version of the previous test's part (d): eight successive fresh clients,
/// each of which recovers fully, with the condemned key required to answer the EXACT
/// same [`FwdFault::Condemned`] value after every single one. A fix that dropped the
/// entry when a boundary stopped needing it, or that re-labelled the component when a
/// new proc was minted, would drift here and not in a single-shot test.
#[test]
fn no_amount_of_recovery_clears_the_condemned_entry() {
    let _wd = watchdog("recovery_never_clears", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let (victim, lane) = (pids[P_HUP], lane_of(P_HUP));
    let condemned = FwdFault::Condemned {
        anchor: ProcAnchor(client_of(P_HUP)),
    };

    core_publish(&mut gpu, GPU1, lane.pdb, VA_WARM).expect("the victim publishes while alive");
    assert!(core_retire_out_of_band(&mut gpu, victim));

    for k in 0..8u32 {
        // Each round is one more re-initialisation of the same application.
        let client = HClient(0xB8 + k);
        let pdb = Pdb(0x3c00_0000 + u64::from(k) * 0x10_0000);
        let gr = VChid(0x150 + k as u16);
        let ce = VChid(0x250 + k as u16);
        reinit(&mut gpu, client, pdb, gr, ce, GPU1);
        let rung = publish_and_ring(&mut gpu, GPU1, pdb, gr, VA_CTL + u64::from(k) * 0x1000);
        assert!(
            !gpu.spine.is_condemned(client),
            "recovery #{k} was condemned"
        );

        // ★ The exact fault, every round — never `is_err()` (§12.10).
        assert_eq!(
            core_publish(&mut gpu, GPU1, lane.pdb, VA_HELD + u64::from(k) * 0x1000),
            Err(condemned),
            "★★ recovery #{k} cleared its predecessor's condemnation"
        );
        assert_eq!(
            kayfabe_fwd::handle_doorbell(&mut gpu, GPU1, MockArch::token_for(lane.gr), &[]),
            Err(condemned),
            "★★ recovery #{k} revived the condemned component's exec plane"
        );
        assert_eq!(
            gpu.spine.condemned_len(),
            1,
            "recovery #{k} split or duplicated the condemned entry"
        );
        assert!(gpu.spine.is_condemned(client_of(P_HUP)));
        assert!(
            gpu.procs
                .values()
                .all(|p| !p.clients.contains(&client_of(P_HUP))),
            "recovery #{k} minted a `Proc` holding the condemned client"
        );
        // …and the recovery is a real one, not a routing accident.
        assert_ne!(rung.proc, victim);
    }
}

/// ★ **Process death clears the condemnation with NO cooperation from the application**
/// — and the guest's own identity becomes reusable afterwards.
///
/// This is the second escape hatch, and it is the one that needs no application changes
/// at all: in Mode-2 the **guest kernel** is the garbage collector at one boundary (it
/// frees a dead process's RM clients on its behalf) and the host kernel is the garbage
/// collector at the other; condemnation only has to refuse to paper over the gap between
/// them. Measured on real GA106, 2026-07-25: killing a guest process produces **178
/// `fn=10` (RM FREE) RPCs** followed by fn-47, with the host stub still alive — i.e. the
/// client-root free below is not a synthetic event, it is what a `SIGKILL` looks like
/// from the core's side.
///
/// Distinct from [`a_fresh_client_recovers_from_its_condemned_predecessor`]: there the
/// entry REMAINS and a new identity is served alongside it; here the entry is GONE and
/// the *same* client handle and the *same* PDB — which a guest kernel will hand out
/// again — are served. Neither implies the other.
#[test]
fn process_death_clears_the_condemnation_with_no_application_cooperation() {
    let _wd = watchdog("recovery_process_death", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let (victim, lane) = (pids[P_HUP], lane_of(P_HUP));
    let h = identical_handles(lane.gr.0, lane.ce.0);

    core_publish(&mut gpu, GPU1, lane.pdb, VA_WARM).expect("the victim publishes while alive");
    assert!(core_retire_out_of_band(&mut gpu, victim));
    assert!(gpu.spine.is_condemned(client_of(P_HUP)));

    // ---- The guest process is killed. The GUEST KERNEL frees its client root; the
    // application does nothing, knows nothing, and is not asked.
    gpu.apply(RmEvent::Free {
        client: client_of(P_HUP),
        handle: h.client_root,
    })
    .expect("the guest kernel's client-root free applies");

    assert!(
        !gpu.spine.is_condemned(client_of(P_HUP)),
        "★★ the condemnation outlived the process that earned it — an application that \
         cannot even recover by DYING is bricked, not sticky-fatal"
    );
    assert_eq!(gpu.spine.condemned_len(), 0);

    // ---- ★ The identity is reusable: the SAME client handle and the SAME PDB the
    // guest kernel will hand out again are served normally. Condemnation was the death
    // of one component, never the poisoning of a value.
    reinit(&mut gpu, client_of(P_HUP), lane.pdb, lane.gr, lane.ce, GPU1);
    let pid = gpu.spine.by_pdb[&(GPU1, lane.pdb)];
    assert_ne!(pid, victim, "ProcIds are never reused");
    assert!(!gpu.spine.is_condemned(client_of(P_HUP)));
    let rung = publish_and_ring(&mut gpu, GPU1, lane.pdb, lane.gr, VA_CTL);
    assert_eq!(rung.proc, pid);
    assert_eq!(gpu.spine.condemned_len(), 0);
}

/// ★ **A recovered component shares NOTHING host-side with the one it replaced** — not
/// an arena, not a single host handle.
///
/// Recovery must be a new blast radius, not the old one with a new label. The mock
/// namespaces every host identity by `(isolate, GPU)` (see [`handle_lane`]), so "no host
/// handle of the condemned component is ever observed by the recovered one" is a direct
/// assertion here rather than a hope.
///
/// The GPA arena is asserted **disjoint while the condemned proc is still unreaped**,
/// which is the only window where the claim is unconditional: after the deferred reap
/// the released range is deliberately recycled by the #80 free-list, and handing that
/// range to a genuinely new process is correct (that is
/// [`a_condemned_components_arena_is_released_once_and_never_recycled_to_an_impostor`]).
/// The host-handle disjointness holds in BOTH windows, and is asserted across the reap.
#[test]
fn a_recovered_component_shares_no_arena_or_host_handle_with_the_condemned_one() {
    let _wd = watchdog("recovery_isolation", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let (victim, lane) = (pids[P_HUP], lane_of(P_HUP));

    // Warm the victim's host state so it has handles worth confusing: a publication, a
    // host VAS, and a materialized+scheduled host channel.
    core_publish(&mut gpu, GPU1, lane.pdb, VA_WARM).expect("the victim publishes while alive");
    kayfabe_fwd::handle_doorbell(
        &mut gpu,
        GPU1,
        MockArch::token_for(lane.gr),
        &[GpuVa(VA_WARM)],
    )
    .expect("the victim rings while alive");
    let dead_handles = host_identities(&gpu.procs[&victim]);
    let dead_arena = gpu.procs[&victim].arenas[&GPU1].range.clone();
    assert!(
        dead_handles.len() >= 3,
        "the victim must actually hold host state for this test to mean anything"
    );

    assert!(core_retire_out_of_band(&mut gpu, victim));
    reinit(&mut gpu, R_CLIENT, R_PDB, R_GR, R_CE, GPU1);
    publish_and_ring(&mut gpu, GPU1, R_PDB, R_GR, VA_CTL);
    let pid = gpu.spine.by_pdb[&(GPU1, R_PDB)];

    // ---- (a) A different GPA arena, while the dead one still holds its range.
    let live_arena = gpu.procs[&pid].arenas[&GPU1].range.clone();
    assert!(
        live_arena.end <= dead_arena.start || dead_arena.end <= live_arena.start,
        "★★ the recovered component was carved into the condemned component's arena \
         {dead_arena:?} (got {live_arena:?}) — the #14 collision class, back"
    );

    // ---- (b) Not one host handle in common, and every one of the recovered
    // component's own handles is in ITS lane — so the disjointness is structural, not
    // an accident of which values happened to be minted.
    let live_handles = host_identities(&gpu.procs[&pid]);
    assert!(!live_handles.is_empty());
    for h in &live_handles {
        assert!(
            !dead_handles.contains(h),
            "★★ host handle {h:#x} of the CONDEMNED component is observed by the \
             recovered one — recovery is reusing the dead isolate's namespace"
        );
        assert_eq!(
            handle_lane(*h),
            (pid.0 + 1, GPU1.0),
            "★ the recovered component holds a host handle from another (proc, GPU)"
        );
    }
    for h in &dead_handles {
        assert_ne!(
            handle_lane(*h).0,
            pid.0 + 1,
            "★ the condemned component's handle {h:#x} is in the recovered proc's lane"
        );
    }

    // ---- (c) Across the reap the recovered component is unmoved: reclaiming the dead
    // isolate's host state must not touch the live one's (the recycle is a GPA-range
    // fact, never a host-handle fact).
    assert_eq!(gpu.reap_retired().len(), 1, "reaped exactly once");
    assert_eq!(
        host_identities(&gpu.procs[&pid]),
        live_handles,
        "★ the reap of the condemned component disturbed the recovered one's host state"
    );
    publish_and_ring(&mut gpu, GPU1, R_PDB, R_GR, VA_CTL + 0x1000);
}

/// ★ **MG: recovery after a multi-GPU condemnation works on BOTH targets, and the
/// bystanders on byte-identical numeric lanes never notice.**
///
/// The mirror image of
/// [`condemnation_spans_a_procs_gpus_and_spares_identical_ids_on_other_gpus`]: there a
/// component spanning GPU0+GPU1 loses its **GPU1** worker and dies on both planes; here
/// the application re-initialises into a component that likewise spans both targets
/// (two fresh clients joined by the same UVM-shaped `DUP_OBJECT`), and must be served on
/// both. If condemnation were keyed on anything numeric, the recovery's own `Pdb`/`VChid`
/// values — or the four bystanders' identical ones — would collide with the corpse.
#[test]
fn recovery_after_a_multi_gpu_condemnation_serves_both_targets() {
    let _wd = watchdog("recovery_multi_gpu", Duration::from_secs(60));
    let (mut gpu, _pids, _rec) = mean_gpu();

    // One component over two targets, exactly as the multi-GPU condemnation test builds
    // it, then killed through its GPU1 worker.
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(client_of(P_TEARDOWN), H_VASPACE),
        dst: NodeKey::new(client_of(P_CHANFREE), HObject(0x7100_00a0)),
    })
    .expect("the cross-GPU dup applies");
    let spanning = gpu.spine.by_pdb[&(GPU0, lane_of(P_TEARDOWN).pdb)];
    core_publish(&mut gpu, GPU0, lane_of(P_TEARDOWN).pdb, VA_WARM).expect("its GPU0 plane works");
    core_publish(&mut gpu, GPU1, lane_of(P_CHANFREE).pdb, VA_WARM).expect("its GPU1 plane works");
    assert!(core_retire_out_of_band(&mut gpu, spanning));

    // ---- ★ The application re-initialises across both GPUs.
    reinit(&mut gpu, R_CLIENT, R_PDB, R_GR, R_CE, GPU0);
    reinit(&mut gpu, R_CLIENT2, R_PDB2, R_GR2, R_CE2, GPU1);
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(R_CLIENT, H_VASPACE),
        dst: NodeKey::new(R_CLIENT2, R_ALIAS),
    })
    .expect("★ the recovery's own cross-GPU dup applies — it touches nothing condemned");

    for c in [R_CLIENT, R_CLIENT2] {
        assert!(
            !gpu.spine.is_condemned(c),
            "★★ the recovering client {c:?} was condemned by association with the \
             component it is replacing"
        );
    }
    let recovered = gpu
        .spine
        .by_pdb
        .get(&(GPU0, R_PDB))
        .copied()
        .expect("★★ the recovered component was never materialized on GPU0");
    assert_eq!(
        gpu.spine
            .by_pdb
            .get(&(GPU1, R_PDB2))
            .copied()
            .expect("★★ the recovered component was never materialized on GPU1"),
        recovered,
        "the recovery is ONE component over two targets, like the one it replaces"
    );
    assert_ne!(recovered, spanning, "ProcIds are never reused");
    assert_eq!(
        gpu.procs[&recovered].targets,
        BTreeSet::from([GPU0, GPU1]),
        "one proc, two per-target isolates (MG-5)"
    );

    // Served on BOTH planes, end to end.
    for (g, pdb, gr) in [(GPU0, R_PDB, R_GR), (GPU1, R_PDB2, R_GR2)] {
        let rung = publish_and_ring(&mut gpu, g, pdb, gr, VA_CTL);
        assert_eq!(rung.proc, recovered);
        assert_eq!(
            token_lane(rung.host_token),
            (recovered.0 + 1, g.0),
            "★ the recovered {g:?} plane rang a token from its OWN (proc, GPU) isolate"
        );
    }

    // ---- …the condemned component is still condemned, on both of ITS planes…
    for (g, i) in [(GPU0, P_TEARDOWN), (GPU1, P_CHANFREE)] {
        assert_eq!(
            core_publish(&mut gpu, g, lane_of(i).pdb, VA_HELD),
            Err(FwdFault::Condemned {
                anchor: ProcAnchor(client_of(P_TEARDOWN)),
            }),
            "★★ recovering on {g:?} revived the condemned component's {g:?} plane"
        );
        assert!(gpu.spine.is_condemned(client_of(i)));
    }
    assert_eq!(
        gpu.spine.condemned_len(),
        1,
        "ONE component, not one per GPU"
    );

    // ---- …and the four bystanders, carrying byte-identical `Pdb`/`VChid` VALUES on
    // both GPUs, are untouched by the death AND by the recovery.
    for i in [P_WITNESS, P_PEER, P_REROUTE, P_HUP] {
        assert!(!gpu.spine.is_condemned(client_of(i)));
        core_publish(&mut gpu, gpu_of(i), lane_of(i).pdb, VA_CTL)
            .unwrap_or_else(|e| panic!("bystander proc {i} was disturbed by the recovery: {e:?}"));
        kayfabe_fwd::handle_doorbell(&mut gpu, gpu_of(i), MockArch::token_for(lane_of(i).gr), &[])
            .unwrap_or_else(|e| {
                panic!("bystander proc {i}'s exec plane was disturbed by the recovery: {e:?}")
            });
    }
}
