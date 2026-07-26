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
use kayfabe_core::rmgraph::{AllocFacts, NodeKey, RmEvent, RmGraphError};
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
use kayfabe_tests::{
    Guarded, ResidueClaim, Scenario, identical_handles, reachable_maps, reachable_objects,
};
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
/// ★ T0's lane: the VA the subset-churn publishes into the VASpace it is about to free.
const VA_T0: u64 = 0x40_0000_0000;

// ---- ★ T0 (`l1_os_shell.md` §7.6 T0 / gap G2): the guest frees a SUBSET of its
// objects while the process keeps running. Identical values on both GPUs, like the
// lanes, so a routing collapse would show here too.

/// How many allocate→use→free rounds the T0 churn runs per proc. Three, not one: T0's
/// claim is about a *steady state* (a training job's map/unmap loop), so the script has
/// to show the residue ACCUMULATING rather than a single one-shot drop.
const T0_ROUNDS: u32 = 3;
/// The page-directory base the T0 churn declares (distinct from every lane's).
const T0_PDB: Pdb = Pdb(0x3700_0000);
/// The vChid the T0 churn's throw-away GR channel takes.
const T0_VCHID: VChid = VChid(0x300);
/// Handle base for the T0 churn's VASpaces (one per round — a real guest reuses handle
/// values, but distinct ones keep the graph's alloc-over-a-live-handle refusal out of
/// the script's way).
const T0_VAS: HObject = HObject(0x5c00_0200);
/// Handle base for the T0 churn's channels (one per round).
const T0_CHAN: HObject = HObject(0x5c00_0300);

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
fn mean_world(mode: LockMode) -> (Guarded<Arc<SharedDevice>>, Vec<ProcId>, SharedRecorder) {
    let (gpu, pids, recorder) = mean_gpu();
    (
        gpu.map(|g| Arc::new(SharedDevice::new(g, mode))),
        pids,
        recorder,
    )
}

