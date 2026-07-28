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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant as WallInstant};

use kayfabe_arch::GspReg;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_completion::OsEventRef;
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, GpuError};
use kayfabe_core::project::{NO_CONDEMNED, project};
use kayfabe_core::reactor::SourceKind;
use kayfabe_core::rmgraph::ClientKey;
use kayfabe_core::rmgraph::{AllocFacts, NodeKey, RmEvent, RmGraphError, RmNode};
use kayfabe_core::{ChanId, ProcAnchor, ProcId};
use kayfabe_fwd::{ControlRoute, DoorbellOutcome, FwdFault, Published, Stale};
use kayfabe_gsp::{BootPhase, GspFault, QueueState, Transition};
use kayfabe_isolate::{CancelReason, HostHandle, IsolateId, RmError, WorkerId};
use kayfabe_mmu::AddressFault;
use kayfabe_mocks::{
    HoldSpec, MockArch, MockIsolateFactory, MockPushbuffer, RmVerb, SharedRecorder, VerbHold,
    VerbKind, VmmErrorKind, mock_classes, mock_ctrl,
};
use kayfabe_rmrpc::GraphPolicy;
use kayfabe_rt::device::{LockMode, SharedDevice, SignalOutcome};
use kayfabe_rt::executor::{Effect, Executor};
use kayfabe_rt::inbox::{CoreEvent, inbox};
use kayfabe_rt::lock;
use kayfabe_tests::gspworld::{GspWorld, MODEL_A, MODEL_B, P580, P610, REAL_QUEUE_SIZE, RingId};
use kayfabe_tests::rpcwire::{self as w, RpcScript, fn_id};
use kayfabe_tests::{
    Guarded, ResidueClaim, Scenario, SharedVmm, gpfifo_ring, identical_handles, reachable_maps,
    reachable_objects, script_ring,
};
use kayfabe_trace::FaultTag;
use kayfabe_util::Instant;
use kayfabe_vmm::Vmm as _;
use kayfabe_vmm::{GuestRamMap, RamRegionId, RamSpan, RegionKind};

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
            IsolateId::new(pid.0, gpu),
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
/// latch (§8.4). ★ N3: an [`IsolateId`] IS the `(proc, GPU)` pair, so `(pid, gpu)`
/// names the isolate exactly — there is no second, separately-supplied GPU that could
/// disagree with it.
fn hold(
    rec: &SharedRecorder,
    pid: ProcId,
    gpu: GpuId,
    worker: u32,
    verb: VerbKind,
) -> Arc<VerbHold> {
    rec.lock().expect("recorder").hold(HoldSpec::exact(
        IsolateId::new(pid.0, gpu),
        WorkerId(worker),
        verb,
    ))
}

/// How many host `free` verbs `(pid, gpu)`'s isolate has issued (the mid-chain leak
/// check). ★ N3: per ISOLATE, so a proc that spans two GPUs is counted per target
/// rather than summed into one number that names neither.
fn count_frees(rec: &SharedRecorder, pid: ProcId, gpu: GpuId) -> usize {
    rec.lock()
        .expect("recorder")
        .verbs_of(IsolateId::new(pid.0, gpu))
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
    /// ★★ M2-e canary (d): the requester ABANDONED by §7.5's escape, released inside the
    /// window by the worker HUP rather than by the latch at phase 7.
    canary_wedge: Result<Published, FwdFault>,
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

/// ★★★ §12.42 — **THE TWO-GENERATION NAMESPACE RECYCLE, INSIDE THE COMPOSED WINDOW.**
///
/// The focused proof is
/// [`a_superseded_declarations_kernel_held_memory_is_never_freed_by_its_successor`]; this
/// is the same script run while five host verbs are parked, six workload threads are hot
/// on both GPUs, two procs are being killed out of band and T0's churn is allocating and
/// freeing underneath — because the defect it guards was only ever visible in a composed
/// sequence, and the isolated case tests what we thought of.
///
/// Returns `(gen1, gen2)`: the superseded declaration's `Proc` (kept alive by the guest
/// kernel's `DUP_OBJECT`, exactly as §12.33 requires) and the successor's.
fn generation_recycle(
    dev: &SharedDevice,
    rec: &SharedRecorder,
    mode: LockMode,
) -> (ProcId, ProcId) {
    let handles = identical_handles(0, 0);

    // (1) generation 1 on GPU0: a whole CUDA-context-shaped process, published and rung,
    //     so what the recycle can destroy is REAL host state and not an empty shell.
    let mut s = Scenario::new();
    s.compute_process_on_gpu(
        GEN_CLIENT,
        GEN1_PDB,
        identical_handles(GEN1_GR.0, GEN1_CE.0),
        None,
    );
    // (2) the guest kernel's UVM session takes a reference to its VASpace — the measured
    //     shape (one session per module load, every CUDA process dups into it).
    s.uvm_dup(
        GEN_UVM,
        HObject(GEN_UVM.0),
        GEN_UVM_DEV,
        GEN_UVM_VAS,
        GEN_UVM_PDB,
        GEN1_ALIAS,
        NodeKey::new(GEN_CLIENT, handles.vaspace),
    );
    for ev in s.events {
        dev.apply(ev)
            .expect("({mode:?}) the recycle's generation 1 applies");
    }
    dev.publish_backing(GPU0, GEN1_PDB, GpuVa(VA_GEN), 0x1000)
        .expect("({mode:?}) generation 1 publishes");
    let gen1 = dev
        .doorbell(GPU0, MockArch::token_for(GEN1_GR), &[GpuVa(VA_GEN)])
        .expect("({mode:?}) generation 1 rings")
        .proc;

    // (3) the process exits. Its `Proc` must SURVIVE — RM refcounts the VASpace for the
    //     session (§12.33), so retiring it here would free host memory a live kernel
    //     client is still reading.
    dev.apply(RmEvent::Free {
        client: GEN_CLIENT,
        handle: handles.client_root,
    })
    .expect("({mode:?}) generation 1's client root frees");
    let gen1_named = dev
        .with_proc(gen1, reachable_objects)
        .expect("({mode:?}) §12.33: the owning `Proc` survives its owner");
    assert!(
        gen1_named.len() >= 2,
        "({mode:?}) the recycle's baseline must be real host state, got {gen1_named:?}"
    );

    // (4) ★★ THE RECYCLE, on the OTHER GPU. Accepted — RM recycles `hClient` values by
    //     design and refusing would hang a legal guest.
    let mut s = Scenario::new();
    s.compute_process_on_gpu(
        GEN_CLIENT,
        GEN2_PDB,
        identical_handles(GEN2_GR.0, GEN2_CE.0),
        Some(GPU1.0),
    );
    for ev in s.events {
        dev.apply(ev)
            .expect("({mode:?}) ★ re-declaring a recycled namespace is LEGAL");
    }
    dev.publish_backing(GPU1, GEN2_PDB, GpuVa(VA_GEN), 0x1000)
        .expect("({mode:?}) generation 2 publishes");
    let gen2 = dev
        .doorbell(GPU1, MockArch::token_for(GEN2_GR), &[GpuVa(VA_GEN)])
        .expect("({mode:?}) generation 2 rings")
        .proc;

    // (5) ★★★ THE BREACH, inside the window.
    assert_eq!(
        dev.with_proc(gen1, reachable_objects).unwrap_or_default(),
        gen1_named,
        "★★ ({mode:?}) HOST MEMORY RM SAYS IS LIVE WAS TAKEN AWAY — the UVM session still \
         holds a `DUP_OBJECT` of generation 1's VASpace \
         (`ogkm .../mem_mgr/mem.c:1027-1031`) and an unrelated later tenant of the same \
         `hClient` VALUE left its host VAS and its published backing nameable by nothing"
    );
    assert!(
        outstanding_on(rec, gen1, GPU0).is_superset(&gen1_named),
        "★★ ({mode:?}) …and a host `Free` verb was issued for one of them"
    );
    assert_ne!(
        gen2, gen1,
        "★★ ({mode:?}) the successor INHERITED its predecessor's `Proc` — one isolate, one \
         GPA arena, one host VAS — because the two were matched on a recyclable value"
    );
    // …and the predecessor's plane is still USABLE, not merely present in a map.
    dev.publish_backing(GPU0, GEN1_PDB, GpuVa(VA_GEN + 0x1_0000), 0x1000)
        .expect("★★ the kernel-referenced plane must still accept host work");
    (gen1, gen2)
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
    device.declare_residue(
        ResidueClaim::on(
            IsolateId::new(pids[P_TEARDOWN].0, gpu_of(P_TEARDOWN)),
            "a canary proc killed out of band (`retire_proc`): its isolate is stopped, so \
             its host VAS + backing + channel are the §7.0 namespace-death residue §12.32 \
             measured at 6 objects / 2 mappings across the pair. ★ M2-e: its HELD verb was \
             `alloc_sysmem`, so the retire's cancel caught it before it had allocated \
             anything — the residue is the warm-up's estate and nothing more",
        )
        .objects(VerbKind::AllocVaSpace, 1)
        .objects(VerbKind::AllocSysmem, 1)
        .objects(VerbKind::AllocChannel, 1)
        .maps(1),
    );
    device.declare_residue(
        ResidueClaim::on(
            IsolateId::new(pids[P_HUP].0, gpu_of(P_HUP)),
            "the same §7.0 namespace-death residue as the teardown canary, PLUS one — ★ \
             M2-e: this proc's worker is WEDGED (§7.5), and a wedged worker cannot run its \
             own unwind, so the host memory object its chain had already allocated is \
             staged rather than freed and dies with the session too",
        )
        .objects(VerbKind::AllocVaSpace, 1)
        .objects(VerbKind::AllocSysmem, 2)
        .objects(VerbKind::AllocChannel, 1)
        .maps(1),
    );
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
    // ★★★ §12.42 — filled by phase 5 (g), inside the window.
    let mut recycled: Option<(ProcId, ProcId)> = None;
    // ★★ M2-e — filled by phase 5 (d): what the ABANDONED requester came back with.
    let mut canary_wedge: Option<Result<Published, FwdFault>> = None;

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
            // ★★ M2-e — the WEDGE canary (§7.5), composed into the same window as
            // everything else. `MapGpuVa`, not `AllocSysmem`: the chain must be parked
            // MID-chain, with a host memory object already minted, because a wedged
            // worker cannot run its own unwind (G4) and the whole question is what
            // happens to what it had already allocated. This is the verb phase 5 (d)'s
            // worker HUP abandons.
            latches.arm(&rec, pids[P_HUP], GPU1, 0, VerbKind::MapGpuVa);
            let t_teardown = sc.spawn(move || {
                dev.publish_backing(GPU0, lane_of(P_TEARDOWN).pdb, GpuVa(VA_HELD), 0x1000)
            });
            let t_chanfree = sc.spawn(move || {
                dev.doorbell(GPU1, MockArch::token_for(lane_of(P_CHANFREE).gr), &[])
            });
            let t_reroute = sc
                .spawn(move || dev.doorbell(GPU0, MockArch::token_for(lane_of(P_REROUTE).gr), &[]));
            let t_wedge = sc.spawn(move || {
                dev.publish_backing(GPU1, lane_of(P_HUP).pdb, GpuVa(VA_HELD), 0x1000)
            });

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
            // ★★ M2-e — and the thread parked in that dead worker's verb is RELEASED,
            // inside the same window, with every other verb still parked. Before this,
            // `worker_died` marked the slot dead and NOTHING anywhere told the requester:
            // it would have sat in the mock's condvar until phase 7 released the latch,
            // and on a real socket only the HUP itself would have ended it. §7.5's
            // abandon closes that, and it is safe here for the one reason it is ever
            // safe — the slot is already dead and the component is condemned by the same
            // act. That this join RETURNS AT ALL, with five verbs still parked, is the
            // assertion; its exact value is checked after the scope.
            canary_wedge = Some(t_wedge.join().expect("the wedge canary's thread joins"));
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

            // (g) ★★★ §12.42 (N1/N2): the guest kernel hands one `hClient` value to two
            // successive processes while the UVM session holds a `DUP_OBJECT` of the
            // first one's VASpace. Composed here on purpose — the corruption it guards
            // (host memory RM says is live, freed by the value's next tenant) is a
            // whole-device lifecycle fact, and the previous round measured that NONE of
            // the 359 tests then in the suite caught it.
            recycled = Some(generation_recycle(dev, &rec, mode));

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
    let frees_before = count_frees(&rec, pids[P_WITNESS], gpu_of(P_WITNESS));
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
            count_frees(&rec, pids[P_WITNESS], gpu_of(P_WITNESS)) - frees_before,
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
    let recycled = recycled.expect("phase 5 (g) ran");
    sweep_conservation(&mut gpu, &pids, recycled, mode);

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
        canary_wedge: canary_wedge.expect("phase 5 (d) ran"),
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
        Owner::Condemned(ProcAnchor(ClientKey::first(client_of(i))))
    } else {
        Owner::Live(pids[i])
    }
}

