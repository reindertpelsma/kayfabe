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
//! ## ★★ WHAT THIS TEST FOUND — decision needed (`l1_concurrency.md` §12.13)
//!
//! **An out-of-band retire is undone by the very next `Gpu::apply`.** §7.3 requires a
//! worker HUP to "retire the proc loudly … no resurrect", and
//! `SignalOutcome::WorkerDied` promises "**never a respawn**". But `Spine::retire_proc`
//! only removes the proc from the live set; the guest's client root is still in the
//! graph, so the next `refresh` finds a boundary with no matching live proc, mints a
//! **fresh `ProcId`**, spawns a **brand-new isolate** (a respawned sandboxed worker,
//! new handle namespace) and a fresh GPA arena, rebuilds routing onto it, and the guest
//! resumes full service. See [`out_of_band_retire_must_not_resurrect_the_isolate`],
//! which asserts the design's invariant and is `#[ignore]`d because the fix is a design
//! change (a condemned-component state in the projection→proc derivation, plus the
//! fault surface an op against one should get), not a local edit. The sweep below pins
//! **today's** behavior loudly so that fixing it trips this test too.
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
use kayfabe_core::gpu::Gpu;
use kayfabe_core::reactor::SourceKind;
use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
use kayfabe_core::{ChanId, ProcId};
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
    (Arc::new(SharedDevice::new(gpu, mode)), pids, recorder)
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
            (binding.phys, binding.host_va, off),
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
            (binding.phys, binding.host_va, off) == (p.gpa, Some(p.host_va), 0x40);
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

    // ---- Phase 9: ONE deterministic refresh, so the §12.13 finding the sweep pins is a
    // fact of every run rather than a race with the workload threads' own churn.
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