/// The same six-proc, two-GPU world as a **bare [`Gpu`]**, for the §12.13 property
/// tests below: condemnation is a fact of the pure core, so the tests that pin it read
/// core state directly (arenas, client sets, the source registry) instead of through the
/// lock shell. Deterministic logic-core testing, §8.2's T1 tier.
fn mean_gpu() -> (Guarded<Gpu>, Vec<ProcId>, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    // ★ G9 (§12.21): realized with two physical GPUs — the entitlement.
    let mut gpu = Gpu::realize(arch, Box::new(factory), gpa, &[GpuId::ZERO, GpuId(1)])
        .expect("device realizes");

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
    (
        Guarded::new("l1_mean::mean_gpu", gpu, recorder.clone()),
        pids,
        recorder,
    )
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
    /// ★ The conservation ledger's leak set at quiesce, as `(objects, mappings)`:
    /// outstanding on a **live** proc and unreachable from core state. Mode-independent
    /// because it is script-driven — the retry paths release what they allocate
    /// (`retry_ledger.rs`), so nothing here depends on which thread won a race.
    leaked: (usize, usize),
    /// The §7.0 namespace-death residue at quiesce, as `(objects, mappings)`: reported,
    /// not asserted to be zero — a reaped isolate's whole handle namespace dies with it.
    session_death: (usize, usize),
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
                handle_lane(f.host_object.raw()),
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

/// ★★ **T0 — the guest frees a SUBSET of its objects while the process keeps running**
/// (`l1_os_shell.md` §7.6 T0, gap G2; §7.0's one exception).
///
/// Every other teardown path in this file ends with something *dying*: a proc retires, a
/// worker HUPs, an isolate is reaped — and §7.0's backstop (*"the isolate process
/// boundary is the garbage collector"*) covers all of them. This is the path where
/// **nothing dies**. The proc stays live, its isolate stays healthy, and `Spine::refresh`'s
/// `p.vases.retain(…)` / `p.channels.retain(…)` simply drop the `Vas` and `Channel`
/// values — with `host_vas`, the bindings' `host`, `host_channel`, `host_token`,
/// `host_engine_objects` and the `GpaBlock`s still inside them.
///
/// So the script has to actually MATERIALIZE host state before it frees, or it proves
/// nothing: phase 0 deliberately leaves the canary procs' GR channels VIRGIN, and the two
/// subset-frees the run already performed (`P_CHANFREE`, `P_REROUTE`) therefore dropped
/// `Channel`s holding `host_channel: None`. Measured: with only those, the conservation
/// census reported **zero** leaked objects — a true negative that says the script never
/// reached T0, not that T0 is safe.
///
/// Each round, on ONE live proc, does what a training job does in its steady state:
///
/// 1. declare a fresh VASpace + PDB, and **publish backing into it** — host VAS, host
///    memory, a host GPU mapping, and a `GpaBlock` carved from the proc's own arena;
/// 2. declare a fresh GR channel on the proc's *main* VASpace, **ring it** (host channel
///    plus host work-submit token) and **forward an engine object** onto it — so the exec
///    plane's half of the leak is real too, and independent of the address plane's;
/// 3. **free the channel**, then **free the VASpace** — in that order, because RM frees
///    children and dependents ahead of parents, and because freeing the VASpace first
///    would make the channel's own drop a consequence rather than an independent probe.
///
/// Run from the main thread inside the composed window (device WRITEs and verb ops, with
/// five host verbs parked and six workload threads hot), never quiesced beside it: the
/// §12.13 lesson is that an isolated case tests what you thought of.
fn t0_churn(device: &SharedDevice, i: usize, round: u32) {
    let (gpu, client) = (gpu_of(i), client_of(i));
    let vas = HObject(T0_VAS.0 + round);
    let chan = HObject(T0_CHAN.0 + round);
    let handles = identical_handles(0, 0);

    // (1) a fresh VASpace with its own PDB, and a publication into it.
    device
        .apply(RmEvent::Alloc {
            client,
            parent: H_DEVICE,
            handle: vas,
            class: mock_classes::VASPACE,
            facts: AllocFacts::default(),
        })
        .expect("T0: the guest declares a VASpace");
    device
        .apply(RmEvent::SetPageDir {
            client,
            vaspace: vas,
            pdb: T0_PDB,
        })
        .expect("T0: and binds a page directory to it");
    device
        .publish_backing(gpu, T0_PDB, GpuVa(VA_T0), 0x1000)
        .expect("T0: the guest publishes backing into the VASpace it will free");

    // (2) a fresh GR channel on the proc's MAIN VASpace, rung and given an engine object.
    device
        .apply(RmEvent::Alloc {
            client,
            parent: handles.tsg,
            handle: chan,
            class: mock_classes::CHANNEL_GR,
            facts: AllocFacts {
                h_vaspace: Some(handles.vaspace),
                userd_flags: MockArch::userd_flags_for(T0_VCHID),
                ..Default::default()
            },
        })
        .expect("T0: the guest declares a channel");
    let rung = device
        .doorbell(gpu, MockArch::token_for(T0_VCHID), &[])
        .expect("T0: the guest rings it, materializing a host channel + token");
    assert_eq!(
        token_lane(rung.host_token),
        (ProcId(rung.proc.0).0 + 1, gpu.0),
        "T0's channel was materialized in another (proc, GPU)'s isolate"
    );
    device
        .forward_engine_object(gpu, T0_VCHID, kayfabe_tests::COMPUTE_CLASS, &[])
        .expect("T0: and forwards a Case-1 engine object onto it");

    // (3) the subset free — channel first (children before parents), then the VASpace.
    device
        .apply(RmEvent::Free {
            client,
            handle: chan,
        })
        .expect("T0: the guest frees the channel and keeps running");
    device
        .apply(RmEvent::Free {
            client,
            handle: vas,
        })
        .expect("T0: the guest frees the VASpace and keeps running");
}

// =================================================================================
// ★★ THE COMPOSED RUN
// =================================================================================

/// One complete mean run under `mode`. Deliberately ONE function read top to bottom:
/// the phase ORDER is the test, and hiding it behind helpers would hide the script.
fn mean_run(mode: LockMode) -> MeanReport {
    let (mut device, pids, rec) = mean_world(mode);
    // ★ §12.35 — DECLARED RESIDUE, and it is exactly the number §12.32 pinned as
    // "namespace-death residue: 6 objects, 2 mappings". Both canary procs die VIOLENTLY
    // (`retire_proc` — one scripted teardown, one worker HUP), so `Proc::retire` stops
    // their isolates and the staged release cannot drain: §7.0's process-boundary
    // backstop is the disposition, per §12.17's no-resurrect rule. Every proc that dies
    // CLEANLY in this run — a component vanishing through `Spine::vacate` — now reclaims
    // per object and contributes nothing here, which is the §12.35 delta.
    for i in [P_TEARDOWN, P_HUP] {
        device.declare_residue(
            ResidueClaim::on(
                IsolateId(pids[i].0),
                "a canary proc killed out of band (`retire_proc`): its isolate is stopped, \
                 so its host VAS + backing + channel are the §7.0 namespace-death residue \
                 §12.32 measured at 6 objects / 2 mappings across the pair",
            )
            .objects(VerbKind::AllocVaSpace, 1)
            .objects(VerbKind::AllocSysmem, 1)
            .objects(VerbKind::AllocChannel, 1)
            .maps(1),
        );
    }
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

            // (f) ★★ T0 (G2): two LIVE procs, on the two GPUs, each running the
            // allocate→use→free steady state of a real long-running workload. This is
            // the one lifecycle path §7.0's process-boundary backstop does not cover,
            // and it is composed into the same window as everything else — five parked
            // verbs, six workload threads, and the retires/reroutes above.
            for round in 0..T0_ROUNDS {
                for i in [P_WITNESS, P_PEER] {
                    t0_churn(dev, i, round);
                }
            }

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
    // ★ T0's backstop drain runs FIRST, at the run's first genuinely quiesced moment
    // (every workload thread joined, every latch released, so every isolate is idle).
    // Two reasons, and the second is the one that bit: the queue belongs to the *run*,
    // not to the phase below, and leaving it would make the free-count assertion measure
    // two things at once — it read 13 instead of 1 the first time T0 landed, because
    // three churn rounds' worth of queued releases rode out on the same isolate.
    dev.drain_pending_releases();
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
    // ★ T0's backstop drain (§7.6 T0): the opportunistic path releases a queue on the
    // proc's next verb-issuing op, which covers a busy proc — this covers the one that
    // went quiet, and the run's declared quiesce point is exactly where it belongs. It
    // must be idempotent: the second call has nothing left to do.
    let drained = (dev.drain_pending_releases(), dev.drain_pending_releases());
    assert_eq!(
        drained.1, 0,
        "({mode:?}) the T0 backstop drain is not idempotent — a second sweep at the same \
         quiesce point still found work"
    );
    let reaped = (dev.reap_retired(), dev.reap_retired());
    assert_eq!(
        reaped,
        (2, 0),
        "({mode:?}) exactly the two retired procs reaped, and never again"
    );
    assert_eq!(lock::held_depth(), 0, "the main thread leaked no guard");

    // ---- Phase 11: conservation.
    drop(ex);
    let mut gpu = device.map(|d| {
        Arc::try_unwrap(d)
            .unwrap_or_else(|_| panic!("every device handle was released"))
            .into_gpu()
    });
    sweep_conservation(&mut gpu, &pids, mode);

    // ---- Phase 12: ★ the conservation ledger (§7.8), composed into THIS run rather
    // than asserted beside it. The census runs after the final reap and never in a
    // `Drop` (§7.8's own rule: an assert inside teardown machinery presents as a wedge).
    let census = census(&gpu, &rec);
    report_census(&census, &rec, mode);
    // ★★ THE CONSERVATION INVARIANT, in its strongest form: `Outstanding(ledger) ==
    // Reachable(core state)` for every LIVE proc, as a **set equality** and not a count
    // (`retry_ledger.rs`'s lesson, lifted to the whole composed run). An object that
    // exists and that no `Vas`, binding or channel can name IS a leak, even if the
    // totals happen to match — and on the T0 path nothing will ever free it, because
    // nothing can address it.
    //
    // MEASURED BASELINE, before the T0/G2 fix (both lock modes, identical): 24 objects
    // (6 host VAS + 6 sysmem + 6 channel + 6 engine object), 6 mappings, 24576 GPA
    // bytes — exactly 4 objects + 1 mapping + one 4 KiB block per [`t0_churn`] round,
    // i.e. linear in the number of subset-frees, which is what "a training job's steady
    // state" means.
    assert_eq!(
        (
            census.leaked_object_count(),
            census.leaked_map_count(),
            census.leaked_gpa_total()
        ),
        (0, 0, 0),
        "({mode:?}) ★★ §7.8 conservation: host objects/mappings/GPA outstanding on a LIVE \
         proc that core state can no longer name. This is T0 (G2) — the one lifecycle \
         path with no process-boundary backstop. Leaked: {:?} / {:?} / {:?}",
        census.leaked_objects,
        census.leaked_maps,
        census.leaked_gpa_bytes
    );
    assert_eq!(
        (
            census.dangling_objects.is_empty(),
            census.dangling_maps.is_empty()
        ),
        (true, true),
        "({mode:?}) ★ core state names a host object/mapping the ledger says was already \
         released — a use-after-free shape, not a leak: {:?} / {:?}",
        census.dangling_objects,
        census.dangling_maps
    );
    let l = rec.lock().expect("recorder").ledger();
    assert_eq!(
        (
            l.double_free.as_slice(),
            l.free_of_unknown.as_slice(),
            l.unmap_of_unknown.as_slice()
        ),
        (&[][..], &[][..], &[][..]),
        "({mode:?}) the composed run released something twice, or released something it \
         never acquired (a cross-namespace reach — boundary 2)"
    );

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
        leaked: (census.leaked_object_count(), census.leaked_map_count()),
        session_death: (
            census.session_death_object_count(),
            census.session_death_map_count(),
        ),
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
    // ★ T0 (§7.6 T0): at a real quiesce point, no live proc still owes a release. This is
    // the *queue's* half of the conservation assertion below — an object sitting in
    // `pending_release` is not leaked (the core can still name it) but it is also not
    // reclaimed, and a drain that silently never fires would look identical to a fixed
    // leak if only the ledger were checked.
    for (&pid, p) in &gpu.procs {
        assert_eq!(
            p.pending_release_len(),
            0,
            "({mode:?}) {pid:?} still owes a T0 release at the quiesce point — the \
             opportunistic drain and the backstop sweep both missed it"
        );
    }

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
                    handle_lane(h.raw()),
                    (lane, g.0),
                    "({mode:?}) {pid:?} holds a host VAS from another (proc, GPU)"
                );
                assert_eq!(
                    owner_of.insert(h.raw(), pid),
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
                    handle_lane(h.raw()),
                    (lane, c.gpu.0),
                    "({mode:?}) {pid:?} holds a host channel from another (proc, GPU)"
                );
                assert_eq!(
                    owner_of.insert(h.raw(), pid),
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
                    handle_lane(h.raw()),
                    (lane, c.gpu.0),
                    "({mode:?}) {pid:?} holds an engine object from another (proc, GPU)"
                );
                assert_eq!(
                    owner_of.insert(h.raw(), pid),
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
// ★★ (8) THE CONSERVATION LEDGER — composed into the mean run (`l1_os_shell.md` §7.8)
// =================================================================================

/// What a still-outstanding host object **is**, so a leak can be reported by class
/// rather than as an anonymous count. Read off the mock's own verb log, which is the
/// single funnel every host verb passes through (§7.8: *"a verb that is not in the
/// ledger does not exist"*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum HostClass {
    /// A host VAS (`AllocVaSpace`) — one per `Vas` that ever published.
    Vas,
    /// A host memory object (`AllocSysmem`) — one per published backing.
    Sysmem,
    /// A host channel (`AllocChannel`).
    Channel,
    /// A host engine object on a channel (`AllocEngineObject`).
    EngineObject,
    /// Anything else the mock minted (`Alloc`).
    Other,
}

/// Classify every handle the log ever minted. One pass, so the census is O(log) rather
/// than O(log × leaks).
fn classify_handles(rec: &SharedRecorder) -> BTreeMap<HostHandle, HostClass> {
    let mut m = BTreeMap::new();
    for (_iso, v) in &rec.lock().expect("recorder").log {
        let (h, c) = match v {
            RmVerb::AllocVaSpace { handle } => (*handle, HostClass::Vas),
            RmVerb::AllocSysmem { handle, .. } => (*handle, HostClass::Sysmem),
            RmVerb::AllocChannel { handle, .. } => (*handle, HostClass::Channel),
            RmVerb::AllocEngineObject { handle, .. } => (*handle, HostClass::EngineObject),
            RmVerb::Alloc { handle, .. } => (*handle, HostClass::Other),
            _ => continue,
        };
        m.insert(h, c);
    }
    m
}

/// ★ The end-of-run conservation census: `Outstanding(ledger)` split against
/// `Reachable(core state)`, per isolate, **objects and mappings**.
///
/// The split is the whole point, and it is the distinction `HostLedger`'s own docs
/// insist on: an outstanding handle is not automatically a defect. §7.0 says *the
/// isolate process boundary is the garbage collector*, so an object outstanding on an
/// isolate whose `Proc` was reaped has a real disposition — bulk release at namespace
/// death. An object outstanding on a **live** proc that the core can no longer *name*
/// has none: nothing will ever free it, because nothing can address it. That second
/// set is what §7.8's invariant is about and what this census reports separately.
#[derive(Debug, Default, PartialEq, Eq)]
struct Census {
    /// ★ **THE LEAK SET.** Live proc, outstanding, and unreachable from core state.
    /// No backstop exists for these — §7.0's exception, i.e. G2/T0.
    leaked_objects: BTreeMap<IsolateId, BTreeSet<HostHandle>>,
    /// The mapping half of the leak set, keyed `(host VAS, host GPU VA)`.
    leaked_maps: BTreeMap<IsolateId, BTreeSet<(HostHandle, u64)>>,
    /// The **opposite** imbalance: core state names a host object the ledger says was
    /// already released. A use-after-free shape, and a strictly worse finding than a
    /// leak — the set equality catches it in the same comparison.
    dangling_objects: BTreeMap<IsolateId, BTreeSet<HostHandle>>,
    /// The mapping half of the dangling set.
    dangling_maps: BTreeMap<IsolateId, BTreeSet<(HostHandle, u64)>>,
    /// Outstanding on an isolate whose `Proc` no longer exists (reaped / condemned):
    /// the §7.0 namespace-death bulk disposition. Reported, never asserted to be zero
    /// — the mock does not model namespace death as per-object frees.
    session_death_objects: BTreeMap<IsolateId, BTreeSet<HostHandle>>,
    /// The mapping half of the namespace-death residue.
    session_death_maps: BTreeMap<IsolateId, BTreeSet<(HostHandle, u64)>>,
    /// ★ R6, the **intra-arena** half (§7.8's G6 row): GPA bytes a live proc's arena
    /// still has handed out that no `Vas::blocks` entry can name. The same leak as
    /// [`Census::leaked_objects`] one plane down — and the one that eventually takes a
    /// permanent `FwdFault::Arena` rather than merely burning host handles (#80).
    leaked_gpa_bytes: BTreeMap<(ProcId, GpuId), u64>,
}

impl Census {
    /// Total handles in the leak set (the headline baseline number).
    fn leaked_object_count(&self) -> usize {
        self.leaked_objects.values().map(BTreeSet::len).sum()
    }
    /// Total mappings in the leak set.
    fn leaked_map_count(&self) -> usize {
        self.leaked_maps.values().map(BTreeSet::len).sum()
    }
    /// Total handles left to namespace death.
    fn session_death_object_count(&self) -> usize {
        self.session_death_objects.values().map(BTreeSet::len).sum()
    }
    /// Total mappings left to namespace death.
    fn session_death_map_count(&self) -> usize {
        self.session_death_maps.values().map(BTreeSet::len).sum()
    }
    /// Total unnameable GPA bytes across every live proc's arenas.
    fn leaked_gpa_total(&self) -> u64 {
        self.leaked_gpa_bytes.values().sum()
    }
}

/// Take the census of one finished run: replay the recorder into a [`HostLedger`] and
/// compare it, per isolate, against what the re-assembled core can still name.
///
/// `IsolateId == ProcId` by construction (`Gpu` spawns `IsolateId(pid.0)` per target),
/// and **one `IsolateId` covers both of a proc's per-GPU isolates** — the mock keys its
/// log by isolate id and namespaces the handle VALUES by GPU, so per-isolate here means
/// per-`Proc`, which is exactly the granularity [`reachable_objects`] answers at.
fn census(gpu: &Gpu, rec: &SharedRecorder) -> Census {
    let ledger = rec.lock().expect("recorder").ledger();
    let mut c = Census::default();
    let mut isolates: BTreeSet<IsolateId> = ledger.leaked.keys().copied().collect();
    isolates.extend(ledger.leaked_maps.keys().copied());
    for iso in isolates {
        let outstanding = ledger.leaked_on(iso);
        let outstanding_maps = ledger.leaked_maps.get(&iso).cloned().unwrap_or_default();
        let pid = ProcId(iso.0);
        let live = if pid == Gpu::SYSTEM_PROC {
            Some(&gpu.system)
        } else {
            gpu.procs.get(&pid)
        };
        let Some(proc) = live else {
            // §7.0: the proc is gone, so its isolate session is gone with it. Whatever
            // is outstanding was disposed of in bulk by the namespace's death.
            if !outstanding.is_empty() {
                c.session_death_objects.insert(iso, outstanding);
            }
            if !outstanding_maps.is_empty() {
                c.session_death_maps.insert(iso, outstanding_maps);
            }
            continue;
        };
        let reachable = reachable_objects(proc);
        let reachable_m = reachable_maps(proc);
        let leaked: BTreeSet<HostHandle> = outstanding.difference(&reachable).copied().collect();
        let dangling: BTreeSet<HostHandle> = reachable.difference(&outstanding).copied().collect();
        let leaked_m: BTreeSet<_> = outstanding_maps.difference(&reachable_m).copied().collect();
        let dangling_m: BTreeSet<_> = reachable_m.difference(&outstanding_maps).copied().collect();
        if !leaked.is_empty() {
            c.leaked_objects.insert(iso, leaked);
        }
        if !dangling.is_empty() {
            c.dangling_objects.insert(iso, dangling);
        }
        if !leaked_m.is_empty() {
            c.leaked_maps.insert(iso, leaked_m);
        }
        if !dangling_m.is_empty() {
            c.dangling_maps.insert(iso, dangling_m);
        }
    }
    // ★ R6's intra-arena half, per (proc, target): what the arena still has handed out
    // versus what the proc's own `Vas::blocks` tokens can still name. `GpaBlock` is
    // move-only, so a block dropped with its `Vas` is unrecoverable by construction —
    // which is exactly why the difference is a *leak* and not merely an accounting gap.
    for (&pid, proc) in gpu
        .procs
        .iter()
        .chain(core::iter::once((&Gpu::SYSTEM_PROC, &gpu.system)))
    {
        for (&g, arena) in &proc.arenas {
            let named: u64 = proc
                .vases
                .iter()
                .filter(|&(&(vg, _), _)| vg == g)
                .flat_map(|(_, v)| v.blocks.values())
                .map(|b| b.len)
                .sum();
            let live = arena.live_bytes();
            if live > named {
                c.leaked_gpa_bytes.insert((pid, g), live - named);
            }
        }
    }
    c
}

/// Print the census as NUMBERS — objects and mappings, by isolate and by class. The
/// stage's product is the honest baseline, so it is reported whether or not it is zero
/// (`l1_os_shell.md` §10, M2-a: *"that number, whatever it is, is the honest baseline
/// every later stage is gated against"*). Straight to stderr, bypassing libtest's
/// capture, for the same reason [`kayfabe_tests::skip_slow`] does.
fn report_census(c: &Census, rec: &SharedRecorder, mode: LockMode) {
    use std::io::Write as _;
    let class = classify_handles(rec);
    let mut e = std::io::stderr();
    let _ = writeln!(e, "\n=== CONSERVATION LEDGER — mean run, {mode:?} ===");
    let by_class = |set: &BTreeMap<IsolateId, BTreeSet<HostHandle>>| {
        let mut counts: BTreeMap<HostClass, usize> = BTreeMap::new();
        for h in set.values().flatten() {
            *counts
                .entry(class.get(h).copied().unwrap_or(HostClass::Other))
                .or_default() += 1;
        }
        counts
    };
    let _ = writeln!(
        e,
        "  ★ LEAKED (live proc, outstanding, UNREACHABLE from core state — no backstop):\n\
               objects {} {:?}\n      mappings {}",
        c.leaked_object_count(),
        by_class(&c.leaked_objects),
        c.leaked_map_count(),
    );
    for (iso, set) in &c.leaked_objects {
        let mut per: BTreeMap<HostClass, usize> = BTreeMap::new();
        for h in set {
            *per.entry(class.get(h).copied().unwrap_or(HostClass::Other))
                .or_default() += 1;
        }
        let maps = c.leaked_maps.get(iso).map_or(0, BTreeSet::len);
        let _ = writeln!(e, "      {iso:?}: {per:?} + {maps} mapping(s)");
    }
    let _ = writeln!(
        e,
        "  DANGLING (core names it, ledger says freed — use-after-free shape): objects {} mappings {}",
        c.dangling_objects
            .values()
            .map(BTreeSet::len)
            .sum::<usize>(),
        c.dangling_maps.values().map(BTreeSet::len).sum::<usize>(),
    );
    let _ = writeln!(
        e,
        "  namespace-death residue (§7.0 backstop, reaped procs): objects {} {:?} mappings {}",
        c.session_death_object_count(),
        by_class(&c.session_death_objects),
        c.session_death_map_count(),
    );
    let _ = writeln!(
        e,
        "  ★ LEAKED GPA (live proc, arena bytes no `Vas::blocks` token can name): {} bytes {:?}",
        c.leaked_gpa_total(),
        c.leaked_gpa_bytes,
    );
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
        // ★★ §7.8 — the conservation ledger, as a fact of the composed run and of the
        // cross-mode differential (the whole report is compared below, so a leak that
        // appeared in only ONE lock mode would fail there instead).
        assert_eq!(
            r.leaked,
            (0, 0),
            "({name}) ★★ T0 (G2): host objects/mappings outstanding on a LIVE proc that \
             core state cannot name. Pre-fix baseline was (24, 6)."
        );
        assert_eq!(
            r.session_death,
            (6, 2),
            "({name}) the §7.0 namespace-death residue is the script's own: the two \
             out-of-band-retired procs' host state, disposed of by their isolate \
             sessions' death and by nothing else"
        );
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
    let gpu = device.map(|d| {
        Arc::try_unwrap(d)
            .unwrap_or_else(|_| panic!("every device handle was released"))
            .into_gpu()
    });
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
    // ★ §12.35 — DECLARED RESIDUE: the victim is condemned out of band, so its isolate is
    // stopped and its host VAS + backing go to §7.0 namespace death. The GPA half IS
    // conserved and is what this test is actually about.
    gpu.declare_residue(
        ResidueClaim::on(
            IsolateId(victim.0),
            "condemned component: `retire_proc` stopped the isolate, so the warmed host \
             VAS + backing are disposed of by the session's death (§7.0) — the ARENA, \
             which is this test's subject, is conserved separately and asserted below",
        )
        .objects(VerbKind::AllocVaSpace, 1)
        .objects(VerbKind::AllocSysmem, 1)
        .maps(1),
    );

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
    // ★ §12.27 — a USER peer's dup, which is the shape that actually merges. (A UVM
    // dup no longer does: the session client is the guest kernel's, so its edge is a
    // reference. `peer_dup` is the genuine-sharing edge the `LateMerge` guard is for.)
    let uvm = HClient(0xD0);
    let mut s = Scenario::new();
    s.peer_dup(
        uvm,
        HObject(0x7000_0000),
        HObject(0x7000_0001),
        HObject(0x7000_0010),
        Pdb(0x3800_0000),
        HObject(0x7000_00a0),
        compute_vas,
    );
    for ev in s.events {
        gpu.apply(ev).expect("the peer dup applies");
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

/// ★★ **A `DUP_OBJECT` across the condemnation line MERGES NOTHING** — it is a
/// reference, and it moves neither component (`l1_concurrency.md` §12.37).
///
/// This test used to assert `GpuError::CondemnedMerge`: dup-connected clients are one
/// `Proc` by construction, so honouring the merge would either resurrect the condemned
/// clients around the live proc's working isolate or condemn a healthy proc, and
/// refusing the *event* was the only honest answer left. The answer that was missing is
/// the fourth one — **do not make it a merge at all** — and it is the one that had to
/// exist, because the loud refusal was only ever reachable when the dragged-in side
/// already had a live `Proc`. Planting the identical dup into a namespace that had not
/// declared yet fired the same merge on the *victim's* own client-root alloc, with no
/// proc to protect and therefore in silence
/// ([`a_planted_dup_alias_cannot_condemn_a_client_that_has_not_declared`]).
///
/// So the grouping predicate now refuses to merge across the condemnation line, and what
/// this test pins is that the outcome is *better* than the refusal it replaces: the live
/// proc is untouched and still serving, the condemned component is still condemned by
/// name, no `Proc` holds a condemned client, and the aliased resource — attributed to
/// its ORIGIN, which is condemned — still answers `FwdFault::Condemned` wherever it is
/// reached. Nothing is resurrected, and no bystander pays.
#[test]
fn a_dup_across_the_condemnation_line_merges_nothing() {
    let _wd = watchdog("condemned_line_no_merge", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let victim = pids[P_HUP];
    let condemned = FwdFault::Condemned {
        anchor: ProcAnchor(client_of(P_HUP)),
    };

    core_publish(&mut gpu, GPU1, lane_of(P_HUP).pdb, VA_WARM).expect("publishes while alive");
    assert!(core_retire_out_of_band(&mut gpu, victim));

    let before = gpu.procs.len();
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(client_of(P_HUP), H_VASPACE),
        dst: NodeKey::new(client_of(P_PEER), HObject(0x7200_00a0)),
    })
    .expect("★ the dup applies — RM refcounting is faithful, the alias is just not a merge");

    // NOTHING MOVED: the live proc is untouched and still serving, the condemned one is
    // still condemned, and nothing was minted, merged or absorbed.
    assert_eq!(gpu.procs.len(), before, "the dup minted or dropped a proc");
    assert!(
        !gpu.spine.is_condemned(client_of(P_PEER)),
        "★★ a guest killed a healthy process by dupping into a corpse"
    );
    assert_eq!(gpu.spine.condemned_len(), 1);
    assert!(
        !gpu.procs[&pids[P_PEER]].clients.contains(&client_of(P_HUP)),
        "★ the dup put the condemned client inside a live proc — that IS the resurrect"
    );
    assert_eq!(
        gpu.procs[&pids[P_PEER]].clients,
        BTreeSet::from([client_of(P_PEER)]),
        "the live component is exactly what it was"
    );
    core_publish(&mut gpu, GPU1, lane_of(P_PEER).pdb, VA_CTL)
        .expect("★ the live proc keeps serving");

    // ★ …and the corpse is still a corpse, on both planes — including through the
    // freshly-minted alias, because attribution is by ORIGIN.
    assert_eq!(
        core_publish(&mut gpu, GPU1, lane_of(P_HUP).pdb, VA_HELD),
        Err(condemned),
        "★★ the alias resurrected the condemned component's address plane"
    );
    assert_eq!(
        kayfabe_fwd::handle_doorbell(&mut gpu, GPU1, MockArch::token_for(lane_of(P_HUP).gr), &[]),
        Err(condemned),
        "★★ the alias revived the condemned component's exec plane"
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
        out.extend(vas.host_vas.map(|h| h.raw()));
        for (_va, _len, b) in vas.table.iter() {
            out.extend(b.host_memory().map(|h| h.raw()));
        }
    }
    for c in p.channels.values() {
        out.extend(c.host_channel.map(|h| h.raw()));
        out.extend(c.host_engine_objects.values().map(|h| h.raw()));
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
    // ★ §12.35 — DECLARED RESIDUE: same condemnation shape, one object larger because
    // this test also materializes the victim's host channel before killing it.
    gpu.declare_residue(
        ResidueClaim::on(
            IsolateId(victim.0),
            "condemned component: the warmed host VAS + backing + channel are the §7.0 \
             namespace-death residue of a stopped isolate; the point of the test is that \
             the RECOVERED component shares none of them",
        )
        .objects(VerbKind::AllocVaSpace, 1)
        .objects(VerbKind::AllocSysmem, 1)
        .objects(VerbKind::AllocChannel, 1)
        .maps(1),
    );

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

// =================================================================================
// ★★ BOUNDARY-1, C1/C2 — **a hostile stream must only ever earn its OWN refusal**
//
// `Spine::apply`'s own contract ("a hostile stream can only ever earn its own loud
// refusal") and `Spine::condemned`'s ("it must never reach a client that did not
// [share the blast radius]") were both breakable, by two halves of one defect:
//
//  - **C1** — a `DUP_OBJECT` planted into a namespace that has not declared itself is
//    accepted and parked, and fires the instant the *victim* declares. With no live
//    proc to protect, the condemnation is SILENT: the victim's own first RM event
//    returns `Ok(())` and kills it, permanently, anchored at the attacker's client.
//  - **C2** — a condemned entry retained client handles the guest had since FREED, so
//    the entry poisoned handle VALUES the guest kernel hands out again. A recycled
//    value never shared the blast radius.
//
// The property both tests state, and the one the fix must hold: **the victim is
// unaffected, and the attacker's planted edge earns the ATTACKER a refusal or nothing
// at all.**
// =================================================================================

/// The never-allocated client namespace the attacker plants an alias into. RM's
/// `hClient` **is** its root object's handle, so the value a future process will be
/// handed is predictable — the attack costs one dup per candidate namespace.
const VICTIM_CLIENT: HClient = HClient(0xC0);
/// The victim's own PDB, once it declares.
const VICTIM_PDB: Pdb = Pdb(0x4a00_0000);
/// The victim's GR vChid.
const VICTIM_GR: VChid = VChid(0x160);
/// The victim's CE vChid.
const VICTIM_CE: VChid = VChid(0x260);
/// The handle the planted alias squats inside the victim's namespace.
const PLANTED_ALIAS: HObject = HObject(0x7777_0001);

/// ★★ **C1 — a planted dup alias must not silently condemn a client that has not even
/// declared itself.**
///
/// The composed attack: the attacker's own component is condemned (its worker died),
/// and it then dups one of its own objects into a namespace **nobody owns yet**. That
/// dup is inert while the destination is undeclared — and under the defect it fires on
/// the victim's `Alloc(Client, User)`, i.e. on the *same apply that first creates the
/// victim's boundary*, so no live proc exists to make it a loud `CondemnedMerge` and
/// the victim dies silently, anchored at the attacker's client handle.
///
/// This is the first shape found in this core that lets a hostile stream earn ANOTHER
/// process's refusal, which is why it is asserted end to end (declare → publish → ring)
/// rather than on `is_condemned` alone.
///
/// ★★ **UPDATED by §12.38.** The property is unchanged and still holds; the *mechanism*
/// moved one layer earlier, and this test now says so. §12.37 removed the transfer of
/// fatality (the condemnation line in the grouping predicate); §12.38 removes the
/// planted alias itself — a `DUP_OBJECT` into a namespace with no declared client root
/// is a protocol violation RM refuses outright, so it never enters the graph. The
/// attacker therefore earns a **loud refusal on its own event**, which is the strongest
/// form of boundary-1 available: `Spine::apply`'s contract, satisfied at the event that
/// violates it. The end-to-end half is kept verbatim, because "the victim is served"
/// must remain an assertion about the victim's own plane and not about the graph.
#[test]
fn a_planted_dup_alias_cannot_condemn_a_client_that_has_not_declared() {
    let _wd = watchdog("c1_planted_alias", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let (attacker, lane) = (pids[P_HUP], lane_of(P_HUP));
    let condemned = FwdFault::Condemned {
        anchor: ProcAnchor(client_of(P_HUP)),
    };

    core_publish(&mut gpu, GPU1, lane.pdb, VA_WARM).expect("the attacker publishes while alive");
    assert!(core_retire_out_of_band(&mut gpu, attacker));
    assert!(gpu.spine.is_condemned(client_of(P_HUP)));

    // ---- ★ The attack: ONE dup, out of the condemned component, into a namespace
    // that does not exist. ★ §12.38 — it is REFUSED on the attacker's own event, by
    // name, because RM itself resolves `hClientDst` before anything else and refuses a
    // namespace that has never declared a root. The refusal is mutation-free.
    assert_eq!(
        gpu.apply(RmEvent::Dup {
            src: NodeKey::new(client_of(P_HUP), H_VASPACE),
            dst: NodeKey::new(VICTIM_CLIENT, PLANTED_ALIAS),
        }),
        Err(GpuError::Graph(RmGraphError::UndeclaredClient(
            VICTIM_CLIENT
        ))),
        "★★ the hostile stream must earn its OWN loud refusal, on its OWN event"
    );
    assert!(
        !gpu.spine.is_condemned(VICTIM_CLIENT),
        "an undeclared namespace cannot be condemned — there is nothing there yet"
    );
    assert_eq!(gpu.spine.condemned_len(), 1);

    // ---- ★★ The victim's OWN first event, and every event of its own bring-up.
    reinit(
        &mut gpu,
        VICTIM_CLIENT,
        VICTIM_PDB,
        VICTIM_GR,
        VICTIM_CE,
        GPU1,
    );

    assert!(
        !gpu.spine.is_condemned(VICTIM_CLIENT),
        "★★ the victim was condemned by an edge IT never created — a hostile stream \
         earned another process's refusal, which boundary-1 forbids"
    );
    let pid = gpu.spine.by_pdb.get(&(GPU1, VICTIM_PDB)).copied().expect(
        "★★ the victim's own address plane was never materialized — it was \
                 condemned on the apply that declared it, and nobody was told",
    );
    assert!(gpu.procs[&pid].clients.contains(&VICTIM_CLIENT));
    assert!(
        !gpu.procs[&pid].clients.contains(&client_of(P_HUP)),
        "★ the victim's proc absorbed the condemned client — that IS the resurrect"
    );

    // Genuinely served, end to end, through the #14 ring-gate.
    let rung = publish_and_ring(&mut gpu, GPU1, VICTIM_PDB, VICTIM_GR, VA_CTL);
    assert_eq!(
        rung.proc, pid,
        "the victim's ring routed to the victim's proc"
    );
    assert_eq!(
        token_lane(rung.host_token),
        (pid.0 + 1, GPU1.0),
        "★ the victim rang a host token minted in its OWN isolate lane"
    );

    // ---- ★ …and the attacker earned NOTHING. Its own plane is still condemned, by
    // name, and the resource it aliased is still refused wherever it is reached.
    assert_eq!(
        core_publish(&mut gpu, GPU1, lane.pdb, VA_HELD),
        Err(condemned),
        "★★ the planted alias resurrected the condemned component's address plane"
    );
    assert_eq!(
        kayfabe_fwd::handle_doorbell(&mut gpu, GPU1, MockArch::token_for(lane.gr), &[]),
        Err(condemned),
        "★★ the planted alias revived the condemned component's exec plane"
    );
    assert!(gpu.spine.is_condemned(client_of(P_HUP)));
    assert_eq!(
        gpu.spine.condemned_len(),
        1,
        "one corpse, still exactly one"
    );
}

/// The victim's PDB in the **squat** variant (a separate value from [`VICTIM_PDB`] so
/// the two attacks can never share a routing entry through a stale map).
const SQUAT_VICTIM_PDB: Pdb = Pdb(0x4a10_0000);
/// The squat victim's GR vChid.
const SQUAT_VICTIM_GR: VChid = VChid(0x164);
/// The squat victim's CE vChid.
const SQUAT_VICTIM_CE: VChid = VChid(0x264);

/// ★★★ **C3 — the SAME planted alias, without any condemnation, MERGES an unrelated
/// later process into the ATTACKER's live `Proc`** (`l1_concurrency.md` §12.38).
///
/// §12.37 fixed the condemnation half of C1 and reported this half open. It is the worse
/// one: nothing here is condemned, so the condemnation line never applies. The attacker
/// is an ordinary live compute process; it plants one `DUP_OBJECT` of its own VASpace
/// into a namespace nobody owns yet; the victim later arrives as an ordinary compute
/// process at that `hClient`. The parked edge resolves, both ends are declared `User`,
/// both are alive — so it is a *grouping* edge, and the two become **one `Proc`: one
/// isolate, one GPA arena, one host VAS**. That is #14 un-fixed for a chosen pair,
/// reachable with one event and a guessable `hClient` (RM's `hClient` **is** its root
/// object's handle).
///
/// **The fix is at event acceptance, not at projection** — a `Dup` whose destination
/// namespace has never declared a client root is refused, exactly as RM refuses it
/// (`ogkm src/nvidia/src/libraries/resserv/src/rs_server.c:1674` →
/// `_serverLockDualClientWithLockInfo` → `NV_ERR_INVALID_OBJECT_HANDLE` at `:3547-3550`;
/// `clientValidate`'s `NV_ERR_INVALID_CLIENT`, `rmapi/client.c:782`, is the same refusal
/// one layer up). No legal protocol trace is lost, because RM would never have emitted
/// the RPC — see §12.38's corrected criterion.
///
/// The order of the assertions is deliberate: the **end-state isolation claim comes
/// first**, so reverting the fix reports the breach itself ("the victim landed in the
/// attacker's proc") rather than merely "the dup was accepted".
#[test]
fn a_planted_dup_alias_cannot_squat_a_later_process_into_the_attackers_proc() {
    let _wd = watchdog("c3_squat_into_live_proc", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let (attacker, lane) = (pids[P_WITNESS], lane_of(P_WITNESS));

    // The attacker is an ordinary, LIVE, publishing process — no condemnation anywhere.
    core_publish(&mut gpu, GPU0, lane.pdb, VA_WARM).expect("the attacker publishes while alive");
    assert_eq!(gpu.spine.condemned_len(), 0, "nothing is condemned here");
    let attacker_iso = gpu.procs[&attacker].isolates[&GPU0].id();
    let attacker_arena = gpu.procs[&attacker].arenas[&GPU0].range.clone();
    let attacker_hosts = host_identities(&gpu.procs[&attacker]);

    // ---- ★ The attack: ONE dup, out of the attacker's live component, into a namespace
    // that has never declared a client root.
    let planted = gpu.apply(RmEvent::Dup {
        src: NodeKey::new(client_of(P_WITNESS), H_VASPACE),
        dst: NodeKey::new(VICTIM_CLIENT, PLANTED_ALIAS),
    });

    // ---- The victim arrives: an ordinary compute process that happens to be handed the
    // squatted `hClient`. Its own first event is its own client-root `Alloc`.
    reinit(
        &mut gpu,
        VICTIM_CLIENT,
        SQUAT_VICTIM_PDB,
        SQUAT_VICTIM_GR,
        SQUAT_VICTIM_CE,
        GPU0,
    );

    let victim = gpu
        .spine
        .by_pdb
        .get(&(GPU0, SQUAT_VICTIM_PDB))
        .copied()
        .expect("the victim's own address plane materialized");
    assert_ne!(
        victim, attacker,
        "★★ the victim was merged into the ATTACKER's `Proc` by an edge it never \
         created — one isolate, one GPA arena, one host VAS, i.e. #14 un-fixed for a \
         pair the attacker chose"
    );
    assert_eq!(
        gpu.procs[&victim].clients,
        BTreeSet::from([VICTIM_CLIENT]),
        "★★ the victim's component must contain the victim's client and nothing else"
    );
    assert!(
        !gpu.procs[&attacker].clients.contains(&VICTIM_CLIENT),
        "★★ the attacker's component absorbed the victim's client"
    );

    // Its OWN isolate, its OWN arena — disjoint by construction.
    let victim_iso = gpu.procs[&victim].isolates[&GPU0].id();
    assert_ne!(victim_iso, attacker_iso, "★★ the two share one isolate");
    let victim_arena = gpu.procs[&victim].arenas[&GPU0].range.clone();
    assert!(
        victim_arena.end <= attacker_arena.start || attacker_arena.end <= victim_arena.start,
        "★★ the two share one GPA arena: {victim_arena:?} vs {attacker_arena:?}"
    );

    // …and it is genuinely SERVED, end to end, out of its own isolate lane — sharing
    // **no** host handle with the attacker.
    let rung = publish_and_ring(&mut gpu, GPU0, SQUAT_VICTIM_PDB, SQUAT_VICTIM_GR, VA_CTL);
    assert_eq!(rung.proc, victim, "the victim's ring routed to the victim");
    assert_eq!(
        token_lane(rung.host_token),
        (victim.0 + 1, GPU0.0),
        "★ the victim rang a host token minted in its OWN isolate lane"
    );
    assert!(
        host_identities(&gpu.procs[&victim]).is_disjoint(&attacker_hosts),
        "★★ the victim and the attacker share a host object"
    );

    // ---- ★ And the mechanism that makes all of the above true: the planted dup was
    // refused, by name, at acceptance — so the bad state is unrepresentable rather than
    // merely undetected downstream.
    assert_eq!(
        planted,
        Err(GpuError::Graph(RmGraphError::UndeclaredClient(
            VICTIM_CLIENT
        ))),
        "★★ a `DUP_OBJECT` into a namespace with no declared client root must be refused \
         (RM: `NV_ERR_INVALID_CLIENT`), not parked to fire on the victim's own `Alloc`"
    );
    // The refusal is mutation-free: no alias, no phantom client, no parked edge.
    assert!(
        !gpu.spine
            .rmgraph
            .dups()
            .any(|(d, _)| d == NodeKey::new(VICTIM_CLIENT, PLANTED_ALIAS)),
        "★ the refused dup left a parked edge behind — the refusal must mutate nothing"
    );

    // The attacker itself is undisturbed: its own refusal, and nothing else.
    assert_eq!(gpu.procs[&attacker].id, attacker);
    core_publish(&mut gpu, GPU0, lane.pdb, VA_HELD).expect("the attacker keeps working");
}

/// The first extra client dup-joined into [`P_HUP`]'s component before it dies — a
/// handle value the guest then FREES, and which a future process may be handed.
const JOINED_A: HClient = HClient(0xC8);
/// The second joined client.
const JOINED_B: HClient = HClient(0xC9);
/// [`JOINED_A`]'s PDB while it is part of the doomed component.
const JOINED_A_PDB: Pdb = Pdb(0x4b00_0000);
/// [`JOINED_B`]'s PDB while it is part of the doomed component.
const JOINED_B_PDB: Pdb = Pdb(0x4c00_0000);
/// The PDB of the LATER, unrelated process that is handed [`JOINED_A`]'s freed value.
const RECYCLED_PDB: Pdb = Pdb(0x4d00_0000);
/// The alias handle each joined client holds the shared VASpace under.
const JOIN_ALIAS: HObject = HObject(0x7800_0001);

/// ★★ **C2 — a condemned entry must not retain client handles the guest has freed.**
///
/// Handle reuse is explicit design (`RmGraph`'s resource/handle split exists for it),
/// and `RmGraph::drop_handle` prunes the `client_roots` entry on free *precisely so*
/// the namespace can be re-declared. The carry-forward re-added the WHOLE old condemned
/// entry on every refresh, including handles that by then existed nowhere in the graph
/// — so an attacker could buy N poisoned namespaces for the price of keeping ONE client
/// alive, and any later process whose `hClient` landed on a freed value died the moment
/// it declared.
///
/// That contradicts the entry's own invariant one screen above it: *"Absorb, never
/// widen … it must never reach a client that did not [share the blast radius]."* **A
/// recycled handle value never shared the blast radius.**
#[test]
fn a_condemned_entry_must_not_poison_client_handles_the_guest_has_freed() {
    let _wd = watchdog("c2_freed_handle_recycle", Duration::from_secs(60));
    let (mut gpu, _pids, _rec) = mean_gpu();
    let lane = lane_of(P_HUP);
    let root = identical_handles(0, 0).client_root;
    let condemned = FwdFault::Condemned {
        anchor: ProcAnchor(client_of(P_HUP)),
    };

    // ---- One component of THREE clients, joined by genuine user↔user shares.
    reinit(
        &mut gpu,
        JOINED_A,
        JOINED_A_PDB,
        VChid(0x161),
        VChid(0x261),
        GPU1,
    );
    reinit(
        &mut gpu,
        JOINED_B,
        JOINED_B_PDB,
        VChid(0x162),
        VChid(0x262),
        GPU1,
    );
    for c in [JOINED_A, JOINED_B] {
        gpu.apply(RmEvent::Dup {
            src: NodeKey::new(client_of(P_HUP), H_VASPACE),
            dst: NodeKey::new(c, JOIN_ALIAS),
        })
        .expect("the user↔user share applies");
    }
    let doomed = gpu.spine.by_pdb[&(GPU1, lane.pdb)];
    assert_eq!(
        gpu.procs[&doomed].clients,
        BTreeSet::from([client_of(P_HUP), JOINED_A, JOINED_B]),
        "the three clients are ONE component"
    );

    // ---- Its worker dies: all three clients are condemned, correctly.
    core_publish(&mut gpu, GPU1, lane.pdb, VA_WARM).expect("it publishes while alive");
    assert!(core_retire_out_of_band(&mut gpu, doomed));
    for c in [client_of(P_HUP), JOINED_A, JOINED_B] {
        assert!(gpu.spine.is_condemned(c), "the component that died is dead");
    }

    // ---- ★ The guest FREES two of the three client roots — their namespaces now
    // exist nowhere in the graph. The attacker keeps only the anchor alive.
    for c in [JOINED_A, JOINED_B] {
        gpu.apply(RmEvent::Free {
            client: c,
            handle: root,
        })
        .expect("the guest's client-root free applies");
    }
    assert!(
        gpu.spine.is_condemned(client_of(P_HUP)),
        "the component that actually died must stay dead"
    );
    assert_eq!(gpu.spine.condemned_len(), 1);
    for c in [JOINED_A, JOINED_B] {
        assert!(
            !gpu.spine.is_condemned(c),
            "★★ a client handle the guest FREED is still condemned — the entry is \
             poisoning a VALUE, and a recycled value never shared the blast radius"
        );
    }

    // ---- ★★ A later, unrelated process is handed the recycled `hClient`.
    reinit(
        &mut gpu,
        JOINED_A,
        RECYCLED_PDB,
        VChid(0x163),
        VChid(0x263),
        GPU1,
    );
    assert!(
        !gpu.spine.is_condemned(JOINED_A),
        "★★ an unrelated process was condemned on arrival because its hClient landed \
         on a value a dead component once held"
    );
    let pid = gpu
        .spine
        .by_pdb
        .get(&(GPU1, RECYCLED_PDB))
        .copied()
        .expect("★★ the recycled namespace's own address plane was never materialized");
    let rung = publish_and_ring(&mut gpu, GPU1, RECYCLED_PDB, VChid(0x163), VA_CTL);
    assert_eq!(rung.proc, pid);
    assert!(!gpu.procs[&pid].clients.contains(&client_of(P_HUP)));

    // ---- …and the corpse the attacker DID earn is still exactly as dead.
    assert_eq!(
        core_publish(&mut gpu, GPU1, lane.pdb, VA_HELD),
        Err(condemned),
        "★★ pruning the freed handles let the condemned component back to life"
    );
    assert_eq!(gpu.spine.condemned_len(), 1);
}

// =================================================================================
// ★★ §12.37 — THE THREE EVASIONS the condemned entry's growth exists to stop, re-run
// against the two fixes above. C1 removed the merge across the condemnation line and
// C2 made the entry shrink to the clients the graph still knows; neither may hand the
// guest a way to launder a dead component back to life.
// =================================================================================

/// The fresh client an attacker allocates to try to *carry* a condemned component's
/// resource back into a live `Proc`.
const LAUNDER_CLIENT: HClient = HClient(0xD0);
/// Its own PDB.
const LAUNDER_PDB: Pdb = Pdb(0x4e00_0000);
/// The handle it aliases the condemned VASpace under.
const LAUNDER_ALIAS: HObject = HObject(0x7900_0001);
/// A channel of its own, deliberately BOUND to the aliased (condemned) VASpace.
const LAUNDER_CHANNEL: HObject = HObject(0x5c00_00b0);
/// That channel's vChid.
const LAUNDER_VCHID: VChid = VChid(0x370);

/// ★ **Evasion 1 — dup a FRESH client onto the corpse, then free the corpse's root.**
///
/// The oldest laundering shape: if the condemned resource could be re-attributed to the
/// fresh client's live component, the guest would get its dead-backed VASpace served out
/// of a working isolate — a **zeroed** backing for a VA it believes still holds its data,
/// which is the silent corruption "never resurrect" exists to prevent.
///
/// Attribution is by ORIGIN and the origin is condemned, so neither half works: the dup
/// does not merge (the condemnation line), and freeing the condemned client's root does
/// not release the entry while its resource is still alive in the graph on the
/// attacker's own alias.
#[test]
fn evasion_dup_a_fresh_client_then_free_the_old_root_still_fails() {
    let _wd = watchdog("evasion_launder", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let (victim, lane) = (pids[P_HUP], lane_of(P_HUP));
    let condemned = FwdFault::Condemned {
        anchor: ProcAnchor(client_of(P_HUP)),
    };

    core_publish(&mut gpu, GPU1, lane.pdb, VA_WARM).expect("publishes while alive");
    assert!(core_retire_out_of_band(&mut gpu, victim));

    // A fresh client — live, served, its own everything — aliases the corpse's VASpace.
    reinit(
        &mut gpu,
        LAUNDER_CLIENT,
        LAUNDER_PDB,
        VChid(0x170),
        VChid(0x270),
        GPU1,
    );
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(client_of(P_HUP), H_VASPACE),
        dst: NodeKey::new(LAUNDER_CLIENT, LAUNDER_ALIAS),
    })
    .expect("the alias applies");
    assert!(
        !gpu.spine.is_condemned(LAUNDER_CLIENT),
        "the fresh client lives"
    );

    // ★ …and it goes further than holding the handle: it allocates a channel BOUND to
    // the aliased (condemned) VASpace. This is the one genuinely new state the
    // condemnation line creates — a live `Proc` naming an address plane it does not own
    // — and it must be a named refusal, not a served ring. `gate_working_set_in` looks
    // the PDB up in the ringing proc's OWN vases, which the condemned VAS is not in.
    gpu.apply(RmEvent::Alloc {
        client: LAUNDER_CLIENT,
        parent: identical_handles(0, 0).tsg,
        handle: LAUNDER_CHANNEL,
        class: mock_classes::CHANNEL_GR,
        facts: AllocFacts {
            h_vaspace: Some(LAUNDER_ALIAS),
            userd_flags: MockArch::userd_flags_for(LAUNDER_VCHID),
            ..Default::default()
        },
    })
    .expect("the cross-line channel applies");
    assert_eq!(
        kayfabe_fwd::handle_doorbell(
            &mut gpu,
            GPU1,
            MockArch::token_for(LAUNDER_VCHID),
            &[GpuVa(VA_WARM)],
        ),
        Err(FwdFault::UnknownPdb {
            gpu: GPU1,
            pdb: lane.pdb,
        }),
        "★★ a live proc rang a channel bound to a CONDEMNED component's address plane"
    );

    // ---- …and now the corpse's own client root goes, leaving the resource alive on
    // nothing but the attacker's alias.
    gpu.apply(RmEvent::Free {
        client: client_of(P_HUP),
        handle: identical_handles(lane.gr.0, lane.ce.0).client_root,
    })
    .expect("the client-root free applies");

    assert!(
        gpu.spine.is_condemned(client_of(P_HUP)),
        "★★ freeing the root while a dup keeps the resource alive laundered the \
         condemnation away — the dead-backed VASpace is now servable"
    );
    assert_eq!(
        core_publish(&mut gpu, GPU1, lane.pdb, VA_HELD),
        Err(condemned),
        "★★ the condemned VASpace was resurrected through a fresh client's alias"
    );
    assert_eq!(gpu.spine.condemned_len(), 1);
    assert!(
        gpu.procs
            .values()
            .all(|p| !p.clients.contains(&client_of(P_HUP))),
        "★ a live proc absorbed the condemned client"
    );
    // The attacker's own fresh component is unaffected either way — it never shared the
    // blast radius, and it never acquires it.
    core_publish(&mut gpu, GPU1, LAUNDER_PDB, VA_CTL).expect("the fresh client keeps serving");
}

/// The second client of the doomed component, for the split / re-label evasions.
const SPLIT_CLIENT: HClient = HClient(0xD8);
/// Its PDB.
const SPLIT_PDB: Pdb = Pdb(0x4f00_0000);
/// The handle it holds the shared VASpace under.
const SPLIT_ALIAS: HObject = HObject(0x7a00_0001);

/// ★ **Evasion 2 — SPLIT the condemned component by freeing the edge that joined it.**
///
/// A component of two clients dies; the guest then frees the `DUP_OBJECT` that made them
/// one, so the next projection derives TWO boundaries. Both halves must stay condemned
/// — the entry is a client SET, so both intersect it — and they must stay **one** entry,
/// or the caps and the diagnostics start counting corpses that do not exist.
#[test]
fn evasion_splitting_the_condemned_component_still_fails() {
    let _wd = watchdog("evasion_split", Duration::from_secs(60));
    let (mut gpu, _pids, _rec) = mean_gpu();
    let lane = lane_of(P_HUP);

    reinit(
        &mut gpu,
        SPLIT_CLIENT,
        SPLIT_PDB,
        VChid(0x171),
        VChid(0x271),
        GPU1,
    );
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(client_of(P_HUP), H_VASPACE),
        dst: NodeKey::new(SPLIT_CLIENT, SPLIT_ALIAS),
    })
    .expect("the user↔user share applies");
    let doomed = gpu.spine.by_pdb[&(GPU1, lane.pdb)];
    assert_eq!(
        gpu.spine.by_pdb[&(GPU1, SPLIT_PDB)],
        doomed,
        "ONE component"
    );
    core_publish(&mut gpu, GPU1, lane.pdb, VA_WARM).expect("publishes while alive");
    assert!(core_retire_out_of_band(&mut gpu, doomed));

    // ---- The guest frees the joining edge: two boundaries where there was one.
    gpu.apply(RmEvent::Free {
        client: SPLIT_CLIENT,
        handle: SPLIT_ALIAS,
    })
    .expect("the alias frees");

    // Each half is now its own boundary, so each answers under its OWN anchor — but
    // both answer `Condemned`, which is the property.
    for (c, pdb) in [(client_of(P_HUP), lane.pdb), (SPLIT_CLIENT, SPLIT_PDB)] {
        assert!(
            gpu.spine.is_condemned(c),
            "★★ splitting the component un-condemned half of it"
        );
        assert_eq!(
            core_publish(&mut gpu, GPU1, pdb, VA_HELD),
            Err(FwdFault::Condemned {
                anchor: ProcAnchor(c),
            }),
            "★★ a split-off half of a condemned component was served again"
        );
    }
    assert_eq!(
        gpu.spine.condemned_len(),
        1,
        "★ the split duplicated the entry — both halves are still ONE corpse"
    );
}

/// ★ **Evasion 3 — RE-LABEL the condemned component by freeing its anchor client.**
///
/// The component's `ProcAnchor` is its smallest client handle, so freeing that client
/// re-labels the survivor. Condemnation is keyed on the client SET, never on the label,
/// so the survivor stays dead — and, with C2's shrink, the entry now also stops naming
/// the handle value the guest genuinely gave back.
#[test]
fn evasion_relabelling_the_condemned_component_still_fails() {
    let _wd = watchdog("evasion_relabel", Duration::from_secs(60));
    let (mut gpu, _pids, _rec) = mean_gpu();
    let lane = lane_of(P_HUP);
    assert!(
        client_of(P_HUP) < SPLIT_CLIENT,
        "the anchor must be the client we are about to free, or this proves nothing"
    );

    reinit(
        &mut gpu,
        SPLIT_CLIENT,
        SPLIT_PDB,
        VChid(0x172),
        VChid(0x272),
        GPU1,
    );
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(SPLIT_CLIENT, H_VASPACE),
        dst: NodeKey::new(client_of(P_HUP), SPLIT_ALIAS),
    })
    .expect("the user↔user share applies");
    let doomed = gpu.spine.by_pdb[&(GPU1, lane.pdb)];
    core_publish(&mut gpu, GPU1, lane.pdb, VA_WARM).expect("publishes while alive");
    assert!(core_retire_out_of_band(&mut gpu, doomed));
    assert!(gpu.spine.is_condemned(SPLIT_CLIENT));

    // ---- The guest frees the ANCHOR client's whole namespace. The component survives
    // under a new label; it must not survive as anything but a corpse.
    gpu.apply(RmEvent::Free {
        client: client_of(P_HUP),
        handle: identical_handles(lane.gr.0, lane.ce.0).client_root,
    })
    .expect("the anchor client's root frees");

    assert!(
        gpu.spine.is_condemned(SPLIT_CLIENT),
        "★★ re-labelling the component un-condemned it"
    );
    assert_eq!(
        core_publish(&mut gpu, GPU1, SPLIT_PDB, VA_HELD),
        Err(FwdFault::Condemned {
            anchor: ProcAnchor(SPLIT_CLIENT),
        }),
        "★★ the re-labelled survivor was served again — and the label it answers under \
         must be its OWN anchor, not the freed one"
    );
    assert_eq!(gpu.spine.condemned_len(), 1);
    assert!(
        gpu.procs
            .values()
            .all(|p| !p.clients.contains(&SPLIT_CLIENT)),
        "★ a live proc absorbed the re-labelled corpse"
    );
}

// =================================================================================
// ★★★ §12.39 finding 3 — THE COMPONENT **SPLIT**, the dual of the merge
//
// `GpuError::LateMerge` guards many procs collapsing into one boundary. Nothing guarded
// the mirror: ONE proc matching MANY boundaries, which a legal guest reaches by
// dup-joining two user clients and then freeing the alias. Both halves matched the same
// live proc, so `plan_refresh`'s `survivors` kept it twice, `vanishing` came out EMPTY
// and `sync_proc_to_boundary` ran twice on it — the second call overwriting the first.
// One half silently lost its clients, vases and channels; its `Pdb` left `by_pdb`
// altogether; and its isolate and arena stayed live under the other half.
// =================================================================================

/// The lower-anchored half of the split — the client that keeps the existing `Proc`.
const SPLIT_A: HClient = HClient(0xD8);
/// The higher-anchored half — the client that must get a `Proc` of its OWN.
const SPLIT_B: HClient = HClient(0xD9);
/// [`SPLIT_A`]'s page-directory base.
const SPLIT_A_PDB: Pdb = Pdb(0x5100_0000);
/// [`SPLIT_B`]'s page-directory base.
const SPLIT_B_PDB: Pdb = Pdb(0x5200_0000);
/// [`SPLIT_A`]'s GR vChid.
const SPLIT_A_GR: VChid = VChid(0x180);
/// [`SPLIT_A`]'s CE vChid.
const SPLIT_A_CE: VChid = VChid(0x280);
/// [`SPLIT_B`]'s GR vChid.
const SPLIT_B_GR: VChid = VChid(0x181);
/// [`SPLIT_B`]'s CE vChid.
const SPLIT_B_CE: VChid = VChid(0x281);
/// The handle [`SPLIT_B`] holds [`SPLIT_A`]'s VASpace under while the two are joined.
const JOIN_SPLIT_ALIAS: HObject = HObject(0x7b00_0001);

/// ★★★ **A live component that SPLITS must yield TWO `Proc`s** (`l1_concurrency.md`
/// §12.39, finding 3).
///
/// Two ordinary user processes dup-join (genuine user↔user sharing = one blast radius =
/// one `Proc`, §12.27), both publish and both ring, and then the guest frees the alias
/// that joined them. Freeing a dup is ordinary, legal guest behaviour — **refusing it
/// would hang a legal guest**, which is why this test also asserts the `Free` is
/// accepted — so the component genuinely becomes two, and the runtime has to follow.
///
/// What it must do, and what it did instead:
///
/// | | required | before the fix |
/// |---|---|---|
/// | procs | two, one per half | one — both boundaries matched it |
/// | staging | the departing half's host state queued **once**, through the ordinary staged-death path | nothing staged; `plan.vanishing` was empty |
/// | routing | both halves in `by_pdb` | the overwritten half's `Pdb` dropped out entirely |
/// | host state | the new `Proc` inherits **nothing** | the surviving proc kept the other half's isolate and arena |
///
/// The last row is the one that makes it a security property rather than a bookkeeping
/// one: a later verb naming the lost half found no proc and minted a fresh one — a
/// resurrect into a data plane whose host objects were still owned by somebody else's
/// isolate.
///
/// **Which half keeps the `Proc` is a rule, not an accident**: the first boundary in
/// ascending anchor order claims it, and an anchor is its component's smallest client, so
/// the keeper is the half that still holds the proc's own anchor whenever that client
/// survives. Host state cannot move between isolates (a [`HostHandle`] names the RM
/// client namespace it lives in), so the departing half necessarily re-materialises — its
/// address table starts EMPTY, which makes the loss a loud `AddressFault::Miss` at use
/// rather than a silently zeroed backing.
///
/// Assertions lead with the breach.
#[test]
fn a_live_component_that_splits_yields_two_procs() {
    let _wd = watchdog("component_split", Duration::from_secs(60));
    let (mut gpu, _pids, rec) = mean_gpu();

    // ---- Two ordinary processes, joined by a genuine user↔user share BEFORE either
    // touches its data plane (the early-arm discipline — a late merge is a refusal).
    reinit(&mut gpu, SPLIT_A, SPLIT_A_PDB, SPLIT_A_GR, SPLIT_A_CE, GPU0);
    reinit(&mut gpu, SPLIT_B, SPLIT_B_PDB, SPLIT_B_GR, SPLIT_B_CE, GPU0);
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(SPLIT_A, H_VASPACE),
        dst: NodeKey::new(SPLIT_B, JOIN_SPLIT_ALIAS),
    })
    .expect("a user↔user share is a grouping edge");
    let merged = gpu.spine.by_pdb[&(GPU0, SPLIT_A_PDB)];
    assert_eq!(
        gpu.spine.by_pdb[&(GPU0, SPLIT_B_PDB)],
        merged,
        "precondition: the share made them ONE `Proc`"
    );
    assert_eq!(
        gpu.procs[&merged].clients,
        BTreeSet::from([SPLIT_A, SPLIT_B])
    );

    // ---- Both halves touch the data plane, out of the one isolate and one arena.
    let rung_a = publish_and_ring(&mut gpu, GPU0, SPLIT_A_PDB, SPLIT_A_GR, VA_CTL);
    let rung_b = publish_and_ring(&mut gpu, GPU0, SPLIT_B_PDB, SPLIT_B_GR, VA_RING);
    assert_eq!((rung_a.proc, rung_b.proc), (merged, merged));
    let joined_hosts = host_identities(&gpu.procs[&merged]);
    let merged_iso = gpu.procs[&merged].isolates[&GPU0].id();
    let merged_arena = gpu.procs[&merged].arenas[&GPU0].range.clone();
    let staged_before = gpu.procs[&merged].pending_release_len();

    // ---- ★ THE SPLIT: the guest frees the alias that joined them. Legal, ordinary, and
    // it must be ACCEPTED — a refusal here hangs a guest doing nothing wrong.
    gpu.apply(RmEvent::Free {
        client: SPLIT_B,
        handle: JOIN_SPLIT_ALIAS,
    })
    .expect("★ freeing a dup alias is ordinary guest behaviour and must never be refused");

    // ---- ★★ THE BREACH, asserted first.
    let pid_a = kayfabe_fwd::route_pdb(&gpu.spine, GPU0, SPLIT_A_PDB).unwrap_or_else(|e| {
        panic!(
            "★★ the split DROPPED one half's address plane: {SPLIT_A_PDB:?} no longer \
             routes ({e:?}). Both halves matched the one proc, so the second \
             `sync_proc_to_boundary` overwrote the first and this half's anchor stopped \
             naming any live proc"
        )
    });
    let pid_b = kayfabe_fwd::route_pdb(&gpu.spine, GPU0, SPLIT_B_PDB).unwrap_or_else(|e| {
        panic!("★★ the split DROPPED one half's address plane: {SPLIT_B_PDB:?} ({e:?})")
    });
    assert_ne!(
        pid_a, pid_b,
        "★★ the two halves of a split component still share ONE `Proc` — one isolate \
         (one host RM client namespace), one GPA arena, one host VAS — for two guest \
         processes that no longer share anything"
    );
    assert_eq!(
        (
            gpu.procs[&pid_a].clients.clone(),
            gpu.procs[&pid_b].clients.clone()
        ),
        (BTreeSet::from([SPLIT_A]), BTreeSet::from([SPLIT_B])),
        "★★ each half holds exactly its own client"
    );
    assert_eq!(
        pid_a, merged,
        "the keeper is the half that still holds the proc's own anchor — identity is a \
         rule here, not iteration order"
    );

    // ---- ★ The new `Proc` inherits NOTHING: not the isolate, not the arena, not one
    // host handle. Host objects name the RM client namespace they live in and cannot be
    // moved between them, so inheriting one would be a cross-namespace reach.
    assert_ne!(
        gpu.procs[&pid_b].isolates[&GPU0].id(),
        merged_iso,
        "★★ the departing half kept the keeper's isolate — one host RM client for two \
         unrelated processes"
    );
    let new_arena = gpu.procs[&pid_b].arenas[&GPU0].range.clone();
    assert!(
        new_arena.end <= merged_arena.start || merged_arena.end <= new_arena.start,
        "★★ the departing half kept the keeper's GPA arena: {new_arena:?} vs {merged_arena:?}"
    );
    assert!(
        host_identities(&gpu.procs[&pid_b]).is_empty(),
        "★★ the new `Proc` came up holding host objects it never minted — a resurrect \
         into somebody else's data plane"
    );

    // ---- ★ Conservation: the departing half's host state left through the ORDINARY
    // staged-death path, once. Nothing was freed ad hoc and nothing was dropped.
    assert!(
        gpu.procs[&pid_a].pending_release_len() > staged_before,
        "★★ the departing half's host VAS, backing and channel were dropped on the floor \
         — nothing was queued for release, so nothing can ever free them (§12.33's shape)"
    );
    let l = rec.lock().expect("recorder").ledger();
    assert_eq!(
        (
            l.double_free.as_slice(),
            l.free_of_unknown.as_slice(),
            l.unmap_of_unknown.as_slice()
        ),
        (&[][..], &[][..], &[][..]),
        "★ the split reclaimed nothing twice and reached across no isolate namespace"
    );

    // ---- …and both halves are genuinely SERVED, end to end, in their own lanes.
    for (pid, pdb, gr) in [
        (pid_a, SPLIT_A_PDB, SPLIT_A_GR),
        (pid_b, SPLIT_B_PDB, SPLIT_B_GR),
    ] {
        let rung = publish_and_ring(&mut gpu, GPU0, pdb, gr, VA_HELD);
        assert_eq!(rung.proc, pid, "each half rings in its own proc");
        assert_eq!(
            token_lane(rung.host_token),
            (pid.0 + 1, GPU0.0),
            "★ …on a host token minted in its OWN isolate lane"
        );
    }
    assert!(
        host_identities(&gpu.procs[&pid_b]).is_disjoint(&joined_hosts),
        "★★ the departing half re-materialised onto host objects the joined component \
         had already minted — the new plane must be genuinely new"
    );
    assert!(
        host_identities(&gpu.procs[&pid_b]).is_disjoint(&host_identities(&gpu.procs[&pid_a])),
        "★★ the two halves share a host object"
    );

    // ---- The rest of the world never noticed.
    assert_eq!(gpu.spine.condemned_len(), 0, "a split is not a death");
    core_publish(&mut gpu, GPU0, lane_of(P_WITNESS).pdb, VA_WARM)
        .expect("an unrelated proc is undisturbed by the split");
}