/// The facts that must hold globally, no matter how the threads interleaved. Each is a
/// property the design *claims*; asserting them here is what makes the claim falsifiable
/// by a harsh run instead of by inspection.
fn sweep_conservation(gpu: &mut Gpu, pids: &[ProcId], recycled: (ProcId, ProcId), mode: LockMode) {
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
                .all(|p| !p.client_values().contains(&client_of(i))),
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
        6,
        "({mode:?}) exactly the four untouched procs plus the recycle's TWO generations \
         are live"
    );
    // ★★★ §12.42 — the recycle's end-of-run facts, after everything else the window did.
    let (gen1, gen2) = recycled;
    assert_eq!(
        (
            kayfabe_fwd::route_pdb(&gpu.spine, GPU0, GEN1_PDB),
            kayfabe_fwd::route_pdb(&gpu.spine, GPU1, GEN2_PDB),
        ),
        (Ok(gen1), Ok(gen2)),
        "({mode:?}) ★★ two generations of one `hClient`, two address planes, two `Proc`s \
         — and the superseded one is the one RM keeps alive on the UVM session's alias"
    );
    assert_eq!(
        (
            gpu.procs[&gen1].clients.clone(),
            gpu.procs[&gen2].clients.clone()
        ),
        (
            BTreeSet::from([ClientKey::first(GEN_CLIENT)]),
            BTreeSet::from([ClientKey {
                client: GEN_CLIENT,
                incarnation: 1
            }])
        ),
        "({mode:?}) ★★ the two lifetimes of one `hClient` VALUE must be two components"
    );
    assert!(
        gpu.system.client_values().contains(&GEN_UVM),
        "({mode:?}) the UVM session stayed the SYSTEM component's throughout"
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
/// ★ N3 — **per `(Proc, GpuId)`**, which is what an [`IsolateId`] now is. The ledger
/// keys on the isolate that ISSUED each verb; the core side is filtered by the isolate
/// each handle records itself as MINTED in ([`HostHandle::isolate`]). The set equality
/// below is therefore also a statement that those two agree — an object attributed to
/// the wrong one of a proc's two isolates used to balance perfectly, because both went
/// into one bucket.
fn census(gpu: &Gpu, rec: &SharedRecorder) -> Census {
    let ledger = rec.lock().expect("recorder").ledger();
    let mut c = Census::default();
    let mut isolates: BTreeSet<IsolateId> = ledger.leaked.keys().copied().collect();
    isolates.extend(ledger.leaked_maps.keys().copied());
    for iso in isolates {
        let outstanding = ledger.leaked_on(iso);
        let outstanding_maps = ledger.leaked_maps.get(&iso).cloned().unwrap_or_default();
        let pid = ProcId(iso.proc());
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
        let reachable: BTreeSet<HostHandle> = reachable_objects(proc)
            .into_iter()
            .filter(|h| h.isolate() == iso)
            .collect();
        let reachable_m: BTreeSet<(HostHandle, u64)> = reachable_maps(proc)
            .into_iter()
            .filter(|(vas, _)| vas.isolate() == iso)
            .collect();
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
        // ★★ CHANGED BY M2-e, and the change is the T2 behaviour, not a weakening.
        // `Proc::retire` now latches a break signal for every verb the proc still has in
        // flight (`l1_os_shell.md` §7.6 T2, §15 amendment 4), so this verb no longer runs
        // to completion against a dead proc and no longer reaches its commit — it comes
        // back INTERRUPTED, and the truth about it is "we cancelled it", not "the world
        // moved". §7.3's table says exactly that: cancellation is the third staleness
        // shape, non-retryable and orphan-carrying.
        //
        // ★ The `Stale` arm it used to prove is NOT abandoned: it is the case where the
        // cancel is delivered and the host wait does not break — RM's own answer most of
        // the time (§7.9) — and it is pinned by
        // `cancellation.rs::a_cancel_the_host_wait_never_breaks_still_refuses_as_staleness`.
        assert_eq!(
            r.canary_teardown,
            Err(FwdFault::Cancelled {
                proc: pids[P_TEARDOWN],
                reason: CancelReason::ProcExit
            }),
            "({name}) R5 canary (a): a verb in flight across its proc's retire must come \
             back CANCELLED, naming ProcExit — not as an incidental RM error, and not as \
             a bare staleness that hides who killed it"
        );
        assert_eq!(
            r.canary_wedge,
            Err(FwdFault::Wedged {
                proc: pids[P_HUP],
                gpu: GPU1,
                worker: WorkerId(0)
            }),
            "({name}) ★★ M2-e canary (d): the requester parked in a worker that DIED must \
             be released with the truth — WEDGED, naming the exact slot. Never \
             `Cancelled` (which would claim the host honoured a break signal it never \
             saw), never an anonymous RM error, and above all never left parked: before \
             §7.5's abandon, nothing in the design released it at all"
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
            (7, 2),
            "({name}) the §7.0 namespace-death residue is the script's own: the two \
             out-of-band-retired procs' host state, disposed of by their isolate \
             sessions' death and by nothing else. ★★ M2-e moved this from 6 to 7, and the \
             +1 is EXACTLY the wedge canary's intermediate: a wedged worker cannot run \
             its own unwind (G4), so the host memory object its chain had allocated is \
             staged and then dies with the session. That number growing by anything OTHER \
             than one is the regression this line catches"
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
        anchor: ProcAnchor(ClientKey::first(client_of(P_HUP))),
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
            .all(|p| !p.client_values().contains(&client_of(P_HUP))),
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
        anchor: ProcAnchor(ClientKey::first(client_of(P_HUP))),
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
                .all(|p| !p.client_values().contains(&client_of(P_HUP))),
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
    assert!(gpu.procs[&pid].client_values().contains(&fresh));
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
            IsolateId::new(victim.0, gpu_of(P_HUP)),
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
                .all(|p| !p.client_values().contains(&client_of(P_HUP))),
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
        !gpu.procs[&new_pid]
            .client_values()
            .contains(&client_of(P_HUP)),
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
    assert!(gpu.procs[&merged].client_values().contains(&uvm));
    core_publish(&mut gpu, GPU0, lane_of(P_WITNESS).pdb, VA_CTL)
        .expect("★ the merged proc keeps serving");
}

/// ★★★ **§12.37 carry-forward: when ONE boundary spans TWO condemned entries, the two
/// entries are the SAME entry from then on.**
///
/// Condemnation is carried forward per *component*, and components are not stable: a dup
/// can join two of them and a free can split one. The carry-forward therefore has to be
/// a union-find over the carried entries, not a per-boundary copy — and the union is the
/// half nothing asserted. Every condemnation test in this file drives ONE condemned
/// component, where the union can never fire.
///
/// **Why the union is load-bearing and not bookkeeping.** An entry that fails to merge
/// does not merely count twice: the carried set is rebuilt keyed by union-find *root*, so
/// a component whose root was never merged is **not carried forward at all** — its
/// clients drop straight out of the condemned set. The observable is therefore a
/// condemned client silently coming back to life, which is §12.13 rule 3 fired by
/// accident: condemnation must end when the *guest* frees the roots, never because two
/// components happened to touch.
///
/// The shape is the smallest one where the union can be seen at all — a boundary that
/// spans two entries **and** a second boundary that touches only the higher-indexed of
/// them, which is what makes "merged" and "not merged" produce different *counts*:
///
/// 1. `P_PEER` + `P_CHANFREE` share a VASpace → one component; condemn it → entry **B**.
/// 2. Condemn `P_WITNESS` alone → entry **A** (lower client id ⇒ lower index).
/// 3. Free the shared alias → `P_PEER` and `P_CHANFREE` are two components again, both
///    still carried by entry **B**.
/// 4. A new dup joins `P_WITNESS` and `P_CHANFREE` → that one boundary spans **A** and
///    **B**, while `P_PEER`'s boundary still touches only **B**.
#[test]
fn one_boundary_spanning_two_condemned_entries_merges_them_for_good() {
    let _wd = watchdog("condemned_entries_merge", Duration::from_secs(60));
    let (mut gpu, pids, _rec) = mean_gpu();
    const ALIAS_BC: HObject = HObject(0x7100_00c0);
    const ALIAS_WC: HObject = HObject(0x7100_00c1);

    // ---- 1. One component out of two procs, then condemned as one entry.
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(client_of(P_PEER), H_VASPACE),
        dst: NodeKey::new(client_of(P_CHANFREE), ALIAS_BC),
    })
    .expect("the sharing dup applies");
    let shared = gpu.spine.by_pdb[&(gpu_of(P_PEER), lane_of(P_PEER).pdb)];
    assert_eq!(
        gpu.spine.by_pdb[&(gpu_of(P_CHANFREE), lane_of(P_CHANFREE).pdb)],
        shared,
        "the dup must have made them ONE proc, or there is nothing to condemn jointly",
    );
    assert!(core_retire_out_of_band(&mut gpu, shared));

    // ---- 2. A second, disjoint condemned entry.
    assert!(core_retire_out_of_band(&mut gpu, pids[P_WITNESS]));
    assert_eq!(
        gpu.spine.condemned_len(),
        2,
        "two condemned components that share no client are two entries",
    );

    // ---- 3. Split the shared component. Both halves stay condemned, still under the
    //         one entry that condemned them — a split does not merge anything.
    gpu.apply(RmEvent::Free {
        client: client_of(P_CHANFREE),
        handle: ALIAS_BC,
    })
    .expect("the alias free applies");
    assert_eq!(gpu.spine.condemned_len(), 2, "a split merges nothing");
    for c in [P_WITNESS, P_PEER, P_CHANFREE] {
        assert!(
            gpu.spine.is_condemned(client_of(c)),
            "proc {c} must still be condemned after the split",
        );
    }

    // ---- 4. One boundary now spans BOTH entries.
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(client_of(P_CHANFREE), H_VASPACE),
        dst: NodeKey::new(client_of(P_WITNESS), ALIAS_WC),
    })
    .expect("the cross-entry dup applies");

    assert_eq!(
        gpu.spine.condemned_len(),
        1,
        "★★ a boundary that spans two condemned entries makes them ONE entry — carrying \
         them separately loses whichever one the rebuild is not keyed on",
    );
    for c in [P_WITNESS, P_PEER, P_CHANFREE] {
        assert!(
            gpu.spine.is_condemned(client_of(c)),
            "★★ proc {c} was condemned and NOTHING has freed its client root — a merge \
             of condemned entries may never resurrect a component",
        );
    }
    // …and the merge is durable: an unrelated client's refresh does not undo it.
    churn(&mut gpu, client_of(P_REROUTE), 0);
    assert_eq!(gpu.spine.condemned_len(), 1);
    assert!(gpu.spine.is_condemned(client_of(P_PEER)));
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
                anchor: ProcAnchor(ClientKey::first(client_of(P_TEARDOWN))),
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
        anchor: ProcAnchor(ClientKey::first(client_of(P_HUP))),
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
        !gpu.procs[&pids[P_PEER]]
            .client_values()
            .contains(&client_of(P_HUP)),
        "★ the dup put the condemned client inside a live proc — that IS the resurrect"
    );
    assert_eq!(
        gpu.procs[&pids[P_PEER]].client_values(),
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
        anchor: ProcAnchor(ClientKey::first(client_of(P_HUP))),
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
    assert!(p.client_values().contains(&R_CLIENT));
    assert!(
        !p.client_values().contains(&client_of(P_HUP)),
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
        anchor: ProcAnchor(ClientKey::first(client_of(P_HUP))),
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
                .all(|p| !p.client_values().contains(&client_of(P_HUP))),
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
            IsolateId::new(victim.0, gpu_of(P_HUP)),
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
                anchor: ProcAnchor(ClientKey::first(client_of(P_TEARDOWN))),
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
        anchor: ProcAnchor(ClientKey::first(client_of(P_HUP))),
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
    assert!(gpu.procs[&pid].client_values().contains(&VICTIM_CLIENT));
    assert!(
        !gpu.procs[&pid].client_values().contains(&client_of(P_HUP)),
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
        gpu.procs[&victim].client_values(),
        BTreeSet::from([VICTIM_CLIENT]),
        "★★ the victim's component must contain the victim's client and nothing else"
    );
    assert!(
        !gpu.procs[&attacker]
            .client_values()
            .contains(&VICTIM_CLIENT),
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
        anchor: ProcAnchor(ClientKey::first(client_of(P_HUP))),
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
        gpu.procs[&doomed].client_values(),
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
    assert!(!gpu.procs[&pid].client_values().contains(&client_of(P_HUP)));

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
        anchor: ProcAnchor(ClientKey::first(client_of(P_HUP))),
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
            .all(|p| !p.client_values().contains(&client_of(P_HUP))),
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
                anchor: ProcAnchor(ClientKey::first(c)),
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
            anchor: ProcAnchor(ClientKey::first(SPLIT_CLIENT)),
        }),
        "★★ the re-labelled survivor was served again — and the label it answers under \
         must be its OWN anchor, not the freed one"
    );
    assert_eq!(gpu.spine.condemned_len(), 1);
    assert!(
        gpu.procs
            .values()
            .all(|p| !p.client_values().contains(&SPLIT_CLIENT)),
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
        gpu.procs[&merged].client_values(),
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
            gpu.procs[&pid_a].client_values(),
            gpu.procs[&pid_b].client_values()
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
        gpu.procs[&victim].client_values(),
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
        gpu.procs[&victim].client_values(),
        BTreeSet::from([SHAPE_B_RECYCLED]),
        "★★ the victim's component must hold the victim's client and nothing else"
    );
    // ★★★ §12.42 (N1) — **CORRECTED, and the correction is the whole of N1.** This used to
    // assert `Err(UnknownPdb)`: *"the superseded declaration's address plane must belong to
    // nobody"*. Belonging to nobody is exactly what let `Gpu::vacate` +
    // `stage_dropped_vases` release the orphan's host VAS and its published backing while
    // RM still refcounts the resource for the alias holder
    // (`ogkm src/nvidia/src/kernel/mem_mgr/mem.c:1027-1031`) — the corruption-over-refusal
    // direction §12.40 §1 rejects as D4. Isolation is held by the declaration IDENTITY
    // (`assert_ne!(victim, orphan)` above), never by dropping a live resource on the floor.
    assert_eq!(
        kayfabe_fwd::route_pdb(&gpu.spine, GPU0, SHAPE_B_ORPHAN_PDB),
        Ok(orphan),
        "★★ the SUPERSEDED declaration's still-live address plane was taken away from it \
         by a LATER tenant of its `hClient` value — the plane RM keeps alive on the \
         attacker's alias must stay with the declaration that allocated it"
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
        anchor: ProcAnchor(ClientKey::first(client_of(P_HUP))),
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
        gpu.procs[&reborn].client_values(),
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

// =================================================================================
// ★★★ §12.41 — a RECYCLED OBJECT HANDLE
// =================================================================================

/// The process that recycles one of its own object handle values.
const RECYC_OBJ_CLIENT: HClient = HClient(0xE1);
/// The kernel (UVM-shaped) client that keeps the first incarnation alive.
const RECYC_OBJ_KERNEL: HClient = HClient(0xE2);
/// The first incarnation's page-directory base — the GHOST's address plane.
const RECYC_OBJ_PDB1: Pdb = Pdb(0x6100_0000);
/// The second incarnation's PDB — the SUCCESSOR's address plane, at the same handle.
const RECYC_OBJ_PDB2: Pdb = Pdb(0x6200_0000);
/// The kernel client's own PDB.
const RECYC_OBJ_KPDB: Pdb = Pdb(0x6300_0000);
/// The process's GR/CE vChids.
const RECYC_OBJ_GR: VChid = VChid(0x19a);
/// See [`RECYC_OBJ_GR`].
const RECYC_OBJ_CE: VChid = VChid(0x29a);
/// The kernel client's handles.
const RECYC_OBJ_KROOT: HObject = HObject(0x6e00_0000);
/// See [`RECYC_OBJ_KROOT`].
const RECYC_OBJ_KDEV: HObject = HObject(0x6e00_0001);
/// See [`RECYC_OBJ_KROOT`].
const RECYC_OBJ_KVAS: HObject = HObject(0x6e00_0010);
/// The handle the kernel client holds the ghost VASpace under.
const RECYC_OBJ_ALIAS: HObject = HObject(0x7e00_0001);

/// The VASpace handle both incarnations are allocated at (the `identical_handles`
/// process shape's own VASpace handle — i.e. the value a real CUDA process presents).
const RECYC_OBJ_VAS: HObject = HObject(0x5c00_0010);
/// The device handle both incarnations are parented on.
const RECYC_OBJ_DEV: HObject = HObject(0x5c00_0001);

/// ★★★ **A RECYCLED OBJECT HANDLE MUST NOT STEAL THE GHOST'S ADDRESS PLANE**
/// (`l1_concurrency.md` §12.41 — §12.25's deferred finding 2, closed).
///
/// The last place a value NVIDIA recycles was used as an identity. `RmNode::key` is the
/// handle a resource's `RM_ALLOC` created it at, and object-handle values are reusable
/// **by NVIDIA's design and with no quarantine**: a caller supplies `hObjectNew` verbatim
/// (`0` means "generate", `ogkm src/common/sdk/nvidia/inc/nvos.h:483`;
/// `clientAssignResourceHandle` branches on exactly that,
/// `ogkm .../resserv/src/rs_client.c:998-1005`), the only validation is that the value is
/// not *currently live* (`clientValidateNewResourceHandle_IMPL`, `rs_client.c:1446-1470`,
/// `NV_ERR_INSERT_DUPLICATE_NAME`), and freeing just `mapRemove`s it from the live map
/// (`rs_client.c:1137`) with no free list and no deferred release.
///
/// So `Alloc (A,H) → Dup to K → Free (A,H) → Alloc (A,H)` — five ordinary events, each one
/// legal — leaves **two live VASpace resources reporting at one `(client, handle)`**: the
/// ghost RM keeps alive on K's alias (`ogkm .../mem_mgr/mem.c:1027-1031`) and the current
/// allocation. `ProcBoundary::vases` was keyed on that handle, so the second `insert`
/// **silently overwrote the first**: the ghost left `by_pdb` entirely, its runtime `Vas`
/// was staged for release, and every address op naming the page directory K legitimately
/// holds took `FwdFault::UnknownPdb`. §12.33's landed rule — *a kernel reference keeps its
/// owner's object alive **and usable*** — was false for the one shape that reaches it.
///
/// **The fix may not be a refusal**, exactly as in §12.39: RM recycles by design, so
/// refusing the second `Alloc` would hang a legal guest. The test therefore asserts the
/// re-allocation is *accepted* and that BOTH planes are genuinely served, end to end, out
/// of the one proc's own isolate lane. The identity is minted by us instead —
/// `ResourceKey` = origin handle + live incarnation ordinal, which is order-independent
/// (allocations at one handle value are totally ordered by `ConflictingAlloc`) and so may
/// appear in `Boundaries`, which a counter-minted id may not (§12.40's `ClientId` lesson).
#[test]
fn a_recycled_object_handle_never_steals_the_ghosts_address_plane() {
    let _wd = watchdog("recycled_object_handle_data", Duration::from_secs(60));
    let (mut gpu, _pids, _rec) = mean_gpu();

    // ---- An ordinary compute process, and the guest KERNEL aliasing its VASpace (the
    // measured UVM shape: one session client, every process dups into it).
    reinit(
        &mut gpu,
        RECYC_OBJ_CLIENT,
        RECYC_OBJ_PDB1,
        RECYC_OBJ_GR,
        RECYC_OBJ_CE,
        GPU0,
    );
    let mut s = Scenario::new();
    s.uvm_dup(
        RECYC_OBJ_KERNEL,
        RECYC_OBJ_KROOT,
        RECYC_OBJ_KDEV,
        RECYC_OBJ_KVAS,
        RECYC_OBJ_KPDB,
        RECYC_OBJ_ALIAS,
        NodeKey::new(RECYC_OBJ_CLIENT, RECYC_OBJ_VAS),
    );
    for ev in s.events {
        gpu.apply(ev).expect("the kernel client's dup applies");
    }
    let owner = gpu
        .spine
        .by_pdb
        .get(&(GPU0, RECYC_OBJ_PDB1))
        .copied()
        .expect("the process's first VASpace routes");

    // ---- The recycle: free the VASpace HANDLE (not the namespace — the process lives
    // on), then allocate a DIFFERENT VASpace at the very same handle value.
    gpu.apply(RmEvent::Free {
        client: RECYC_OBJ_CLIENT,
        handle: RECYC_OBJ_VAS,
    })
    .expect("freeing one object handle is ordinary");
    for ev in [
        RmEvent::Alloc {
            client: RECYC_OBJ_CLIENT,
            parent: RECYC_OBJ_DEV,
            handle: RECYC_OBJ_VAS,
            class: mock_classes::VASPACE,
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: RECYC_OBJ_CLIENT,
            vaspace: RECYC_OBJ_VAS,
            pdb: RECYC_OBJ_PDB2,
        },
    ] {
        gpu.apply(ev).expect(
            "★ re-allocating a FREED object handle is LEGAL — RM validates a \
             caller-supplied handle only against the LIVE map (`rs_client.c:1446-1470`) \
             and quarantines nothing on free (`:1137`); refusing would hang a real guest",
        );
    }

    // ---- ★★ THE BREACH, first and in the form the guest sees it: the dup-kept ghost's
    // address plane must still SERVE. (`core_publish` routes through `by_pdb` and then
    // materializes into the owning `Proc`'s own `Vas`, so it fails on either half of the
    // collapse — a lost route or a `Vas` staged for release.)
    assert_eq!(
        core_publish(&mut gpu, GPU0, RECYC_OBJ_PDB1, VA_CTL).map(|_| owner),
        Ok(owner),
        "★★ the dup-kept GHOST VASpace lost its ADDRESS PLANE: a resource RM says is \
         live (the kernel's alias refcounts it, `ogkm mem.c:1027-1031`) can no longer be \
         published into, because the guest re-used its origin handle VALUE and the \
         projection keyed on that value"
    );
    assert_eq!(
        gpu.spine.by_pdb.get(&(GPU0, RECYC_OBJ_PDB1)).copied(),
        Some(owner),
        "★★ the dup-kept GHOST VASpace lost its ADDRESS PLANE: a resource RM says is \
         live (the kernel's alias refcounts it, `ogkm mem.c:1027-1031`) stopped routing \
         because the guest re-used its origin handle VALUE — every op naming the page \
         directory the kernel legitimately holds now takes `UnknownPdb`"
    );
    // …and the successor's plane is its OWN, not the ghost's.
    assert_eq!(
        gpu.spine.by_pdb.get(&(GPU0, RECYC_OBJ_PDB2)).copied(),
        Some(owner),
        "★★ the successor's own address plane was never materialized"
    );
    let vases = &gpu.procs[&owner].vases;
    assert_eq!(
        vases
            .keys()
            .filter(|(g, _)| *g == GPU0)
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([(GPU0, RECYC_OBJ_PDB1), (GPU0, RECYC_OBJ_PDB2)]),
        "★★ two live VASpace resources ⇒ two runtime `Vas`es; a collapse leaves one"
    );
    // The two `Vas`es are DIFFERENT resources, not one resource reported twice — the
    // whole point of the identity. Their origin HANDLE is deliberately identical.
    let (g1, g2) = (
        vases[&(GPU0, RECYC_OBJ_PDB1)].origin,
        vases[&(GPU0, RECYC_OBJ_PDB2)].origin,
    );
    assert_eq!(
        g1.origin, g2.origin,
        "the two incarnations share their origin HANDLE — that is the premise"
    );
    assert_ne!(
        g1.incarnation, g2.incarnation,
        "★★ …and they must NOT share an identity"
    );

    // ---- …and the successor is genuinely SERVED too, end to end, in the proc's own lane.
    core_publish(&mut gpu, GPU0, RECYC_OBJ_PDB2, VA_CTL)
        .expect("★ the successor publishes host backing into the owning `Proc`");
    let rung = publish_and_ring(&mut gpu, GPU0, RECYC_OBJ_PDB2, RECYC_OBJ_GR, VA_HELD);
    assert_eq!(rung.proc, owner);
    assert_eq!(
        token_lane(rung.host_token),
        (owner.0 + 1, GPU0.0),
        "★ the process rang a host token minted in its OWN isolate lane"
    );

    // ---- ★ The mechanism, last: the graph reports two resources at one handle value,
    // and each answers with ITS OWN declared PDB.
    let both: Vec<_> = gpu
        .spine
        .rmgraph
        .nodes()
        .filter(|n| n.key == NodeKey::new(RECYC_OBJ_CLIENT, RECYC_OBJ_VAS))
        .map(|n| (n.id(), gpu.spine.rmgraph.pdb_of_resource(n.id())))
        .collect();
    assert_eq!(
        both.iter().map(|(_, p)| *p).collect::<Vec<_>>(),
        vec![Some(RECYC_OBJ_PDB1), Some(RECYC_OBJ_PDB2)],
        "★ each incarnation must answer with the page directory IT declared"
    );
    // The kernel's alias still resolves to the GHOST, not to the successor.
    assert_eq!(
        gpu.spine
            .rmgraph
            .origin_of(NodeKey::new(RECYC_OBJ_KERNEL, RECYC_OBJ_ALIAS))
            .map(RmNode::id),
        Some(both[0].0),
        "★ the kernel's dup alias must keep resolving to the resource it aliased"
    );
}

/// The process that recycles a CHANNEL handle value.
const RECYC_CH_CLIENT: HClient = HClient(0xE5);
/// The kernel client that keeps the first channel alive through a dup.
const RECYC_CH_KERNEL: HClient = HClient(0xE6);
/// Its PDB.
const RECYC_CH_PDB: Pdb = Pdb(0x6500_0000);
/// The kernel client's PDB.
const RECYC_CH_KPDB: Pdb = Pdb(0x6600_0000);
/// The first incarnation's GR vChid — the GHOST channel's exec plane.
const RECYC_CH_GR1: VChid = VChid(0x19c);
/// The CE vChid (untouched by the recycle).
const RECYC_CH_CE: VChid = VChid(0x29c);
/// The SECOND incarnation's vChid, at the same channel handle value.
const RECYC_CH_GR2: VChid = VChid(0x19d);
/// The GR channel handle both incarnations are allocated at.
const RECYC_CH_HANDLE: HObject = HObject(0x5c00_0019);
/// The TSG both incarnations are parented on.
const RECYC_CH_TSG: HObject = HObject(0x5c00_0012);
/// The kernel client's handles.
const RECYC_CH_KROOT: HObject = HObject(0x6f00_0000);
/// See [`RECYC_CH_KROOT`].
const RECYC_CH_KDEV: HObject = HObject(0x6f00_0001);
/// See [`RECYC_CH_KROOT`].
const RECYC_CH_KVAS: HObject = HObject(0x6f00_0010);
/// The handle the kernel client holds the ghost channel under.
const RECYC_CH_ALIAS: HObject = HObject(0x7f00_0002);

/// ★★★ **A RECYCLED CHANNEL HANDLE MUST NOT SHARE THE GHOST'S HOST CHANNEL**
/// (`l1_concurrency.md` §12.41) — the exec-plane half, and the sharper one.
///
/// `Gpu::sync_proc_to_boundary` mints a **stable `ChanId` per key**
/// (`chan_ids.entry(key).or_insert_with(mint)`), so when two live channel resources
/// reported at one recycled handle value they were handed ONE `ChanId` — one runtime
/// `Channel`, one `host_channel`, one `host_token` — while step 4 of `Spine::refresh`
/// filed **both** their `(GpuId, VChid)` entries onto it. A doorbell on the ghost's vChid
/// therefore rang the successor's host channel: a guest-visible mis-submission, not merely
/// a lost route.
#[test]
fn a_recycled_channel_handle_never_shares_the_ghosts_host_channel() {
    let _wd = watchdog("recycled_object_handle_exec", Duration::from_secs(60));
    let (mut gpu, _pids, _rec) = mean_gpu();

    reinit(
        &mut gpu,
        RECYC_CH_CLIENT,
        RECYC_CH_PDB,
        RECYC_CH_GR1,
        RECYC_CH_CE,
        GPU0,
    );
    let mut s = Scenario::new();
    s.uvm_dup(
        RECYC_CH_KERNEL,
        RECYC_CH_KROOT,
        RECYC_CH_KDEV,
        RECYC_CH_KVAS,
        RECYC_CH_KPDB,
        RECYC_CH_ALIAS,
        NodeKey::new(RECYC_CH_CLIENT, RECYC_CH_HANDLE),
    );
    for ev in s.events {
        gpu.apply(ev)
            .expect("the kernel client's channel dup applies");
    }
    let owner = gpu
        .spine
        .by_pdb
        .get(&(GPU0, RECYC_CH_PDB))
        .copied()
        .expect("the process routes");
    let ghost_cid = gpu.spine.by_vchid[&(GPU0, RECYC_CH_GR1)].1;

    // The recycle: free the channel handle, allocate a DIFFERENT channel at that value.
    gpu.apply(RmEvent::Free {
        client: RECYC_CH_CLIENT,
        handle: RECYC_CH_HANDLE,
    })
    .expect("freeing one channel handle is ordinary");
    gpu.apply(RmEvent::Alloc {
        client: RECYC_CH_CLIENT,
        parent: RECYC_CH_TSG,
        handle: RECYC_CH_HANDLE,
        class: mock_classes::CHANNEL_GR,
        facts: AllocFacts {
            h_vaspace: Some(RECYC_OBJ_VAS),
            userd_flags: MockArch::userd_flags_for(RECYC_CH_GR2),
            ..Default::default()
        },
    })
    .expect("★ re-allocating a freed channel handle is LEGAL (see the data-plane test)");

    // ---- ★★ THE BREACH, first: BOTH live channels must have an exec plane of their own.
    let route = |gpu: &Gpu, v: VChid| gpu.spine.by_vchid.get(&(GPU0, v)).copied();
    let (r1, r2) = (route(&gpu, RECYC_CH_GR1), route(&gpu, RECYC_CH_GR2));
    assert_eq!(
        (r1.map(|(p, _)| p), r2.map(|(p, _)| p)),
        (Some(owner), Some(owner)),
        "★★ a live channel lost its EXEC PLANE to a recycled handle value: the ghost (the \
         kernel's dup keeps it alive) and its successor report at one `(client, handle)`, \
         and the projection keyed `channels`/`chan_ids` on that value — so only one of \
         the two ever reaches `by_vchid` and its doorbell"
    );
    let (c1, c2) = (r1.expect("routed").1, r2.expect("routed").1);
    assert_ne!(
        c1, c2,
        "★★ the ghost and its successor were handed ONE `ChanId` — one runtime `Channel`, \
         one host channel and one host token — so a doorbell on the ghost's vChid rings \
         the SUCCESSOR's channel"
    );
    assert_eq!(
        c1, ghost_cid,
        "★ the ghost KEPT its ChanId (identity is stable)"
    );

    // Ring both. Each must reach its own host channel, and the two host channels must
    // be distinct host objects.
    let mut tokens = BTreeSet::new();
    for vchid in [RECYC_CH_GR1, RECYC_CH_GR2] {
        let rung = kayfabe_fwd::handle_doorbell(&mut gpu, GPU0, MockArch::token_for(vchid), &[])
            .expect("★ both live channels ring");
        assert_eq!(rung.proc, owner);
        tokens.insert(rung.host_token);
    }
    assert_eq!(
        tokens.len(),
        2,
        "★★ the two live channels rang ONE host token — the exec planes are aliased"
    );
    let hosts: BTreeSet<u64> = [c1, c2]
        .iter()
        .filter_map(|c| gpu.procs[&owner].channels[c].host_channel.map(|h| h.raw()))
        .collect();
    assert_eq!(
        hosts.len(),
        2,
        "★★ the two live channels share ONE host channel object"
    );
}

// =================================================================================
// ★★★ §12.42 (N1/N2) — TWO GENERATIONS OF ONE `hClient`, AND THE HOST MEMORY RM SAYS
// IS LIVE
//
// §12.39/§12.40 closed the recycled namespace in the *isolation* direction: a later
// tenant of an `hClient` value must not inherit the previous tenant's `Proc`. It bought
// that with an EXCLUSION — a resource projected only while its recorded owner was *the*
// declaration its value projected under, and a value had exactly one slot
// (`ProcAnchor` and `ProcBoundary::clients` were `HClient`s, so two generations were not
// expressible as two components).
//
// §12.41 §4 measured what the exclusion costs. The instant the value is re-declared, the
// PREVIOUS declaration's still-live resources answer to nobody: no boundary, so the
// component vanishes, so `Gpu::vacate` + `stage_dropped_vases` release its host VAS and
// its published backing — **while a live kernel client still references the resource RM
// refcounts** (`ogkm src/nvidia/src/kernel/mem_mgr/mem.c:1027-1031`). That is the
// corruption-over-refusal direction §12.40 §1 refuses as D4 and `cross_proc_lifetime`
// pins against, reached by walking around the rule rather than through it.
//
// §12.42's fix is the identity, not another exclusion: a component is labelled by a
// `ClientKey` = (`hClient`, incarnation), so the two generations ARE two components and
// every live resource keeps projecting under the declaration that allocated it.
// =================================================================================

/// The `hClient` the guest kernel hands out twice — the recycled namespace value.
const GEN_CLIENT: HClient = HClient(0xC0);
/// Generation 1's page-directory base (allocated on GPU0).
const GEN1_PDB: Pdb = Pdb(0x3c00_0000);
/// Generation 1's GR vChid.
const GEN1_GR: VChid = VChid(0x1a0);
/// Generation 1's CE vChid.
const GEN1_CE: VChid = VChid(0x2a0);
/// Generation 2's page-directory base (allocated on GPU1 — the multi-GPU axis: the two
/// lifetimes of one value need not even share a target).
const GEN2_PDB: Pdb = Pdb(0x3d00_0000);
/// Generation 2's GR vChid.
const GEN2_GR: VChid = VChid(0x1a1);
/// Generation 2's CE vChid.
const GEN2_CE: VChid = VChid(0x2a1);
/// Generation 3's page-directory base (back on GPU0).
const GEN3_PDB: Pdb = Pdb(0x3e00_0000);
/// Generation 3's GR vChid.
const GEN3_GR: VChid = VChid(0x1a2);
/// Generation 3's CE vChid.
const GEN3_CE: VChid = VChid(0x2a2);
/// The guest kernel's UVM session client — the measured `nvUvmInterfaceSessionCreate`
/// shape: one per `nvidia_uvm` module load, every CUDA process dups into it.
const GEN_UVM: HClient = HClient(0xC1D0_0069);
/// The session's own device handle.
const GEN_UVM_DEV: HObject = HObject(0x7b00_0001);
/// The session's own VASpace handle.
const GEN_UVM_VAS: HObject = HObject(0x7b00_0010);
/// The session's own PDB.
const GEN_UVM_PDB: Pdb = Pdb(0x3f00_0000);
/// The handle the session holds generation 1's VASpace under.
const GEN1_ALIAS: HObject = HObject(0x7b00_0100);
/// …generation 2's.
const GEN2_ALIAS: HObject = HObject(0x7b00_0101);
/// A VA lane of this test's own, so it never collides with the world's publications.
const VA_GEN: u64 = 0x50_0000_0000;

/// Every host object still outstanding on `(pid, gpu)`'s isolate, read off the ledger.
fn outstanding_on(rec: &SharedRecorder, pid: ProcId, gpu: GpuId) -> BTreeSet<HostHandle> {
    rec.lock()
        .expect("recorder")
        .ledger()
        .leaked_on(IsolateId::new(pid.0, gpu))
}

/// The host objects `pid`'s live core state can still NAME. Empty when its `Proc` is
/// gone — which, for a resource RM still refcounts, is the corruption this file's N1 test
/// is about: nothing can address it, so `Spine::vacate` has staged it for release.
fn reachable_of(gpu: &Gpu, pid: ProcId) -> BTreeSet<HostHandle> {
    gpu.procs
        .get(&pid)
        .map(reachable_objects)
        .unwrap_or_default()
}

/// Assert the three corruption classes are empty — never a disposition, never
/// declarable (`tests/src/teardown.rs`).
fn assert_no_corruption(rec: &SharedRecorder, when: &str) {
    let l = rec.lock().expect("recorder").ledger();
    assert_eq!(
        (
            l.double_free.as_slice(),
            l.free_of_unknown.as_slice(),
            l.unmap_of_unknown.as_slice()
        ),
        (&[][..], &[][..], &[][..]),
        "★ {when}: a host object was released twice, or through an isolate that never \
         minted it"
    );
}

/// Build one generation of [`GEN_CLIENT`] on `target` and drive it end to end: publish
/// real backing into its plane and ring its GR channel. Returns `(ProcId, VASpace key)`.
fn gen_up(gpu: &mut Gpu, pdb: Pdb, gr: VChid, ce: VChid, target: GpuId, va: u64) -> ProcId {
    reinit(gpu, GEN_CLIENT, pdb, gr, ce, target);
    let rung = publish_and_ring(gpu, target, pdb, gr, va);
    rung.proc
}

/// ★★★ **N1 — A SUPERSEDED DECLARATION'S KERNEL-HELD HOST MEMORY IS NEVER FREED BY ITS
/// SUCCESSOR** (`l1_concurrency.md` §12.42; the defect §12.41 §4 measured and did not
/// land).
///
/// The script is §12.41 §4's, verbatim, driven through the whole device against a live
/// six-proc two-GPU world:
///
/// ```text
/// declare GEN_CLIENT (gen1) on GPU0, publish + ring, UVM session dups its VASpace
/// free GEN_CLIENT's root      ⇒ gen1's `Proc` SURVIVES (§12.33 — RM refcounts the
///                                resource for the session, so retiring the owner would
///                                free host memory a live kernel client is still reading)
/// re-declare GEN_CLIENT (gen2) on GPU1, publish + ring, UVM dups that VASpace too
/// free GEN_CLIENT's root again
/// re-declare GEN_CLIENT (gen3) on GPU0
/// ```
///
/// **The breach, asserted first:** the host objects outstanding on generation 1's isolate
/// at the moment its root was freed must still be outstanding after the value is
/// re-declared. Under the `HClient`-keyed component plane they were not: gen1's
/// declaration was displaced by gen2's, its resources belonged to no boundary, its
/// component vanished and `stage_dropped_vases` freed its host VAS and its backing while
/// the UVM session still held a `DUP_OBJECT` of the VASpace. Measured on the
/// conservation ledger ([`kayfabe_mocks::HostLedger`]), so a revert reports *"we freed
/// something RM says is live"* rather than *"a count differed"*.
///
/// **The hangs-a-legal-guest gates, asserted beside it:** every re-declaration is
/// ACCEPTED (RM recycles `hClient` values by design — a caller-supplied `hRoot` is
/// honoured verbatim, `ogkm rs_server.c:612` with the reject guard compiled out at
/// `:613-616`, and RM's own generator wraps at 2^20 with no free list, `:3319-3341`), and
/// **every** generation stays usable end to end: each publishes and rings in its own
/// isolate lane, on its own target, out of its own arena.
#[test]
fn a_superseded_declarations_kernel_held_memory_is_never_freed_by_its_successor() {
    let _wd = watchdog("generation_recycle_n1", Duration::from_secs(60));
    let (mut gpu, pids, rec) = mean_gpu();

    // ---- Generation 1, on GPU0, with the guest kernel's session holding a reference.
    let gen1 = gen_up(&mut gpu, GEN1_PDB, GEN1_GR, GEN1_CE, GPU0, VA_GEN);
    let gen1_vas = NodeKey::new(GEN_CLIENT, identical_handles(0, 0).vaspace);
    let mut s = Scenario::new();
    s.uvm_dup(
        GEN_UVM,
        HObject(GEN_UVM.0),
        GEN_UVM_DEV,
        GEN_UVM_VAS,
        GEN_UVM_PDB,
        GEN1_ALIAS,
        gen1_vas,
    );
    for ev in s.events {
        gpu.apply(ev).expect("the UVM session's script applies");
    }
    assert!(
        gpu.system.client_values().contains(&GEN_UVM),
        "precondition: the session client is the SYSTEM component's (§12.27)"
    );
    assert_ne!(
        kayfabe_fwd::route_pdb(&gpu.spine, GPU0, GEN1_PDB),
        Ok(Gpu::SYSTEM_PROC),
        "precondition: a kernel dup is a REFERENCE, never a merge — gen1 keeps its own \
         `Proc`"
    );
    let gen1_hosts = host_identities(&gpu.procs[&gen1]);
    let gen1_arena = gpu.procs[&gen1].arenas[&GPU0].range.clone();

    // ---- The process exits: its client root is freed while the session's alias lives.
    gpu.apply(RmEvent::Free {
        client: GEN_CLIENT,
        handle: identical_handles(0, 0).client_root,
    })
    .expect("gen1's client root frees");
    // §12.33's landed rule, re-asserted here as the regression gate this test builds on:
    // the owning `Proc` must survive its owner, because RM's refcount says the resource
    // is live.
    assert_eq!(
        kayfabe_fwd::route_pdb(&gpu.spine, GPU0, GEN1_PDB),
        Ok(gen1),
        "★ §12.33: a kernel reference keeps its owner's `Proc` — and its host memory — \
         alive after the owner is gone"
    );
    // ★ THE BASELINE, in the teardown post-condition's own two-part vocabulary
    // (`tests/src/teardown.rs`): what gen1's core state can still NAME, and what the host
    // still holds for its isolate (a superset — the channels its root free dropped are
    // staged, not yet drained). Non-empty by construction (it published and rang), which
    // the assertion states rather than assumes.
    let gen1_named = reachable_of(&gpu, gen1);
    let gen1_held = outstanding_on(&rec, gen1, GPU0);
    assert!(
        gen1_named.len() >= 2 && gen1_held.is_superset(&gen1_named),
        "★ the baseline must be REAL host state the core can name — a host VAS and its \
         backing at least: named {gen1_named:?}, held {gen1_held:?}"
    );
    assert_no_corruption(&rec, "after gen1's root free");

    // ---- ★★ THE RECYCLE. A later process is handed the same `hClient`. It must be
    // ACCEPTED (D0 is the hangs-a-legal-guest error), and it lands on the OTHER GPU.
    let gen2 = gen_up(&mut gpu, GEN2_PDB, GEN2_GR, GEN2_CE, GPU1, VA_GEN);

    // ---- ★★★ THE BREACH, in two halves, and the FIRST is the corruption.
    assert_eq!(
        reachable_of(&gpu, gen1),
        gen1_named,
        "★★ HOST MEMORY RM SAYS IS LIVE WAS TAKEN AWAY FROM ITS OWNER. The guest kernel's \
         UVM session still holds a `DUP_OBJECT` of gen1's VASpace — RM refcounts it \
         (`ogkm .../mem_mgr/mem.c:1027-1031`) — and an unrelated later process being \
         handed the same `hClient` VALUE left gen1's host VAS and its published backing \
         nameable by NOTHING in core state, which is `Spine::vacate` staging them for \
         release. That is `corruption over refusal`, which §12.40 §1 rejects as D4"
    );
    // …and it is not merely un-nameable: drive the reclamation the core has SCHEDULED.
    // Under the defect gen1's `Proc` is on the retired list and this reap issues the host
    // `Free` verbs for a live kernel client's memory; under the rule it is a no-op.
    let _ = gpu.reap_retired();
    assert_eq!(
        outstanding_on(&rec, gen1, GPU0)
            .intersection(&gen1_named)
            .copied()
            .collect::<BTreeSet<_>>(),
        gen1_named,
        "★★ …and the scheduled reclamation RAN: host `Free` verbs were issued for a \
         VASpace and a backing the guest kernel is still legally reading"
    );
    assert_eq!(
        kayfabe_fwd::route_pdb(&gpu.spine, GPU0, GEN1_PDB),
        Ok(gen1),
        "★★ the superseded declaration's still-live address plane stopped routing — a \
         VA the session can still legally use became an anonymous miss"
    );
    // …and it is USABLE, not merely present in a map: the session's reference can still
    // be published into (§12.33's "alive AND usable").
    core_publish(&mut gpu, GPU0, GEN1_PDB, VA_GEN + 0x1_0000)
        .expect("★★ the kernel-referenced plane must still accept host work");
    // …which mints exactly one more nameable host object, so the baseline moves FORWARD.
    // Re-taken rather than relaxed: the property is "nothing that was live was released",
    // and a growing EXACT set keeps it an equality at every later step instead of a
    // `contains` that a wholesale reclamation could still satisfy.
    let gen1_named_after_use = reachable_of(&gpu, gen1);
    assert_eq!(
        gen1_named_after_use.len(),
        gen1_named.len() + 1,
        "★ the usability probe must ADD exactly its own backing and release nothing: \
         {gen1_named:?} -> {gen1_named_after_use:?}"
    );
    let gen1_named = gen1_named_after_use;

    // ---- …while the successor is a genuinely DIFFERENT component, sharing nothing.
    assert_ne!(
        gen2, gen1,
        "★★ the successor INHERITED the previous tenant's `Proc` — its isolate (one host \
         RM client namespace), its GPA arena, its host VASes and its `pending_release` \
         queue — because the two were matched on a recyclable `hClient` VALUE"
    );
    assert_eq!(
        gpu.procs[&gen2].clients,
        BTreeSet::from([ClientKey {
            client: GEN_CLIENT,
            incarnation: 1
        }]),
        "★★ the successor's component holds its OWN declaration and nothing else"
    );
    assert_eq!(
        gpu.procs[&gen1].clients,
        BTreeSet::from([ClientKey::first(GEN_CLIENT)]),
        "★★ …and the predecessor keeps its own — two lifetimes of one value, two \
         components, which an `HClient`-keyed `ProcAnchor` could not express (N2)"
    );
    assert!(
        host_identities(&gpu.procs[&gen2]).is_disjoint(&gen1_hosts),
        "★★ the successor came up holding a host object its predecessor minted"
    );
    let gen2_arena = gpu.procs[&gen2].arenas[&GPU1].range.clone();
    assert!(
        gen2_arena.end <= gen1_arena.start || gen1_arena.end <= gen2_arena.start,
        "★★ the successor carved into its predecessor's GPA arena: {gen2_arena:?} vs \
         {gen1_arena:?}"
    );
    assert_no_corruption(&rec, "after the first recycle");

    // ---- ★ A THIRD generation, so the property is about a SEQUENCE and not about one
    // pair — the ordinary state of a long-running guest, whose client index wraps at
    // 2^20 with no free list. The session aliases gen2 as well, then gen2's root goes.
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(GEN_CLIENT, identical_handles(0, 0).vaspace),
        dst: NodeKey::new(GEN_UVM, GEN2_ALIAS),
    })
    .expect("the session aliases generation 2's VASpace too");
    gpu.apply(RmEvent::Free {
        client: GEN_CLIENT,
        handle: identical_handles(0, 0).client_root,
    })
    .expect("gen2's client root frees");
    let gen2_named = reachable_of(&gpu, gen2);
    let gen3 = gen_up(&mut gpu, GEN3_PDB, GEN3_GR, GEN3_CE, GPU0, VA_GEN);

    assert_eq!(
        (reachable_of(&gpu, gen1), reachable_of(&gpu, gen2)),
        (gen1_named.clone(), gen2_named),
        "★★ a third tenant of the value took an EARLIER generation's kernel-held host \
         memory away — the defect is about the number of live declarations, not about two"
    );
    let planes: BTreeMap<Pdb, Result<ProcId, FwdFault>> = [
        (GEN1_PDB, kayfabe_fwd::route_pdb(&gpu.spine, GPU0, GEN1_PDB)),
        (GEN2_PDB, kayfabe_fwd::route_pdb(&gpu.spine, GPU1, GEN2_PDB)),
        (GEN3_PDB, kayfabe_fwd::route_pdb(&gpu.spine, GPU0, GEN3_PDB)),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        planes,
        BTreeMap::from([
            (GEN1_PDB, Ok(gen1)),
            (GEN2_PDB, Ok(gen2)),
            (GEN3_PDB, Ok(gen3)),
        ]),
        "★★ three live declarations of one `hClient`, three address planes, three `Proc`s \
         — every one of them still routing to the component that allocated it"
    );
    assert_eq!(
        BTreeSet::from([gen1, gen2, gen3]).len(),
        3,
        "★★ two generations of one value collapsed onto one `Proc`"
    );

    // ---- ★ THE OTHER HALF OF THE BOUND: this is a DEFERRAL, not a leak. When the guest
    // kernel finally drops its alias, gen1's declaration owns nothing live, its component
    // goes, and its host state leaves through `Spine::vacate` — which STAGES before it
    // removes (§12.35's one removal point). Reclaimed when RM says it is dead, and not one
    // event sooner.
    gpu.apply(RmEvent::Free {
        client: GEN_UVM,
        handle: GEN1_ALIAS,
    })
    .expect("the session drops its reference to generation 1");
    assert_eq!(
        kayfabe_fwd::route_pdb(&gpu.spine, GPU0, GEN1_PDB),
        Err(FwdFault::UnknownPdb {
            gpu: GPU0,
            pdb: GEN1_PDB
        }),
        "★ with its last reference gone the superseded declaration owns nothing, so its \
         plane is a named MISS — the exclusion happens at refcount 0, which is where RM \
         puts it"
    );
    assert!(
        !gpu.procs.contains_key(&gen1),
        "★ …and its `Proc` left with it"
    );
    assert!(
        outstanding_on(&rec, gen1, GPU0).is_superset(&gen1_named),
        "★ its host state is STAGED at this instant, not yet drained — reclamation is a \
         scheduled fact of `pending_release`, and the teardown post-condition proves it \
         completes (this test declares no residue)"
    );
    assert_no_corruption(&rec, "after the reference finally drops");

    // ---- The world the recycle ran inside is undisturbed: all six original procs still
    // publish and ring in their own lanes, and generations 2 and 3 still serve.
    for (i, &pid) in pids.iter().enumerate() {
        let rung = publish_and_ring(
            &mut gpu,
            gpu_of(i),
            lane_of(i).pdb,
            lane_of(i).gr,
            VA_GEN + 0x2_0000,
        );
        assert_eq!(rung.proc, pid, "proc {i} was disturbed by the recycle");
        assert_eq!(
            token_lane(rung.host_token),
            (pid.0 + 1, gpu_of(i).0),
            "proc {i} rang out of another (proc, GPU)'s isolate"
        );
    }
    // Generation 3 is a live process: it publishes AND rings. Generation 2 is an orphan
    // like generation 1 was — its channels died with its root, so the honest probe is
    // that its kernel-referenced address plane still accepts host work.
    let rung = publish_and_ring(&mut gpu, GPU0, GEN3_PDB, GEN3_GR, VA_GEN + 0x3_0000);
    assert_eq!(rung.proc, gen3);
    assert_eq!(
        token_lane(rung.host_token),
        (gen3.0 + 1, GPU0.0),
        "★ the newest generation rang out of another isolate's lane"
    );
    core_publish(&mut gpu, GPU1, GEN2_PDB, VA_GEN + 0x3_0000)
        .expect("★ generation 2's kernel-referenced plane is still usable too");
    assert_eq!(
        kayfabe_fwd::handle_doorbell(&mut gpu, GPU1, MockArch::token_for(GEN2_GR), &[]),
        Err(FwdFault::UnknownVchid {
            gpu: GPU1,
            vchid: GEN2_GR
        }),
        "★ …but its EXEC plane died with its root: nothing dup'd its channels, so they \
         were reclaimed per object and their vChid is a named MISS"
    );

    // ★ AND THE RECLAMATION IS COMPLETE. Generation 1's `Proc` left through
    // `Spine::vacate`, which STAGES before it removes (§12.35's one removal point), so its
    // host VAS, its two backings and its channel are queued on its own isolate and
    // reclaimed per object — there is no residue to declare here, which is why this test
    // carries no `ResidueClaim`. The teardown post-condition
    // (`tests/src/teardown.rs`) asserts that for us when the guard drops.
}

// =================================================================================
// ★★★ THE GUEST-STEERABLE GPA HAZARD — `l1_os_shell.md` §10.1 item 6 / §6.1 / §6.3
//
// `Vmm::gpa_read` is the ONE in-lock-legal capability that takes a guest-chosen
// address, and `SharedDevice::parse_pushbuffer` runs it with rank 0 held. On a QEMU
// backend the obvious implementation decides *at run time, from the guest's number*,
// whether it is a memcpy or an acquisition of the VMM's global lock
// (`[src] v10.2.0 system/physmem.c:3250` / `:3347` -> `prepare_mmio_access`
// `:3196-3209`). That is §6.3's ABBA inversion, constructed on demand, and invisible
// to all four of §6.3's enforcement layers.
//
// The fix is a positive proof — `kayfabe_vmm::GuestRamMap` — and these tests are what
// makes it more than a sentence. Before them the harness COULD NOT EXPRESS the hazard:
// `MockVmm`'s guest-physical space was entirely RAM and `gpa_read` had no failing arm
// at all, so any test of the refusal would have been green on a path that did not
// exist (`testing_doctrine.md` §1).
// =================================================================================

/// Guest RAM. Everything outside the windows carved below stays RAM.
const GPA_PB_BASE: u64 = 0x1000_0000;
/// **Another emulated device's BAR, mapped INSIDE guest RAM** — the shape a guest
/// produces by re-programming a PCI BAR, and the one that makes a straddling read
/// possible at all.
const GPA_PEER_BAR: Range<u64> = 0x4000_0000..0x4001_0000;
/// **Our own trapped BAR0.** The nastiest target: serving it would have us DMA into
/// our own MMIO dispatch from inside a locked section.
const GPA_OUR_BAR0: Range<u64> = 0x7000_0000..0x7001_0000;
/// A hole in the flat view — backed by nothing. The near neighbour that must report
/// under a *different* name.
const GPA_HOLE: Range<u64> = 0x6000_0000..0x6001_0000;
/// The window torn down mid-flight while a thread is actively parsing out of it.
const GPA_REVOKE: u64 = 0x5000_0000;
/// Physical page base the DMA workload's CE PT-writes name (clear of every VA above).
const VA_PT: u64 = 0x60_0000_0000;
/// Submits per DMA thread. Four hostile shapes per legal one, so every arm runs many
/// times while the peers do real work.
const DMA_OPS: usize = 200;
/// Bounded budget for the revoke prober (an EDGE loop, never a sleep).
const REVOKE_BUDGET: usize = 100_000;