/// The facts that must hold globally, no matter how the threads interleaved. Each is a
/// property the design *claims*; asserting them here is what makes the claim falsifiable
/// by a harsh run instead of by inspection.
fn sweep_conservation(gpu: &mut Gpu, pids: &[ProcId], mode: LockMode) {
    // ---- (1) ★ Per-`(GpuId, ·)` routing NEVER collapses. Each lane's PDB and CE vChid
    // exist IDENTICALLY on both GPUs and must resolve to two DIFFERENT procs.
    for (l, lane) in LANES.iter().enumerate() {
        let (a, b) = (
            gpu.spine.by_pdb[&(GPU0, lane.pdb)],
            gpu.spine.by_pdb[&(GPU1, lane.pdb)],
        );
        assert_ne!(
            a, b,
            "({mode:?}) identical PDBs on two GPUs collapsed onto one proc"
        );
        let (ca, _) = gpu.spine.by_vchid[&(GPU0, lane.ce)];
        let (cb, _) = gpu.spine.by_vchid[&(GPU1, lane.ce)];
        assert_ne!(
            ca, cb,
            "({mode:?}) identical vChids on two GPUs collapsed onto one proc"
        );
        assert_eq!((ca, cb), (a, b), "({mode:?}) by_pdb and by_vchid disagree");
        // Lane A (the two multi-threaded procs) is never retired, so its routing must
        // still name the ORIGINAL procs — the resurrection below must not have touched
        // procs that were never retired.
        if l == 0 {
            assert_eq!((a, b), (pids[P_WITNESS], pids[P_PEER]));
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

    // ---- (3) ★★ DEFECT PINNED (`l1_concurrency.md` §12.13, and the `#[ignore]`d
    // `out_of_band_retire_must_not_resurrect_the_isolate` below).
    //
    // §7.3 says an out-of-band retire has "no resurrect" and `WorkerDied` says "never a
    // respawn". What actually happens: the retired proc leaves the live set, but its
    // client root is still in the guest's graph, so the NEXT `refresh` finds a boundary
    // with no matching live proc and mints a brand-new `ProcId` with a brand-new isolate
    // (a respawned sandboxed worker) and a fresh arena. The two procs this run killed
    // out of band are therefore BACK, under new identities, before the sweep runs.
    //
    // This assertion pins TODAY'S behavior deliberately, so that fixing the defect trips
    // this test and forces the doc, the ignored test and this sweep to move together.
    // It is NOT an endorsement.
    for &i in &[P_TEARDOWN, P_HUP] {
        assert!(
            !gpu.procs.contains_key(&pids[i]),
            "({mode:?}) the retired proc's ORIGINAL identity is gone (it was reaped)"
        );
        let now = gpu.spine.by_pdb[&(gpu_of(i), lane_of(i).pdb)];
        assert_ne!(
            now, pids[i],
            "({mode:?}) §12.13: routing was rebuilt onto a fresh proc identity"
        );
        assert!(
            gpu.procs.contains_key(&now),
            "★★ ({mode:?}) §12.13 DEFECT: an out-of-band-retired proc was RESURRECTED as \
             {now:?} with a fresh isolate — §7.3 promises 'no resurrect'. If this assert \
             now FAILS, the defect was fixed: update §12.13, un-ignore \
             `out_of_band_retire_must_not_resurrect_the_isolate`, and delete this block."
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
                let Some(hv) = b.host_va else { continue };
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

/// ★★ **DEFECT — `l1_concurrency.md` §12.13. IGNORED BECAUSE THE FIX IS A DESIGN
/// CHANGE, NOT BECAUSE THE ASSERT IS WRONG.**
///
/// §7.3: *"Worker death out-of-band (crash) is a reactor source firing HUP → dispatch →
/// retire the proc loudly … either way MISS=FAULT posture, **no resurrect**."*
/// [`SignalOutcome::WorkerDied`]: *"the slot is permanently dead (**never a respawn** —
/// a worker that died mid-verb may have left host state the core cannot reason
/// about)"*.
///
/// What actually happens, proven below: `Spine::retire_proc` removes the proc from the
/// live set, but the guest's client root is untouched in the RM graph, so the **next
/// `Gpu::apply`** re-derives that boundary, finds no matching live proc, mints a fresh
/// `ProcId`, spawns a **brand-new isolate** (new sandbox, new handle namespace — the
/// respawn §7.3 forbids), carves a fresh GPA arena, and rebuilds `by_pdb`/`by_vchid`
/// onto it. The guest then publishes and rings again with no refusal whatsoever. Only
/// the dead *worker slot* stayed dead; the isolate came back around it.
///
/// Why this is not a local fix: making it stick requires the derivation to carry a
/// **condemned-component** state (an out-of-band-retired boundary must stay dead until
/// the guest itself frees its client root) *and* a decision about what an op against a
/// condemned component returns — there is no `Proc` left to name in a `RetiredProc`
/// fault. §12.11 already flagged the neighbouring half of this ("unifying the two
/// teardown routes means deciding whether a host-side failure should retroactively edit
/// the guest's routing truth, which is a design question, not a fix"). This is that
/// question, with teeth.
///
/// Un-ignore this test when §12.13 is decided; [`sweep_conservation`] step (3) pins the
/// current behavior and will fail at the same moment, by design.
#[test]
#[ignore = "★★ KNOWN DEFECT l1_concurrency.md §12.13 — an out-of-band retire is undone \
            by the next refresh (the isolate is respawned). The fix is a design change; \
            this assert is the design's own invariant and must NOT be weakened."]
fn out_of_band_retire_must_not_resurrect_the_isolate() {
    let _wd = watchdog(
        "out_of_band_retire_must_not_resurrect",
        Duration::from_secs(60),
    );
    let (device, pids, _rec) = mean_world(LockMode::Sharded);
    let victim = pids[P_HUP];

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
        Err(FwdFault::RetiredProc(victim)),
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

    // THE INVARIANT (§7.3): the condemned component must stay dead. It does not.
    assert_eq!(
        device.publish_backing(GPU1, lane_of(P_HUP).pdb, GpuVa(VA_HELD), 0x1000),
        Err(FwdFault::RetiredProc(victim)),
        "★★ §12.13: a refresh RESURRECTED an out-of-band-retired proc on a fresh isolate \
         — §7.3 promises no resurrect and WorkerDied promises never a respawn"
    );
    assert_eq!(
        device.doorbell(GPU1, MockArch::token_for(lane_of(P_HUP).gr), &[]),
        Err(FwdFault::RetiredProc(victim)),
        "★★ §12.13: and its channels serve the guest again"
    );
}