// =================================================================================
// ★★★ §12.39 — THE RECYCLED NAMESPACE, §12.38's surviving sibling
//
// §12.38 closed the *never-declared* squat. This is the one it left open, and it is not
// the weak leftover: in its cheapest shape it costs four events, no host resources and
// nothing observable until it fires. It needs neither a race, nor a condemnation, nor a
// compromised guest kernel — and, eventually, no attacker at all, because RM's own client
// index wraps at 2^20 per driver load with no free list and no epoch
// (`ogkm src/nvidia/src/libraries/resserv/src/rs_server.c:3319-3341`).
//
// Two shapes, two independent fixes, and neither subsumes the other:
//   Shape A — a PARKED fact outlives the free of its own namespace's root (Part A);
//   Shape B — an ORPHANED resource's origin key names a namespace that has since been
//             RE-DECLARED by somebody else (Part B, the `ClientId` identity model).
// =================================================================================

/// The `hClient` VALUE the attacker declares, plants in, frees — and that the victim is
/// later handed. Guessable: RM's generator is a sequential index and the shipped driver
/// honours a caller-supplied `hRoot` verbatim (`rs_server.c:612`, reject guard compiled
/// out under `RS_COMPATABILITY_MODE=1`).
const RECYCLED_CLIENT: HClient = HClient(0xDA);
/// The victim's PDB once it is handed [`RECYCLED_CLIENT`].
const RECYCLED_VICTIM_PDB: Pdb = Pdb(0x5300_0000);
/// The victim's GR vChid.
const RECYCLED_VICTIM_GR: VChid = VChid(0x188);
/// The victim's CE vChid.
const RECYCLED_VICTIM_CE: VChid = VChid(0x288);
/// The handle the attacker's parked alias squats inside the recycled namespace.
const RECYCLED_PLANT: HObject = HObject(0x7c00_0001);
/// The attacker's own object, deliberately allocated LAST, whose arrival is what promotes
/// the parked edge.
const RECYCLED_LATER: HObject = HObject(0x5c00_0f01);