/// ★ The revoke prober's start signal — and it opens on **unwind** as well as on
/// success.
///
/// Found by a bite-check, not by design: neutering the resolver to refuse *everything*
/// made the prober panic on its very first guest write, so it never signalled, and the
/// main thread spun forever inside `thread::scope` waiting for a flag nobody would set.
/// The failure then looked like a **wedge** instead of like the assertion it was — the
/// exact shape [`Latches`]' own `Drop` guard exists for. A harness that hangs when the
/// code is wrong is worse than one that fails, because a hang gets re-run.
#[derive(Default)]
struct StartGate {
    open: Mutex<bool>,
    cv: Condvar,
}

impl StartGate {
    fn open(&self) {
        *self.open.lock().expect("start gate") = true;
        self.cv.notify_all();
    }
    /// Block until the prober has been served once **or has died** — an edge, never a
    /// sleep and never a spin.
    fn wait(&self) {
        let mut g = self.open.lock().expect("start gate");
        while !*g {
            g = self.cv.wait(g).expect("start gate");
        }
    }
}

/// Opens its [`StartGate`] on drop, including on unwind.
struct OpenOnDrop<'a>(&'a StartGate);
impl Drop for OpenOnDrop<'_> {
    fn drop(&mut self) {
        self.0.open();
    }
}

/// What one DMA thread observed. Every field is script-determined, so both lock modes
/// must produce identical values.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DmaTally {
    /// Legal pushbuffers that parsed and applied.
    legal_ok: usize,
    /// Refusals naming a device window (`NonRamGpa`) — the security-relevant arm.
    non_ram: usize,
    /// Refusals naming an unbacked address (`GpaRead`) — its near neighbour.
    unbacked: usize,
}

/// The scripted guest machine: RAM everywhere, with two device windows and a hole
/// carved out of it. Written as a narrowing of the mock's all-RAM default so the
/// declarations that MATTER are the only ones in the test.
fn dma_machine() -> SharedVmm {
    let vmm = SharedVmm::default();
    vmm.with(|v| {
        v.declare_device_mmio(GPA_PEER_BAR);
        v.declare_device_mmio(GPA_OUR_BAR0);
        v.declare_unbacked(GPA_HOLE);
    });
    vmm
}

/// **The DMA-descriptor workload.** Interleaves a legitimate pushbuffer with the four
/// hostile shapes a guest can build out of one 16-byte GPFIFO entry, and asserts the
/// EXACT fault for each — including the boundary address of a range that starts in RAM
/// and runs into a device window, which is the byte a start-address-only check would
/// never look at.
fn dma_workload(dev: &SharedDevice, vmm: &SharedVmm, pids: &[ProcId], i: usize) -> DmaTally {
    let (lane, gpu, pid) = (lane_of(i), gpu_of(i), pids[i]);
    let mut vmm = vmm.clone();
    let cid = dev
        .doorbell(gpu, MockArch::token_for(lane.ce), &[])
        .expect("the DMA thread's CE channel rings")
        .chan;
    let scratch = GPA_PB_BASE + (i as u64) * 0x10_0000;
    let mut t = DmaTally::default();

    for k in 0..DMA_OPS {
        // ---- the LEGITIMATE submit: real work, through the same locked path.
        let pt_page = VA_PT + (i as u64) * 0x100_0000 + (k as u64) * 0x1000;
        let ring = script_ring(
            &vmm,
            scratch + (k as u64) * 0x100,
            &[
                MockPushbuffer::set_object(mock_classes::DMA_COPY),
                MockPushbuffer::ce_launch_dma(pt_page, 0x1000, false),
                MockPushbuffer::tlb_invalidate(lane.pdb.0, true),
                MockPushbuffer::method(0xEE, &[1, 2, 3]),
            ],
        );
        let out = dev
            .parse_pushbuffer(&mut vmm, pid, cid, &ring)
            .expect("a RAM-backed pushbuffer parses while peers hold verbs and DMA at MMIO");
        assert_eq!(
            out.pt_writes,
            vec![pt_page],
            "the legitimate submit's CE PT-write must be captured — non-vacuity for \
             every refusal below (an instrument that only ever refuses proves nothing)"
        );
        assert_eq!(out.opaque, 1, "the opaque method passed through");
        t.legal_ok += 1;

        // ---- the five HOSTILE shapes, each asserted by exact variant AND payload.
        let hostile: (Vec<u8>, Result<kayfabe_fwd::PushbufferOutcome, FwdFault>) = match k % 5 {
            // (a) aimed squarely at ANOTHER device's registers.
            0 => (
                gpfifo_ring(GPA_PEER_BAR.start, 0x40),
                Err(FwdFault::NonRamGpa {
                    gpa: GPA_PEER_BAR.start,
                }),
            ),
            // (b) aimed at OUR OWN trapped BAR0.
            1 => (
                gpfifo_ring(GPA_OUR_BAR0.start + 0x800, 8),
                Err(FwdFault::NonRamGpa {
                    gpa: GPA_OUR_BAR0.start + 0x800,
                }),
            ),
            // (c) aimed at a hole — the NEAR NEIGHBOUR, which must report differently.
            2 => (
                gpfifo_ring(GPA_HOLE.start + 0x10, 0x20),
                Err(FwdFault::GpaRead {
                    gpa: GPA_HOLE.start + 0x10,
                }),
            ),
            // (d) STRADDLING into a DEVICE window: starts in real RAM, runs into the
            //     peer BAR. A check that looked only at the start address would serve
            //     the first bytes and take the VMM's global lock on the continuation
            //     step. Reports the BOUNDARY byte, not the requested address.
            3 => (
                gpfifo_ring(GPA_PEER_BAR.start - 8, 0x40),
                Err(FwdFault::NonRamGpa {
                    gpa: GPA_PEER_BAR.start,
                }),
            ),
            // ★ (e) STRADDLING into a HOLE — (d)'s NEAR NEIGHBOUR, and the shape that
            //     was missing. `l1_os_shell.md` §14.8 **F6**: `kayfabe_fwd::guest_read`
            //     classified `VmmError::BadGpa` through a catch-all `_ =>` arm that
            //     substitutes the address the *request* started at, so the boundary the
            //     port had named was preserved on the `NonRamGpa` arm and DISCARDED on
            //     this one. Every straddle case the suite had was RAM→DEVICE — i.e. the
            //     arm that keeps it — so two refusals whose payloads meant different
            //     things went unnoticed. Fixed 2026-07-27; this row is what bites.
            _ => (
                gpfifo_ring(GPA_HOLE.start - 8, 0x40),
                Err(FwdFault::GpaRead {
                    gpa: GPA_HOLE.start,
                }),
            ),
        };
        let (ring, expect) = hostile;
        let got = dev.parse_pushbuffer(&mut vmm, pid, cid, &ring);
        assert_eq!(
            got, expect,
            "a guest-steered descriptor must refuse by EXACT name (k={k}, proc={pid:?})"
        );
        match expect {
            Err(FwdFault::NonRamGpa { .. }) => t.non_ram += 1,
            _ => t.unbacked += 1,
        }
        assert_eq!(
            lock::held_depth(),
            0,
            "a refused guest-memory read leaked a ranked guard"
        );
    }
    t
}

/// Result of the mid-flight window teardown probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RevokeProbe {
    /// Submits served before the window was torn down (must be > 0, or the probe never
    /// reached the path it claims to test).
    served: usize,
    /// The first refusal, exactly.
    first_refusal: Option<FwdFault>,
    /// How many refusals it saw in total. Carried so the port-level refusal census
    /// below is DERIVED rather than hard-coded — the first version of this test
    /// hard-coded the DMA threads' count and was off by exactly this number, which is
    /// the composed run finding an accounting error in its own instrument.
    refusals: usize,
    /// Submits served AFTER the first refusal — must be 0. A torn-down window that
    /// starts resolving again is a resurrected mapping.
    served_after_refusal: usize,
}

/// ★★★ **THE MEAN COMPOSED RUN for the guest-steerable GPA hazard.**
///
/// Everything in the same window, under **both** lock modes: two DMA threads (one per
/// GPU) aiming descriptors at two different device windows, a hole and a straddling
/// boundary, while
///
/// - four peer threads (`ctl_workload` + `ring_workload`, one pair per GPU) do real
///   publish/doorbell work,
/// - **five** host verbs are parked on four isolates across both GPUs,
/// - a proc is retired out of band and another's routed channel is freed mid-flight,
/// - a guest-memory **window is torn down while a thread is parsing out of it**.
///
/// The assertions are the composed ones: the exact refusal for each hostile shape, the
/// exact *counts* (so the instrument is proven reached, `testing_doctrine.md` §1 rule
/// 2), the peers' full progress, and a bit-identical outcome across the two lock modes.
#[test]
fn a_guest_steering_dma_descriptors_at_device_windows_is_refused_by_name_under_load() {
    let _wd = watchdog(
        "a_guest_steering_dma_descriptors_at_device_windows_is_refused_by_name_under_load",
        Duration::from_secs(300),
    );
    let degenerate = dma_mean_run(LockMode::Degenerate);
    let sharded = dma_mean_run(LockMode::Sharded);

    for (name, r) in [("Degenerate", &degenerate), ("Sharded", &sharded)] {
        let (w, p) = (r.witness, r.peer);
        // ---- every hostile shape was REACHED, the exact number of times scripted.
        for (who, t) in [("witness", w), ("peer", p)] {
            assert_eq!(
                t.legal_ok, DMA_OPS,
                "({name}/{who}) every RAM-backed submit must still be served — a \
                 refusal that also breaks the legal path is not a fix"
            );
            assert_eq!(
                t.non_ram,
                3 * (DMA_OPS / 5),
                "({name}/{who}) three of the five hostile shapes name a DEVICE window"
            );
            assert_eq!(
                t.unbacked,
                2 * (DMA_OPS / 5),
                "({name}/{who}) and exactly two name a HOLE — the near neighbour, \
                 counted separately so the two can never silently merge. Both a \
                 wholly-unbacked range and one that STRADDLES out of RAM into the hole \
                 land here, which is the pair F6 was hiding: they take the same arm and \
                 must still name different addresses"
            );
        }
        // ---- the mock's own ledger agrees: the refusals happened AT THE PORT, and
        // every one of them was classified. (The peers' legitimate traffic performs no
        // refused access at all, so this number is the DMA threads' alone.)
        assert_eq!(
            r.port_refusals,
            (
                2 * (3 * (DMA_OPS / 5)),                     // NonRamGpa
                2 * (2 * (DMA_OPS / 5)) + r.revoke.refusals, // BadGpa
            ),
            "({name}) the port refused exactly the accesses the core reported — the two \
             DMA threads' hostile shapes PLUS the revoke prober's, and nothing else. \
             Every refusal is classified: no third arm, no silent pass-through"
        );
        // ---- nothing hostile was ever half-applied.
        assert_eq!(
            r.pt_pages,
            (DMA_OPS, DMA_OPS),
            "({name}) each proc captured exactly its LEGAL submits' PT pages — a \
             refused ring must apply nothing at all"
        );
        // ---- mid-flight window teardown: served, then refused, and never served again.
        assert!(
            r.revoke.served > 0,
            "({name}) the revoke prober never had its window served — it proves nothing"
        );
        assert_eq!(
            r.revoke.first_refusal,
            Some(FwdFault::GpaRead { gpa: GPA_REVOKE }),
            "({name}) a window torn down under a live parser must refuse as UNBACKED — \
             not as a device window, and never as stale bytes"
        );
        assert_eq!(
            r.revoke.served_after_refusal, 0,
            "({name}) a torn-down window resolved again after being revoked"
        );
        // ---- the peers made full progress the whole time.
        assert_eq!(
            r.publications,
            2 * (CTL_OPS + RING_OPS.div_ceil(8)),
            "({name}) a peer workload thread bailed — the DMA refusals cost the rest of \
             the device its progress"
        );
        assert!(
            r.latches_all_pending,
            "({name}) the workloads only completed after a parked verb was released"
        );
        assert_eq!(
            r.canary_teardown,
            Err(FwdFault::Cancelled {
                proc: r.pid_teardown,
                reason: CancelReason::ProcExit
            }),
            "({name}) the retire canary still names the truth with DMA traffic composed \
             in: its verb was CANCELLED by the retire (§7.6 T2), not left to run to \
             completion against a proc that no longer exists"
        );
        // ★★ THE PREMISE, asserted rather than assumed: the guest-memory reads really
        // did run with one of OUR ranked locks held. If they did not, every refusal
        // above is a fact about a lock-free path and the hazard was never exercised at
        // all — the §1 "green instrument" failure, one level up. Exactly 1: the route
        // phase holds rank 0 (device read in Sharded, device write in Degenerate) and
        // nothing else while it reads guest memory.
        assert_eq!(
            r.lock_depth_span,
            (0, 1),
            "({name}) guest memory was accessed at ranked-lock depths {:?}. BOTH ends \
             are the claim: max 1 = the pushbuffer reads really ran under rank 0 (device \
             read in Sharded, device write in Degenerate), so the in-lock hazard was \
             actually constructed; min 0 = the scripting writes really ran lock-free, so \
             the witness varies with its caller instead of reporting a constant",
            r.lock_depth_span
        );
    }
    assert_eq!(
        degenerate.mode_independent(),
        sharded.mode_independent(),
        "the lock configuration is observable through the guest-DMA path"
    );
}

/// Everything the composed DMA run observed (mode-independent by construction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DmaMeanReport {
    witness: DmaTally,
    peer: DmaTally,
    /// `(NonRamGpa, BadGpa)` counted at the PORT, not at the core.
    port_refusals: (usize, usize),
    /// `Vas::pt_pages` len for the two DMA procs at the end.
    pt_pages: (usize, usize),
    revoke: RevokeProbe,
    publications: usize,
    latches_all_pending: bool,
    canary_teardown: Result<Published, FwdFault>,
    pid_teardown: ProcId,
    /// `(min, max)` ranked-lock nesting across every guest-memory access.
    lock_depth_span: (u32, u32),
}

impl DmaMeanReport {
    /// The report with its one genuinely racy field normalised away.
    ///
    /// `revoke.served` counts how many submits landed *before* the main thread tore the
    /// window down — a real race between two live threads, and therefore a **clock**,
    /// not an edge (`testing_doctrine.md` §3 rule 3). It is asserted as a BOUND above
    /// (`> 0`, i.e. the path was reached) and must never appear in a differential. The
    /// first version of this test compared it and failed on the second run with
    /// `served: 1` vs `served: 3` — which is the doctrine's own rule catching the test
    /// that violated it.
    fn mode_independent(mut self) -> Self {
        self.revoke.served = 0;
        self
    }
}

fn dma_mean_run(mode: LockMode) -> DmaMeanReport {
    let (device, pids, rec) = mean_world(mode);
    let vmm = dma_machine();
    let dev: &SharedDevice = &device;
    let pid_ref: &[ProcId] = &pids;
    let handles = identical_handles(0, 0);

    // Warm-up: every proc materializes its host VAS and its CE channel, so a canary's
    // held verb is the one the script names and not an incidental first touch.
    for i in 0..N_PROCS {
        dev.publish_backing(gpu_of(i), lane_of(i).pdb, GpuVa(VA_WARM), 0x1000)
            .expect("warm-up publish");
        dev.doorbell(gpu_of(i), MockArch::token_for(lane_of(i).ce), &[])
            .expect("warm-up CE ring");
    }

    let started = StartGate::default();
    let (witness, peer, revoke, publications, latches_all_pending, canary_teardown) =
        thread::scope(|sc| {
            // ---- park five host verbs across four isolates and both GPUs.
            let mut latches = Latches::new();
            latches.arm(&rec, pids[P_WITNESS], GPU0, 0, VerbKind::AllocSysmem);
            let t_w1 = sc.spawn(move || {
                dev.publish_backing(GPU0, lane_of(P_WITNESS).pdb, GpuVa(VA_HELD), 0x1000)
            });
            latches.wait_all_pending();
            latches.arm(&rec, pids[P_WITNESS], GPU0, 1, VerbKind::AllocSysmem);
            let t_w2 = sc.spawn(move || {
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

            // ---- the mixed concurrent workloads: DMA on both GPUs + the real peers.
            let vw = &vmm;
            let t_dma_w = sc.spawn(move || dma_workload(dev, vw, pid_ref, P_WITNESS));
            let t_dma_p = sc.spawn(move || dma_workload(dev, vw, pid_ref, P_PEER));
            let mut peers = Vec::new();
            for i in [P_WITNESS, P_PEER] {
                peers.push(sc.spawn(move || ctl_workload(dev, pid_ref, i)));
                peers.push(sc.spawn(move || ring_workload(dev, pid_ref, i)));
            }

            // ---- the revoke prober: parses out of GPA_REVOKE until its window is torn
            // down under it, then must never be served again.
            let started_ref = &started;
            let t_revoke = sc.spawn(move || {
                // ★ Opens the gate even if this thread unwinds — see `StartGate`.
                let _gate = OpenOnDrop(started_ref);
                let mut vmm = vw.clone();
                let cid = dev
                    .doorbell(GPU0, MockArch::token_for(lane_of(P_WITNESS).ce), &[])
                    .expect("the prober's CE channel rings")
                    .chan;
                // The ring bytes are OURS (never guest memory), so the only thing the
                // teardown can change is whether the RANGE resolves.
                let ring = gpfifo_ring(GPA_REVOKE, 0x40);
                vmm.gpa_write(GPA_REVOKE, &[0u8; 0x40])
                    .expect("the window is RAM before the teardown");
                let mut p = RevokeProbe {
                    served: 0,
                    first_refusal: None,
                    refusals: 0,
                    served_after_refusal: 0,
                };
                for _ in 0..REVOKE_BUDGET {
                    match dev.parse_pushbuffer(&mut vmm, pid_ref[P_WITNESS], cid, &ring) {
                        Ok(_) => {
                            if p.first_refusal.is_some() {
                                p.served_after_refusal += 1;
                                break;
                            }
                            p.served += 1;
                            started_ref.open();
                        }
                        Err(e) => {
                            p.refusals += 1;
                            if p.first_refusal.is_none() {
                                p.first_refusal = Some(e);
                                // One more round trip to prove it STAYS refused.
                                continue;
                            }
                            break;
                        }
                    }
                }
                p
            });

            // ---- the world MOVES: a retire, a channel free, and the window teardown.
            assert!(
                dev.retire_proc(pids[P_TEARDOWN]),
                "({mode:?}) the teardown proc was live when the script retired it"
            );
            dev.apply(RmEvent::Free {
                client: client_of(P_CHANFREE),
                handle: handles.gr_channel,
            })
            .expect("the channel free applies");
            // Wait for the prober to have been SERVED at least once (an edge, never a
            // sleep), then tear its window down from under it.
            started.wait();
            vmm.with(|v| v.declare_unbacked(GPA_REVOKE..GPA_REVOKE + 0x1000));

            let dw = t_dma_w.join().expect("the GPU0 DMA thread panicked");
            let dp = t_dma_p.join().expect("the GPU1 DMA thread panicked");
            let rp = t_revoke.join().expect("the revoke prober panicked");
            let mut published = 0usize;
            for h in peers {
                published += h.join().expect("a peer workload thread panicked");
            }
            let all_pending = latches.all_pending();
            latches.release_all();
            let canary = t_teardown.join().expect("the teardown canary panicked");
            let _ = t_w1.join();
            let _ = t_w2.join();
            let _ = t_chanfree.join();
            let _ = t_reroute.join();
            (dw, dp, rp, published, all_pending, canary)
        });

    let port_refusals = vmm.with(|v| {
        let non_ram = v
            .refused
            .iter()
            .filter(|r| r.err == VmmErrorKind::NonRamGpa)
            .count();
        (non_ram, v.refused.len() - non_ram)
    });
    let pt_of = |i: usize| {
        device
            .with_proc(pids[i], |p| {
                p.vases[&(gpu_of(i), lane_of(i).pdb)].pt_pages.len()
            })
            .expect("a live DMA proc")
    };
    DmaMeanReport {
        witness,
        peer,
        port_refusals,
        pt_pages: (pt_of(P_WITNESS), pt_of(P_PEER)),
        revoke,
        publications,
        latches_all_pending,
        canary_teardown,
        pid_teardown: pids[P_TEARDOWN],
        lock_depth_span: vmm.lock_depth_span(),
    }
}

// =================================================================================
// ★★★ THE GPA IS AN ADDRESS, NOT JUST A LEDGER ENTRY
// (`testing_doctrine.md` §1 "a green instrument on an unexercised path is worse than
// none" / §2 "assert the exact thing"; `mode2_address_table.md`)
//
// Measured finding (mutation round 2026-07-27): every mutant of `kayfabe_core::gpa`'s
// address ARITHMETIC survived the whole suite. `GpaSpace` appears at 74 test sites and
// the conservation ledger above already checks that GPA arenas are pairwise disjoint
// and that every published GPA lies inside its own proc's arena — but **conservation
// is invariant to address arithmetic**. An arena that hands out wrong-but-self-
// consistent addresses balances perfectly, stays inside its bounds, and passes all of
// it.
//
// In Mode 2 the GPA is the number that gets published into the guest's page tables. A
// wrong-but-consistent GPA is the guest reading the wrong memory *while our ledger
// reports perfect balance* — the silent-corruption class this project cares most about.
//
// So this run makes the address OBSERVABLE end-to-end: every publication is read back
// **through the address table**, its `phys` is proven against
// `kayfabe_vmm::GuestRamMap` — the production prove-RAM port, narrowed here to one
// declared RAM region **per arena slot** — and a per-publication tag is written into a
// shadow guest RAM and read back when the range is reclaimed. A GPA that drifts outside
// its arena FAULTS at the port; one that drifts into another proc's arena resolves to
// the WRONG REGION; one that aliases a live publication clobbers a tag. None of those
// three is visible to a conservation assertion.
//
// The arena is deliberately TINY (1 MiB) and the churn deliberately exceeds it many
// times over, so the free list is not merely reached but LOAD-BEARING: a reuse path
// that silently stopped reusing shows up as `FwdFault::Arena`, not as a slower test.
// =================================================================================

/// Base of the guest-physical window this run's device is realized with. NON-ZERO on
/// purpose: with a window based at 0, `cursor - range.start` and `cursor + range.start`
/// are the same number and half of the arena's arithmetic is untestable.
const GPA_WIN_BASE: u64 = 0x2_0000_0000;
/// Per-target window length (MG-6 mints one of these per GPU, back to back).
const GPA_WINDOW_LEN: u64 = 0x100_0000; // 16 MiB = 16 arena slots
/// ★ Per-proc arena: 1 MiB = 256 pages. Small enough that the churn below MUST reclaim.
const GPA_ARENA_LEN: u64 = 0x10_0000;
/// Publish/reclaim rounds per workload thread.
const GPA_ROUNDS: u32 = 48;
/// Publications per round, at mixed lengths (1–3 pages) — the fragmentation shape.
const GPA_PER_ROUND: u64 = 12;
/// Guest-VA lane for this run (clear of every other lane in this file).
const VA_GPA: u64 = 0xA0_0000_0000;
/// PDB base for the churn's throw-away VASpaces.
const GPA_PDB_BASE: u64 = 0x3800_0000;
/// Handle base for the churn's throw-away VASpaces.
const GPA_VAS_BASE: HObject = HObject(0x5c00_0400);
/// The client of the ★ post-reap process — the one handed a RECYCLED arena.
const GPA_HEIR_CLIENT: HClient = HClient(0xB0);
/// Its PDB / channel identities (distinct from every lane).
const GPA_HEIR_PDB: Pdb = Pdb(0x3900_0000);
const GPA_HEIR_GR: VChid = VChid(0x400);
const GPA_HEIR_CE: VChid = VChid(0x401);
/// Tag namespace for the heir's publications, clear of every workload thread's.
const HEIR_TAG: u64 = 0xE1_0000_0000_0000;

/// ★ The prove-RAM instrument: **one declared RAM region per arena slot**, across both
/// per-target windows, with the region id equal to the slot's own base.
///
/// That identity is what turns "the GPA is in the right place" into an EXACT equality
/// rather than a range test: a publication by proc P on GPU G must resolve to
/// `RamRegionId(P's own arena base on G)` at offset `gpa - that base`. A GPA that drifts
/// into the next arena resolves to a different region — it does not merely "still lie
/// inside the window".
///
/// Everything outside the two windows is left undeclared, so a GPA that leaves the
/// device's guest-physical window at all is `VmmError::BadGpa` — a FAULT, at the same
/// port `Vmm::gpa_read` proves every guest-steered address through.
fn arena_regions() -> GuestRamMap {
    let mut m = GuestRamMap::new();
    for w in 0..2u64 {
        for slot in 0..(GPA_WINDOW_LEN / GPA_ARENA_LEN) {
            let base = GPA_WIN_BASE + w * GPA_WINDOW_LEN + slot * GPA_ARENA_LEN;
            m.declare(RamRegionId(base), RegionKind::Ram, base, GPA_ARENA_LEN)
                .expect("an arena slot inside the 64-bit space");
        }
    }
    m
}

/// ★ The device-wide address auditor, shared by every workload thread.
///
/// Deliberately a **second, dumber accounting** than anything in the core: a flat map of
/// live guest-physical ranges plus a page-granular shadow of guest RAM. The core's own
/// ledger cannot play this role — it is the thing under test, and it balances whether or
/// not the addresses are right.
#[derive(Debug, Default)]
struct GpaAudit {
    /// `gpa -> (len, tag)` for every publication live anywhere on the device.
    live: BTreeMap<u64, (u64, u64)>,
    /// Shadow guest RAM at page granularity: `page -> the tag that owns it`.
    ram: BTreeMap<u64, u64>,
    /// Publications recorded (non-vacuity).
    issued: usize,
    /// Publications served from a range that had already been handed out and given back
    /// — i.e. from the free list. The run asserts this is large: a reuse path that
    /// silently stopped reusing must not pass as "still correct".
    recycled: usize,
    /// Every page ever issued, so `recycled` can be counted.
    seen: BTreeSet<u64>,
}

impl GpaAudit {
    /// Record one publication, asserting it overlaps NOTHING live anywhere on the
    /// device, and stamp its pages in the shadow RAM.
    fn publish(&mut self, gpa: u64, len: u64, tag: u64, what: &str) {
        let end = gpa
            .checked_add(len)
            .expect("a publication that does not wrap");
        if let Some((&ps, &(pl, pt))) = self.live.range(..=gpa).next_back() {
            assert!(
                ps + pl <= gpa,
                "{what}: GPA {gpa:#x}..{end:#x} overlaps the LIVE publication \
                 {ps:#x}..{:#x} (tag {pt:#x}) — one range issued to two live mappings",
                ps + pl,
            );
        }
        if let Some((&ns, &(nl, nt))) = self.live.range(gpa..).next() {
            assert!(
                end <= ns,
                "{what}: GPA {gpa:#x}..{end:#x} overlaps the LIVE publication \
                 {ns:#x}..{:#x} (tag {nt:#x})",
                ns + nl,
            );
        }
        assert_eq!(
            self.live.insert(gpa, (len, tag)),
            None,
            "{what}: GPA {gpa:#x} was issued twice while still live",
        );
        let mut fresh = false;
        // Only pages this publication owns WHOLLY are stamped: a sub-page length leaves
        // a tail page two publications legitimately share, and stamping it would make
        // the auditor report a clobber that never happened.
        for p in (gpa..(end / 0x1000) * 0x1000).step_by(0x1000) {
            self.ram.insert(p, tag);
            fresh |= self.seen.insert(p);
        }
        self.issued += 1;
        if !fresh {
            self.recycled += 1;
        }
    }

    /// Reclaim one publication, asserting every page of it still reads back **this**
    /// publication's tag. A GPA that aliased somebody else's live range shows up here as
    /// the clobber it is, at the byte the guest would have read.
    fn reclaim(&mut self, gpa: u64, len: u64, tag: u64, what: &str) {
        let end = (gpa + len) / 0x1000 * 0x1000;
        for p in (gpa..end).step_by(0x1000) {
            assert_eq!(
                self.ram.get(&p).copied(),
                Some(tag),
                "{what}: guest page {p:#x} of the publication at {gpa:#x} no longer \
                 holds its own tag {tag:#x} — another publication was handed an \
                 overlapping GPA and wrote through it",
            );
        }
        assert_eq!(
            self.live.remove(&gpa),
            Some((len, tag)),
            "{what}: reclaiming {gpa:#x}, which the auditor did not hold as live",
        );
    }
}

/// Realize the six-proc, two-GPU world with a **tiny** GPA arena, so the arena's
/// reclamation is load-bearing rather than incidental. Otherwise identical to
/// [`mean_gpu`] — same lanes, same identical-handle shape, same two targets.
fn gpa_world(mode: LockMode) -> (Guarded<Arc<SharedDevice>>, Vec<ProcId>, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(GPA_WIN_BASE..GPA_WIN_BASE + GPA_WINDOW_LEN, GPA_ARENA_LEN);
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
    }
    for ev in s.events {
        gpu.apply(ev).expect("the scenario applies cleanly");
    }
    assert_eq!(gpu.procs.len(), N_PROCS, "six distinct procs were derived");
    let pids: Vec<ProcId> = (0..N_PROCS)
        .map(|i| gpu.spine.by_pdb[&(gpu_of(i), lane_of(i).pdb)])
        .collect();
    let guarded = Guarded::new("l1_mean::gpa_world", gpu, recorder.clone())
        .map(|g| Arc::new(SharedDevice::new(g, mode)));
    (guarded, pids, recorder)
}

/// One publication as the auditor records it: `(gpa, len, tag)`.
type Publication = (u64, u64, u64);

/// ★ **The address-churn workload.** One thread; `slot` distinguishes the two threads
/// that share one `Proc` (and therefore share ONE `GpaArena` — the contention that
/// matters here).
///
/// Each round declares a throw-away VASpace, publishes [`GPA_PER_ROUND`] ranges of
/// mixed length into it, then frees the **previous** round's VASpace. Freeing one round
/// behind is what produces genuine fragmentation: the reclaimed ranges are interior
/// holes between ranges that are still live, so the next round's publications can only
/// be served by a free list that actually coalesces and actually re-issues.
///
/// Every publication is checked four ways, and only the first is conservation-shaped:
/// it must resolve back through the address table to the `phys` its own commit computed;
/// it must be page-aligned; it must prove RAM in **its own proc's arena region** at the
/// exact offset; and it must overlap nothing live anywhere on the device.
fn gpa_workload(
    dev: &SharedDevice,
    ram: &GuestRamMap,
    audit: &Mutex<GpaAudit>,
    pids: &[ProcId],
    i: usize,
    slot: u32,
) -> usize {
    let (gpu, client, pid) = (gpu_of(i), client_of(i), pids[i]);
    let arena = dev
        .with_proc(pid, |p| p.arenas[&gpu].range.clone())
        .expect("a live proc, with its arena materialized by the warm-up");
    assert_eq!(
        arena.end - arena.start,
        GPA_ARENA_LEN,
        "the world must hand this run the small arena it depends on",
    );
    // (vas handle, its publications) — freed one round behind.
    let mut carry: Option<(HObject, Vec<Publication>)> = None;
    let mut published = 0usize;

    for round in 0..GPA_ROUNDS {
        let pdb = Pdb(GPA_PDB_BASE
            + (i as u64) * 0x1_0000
            + u64::from(slot) * 0x100
            + u64::from(round % 2));
        let vas_h = HObject(GPA_VAS_BASE.0 + (i as u32) * 0x1000 + slot * 0x100 + round);
        dev.apply(RmEvent::Alloc {
            client,
            parent: H_DEVICE,
            handle: vas_h,
            class: mock_classes::VASPACE,
            facts: AllocFacts::default(),
        })
        .expect("the guest declares a throw-away VASpace");
        dev.apply(RmEvent::SetPageDir {
            client,
            vaspace: vas_h,
            pdb,
        })
        .expect("…and binds a page directory to it");

        let mut mine = Vec::new();
        for k in 0..GPA_PER_ROUND {
            // Mixed lengths: a same-size stream can be served by a free list that never
            // splits or coalesces anything, which is the easy half of the problem.
            //
            // ★ And every fourth one is NOT a page multiple. That is deliberate and it
            // is the only way this layer reaches `GpaArena::alloc`'s alignment-head
            // branch at all: `publish_backing` asks for a constant 0x1000 alignment, so
            // as long as every length is a page multiple every free range starts
            // page-aligned, the round-up is a no-op and the head branch is dead code
            // from the fwd plane's point of view. A sub-page length leaves a
            // MISALIGNED tail on the free list, and the next request has to round up
            // over it. `plan.len` is guest-supplied, so this is a shape a guest can
            // produce — and the branch stops being reachable-only-in-a-unit-test the
            // moment a big-page (64 KiB / 2 MiB) mapping path asks for a coarser align.
            let len = 0x1000 * (1 + (k + u64::from(round)) % 3)
                + if k.is_multiple_of(4) { 0x800 } else { 0 };
            let va = GpuVa(VA_GPA + u64::from(slot) * 0x100_0000 + k * 0x1_0000);
            let tag = ((i as u64) << 40) | (u64::from(slot) << 36) | (u64::from(round) << 16) | k;
            let what = format!("proc {i} slot {slot} round {round} k {k}");

            let p = dev
                .publish_backing(gpu, pdb, va, len)
                .unwrap_or_else(|e| panic!("{what}: publish refused: {e:?}"));

            // ---- read the publication back THROUGH THE ADDRESS TABLE, at an offset,
            // so the number under test is the one the guest's TLB would answer with.
            let (binding, off) = dev
                .resolve(gpu, pdb, GpuVa(va.0 + 0x40))
                .unwrap_or_else(|e| panic!("{what}: a just-published VA must resolve: {e:?}"));
            assert_eq!(
                (binding.phys, off),
                (p.gpa, 0x40),
                "{what}: the address table answers with a different phys than the \
                 commit computed",
            );
            let gpa = binding.phys;

            // ---- ALIGNMENT. `publish_backing` asks the arena for 0x1000; a misaligned
            // GPA goes straight into a guest page table.
            assert_eq!(gpa % 0x1000, 0, "{what}: GPA {gpa:#x} is not page-aligned");

            // ---- ★ PROVE-RAM, at the production port, against THIS proc's own region.
            let span = ram.resolve(gpa, len).unwrap_or_else(|e| {
                panic!(
                    "{what}: published GPA {gpa:#x}+{len:#x} is not \
                     provable guest RAM at all: {e:?}"
                )
            });
            assert_eq!(
                span,
                RamSpan {
                    region: RamRegionId(arena.start),
                    offset: gpa - arena.start,
                    len,
                },
                "{what}: the published GPA resolves to the wrong place — expected \
                 offset {:#x} inside this proc's own arena {arena:?}",
                gpa.wrapping_sub(arena.start),
            );

            audit
                .lock()
                .expect("the address auditor")
                .publish(gpa, len, tag, &what);
            mine.push((gpa, len, tag));
            published += 1;
        }

        // ---- reclaim the round BEFORE this one, so the holes it leaves are interior.
        if let Some((prev_h, prev)) = carry.take() {
            {
                let mut g = audit.lock().expect("the address auditor");
                for &(gpa, len, tag) in &prev {
                    g.reclaim(
                        gpa,
                        len,
                        tag,
                        &format!("proc {i} slot {slot} round {round} free"),
                    );
                }
            }
            dev.apply(RmEvent::Free {
                client,
                handle: prev_h,
            })
            .expect("the guest frees the VASpace and keeps running");
        }
        carry = Some((vas_h, mine));
    }

    // ★ Slot 1 drains; slot 0 deliberately leaves its LAST round's VASpace live, so the
    // shared arena is still FRAGMENTED when the run quiesces — the previous round's
    // ranges are holes underneath ranges that are still mapped. That is not tidiness:
    // `gpa_sweep`'s `live_bytes` check is vacuous on an arena whose free list is empty,
    // and a fully-drained arena's free list coalesces straight back into the cursor.
    // Measured — the `live_bytes` mutant survived this run until this line existed.
    if slot == 1
        && let Some((h, last)) = carry.take()
    {
        {
            let mut g = audit.lock().expect("the address auditor");
            for &(gpa, len, tag) in &last {
                g.reclaim(gpa, len, tag, &format!("proc {i} slot {slot} final free"));
            }
        }
        dev.apply(RmEvent::Free { client, handle: h })
            .expect("the final VASpace frees");
    }
    published
}

/// What the composed GPA run observed. Every field is script-determined, so both lock
/// modes must produce identical values.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GpaMeanReport {
    /// Publications per workload thread, in spawn order.
    published: Vec<usize>,
    /// `(issued, recycled)` from the device-wide auditor.
    audited: (usize, usize),
    /// Live publications still on the auditor's books when the threads joined.
    still_live: usize,
    /// Procs reaped by the mid-run teardown (must be exactly 1).
    reaped: usize,
    /// The arena range the reaped proc gave back, and the one its heir was handed.
    recycled_arena: (Range<u64>, Range<u64>),
    /// Publications made by the heir through its recycled arena.
    heir_published: usize,
    /// Bindings the end-of-run sweep audited, device-wide.
    swept_bindings: usize,
    /// Arenas the sweep found still FRAGMENTED — i.e. with a genuinely non-empty free
    /// list, which is what makes its `live_bytes` check non-vacuous.
    fragmented_arenas: usize,
    /// Whether the parked verbs were still parked when the workloads finished.
    latches_all_pending: bool,
}

impl GpaMeanReport {
    /// The report with its one genuinely racy field normalised away, for the
    /// lock-mode differential — the same discipline [`DmaMeanReport::mode_independent`]
    /// applies, and for the same reason.
    ///
    /// `audited.1` counts publications served from a page that had been handed out
    /// before. Two threads share one `GpaArena`, so **which** page a request lands on
    /// depends on the order the two threads reached the allocator — a race, not a
    /// script. It is asserted as a lower BOUND (the free list must be load-bearing) and
    /// must never appear in a differential. Found by a bite-check, not by design: three
    /// mutants "died" here on a mode difference in this number rather than on any
    /// property, which is a kill for the wrong reason and a latent flake.
    fn mode_independent(mut self) -> Self {
        self.audited.1 = 0;
        self
    }
}

/// ★ **The end-of-run sweep: every live binding on the device, audited by address.**
///
/// Complements the per-operation checks in [`gpa_workload`] — it covers publications the
/// workload threads never made (the warm-up, the heir, the held verbs) and it is where
/// `GpaArena::live_bytes` is checked against a **second accounting**: the summed lengths
/// of the `GpaBlock` tokens the proc still holds. An accounting function compared only
/// with itself is not compared at all.
fn gpa_sweep(dev: &SharedDevice, ram: &GuestRamMap, mode: LockMode) -> (usize, usize) {
    let mut all: Vec<(ProcId, GpuId, u64, u64)> = Vec::new();
    let mut fragmented = 0usize;
    for pid in dev.live_pids() {
        dev.with_proc(pid, |p| {
            for (&g, arena) in &p.arenas {
                // ---- the second accounting: live_bytes vs the tokens actually held.
                let blocks: Vec<(u64, u64)> = p
                    .vases
                    .iter()
                    .filter(|((vg, _), _)| *vg == g)
                    .flat_map(|(_, v)| v.blocks.values())
                    .map(|b| (b.gpa.0, b.len))
                    .collect();
                let held: u64 = blocks.iter().map(|&(_, l)| l).sum();
                assert_eq!(
                    arena.live_bytes(),
                    held,
                    "({mode:?}) {pid:?}/{g:?}: live_bytes disagrees with the summed \
                     lengths of the GpaBlocks the proc still holds",
                );
                // ★ NON-VACUITY for the check immediately above, and it is not
                // decoration: `live_bytes` is `(cursor - base) - sum(free list)`, so on
                // an arena whose free list happens to be EMPTY the free-list term is
                // zero and the whole comparison cannot see it. Measured, not reasoned:
                // a `- → +` mutant of that subtraction survived this sweep until the
                // churn was made to leave one arena fragmented on purpose. A hole below
                // the highest live block IS a free-list entry, so this derives the
                // free list's non-emptiness from state the test can actually read.
                if let Some(top) = blocks.iter().map(|&(s, l)| s + l).max()
                    && top - arena.range.start > held
                {
                    fragmented += 1;
                }
            }
            for (&(g, _pdb), vas) in &p.vases {
                let arena = p
                    .arenas
                    .get(&g)
                    .unwrap_or_else(|| panic!("({mode:?}) a Vas with no arena"))
                    .range
                    .clone();
                for (_va, len, b) in vas.table.iter() {
                    if b.host.is_none() {
                        continue; // declared by RPC only — nothing was allocated for it
                    }
                    assert_eq!(
                        ram.resolve(b.phys, len),
                        Ok(RamSpan {
                            region: RamRegionId(arena.start),
                            offset: b.phys - arena.start,
                            len,
                        }),
                        "({mode:?}) {pid:?}/{g:?} holds a binding at {:#x}+{len:#x} that \
                         does not prove out as RAM inside its OWN arena {arena:?}",
                        b.phys,
                    );
                    all.push((pid, g, b.phys, len));
                }
            }
        });
    }
    all.sort_unstable_by_key(|&(_, _, p, _)| p);
    for w in all.windows(2) {
        let (pa, ga, sa, la) = w[0];
        let (pb, gb, sb, _) = w[1];
        assert!(
            sa + la <= sb,
            "({mode:?}) two live bindings overlap: {pa:?}/{ga:?} {sa:#x}+{la:#x} vs \
             {pb:?}/{gb:?} {sb:#x} — the #14 collision class, inside one window",
        );
    }
    (all.len(), fragmented)
}

fn gpa_mean_run(mode: LockMode) -> GpaMeanReport {
    let (mut device, pids, rec) = gpa_world(mode);
    // The mid-run teardown kills a proc out of band, so its isolate is stopped and its
    // staged release cannot drain — §7.0's process-boundary backstop, exactly as
    // `mean_run` declares it.
    device.declare_residue(
        ResidueClaim::on(
            IsolateId::new(pids[P_TEARDOWN].0, gpu_of(P_TEARDOWN)),
            "the proc retired out of band mid-run: its isolate is stopped, so its host \
             VAS + warm-up backing are the §7.0 namespace-death residue",
        )
        .objects(VerbKind::AllocVaSpace, 1)
        .objects(VerbKind::AllocSysmem, 1)
        .maps(1),
    );
    let ram = arena_regions();
    let audit = Mutex::new(GpaAudit::default());
    let dev: &SharedDevice = &device;
    let pid_ref: &[ProcId] = &pids;

    // ---- Warm-up: every proc materializes its host VAS and one binding, so each
    // arena exists (the workload reads its base) and a parked verb is the one the
    // script names rather than an incidental first touch.
    for i in 0..N_PROCS {
        dev.publish_backing(gpu_of(i), lane_of(i).pdb, GpuVa(VA_WARM), 0x1000)
            .expect("warm-up publish");
    }
    let doomed_arena = dev
        .with_proc(pids[P_TEARDOWN], |p| p.arenas[&GPU0].range.clone())
        .expect("the doomed proc's arena");

    let (published, reaped, heir_arena, heir_published, latches_all_pending) =
        thread::scope(|sc| {
            // ---- park two host verbs, on two isolates, on both GPUs. Neither is on a
            // proc this script retires: a reap requires a quiesced isolate (G3), and
            // the point here is the RECYCLE, not the deferral.
            let mut latches = Latches::new();
            latches.arm(&rec, pids[P_CHANFREE], GPU1, 0, VerbKind::AllocChannel);
            latches.arm(&rec, pids[P_REROUTE], GPU0, 0, VerbKind::AllocChannel);
            let t_hold1 = sc.spawn(move || {
                dev.doorbell(GPU1, MockArch::token_for(lane_of(P_CHANFREE).gr), &[])
            });
            let t_hold2 = sc
                .spawn(move || dev.doorbell(GPU0, MockArch::token_for(lane_of(P_REROUTE).gr), &[]));
            latches.wait_all_pending(); // a progress EDGE, never a sleep

            // ---- FOUR address-churn threads: two per proc (one arena, two threads) on
            // two procs (one per GPU).
            let ra = &ram;
            let au = &audit;
            let mut workers = Vec::new();
            for i in [P_WITNESS, P_PEER] {
                for slot in 0..2u32 {
                    workers.push(sc.spawn(move || gpa_workload(dev, ra, au, pid_ref, i, slot)));
                }
            }

            // ---- ★ THE WHOLE-ARENA LIFO RECYCLE, while the churn threads allocate.
            // A proc dies out of band; its arena goes back to its target's window; a
            // brand-new guest process is then handed that very range.
            assert!(
                dev.retire_proc(pids[P_TEARDOWN]),
                "({mode:?}) the doomed proc was live when the script retired it"
            );
            let reaped = dev.reap_retired();
            let mut s = Scenario::new();
            s.compute_process_on_gpu(
                GPA_HEIR_CLIENT,
                GPA_HEIR_PDB,
                identical_handles(GPA_HEIR_GR.0, GPA_HEIR_CE.0),
                None,
            );
            for ev in s.events {
                dev.apply(ev).expect("({mode:?}) the heir process declares");
            }
            let heir = dev
                .with_proc(
                    dev.live_pids()
                        .into_iter()
                        .max()
                        .expect("the heir is the newest proc"),
                    |p| (p.id, p.arenas[&GPU0].range.clone()),
                )
                .expect("the heir");
            let mut heir_published = 0usize;
            for k in 0..16u64 {
                let va = GpuVa(VA_GPA + 0x8000_0000 + k * 0x1_0000);
                let p = dev
                    .publish_backing(GPU0, GPA_HEIR_PDB, va, 0x1000)
                    .expect("({mode:?}) the heir publishes through its recycled arena");
                let span = ram.resolve(p.gpa, 0x1000).unwrap_or_else(|e| {
                    panic!(
                        "({mode:?}) the heir's GPA {:#x} is not provable RAM: {e:?}",
                        p.gpa
                    )
                });
                assert_eq!(
                    span,
                    RamSpan {
                        region: RamRegionId(heir.1.start),
                        offset: p.gpa - heir.1.start,
                        len: 0x1000,
                    },
                    "({mode:?}) the heir published outside its own recycled arena",
                );
                audit
                    .lock()
                    .expect("auditor")
                    .publish(p.gpa, 0x1000, HEIR_TAG | k, "heir");
                heir_published += 1;
            }

            let published: Vec<usize> = workers
                .into_iter()
                .map(|h| h.join().expect("an address-churn thread panicked"))
                .collect();
            let all_pending = latches.all_pending();
            latches.release_all();
            let _ = t_hold1.join();
            let _ = t_hold2.join();
            (published, reaped, heir.1, heir_published, all_pending)
        });

    // Everything the threads staged for release is disposed of at the real quiesce
    // point (the T0 drain), so the teardown ledger measures leaks and not backlog.
    for _ in 0..4 {
        dev.drain_pending_releases();
    }
    let (swept_bindings, fragmented_arenas) = gpa_sweep(dev, &ram, mode);
    let g = audit.lock().expect("auditor");
    GpaMeanReport {
        published,
        audited: (g.issued, g.recycled),
        still_live: g.live.len(),
        reaped,
        recycled_arena: (doomed_arena, heir_arena),
        heir_published,
        swept_bindings,
        fragmented_arenas,
        latches_all_pending,
    }
}

/// ★★★ **THE COMPOSED ADDRESS-OBSERVABILITY RUN.**
///
/// Four address-churn threads (two per `Proc`, so two threads share ONE `GpaArena`) on
/// two procs on two GPUs, each cycling declare-VASpace → publish → free-VASpace one
/// round behind so the arena is permanently fragmented; a **1 MiB** arena serving many
/// times its own size, so the free list is load-bearing and a reuse path that stopped
/// reusing is a `FwdFault::Arena` and not a slower run; two host verbs parked on two
/// isolates across both GPUs the whole time; and — in the middle of all of it — a proc
/// **retired and reaped**, its whole arena recycled LIFO into its target's window, and a
/// brand-new guest process handed that very range while the churn threads keep
/// allocating. Under **both** lock configurations.
///
/// Every publication is read back through the address table and its `phys` proven at the
/// production prove-RAM port against its own proc's arena region. The end-of-run sweep
/// then audits every live binding on the device the same way, and checks
/// `GpaArena::live_bytes` against the summed lengths of the `GpaBlock` tokens the procs
/// actually hold.
#[test]
fn a_published_gpa_is_provably_its_own_procs_ram_under_mean_arena_churn() {
    let _wd = watchdog(
        "a_published_gpa_is_provably_its_own_procs_ram_under_mean_arena_churn",
        Duration::from_secs(300),
    );
    let degenerate = gpa_mean_run(LockMode::Degenerate);
    let sharded = gpa_mean_run(LockMode::Sharded);

    let per_thread = (GPA_ROUNDS as usize) * (GPA_PER_ROUND as usize);
    for (name, r) in [("Degenerate", &degenerate), ("Sharded", &sharded)] {
        // ---- every workload thread ran to completion. With reclamation broken this is
        // where the run dies: 1 MiB of arena cannot serve this stream by bumping.
        assert_eq!(
            r.published,
            vec![per_thread; 4],
            "({name}) an address-churn thread did not complete its whole stream",
        );
        // ---- the auditor saw every publication, and the arena really did RECYCLE.
        // Without this bound the whole run would still pass on an allocator that leaked
        // every freed range and simply bumped — the instrument would be green on a path
        // it never took (`testing_doctrine.md` §1).
        assert_eq!(
            r.audited.0,
            4 * per_thread + r.heir_published,
            "({name}) the auditor did not see every publication",
        );
        assert!(
            r.audited.1 > 3 * per_thread,
            "({name}) only {} of {} publications came out of a recycled page — the free \
             list this run exists to exercise was barely reached",
            r.audited.1,
            r.audited.0,
        );
        // ---- nothing was left on the auditor's books: every range the churn published
        // was reclaimed with its own tag still intact.
        assert_eq!(
            r.still_live,
            r.heir_published + 2 * (GPA_PER_ROUND as usize),
            "({name}) exactly the heir's publications plus the two slot-0 threads' \
             deliberately-retained final round must still be live — every other range \
             the churn published was reclaimed, with its own tag intact",
        );
        // ★ the sweep's `live_bytes` check was NOT vacuous: at least the two churn
        // procs' arenas still had a non-empty free list when it ran.
        assert!(
            r.fragmented_arenas >= 2,
            "({name}) no arena was still fragmented at the sweep, so its `live_bytes` \
             audit could not see the free-list term at all",
        );
        // ---- ★ the whole-arena LIFO recycle really happened, and it is the SAME range.
        assert_eq!(
            r.reaped, 1,
            "({name}) the retired proc was not reaped — the recycle never occurred"
        );
        assert_eq!(
            r.recycled_arena.1, r.recycled_arena.0,
            "({name}) the heir process was not handed the reaped proc's arena, so the \
             #80 LIFO recycle was never exercised under load",
        );
        assert_eq!(
            r.heir_published, 16,
            "({name}) the heir did not publish through its recycled arena",
        );
        // ---- and the device-wide sweep audited real state, not an empty set.
        assert!(
            r.swept_bindings >= N_PROCS,
            "({name}) the end-of-run sweep audited only {} bindings",
            r.swept_bindings,
        );
        assert!(
            r.latches_all_pending,
            "({name}) the parked verbs were released before the workloads finished — \
             the churn did not actually run against held host verbs",
        );
    }
    assert_eq!(
        degenerate.mode_independent(),
        sharded.mode_independent(),
        "the lock configuration is observable through the GPA plane",
    );
}

// =================================================================================
// ★★★ THE REAL MEMORY PLANE — `l1_os_shell.md` §6.1/§6.2/§6.7, stage M2-c
//
// Everything above this line drives `MockVmm`, whose `map_guest` is a
// `BTreeMap::insert` that CANNOT BLOCK. §6.2 records the consequence as gap **O1**:
// *"nothing in the suite can distinguish a memcpy from a KVM_SET_USER_MEMORY_REGION."*
// So R1 — no blocking call under ANY lock — has never actually been tested; it has
// only ever been true of a harness that had no blocking calls in it.
//
// What follows is the same core, unmodified, driven through `kayfabe_vmm_kvm::KvmVmm`:
// a real /dev/kvm descriptor, real memslots, real mmap'd windows, real MAP_FIXED
// placements, and real kernel refusals (EEXIST, ENOMEM) happening WHILE the traffic
// runs. `SharedDevice::parse_pushbuffer` does not know which backend is underneath —
// which is the portability contract used rather than asserted.
// =================================================================================

/// Proc-0's guest-RAM window. 64 host pages: the lower half carries scripted
/// pushbuffers, the upper half is churned with placements, and the two never overlap
/// (a `MAP_FIXED` over a scripted ring would replace it with zeroes, and the test would
/// then be about the wrong thing).
const RGPA_A: u64 = 0x1000_0000;
/// Proc-1's window, on the other GPU.
const RGPA_B: u64 = 0x2000_0000;
/// The window torn down while a thread is parsing out of it.
const RGPA_REVOKE: u64 = 0x3000_0000;
/// **Our own trapped BAR0** — declared a device region, which on KVM means it has no
/// memslot at all and a guest access to it exits instead of landing anywhere.
const RGPA_BAR0: u64 = 0x7000_0000;
/// Never declared. On a real backend the region map starts EMPTY, so this is a hole by
/// construction rather than by declaration — the opposite of the mock's all-RAM default.
const RGPA_HOLE: u64 = 0x6000_0000;
/// Window size in pages.
const RWIN_PAGES: u64 = 64;
/// Submits per real-DMA thread. Four hostile shapes cycle, so a multiple of four keeps
/// the arm counts exact rather than approximately balanced.
const RDMA_OPS: usize = 48;
/// Publish/unpublish cycles per churn thread.
const RPUB_OPS: usize = 40;
/// Bounded budget for the revoke prober (an EDGE loop, never a sleep).
const RREVOKE_BUDGET: usize = 50_000;
/// Real host refusals attempted while the traffic runs.
const RREFUSE_OPS: usize = 30;

fn host_page() -> u64 {
    kayfabe_linux_raw::HostPageSize::query().bytes()
}

/// What one real-DMA thread observed. Every field is script-determined, so both lock
/// modes and both backends must produce identical values.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RealDmaTally {
    legal_ok: usize,
    non_ram: usize,
    unbacked: usize,
}

/// What the churn thread observed. `placed`/`restored` are script-determined; the
/// refusal split is not (it depends on when the main thread's teardown lands), so it is
/// asserted as a **conservation identity** rather than as a value.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ChurnTally {
    placed: usize,
    restored: usize,
    refused: usize,
}

/// Realize a machine whose guest-physical layout is the mean run's.
fn real_machine() -> kayfabe_vmm_kvm::KvmMachine {
    kayfabe_vmm_kvm::KvmMachine::realize(kayfabe_vmm_kvm::MachineConfig {
        shareable_ram: true,
        bars: vec![kayfabe_vmm_kvm::BarPlacement {
            bar: kayfabe_vmm::BarId::Bar0,
            base: RGPA_BAR0,
            len: 0x1_0000,
        }],
    })
    .expect(
        "/dev/kvm must be present and permitted to run the real-Vmm mean run \
         (l1_os_shell.md §10, decision #48). This is a deployment fact no code gate can \
         observe, and a silently-skipped OS test is the green instrument \
         testing_doctrine.md §1 forbids",
    )
}

/// **The real DMA-descriptor workload.** Identical in shape to [`dma_workload`] above,
/// and deliberately so: the same guest, the same four hostile shapes, the same exact
/// faults — but every guest-memory access is a copy through a real `mmap`ed window, and
/// every refusal is the real region map saying that guest-physical address has no
/// memslot behind it.
fn real_dma_workload(
    dev: &SharedDevice,
    vmm: &kayfabe_vmm_kvm::KvmVmm,
    pids: &[ProcId],
    i: usize,
    base: u64,
) -> RealDmaTally {
    let (lane, gpu, pid) = (lane_of(i), gpu_of(i), pids[i]);
    let mut vmm = vmm.clone();
    let page = host_page();
    let cid = dev
        .doorbell(gpu, MockArch::token_for(lane.ce), &[])
        .expect("the DMA thread's CE channel rings")
        .chan;
    let mut t = RealDmaTally::default();

    for k in 0..RDMA_OPS {
        // ---- the LEGITIMATE submit, scripted into REAL guest memory.
        let pt_page = VA_PT + (i as u64) * 0x100_0000 + (k as u64) * 0x1000;
        let ring = kayfabe_tests::script_ring_via(
            &mut vmm,
            base + ((k % 16) as u64) * 0x100,
            &[
                MockPushbuffer::set_object(mock_classes::DMA_COPY),
                MockPushbuffer::ce_launch_dma(pt_page, 0x1000, false),
                MockPushbuffer::tlb_invalidate(lane.pdb.0, true),
                MockPushbuffer::method(0xEE, &[1, 2, 3]),
            ],
        );
        let out = dev
            .parse_pushbuffer(&mut vmm, pid, cid, &ring)
            .expect("a RAM-backed pushbuffer parses out of a real mmap'd window");
        assert_eq!(
            out.pt_writes,
            vec![pt_page],
            "the legitimate submit's CE PT-write must be captured — non-vacuity for every \
             refusal below"
        );
        t.legal_ok += 1;

        // ---- the four HOSTILE shapes, each by exact variant AND payload.
        let (ring, expect) = match k % 4 {
            // (a) squarely at our own trapped BAR0 — a region with NO MEMSLOT.
            0 => (
                gpfifo_ring(RGPA_BAR0, 0x40),
                Err(FwdFault::NonRamGpa { gpa: RGPA_BAR0 }),
            ),
            // (b) deeper into the same device window.
            1 => (
                gpfifo_ring(RGPA_BAR0 + 0x800, 8),
                Err(FwdFault::NonRamGpa {
                    gpa: RGPA_BAR0 + 0x800,
                }),
            ),
            // (c) a hole — the NEAR NEIGHBOUR, which must report differently.
            2 => (
                gpfifo_ring(RGPA_HOLE + 0x10, 0x20),
                Err(FwdFault::GpaRead {
                    gpa: RGPA_HOLE + 0x10,
                }),
            ),
            // (d) STRADDLING: starts in real, memslot-backed RAM and runs off the end
            //     of the window. A start-address-only check would memcpy the first bytes
            //     out of a live mapping and then fault, or worse, read whatever the next
            //     mapping happened to be.
            //
            // ★★ **F6, FOUND HERE AND NOW FIXED** (`l1_os_shell.md` §14.8). The **port**
            // names the BOUNDARY byte for this shape — `GuestRamMap::resolve` returns
            // `BadGpa { gpa: <end of the window> }`, and the focused test
            // `a_device_region_is_the_absence_of_a_memslot_and_refuses_by_name` in
            // `kayfabe-vmm-kvm` asserts exactly that. `kayfabe_fwd::guest_read` used to
            // classify it through a catch-all `_ => FwdFault::GpaRead { gpa }`, taking
            // the **requested** address from its own argument rather than the one the
            // port reported — so the boundary was preserved on the `NonRamGpa` arm and
            // discarded on its near neighbour. Invisible until this run, because
            // §12.43's straddle test uses a RAM→DEVICE range, i.e. the arm that keeps
            // it. The named one-liner (`VmmError::BadGpa { gpa } => FwdFault::GpaRead
            // { gpa }`) landed 2026-07-27, and this row now asserts the BOUNDARY — over
            // a REAL mmap'd window and a real KVM memslot, which is the half the mock
            // cannot reach.
            _ => (
                gpfifo_ring(base + RWIN_PAGES * page - 8, 0x40),
                Err(FwdFault::GpaRead {
                    gpa: base + RWIN_PAGES * page,
                }),
            ),
        };
        let got = dev.parse_pushbuffer(&mut vmm, pid, cid, &ring);
        assert_eq!(
            got, expect,
            "a guest-steered descriptor must refuse by EXACT name against the REAL \
             backend too (k={k}, proc={pid:?})"
        );
        match expect {
            Err(FwdFault::NonRamGpa { .. }) => t.non_ram += 1,
            _ => t.unbacked += 1,
        }
        assert_eq!(
            lock::held_depth(),
            0,
            "a refused guest-memory read leaked a ranked guard"
        );
    }
    t
}

/// **The map/unmap churn**: real `MAP_FIXED` placements into the upper half of a live
/// window, and real restores back to anonymous — the fine tier, at publication
/// frequency, while everything else runs.
fn real_churn_workload(
    machine: &kayfabe_vmm_kvm::KvmMachine,
    vmm: &kayfabe_vmm_kvm::KvmVmm,
    base: u64,
    backing: kayfabe_vmm::HostRegion,
) -> ChurnTally {
    let mut vmm = vmm.clone();
    let page = host_page();
    let mut t = ChurnTally::default();
    let _ = machine;
    for k in 0..RPUB_OPS {
        let at = base + (RWIN_PAGES / 2 + (k as u64 % (RWIN_PAGES / 2))) * page;
        match vmm.map_guest(at, page, backing, kayfabe_vmm::Prot::ReadWrite) {
            Ok(slot) => {
                t.placed += 1;
                // A placement is guest-visible memory: write through it and read it back,
                // so the test is about a mapping and not about a bookkeeping entry.
                vmm.gpa_write(at, &(k as u64).to_le_bytes())
                    .expect("a freshly placed backing is writable");
                let mut got = [0u8; 8];
                vmm.gpa_read(at, &mut got).expect("and readable");
                assert_eq!(
                    u64::from_le_bytes(got),
                    k as u64,
                    "a MAP_FIXED placement must carry its own bytes"
                );
                vmm.unmap_guest(slot).expect("restore");
                t.restored += 1;
                // ★ §6.7 item 3: a restore is `mmap(MAP_FIXED|MAP_ANONYMOUS)`, NEVER a
                // `munmap` — so the range must still RESOLVE afterwards, reading zeroes.
                // A hole here would leave the live memslot pointing at a gap.
                vmm.gpa_read(at, &mut got)
                    .expect("a restored range is still backed by the window");
                assert_eq!(
                    got, [0u8; 8],
                    "a restored range reads as anonymous zeroes, not as the object that \
                     used to be there"
                );
            }
            Err(e) => {
                t.refused += 1;
                assert!(
                    matches!(
                        e,
                        kayfabe_vmm::VmmError::BadGpa { .. }
                            | kayfabe_vmm::VmmError::Unsupported(_)
                    ),
                    "a churn refusal must be a named plan/R5 refusal, never a host fault \
                     the adapter did not expect: {e:?}"
                );
            }
        }
    }
    t
}

/// What the churn thread aimed at the **doomed** window observed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct RevokeChurn {
    /// Placements that succeeded before the window was torn down (must be > 0, or the
    /// race never happened).
    placed: usize,
    /// Placements this thread restored itself, its window still alive.
    restored: usize,
    /// ★ Placements whose **restore** lost the race — the window was removed between the
    /// placement's commit and its restore, so the slot went with the window. Contract
    /// item 2, one iteration narrower than `held_slot_after_teardown` below; counted
    /// rather than tolerated, so `placed` still has to balance.
    lost_with_the_window: usize,
    /// The first refusal after it, exactly.
    first_refusal: Option<kayfabe_vmm::VmmError>,
    /// Placements that succeeded AFTER the first refusal — must be 0.
    placed_after_refusal: usize,
    /// What `unmap_guest` said about the placement that was still LIVE when its window
    /// was removed.
    held_slot_after_teardown: Option<kayfabe_vmm::VmmError>,
    /// The id of that placement, so the assertion above names the exact slot rather than
    /// matching any `BadSlot`.
    held_slot_id: u64,
}

/// ★★ **A teardown racing a live mapping**, in its sharpest form: this thread publishes
/// into the window the main thread is about to delete, and deliberately leaves **one
/// placement live** across the deletion.
///
/// Three things have to be true afterwards and none of them is obvious:
/// 1. a placement into a removed window is refused by a **named** error, never served;
/// 2. the live placement's slot is gone with its window — `unmap_guest` on it is
///    [`kayfabe_vmm::VmmError::BadSlot`], not a `restore` into freed address space;
/// 3. the conservation ledger still balances, i.e. the window's removal accounted for
///    the placement nobody unmapped.
fn real_revoke_churn(
    vmm: &kayfabe_vmm_kvm::KvmVmm,
    backing: kayfabe_vmm::HostRegion,
    gate: &StartGate,
) -> RevokeChurn {
    let _open = OpenOnDrop(gate);
    let mut vmm = vmm.clone();
    let page = host_page();
    let mut t = RevokeChurn::default();
    // The placement deliberately left live across the teardown.
    let held = vmm
        .map_guest(RGPA_REVOKE, page, backing, kayfabe_vmm::Prot::ReadWrite)
        .expect("the doomed window is live when the churn starts");
    t.placed += 1;
    gate.open();

    for _ in 0..RREVOKE_BUDGET {
        match vmm.map_guest(
            RGPA_REVOKE + 2 * page,
            page,
            backing,
            kayfabe_vmm::Prot::ReadWrite,
        ) {
            Ok(slot) => {
                if t.first_refusal.is_some() {
                    t.placed_after_refusal += 1;
                    break;
                }
                t.placed += 1;
                match vmm.unmap_guest(slot) {
                    Ok(()) => t.restored += 1,
                    // ★ The window was removed in the gap between this placement's COMMIT
                    // and its restore. That is contract item 2 exactly — the slot goes
                    // WITH its window — so the only permitted outcome is the NAMED
                    // `BadSlot` for THIS slot. Asserted by exact variant, never
                    // `is_err()`, and counted so the identity below still has to balance.
                    // (An earlier `.expect("restore")` here asserted a postcondition this
                    // function's own doc comment contradicts, and failed ~1 run in 6.)
                    Err(e) => {
                        assert_eq!(
                            e,
                            kayfabe_vmm::VmmError::BadSlot(slot),
                            "a restore that loses its race with the teardown must be the \
                             named BadSlot for that slot — anything else would be a \
                             `restore` into address space the adapter no longer owns"
                        );
                        t.lost_with_the_window += 1;
                    }
                }
            }
            Err(e) => {
                if t.first_refusal.is_none() {
                    t.first_refusal = Some(e);
                    continue; // one more round trip, to prove it STAYS refused
                }
                break;
            }
        }
    }
    t.held_slot_id = held.0;
    t.held_slot_after_teardown = vmm.unmap_guest(held).err();
    t
}