/// ★★★ **Shape A — a parked `DUP_OBJECT` must not outlive the free of its own namespace's
/// client root** (`l1_concurrency.md` §12.39, Part A).
///
/// `RmGraph::free_subtree` prunes the parked tables by membership in `doomed`, which is
/// the set of live HANDLES the free removed. A parked dup's `dst` is by definition *not* a
/// live handle — that is what "parked" means — so it survived the free of its own client
/// root and sat in the table waiting for a handle only a later declaration of the same
/// `hClient` could create.
///
/// The attack, in four events and no host state:
///
/// ```text
/// A: Alloc(Client cV, User)                      legal — `hRoot` is caller-supplied
/// A: Dup { src: (cA, H_LATER), dst: (cV, PLANT) } src unobserved ⇒ PARKS (category 2)
/// A: Free (cV, cV)                                the root dies; the parked edge did NOT
///    …graph footprint from here: one parked edge. No resource, no client in the
///      projection's universe, therefore no phantom `Proc`, no isolate, no arena.
/// V: (an ordinary process handed hClient = cV) declares, builds, publishes
/// A: Alloc (cA, H_LATER)                          promotes the edge — a live ALIAS
///                                                 inside the VICTIM's namespace
/// ⇒ both ends declared `User`, both alive ⇒ a GROUPING edge ⇒ ONE `Proc`.
/// ```
///
/// **§12.38 does not cover this.** Every event above names a namespace that existed at the
/// moment it was issued, so `undeclared_namespace` is satisfied honestly and completely.
/// It is not a bypass of that rule; it is a hole the rule never covered.
///
/// The isolation claim is asserted **before** the mechanism, so a revert reports the
/// breach rather than "a parked edge was left behind".
#[test]
fn a_recycled_namespace_cannot_squat_a_later_process_into_the_attackers_proc() {
    let _wd = watchdog("recycled_shape_a", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let (attacker, lane) = (pids[P_WITNESS], lane_of(P_WITNESS));

    // The attacker is an ordinary, LIVE, publishing process — nothing is condemned.
    core_publish(&mut gpu, GPU0, lane.pdb, VA_WARM).expect("the attacker publishes while alive");
    assert_eq!(gpu.spine.condemned_len(), 0, "nothing is condemned here");
    let attacker_iso = gpu.procs[&attacker].isolates[&GPU0].id();
    let attacker_arena = gpu.procs[&attacker].arenas[&GPU0].range.clone();
    let attacker_hosts = host_identities(&gpu.procs[&attacker]);

    // ---- ★ The plant: declare the namespace, park an edge in it, free the root.
    gpu.apply(kayfabe_tests::client_root(RECYCLED_CLIENT))
        .expect("declaring a namespace of one's own is legal");
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(client_of(P_WITNESS), RECYCLED_LATER),
        dst: NodeKey::new(RECYCLED_CLIENT, RECYCLED_PLANT),
    })
    .expect("a dup whose SOURCE OBJECT is unobserved parks — category 2, and it must");
    gpu.apply(RmEvent::Free {
        client: RECYCLED_CLIENT,
        handle: HObject(RECYCLED_CLIENT.0),
    })
    .expect("the attacker frees the namespace it will hand back");

    // ---- The victim is handed that `hClient` and comes up as an ordinary process.
    reinit(
        &mut gpu,
        RECYCLED_CLIENT,
        RECYCLED_VICTIM_PDB,
        RECYCLED_VICTIM_GR,
        RECYCLED_VICTIM_CE,
        GPU0,
    );
    // ---- …and the attacker fires: it allocates the source object at last.
    gpu.apply(RmEvent::Alloc {
        client: client_of(P_WITNESS),
        parent: H_DEVICE,
        handle: RECYCLED_LATER,
        class: mock_classes::VASPACE,
        facts: AllocFacts::default(),
    })
    .expect("the attacker's own alloc applies");

    // ---- ★★ THE BREACH.
    let victim = gpu
        .spine
        .by_pdb
        .get(&(GPU0, RECYCLED_VICTIM_PDB))
        .copied()
        .expect("the victim's own address plane materialized");
    assert_ne!(
        victim, attacker,
        "★★ the victim was merged into the ATTACKER's `Proc` by an edge planted in a \
         namespace the attacker had already FREED — one isolate, one GPA arena, one host \
         VAS, i.e. #14 un-fixed for a pair the attacker chose"
    );
    assert_eq!(
        gpu.procs[&victim].clients,
        BTreeSet::from([RECYCLED_CLIENT]),
        "★★ the victim's component must hold the victim's client and nothing else"
    );
    assert_ne!(
        gpu.procs[&victim].isolates[&GPU0].id(),
        attacker_iso,
        "★★ the two share one isolate — one host RM client namespace"
    );
    let victim_arena = gpu.procs[&victim].arenas[&GPU0].range.clone();
    assert!(
        victim_arena.end <= attacker_arena.start || attacker_arena.end <= victim_arena.start,
        "★★ the two share one GPA arena: {victim_arena:?} vs {attacker_arena:?}"
    );

    // …and it is genuinely SERVED, end to end, out of its own isolate lane.
    let rung = publish_and_ring(
        &mut gpu,
        GPU0,
        RECYCLED_VICTIM_PDB,
        RECYCLED_VICTIM_GR,
        VA_CTL,
    );
    assert_eq!(rung.proc, victim, "the victim's ring routed to the victim");
    assert_eq!(
        token_lane(rung.host_token),
        (victim.0 + 1, GPU0.0),
        "★ the victim rang a host token minted in its OWN isolate lane"
    );
    assert!(
        host_identities(&gpu.procs[&victim]).is_disjoint(&attacker_hosts),
        "★★ the victim and the attacker share a host object"
    );

    // ---- ★ The mechanism, last: the parked edge died with its namespace's root, so the
    // attacker's alloc promoted nothing.
    assert!(
        !gpu.spine
            .rmgraph
            .dups()
            .any(|(d, _)| d == NodeKey::new(RECYCLED_CLIENT, RECYCLED_PLANT)),
        "★ the parked edge outlived the free of its own namespace's client root"
    );
    assert!(
        gpu.spine
            .rmgraph
            .origin_of(NodeKey::new(RECYCLED_CLIENT, RECYCLED_PLANT))
            .is_none(),
        "★ …and it must not have resolved into a live alias inside the victim's namespace"
    );

    // The attacker earned nothing and lost nothing: its own proc still works.
    core_publish(&mut gpu, GPU0, lane.pdb, VA_HELD).expect("the attacker keeps working");
}