/// What the whole real-backend run observed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RealMeanReport {
    witness: RealDmaTally,
    peer: RealDmaTally,
    churn_a: ChurnTally,
    churn_b: ChurnTally,
    revoke: RevokeProbe,
    revoke_churn: RevokeChurn,
    refusals_eexist: usize,
    refusals_enomem: usize,
    audit: kayfabe_vmm_kvm::AuditReport,
    map_regions_checked: usize,
}

impl RealMeanReport {
    /// The report with its genuinely racy fields normalised away, for the cross-mode
    /// differential. `revoke.served` is a race between two live threads (a clock, which
    /// §3 rule 3 forbids in an assertion) and the churn refusal split depends on when the
    /// teardown lands. Both are asserted as **bounds and identities** above; neither may
    /// appear in an equality.
    fn mode_independent(mut self) -> Self {
        self.revoke.served = 0;
        self.revoke.refusals = 0;
        self.churn_a.refused = 0;
        self.churn_b.refused = 0;
        self.churn_a.placed = 0;
        self.churn_a.restored = 0;
        self.churn_b.placed = 0;
        self.churn_b.restored = 0;
        self.revoke_churn.placed = 0;
        self.revoke_churn.restored = 0;
        self.revoke_churn.lost_with_the_window = 0;
        // The slot ID depends on how many slots the racing threads minted first, which is
        // a clock. It is asserted EXACTLY above, against `held_slot_id`.
        self.revoke_churn.held_slot_id = 0;
        self.revoke_churn.held_slot_after_teardown = None;
        self.audit = kayfabe_vmm_kvm::AuditReport {
            live_placements: 0,
            peak_placements: 0,
            placements_made: 0,
            accesses_served: 0,
            accesses_refused: 0,
            r5_revalidation_failures: 0,
            // ★ Whether a window's release had to be DEFERRED depends on whether an
            // accessor happened to be mid-copy when the teardown landed — a clock, and
            // the sharpest one in this run. It is asserted as a bound below; the
            // *number* of mappings released is the mode-independent half and stays.
            window_releases_deferred: 0,
            ..self.audit
        };
        self
    }
}

fn real_mean_run(mode: LockMode) -> RealMeanReport {
    let (device, pids, rec) = mean_world(mode);
    let machine = real_machine();
    let page = host_page();
    let vmm = machine.vmm();
    let dev: &SharedDevice = &device;
    let pid_ref: &[ProcId] = &pids;
    let handles = identical_handles(0, 0);

    // ---- realize the guest-physical layout: three RAM windows, one device BAR (already
    // declared at realize), and a hole that is simply everything we never declared.
    let win_a = machine
        .install_ram_window(RGPA_A, RWIN_PAGES * page)
        .expect("proc-0's window");
    let win_b = machine
        .install_ram_window(RGPA_B, RWIN_PAGES * page)
        .expect("proc-1's window");
    let win_revoke = machine
        .install_ram_window(RGPA_REVOKE, 4 * page)
        .expect("the window that gets torn down mid-flight");
    let backing_a = machine.register_backing(page).expect("a host backing");
    let backing_b = machine.register_backing(page).expect("a host backing");
    let backing_r = machine.register_backing(page).expect("a host backing");
    assert_eq!(
        machine.assert_map_matches_the_kernel(),
        3,
        "★ every RAM region the map resolves is a LIVE MEMSLOT, and there are three of \
         them — the consistency the mock cannot express, asserted before the run starts"
    );

    // Warm-up, exactly as the mock mean run does.
    for i in 0..N_PROCS {
        dev.publish_backing(gpu_of(i), lane_of(i).pdb, GpuVa(VA_WARM), 0x1000)
            .expect("warm-up publish");
        dev.doorbell(gpu_of(i), MockArch::token_for(lane_of(i).ce), &[])
            .expect("warm-up CE ring");
    }

    let started = StartGate::default();
    let churn_started = StartGate::default();
    let (witness, peer, churn_a, churn_b, revoke, revoke_churn, eexist, enomem) =
        thread::scope(|sc| {
            // ---- park host verbs across both GPUs, so every memory-plane op below runs
            // while real host work is outstanding.
            let mut latches = Latches::new();
            latches.arm(&rec, pids[P_WITNESS], GPU0, 0, VerbKind::AllocSysmem);
            let t_held = sc.spawn(move || {
                dev.publish_backing(GPU0, lane_of(P_WITNESS).pdb, GpuVa(VA_HELD), 0x1000)
            });
            latches.arm(&rec, pids[P_PEER], GPU1, 0, VerbKind::AllocChannel);
            let t_held2 =
                sc.spawn(move || dev.doorbell(GPU1, MockArch::token_for(lane_of(P_PEER).gr), &[]));
            latches.wait_all_pending();

            // ---- the DMA workloads: one per GPU, through the real backend.
            let vw = &vmm;
            let t_dma_a = sc.spawn(move || real_dma_workload(dev, vw, pid_ref, P_WITNESS, RGPA_A));
            let t_dma_b = sc.spawn(move || real_dma_workload(dev, vw, pid_ref, P_PEER, RGPA_B));

            // ---- the churn: real MAP_FIXED placements and restores, both windows at once.
            let m = &machine;
            let t_churn_a = sc.spawn(move || real_churn_workload(m, vw, RGPA_A, backing_a));
            let t_churn_b = sc.spawn(move || real_churn_workload(m, vw, RGPA_B, backing_b));
            // ★ the churn that RACES the teardown, aimed at the doomed window.
            let cg = &churn_started;
            let t_churn_r = sc.spawn(move || real_revoke_churn(vw, backing_r, cg));

            // ---- the peers do real publish/doorbell work the whole time.
            let mut peers = Vec::new();
            for i in [P_WITNESS, P_PEER] {
                peers.push(sc.spawn(move || ctl_workload(dev, pid_ref, i)));
                peers.push(sc.spawn(move || ring_workload(dev, pid_ref, i)));
            }

            // ---- REAL HOST REFUSALS, injected while all of the above runs. These are not
            // scripted failures: the kernel refuses, with its own errno.
            let t_refuse = sc.spawn(move || {
                let mut eexist = 0usize;
                let mut enomem = 0usize;
                for k in 0..RREFUSE_OPS {
                    if k % 2 == 0 {
                        // A window overlapping proc-0's — the guest-physical flat view is the
                        // KERNEL's, and it says EEXIST.
                        assert_eq!(
                            m.install_ram_window(RGPA_A + 8 * page, 4 * page),
                            Err(kayfabe_vmm::VmmError::HostRefused {
                                what: "a memslot install",
                                errno: Some(libc::EEXIST),
                            }),
                            "the EXACT errno, never is_err(): EEXIST (an overlap) and EINVAL \
                         (an exhausted ceiling) are operationally different"
                        );
                        eexist += 1;
                    } else {
                        // A window the address space cannot hold — the mmap fails BEFORE any
                        // memslot exists, which is the other partial-failure shape.
                        assert_eq!(
                            m.install_ram_window(0x8000_0000, (1u64 << 62) & !(page - 1)),
                            Err(kayfabe_vmm::VmmError::HostRefused {
                                what: "a window mapping",
                                errno: Some(libc::ENOMEM),
                            })
                        );
                        enomem += 1;
                    }
                }
                (eexist, enomem)
            });

            // ---- the revoke prober: parses out of RGPA_REVOKE until the main thread tears
            // its window down, then must never be served again.
            let started_ref = &started;
            let t_revoke = sc.spawn(move || {
                let _gate = OpenOnDrop(started_ref);
                let mut vmm = vw.clone();
                let cid = dev
                    .doorbell(GPU0, MockArch::token_for(lane_of(P_WITNESS).ce), &[])
                    .expect("the prober's CE channel rings")
                    .chan;
                let ring = gpfifo_ring(RGPA_REVOKE, 0x40);
                vmm.gpa_write(RGPA_REVOKE, &[0u8; 0x40])
                    .expect("the window is real RAM before the teardown");
                let mut p = RevokeProbe {
                    served: 0,
                    first_refusal: None,
                    refusals: 0,
                    served_after_refusal: 0,
                };
                for _ in 0..RREVOKE_BUDGET {
                    match dev.parse_pushbuffer(&mut vmm, pid_ref[P_WITNESS], cid, &ring) {
                        Ok(_) => {
                            if p.first_refusal.is_some() {
                                p.served_after_refusal += 1;
                                break;
                            }
                            p.served += 1;
                            started_ref.open();
                        }
                        Err(e) => {
                            p.refusals += 1;
                            if p.first_refusal.is_none() {
                                p.first_refusal = Some(e);
                                continue;
                            }
                            break;
                        }
                    }
                }
                p
            });

            // ---- the world MOVES: a channel free, and the window teardown racing a live
            // parser that is copying out of it.
            dev.apply(RmEvent::Free {
                client: client_of(P_CHANFREE),
                handle: handles.gr_channel,
            })
            .expect("the channel free applies");
            started.wait();
            churn_started.wait();
            machine
                .remove_window(win_revoke)
                .expect("★ a REAL memslot DELETE plus a munmap, while a thread parses out of it");

            let a = t_dma_a.join().expect("the GPU0 DMA thread panicked");
            let b = t_dma_b.join().expect("the GPU1 DMA thread panicked");
            let ca = t_churn_a.join().expect("the GPU0 churn thread panicked");
            let cb = t_churn_b.join().expect("the GPU1 churn thread panicked");
            let rp = t_revoke.join().expect("the revoke prober panicked");
            let rc = t_churn_r.join().expect("the revoke churn thread panicked");
            let (ee, en) = t_refuse.join().expect("the host-refusal thread panicked");
            for h in peers {
                h.join().expect("a peer workload thread panicked");
            }
            latches.release_all();
            let _ = t_held.join();
            let _ = t_held2.join();
            (a, b, ca, cb, rp, rc, ee, en)
        });

    // ---- teardown: both remaining windows, so the ledger has somewhere to balance to.
    let checked = machine.assert_map_matches_the_kernel();
    machine.remove_window(win_a).expect("remove A");
    machine.remove_window(win_b).expect("remove B");
    drop(device);

    RealMeanReport {
        witness,
        peer,
        churn_a,
        churn_b,
        revoke,
        revoke_churn,
        refusals_eexist: eexist,
        refusals_enomem: enomem,
        audit: machine.audit(),
        map_regions_checked: checked,
    }
}

/// ★★★ **THE MEAN COMPOSED RUN AGAINST A REAL `Vmm`** — `l1_os_shell.md` §10 stage M2-c,
/// and the first test in this project's history that could tell a memcpy from a
/// `KVM_SET_USER_MEMORY_REGION` (gap **O1**).
///
/// In one window, under **both** lock modes: two DMA threads aiming descriptors at a real
/// device region, a hole and a window boundary while parsing legitimate pushbuffers out
/// of real `mmap`ed memory; two churn threads doing real `MAP_FIXED` placements and
/// restores; four peer threads doing real publish/doorbell work; two host verbs parked;
/// a thread provoking **real kernel refusals** (`EEXIST`, `ENOMEM`) throughout; and a
/// **real memslot DELETE plus `munmap`** landing on a window while a thread is copying
/// out of it.
///
/// The assertions are the composed ones: the exact refusal for each hostile shape, the
/// exact counts, R1 on both halves (ranked **and** the adapter's own), the memslot
/// frequency gate, the conservation ledger, and a bit-identical outcome across the two
/// lock modes.
#[test]
fn a_real_memory_plane_survives_multiproc_churn_teardown_and_host_refusal_under_load() {
    kayfabe_linux_raw::require_kvm!(
        "a_real_memory_plane_survives_multiproc_churn_teardown_and_host_refusal_under_load"
    );
    let _wd = watchdog(
        "a_real_memory_plane_survives_multiproc_churn_teardown_and_host_refusal_under_load",
        Duration::from_secs(300),
    );
    let degenerate = real_mean_run(LockMode::Degenerate);
    let sharded = real_mean_run(LockMode::Sharded);

    for (name, r) in [("Degenerate", &degenerate), ("Sharded", &sharded)] {
        let a = r.audit;

        // ---- every hostile shape was REACHED, the exact number of times scripted.
        for (who, t) in [("witness", r.witness), ("peer", r.peer)] {
            assert_eq!(
                t.legal_ok, RDMA_OPS,
                "({name}/{who}) every memslot-backed submit must still be served — a \
                 refusal that also breaks the legal path is not a fix"
            );
            assert_eq!(
                t.non_ram,
                RDMA_OPS / 2,
                "({name}/{who}) two of the four hostile shapes name our own trapped BAR, \
                 which on KVM has NO MEMSLOT at all"
            );
            assert_eq!(
                t.unbacked,
                RDMA_OPS / 2,
                "({name}/{who}) and two name nothing — the hole and the window boundary. \
                 Counted separately from the device arm so the two can never silently merge"
            );
        }

        // ---- ★ R1, BOTH HALVES. This is what the stage exists to test.
        assert_eq!(
            a.syscall_ranked_depth,
            (0, 0),
            "({name}) ★ R1: every syscall-shaped Vmm method ran with ZERO ranked locks. \
             A `min` of u32::MAX would mean no syscall ran at all — which is why the PAIR \
             is asserted, not the max"
        );
        assert_eq!(
            a.accessor_ranked_depth,
            (0, 1),
            "({name}) ★ and the in-lock-legal accessors really were entered WITH a ranked \
             lock (max 1, the route phase of parse_pushbuffer) AND lock-free (min 0, the \
             scripting writes). Without both ends this whole run could be green about a \
             lock-free path"
        );
        assert_eq!(
            a.copy_leaf_depth_max, 0,
            "({name}) ★★ THE ADAPTER HALF OF R1 (§12.43 residual 2): the guest-memory \
             memcpy runs OUTSIDE the adapter's own map lock, held alive by an Arc. \
             `lockwitness` is blind to this lock — it has no rank — so a copy under it \
             would serialise every guest read in the machine behind every memslot ioctl, \
             with every existing assert green"
        );
        assert!(
            a.view_leaf_depth_max >= 1,
            "({name}) ★ NON-VACUITY for the line above: the map lock must actually have \
             been taken. With no lock at all, `copy_leaf_depth_max == 0` is true and means \
             nothing"
        );
        assert!(
            a.accesses_served > u64::try_from(RDMA_OPS).expect("fits") && a.accesses_refused > 0,
            "({name}) ★ NON-VACUITY for both R1 halves: the accessors must have run, and \
             both outcomes must have occurred ({} served, {} refused). A depth span over \
             an unexercised path is the shape §12.43's N12 found",
            a.accesses_served,
            a.accesses_refused,
        );

        // ---- ★ the MEMSLOT-FREQUENCY GATE (§9.3 / §6.7).
        assert_eq!(
            a.memslot_installs,
            3,
            "({name}) ★ installs must scale with WINDOWS (three of them), never with \
             publications. The failed EEXIST/ENOMEM attempts installed nothing, and the \
             {placed} successful placements installed nothing — a per-object memslot is \
             the C artifact's measured regression (>1500 slots for one cuCtxCreate), \
             caught structurally and without a clock",
            placed = a.placements_made,
        );
        assert!(
            a.placements_made >= 8,
            "({name}) ★ NON-VACUITY for the gate: only {} placements happened, so the \
             ratio it asserts is not being exercised",
            a.placements_made
        );

        // ---- ★ real host refusals happened, and left NOTHING behind.
        assert_eq!(
            (r.refusals_eexist, r.refusals_enomem),
            (RREFUSE_OPS / 2, RREFUSE_OPS - RREFUSE_OPS / 2),
            "({name}) every injected refusal must have been the kernel's, by exact errno"
        );
        assert_eq!(
            a.host_refusals, RREFUSE_OPS as u64,
            "({name}) ★ and the adapter COUNTED every one — an uncounted refusal is a \
             refusal the conservation assertions below cannot be about"
        );

        // ---- ★ the CONSERVATION LEDGER balances after every window is gone.
        assert_eq!(
            (
                a.live_windows,
                a.live_memslots,
                a.live_placements,
                a.window_bytes
            ),
            (0, 0, 0, 0),
            "({name}) ★ nothing leaked: not a window, not a memslot, not a placement, not \
             a byte of address space — across 2 GPUs, {} placements, {} real host \
             refusals and a teardown that raced a live parser",
            a.placements_made,
            a.host_refusals,
        );
        assert_eq!(
            (a.peak_windows, a.peak_memslots),
            (3, 3),
            "({name}) ★ NON-VACUITY for the ledger: it really did count three live \
             windows at once. A ledger that was never incremented balances perfectly"
        );
        assert_eq!(
            a.window_mappings_released, 3,
            "({name}) ★★ and the address space was really GIVEN BACK: three windows \
             removed, three `munmap`s performed — every one of them by a thread the \
             adapter had already proved lock-free. A removed window whose mapping an \
             accessor was still reading is parked until nobody holds it (§14.8 F7(b)'s \
             unnamed half), and the hazard the parking removes is that the accessor's \
             own `Arc` drop performs the release: `gpa_read` is entered WITH rank 0 held, \
             so that `munmap` is an R1 violation. `window_bytes == 0` above is \
             bookkeeping and would be true of a mapping nobody ever released; this is the \
             syscall"
        );
        assert!(
            a.peak_placements >= 1,
            "({name}) ★ NON-VACUITY: the churn really did hold a live placement"
        );

        // ---- mid-flight window teardown: served, then refused, and never served again.
        assert!(
            r.revoke.served > 0,
            "({name}) the revoke prober never had its window served — it proves nothing"
        );
        assert_eq!(
            r.revoke.first_refusal,
            Some(FwdFault::GpaRead { gpa: RGPA_REVOKE }),
            "({name}) ★ a window whose memslot was DELETED and whose mapping was munmapped \
             under a live parser must refuse as UNBACKED — not as a device window, never \
             as stale bytes, and never as a SIGSEGV. The reader that had already resolved \
             holds an Arc on the mapping, so its copy completes against memory that is \
             still there; the reader that had not gets a named refusal"
        );
        assert_eq!(
            r.revoke.served_after_refusal, 0,
            "({name}) a torn-down window resolved again after being revoked"
        );

        // ---- ★★ THE TEARDOWN THAT RACED A LIVE MAPPING.
        assert!(
            r.revoke_churn.placed > 0,
            "({name}) the racing churn never placed anything into the doomed window —              the race it exists to run never happened"
        );
        assert_eq!(
            r.revoke_churn.first_refusal,
            Some(kayfabe_vmm::VmmError::BadGpa {
                gpa: RGPA_REVOKE + 2 * host_page()
            }),
            "({name}) ★ a publication into a window that has been removed must be refused              by NAME. Either the plan found no window (this arm) or the R5 re-validation              caught the generation change at commit — both are BadGpa, and both are              counted; what must never happen is a MAP_FIXED into address space the              adapter no longer owns"
        );
        assert_eq!(
            r.revoke_churn.placed_after_refusal, 0,
            "({name}) a removed window accepted a publication again — a resurrected              mapping"
        );
        assert_eq!(
            r.revoke_churn.placed,
            1 + r.revoke_churn.restored + r.revoke_churn.lost_with_the_window,
            "({name}) ★ the doomed window's conservation identity: every placement is the \
             one deliberately HELD across the teardown, or one this thread restored \
             itself, or one whose slot went with its window ({} restored, {} lost). The \
             SPLIT is a clock and is normalised away for the cross-mode differential; the \
             identity is not, and a third outcome would be a MAP_FIXED nobody owns",
            r.revoke_churn.restored,
            r.revoke_churn.lost_with_the_window,
        );
        assert_eq!(
            r.revoke_churn.held_slot_after_teardown,
            Some(kayfabe_vmm::VmmError::BadSlot(kayfabe_vmm::SlotId(
                r.revoke_churn.held_slot_id
            ))),
            "({name}) ★ the placement that was still LIVE when its window was removed              must have gone WITH the window: unmapping it is a named BadSlot, never a              `restore` into address space the process has already returned to the kernel"
        );

        // ---- the churn's conservation identity (the split is racy; the identity is not).
        for (who, c) in [("A", r.churn_a), ("B", r.churn_b)] {
            assert_eq!(
                c.placed, c.restored,
                "({name}/churn {who}) every placement must have been restored — a \
                 placement without its restore is a live MAP_FIXED nobody owns"
            );
            assert_eq!(
                c.placed + c.refused,
                RPUB_OPS,
                "({name}/churn {who}) every attempt is either a placement or a NAMED \
                 refusal; a third outcome would be an op that silently did nothing"
            );
            assert!(
                c.placed > 0,
                "({name}/churn {who}) ★ NON-VACUITY: the churn thread placed nothing at \
                 all, so every assertion about it is about an operation that never ran"
            );
        }

        assert_eq!(
            r.map_regions_checked, 2,
            "({name}) at the end of the run the map and the kernel still agreed about the \
             two surviving windows"
        );
    }

    assert_eq!(
        degenerate.mode_independent(),
        sharded.mode_independent(),
        "★ the lock configuration is observable through the REAL memory plane — the \
         script-determined half of the run must be bit-identical in both modes"
    );
}

// =================================================================================
// ★★★ M2-d — THE GUEST IS REAL: vCPUs, MMIO exit dispatch, the reactor, and the
// teardown order that only a guest can see (l1_os_shell.md §10 stage M2-d)
// =================================================================================
//
// Everything above this line drives the core through its own entry points. This section
// drives it through a **guest**: real vCPUs on the real KVM machine, executing real x86
// instructions, whose stores exit to userspace and are dispatched into the same
// `SharedDevice` every other test in this file calls directly.
//
// Three things become observable here that were not, and §14.8 F7 named two of them:
//
//   (a) WHERE a memslot points. The guest reads a cookie we placed in its probe page and
//       presents it at the doorbell; a wrong offset delivers a different cookie.
//   (b) ★★ RESIDUAL N13 — the ORDER of a teardown. M2-c's bite-check ("DELETE the memslot
//       before undeclaring the region") did not bite, because with no vCPU the `Arc` keeps
//       the mapping alive either way. With a vCPU the orders are physically different: a
//       range with no memslot EXITS, so a premature DELETE opens a window in which the
//       guest's access arrives as an MMIO exit while the region map still calls the range
//       RAM. `AuditReport::ram_declared_exits` counts exactly that, the invariant is `== 0`,
//       and under the correct order the window is zero-wide by construction (the undeclare
//       completes under the view lock the exit handler consults).
//   (c) That a registered trap actually DELIVERS. M2-c registered traps and asserted their
//       consistency with the memslot map; nothing dispatched one.

/// Guest-physical base of the vCPUs' code window (one page per vCPU).
///
/// ★ Below 4 GiB, and that is a **requirement**, not a convention: the guest runs in flat
/// 32-bit protected mode with paging off, so guest-linear is guest-physical and every
/// address it can name is 32 bits wide. A window above 4 GiB would be unreachable, which
/// the `u32::try_from` on the image's immediate turns into a loud failure rather than a
/// silently-truncated probe address.
const RGPA_CODE: u64 = 0x1000_0000;
/// Guest-physical base of the per-vCPU probe windows (one page each, `RGPA_PROBE_STRIDE`
/// apart so each is its own window and can be torn down alone).
const RGPA_PROBE: u64 = 0x1100_0000;
/// Distance between consecutive probe windows.
const RGPA_PROBE_STRIDE: u64 = 0x10_0000;
/// How many vCPUs storm the doorbell at once.
const N_VCPUS: usize = 3;
/// Which vCPU's probe window is torn down while every vCPU is running.
const VCPU_DOOMED: usize = N_VCPUS - 1;
/// MMIO exits each vCPU handles before it stops. A **count**, never a duration.
const VCPU_EXITS: u64 = 400;
/// Completion signals the reactor's counter source carries during the run.
const RSIGNALS: u64 = 150;

/// The cookie vCPU `i` finds in its probe page. Deliberately not `i` and not a small
/// number: a stale page, a zero-filled one and an off-by-one window all produce values
/// this pattern cannot be confused with.
fn vcpu_cookie(i: usize) -> u32 {
    0x5A5A_0000 | (i as u32 + 1)
}

/// What one composed M2-d run observed.
#[derive(Debug, Clone)]
struct GuestMeanReport {
    audit: kayfabe_vmm_kvm::AuditReport,
    reactor: kayfabe_shell::ReactorStats,
    device: kayfabe_tests::DeviceTally,
    /// Per-vCPU: (exits, ram-declared exits, unclaimed exits).
    vcpus: Vec<(u64, u64, u64)>,
    /// Why each guest stopped. Asserted, because a triple-faulted guest also has
    /// `exits > 0` and would otherwise pass every count in this run — a bite-check found
    /// exactly that gap (neutering the stop-flag check did not bite, because the exit
    /// BUDGET was silently doing the work).
    stops: Vec<kayfabe_vmm_kvm::vcpu::StopReason>,
    /// The first address that exited while still declared RAM, if any (N13's witness).
    first_contradiction: Option<u64>,
    /// Was the HUP'd proc retired by the reactor's worker-death path?
    hup_proc_retired: bool,
    /// Did the deferred deadline that fell during the teardown come due?
    deferred_due: Vec<kayfabe_vmm::CoreEvent>,
    /// Effects the executor thread ran.
    signal_effects: usize,
    /// Live procs before the worker died, and after — a **difference**, never an absolute,
    /// because the realized device also carries the system proc and an absolute would be
    /// asserting the harness's shape rather than the blast radius.
    live_pids: (usize, usize),
}

impl GuestMeanReport {
    /// The script-determined half — must be bit-identical across lock modes.
    fn mode_independent(&self) -> (u64, u64, u64, u64, bool, (usize, usize)) {
        (
            self.audit.ram_declared_exits,
            self.audit.memslot_installs,
            self.device.cookie_mismatch,
            self.reactor.signals_pushed,
            self.hup_proc_retired,
            self.live_pids,
        )
    }
}

/// ★★ One composed M2-d run: `N_VCPUS` guests storming a trapped doorbell across two
/// GPUs, a real reactor draining real counters, a worker channel that HUPs mid-storm, a
/// window torn down underneath a running guest, and a deferred deadline that falls during
/// the teardown.
fn guest_mean_run(mode: LockMode) -> GuestMeanReport {
    let (device, pids, _rec) = mean_world(mode);
    let machine = real_machine();
    let page = host_page();
    let dev: &SharedDevice = &device;

    // ---- the guest's address space: one code window, N probe windows, one trapped BAR
    // (declared at realize, i.e. NO memslot at all — which on KVM is what a device region
    // physically IS).
    let code_win = machine
        .install_ram_window(RGPA_CODE, (N_VCPUS as u64) * page)
        .expect("the code window");
    let probe_wins: Vec<_> = (0..N_VCPUS)
        .map(|i| {
            let gpa = RGPA_PROBE + (i as u64) * RGPA_PROBE_STRIDE;
            (
                gpa,
                machine
                    .install_ram_window(gpa, page)
                    .expect("a probe window"),
            )
        })
        .collect();

    // ---- write the images and the cookies through the REAL memory plane.
    let mut vmm = machine.vmm();
    for (i, (probe, _)) in probe_wins.iter().enumerate() {
        let image = kayfabe_tests::probe_loop_image(
            u32::try_from(*probe).expect("the probe window is below 4 GiB"),
        );
        vmm.gpa_write(RGPA_CODE + (i as u64) * page, &image)
            .expect("the image lands in guest RAM");
        vmm.gpa_write(*probe, &vcpu_cookie(i).to_le_bytes())
            .expect("the cookie lands in guest RAM");
    }
    assert_eq!(
        machine.assert_map_matches_the_kernel(),
        1 + N_VCPUS,
        "every RAM region the map resolves is a live memslot before the guests start"
    );

    // ---- ★★ THE N13 DETECTOR'S POSITIVE CONTROL, and it is here because of a bite-check
    // that did NOT bite: making `classify_exit` unable to answer `RamDeclared` left the
    // whole run green, since `ram_declared_exits == 0` is equally true of a working
    // detector and of one that cannot fire. All three answers are asserted against a
    // layout this function chose, so the invariant below is measured with an instrument
    // that has been shown to work.
    use kayfabe_vmm_kvm::vcpu::ExitClass;
    assert_eq!(
        machine.classify_guest_exit(probe_wins[0].0, 4),
        ExitClass::RamDeclared {
            gpa: probe_wins[0].0
        },
        "★ a LIVE RAM window classifies as the contradiction — correctly: the answer means \
         'an exit here would be the map and the kernel disagreeing', and while the window \
         is live no exit can happen. This is the arm the invariant is about, and it must \
         be REACHABLE"
    );
    assert_eq!(
        machine.classify_guest_exit(RGPA_BAR0 + 0x100, 4),
        ExitClass::Bar {
            bar: kayfabe_vmm::BarId::Bar0,
            off: 0x100
        },
        "★ and a BAR address classifies to the BAR and the OFFSET WITHIN IT — an offset \
         that was not base-relative would dispatch every doorbell to the wrong lane"
    );
    assert_eq!(
        machine.classify_guest_exit(RGPA_HOLE, 4),
        ExitClass::Unclaimed { gpa: RGPA_HOLE },
        "★ and an address nothing declares is unclaimed — never silently absorbed into \
         either of the other two"
    );

    // ---- the device the exits dispatch into: one doorbell lane per vCPU, alternating
    // GPUs, each with the cookie we just wrote and the CE token it stands for.
    let doorbell = Arc::new(kayfabe_tests::DoorbellDevice::new(
        Arc::clone(&device),
        (0..N_VCPUS)
            .map(|i| kayfabe_tests::DoorbellLane {
                gpu: gpu_of(i),
                cookie: vcpu_cookie(i),
                token: MockArch::token_for(lane_of(i).ce),
            })
            .collect(),
    ));

    // ---- the reactor, over real host readiness primitives.
    let poller = Arc::new(kayfabe_linux_raw::Poller::create().expect("a readiness set"));
    let registrar = Arc::new(kayfabe_shell::Registrar::new(poller).expect("a registrar"));
    let (tx, rx) = inbox();
    let parker = Arc::new(kayfabe_rt::executor::Parker::new());
    let (mut reactor, rhandle) = kayfabe_shell::Reactor::new(
        Arc::clone(&registrar),
        tx,
        Arc::clone(&parker) as Arc<dyn kayfabe_rt::executor::ExecutorWaker>,
    )
    .expect("the reactor builds");

    // A hot counter source…
    let hot = dev.register_source(SourceKind::OsEvent {
        proc: pids[P_WITNESS],
        gpu: GPU0,
        ev: EV[0],
    });
    let hot_counter = registrar.arm_counter(hot).expect("arms");
    // …a source that is NEVER signalled (the wedged one — it must cost exactly zero
    // wakes, which is the other polarity of the F1 quantity)…
    let wedged = dev.register_source(SourceKind::OsEvent {
        proc: pids[P_PEER],
        gpu: GPU1,
        ev: EV[1],
    });
    let _wedged_counter = registrar.arm_counter(wedged).expect("arms");
    // …and a worker channel whose HUP retires a proc mid-storm.
    let hup = dev.register_source(SourceKind::Worker {
        proc: pids[P_HUP],
        gpu: gpu_of(P_HUP),
        worker: WorkerId(0),
    });
    let hup_channel = registrar.arm_channel(hup).expect("arms");

    // ---- a deferred deadline that will fall DURING the teardown (§6.4's shared queue —
    // the same `DeferQueue` the mock owns, so "matches the mock" is a tautology).
    vmm.defer(
        Duration::from_millis(10),
        kayfabe_vmm::CoreEvent::Deferred(kayfabe_vmm::CoreEventKind::CompletionRedeliver(GPU0)),
    );

    let live_before = dev.live_pids().len();
    let stop = Arc::new(AtomicBool::new(false));
    let effects = Arc::new(Mutex::new(Vec::<Effect>::new()));
    let storm_started = StartGate::default();

    let mut runners: Vec<_> = (0..N_VCPUS)
        .map(|i| {
            let r = machine
                .create_vcpu(
                    u32::try_from(i).expect("few vCPUs"),
                    Arc::clone(&doorbell) as _,
                )
                .expect("a real vCPU");
            r.enter_at(
                RGPA_CODE + (i as u64) * page,
                kayfabe_tests::DoorbellDevice::lane_gpa(RGPA_BAR0, i),
            )
            .expect("flat protected mode");
            r
        })
        .collect();

    let deferred_due = thread::scope(|sc| {
        // ---- the reactor thread.
        let t_reactor = sc
            .spawn(move || reactor.run_with(kayfabe_linux_raw::PollTimeout::Millis(20), 1_000_000));
        // ---- the executor thread, parked on the ExecutorWaker.
        let d = Arc::clone(&device);
        let sink = Arc::clone(&effects);
        let pk = Arc::clone(&parker);
        let t_exec = sc.spawn(move || {
            let mut exec = Executor::new(d, rx);
            exec.run_until_stopped(&pk, Duration::from_millis(20), |e| {
                sink.lock().expect("sink").push(e);
            });
        });

        // ---- ★ the MMIO storm: N vCPUs at once, each on its own thread.
        let gate = &storm_started;
        let stop_ref = &stop;
        let mut vcpu_threads = Vec::new();
        for (i, mut runner) in runners.drain(..).enumerate() {
            vcpu_threads.push(sc.spawn(move || {
                if i == 0 {
                    gate.open();
                }
                let reason = runner.run_until(stop_ref, VCPU_EXITS).expect("KVM_RUN");
                (reason, runner.report(), runner.first_ram_declared_exit())
            }));
        }
        gate.wait();

        // ---- a relay writing the hot counter while the guests run.
        let hc = Arc::clone(&hot_counter);
        let t_relay = sc.spawn(move || {
            for _ in 0..RSIGNALS {
                hc.signal().expect("relay write");
            }
        });

        // ---- ★★ THE TEARDOWN, UNDER A RUNNING GUEST. The doomed vCPU is looping on this
        // window's probe page; removing it must undeclare the region FIRST, so that by the
        // time the memslot is gone the map has already stopped calling the range RAM.
        // Every access that lands in the window between the two steps is what
        // `ram_declared_exits` counts.
        machine
            .remove_window(probe_wins[VCPU_DOOMED].1)
            .expect("the doomed window goes");

        // ---- the worker dies mid-storm (§7.3 through the §6 reactor).
        drop(hup_channel);

        // ---- and the deferred deadline falls right here, in the middle of the teardown.
        let due = machine.advance(Duration::from_millis(10));

        t_relay.join().expect("the relay joins");
        let reports: Vec<_> = vcpu_threads
            .drain(..)
            .map(|t| t.join().expect("a vCPU thread joins"))
            .collect();
        stop.store(true, Ordering::Release);

        // Wait for the reactor to have carried every signal (a condition, not a sleep).
        let deadline = WallInstant::now() + Duration::from_secs(60);
        while rhandle.stats().signals_pushed < RSIGNALS + 1 {
            assert!(
                WallInstant::now() < deadline,
                "({mode:?}) the reactor wedged: {:?}",
                rhandle.stats()
            );
            thread::yield_now();
        }
        rhandle.shutdown().expect("shutdown");
        assert_eq!(
            t_reactor.join().expect("the reactor thread joins"),
            Ok(()),
            "({mode:?}) ★ the loop exited cleanly — an F1 refusal here would mean one of \
             these real sources could not be drained"
        );
        parker.stop();
        t_exec.join().expect("the executor thread joins");

        // Stash the per-vCPU numbers where the report can reach them.
        (due, reports)
    });
    let (due, vcpu_reports) = deferred_due;

    // ---- tear the rest down; the ledger must balance.
    for (i, (_, w)) in probe_wins.into_iter().enumerate() {
        if i != VCPU_DOOMED {
            machine.remove_window(w).expect("a probe window goes");
        }
    }
    machine
        .remove_window(code_win)
        .expect("the code window goes");
    registrar.disarm_all();

    let first_contradiction = vcpu_reports.iter().find_map(|(_, _, c)| *c);
    GuestMeanReport {
        audit: machine.audit(),
        reactor: rhandle.stats(),
        device: doorbell.tally(),
        vcpus: vcpu_reports
            .iter()
            .map(|(_, r, _)| (r.exits, r.ram_declared_exits, r.unclaimed_exits))
            .collect(),
        stops: vcpu_reports.iter().map(|(s, _, _)| *s).collect(),
        first_contradiction,
        hup_proc_retired: !dev.live_pids().contains(&pids[P_HUP]),
        deferred_due: due,
        signal_effects: effects.lock().expect("sink").len(),
        live_pids: (live_before, dev.live_pids().len()),
    }
}

/// ★★★ **THE MEAN COMPOSED RUN WITH A REAL GUEST** — `l1_os_shell.md` §10 stage M2-d,
/// and the test that closes residual **N13**.
///
/// In one window, under **both** lock modes: three real vCPUs executing real x86
/// instructions and storming a trapped doorbell across two GPUs; a real reactor loop
/// blocking in a real `epoll_wait` over a hot counter source, a never-signalled one and a
/// worker channel; a real executor thread woken through the `ExecutorWaker`; a **real
/// memslot DELETE plus `munmap`** landing on a window a guest is looping on; a worker
/// dying with completions in flight; and a deferred deadline falling in the middle of the
/// teardown.
///
/// The assertions are the composed ones: N13's contradiction count, the cookie round trip,
/// the F1 wake-count quantity, both halves of R1, the memslot-frequency gate, the
/// conservation ledger, and a bit-identical script-determined outcome across lock modes.
#[test]
fn a_real_guest_storm_survives_a_window_teardown_worker_death_and_a_deferred_deadline() {
    kayfabe_linux_raw::require_kvm!(
        "a_real_guest_storm_survives_a_window_teardown_worker_death_and_a_deferred_deadline"
    );
    let _wd = watchdog(
        "a_real_guest_storm_survives_a_window_teardown_worker_death_and_a_deferred_deadline",
        Duration::from_secs(300),
    );
    let degenerate = guest_mean_run(LockMode::Degenerate);
    let sharded = guest_mean_run(LockMode::Sharded);

    for (name, r) in [("Degenerate", &degenerate), ("Sharded", &sharded)] {
        let a = r.audit;

        // ---- ★★ RESIDUAL N13, CLOSED. The invariant is a count and it is zero.
        assert_eq!(
            (a.ram_declared_exits, r.first_contradiction),
            (0, None),
            "({name}) ★★ N13: a guest access must NEVER exit to userspace at an address \
             the region map still calls RAM. That state exists only between a memslot \
             DELETE and the undeclare that should have preceded it — so a nonzero count \
             here IS the teardown order being wrong, observed from the guest's side. \
             (M2-c could not test this: with no vCPU the Arc keeps the mapping alive and \
             both orders look identical.)"
        );
        assert!(
            a.guest_exits >= (N_VCPUS as u64) * VCPU_EXITS / 2,
            "({name}) ★ NON-VACUITY for the line above: only {} guest exits were dispatched, \
             so 'zero contradictions' may just mean 'no guest ever ran'. A window torn down \
             under a guest that is not executing proves nothing",
            a.guest_exits
        );
        for (i, (exits, contradictions, _)) in r.vcpus.iter().enumerate() {
            assert_eq!(
                *contradictions, 0,
                "({name}/vcpu {i}) and per vCPU, not merely in aggregate"
            );
            assert!(
                *exits > 0,
                "({name}/vcpu {i}) ★ NON-VACUITY: this vCPU never exited at all, so the \
                 'MMIO storm from multiple vCPUs at once' is one vCPU wearing a plural"
            );
        }
        // ★ WHY each guest stopped, by exact variant. A guest that triple-faulted, or that
        // KVM could not emulate, also satisfies `exits > 0` — so without this the whole run
        // could be green about three guests that died three instructions in.
        for (i, stop) in r.stops.iter().enumerate() {
            assert!(
                matches!(
                    stop,
                    kayfabe_vmm_kvm::vcpu::StopReason::BudgetSpent
                        | kayfabe_vmm_kvm::vcpu::StopReason::Stopped
                ),
                "({name}/vcpu {i}) a guest must end because WE ended it — by its exit \
                 budget or by the stop flag — never by faulting: {stop:?}"
            );
        }

        // ---- ★ (a) WHERE the memslot points — the cookie round trip, at the device.
        assert_eq!(
            r.device.cookie_mismatch, 0,
            "({name}) ★ every doorbell write carried EXACTLY the cookie we placed in that \
             vCPU's probe page. §14.8 F7(a): neutering the sub-range address arithmetic \
             'broke nothing, because KVM validates nothing at install time and nothing \
             executes' — something executes now, and this is what it proves"
        );
        assert!(
            r.device.rang > 0,
            "({name}) ★ NON-VACUITY: not one doorbell write reached the core, so the \
             cookie assertion above is about writes that never happened"
        );
        assert_eq!(
            r.device.stray_writes, 0,
            "({name}) every exit was classified to a real doorbell lane — a stray means the \
             BAR offset arithmetic in the exit dispatch is wrong"
        );
        assert!(
            r.device.unbacked_cookie > 0,
            "({name}) ★ and the OTHER polarity, which is what keeps the assertion above \
             honest: the doomed vCPU's probe page really did stop resolving, so its loads \
             came back as the all-ones 'nobody answered' value. Zero here would mean the \
             teardown never took effect underneath a running guest — and then \
             `cookie_mismatch == 0` would be true for the boring reason"
        );

        // ---- ★ (c) a registered trap DELIVERS. The doomed vCPU's probe window is gone, so
        // its loads exit unclaimed; every vCPU's stores keep reaching the device.
        let unclaimed: u64 = r.vcpus.iter().map(|(_, _, u)| u).sum();
        assert!(
            unclaimed > 0,
            "({name}) ★ the torn-down window's guest must have taken UNCLAIMED exits after \
             the teardown — that is the memslot really being gone, not merely marked gone"
        );

        // ---- ★ the F1 wake-count gate, over real sources under real load.
        let s = r.reactor;
        assert_eq!(
            s.signals_pushed,
            RSIGNALS + 1,
            "({name}) ★ EXACTLY the signals the producers sent: {RSIGNALS} counter writes \
             plus the one worker HUP. Coalescing moves the wake count, never this. {s:?}"
        );
        // ★ The control channel is an instrument too, and until 2026-07-27 it was the one
        // instrument in this block nothing asserted — which is exactly how it went missing
        // from the bound below. `ReactorHandle::shutdown` is the only thing in this harness
        // that signals it (`wake()` is never called here), and the teardown calls it once.
        assert_eq!(
            s.control_reports, 1,
            "({name}) ★ EXACTLY the one control wake this run performs — the teardown's \
             `shutdown()`. It is asserted rather than merely added to the bound below, \
             because an unbounded control term would turn that bound into a free pass. \
             {s:?}"
        );
        // ★★ THE BOUND INCLUDES THE CONTROL WAKES — and it did not until 2026-07-27, when
        // this assertion was caught flaking at 1-in-20 under CPU contention (`wakes: 152`
        // against a `RSIGNALS + 1` = 151 ceiling; reproduced on an unmodified tree, so it
        // predates the change that found it). It was an off-by-one in the TEST, not a
        // reactor bug: `ReactorStats::wakes`'s own doc says "bounded above by the number of
        // signals sent **plus the control wakes**", and the hardcoded `RSIGNALS + 1` had
        // dropped the second term. Under most schedules the shutdown wake coalesces into a
        // wait that also carried a signal and the count stays under 151; under load it
        // arrives alone and does not.
        //
        // A flake here is not cosmetic: a red test in an UNMUTATED tree aborts
        // `cargo mutants` outright, which is the same way the fetch-from-nothing test was
        // blocking the mutation gate on the bench.
        assert!(
            s.wakes >= 1 && s.wakes <= s.signals_pushed + s.control_reports,
            "({name}) ★ the F1 quantity: {} wakes for {} signals plus {} control wakes. \
             The upper bound is what an undrained level-triggered source blows through \
             without limit — and the never-signalled source in this run contributes ZERO \
             to it, which is the other polarity. Both terms are pinned exactly by the two \
             assertions above, so this stays a real ceiling and not an arithmetic \
             identity. {s:?}",
            s.wakes,
            s.signals_pushed,
            s.control_reports,
        );
        assert_eq!(
            (s.undrained_reports, s.stale_reports),
            (0, 0),
            "({name}) every ready report was drained and named a live source: {s:?}"
        );
        assert_eq!(
            s.terminal_fired, 1,
            "({name}) ★ the worker HUP fired ONCE — a hung-up channel is readable forever, \
             so a loop that tried to drain it would still be spinning. {s:?}"
        );

        // ---- ★ the worker died with events in flight, and the core's consequence ran.
        assert!(
            r.hup_proc_retired,
            "({name}) ★ a worker death must retire its proc (§7.3 — never a silent respawn), \
             and it must do so while {RSIGNALS} completions for OTHER procs are in flight"
        );
        assert_eq!(
            r.live_pids.1,
            r.live_pids.0 - 1,
            "({name}) exactly ONE proc retired — the blast radius of a worker death is its \
             own proc, not the device"
        );
        assert!(
            r.signal_effects >= (RSIGNALS + 1) as usize,
            "({name}) the executor thread ran every pushed signal ({} effects)",
            r.signal_effects
        );

        // ---- ★ the deferred deadline that fell during the teardown.
        assert_eq!(
            r.deferred_due,
            vec![kayfabe_vmm::CoreEvent::Deferred(
                kayfabe_vmm::CoreEventKind::CompletionRedeliver(GPU0)
            )],
            "({name}) ★ a deadline that passes while a window is being torn down still comes \
             due, exactly once, in deadline order — §6.4's shared DeferQueue, which is the \
             SAME code the mock runs, so 'matches the mock' is a tautology rather than a claim"
        );

        // ---- ★ R1, BOTH HALVES, with three vCPU threads and a reactor thread running.
        assert_eq!(
            a.syscall_ranked_depth,
            (0, 0),
            "({name}) ★ R1: every syscall-shaped Vmm method ran with ZERO ranked locks — \
             including the ones a vCPU thread issued from inside an exit dispatch"
        );
        assert_eq!(
            a.copy_leaf_depth_max, 0,
            "({name}) ★★ the adapter half of R1: the guest-memory memcpy runs OUTSIDE the \
             adapter's own map lock, which `lockwitness` is structurally blind to"
        );
        assert!(
            a.view_leaf_depth_max >= 1,
            "({name}) ★ NON-VACUITY for the line above: the map lock was really taken"
        );

        // ---- ★ the memslot-frequency gate: installs scale with WINDOWS.
        assert_eq!(
            a.memslot_installs,
            1 + N_VCPUS as u64,
            "({name}) ★ one install per window and not one more — a guest that took \
             {} exits installed exactly zero memslots",
            a.guest_exits
        );

        // ---- ★ the conservation ledger.
        assert_eq!(
            (a.live_windows, a.live_memslots, a.window_bytes),
            (0, 0, 0),
            "({name}) ★ nothing leaked across three running guests, a mid-storm teardown, a \
             worker death and a deferred deadline"
        );
        assert_eq!(
            a.peak_windows,
            1 + N_VCPUS as u64,
            "({name}) ★ NON-VACUITY for the ledger: it counted every window live at once"
        );
    }

    assert_eq!(
        degenerate.mode_independent(),
        sharded.mode_independent(),
        "★ the lock configuration is not observable through a real guest: the \
         script-determined half of the run must be identical in both modes"
    );
}

/// ★ **The stop flag really stops a running guest** — closing a bite-check that did not
/// bite in the composed run above, where the exit *budget* was silently doing the work.
///
/// Here the budget is effectively infinite, so the only thing that can end the guest is the
/// flag a teardown sets. The assertion is the exact [`StopReason`], not "it returned".
#[test]
fn a_running_guest_is_stopped_by_the_flag_a_teardown_sets_not_by_a_budget() {
    kayfabe_linux_raw::require_kvm!(
        "a_running_guest_is_stopped_by_the_flag_a_teardown_sets_not_by_a_budget"
    );
    use kayfabe_vmm_kvm::vcpu::StopReason;
    let _wd = watchdog(
        "a_running_guest_is_stopped_by_the_flag_a_teardown_sets_not_by_a_budget",
        Duration::from_secs(120),
    );
    let (device, _pids, _rec) = mean_world(LockMode::Sharded);
    let machine = real_machine();
    let page = host_page();

    let _code = machine
        .install_ram_window(RGPA_CODE, page)
        .expect("the code window");
    let _probe = machine
        .install_ram_window(RGPA_PROBE, page)
        .expect("the probe window");
    let mut vmm = machine.vmm();
    vmm.gpa_write(
        RGPA_CODE,
        &kayfabe_tests::probe_loop_image(u32::try_from(RGPA_PROBE).expect("below 4 GiB")),
    )
    .expect("the image lands");
    vmm.gpa_write(RGPA_PROBE, &vcpu_cookie(0).to_le_bytes())
        .expect("the cookie lands");

    let doorbell = Arc::new(kayfabe_tests::DoorbellDevice::new(
        Arc::clone(&device),
        vec![kayfabe_tests::DoorbellLane {
            gpu: GPU0,
            cookie: vcpu_cookie(0),
            token: MockArch::token_for(lane_of(0).ce),
        }],
    ));
    let mut runner = machine
        .create_vcpu(0, Arc::clone(&doorbell) as _)
        .expect("a real vCPU");
    runner
        .enter_at(
            RGPA_CODE,
            kayfabe_tests::DoorbellDevice::lane_gpa(RGPA_BAR0, 0),
        )
        .expect("flat protected mode");

    let stop = Arc::new(AtomicBool::new(false));
    let reason = thread::scope(|sc| {
        let s = Arc::clone(&stop);
        // ★ u64::MAX exits: the budget CANNOT be what ends this run.
        let t = sc.spawn(move || runner.run_until(&s, u64::MAX).expect("KVM_RUN"));
        // Wait until the guest is provably running (a condition on ITS progress, never a
        // sleep), then stop it — which is what every teardown path does.
        let deadline = WallInstant::now() + Duration::from_secs(60);
        while doorbell.tally().rang == 0 {
            assert!(WallInstant::now() < deadline, "the guest never ran");
            thread::yield_now();
        }
        stop.store(true, Ordering::Release);
        t.join().expect("the vCPU thread joins")
    });
    assert_eq!(
        reason,
        StopReason::Stopped,
        "★ a guest with an unbounded budget must end because the stop flag was set — the \
         mechanism a teardown actually uses. `BudgetSpent` here would mean the composed \
         run's stop path was never exercised at all"
    );
    assert!(
        doorbell.tally().rang > 0 && doorbell.tally().cookie_mismatch == 0,
        "and it was doing real work when it was stopped: {:?}",
        doorbell.tally()
    );
}

/// ★★ **Where a multi-span window's memslots point** — the other half of §14.8 F7(a),
/// closing a bite-check that did not bite in the composed run because every window there
/// is a single span.
///
/// A read-native overlay (§6.1 group 7) is the one thing that installs **three** memslots
/// for one window: read-write either side of a page-aligned write-trap sub-range, and
/// read-only across it. Only the second and third carry a nonzero window offset — so
/// neutering that offset is invisible to any test whose windows all start at zero.
///
/// Here a guest reads out of the **third** span. If the install ignored the span's offset,
/// that memslot would point at the window's base and the guest would read the first span's
/// bytes instead, which the cookie makes unmistakable.
#[test]
fn a_guest_reading_from_a_read_native_windows_third_span_gets_that_spans_bytes() {
    kayfabe_linux_raw::require_kvm!(
        "a_guest_reading_from_a_read_native_windows_third_span_gets_that_spans_bytes"
    );
    let _wd = watchdog(
        "a_guest_reading_from_a_read_native_windows_third_span_gets_that_spans_bytes",
        Duration::from_secs(120),
    );
    let (device, _pids, _rec) = mean_world(LockMode::Sharded);
    let machine = real_machine();
    let page = host_page();

    // The code lives in an ordinary window; the overlay is what is under test.
    let _code = machine
        .install_ram_window(RGPA_CODE, page)
        .expect("the code window");
    let overlay_gpa = RGPA_PROBE;
    let third_span = overlay_gpa + 2 * page;
    let slot = {
        let mut vmm = machine.vmm();
        vmm.map_read_native(
            overlay_gpa,
            3 * page,
            // `u64::MAX` is this backend's "fill it from nothing": the overlay's pages are
            // the window's own anonymous ones, which is all this test needs.
            kayfabe_vmm::HostRegion {
                id: u64::MAX,
                offset: 0,
            },
            Some(overlay_gpa + page..overlay_gpa + 2 * page),
        )
        .expect("a read-native overlay")
    };
    assert_eq!(
        machine.audit().memslot_installs,
        1 + 3,
        "★ the overlay really installed THREE memslots — one per span. With one, the \
         offset this test is about would not exist"
    );

    // The first span gets a decoy; the third gets the cookie the guest must present.
    let mut vmm = machine.vmm();
    vmm.gpa_write(overlay_gpa, &0xDECA_F000u32.to_le_bytes())
        .expect("the decoy lands in the FIRST span");
    vmm.gpa_write(third_span, &vcpu_cookie(1).to_le_bytes())
        .expect("the cookie lands in the THIRD span");
    vmm.gpa_write(
        RGPA_CODE,
        &kayfabe_tests::probe_loop_image(u32::try_from(third_span).expect("below 4 GiB")),
    )
    .expect("the image lands");

    let doorbell = Arc::new(kayfabe_tests::DoorbellDevice::new(
        Arc::clone(&device),
        vec![kayfabe_tests::DoorbellLane {
            gpu: GPU0,
            cookie: vcpu_cookie(1),
            token: MockArch::token_for(lane_of(0).ce),
        }],
    ));
    let mut runner = machine
        .create_vcpu(0, Arc::clone(&doorbell) as _)
        .expect("a real vCPU");
    runner
        .enter_at(
            RGPA_CODE,
            kayfabe_tests::DoorbellDevice::lane_gpa(RGPA_BAR0, 0),
        )
        .expect("flat protected mode");
    let stop = AtomicBool::new(false);
    runner.run_until(&stop, 8).expect("KVM_RUN");

    let t = doorbell.tally();
    assert_eq!(
        (t.cookie_mismatch, t.unbacked_cookie),
        (0, 0),
        "★ the guest read the THIRD span's bytes. A memslot installed at the window's base \
         regardless of its span offset would have delivered the first span's decoy — which \
         is exactly what `cookie_mismatch` counts. {t:?}"
    );
    assert!(
        t.rang >= 1,
        "★ NON-VACUITY: the guest presented the cookie at least once, so the equality above \
         is about accesses that happened. {t:?}"
    );
    // And the overlay tears down as one window, memslots and all.
    let mut vmm = machine.vmm();
    vmm.unmap_guest(slot).expect("the overlay goes");
    assert_eq!(
        (machine.audit().live_memslots, machine.audit().live_windows),
        (1, 1),
        "removing the overlay removed all THREE of its memslots, leaving only the code \
         window"
    );
}

/// ★ **A wedged guest is LOUD.** Closing a bite-check that did not bite: mislabelling a
/// faulted guest as "its budget ran out" was invisible, because no guest in the composed
/// run ever faults — a classification whose input never occurs is a classification nothing
/// tests.
///
/// So one is made to fault, in the way a real one does: entered at a guest-physical address
/// no memslot covers, i.e. an instruction fetch from nothing. KVM cannot emulate that and
/// says so, and the runner must carry the reason out by name rather than resuming a guest
/// that has lost its way.
///
/// ★★ **The assertion is the END-STATE, not the host's mechanism** (2026-07-27). This test
/// used to assert `KVM_RUN itself succeeds` and then match the stop reason. That is a claim
/// about *how the host reported the fault*, and it is **host-dependent** — measured on two
/// boxes:
///
/// - **7.0.0-14, AMD/SVM** (dev box): `KVM_RUN` succeeds and the fault arrives as an
///   ordinary exit, so the runner returns `Ok(StopReason::GuestFaulted(..))`.
/// - **6.8.0-124** (the GPU bench): `KVM_RUN` is refused outright and the runner returns
///   `Err(VmmError::HostRefused { what: "entering the guest", .. })`, errno 28.
///
/// ★ The two boxes differ in **kernel version AND host CPU vendor**, and which of the two
/// is the discriminator has NOT been measured — a `KVM_RUN` refusal on a VM with zero user
/// memslots is the shape of VMX's private rmode/identity-map setup, which has no SVM
/// counterpart, so "6.8 vs 7.0" is a plausible label for an Intel-vs-AMD fact. Do not
/// quote it as a kernel-version finding; it is a host-divergence finding with two live
/// candidates. Either way it is a *mechanism*, which is exactly why the assertion below is
/// not about one.
///
/// Same fact, two reports. The old shape did not merely fail on the bench: because that box
/// has `/dev/kvm`, this test *ran* there, panicked, and `cargo mutants` aborted with
/// "cargo test failed in an unmutated tree" — a host detail was blocking the whole
/// mutation gate.
///
/// The durable claim is **"the guest is never resumed"**, and it is asserted *outside* the
/// match so it holds whichever way the kernel reported. Both arms are still exact
/// (`testing_doctrine.md` §2 rule 3): neither is `is_err()` and neither is
/// `matches!(.., _)` — a third outcome (`BudgetSpent`, `Halted`, a refusal of some *other*
/// operation) still fails. What is deliberately **not** pinned is the `errno` number: that
/// is one level further into the mechanism than the thing this fix is about, and it is the
/// only field here that was not measured on the box running the assertion. It is asserted
/// *present* and printed instead.
#[test]
fn a_guest_entered_at_an_address_with_no_memory_faults_by_name_and_is_never_resumed() {
    kayfabe_linux_raw::require_kvm!(
        "a_guest_entered_at_an_address_with_no_memory_faults_by_name_and_is_never_resumed"
    );
    use kayfabe_linux_raw::VcpuExit;
    use kayfabe_vmm::VmmError;
    use kayfabe_vmm_kvm::vcpu::StopReason;
    let _wd = watchdog(
        "a_guest_entered_at_an_address_with_no_memory_faults_by_name_and_is_never_resumed",
        Duration::from_secs(120),
    );
    let (device, _pids, _rec) = mean_world(LockMode::Sharded);
    let machine = real_machine();
    let doorbell = Arc::new(kayfabe_tests::DoorbellDevice::new(
        Arc::clone(&device),
        vec![kayfabe_tests::DoorbellLane {
            gpu: GPU0,
            cookie: vcpu_cookie(0),
            token: MockArch::token_for(lane_of(0).ce),
        }],
    ));
    let mut runner = machine
        .create_vcpu(0, Arc::clone(&doorbell) as _)
        .expect("a real vCPU");
    // Not one memslot exists on this machine: the very first instruction fetch has nowhere
    // to come from.
    runner
        .enter_at(
            RGPA_CODE,
            kayfabe_tests::DoorbellDevice::lane_gpa(RGPA_BAR0, 0),
        )
        .expect("flat protected mode");

    let stop = AtomicBool::new(false);
    // ★ TWO EXACT ARMS, one per measured host report. Not a disjunction weakened to
    // "something went wrong": each arm names precisely what it accepts.
    match runner.run_until(&stop, 64) {
        Ok(reason) => assert_eq!(
            reason,
            StopReason::GuestFaulted(VcpuExit::InternalError),
            "★ a guest that cannot execute must be reported as FAULTED, by name, carrying \
             the exit KVM gave. Reporting it as `BudgetSpent` or `Halted` would let a run \
             where every guest died on its first instruction satisfy every count in the \
             composed run above. A DIFFERENT `VcpuExit` here is also a finding, not a \
             nuisance: it means this kernel classifies a fetch-from-nothing some other \
             way, and the arm should be widened to that measured variant BY NAME — never \
             to `GuestFaulted(_)`"
        ),
        Err(VmmError::HostRefused { what, errno }) => {
            assert_eq!(
                what, "entering the guest",
                "★ the refusal must be the one from `KVM_RUN` itself. A `HostRefused` \
                 naming some OTHER operation means this test faulted somewhere it did not \
                 intend to and would be green for the wrong reason"
            );
            assert!(
                errno.is_some(),
                "★ NON-VACUITY on the refusal: the host's error number survived the port \
                 into `VmmError` (that is the whole reason `HostRefused` carries one \
                 rather than being an `Unsupported(&str)`). A refusal that reached us as \
                 'it failed' with no number would make this arm unfalsifiable"
            );
        }
        other => panic!(
            "★ a guest entered with NO memory has exactly two measured outcomes — KVM \
             admits it and reports the fault as an exit (7.0.0-14/AMD), or KVM refuses \
             the entry (6.8.0-124). This is a third. Measure it, then add a THIRD EXACT \
             ARM; do not relax the two above: {other:?}"
        ),
    }
    assert_eq!(
        runner.report().exits,
        0,
        "★ THE CLAIM, and it is asserted outside the match so it holds whichever way the \
         kernel reported: it was never dispatched into the device. A guest that has lost \
         its way is not something to resume, and nothing it 'did' reached the core"
    );
    assert_eq!(
        machine.audit().guest_exits,
        0,
        "the device-wide exit counter agrees — the two instruments cannot drift"
    );

    // ---- ★ NON-VACUITY, in-test: the two zeros above are LIVE INSTRUMENTS.
    //
    // Every assertion so far is `== 0`, and a counter that can never be anything else
    // satisfies all of them. So the same machine, the same audit and a second vCPU are
    // made to do the thing the first one could not — with memory this time — and both
    // instruments must move. Without this, a `run_until` that returned before entering the
    // guest at all, or a `report()` wired to a constant, would be indistinguishable from
    // the property under test.
    let page = host_page();
    let _code = machine
        .install_ram_window(RGPA_CODE, page)
        .expect("the code window");
    let _probe = machine
        .install_ram_window(RGPA_PROBE, page)
        .expect("the probe window");
    {
        let mut vmm = machine.vmm();
        vmm.gpa_write(
            RGPA_CODE,
            &kayfabe_tests::probe_loop_image(u32::try_from(RGPA_PROBE).expect("below 4 GiB")),
        )
        .expect("the image lands");
        vmm.gpa_write(RGPA_PROBE, &vcpu_cookie(0).to_le_bytes())
            .expect("the cookie lands");
    }
    let mut live = machine
        .create_vcpu(1, Arc::clone(&doorbell) as _)
        .expect("a second real vCPU");
    live.enter_at(
        RGPA_CODE,
        kayfabe_tests::DoorbellDevice::lane_gpa(RGPA_BAR0, 0),
    )
    .expect("flat protected mode");
    let go = AtomicBool::new(false);
    live.run_until(&go, 8).expect("the BACKED guest runs");
    assert!(
        live.report().exits > 0 && machine.audit().guest_exits > 0,
        "★ NON-VACUITY: a vCPU that DOES have memory moved both counters, so the two \
         `== 0` assertions above are about instruments that can be non-zero — they are \
         the guest not being resumed, not a counter that never counts. {:?} / {}",
        live.report(),
        machine.audit().guest_exits
    );
}

// =================================================================================
// ★★ THE TRACE ARM (`kayfabe-trace`) — the replay vocabulary under the SAME mean load
//
// `testing_doctrine.md` §3.1 obligation 3: the mean test is where a new milestone gets
// wired, not a fresh isolated file — because §3's incident is that isolated cases go
// green for the wrong reason. So the trace crate's two load-bearing claims are asserted
// HERE, against the six-proc / two-GPU world, multi-threaded, with a host verb PARKED:
//
//   1. **The order is total across threads when the recorder is shared** — dense from
//      zero, gapless, exact count. That is the whole basis of a replay differential, and
//      a per-thread recorder (the shape that looks equally reasonable) provably is not:
//      `trace_replay.rs::two_recorders_do_not_share_an_order`.
//   2. **The trace is conserved** — every op each thread performed appears exactly once,
//      routed to its OWN proc and its OWN GPU. A trace that lost or duplicated events
//      under contention would make every differential built on it a coin flip.
//
// The parked verb is not decoration: it is what makes the run *mean*. The workload
// threads are joined while the latch is STILL pending, so the recorded event count is a
// progress edge (§8.3: never a clock), and the trace is being written by six threads
// while a seventh is blocked inside the mock backend.
// =================================================================================

mod trace_arm {
    use super::*;
    use kayfabe_trace::{
        CompletionOp, Counters, Dispatched, EventKind, Faulted, Outcome, ProcRef, Recorder,
        Resolved, RouteKey, Routed, Seq, TraceEvent, TraceLog, check_dense_order,
    };

    /// Ops per traced workload thread. Small on purpose — this arm is about ORDER and
    /// CONSERVATION under contention, and the composed run above already carries the
    /// heavy op counts.
    const TRACE_OPS: usize = 150;
    /// Events each op emits: bind, resolve, doorbell, poll.
    const EVENTS_PER_OP: usize = 4;
    /// The traced threads' VA lane base (disjoint from every other lane in this file).
    const VA_TRACE: u64 = 0x100_0000_0000;

    /// One traced workload thread: publish → read back → ring → poll, emitting from the
    /// REAL return value of each op into the SHARED recorder.
    fn traced_workload(
        dev: &SharedDevice,
        rec: &Mutex<Recorder<TraceLog>>,
        pids: &[ProcId],
        i: usize,
    ) {
        let (lane, gpu, pid) = (lane_of(i), gpu_of(i), pids[i]);
        for k in 0..TRACE_OPS {
            let va = GpuVa(VA_TRACE + (i as u64) * 0x1_0000_0000 + (k as u64) * 0x1000);

            let p = dev
                .publish_backing(gpu, lane.pdb, va, 0x1000)
                .expect("the traced thread publishes while a sibling's verb is parked");
            let (b, off) = dev
                .resolve(gpu, lane.pdb, GpuVa(va.0 + 0x40))
                .expect("resolves");
            let out = dev.doorbell(gpu, MockArch::token_for(lane.gr), &[va]);
            let batch = dev.completion_poll(gpu, pid, Instant(k as u64));
            let posted = batch.as_ref().map(|x| x.batch);
            let outstanding = batch.as_ref().map_or(0, |x| x.events.len());
            if batch.is_some() {
                dev.completions_drained(gpu);
            }
            assert_eq!(lock::held_depth(), 0, "a traced op leaked a guard");

            // ONE lock acquisition, four events — so the four events of one op are
            // CONTIGUOUS in the total order. That is the adapter obligation the crate
            // docs state: the counter orders emissions, so an emitter that wants its own
            // ops atomic in the stream must emit them under one exclusion.
            let mut g = rec.lock().expect("recorder");
            let mut tr = g.trace();
            tr.emit(|| TraceEvent::Route {
                gpu,
                key: RouteKey::Pdb(lane.pdb),
                outcome: Routed::To(ProcRef(pid.0)),
            });
            tr.emit(|| TraceEvent::AddressResolve {
                gpu,
                pdb: lane.pdb,
                va: GpuVa(va.0 + 0x40),
                outcome: Resolved::Hit {
                    offset: off,
                    host: b.host,
                },
            });
            tr.emit(|| TraceEvent::Doorbell {
                gpu,
                vchid: lane.gr,
                token: MockArch::token_for(lane.gr),
                outcome: match &out {
                    Ok(o) => Dispatched::Rung {
                        proc: ProcRef(o.proc.0),
                        host_token: o.host_token,
                        scheduled_now: o.scheduled_now,
                    },
                    Err(e) => Dispatched::Refused(e.fault_tag()),
                },
            });
            tr.emit(|| TraceEvent::Completion {
                gpu,
                proc: ProcRef(pid.0),
                op: CompletionOp::Polled {
                    posted,
                    outstanding,
                },
            });
            drop(g);
            // The publication itself is a fact the assertions below re-derive, so it is
            // read here rather than ignored.
            assert_eq!(b.host_va(), Some(p.host_va));
        }
    }