/// The attacker's own client in the Shape-B script (lower than [`SHAPE_B_RECYCLED`], so
/// the attacker's half is the one that keeps the joined `Proc` when they split).
const SHAPE_B_ATTACKER: HClient = HClient(0xDC);
/// The namespace the attacker allocates a VASpace in and then frees — the one whose
/// `hClient` the victim is later handed.
const SHAPE_B_RECYCLED: HClient = HClient(0xDD);
/// The attacker's PDB.
const SHAPE_B_ATT_PDB: Pdb = Pdb(0x5400_0000);
/// The PDB of the VASpace the attacker orphans — the previous tenant's address plane.
const SHAPE_B_ORPHAN_PDB: Pdb = Pdb(0x5500_0000);
/// The victim's own PDB once it is handed [`SHAPE_B_RECYCLED`].
const SHAPE_B_VICTIM_PDB: Pdb = Pdb(0x5600_0000);
/// The attacker's GR/CE vChids.
const SHAPE_B_ATT_GR: VChid = VChid(0x18a);
/// See [`SHAPE_B_ATT_GR`].
const SHAPE_B_ATT_CE: VChid = VChid(0x28a);
/// The victim's GR/CE vChids.
const SHAPE_B_VICTIM_GR: VChid = VChid(0x18b);
/// See [`SHAPE_B_VICTIM_GR`].
const SHAPE_B_VICTIM_CE: VChid = VChid(0x28b);
/// The orphaned namespace's device handle.
const SHAPE_B_DEV: HObject = HObject(0x6d00_0001);
/// The orphaned namespace's VASpace handle.
const SHAPE_B_VAS: HObject = HObject(0x6d00_0010);
/// The handle the attacker keeps the orphaned VASpace alive under.
const SHAPE_B_ALIAS: HObject = HObject(0x7d00_0001);

/// ★★★ **Shape B — a recycled namespace must not inherit the previous tenant's address
/// plane** (`l1_concurrency.md` §12.39, Part B).
///
/// A resource survives its origin handle's free while any foreign alias references it
/// (faithful RM refcounting, `ogkm .../mem_mgr/mem.c:986-1039` — correct, and deliberately
/// unchanged), but its `RmNode::key` still carries the `HClient` **VALUE** of the
/// namespace that allocated it. An `hClient` is recyclable *by NVIDIA's design*: the
/// shipped driver honours a caller-supplied `hRoot` (`rs_server.c:612`, reject guard
/// compiled out at `:613-616` under `RS_COMPATABILITY_MODE=1`), the RM-chosen values come
/// from an index that wraps at 2^20 with no free list (`:3319-3341`), and RM carries no
/// epoch anywhere that could tell two lifetimes apart. So the stale value used to
/// re-attach the orphan to **whoever holds the number now** — the victim — putting an
/// attacker-owned VASpace inside the victim's component and making the surviving dup edge
/// a grouping edge.
///
/// **The fix may not be a refusal.** Because RM recycles by design, refusing to re-declare
/// a namespace that still holds live resources would refuse a stream the guest's own RM
/// emits — so this test also asserts the victim's `Alloc(Client)` is *accepted* and that
/// the victim is genuinely served. The identity is minted by us instead: a `ClientId`, the
/// client root's never-reused `ResId`, recorded on every resource at its alloc.
///
/// Composed with §12.39 finding 3 on purpose: freeing the orphaned namespace's root while
/// the attacker's alias lives **splits** the joined component, so this script exercises
/// the split path and the identity model in one run.
#[test]
fn a_recycled_namespace_cannot_inherit_the_previous_tenants_address_plane() {
    let _wd = watchdog("recycled_shape_b", Duration::from_secs(60));
    let (mut gpu, _pids, rec) = mean_gpu();

    // ---- The attacker: an ordinary compute process, plus a SECOND namespace of its own
    // holding a VASpace with a page directory bound.
    reinit(
        &mut gpu,
        SHAPE_B_ATTACKER,
        SHAPE_B_ATT_PDB,
        SHAPE_B_ATT_GR,
        SHAPE_B_ATT_CE,
        GPU0,
    );
    for ev in [
        kayfabe_tests::client_root(SHAPE_B_RECYCLED),
        RmEvent::Alloc {
            client: SHAPE_B_RECYCLED,
            parent: HObject(SHAPE_B_RECYCLED.0),
            handle: SHAPE_B_DEV,
            class: mock_classes::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: SHAPE_B_RECYCLED,
            parent: SHAPE_B_DEV,
            handle: SHAPE_B_VAS,
            class: mock_classes::VASPACE,
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: SHAPE_B_RECYCLED,
            vaspace: SHAPE_B_VAS,
            pdb: SHAPE_B_ORPHAN_PDB,
        },
        // The alias that will keep the VASpace alive past its namespace's death.
        RmEvent::Dup {
            src: NodeKey::new(SHAPE_B_RECYCLED, SHAPE_B_VAS),
            dst: NodeKey::new(SHAPE_B_ATTACKER, SHAPE_B_ALIAS),
        },
    ] {
        gpu.apply(ev).expect("the attacker's own script applies");
    }
    let attacker = gpu.spine.by_pdb[&(GPU0, SHAPE_B_ATT_PDB)];
    assert_eq!(
        gpu.spine.by_pdb[&(GPU0, SHAPE_B_ORPHAN_PDB)],
        attacker,
        "precondition: a user↔user share makes the two namespaces ONE `Proc`"
    );
    // The attacker publishes into the plane it is about to orphan — so the state a
    // recycled namespace could inherit is real host memory, not an empty shell.
    core_publish(&mut gpu, GPU0, SHAPE_B_ORPHAN_PDB, VA_WARM).expect("the attacker publishes");
    let attacker_hosts = host_identities(&gpu.procs[&attacker]);

    // ---- The attacker frees the namespace's root. The VASpace survives on its alias
    // (RM's refcount), so its component survives with it — a `Proc` of its own, since the
    // edge is no longer a grouping edge once the source namespace has no live root.
    gpu.apply(RmEvent::Free {
        client: SHAPE_B_RECYCLED,
        handle: HObject(SHAPE_B_RECYCLED.0),
    })
    .expect("the attacker frees the root it will hand back");
    let orphan = kayfabe_fwd::route_pdb(&gpu.spine, GPU0, SHAPE_B_ORPHAN_PDB)
        .expect("the orphaned VASpace is still alive and still routes — RM's refcount");
    assert_ne!(orphan, attacker, "freeing the root split the component");
    let orphan_iso = gpu.procs[&orphan].isolates[&GPU0].id();
    let orphan_arena = gpu.procs[&orphan].arenas[&GPU0].range.clone();

    // ---- ★ The victim is handed that `hClient`. Its own first event is its client root,
    // and it MUST be accepted: RM recycles by design, so refusing hangs a legal guest.
    reinit(
        &mut gpu,
        SHAPE_B_RECYCLED,
        SHAPE_B_VICTIM_PDB,
        SHAPE_B_VICTIM_GR,
        SHAPE_B_VICTIM_CE,
        GPU0,
    );

    // ---- ★★ THE BREACH.
    let victim = gpu
        .spine
        .by_pdb
        .get(&(GPU0, SHAPE_B_VICTIM_PDB))
        .copied()
        .expect("★★ the victim's own address plane was never materialized");
    assert_ne!(
        victim, orphan,
        "★★ the victim INHERITED the previous tenant's `Proc` — its isolate (one host RM \
         client namespace), its GPA arena, its host VAS and its `pending_release` queue — \
         because the two were matched on a recyclable `hClient` VALUE"
    );
    assert_ne!(
        victim, attacker,
        "★★ the victim was merged into the ATTACKER's `Proc` by a dup edge whose `src` \
         names a namespace the attacker had already freed"
    );
    assert_eq!(
        gpu.procs[&victim].clients,
        BTreeSet::from([SHAPE_B_RECYCLED]),
        "★★ the victim's component must hold the victim's client and nothing else"
    );
    assert_eq!(
        kayfabe_fwd::route_pdb(&gpu.spine, GPU0, SHAPE_B_ORPHAN_PDB),
        Err(FwdFault::UnknownPdb {
            gpu: GPU0,
            pdb: SHAPE_B_ORPHAN_PDB
        }),
        "★★ the SUPERSEDED declaration's address plane must belong to nobody — routable \
         means routable to a `Proc`, and there is no longer one it can honestly name. \
         `UnknownPdb`, by name, not an anonymous miss"
    );
    assert_ne!(
        gpu.procs[&victim].isolates[&GPU0].id(),
        orphan_iso,
        "★★ the victim came up in the previous tenant's isolate"
    );
    let victim_arena = gpu.procs[&victim].arenas[&GPU0].range.clone();
    assert!(
        victim_arena.end <= orphan_arena.start || orphan_arena.end <= victim_arena.start,
        "★★ the victim carved into the previous tenant's GPA arena: {victim_arena:?} vs \
         {orphan_arena:?}"
    );
    assert!(
        host_identities(&gpu.procs[&victim]).is_disjoint(&attacker_hosts),
        "★★ the victim came up holding a host object the attacker minted"
    );

    // ---- …and it is genuinely SERVED, end to end, in its own lane.
    let rung = publish_and_ring(
        &mut gpu,
        GPU0,
        SHAPE_B_VICTIM_PDB,
        SHAPE_B_VICTIM_GR,
        VA_CTL,
    );
    assert_eq!(rung.proc, victim);
    assert_eq!(
        token_lane(rung.host_token),
        (victim.0 + 1, GPU0.0),
        "★ the victim rang a host token minted in its OWN isolate lane"
    );

    // ---- The previous tenant's host state left through the ordinary staged path: nothing
    // was freed twice and nothing reached across an isolate namespace.
    let l = rec.lock().expect("recorder").ledger();
    assert_eq!(
        (
            l.double_free.as_slice(),
            l.free_of_unknown.as_slice(),
            l.unmap_of_unknown.as_slice()
        ),
        (&[][..], &[][..], &[][..]),
        "★ the supersession reclaimed nothing twice and reached across no namespace"
    );
    // The attacker is undisturbed — it earned its own outcome and nobody else's.
    core_publish(&mut gpu, GPU0, SHAPE_B_ATT_PDB, VA_HELD).expect("the attacker keeps working");
}