    /// One composed traced run in `mode`. Returns the counters and the recorded stream.
    fn traced_mean_run(mode: LockMode) -> (Counters, Vec<kayfabe_trace::Record>) {
        let _w = watchdog("l1_mean::traced_mean_run", Duration::from_secs(180));
        let (device, pids, rec_verbs) = mean_world(mode);
        let dev: &SharedDevice = &device;
        let pid_ref: &[ProcId] = &pids;
        let trace = Mutex::new(Recorder::new(TraceLog::new()));

        // Warm-up, so a canary's parked verb is the one the script NAMES.
        for i in 0..N_PROCS {
            dev.publish_backing(gpu_of(i), lane_of(i).pdb, GpuVa(VA_WARM), 0x1000)
                .expect("warm-up publish");
            dev.doorbell(gpu_of(i), MockArch::token_for(lane_of(i).ce), &[])
                .expect("warm-up CE ring");
        }

        thread::scope(|sc| {
            // ★ Park ONE host verb on the witness's own isolate, and CONFIRM it parked
            // before any traced thread starts — so what follows is written while a
            // seventh thread is genuinely blocked inside the backend.
            let mut latches = Latches::new();
            latches.arm(&rec_verbs, pids[P_WITNESS], GPU0, 0, VerbKind::AllocSysmem);
            let parked = sc.spawn(move || {
                dev.publish_backing(GPU0, lane_of(P_WITNESS).pdb, GpuVa(VA_HELD), 0x1000)
            });
            latches.wait_all_pending();

            let workers: Vec<_> = (0..N_PROCS)
                .map(|i| {
                    let t = &trace;
                    sc.spawn(move || traced_workload(dev, t, pid_ref, i))
                })
                .collect();
            for w in workers {
                w.join().expect("no traced thread panicked");
            }

            // ★ THE PROGRESS EDGE (§8.3: never a clock). Every traced thread ran to
            // completion while the parked verb is STILL parked — so the stream below was
            // written under contention, not after it drained.
            assert!(
                latches.all_pending(),
                "the parked verb was released before the traced threads finished — the \
                 whole run happened after the contention, and proves nothing about it"
            );
            assert_eq!(
                trace.lock().expect("recorder").counters().total(),
                (N_PROCS * TRACE_OPS * EVENTS_PER_OP) as u64,
                "every traced op was recorded WHILE the verb was parked"
            );

            latches.release_all();
            parked.join().expect("the parked thread joins").expect(
                "the witness's held publish commits once released — a failure here means \
                 the latch, not the trace, is what this run measured",
            );
        });

        let rec = trace.into_inner().expect("recorder");
        (*rec.counters(), rec.into_sink().records().to_vec())
    }

    /// ★ The mean arm: one shared recorder, six threads, two GPUs, a parked host verb.
    #[test]
    fn the_shared_trace_is_totally_ordered_and_conserved_under_the_mean_load() {
        let mut per_mode = Vec::new();
        for mode in [LockMode::Degenerate, LockMode::Sharded] {
            let (counters, records) = traced_mean_run(mode);
            let expected = N_PROCS * TRACE_OPS * EVENTS_PER_OP;

            // (1) ★ THE ORDER: dense from zero, strictly increasing, across six emitters.
            assert_eq!(
                check_dense_order(&records, Seq(0)),
                Ok(()),
                "{mode:?}: a shared recorder must totally order concurrent emitters"
            );
            assert_eq!(
                records.len(),
                expected,
                "{mode:?}: nothing lost or duplicated"
            );
            assert_eq!(counters.total(), expected as u64);

            // (2) ★ NON-VACUITY, stated exactly: these four planes ran, the other eleven
            // did not. An unexpected silence is a plane that stopped emitting; an
            // unexpected noise is an event nobody asked for.
            assert_eq!(
                counters.seen_kinds(),
                vec![
                    EventKind::Route,
                    EventKind::AddressResolve,
                    EventKind::Doorbell,
                    EventKind::Completion,
                ],
                "{mode:?}: exactly the driven planes appear"
            );
            for k in counters.seen_kinds() {
                assert_eq!(
                    counters.of(k),
                    (N_PROCS * TRACE_OPS) as u64,
                    "{mode:?}: every thread emitted {k} once per op"
                );
            }

            // (3) ★ CONSERVATION + ROUTING, read out of the projection alone: every
            // doorbell rang on its OWN proc and its OWN GPU. Byte-identical vChid values
            // live on both GPUs in this world (MG-3), so a routing map that collapsed
            // `(GpuId, VChid)` shows up right here.
            let mut per_proc: BTreeMap<(u32, u32), usize> = BTreeMap::new();
            for r in &records {
                match &r.ev {
                    TraceEvent::Doorbell {
                        gpu,
                        vchid,
                        outcome: Dispatched::Rung { proc, .. },
                        ..
                    } => {
                        *per_proc.entry((proc.0, gpu.0)).or_default() += 1;
                        // The vChid is the lane's GR channel of exactly one proc.
                        assert!(
                            LANES.iter().any(|l| l.gr == *vchid),
                            "{mode:?}: a doorbell rang a vChid no lane owns"
                        );
                    }
                    TraceEvent::Doorbell {
                        outcome: Dispatched::Refused(t),
                        ..
                    } => panic!("{mode:?}: a traced doorbell was refused: {t}"),
                    _ => {}
                }
            }
            assert_eq!(
                per_proc.len(),
                N_PROCS,
                "{mode:?}: all six procs are represented, each under its own (proc, gpu)"
            );
            assert!(
                per_proc.values().all(|n| *n == TRACE_OPS),
                "{mode:?}: each proc's doorbells are conserved exactly: {per_proc:?}"
            );

            // (4) ★ Every AddressResolve HIT is host-published — the #14 gate's own
            // predicate, visible in the trace with no core access at all.
            let unpublished = records
                .iter()
                .filter(|r| {
                    matches!(
                        &r.ev,
                        TraceEvent::AddressResolve {
                            outcome: Resolved::Hit { host: None, .. },
                            ..
                        }
                    )
                })
                .count();
            assert_eq!(
                unpublished, 0,
                "{mode:?}: a traced resolve reported a bound-but-UNPUBLISHED range — that \
                 is the #14 EXECUTION fault's precondition, and the vocabulary is required \
                 to be able to say it (which is why this counts a named state, not an \
                 absence)"
            );

            per_mode.push(counters);
        }

        // (5) The two lock modes are two configurations of one design (§7 / review P5),
        // so the trace's plane counts must agree exactly.
        assert_eq!(
            per_mode[0], per_mode[1],
            "the trace's per-plane counts are a mode-INDEPENDENT fact; a disagreement \
             means one lock mode dropped or duplicated observations"
        );
        // ★ Non-vacuity for (5) itself: the thing being compared is not trivially empty.
        assert!(per_mode[0].total() > 1_000);
    }