/// The bystander user client that aliases a CONDEMNED component's resource — a
/// *reference* across the condemnation line, never a merge (§12.37 C1), and the thing
/// that keeps the corpse's resource alive after the guest frees its root.
const KEEPALIVE_CLIENT: HClient = HClient(0xDE);
/// [`KEEPALIVE_CLIENT`]'s own PDB.
const KEEPALIVE_PDB: Pdb = Pdb(0x5700_0000);
/// [`KEEPALIVE_CLIENT`]'s GR vChid.
const KEEPALIVE_GR: VChid = VChid(0x18c);
/// [`KEEPALIVE_CLIENT`]'s CE vChid.
const KEEPALIVE_CE: VChid = VChid(0x28c);
/// The handle it holds the condemned VASpace under.
const KEEPALIVE_ALIAS: HObject = HObject(0x7e00_0001);
/// The PDB of the process that is later handed the dead component's `hClient`.
const REBORN_PDB: Pdb = Pdb(0x5800_0000);
/// That process's GR vChid.
const REBORN_GR: VChid = VChid(0x18d);
/// That process's CE vChid.
const REBORN_CE: VChid = VChid(0x28d);

/// ★★★ **THE HANGS-A-LEGAL-GUEST GATE: a recycled namespace is a DIFFERENT component and
/// is NOT condemned** (`l1_concurrency.md` §12.39, and §12.37's C2 one turn further).
///
/// This is the test that fails if anyone ever "fixes" the recycled-namespace vector by
/// refusing to re-declare a namespace that still holds live resources. RM recycles
/// `hClient` values **by design** — a caller-supplied `hRoot` is honoured verbatim
/// (`ogkm src/nvidia/src/libraries/resserv/src/rs_server.c:612`, the reject guard compiled
/// out at `:613-616` under `RS_COMPATABILITY_MODE=1`), the generator wraps at 2^20 per
/// driver load with no free list (`:3319-3341`), and there is no epoch anywhere in RM's
/// own structs to tell two lifetimes apart. Refusing the successor's `Alloc(Client)` would
/// refuse a *victim* for a *predecessor's* state: the bystander-refusal shape §12.37 was
/// written to remove.
///
/// It also closes the hole §12.37's C2 shrink could not reach on its own, and the two
/// halves are asserted in order:
///
/// 1. **the corpse stays dead** — an orphaned resource of a condemned component keeps
///    answering the exact [`FwdFault::Condemned`] even after its own namespace's root is
///    freed, because the entry names a `ClientId` that is never reused;
/// 2. **the successor lives** — the very next process handed that `hClient` value gets its
///    own live `Proc`, its own isolate, its own arena, and is servable end to end.
///
/// Under an `HClient`-keyed condemnation those two are in direct conflict: the orphan
/// keeps the dead value in the projection, so C2's shrink never drops it, so the entry
/// still contains the value the guest hands out next — and the successor is condemned on
/// arrival, silently, on its own first RM event.
#[test]
fn a_recycled_namespace_is_a_different_component_and_is_not_condemned() {
    let _wd = watchdog("recycled_not_condemned", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    let (doomed, lane) = (pids[P_HUP], lane_of(P_HUP));
    let condemned = FwdFault::Condemned {
        anchor: ProcAnchor(client_of(P_HUP)),
    };

    // A bystander that will keep one of the doomed component's resources alive.
    reinit(
        &mut gpu,
        KEEPALIVE_CLIENT,
        KEEPALIVE_PDB,
        KEEPALIVE_GR,
        KEEPALIVE_CE,
        GPU1,
    );
    core_publish(&mut gpu, GPU1, lane.pdb, VA_WARM).expect("the doomed proc publishes while alive");

    // ---- The worker dies out of band: the component is condemned (§12.13).
    assert!(core_retire_out_of_band(&mut gpu, doomed));
    assert!(gpu.spine.is_condemned(client_of(P_HUP)));

    // The bystander aliases one of the corpse's objects. A dup across the condemnation
    // line is a REFERENCE, never a merge — so the bystander stays live…
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(client_of(P_HUP), H_VASPACE),
        dst: NodeKey::new(KEEPALIVE_CLIENT, KEEPALIVE_ALIAS),
    })
    .expect("a cross-line alias applies");
    assert!(
        !gpu.spine.is_condemned(KEEPALIVE_CLIENT),
        "a live client that aliases a corpse does not inherit its fatality (§12.37 C1)"
    );

    // ---- The guest frees the dead component's client root. Its VASpace survives on the
    // bystander's alias (RM's refcount), so the corpse's namespace is still in the
    // projection — which is exactly what used to keep its freed `hClient` VALUE poisoned.
    gpu.apply(RmEvent::Free {
        client: client_of(P_HUP),
        handle: identical_handles(lane.gr.0, lane.ce.0).client_root,
    })
    .expect("the client-root free applies");

    // (1) The corpse stays dead, by name.
    assert_eq!(
        core_publish(&mut gpu, GPU1, lane.pdb, VA_HELD),
        Err(condemned),
        "★★ an orphaned resource of a condemned component stopped answering `Condemned` \
         once its own namespace's root was freed — condemnation must not depend on a \
         handle VALUE still being 'known'"
    );
    assert!(gpu.spine.is_condemned(client_of(P_HUP)));

    // ---- ★★ (2) The successor: an ordinary new process, handed the dead component's
    // `hClient`. Its own first event is its own client root, and it MUST be accepted.
    reinit(
        &mut gpu,
        client_of(P_HUP),
        REBORN_PDB,
        REBORN_GR,
        REBORN_CE,
        GPU1,
    );
    assert!(
        !gpu.spine.is_condemned(client_of(P_HUP)),
        "★★ a process was condemned on arrival because its `hClient` landed on a value a \
         dead component still held — a bystander refusal, and the guest cannot recover \
         from it because it did nothing"
    );
    let reborn = gpu.spine.by_pdb.get(&(GPU1, REBORN_PDB)).copied().expect(
        "★★ the successor's own address plane was never materialized. Either it was \
             CONDEMNED on the apply that declared it (a bystander death earned by a \
             recycled `hClient`), or its VASpace resolved the SUPERSEDED declaration's \
             `Pdb` because an origin key is not unique among live resources — and nobody \
             was told either way",
    );
    assert_eq!(
        gpu.procs[&reborn].clients,
        BTreeSet::from([client_of(P_HUP)]),
        "★★ the successor's component holds its own client and nothing else"
    );
    assert!(
        gpu.procs[&reborn].isolates.contains_key(&GPU1)
            && gpu.procs[&reborn].arenas.contains_key(&GPU1),
        "★★ the successor got no data plane of its own — a live `Proc` needs an isolate \
         and an arena, or it is a husk"
    );

    // …and it is genuinely SERVED, end to end, in its own lane.
    let rung = publish_and_ring(&mut gpu, GPU1, REBORN_PDB, REBORN_GR, VA_CTL);
    assert_eq!(rung.proc, reborn);
    assert_eq!(
        token_lane(rung.host_token),
        (reborn.0 + 1, GPU1.0),
        "★ the successor rang a host token minted in its OWN isolate lane"
    );

    // ---- The bystander was never disturbed by any of it.
    assert!(!gpu.spine.is_condemned(KEEPALIVE_CLIENT));
    core_publish(&mut gpu, GPU1, KEEPALIVE_PDB, VA_HELD).expect("the bystander keeps working");
}