    /// ★ The bite check for the assertions above, run as a test rather than by hand: the
    /// mean arm's `check_dense_order` claim must FAIL on a stream that lost one record.
    /// Without this, an ordering checker that always returned `Ok` would leave the mean
    /// arm green — the exact "green instrument on an unexercised path" shape.
    #[test]
    fn the_mean_arms_order_check_would_notice_a_lost_record() {
        let mut rec = Recorder::new(TraceLog::new());
        {
            let mut tr = rec.trace();
            for i in 0..8u64 {
                tr.emit(|| TraceEvent::Clock { ns: i });
            }
        }
        let whole = rec.sink().records().to_vec();
        assert_eq!(check_dense_order(&whole, Seq(0)), Ok(()));
        let mut lossy = whole.clone();
        lossy.remove(5);
        assert_eq!(
            check_dense_order(&lossy, Seq(0)),
            Err(kayfabe_trace::OrderingError::Gap {
                index: 5,
                expected: Seq(5),
                seq: Seq(6),
            }),
            "the mean arm's order assertion has teeth: one lost record is named, by index"
        );
        // And the refusal-carrying arm of the vocabulary is non-vacuous too: an `Outcome`
        // can actually be refused, so the mean arm's "was any doorbell refused?" panic is
        // reachable rather than decorative.
        assert_eq!(
            TraceEvent::Route {
                gpu: GPU0,
                key: RouteKey::Pdb(Pdb(0xbad)),
                outcome: Routed::Refused(
                    FwdFault::UnknownPdb {
                        gpu: GPU0,
                        pdb: Pdb(0xbad)
                    }
                    .fault_tag()
                ),
            }
            .refusal()
            .map(|t| t.0),
            Some("FwdFault::UnknownPdb")
        );
        assert_eq!(
            TraceEvent::RmApply {
                gpu: Some(GPU0),
                client: client_of(0),
                handle: H_VASPACE,
                verb: kayfabe_trace::RmVerb::Free,
                outcome: Outcome::Ok,
            }
            .refusal(),
            None
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────
// ★★ The faked GSP, composed into this run (`mode2_gsp_port_plan.md` S3/S4;
// `testing_doctrine.md` §3.1.3: a milestone is done when it is wired into THIS file, not
// only into a fresh isolated one).
// ─────────────────────────────────────────────────────────────────────────────────────

/// The GSP boot/teardown cycle **while the device planes are under real load**.
///
/// §3's rule, applied to a new subsystem: isolated cases test what you thought of,
/// composed runs test what you didn't. The isolated GSP suite (`gsp_boot.rs`) drives the
/// protocol against an independent guest; this drives the same lifecycle *concurrently
/// with* the multi-proc, multi-GPU control-plane workload the rest of this file exists
/// for — three full driver lifetimes interleaved with alloc/map/publish traffic on two
/// GPUs, all in one process.
///
/// What it can find that the isolated suite cannot: that the GSP plane is a **plain
/// value** with no hidden global — no static, no lazily-initialised singleton, no
/// process-wide latch. If any of the C's eight scattered latches had survived as a
/// `static`, a second `GspWorld` in the same process would see the first one's state.
///
/// Would the isolated case have been green? Yes — and that sentence is the reason this
/// one exists.
#[test]
fn the_gsp_boot_cycle_composes_with_live_multiproc_traffic() {
    let _wd = watchdog(
        "the_gsp_boot_cycle_composes_with_live_multiproc_traffic",
        Duration::from_secs(120),
    );
    let (world, pids, _rec) = mean_world(LockMode::Sharded);
    let device = Arc::clone(&world);

    // Two independent GSP devices in one process, on the two register models, cycling in
    // lockstep — the "no hidden global" probe.
    let mut a = GspWorld::new(P580, MODEL_A);
    let mut b = GspWorld::new(P610, MODEL_B);

    // ★ Bounded, deliberately. The first version of this test ran its workers *until a
    // stop flag*, and that is a defect rather than a stronger test: two threads spinning
    // an unbounded alloc/map workload starved the rest of this file's tests and made two
    // of them fail intermittently. A flaky green is worse than a red one
    // (`testing_doctrine.md` §1), and the composition being tested — GSP lifecycle
    // *concurrent with* device load — needs the two to overlap, not to run forever.
    let workers: Vec<_> = (0..2)
        .map(|i| {
            let device = Arc::clone(&device);
            let pids = pids.clone();
            thread::spawn(move || {
                // ONE pass per proc: `ctl_workload` publishes at fixed VAs, so a second
                // pass overlaps its own bindings and fails with `Address(Overlap)` — which
                // is the address plane being right, not the workload being flaky. Found by
                // running this test twice.
                ctl_workload(&device, &pids, i)
            })
        })
        .collect();

    let mut cycles = 0usize;
    for life in 0..3 {
        for (tag, w) in [("A", &mut a), ("B", &mut b)] {
            if life > 0 {
                w.allocate_guest_memory();
            }
            assert_eq!(
                w.boot(),
                vec![Transition::E1, Transition::E6, Transition::E5],
                "{tag} life {life}: the boot is unaffected by the device traffic beside it",
            );
            let msgs = w.link_and_drain();
            assert_eq!(msgs.len(), 1, "{tag} life {life}: INIT_DONE");
            assert_eq!(
                msgs[0].seq_num, 0,
                "{tag} life {life}: a new queue instance starts its stream at 0",
            );

            w.guest
                .send(&mut w.ram, 76, 900 + life as u32, &[0xC5; 24])
                .unwrap();
            w.doorbell().unwrap();
            assert_eq!(w.guest.recv(&mut w.ram).unwrap().len(), 1, "{tag}: reply");
            assert_eq!(w.fsm.phase(), BootPhase::Running, "{tag} life {life}");

            // ★★ GSP-S1, composed: a command declaring more elements than the guest's
            // 65 536-byte receive staging buffer can hold is refused **by name** while
            // the device planes are under load, and the device keeps serving afterwards.
            // Only on the profile that HAS the field — at 610 there is nothing to
            // corrupt, which is itself the version seam being right.
            if tag == "A" {
                w.guest
                    .send(&mut w.ram, 76, 800 + life as u32, &[0x11; 16])
                    .unwrap();
                let slot = w.last_command_slot();
                // 48/32 are the 580 element's hdr and checkSum offsets, passed
                // explicitly so this does not read them from the table under test
                // (`ogkm-580: message_queue_priv.h:43-51`).
                w.poke_element(RingId::Command, slot, 48, 32, 40, 62);
                assert_eq!(
                    w.doorbell(),
                    Err(GspFault::ElementCountOutOfRange { count: 62, max: 16 }),
                    "{tag} life {life}: the exact refusal, under concurrent device load",
                );
                assert_eq!(
                    w.fsm.phase(),
                    BootPhase::Running,
                    "{tag} life {life}: a refusal is per-MESSAGE — the device never stops",
                );
                // …and the ring did not move, so the guest can rewrite the element.
                w.poke_element(RingId::Command, slot, 48, 32, 40, 1);
                let r = w.doorbell().expect("the corrected command is served");
                assert_eq!(r.commands.len(), 1, "{tag} life {life}: recovered");
                assert_eq!(w.guest.recv(&mut w.ram).unwrap().len(), 1);
            }

            w.guest
                .send(&mut w.ram, 47, 950 + life as u32, &[])
                .unwrap();
            w.doorbell().unwrap();
            assert_eq!(
                w.guest.recv(&mut w.ram).unwrap().len(),
                1,
                "{tag}: fn-47 ack"
            );
            let start = w.arch.model().startcpu();
            w.wr(GspReg::GspFalconCpuctl, start).unwrap();
            assert_eq!(w.fsm.phase(), BootPhase::Halted, "{tag} life {life}");
            assert_eq!(*w.fsm.queue(), QueueState::Unbound, "{tag} life {life}");
            cycles += 1;
        }
    }
    assert_eq!(
        cycles, 6,
        "three lifetimes on each of two devices actually ran"
    );

    let published: usize = workers.into_iter().map(|h| h.join().expect("worker")).sum();
    assert!(
        published > 0,
        "non-vacuity: the device planes really were under load for the whole cycle",
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────
// ★★ N3 — ONE `Proc`, TWO GPUs: the isolate identity has to separate them
// ─────────────────────────────────────────────────────────────────────────────────────
//
// Every proc in the world above lives on exactly ONE GPU, and that is precisely why the
// defect these tests pin survived: the shape that breaks it is a **single guest process
// with a `Device` on each `deviceInstance`** — one dup-component, one `ProcId`, one
// `Proc`, and (MG-5) TWO isolates. `IsolateId` used to be the `ProcId` alone, so those
// two isolates wore ONE id, and:
//
//   * `HostHandle::belongs_to` answered `true` across them, so `Worker::execute`'s
//     foreign-handle gate — the ONE place the `(Proc, GpuId)`-scoped-handle rule is
//     enforced — enforced only the `Proc` half. **Measured**, before the fix: a host
//     object minted on the proc's GPU0 isolate, presented in a control aimed at its GPU1
//     isolate, sailed through the gate and reached the backend. It came back
//     `RmError::BadHandle` only because `MockRmBackend` validates handles against its own
//     per-isolate namespace — the "backend's luck" `HostHandle`'s own docs say a real
//     host does NOT provide (RM mints every client's handles from one
//     `RS_CLIENT_HANDLE_BASE`, so the same raw value is live-and-unrelated in the other
//     isolate's client, and the verb would have hit a bystander object).
//   * every per-isolate ACCOUNT keyed on the id merged the two — the `HostLedger`, the
//     verb log, the cancel census, the teardown post-condition. An instrument that cannot
//     separate a proc's two isolates cannot witness the first bullet either.

/// The straddler's client — ONE guest process holding a `Device` on each GPU.
const N3_CLIENT: HClient = HClient(0xC0);
/// The bystander on GPU0.
const N3_BY0_CLIENT: HClient = HClient(0xC1);
/// The bystander on GPU1.
const N3_BY1_CLIENT: HClient = HClient(0xC2);
/// The straddler's page-directory base — the **same value on both of its GPUs**, which
/// is legal (`Pdb` is a per-GPU namespace) and is the shape a `(GpuId, ·)` collapse
/// mis-routes on.
const N3_PDB: Pdb = Pdb(0x3B00_0000);
/// The straddler's GR vChid — likewise identical on both GPUs.
const N3_GR: VChid = VChid(0x400);
/// The straddler's CE vChid.
const N3_CE: VChid = VChid(0x401);
/// The bystanders' PDB (identical on the two GPUs, distinct from the straddler's).
const N3_BY_PDB: Pdb = Pdb(0x3B10_0000);
/// The bystanders' GR vChid.
const N3_BY_GR: VChid = VChid(0x410);
/// The bystanders' CE vChid.
const N3_BY_CE: VChid = VChid(0x411);
/// The straddler's GPU0 publication lane.
const VA_N3_G0: u64 = 0x60_0000_0000;
/// The straddler's GPU1 publication lane.
const VA_N3_G1: u64 = 0x64_0000_0000;
/// The bystanders' lane.
const VA_N3_BY: u64 = 0x68_0000_0000;
/// The lane the straddler's two ring threads publish into.
const VA_N3_RING: u64 = 0x6c_0000_0000;

/// The handles of the straddler's SECOND `Device` — distinct within its one client
/// namespace, because a client's handle namespace is singular even though its devices
/// are not.
fn n3_second_device_handles() -> kayfabe_tests::ProcessHandles {
    kayfabe_tests::ProcessHandles {
        client_root: HObject(0x5c00_0000),
        device: HObject(0x5d00_0001),
        vaspace: HObject(0x5d00_0010),
        tsg: HObject(0x5d00_0012),
        gr_channel: HObject(0x5d00_0019),
        gr_vchid: N3_GR,
        ce_channel: HObject(0x5d00_001a),
        ce_vchid: N3_CE,
    }
}

/// Script a SECOND `Device` (and its VASpace + TSG + channels) under an **already
/// declared** client — the straddle. Deliberately not `compute_process_on_gpu`, which
/// re-emits the client root.
fn n3_push_second_device(
    s: &mut Scenario,
    client: HClient,
    h: &kayfabe_tests::ProcessHandles,
    pdb: Pdb,
    instance: u32,
) {
    s.push(RmEvent::Alloc {
        client,
        parent: h.client_root,
        handle: h.device,
        class: mock_classes::DEVICE,
        facts: AllocFacts {
            device_instance: Some(instance),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client,
        parent: h.device,
        handle: h.vaspace,
        class: mock_classes::VASPACE,
        facts: AllocFacts::default(),
    });
    s.push(RmEvent::SetPageDir {
        client,
        vaspace: h.vaspace,
        pdb,
    });
    s.push(RmEvent::Alloc {
        client,
        parent: h.device,
        handle: h.tsg,
        class: mock_classes::TSG,
        facts: AllocFacts {
            h_vaspace: Some(h.vaspace),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client,
        parent: h.tsg,
        handle: h.gr_channel,
        class: mock_classes::CHANNEL_GR,
        facts: AllocFacts {
            h_vaspace: Some(h.vaspace),
            userd_flags: MockArch::userd_flags_for(h.gr_vchid),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client,
        parent: h.tsg,
        handle: h.ce_channel,
        class: mock_classes::CHANNEL_CE,
        facts: AllocFacts {
            h_vaspace: Some(h.vaspace),
            userd_flags: MockArch::userd_flags_for(h.ce_vchid),
            ..Default::default()
        },
    });
}

/// The N3 world: the straddler (one `Proc`, both GPUs) plus a bystander on each GPU, so
/// every assertion below runs multi-process as well as multi-GPU. Returns
/// `(device, straddler, bystander0, bystander1, recorder)`.
fn n3_world(
    mode: LockMode,
) -> (
    Guarded<Arc<SharedDevice>>,
    ProcId,
    ProcId,
    ProcId,
    SharedRecorder,
) {
    let (gpu, s, by0, by1, rec) = n3_gpu();
    (
        gpu.map(|g| Arc::new(SharedDevice::new(g, mode))),
        s,
        by0,
        by1,
        rec,
    )
}

/// The same world as a **bare [`Gpu`]**, for the core-level property tests (§8.2's T1
/// tier): the isolate identity is a fact of the pure core, so the tests that pin its
/// accounting read core state directly rather than through the lock shell.
fn n3_gpu() -> (Guarded<Gpu>, ProcId, ProcId, ProcId, SharedRecorder) {
    let arch = Box::new(MockArch::new());
    let (factory, recorder) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x10_0000_0000..0x1000_0000_0000, 0x10_0000_0000);
    let mut gpu = Gpu::realize(arch, Box::new(factory), gpa, &[GPU0, GPU1])
        .expect("the two-GPU device realizes");

    let mut s = Scenario::new();
    // The straddler: device 0 on GPU0 …
    s.compute_process_on_gpu(
        N3_CLIENT,
        N3_PDB,
        identical_handles(N3_GR.0, N3_CE.0),
        Some(GPU0.0),
    );
    // … and device 1 on GPU1, under the SAME client root.
    n3_push_second_device(
        &mut s,
        N3_CLIENT,
        &n3_second_device_handles(),
        N3_PDB,
        GPU1.0,
    );
    // Two ordinary single-GPU bystanders, one per target, sharing PDB/vChid VALUES.
    s.compute_process_on_gpu(
        N3_BY0_CLIENT,
        N3_BY_PDB,
        identical_handles(N3_BY_GR.0, N3_BY_CE.0),
        Some(GPU0.0),
    );
    s.compute_process_on_gpu(
        N3_BY1_CLIENT,
        N3_BY_PDB,
        identical_handles(N3_BY_GR.0, N3_BY_CE.0),
        Some(GPU1.0),
    );
    for ev in s.events {
        gpu.apply(ev).expect("the N3 scenario applies cleanly");
    }

    let straddler = gpu.spine.by_pdb[&(GPU0, N3_PDB)];
    assert_eq!(
        gpu.spine.by_pdb[&(GPU1, N3_PDB)],
        straddler,
        "one client with a Device on each GPU is ONE proc — the straddle premise"
    );
    let by0 = gpu.spine.by_pdb[&(GPU0, N3_BY_PDB)];
    let by1 = gpu.spine.by_pdb[&(GPU1, N3_BY_PDB)];
    assert_eq!(
        gpu.procs.len(),
        3,
        "three guest procs: the straddler and one bystander per GPU"
    );
    assert_eq!(
        gpu.procs[&straddler]
            .isolates
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![GPU0, GPU1],
        "★ MG-5: the straddler owns TWO isolates — one per target"
    );
    (
        Guarded::new("l1_mean::n3_world", gpu, recorder.clone()),
        straddler,
        by0,
        by1,
        recorder,
    )
}

/// ★★ **THE BITE.** A host handle minted in a proc's GPU0 isolate, presented on the
/// SAME proc's GPU1 isolate, must be refused by [`kayfabe_isolate::Worker::execute`]'s
/// foreign-handle gate — with the EXACT [`RmError::ForeignHandle`], naming the exact
/// handle and the exact isolate that would have issued the verb.
///
/// Before N3 this refusal did not happen at all: `belongs_to` compared the `ProcId` half
/// only, the plan reached the backend, and what came back was the mock's own
/// `BadHandle` — an answer produced by the *test double*, not by the design. The three
/// asserts are therefore layered on purpose: the two isolates are distinct identities,
/// the gate refuses, and **nothing ran** (the ledger is byte-identical across the
/// refused call, which is the gate's "before any verb" promise made checkable).
///
/// `route_control` is the reachable path, not a synthetic one: it takes the target GPU
/// and the object handle as two independent guest-derived arguments, and nothing between
/// it and the gate cross-checks them.
#[test]
fn n3_a_cross_gpu_handle_is_refused_by_the_foreign_handle_gate() {
    let _wd = watchdog("n3_cross_gpu_gate", Duration::from_secs(60));
    for mode in [LockMode::Sharded, LockMode::Degenerate] {
        let (world, s, by0, by1, rec) = n3_world(mode);
        let device = Arc::clone(&world);

        // Materialize a host engine object on EACH of the straddler's two targets.
        let on0 = device
            .forward_engine_object(GPU0, N3_GR, kayfabe_tests::COMPUTE_CLASS, &[])
            .expect("({mode:?}) the straddler's GPU0 engine object forwards");
        let on1 = device
            .forward_engine_object(GPU1, N3_GR, kayfabe_tests::COMPUTE_CLASS, &[])
            .expect("({mode:?}) the straddler's GPU1 engine object forwards");

        // ---- (1) The identity itself: ONE proc, TWO isolate identities.
        assert_eq!(
            (on0.host_object.isolate(), on1.host_object.isolate()),
            (IsolateId::new(s.0, GPU0), IsolateId::new(s.0, GPU1)),
            "({mode:?}) a handle must record the (Proc, GpuId) namespace it was minted in"
        );
        assert_ne!(
            on0.host_object.isolate(),
            on1.host_object.isolate(),
            "★★ ({mode:?}) one Proc on two GPUs is TWO isolates with two RM client \
             namespaces; an IsolateId that names only the proc collapses them"
        );

        // ---- (2) The gate, in BOTH directions, with the EXACT variant.
        let ledger_before = rec.lock().expect("recorder").ledger();
        let mut payload = [0u8; 4];
        assert_eq!(
            device.route_control(
                GPU1,
                s,
                on0.host_object,
                mock_ctrl::FORWARDABLE,
                &mut payload
            ),
            Err(FwdFault::Rm(RmError::ForeignHandle {
                handle: on0.host_object,
                worker_isolate: IsolateId::new(s.0, GPU1),
            })),
            "★★ ({mode:?}) a GPU0 handle reached the SAME proc's GPU1 isolate — on a real \
             host that raw value names a different LIVE object in that isolate's client"
        );
        assert_eq!(
            device.route_control(
                GPU0,
                s,
                on1.host_object,
                mock_ctrl::FORWARDABLE,
                &mut payload
            ),
            Err(FwdFault::Rm(RmError::ForeignHandle {
                handle: on1.host_object,
                worker_isolate: IsolateId::new(s.0, GPU0),
            })),
            "★★ ({mode:?}) …and symmetrically in the other direction"
        );

        // ---- (3) The gate ran BEFORE any verb: nothing was allocated, freed or mapped.
        assert_eq!(
            rec.lock().expect("recorder").ledger(),
            ledger_before,
            "({mode:?}) the foreign-handle gate must refuse before the chain runs — the \
             refused calls moved the host ledger"
        );

        // ---- (4) …and the LEGAL directions are untouched: this is a separation, not a
        //          blanket refusal (the non-vacuity half — a `belongs_to` that always
        //          answered `false` would pass every assert above).
        assert_eq!(
            device.route_control(
                GPU0,
                s,
                on0.host_object,
                mock_ctrl::FORWARDABLE,
                &mut payload
            ),
            Ok(ControlRoute::Forwarded),
            "({mode:?}) a GPU0 handle on the GPU0 isolate is the ordinary case"
        );
        assert_eq!(
            device.route_control(
                GPU1,
                s,
                on1.host_object,
                mock_ctrl::FORWARDABLE,
                &mut payload
            ),
            Ok(ControlRoute::Forwarded),
            "({mode:?}) …and so is a GPU1 handle on the GPU1 isolate"
        );

        // ---- (5) The `Proc` axis the gate always held still holds — cross-PROC is
        //          refused with the same exact variant, on both GPUs.
        let by0_obj = device
            .forward_engine_object(GPU0, N3_BY_GR, kayfabe_tests::COMPUTE_CLASS, &[])
            .expect("bystander 0 forwards")
            .host_object;
        let by1_obj = device
            .forward_engine_object(GPU1, N3_BY_GR, kayfabe_tests::COMPUTE_CLASS, &[])
            .expect("bystander 1 forwards")
            .host_object;
        assert_eq!(
            device.route_control(GPU0, s, by0_obj, mock_ctrl::FORWARDABLE, &mut payload),
            Err(FwdFault::Rm(RmError::ForeignHandle {
                handle: by0_obj,
                worker_isolate: IsolateId::new(s.0, GPU0),
            })),
            "({mode:?}) another PROC's handle on this proc's isolate stays refused"
        );
        assert_eq!(
            (by0_obj.isolate(), by1_obj.isolate()),
            (IsolateId::new(by0.0, GPU0), IsolateId::new(by1.0, GPU1)),
            "({mode:?}) the bystanders' handles are in their own namespaces"
        );
    }
}

/// ★ The **accounting** half: the two isolates of one proc are separately auditable.
///
/// The mock's `HostLedger` keys on the isolate that ISSUED each verb, and every handle
/// independently records the namespace it was MINTED in (in the mock, in disjoint bit
/// lanes of the raw value). Asserting the two agree, per target, is what makes the
/// teardown post-condition able to see a cross-isolate misattribution at all — while one
/// id covered both targets, an object accounted against the *wrong* one of a proc's
/// isolates balanced perfectly.
#[test]
fn n3_b_the_two_isolates_of_one_proc_are_separately_accounted() {
    let _wd = watchdog("n3_separate_accounts", Duration::from_secs(60));
    let (world, s, _by0, _by1, rec) = n3_world(LockMode::Sharded);
    let device = Arc::clone(&world);

    // Real work on BOTH of the straddler's targets: publish, ring, forward.
    for (gpu, base) in [(GPU0, VA_N3_G0), (GPU1, VA_N3_G1)] {
        for k in 0..4u64 {
            device
                .publish_backing(gpu, N3_PDB, GpuVa(base + k * 0x1000), 0x1000)
                .expect("the straddler publishes on both of its targets");
        }
        device
            .doorbell(gpu, MockArch::token_for(N3_GR), &[])
            .expect("the straddler rings on both of its targets");
        device
            .forward_engine_object(gpu, N3_GR, kayfabe_tests::COMPUTE_CLASS, &[])
            .expect("the straddler forwards on both of its targets");
    }

    let ledger = rec.lock().expect("recorder").ledger();
    let g0 = ledger.leaked_on(IsolateId::new(s.0, GPU0));
    let g1 = ledger.leaked_on(IsolateId::new(s.0, GPU1));
    assert!(
        !g0.is_empty() && !g1.is_empty(),
        "non-vacuity: both of the straddler's isolates really did mint host objects \
         (g0={g0:?}, g1={g1:?})"
    );
    assert_eq!(
        g0.intersection(&g1).count(),
        0,
        "★ the two accounts must be disjoint — they are two RM client namespaces"
    );
    // ★ The cross-check that makes this non-circular: the ledger's KEY (the isolate that
    // issued the verb) and the handle's own recorded provenance must agree, and so must
    // the mock's independent raw-value lane.
    for (gpu, set) in [(GPU0, &g0), (GPU1, &g1)] {
        for h in set {
            assert_eq!(
                h.isolate(),
                IsolateId::new(s.0, gpu),
                "an object on {gpu:?}'s account was minted in another namespace: {h:?}"
            );
            assert_eq!(
                handle_lane(h.raw()),
                (s.0 + 1, gpu.0),
                "…and the mock's own value lane disagrees too: {h:?}"
            );
        }
    }
    // The verb LOG separates them for the same reason: no verb recorded on one target's
    // isolate may name a handle from the other's.
    for (gpu, other) in [(GPU0, &g1), (GPU1, &g0)] {
        let verbs = rec
            .lock()
            .expect("recorder")
            .verbs_of(IsolateId::new(s.0, gpu));
        assert!(
            !verbs.is_empty(),
            "non-vacuity: {gpu:?}'s isolate issued verbs"
        );
        for v in &verbs {
            if let RmVerb::Free { obj } | RmVerb::Schedule { chan: obj } = v {
                assert!(
                    !other.contains(obj),
                    "{gpu:?}'s isolate issued {v:?} naming the OTHER target's handle"
                );
            }
        }
    }
}

/// ★★ **The MEAN N3 run**: the straddler's two isolates, both with a verb parked, while
/// six threads across three procs and two GPUs make full progress — and the cross-GPU
/// gate is asserted *inside that window*, under contention, rather than on a quiet
/// device.
///
/// Composition is the point (§8.4). What this reaches that
/// [`n3_a_cross_gpu_handle_is_refused_by_the_foreign_handle_gate`] cannot:
///
/// - the two isolates of ONE `Proc` are independent **pools**, so parking a worker in
///   each does not stop either target — if the two ever collapsed onto one isolate, the
///   sibling threads would queue behind the parked verbs and the joins would never
///   return (the watchdog aborts, loudly, rather than the box's speed deciding);
/// - the refusal is a property of the gate, not of a quiet moment: it is asserted while
///   both isolates have a verb in flight and four workload threads are hammering them;
/// - device **WRITE** ops complete in the same window, which proves the parked verbs
///   hold no lock at all;
/// - and the per-`(Proc, GpuId)` accounts still separate after tens of thousands of
///   interleaved verbs.
///
/// Progress is required as **termination** — no sleeps, no clock, no wall-time budget.
#[test]
fn n3_c_the_mean_two_gpu_straddler_progresses_under_pending_work_on_both_isolates() {
    let _wd = watchdog("n3_mean_straddler", Duration::from_secs(180));
    for mode in [LockMode::Sharded, LockMode::Degenerate] {
        /// Ops per workload thread — sized to well under a second per mode.
        const N3_OPS: u64 = 200;

        let (world, s, by0, by1, rec) = n3_world(mode);
        let device: Arc<SharedDevice> = Arc::clone(&world);

        // ---- Arm ONE latch on each of the straddler's two isolates, same worker slot.
        //      `WorkerId(0)` of its GPU0 isolate and `WorkerId(0)` of its GPU1 isolate are
        //      unrelated identities — which only became expressible when `IsolateId`
        //      started naming the target.
        let mut latches = Latches::new();
        latches.arm(&rec, s, GPU0, 0, VerbKind::AllocSysmem);
        latches.arm(&rec, s, GPU1, 0, VerbKind::AllocSysmem);
        // The mock's isolate lane for the straddler (its handles read back as ProcId+1).
        let lane = s.0 + 1;

        let parked: Mutex<Vec<(GpuId, Published)>> = Mutex::new(Vec::new());
        thread::scope(|scope| {
            // ★ The latches are MOVED into the scope, so an assert that fires anywhere in
            // here unwinds through `Latches::drop` and releases them BEFORE
            // `thread::scope` joins — otherwise a real failure presents as a wedge
            // (this file's own harness lesson, learned again here).
            let latches = latches;
            // ---- The two threads that PARK, one per isolate.
            for gpu in [GPU0, GPU1] {
                let held_dev = Arc::clone(&device);
                let parked = &parked;
                scope.spawn(move || {
                    let p = held_dev
                        .publish_backing(gpu, N3_PDB, GpuVa(VA_N3_G0), 0x1000)
                        .expect("the parked publication completes once released");
                    parked.lock().expect("parked").push((gpu, p));
                });
            }
            latches.wait_all_pending();

            // ---- Four workload threads on the STRADDLER (two per target) …
            let mut workers = Vec::new();
            for gpu in [GPU0, GPU1] {
                let pub_dev = Arc::clone(&device);
                workers.push(scope.spawn(move || {
                    let mut n = 0u64;
                    for k in 0..N3_OPS {
                        let va = GpuVa(VA_N3_G1 + k * 0x1000);
                        let p = pub_dev
                            .publish_backing(gpu, N3_PDB, va, 0x1000)
                            .expect("a sibling thread publishes while BOTH isolates park");
                        // Read back exactly what THIS commit landed, on THIS target.
                        assert_eq!(
                            pub_dev
                                .resolve(gpu, N3_PDB, GpuVa(va.0 + 0x40))
                                .map(|(b, off)| (b.phys, b.host_va(), off)),
                            Ok((p.gpa, Some(p.host_va), 0x40)),
                            "another target's commit landed in this one's binding"
                        );
                        assert_eq!(
                            host_va_lane(p.host_va),
                            (lane, gpu.0),
                            "a host VA was minted in the wrong (proc, GPU) isolate"
                        );
                        n += 1;
                        assert_eq!(lock::held_depth(), 0, "the op leaked a guard");
                    }
                    n
                }));
                let ring_dev = Arc::clone(&device);
                workers.push(scope.spawn(move || {
                    let mut n = 0u64;
                    for k in 0..N3_OPS {
                        let out = ring_dev
                            .doorbell(gpu, MockArch::token_for(N3_GR), &[])
                            .expect("the straddler rings while BOTH isolates park");
                        assert_eq!(
                            token_lane(out.host_token),
                            (lane, gpu.0),
                            "the rung host token came from the OTHER target's isolate"
                        );
                        if k.is_multiple_of(8) {
                            ring_dev
                                .publish_backing(
                                    gpu,
                                    N3_PDB,
                                    GpuVa(VA_N3_RING + k * 0x1000),
                                    0x1000,
                                )
                                .expect("the ring thread publishes its own working set");
                        }
                        n += 1;
                        assert_eq!(lock::held_depth(), 0, "the op leaked a guard");
                    }
                    n
                }));
            }
            // … and one on each BYSTANDER proc, so the run is multi-process too.
            for (gpu, pid) in [(GPU0, by0), (GPU1, by1)] {
                let by_dev = Arc::clone(&device);
                workers.push(scope.spawn(move || {
                    let mut n = 0u64;
                    for k in 0..N3_OPS {
                        by_dev
                            .publish_backing(gpu, N3_BY_PDB, GpuVa(VA_N3_BY + k * 0x1000), 0x1000)
                            .expect("a bystander proc publishes while the straddler parks");
                        let out = by_dev
                            .doorbell(gpu, MockArch::token_for(N3_BY_GR), &[])
                            .expect("a bystander proc rings while the straddler parks");
                        assert_eq!(out.proc, pid, "the bystander's doorbell mis-demuxed");
                        n += 1;
                    }
                    n
                }));
            }

            // ---- ★★ THE ASSERTIONS INSIDE THE WINDOW. Both isolates still parked.
            let on0 = device
                .forward_engine_object(GPU0, N3_GR, kayfabe_tests::COMPUTE_CLASS, &[])
                .expect("the straddler forwards on GPU0 with a verb parked on it")
                .host_object;
            let on1 = device
                .forward_engine_object(GPU1, N3_GR, kayfabe_tests::COMPUTE_CLASS, &[])
                .expect("the straddler forwards on GPU1 with a verb parked on it")
                .host_object;
            let mut payload = [0u8; 4];
            assert_eq!(
                device.route_control(GPU1, s, on0, mock_ctrl::FORWARDABLE, &mut payload),
                Err(FwdFault::Rm(RmError::ForeignHandle {
                    handle: on0,
                    worker_isolate: IsolateId::new(s.0, GPU1),
                })),
                "★★ ({mode:?}) the cross-GPU gate must hold under contention too"
            );
            assert_eq!(
                device.route_control(GPU0, s, on1, mock_ctrl::FORWARDABLE, &mut payload),
                Err(FwdFault::Rm(RmError::ForeignHandle {
                    handle: on1,
                    worker_isolate: IsolateId::new(s.0, GPU0),
                })),
                "★★ ({mode:?}) …in both directions"
            );
            assert_eq!(
                device.route_control(GPU0, s, on0, mock_ctrl::FORWARDABLE, &mut payload),
                Ok(ControlRoute::Forwarded),
                "({mode:?}) the legal direction still forwards with a verb parked"
            );

            // A device **WRITE** in the same window — the sharpest probe that the parked
            // verbs hold no lock at all.
            assert_eq!(
                device.apply(RmEvent::Alloc {
                    client: N3_BY0_CLIENT,
                    parent: HObject(0x5c00_0001),
                    handle: HObject(0x7000_0001),
                    class: mock_classes::EVENT,
                    facts: AllocFacts::default(),
                }),
                Ok(()),
                "({mode:?}) a device WRITE completed while two verbs were parked"
            );
            assert!(
                latches.all_pending(),
                "★ ({mode:?}) the window is a lie if a latch was released early"
            );

            // ---- Termination: every workload thread finished END TO END while the two
            //      parked verbs were still parked. THEN release.
            let done: u64 = workers
                .into_iter()
                .map(|w| w.join().expect("workload thread"))
                .sum();
            assert_eq!(
                done,
                N3_OPS * 6,
                "({mode:?}) all six workload threads ran to completion under pending work"
            );
            assert!(
                latches.all_pending(),
                "★★ ({mode:?}) PROGRESS-UNDER-PENDING: the latches must still be held at \
                 the moment the workloads finished, or the test proved nothing"
            );
            latches.release_all();
        });

        // ---- The two parked publications completed, one per target, each in its OWN
        //      arena and its OWN host VAS — at the SAME guest VA on both GPUs.
        let mut parked = parked.into_inner().expect("parked");
        parked.sort_by_key(|(g, _)| g.0);
        assert_eq!(
            parked.len(),
            2,
            "({mode:?}) both parked publications returned"
        );
        assert_ne!(
            parked[0].1.gpa, parked[1].1.gpa,
            "★ ({mode:?}) the same guest VA on the proc's two targets must land in two \
             DISJOINT GPA ranges — #14 lifted onto the GPU axis"
        );
        for (i, (gpu, p)) in parked.iter().enumerate() {
            assert_eq!(gpu.0, i as u32);
            assert_eq!(
                host_va_lane(p.host_va),
                (s.0 + 1, gpu.0),
                "({mode:?}) a parked publication's host VA came from the other isolate"
            );
        }

        // ---- …and the accounts are still separate after all of that.
        let ledger = rec.lock().expect("recorder").ledger();
        let g0 = ledger.leaked_on(IsolateId::new(s.0, GPU0));
        let g1 = ledger.leaked_on(IsolateId::new(s.0, GPU1));
        assert!(
            g0.len() > 10 && g1.len() > 10,
            "({mode:?}) non-vacuity: both isolates carry a real account (g0={}, g1={})",
            g0.len(),
            g1.len()
        );
        assert_eq!(
            g0.intersection(&g1).count(),
            0,
            "★ ({mode:?}) the straddler's two isolate accounts must stay disjoint"
        );
        for (gpu, set) in [(GPU0, &g0), (GPU1, &g1)] {
            for h in set {
                assert_eq!(
                    (h.isolate(), handle_lane(h.raw())),
                    (IsolateId::new(s.0, gpu), (s.0 + 1, gpu.0)),
                    "({mode:?}) {gpu:?}'s account holds a handle from the other target"
                );
            }
        }
    }
}

/// ★ The conservation **census** must answer per `(Proc, GpuId)` too.
///
/// [`census`] compares, per isolate, what the ledger says is outstanding against what
/// core state can still name. For a proc that spans two GPUs those are two different
/// questions with two different answers, and asking one proc-wide question of both
/// isolates is not a weaker check — it is a **wrong** one: each isolate's outstanding
/// set gets compared against the union, so every object of the OTHER target reads as
/// `DANGLING` (or would be silently excused, depending on which way the imbalance
/// fell). The straddler is the world that makes that visible.
#[test]
fn n3_d_the_conservation_census_separates_a_procs_two_isolates() {
    let _wd = watchdog("n3_census_separation", Duration::from_secs(60));
    let (mut gpu, s, _by0, _by1, rec) = n3_gpu();

    // Real host state on BOTH of the straddler's targets, at the SAME guest VAs.
    for target in [GPU0, GPU1] {
        for k in 0..3u64 {
            core_publish(&mut gpu, target, N3_PDB, VA_N3_G0 + k * 0x1000)
                .expect("the straddler publishes on both targets");
        }
    }
    // …and on a bystander, so the census is multi-process as well as multi-GPU.
    core_publish(&mut gpu, GPU0, N3_BY_PDB, VA_N3_BY).expect("bystander 0 publishes");
    core_publish(&mut gpu, GPU1, N3_BY_PDB, VA_N3_BY).expect("bystander 1 publishes");

    let c = census(&gpu, &rec);
    // Non-vacuity FIRST: both of the straddler's isolates must actually be in the
    // ledger with a real account, or every emptiness assert below is free.
    let ledger = rec.lock().expect("recorder").ledger();
    for target in [GPU0, GPU1] {
        assert!(
            ledger.leaked_on(IsolateId::new(s.0, target)).len() >= 4,
            "non-vacuity: the straddler's {target:?} isolate holds a host VAS + 3 backings"
        );
    }
    assert_eq!(
        c.dangling_objects,
        BTreeMap::new(),
        "★★ core state names host objects the ledger has no live record of, per isolate — \
         the shape a proc-wide census reports for EVERY object of a straddler's other GPU"
    );
    assert_eq!(
        c.dangling_maps,
        BTreeMap::new(),
        "★★ …and the mapping half of the same imbalance"
    );
    assert_eq!(
        c.leaked_objects,
        BTreeMap::new(),
        "★ nothing outstanding on a live proc is unreachable from its own core state"
    );
    assert_eq!(c.leaked_maps, BTreeMap::new(), "★ …and no mapping either");
}

// ─────────────────────────────────────────────────────────────────────────────────────
// ★★ **B5 — the GSP → core BRIDGE's mean test** (`gsp_core_bridge.md` §5.3).
//
// `rmrpc_bridge.rs` drives the bridge in isolation, one property per test. §5.3 asks for
// the other thing, and asks for it here: ONE composed run, mean rather than happy-path,
// in this file, because *"isolated cases test what you thought of, composed runs test
// what you didn't"*.
//
// What is composed, and why each piece is in the list:
//
//  * **two guest processes' RPC streams, interleaved element-by-element in ONE command
//    queue**, carrying IDENTICAL `hObject` values (the #14 shape) — so the graph's
//    `(client, handle)` keying and the bridge's namespace attribution are under load at
//    the same time, from bytes, through a real msgq ring;
//  * **malformed messages between the valid ones**, including a serialized-params alloc
//    and an unknown function, each counted by variant — a census, never a total;
//  * **a handle recycled mid-stream**: the guest kernel's UVM session dups a process's
//    VASpace, the process exits, and a later process is handed the same `hClient`. That is
//    the §12.41/§12.42 regression — *"host memory RM says is live was taken away from its
//    owner"* — driven from **wire bytes** for the first time;
//  * **real host work between the RPC phases**, under **both lock modes**, so "two
//    distinct `Proc`s" is proved by two host VASes minted in two isolate namespaces
//    rather than by two entries in a map;
//  * **both element layouts**, because the transport under the bridge is version-split;
//  * and all of it **while the six-proc two-GPU device is under concurrent control-plane
//    load**, which is what this file is for.
// ─────────────────────────────────────────────────────────────────────────────────────

/// The bridge world's handles. ★ Every **object** handle is shared by both guest
/// processes — that is the #14 shape, and it is the reason a bridge that keyed anything
/// on a handle value would mis-attribute here rather than in a unit test.
mod rb {
    /// Guest process 1's client.
    pub const C1: u32 = 0xc1d0_0069;
    /// Its pid.
    pub const PID1: u32 = 0x0000_dd13;
    /// Guest process 2's client — adjacent, so a confusion still looks plausible.
    pub const C2: u32 = 0xc1d0_006a;
    /// Its pid.
    pub const PID2: u32 = 0x0000_dd14;
    /// The pid of the process that is later handed process 1's recycled `hClient`.
    pub const PID3: u32 = 0x0000_dd15;
    /// The guest kernel's UVM session client.
    pub const K: u32 = 0xdead_c0de;

    /// ★ The Device handle — the SAME value in both namespaces.
    pub const DEV: u32 = 0x5c00_0001;
    /// ★ The VASpace handle — the same value in both namespaces.
    pub const VAS: u32 = 0x5c00_0010;
    /// ★ The TSG handle — likewise.
    pub const TSG: u32 = 0x5c00_0012;
    /// ★ The GR channel handle — likewise.
    pub const GR: u32 = 0x5c00_0019;
    /// ★ The CE channel handle — likewise.
    pub const CE: u32 = 0x5c00_001a;
    /// ★ The compute engine object — likewise.
    pub const GR_OBJ: u32 = 0x5c00_0020;
    /// ★ The copy engine object — likewise.
    pub const CE_OBJ: u32 = 0x5c00_0021;
    /// The UVM session's alias of a process VASpace.
    pub const KALIAS: u32 = 0x5d00_0031;

    /// Process 1's page directory.
    pub const PDB1: u64 = 0x0000_0000_0034_1000;
    /// Process 2's page directory.
    pub const PDB2: u64 = 0x0000_0000_0035_1000;
    /// The recycled declaration's page directory.
    pub const PDB3: u64 = 0x0000_0000_0036_1000;
}

/// Process 1's channels.
const RB_GR1: VChid = VChid(0x31);
/// Process 1's copy channel.
const RB_CE1: VChid = VChid(0x32);
/// Process 2's channels — different vChids at the SAME channel handles, which is exactly
/// what makes the identical-handle world routable at all.
const RB_GR2: VChid = VChid(0x41);
/// Process 2's copy channel.
const RB_CE2: VChid = VChid(0x42);

/// A CUDA-process-shaped RPC stream, as **bytes**. One per guest process, and the two
/// differ only in `hClient`, `processID`, the page directory and the two USERD words.
fn rb_process_stream(client: u32, pid: u32, pdb: u64, gr: VChid, ce: VChid) -> RpcScript {
    let mut s = RpcScript::new();
    s.client_root(w::NV01_ROOT, client, pid)
        .device(client, client, rb::DEV, GPU0.0)
        .vaspace(client, rb::DEV, rb::VAS)
        .set_page_dir(client, rb::DEV, rb::VAS, pdb, w::PDB_FLAGS_ALL_CHANNELS)
        .tsg(client, rb::DEV, rb::TSG, rb::VAS)
        .channel(
            client,
            rb::TSG,
            rb::GR,
            MockArch::userd_flags_for(gr),
            0,
            rb::VAS,
        )
        .channel(
            client,
            rb::TSG,
            rb::CE,
            MockArch::userd_flags_for(ce),
            0,
            rb::VAS,
        )
        .engine_object(client, rb::GR, rb::GR_OBJ, w::AMPERE_COMPUTE_B)
        .engine_object(client, rb::CE, rb::CE_OBJ, w::AMPERE_DMA_COPY_B);
    s
}

/// The hostile traffic interleaved into the two streams, each with the **exact** tag it
/// must be counted under. Every one of them names `rb::C1`, which the very first message
/// of the interleaved run declares, so none of them short-circuits on a namespace that
/// simply has not arrived yet — a refusal for an accidental reason would count under the
/// right tag for the wrong reason.
fn rb_hostile() -> Vec<(w::Step, FaultTag)> {
    vec![
        (
            w::Step {
                function: 999,
                body: vec![0u8; 8],
            },
            FaultTag("BridgeRefusal::UnknownFunction"),
        ),
        (
            // A serialized-params alloc: otherwise perfect, refused on a DECLARED bit.
            w::Step {
                function: fn_id::GSP_RM_ALLOC,
                body: w::alloc_body(
                    rb::C1,
                    0,
                    0,
                    w::NV01_ROOT,
                    8,
                    w::RMAPI_RPC_FLAGS_SERIALIZED,
                    &w::client_root_params(rb::C1, rb::PID1),
                ),
            },
            FaultTag("BridgeRefusal::SerializedParams"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_ALLOC,
                body: w::alloc_body(
                    rb::C1,
                    rb::DEV,
                    0x5c00_0099,
                    w::NV01_MEMORY_SYSTEM,
                    8,
                    0,
                    &[0u8; 8],
                ),
            },
            FaultTag("BridgeRefusal::UnmappedAllocClass"),
        ),
        (
            w::Step {
                function: fn_id::FREE,
                body: vec![0u8; 3],
            },
            FaultTag("BridgeRefusal::Abi"),
        ),
        (
            w::Step {
                function: fn_id::GSP_RM_CONTROL,
                body: w::control_body(
                    rb::C1,
                    rb::DEV,
                    w::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES,
                    0,
                    w::RMAPI_RPC_FLAGS_NONE,
                    &[],
                ),
            },
            FaultTag("BridgeRefusal::PageDirControlNotModelled"),
        ),
        (
            w::Step {
                function: fn_id::GSP_INIT_DONE,
                body: vec![0u8; 8],
            },
            FaultTag("BridgeRefusal::EventFromGuest"),
        ),
        (
            // ★ B5's own: a dup planted in a namespace nobody has declared. The bridge
            // translates it — it resolves nothing — and the GRAPH refuses it, which is
            // the anti-squat rule reached from the wire.
            w::Step {
                function: fn_id::DUP_OBJECT,
                body: w::dup_body(0xfeed_0001, 0, rb::KALIAS, rb::C1, rb::VAS, 0),
            },
            FaultTag("RmGraphError::UndeclaredClient"),
        ),
        (
            // The envelope's own client is zero: no namespace to attribute the message
            // to, refused by the bridge without consulting anything.
            w::Step {
                function: fn_id::DUP_OBJECT,
                body: w::dup_body(0, 0, rb::KALIAS, rb::C1, rb::VAS, 0),
            },
            FaultTag("BridgeRefusal::ReservedClient"),
        ),
    ]
}

/// A device for the bridge to declare into: real NVIDIA class ids (`WireClassArch`), one
/// GPU, mock isolates.
fn rb_gpu() -> (Guarded<Gpu>, SharedRecorder) {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x1000_0000_0000, 0x1_0000_0000);
    let gpu = Gpu::new(
        Box::new(kayfabe_mocks::WireClassArch::new()),
        Box::new(factory),
        gpa,
    )
    .expect("the bridge device realizes");
    (Guarded::new("l1_mean::rb_gpu", gpu, rec.clone()), rec)
}

/// Post `steps` through the world's real command ring, service them with a
/// [`GraphPolicy`] over `gpu`, and let the guest drain. Returns
/// `(replies' rpc_results, census, applied, inert)`.
fn rb_post(
    world: &mut GspWorld,
    gpu: &mut Gpu,
    profile: kayfabe_tests::gspworld::Profile,
    steps: &[w::Step],
    seq_base: u32,
) -> (Vec<u32>, BTreeMap<FaultTag, usize>, u64, u64) {
    let mut policy = GraphPolicy::new(profile.table(), gpu);
    for (i, s) in steps.iter().enumerate() {
        world
            .guest
            .send(&mut world.ram, s.function, seq_base + i as u32, &s.body)
            .expect("the ring has room");
    }
    world
        .doorbell_with(&mut policy)
        .expect("the doorbell services the ring");
    let replies = world
        .guest
        .recv(&mut world.ram)
        .expect("a clean status stream")
        .iter()
        .map(|m| m.rpc_result)
        .collect();
    (
        replies,
        policy.census().tags().collect(),
        policy.applied(),
        policy.inert(),
    )
}

/// ★★ **THE BRIDGE'S MEAN RUN.** See the block comment above for what is composed and
/// why. Runs under both lock modes and both element layouts, with the six-proc two-GPU
/// world under concurrent control-plane load throughout.
#[test]
fn the_rpc_bridge_survives_two_interleaved_guest_streams_under_mean_device_load() {
    let _wd = watchdog("rmrpc_bridge_mean", Duration::from_secs(180));

    for mode in [LockMode::Sharded, LockMode::Degenerate] {
        for (profile, model) in [(P580, MODEL_A), (P610, MODEL_B)] {
            let (world, pids, _rec) = mean_world(mode);
            let device = Arc::clone(&world);
            // The rest of this file's device, under load for the whole run — bounded
            // (one pass per proc), for the starvation reason `the_gsp_boot_cycle…` test
            // records.
            let load: Vec<_> = (0..2)
                .map(|i| {
                    let device = Arc::clone(&device);
                    let pids = pids.clone();
                    thread::spawn(move || ctl_workload(&device, &pids, i))
                })
                .collect();

            let (mut bridge, rec) = rb_gpu();
            let mut gsp = GspWorld::new_sized(profile, model, REAL_QUEUE_SIZE);
            gsp.boot_with(&mut kayfabe_gsp::EchoOk);
            assert_eq!(
                gsp.link_and_drain().len(),
                1,
                "{mode:?}/{profile:?}: the bind posts exactly GSP_INIT_DONE",
            );

            // ---- Phase A: two streams, interleaved element by element, hostile traffic
            //      between them.
            let s1 = rb_process_stream(rb::C1, rb::PID1, rb::PDB1, RB_GR1, RB_CE1);
            let s2 = rb_process_stream(rb::C2, rb::PID2, rb::PDB2, RB_GR2, RB_CE2);
            let hostile = rb_hostile();
            let mut interleaved: Vec<w::Step> = Vec::new();
            // ★ Positional, not a count: which SLOT of the answered stream carries a
            // failure. A count is satisfied by any eight failures, including eight in
            // the wrong places with eight valid messages silently refused beside them.
            let mut want_failed: Vec<bool> = Vec::new();
            let mut want_tags: BTreeMap<FaultTag, usize> = BTreeMap::new();
            let mut h = hostile.iter();
            for (a, b) in s1.steps().iter().zip(s2.steps()) {
                interleaved.push(a.clone());
                interleaved.push(b.clone());
                want_failed.extend([false, false]);
                if let Some((step, tag)) = h.next() {
                    interleaved.push(step.clone());
                    want_failed.push(true);
                    *want_tags.entry(*tag).or_default() += 1;
                }
            }
            for (step, tag) in h {
                interleaved.push(step.clone());
                want_failed.push(true);
                *want_tags.entry(*tag).or_default() += 1;
            }
            assert_eq!(
                want_tags.values().sum::<usize>(),
                hostile.len(),
                "every hostile message was posted",
            );

            let (replies, census, applied, inert) =
                rb_post(&mut gsp, &mut bridge, profile, &interleaved, 0x1000);
            assert_eq!(
                replies.len(),
                interleaved.len(),
                "{mode:?}/{profile:?}: every command answered on (function, sequence)",
            );
            assert_eq!(
                replies.iter().map(|r| *r != 0).collect::<Vec<_>>(),
                want_failed,
                "…and exactly the hostile ones, IN THEIR POSITIONS, carry a failure",
            );
            assert_eq!(
                census, want_tags,
                "{mode:?}/{profile:?}: ★ the refusal census, BY VARIANT",
            );
            assert_eq!(
                (applied, inert),
                ((s1.steps().len() + s2.steps().len()) as u64, 0),
                "and every valid message declared a fact",
            );

            // ---- The two processes are two `Proc`s, despite sharing every object handle.
            let b = project(&bridge.spine.rmgraph, bridge.spine.arch(), &NO_CONDEMNED)
                .expect("projects");
            assert_eq!(
                b.procs
                    .iter()
                    .map(|p| p.client_values())
                    .collect::<Vec<_>>(),
                vec![
                    [HClient(rb::C1)].into_iter().collect(),
                    [HClient(rb::C2)].into_iter().collect(),
                ],
                "{mode:?}/{profile:?}: ★★ identical handles, TWO blast radii",
            );
            assert_eq!(
                b.by_pdb.keys().copied().collect::<Vec<_>>(),
                vec![(GPU0, Pdb(rb::PDB1)), (GPU0, Pdb(rb::PDB2))],
            );
            assert_eq!(
                b.by_vchid.keys().copied().collect::<Vec<_>>(),
                vec![
                    (GPU0, RB_GR1),
                    (GPU0, RB_CE1),
                    (GPU0, RB_GR2),
                    (GPU0, RB_CE2),
                ],
                "★ four channels at TWO handle values — the vChid came off the wire",
            );
            let p1 = bridge.spine.by_pdb[&(GPU0, Pdb(rb::PDB1))];
            let p2 = bridge.spine.by_pdb[&(GPU0, Pdb(rb::PDB2))];
            assert_ne!(p1, p2, "two procs");
            let (a1, a2) = (
                bridge.procs[&p1].arenas[&GPU0].range.clone(),
                bridge.procs[&p2].arenas[&GPU0].range.clone(),
            );
            assert!(
                a1.end <= a2.start || a2.end <= a1.start,
                "{mode:?}/{profile:?}: ★ two DISJOINT GPA arenas: {a1:?} vs {a2:?}",
            );

            // ---- Phase B: real host work, under `mode`. Two host VASes, in two isolate
            //      namespaces — which is what "two distinct procs" actually buys.
            let shell = bridge.map(|g| Arc::new(SharedDevice::new(g, mode)));
            let pub1 = shell
                .publish_backing(GPU0, Pdb(rb::PDB1), GpuVa(0x50_0000_0000), 0x1000)
                .expect("process 1 publishes");
            let pub2 = shell
                .publish_backing(GPU0, Pdb(rb::PDB2), GpuVa(0x50_0000_0000), 0x1000)
                .expect("process 2 publishes at the SAME guest VA");
            assert_ne!(
                host_va_lane(pub1.host_va),
                host_va_lane(pub2.host_va),
                "{mode:?}/{profile:?}: ★★ two host VASes, in two isolate namespaces — the \
                 same guest VA in two procs must never share host state",
            );
            assert_ne!(pub1.gpa, pub2.gpa, "and two disjoint arenas backed them");
            let mut bridge = shell.map(|a| Arc::into_inner(a).expect("sole owner").into_gpu());

            // ---- Phase C: ★ the recycle, from wire bytes. The UVM session dups process
            //      1's VASpace, process 1 exits, and a LATER process is handed the same
            //      `hClient` value.
            let named1 = reachable_of(&bridge, p1);
            assert!(
                named1.len() >= 2,
                "{mode:?}/{profile:?}: the baseline is real host state: {named1:?}",
            );
            let mut recycle = RpcScript::new();
            recycle
                .client_root(w::NV01_ROOT, rb::K, w::KERNEL_PID)
                .dup(rb::K, rb::K, rb::KALIAS, rb::C1, rb::VAS)
                .free(rb::C1, rb::C1)
                .client_root(w::NV01_ROOT, rb::C1, rb::PID3)
                .device(rb::C1, rb::C1, rb::DEV, GPU0.0)
                .vaspace(rb::C1, rb::DEV, rb::VAS)
                .set_page_dir(
                    rb::C1,
                    rb::DEV,
                    rb::VAS,
                    rb::PDB3,
                    w::PDB_FLAGS_ALL_CHANNELS,
                );
            let (replies, census, applied, _) =
                rb_post(&mut gsp, &mut bridge, profile, recycle.steps(), 0x2000);
            assert!(
                census.is_empty(),
                "{mode:?}/{profile:?}: ★ a recycle is LEGAL traffic — refusing it hangs a \
                 conforming guest: {census:?}",
            );
            assert_eq!(replies, vec![0; recycle.steps().len()]);
            assert_eq!(applied, recycle.steps().len() as u64);

            // §12.33: the kernel alias keeps process 1's `Proc` — and its host memory —
            // alive after its owner's root is gone.
            assert_eq!(
                kayfabe_fwd::route_pdb(&bridge.spine, GPU0, Pdb(rb::PDB1)),
                Ok(p1),
                "{mode:?}/{profile:?}: ★ a live kernel reference keeps its owner's proc",
            );
            // ★★★ §12.41/§12.42: the successor is a DIFFERENT proc on a DIFFERENT
            // declaration, and it took nothing away from its predecessor.
            let p3 = bridge.spine.by_pdb[&(GPU0, Pdb(rb::PDB3))];
            assert!(
                p3 != p1 && p3 != p2,
                "{mode:?}/{profile:?}: the recycled hClient is a NEW proc",
            );
            assert_eq!(
                reachable_of(&bridge, p1),
                named1,
                "{mode:?}/{profile:?}: ★★ host memory RM says is live must not be taken \
                 away from its owner by a successor holding the same hClient VALUE",
            );
            let keys1 = bridge.procs[&p1].clients.clone();
            let keys3 = bridge.procs[&p3].clients.clone();
            assert_eq!(
                (
                    keys1.iter().map(|k| k.client).collect::<BTreeSet<_>>(),
                    keys3.iter().map(|k| k.client).collect::<BTreeSet<_>>(),
                ),
                (
                    [HClient(rb::C1)].into_iter().collect(),
                    [HClient(rb::C1)].into_iter().collect(),
                ),
                "one hClient VALUE …",
            );
            assert!(
                keys1.is_disjoint(&keys3),
                "{mode:?}/{profile:?}: ★★ … and TWO ClientKey incarnations of it",
            );
            assert!(
                bridge.system.client_values().contains(&HClient(rb::K)),
                "the UVM session is the SYSTEM component's, and merged nobody",
            );
            let _ = bridge.reap_retired();
            assert_eq!(
                reachable_of(&bridge, p1),
                named1,
                "{mode:?}/{profile:?}: …and the reclamation the core SCHEDULED frees none \
                 of it either",
            );
            assert_no_corruption(&rec, "after the bridge's recycle");

            // ---- Phase D: the successor is usable end to end, in its OWN isolate.
            let shell = bridge.map(|g| Arc::new(SharedDevice::new(g, mode)));
            let pub3 = shell
                .publish_backing(GPU0, Pdb(rb::PDB3), GpuVa(0x50_0000_0000), 0x1000)
                .expect("the recycled declaration publishes in its own isolate");
            assert!(
                host_va_lane(pub3.host_va) != host_va_lane(pub1.host_va)
                    && host_va_lane(pub3.host_va) != host_va_lane(pub2.host_va),
                "{mode:?}/{profile:?}: ★ a re-declared hClient never inherits the previous \
                 tenant's isolate",
            );
            drop(shell);

            let published: usize = load.into_iter().map(|h| h.join().expect("worker")).sum();
            assert!(
                published > 0,
                "non-vacuity: the device planes really were under load throughout",
            );
        }
    }
}
