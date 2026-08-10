//! ★ [`SharedDevice`] — the lock-swap of the core's sharding shape
//! (`l1_concurrency.md` §2/§3.4, decisions #34/#35/#37).
//!
//! The core was already reshaped for this (decision #35, landed): `Gpu` is the L0
//! bundle of a device-global [`Spine`], the system [`Proc`], and the user procs —
//! separately borrowable, with spine ops taking `&mut Spine + &mut Proc(system) +
//! &mut impl ProcSet`. This module is therefore a **lock placement, not a
//! redesign**:
//!
//! ```text
//!   RankedRwLock<DeviceState>                       // rank 0 — the device lock
//!     DeviceState {
//!         spine:  Spine,                            // device-global, guarded by rank 0
//!         system: RankedMutex<Proc>,                // rank 1
//!         procs:  BTreeMap<ProcId, RankedMutex<Proc>>, // rank 1, one per proc
//!     }
//! ```
//!
//! ## ★ The key mechanic: spine ops acquire ZERO proc locks
//!
//! A spine op needs `&mut impl ProcSet` over *every* proc — naively N proc locks at
//! once, an R3 violation ("at most one lock per rank"). It doesn't have to be:
//! under the device **write** guard the op holds `&mut DeviceState`, and
//! [`RankedMutex::get_mut`] yields `&mut Proc` **without acquiring the lock at
//! all** — sound precisely because `&mut` already proves exclusivity. The
//! [`ExclusiveProcs`] adapter implements the core's [`ProcSet`] over the cell map
//! this way: zero proc-lock acquisitions, zero rank interactions (provable via
//! [`crate::lock::acquisitions`]; pinned by the `rt_shell` integration test).
//!
//! ## The two op shapes (R2/R4)
//!
//! - **spine op** (apply / pump / poll / drained / reap / source registration) —
//!   device *write* guard, procs via [`ExclusiveProcs`]. One lock, rank 0.
//! - **per-proc op** (doorbell, publish, resolve, gate, source dispatch) — device
//!   *read* guard (rank 0) → look up that proc's `RankedMutex` → `lock()` it
//!   (rank 1) → the act phase. Two locks, in rank order, one per rank — the
//!   route/act split (R4) the core's `kayfabe-fwd` signatures already factor.
//!
//! ## Both lock configurations, from day one (§8.2, review item P5)
//!
//! [`LockMode`] is chosen at construction and **never leaks into call sites** —
//! same public API, same results:
//!
//! - [`LockMode::Sharded`] — the shape above;
//! - [`LockMode::Degenerate`] — every op takes the device **write** lock and
//!   reaches procs via `get_mut` (bit-for-bit the stress-proven single-lock shape
//!   L1-M1 ships with; §3.4's staging recommendation).
//!
//! The `rt_shell` lock-mode differential test runs one scripted sequence under
//! both and asserts identical end state — the test that makes the late granularity
//! flip (the #14 gate) safe instead of the untested mode.
//!
//! ## R1 status in stage 2 (honest)
//!
//! The act phases below still run the mock `RmBackend` verbs inline under the proc
//! lock — exactly as the core currently shapes `exec_doorbell`/`publish_backing`.
//! That is stage-2-correct because the mock verbs are pure µs bookkeeping; the
//! REAL blocking verb path is stage 3's plan/execute/commit
//! ([`crate::lock::BlockingSection`] is its already-armed precondition), which
//! splits the verb out from under these locks per R1.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};

use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuId, GpuVa, Pdb, VChid};
use kayfabe_completion::{CompletionError, OsEventRef, PostBatch};
use kayfabe_core::gpu::{Gpu, GpuError, PendingSpawn, PendingSpawns, Proc, ProcSet, Spine};
use kayfabe_core::reactor::{CompletionSource, Dispatch, SourceFault, SourceKind};
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_core::{ChanId, ProcId};
use kayfabe_fwd::{
    ControlRoute, DoorbellOutcome, EngineObjectForwarded, FwdFault, Orphans, Planned, Published,
    Refusal,
};
use kayfabe_isolate::{
    HostHandle, IsolateBox, IsolateFactory, IsolateId, RmError, VerbPlan, VerbReply, Worker,
    WorkerId,
};
use kayfabe_mmu::Binding;
use kayfabe_util::Instant;

use crate::lock::{BlockingSection, LockRank, RankedMutex, RankedRwLock};

/// Which lock configuration a [`SharedDevice`] runs (§8.2 / P5: BOTH are tested
/// from day one; the mode must never be observable through the API).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    /// Every op takes the device **write** lock; procs are reached via `get_mut`.
    /// Bit-for-bit the stress-proven single-lock shape — L1-M1's shipping
    /// configuration (§3.4 staging).
    Degenerate,
    /// Spine ops write-lock the device; per-proc ops take device-read (rank 0)
    /// then that proc's mutex (rank 1). The #14-gate configuration.
    Sharded,
}

/// The rank-0-guarded state: the spine plus the per-proc lock cells.
struct DeviceState {
    /// Device-global spine (graph, routing maps, targets, delivery, sources).
    spine: Spine,
    /// The system proc's rank-1 cell (kernel RM / scrubber / CeUtils traffic).
    system: RankedMutex<Proc>,
    /// One rank-1 cell per derived user proc.
    procs: BTreeMap<ProcId, RankedMutex<Proc>>,
}

impl DeviceState {
    /// The rank-1 cell of `pid` — the system proc lives in its own field, not the
    /// map (mirroring `Gpu`'s shape), so routing must check it explicitly rather
    /// than "miss then guess".
    fn proc_cell(&self, pid: ProcId) -> Option<&RankedMutex<Proc>> {
        if pid == Gpu::SYSTEM_PROC {
            Some(&self.system)
        } else {
            self.procs.get(&pid)
        }
    }

    /// Lock-free (`get_mut`) access to `pid`'s proc — the degenerate-mode /
    /// write-guard path.
    fn proc_mut(&mut self, pid: ProcId) -> Option<&mut Proc> {
        if pid == Gpu::SYSTEM_PROC {
            Some(self.system.get_mut())
        } else {
            self.procs.get_mut(&pid).map(RankedMutex::get_mut)
        }
    }
}

/// ★ The [`ProcSet`] adapter over the lock-cell map: `&mut` access to every proc
/// with **zero lock acquisitions** (module docs, "the key mechanic"). Exists only
/// inside a device write guard, which is what makes the `get_mut`s sound.
struct ExclusiveProcs<'a>(&'a mut BTreeMap<ProcId, RankedMutex<Proc>>);

impl ProcSet for ExclusiveProcs<'_> {
    fn get_mut(&mut self, id: ProcId) -> Option<&mut Proc> {
        self.0.get_mut(&id).map(RankedMutex::get_mut)
    }
    fn insert(&mut self, id: ProcId, proc: Proc) {
        // A freshly-derived proc is born already inside its rank-1 cell.
        self.0.insert(id, RankedMutex::new(LockRank::Proc, proc));
    }
    fn remove(&mut self, id: ProcId) -> Option<Proc> {
        // `into_inner` — no acquisition; `Spine::completion_poll` does exactly
        // this remove/put-back split-borrow, so remove MUST hand back the naked
        // `Proc` (it re-enters through `insert` above).
        self.0.remove(&id).map(RankedMutex::into_inner)
    }
    fn iter_mut(&mut self) -> impl Iterator<Item = (ProcId, &mut Proc)> {
        // BTreeMap iteration is ascending by ProcId — the core's determinism
        // contract (ProcSet doc: derived state must never depend on iteration
        // order, decision #27) rides on this.
        self.0.iter_mut().map(|(&id, m)| (id, m.get_mut()))
    }
}

/// What one completion-source signal did — the typed result of
/// [`SharedDevice::signal_source`], mirroring the core's [`Dispatch`] with its
/// effect applied (or its refusal surfaced). MISS = FAULT: a stale/unknown source
/// is [`SignalOutcome::Fault`], loud and mutation-free — never a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalOutcome {
    /// The os-event landed in the **owning** proc's own `CompletionQueue`
    /// (observation only — see [`SharedDevice::signal_source`] on why the pump
    /// edge is not run here in stage 2).
    Observed {
        /// The owning proc.
        proc: ProcId,
        /// The GPU target the event was armed on.
        gpu: GpuId,
        /// The observed event.
        ev: OsEventRef,
    },
    /// ★ A worker of `proc`'s `gpu` isolate died, and the §7.3 consequence has been
    /// applied: the slot is permanently dead (**never a respawn**) and the owning proc
    /// has been **retired loudly**, deregistering every completion source it owned. Its
    /// completions die with it; nothing is resurrected, and its component is
    /// **condemned** (`l1_concurrency.md` §12.13).
    ///
    /// **Why never a respawn.** Not because the dead worker left host state the core
    /// cannot reason about — the isolate is a *process*, so the host kernel reclaims
    /// what it held (fds close, RM tears down its client objects, its mmaps go). It is
    /// because **the guest's data died with it**: a published backing is host memory
    /// (`RmBackend::alloc_sysmem`) owned by that isolate's RM client, so re-materialising
    /// would hand the guest a fresh, **zeroed** backing for a VA it believes still holds
    /// its data — silent corruption, strictly worse than the resurrect it would be
    /// fixing. The refusal instead gives the guest the semantic real hardware already
    /// has: **sticky-fatal**, exactly like an Xid, recoverable by re-initialising
    /// (a fresh RM client is a different component and is not condemned) or by dying
    /// (the guest kernel frees the clients and the entry clears).
    WorkerDied {
        /// The owning proc.
        proc: ProcId,
        /// The isolate's GPU target.
        gpu: GpuId,
        /// The dead worker slot.
        worker: WorkerId,
    },
    /// ★ **A worker of the SYSTEM isolate died — this is DEVICE-FATAL, not a
    /// condemnation** (`l1_concurrency.md` §12.26).
    ///
    /// [`SignalOutcome::WorkerDied`]'s consequence is "retire the proc, condemn its
    /// component, recover by re-initialising with a fresh RM client". None of that is
    /// available here: the system component's clients are the **guest kernel's**, held
    /// for the lifetime of the loaded module, so condemning it would kill the guest's
    /// driver permanently — the recovery path requires the guest kernel to mint fresh
    /// clients, and the guest kernel is what was condemned.
    ///
    /// Before this variant existed the outcome was reported as `WorkerDied` and the
    /// consequence **silently did nothing**: `Spine::retire_proc` reached the system proc
    /// through a `ProcSet` that does not contain it, missed, and answered `false` that
    /// nobody read. The device carried on with a permanently dead system worker slot and
    /// no fault anywhere.
    ///
    /// The slot is still killed (never a respawn, §7.3) — what does **not** happen is the
    /// retire and the condemnation. RM's own answer to an unrecoverable kernel-side
    /// failure has the same shape and the same level: `gpuMarkDeviceForReset` +
    /// `NV2080_NOTIFIERS_GPU_UNAVAILABLE`
    /// (★ a **version seam**, and the shared part is the part this rests on:
    /// `ogkm-610: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:2771-2792` reaches it from
    /// `_kgspHandleFatalTimeout`, notifying at `:2789` with the classified `errorNum`, and
    /// only when TDR is unsupported, with `gpuMarkDeviceForReset` two lines above it at
    /// `:2779`. At `ogkm-580:` the pair is **split across two functions**: the notify is
    /// unconditional inside `_kgspLogXid119` (`:2130-2205`, notifying at `:2169` with
    /// `GSP_RPC_TIMEOUT`), and `gpuMarkDeviceForReset` is the caller's own three-back-to-back
    /// timeout branch (`:2459`). Different trigger, different payload, different nesting —
    /// but `gpuNotifySubDeviceEvent` at **both**, so the *scope* claim is tag-independent) — **device** scope,
    /// never client scope. Escalating this to a guest-visible device-unavailable
    /// notification is L1-M2's (T4/T7); the core's obligation is to make it
    /// distinguishable and loud, which is what this variant is.
    DeviceFatal {
        /// The system isolate's GPU target.
        gpu: GpuId,
        /// The dead worker slot.
        worker: WorkerId,
    },
    /// The cross-process reflection seam fired (future source class; surfaced,
    /// unwired).
    CrossSignal {
        /// Originating proc.
        from: ProcId,
        /// Receiving proc.
        to: ProcId,
    },
    /// The notifiable source fired: re-read the source set. Touches no proc.
    Wake,
    /// The source resolved to nothing — never registered, deregistered, or its
    /// proc retired (indistinguishable by design; handles are never reused).
    /// Nothing was mutated.
    Fault(SourceFault),
    /// The owning proc's queue refused the observation (bounded queue, hostile
    /// guest posture) — loud, queue unchanged.
    ObserveRefused {
        /// The owning proc.
        proc: ProcId,
        /// The refused event.
        ev: OsEventRef,
        /// Why the queue refused.
        err: CompletionError,
    },
}

/// ★ The L1 shared device: the core behind the two ranked locks, in either
/// [`LockMode`]. All entry points take `&self` — the locks inside provide the
/// exclusivity the core's `&mut` signatures demand. Each method documents which
/// phase holds which lock.
pub struct SharedDevice {
    mode: LockMode,
    state: RankedRwLock<DeviceState>,
    pool: PoolGate,
    /// ★★★ **The isolate factory, reachable with NO lock held** (R1, `l1_concurrency.md`
    /// §3.3) — a clone of the `Arc` the wrapped `Gpu` was realized with, so there is one
    /// factory and this is a second handle on it, never a second factory.
    ///
    /// It is a field rather than a read through [`SharedDevice::state`] for a reason the
    /// alternative makes obvious: the spawn must run with every guard dropped, so reaching
    /// the factory *through* the device lock would mean acquiring rank 0 purely to learn
    /// how to do something that must not be done under rank 0 — one extra acquisition on
    /// a path `rt_shell::spine_ops_acquire_no_proc_lock_via_get_mut` counts, in service of
    /// nothing.
    spawner: Arc<dyn IsolateFactory>,
}

/// ★ **Pool-saturation counters for ONE GPU target** (`l1_concurrency.md` §7.2, §12.29).
///
/// Saturation is *correct* behaviour — bounded pool, well-behaved backpressure — and
/// from outside the process it is **indistinguishable from a hang**: guest threads stop
/// making progress and nothing anywhere says why. That is a diagnostic gap, not an
/// architectural one (the bound is right: RM serialises every ioctl-reachable path on the
/// per-client write lock, so the pool buys liveness isolation, not throughput). These
/// counters close it: a stalled device with `parked > 0` and `waiting > 0` is congested;
/// one with `waiting == 0` is wedged somewhere else, and the two need different answers.
///
/// **Counted, never timed.** There is no clock in this crate (§8.3), and a duration would
/// be the wrong measurement anyway: "how long did a wait take" mixes the queue with the
/// verb latency behind it. Events and depths are exact, comparable across runs, and
/// assertable — which is also the testable form of the restated F1 rule (§4.2: *every
/// poll must be provably bounded* — **assert a bound, not an absence**).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PoolWaits {
    /// Checkouts that found every worker of the target's isolate in flight and entered
    /// the gate. **The saturation event itself** — it counts even when the wait turns out
    /// to be free, because the pool WAS full at the moment the plan phase asked.
    pub saturated: u64,
    /// Of those, the ones that actually blocked: a worker returned in the sampling gap
    /// leaves the predicate already false, and that thread is served without parking.
    /// `saturated - parked` is therefore the near-miss count.
    pub parked: u64,
    /// ★ The largest number of threads parked here at one instant — **how long the queue
    /// got**. The number that says whether the pool is merely touched or genuinely the
    /// constraint.
    pub peak_waiters: u32,
    /// How many are parked right now. A live-state read, exact at the moment the snapshot
    /// takes the gate's mutex.
    pub waiting: u32,
}

/// The gate's own mutable state: the wakeup generation plus the per-target counters.
///
/// Keyed by [`GpuId`] and **deliberately not by `(ProcId, GpuId)`**: a `ProcId` is
/// monotonic and never reused, so a per-proc map would grow without bound under a guest
/// that churns processes — a hostile guest could inflate it for free (boundary-1). The
/// target set is bounded by the device's entitled GPUs, so this map is bounded by
/// construction. Per-proc attribution, if it is ever wanted, belongs in a trace event
/// (which is discarded) and not in retained state.
#[derive(Debug, Default)]
struct GateState {
    generation: u64,
    waits: BTreeMap<GpuId, PoolWaits>,
}

/// ★ The pool-full waiting point (`l1_concurrency.md` §7.2/§7.3): "pool exhaustion is
/// well-behaved backpressure — the guest thread waits (lock-free, R1) for a worker".
///
/// A generation counter plus its condvar, and the [`PoolWaits`] counters that make the
/// wait **observable** rather than merely correct. A thread that finds every worker in
/// flight releases **all** ranked locks, waits here, and re-enters the op **from the top**
/// with full R5 re-validation — the proc may have retired while it waited.
///
/// Its mutex is deliberately NOT a [`RankedMutex`]: it is the condvar's own mutex,
/// which R1 explicitly exempts ("a condvar wait that atomically releases its own
/// mutex is lock-free with respect to THAT mutex, but the waiter must hold no *other*
/// lock"). Keeping it out of the rank system is what lets the R1 assert stay true
/// while the waiter is parked; the [`BlockingSection`] the waiter opens first is what
/// proves the "no *other* lock" half.
///
/// Lost-wakeup freedom: the waiter samples the generation **before** it takes any
/// lock to attempt the checkout, so a return that lands in the gap bumps the
/// generation and the wait predicate is already false.
#[derive(Debug, Default)]
struct PoolGate {
    state: Mutex<GateState>,
    returned: Condvar,
}

impl PoolGate {
    /// Sample the generation before an attempt (no ranked lock may be held yet).
    fn sample(&self) -> u64 {
        self.state.lock().expect("pool gate").generation
    }

    /// A worker came back: wake every waiter (they re-enter from the top and
    /// re-race for it — bounded, and correct under any number of waiters).
    fn signal_return(&self) {
        self.state.lock().expect("pool gate").generation += 1;
        self.returned.notify_all();
    }

    /// A consistent snapshot of every target's counters.
    fn snapshot(&self) -> BTreeMap<GpuId, PoolWaits> {
        self.state.lock().expect("pool gate").waits.clone()
    }

    /// Wait until the generation moves past `seen`, recording the saturation against
    /// `gpu`. **Panics (R1) unless the caller holds zero ranked locks** — the whole point
    /// of the exercise.
    fn wait_for_return(&self, gpu: GpuId, seen: u64) {
        let mut section = BlockingSection::enter();
        section.run(|| {
            let mut g = self.state.lock().expect("pool gate");
            // The saturation event is recorded whether or not this thread ends up
            // parking: the pool WAS full when the plan phase asked for a worker.
            let w = g.waits.entry(gpu).or_default();
            w.saturated = w.saturated.saturating_add(1);
            if g.generation != seen {
                return; // a worker returned in the sampling gap — no park at all
            }
            let w = g.waits.entry(gpu).or_default();
            w.parked = w.parked.saturating_add(1);
            w.waiting = w.waiting.saturating_add(1);
            w.peak_waiters = w.peak_waiters.max(w.waiting);
            while g.generation == seen {
                g = self.returned.wait(g).expect("pool gate");
            }
            let w = g.waits.entry(gpu).or_default();
            w.waiting = w.waiting.saturating_sub(1);
        });
    }
}

/// How many times a converging-staleness commit re-plans before surfacing its fault
/// (see [`SharedDevice::verb_op`]). Each retry observes a strictly more materialized
/// world, so one pass is the expected worst case; the bound exists so a bug cannot
/// turn a race into a spin.
///
/// ★ **Public because the retry LEDGER is tested against it** (`l1_concurrency.md`
/// §12.28). `retry_ledger.rs` drives a scripted re-stale round per attempt until the
/// bound is exhausted, and it must drive exactly as many rounds as the bound allows —
/// one too few and the bound is never hit, one too many and the harness waits on an
/// attempt that will never be made. Reading the constant instead of copying its value
/// is what keeps that test honest when the bound moves; it is **not** a tuning knob and
/// nothing outside the crate should branch on it.
pub const MAX_COMMIT_RETRIES: u32 = 8;

/// What one locked plan phase staged for the lock-free execute phase: the plan's
/// ID-shaped hints, the verb chain, and the checked-out worker that will run it.
///
/// `worker: None` **with** `verbs: Some` is the pool-full signal — backpressure, not
/// failure (§7.2). `verbs: None` means the site needed no host work at all.
struct Staged<P> {
    proc: ProcId,
    gpu: GpuId,
    plan: P,
    verbs: Option<VerbPlan>,
    worker: Option<Worker>,
    /// ★ T0 (`l1_os_shell.md` §7.6 T0, gap G2): what a previous `refresh` queued for
    /// release on this `(proc, gpu)` isolate, picked up because a worker is checked out
    /// anyway. Empty unless the guest freed a subset of its objects since the last op.
    release: Orphans,
}

impl<P> Staged<P> {
    /// Finish a plan phase by checking a worker out of `proc`'s `gpu` isolate — the
    /// last thing that happens under the lock, per §7.3.
    ///
    /// ★ **And T0's opportunistic drain point** (§7.6 T0, "when to drain"): a checked-out
    /// worker is exactly the opportunity, so the pending-release queue rides out of the
    /// locked phase with it at near-zero marginal cost — via
    /// [`kayfabe_fwd::checkout_and_drain`], which also carries the idle precondition that
    /// keeps the drain from racing an in-flight verb. The pool-full path re-enters from
    /// the top and must not have swallowed the queue on its way past, and the no-host-work
    /// path has no worker to run the release on; both are handled there.
    fn check_out(proc: &mut Proc, gpu: GpuId, planned: Planned<P>) -> Result<Self, FwdFault> {
        let (worker, release) = match planned.verbs {
            Some(_) => kayfabe_fwd::checkout_and_drain(proc, gpu)?,
            None => (None, Orphans::default()),
        };
        Ok(Staged {
            proc: proc.id,
            gpu,
            plan: planned.plan,
            verbs: planned.verbs,
            worker,
            release,
        })
    }
}

impl SharedDevice {
    /// Wrap a realized [`Gpu`] in the chosen lock configuration. The decomposition
    /// is exactly `Gpu`'s #35 ownership split — no state is reshaped, only wrapped.
    #[must_use]
    pub fn new(gpu: Gpu, mode: LockMode) -> Self {
        let Gpu {
            spine,
            system,
            procs,
        } = gpu;
        // ★ Taken BEFORE the spine goes behind the lock — afterwards, reaching it would
        // cost a rank-0 acquisition (see [`SharedDevice::spawner`]).
        let spawner = spine.isolate_factory();
        SharedDevice {
            mode,
            pool: PoolGate::default(),
            spawner,
            state: RankedRwLock::new(
                LockRank::Device,
                DeviceState {
                    spine,
                    system: RankedMutex::new(LockRank::Proc, system),
                    procs: procs
                        .into_iter()
                        .map(|(id, p)| (id, RankedMutex::new(LockRank::Proc, p)))
                        .collect(),
                },
            ),
        }
    }

    /// The configured lock mode (diagnostics; behavior never depends on it).
    #[must_use]
    pub fn mode(&self) -> LockMode {
        self.mode
    }

    /// ★ **Pool saturation, per GPU target — the answer to "is it congested or is it
    /// wedged?"** (`l1_concurrency.md` §7.2, §12.29; see [`PoolWaits`]).
    ///
    /// Takes only the gate's own mutex, so it is safe to call from a diagnostic thread
    /// while every guest thread is blocked — which is exactly the situation it exists
    /// for, and would be useless if it needed the device lock the stalled ops hold.
    ///
    /// Targets with no saturation ever are simply absent from the map: an empty result
    /// means the pool has never been the constraint.
    #[must_use]
    pub fn pool_waits(&self) -> BTreeMap<GpuId, PoolWaits> {
        self.pool.snapshot()
    }

    /// Consume the device back into a plain [`Gpu`] (tests: the lock-mode
    /// differential snapshots the reassembled core). Ownership-based — no
    /// acquisition.
    #[must_use]
    pub fn into_gpu(self) -> Gpu {
        let SharedDevice { state, .. } = self;
        let DeviceState {
            spine,
            system,
            procs,
        } = state.into_inner();
        Gpu {
            spine,
            system: system.into_inner(),
            procs: procs
                .into_iter()
                .map(|(id, m)| (id, m.into_inner()))
                .collect(),
        }
    }

    // ---- Spine ops: device WRITE guard, procs via ExclusiveProcs (no rank 1) ----

    /// Apply one RM protocol event (`Spine::apply`). **Spine op**: device write
    /// guard for the whole call; every proc reached lock-free through
    /// [`ExclusiveProcs`]. Identical in both modes (a spine op's shape does not
    /// degenerate further).
    /// ★★ **And it discharges cancels** (`l1_os_shell.md` §7.6 T2). A guest process
    /// exiting — normally, killed, or killed *while a verb is pending* — arrives here as
    /// an `RmEvent::Free` of its client root, so `Spine::refresh` vacates its proc and
    /// latches a break signal for every verb it still has in flight. The latches are
    /// fired **after the guard drops**: firing one is a syscall, and R1 admits no
    /// exception for it.
    /// ★★★ **And it MATERIALIZES the isolates the event decided on** (R1's spawn
    /// deferral, `l1_concurrency.md` §3.3; `kayfabe_core::gpu::Spine::defer_isolate`).
    ///
    /// Exactly the same two-step, for exactly the same reason, as the cancels above:
    /// deciding a `(Proc, GpuId)` needs an isolate is pure bookkeeping and belongs under
    /// the write lock; *spawning* one is `clone` into six namespaces + `execveat` +
    /// a blocking handshake (+ real host RM ioctls under `KAYFABE_ISOLATES=real`), and R1
    /// admits no exception for it. Doing it in place aborted QEMU on the guest's first
    /// register write that reached a `GSP_RM_ALLOC` —
    /// `docs/reference/bench_evidence/f0b7efa_run_basereal_qemu.log`.
    pub fn apply(&self, ev: RmEvent) -> Result<(), GpuError> {
        // ★ The latches are TAKEN inside the guard this op already holds, never in a
        // second critical section. Taking a fresh write lock to ask "is anything
        // latched?" would add a rank-0 acquisition to the hottest spine path — caught by
        // `rt_shell::spine_ops_acquire_no_proc_lock_via_get_mut`, which counts them.
        let (out, cancels, spawns) = {
            let mut g = self.state.write();
            let st = &mut *g;
            let out = st
                .spine
                .apply(st.system.get_mut(), &mut ExclusiveProcs(&mut st.procs), ev);
            (
                out,
                st.spine.take_pending_cancels(),
                st.spine.take_pending_spawns(),
            )
        };
        // Guards dropped (R1): firing one is a syscall.
        cancels.discharge_all();
        // Guards dropped (R1): spawning a sandbox is many syscalls.
        self.materialize(spawns);
        out
    }

    /// ★★★ **The DRAIN half of R1's spawn deferral: spawn lock-free, then re-acquire and
    /// RE-VALIDATE (R5).**
    ///
    /// Idempotent and safe to run from any thread — which it must be, because it is not
    /// only the accepting thread that runs it. A second vCPU thread can route to a proc in
    /// the window between the write guard dropping here and the install landing; it gets
    /// [`FwdFault::IsolatePending`] and materializes the same pair itself
    /// ([`SharedDevice::verb_op`]). Both spawn; exactly one installs; the loser's sandbox
    /// is surplus and is dropped with no lock held. That is §12.9's compare-and-swap
    /// verbatim, and it is deliberately preferred to gating the second thread: a gate
    /// would be a hand-rolled mutex over a blocking call (§4.2 forbids both halves), and
    /// §12.9's own conclusion is that the loser releases its duplicate rather than being
    /// refused.
    ///
    /// ⊘ The cost of losing is one extra sandbox spawned and immediately reaped. It is
    /// paid at most once per `(Proc, GpuId)` per race, and it is a **named** cost, not a
    /// leak: the surplus is an `IsolateBox`, so if it were ever dropped under a lock the
    /// assert would say so.
    fn materialize(&self, spawns: PendingSpawns) {
        for PendingSpawn { proc, gpu } in spawns {
            self.materialize_one(proc, gpu);
        }
    }

    /// One `(proc, gpu)`: spawn with zero locks held, then install under the write lock
    /// with full R5 re-validation. See [`SharedDevice::materialize`].
    fn materialize_one(&self, pid: ProcId, gpu: GpuId) {
        // ---- SPAWN: no lock held. `IsolateBox::new` asserts exactly that. ----
        let iso = IsolateBox::new(self.spawner.spawn(IsolateId::new(pid.0, gpu)));
        // ---- INSTALL: rank 0, and the core decides whether it may (R5). ----
        //
        // A spine op, so both lock modes take the write guard and reach the proc through
        // `get_mut` — installing touches the spine's own census, so there is no sharded
        // shape here to degenerate from.
        let surplus = {
            let mut g = self.state.write();
            let DeviceState {
                spine,
                system,
                procs,
            } = &mut *g;
            let p = if pid == Gpu::SYSTEM_PROC {
                Some(system.get_mut())
            } else {
                procs.get_mut(&pid).map(RankedMutex::get_mut)
            };
            match p {
                // Divergent staleness: the proc left the live set in the gap. Nothing to
                // install it into, and nothing to retry — MISS = FAULT.
                None => Some(iso),
                Some(p) => spine.install_isolate(p, gpu, iso).err(),
            }
        };
        // Guard dropped (R1): an isolate's `Drop` is `waitpid` + namespace teardown.
        drop(surplus);
    }

    /// Compose+post one completion batch for `gpu` if its drain gate is open
    /// (`Spine::pump_completions`). **Spine op** (write guard): it composes across
    /// procs' queues and consults the per-target gate. The caller owns the §5.2
    /// edge semantics (encode + `raise_irq`) — the `Vmm` seam is the shell's,
    /// stage 3.
    pub fn pump_completions(&self, gpu: GpuId) -> Option<PostBatch> {
        let mut g = self.state.write();
        let st = &mut *g;
        st.spine
            .pump_completions(st.system.get_mut(), &mut ExclusiveProcs(&mut st.procs), gpu)
    }

    /// Proc `pid`'s own completion-poll RPC on target `gpu` (the starvation fix's
    /// entry — re-posts off the OWNER's poll). **Spine op** (write guard): the
    /// core's poll does the remove/put-back split-borrow through the [`ProcSet`].
    /// `now` is caller-supplied — no clock exists in this crate (§8.3).
    pub fn completion_poll(&self, gpu: GpuId, pid: ProcId, now: Instant) -> Option<PostBatch> {
        let mut g = self.state.write();
        let st = &mut *g;
        st.spine.completion_poll(
            st.system.get_mut(),
            &mut ExclusiveProcs(&mut st.procs),
            gpu,
            pid,
            now,
        )
    }

    /// The guest drained target `gpu`'s outstanding batch (IRQSCLR). **Spine op**
    /// (write guard).
    pub fn completions_drained(&self, gpu: GpuId) {
        let mut g = self.state.write();
        let st = &mut *g;
        st.spine
            .completions_drained(st.system.get_mut(), &mut ExclusiveProcs(&mut st.procs), gpu);
    }

    /// Reap retired procs at the adapter-declared quiesce point (inherited law 8,
    /// L10). **Spine op** (write guard). Returns the number reaped.
    ///
    /// ★ **G3b (§12.16): the drop happens OUTSIDE the guard, and that is the point of
    /// the signature change.** `Spine::reap_retired` used to drop each retired `Proc`
    /// in place — including its [`kayfabe_isolate::IsolateBox`]es, whose real `Drop`
    /// is `waitpid` + namespace teardown. Under this write guard that is a blocking
    /// syscall under a rank-0 lock: a live R1 violation that no assert covered,
    /// because `Worker::execute`'s `assert_lock_free` guards *verbs*, not drops. The
    /// reap now hands the corpses back, the guard is released, and only then does the
    /// value fall. `IsolateBox`'s own `Drop` asserts lock-freedom, so re-introducing
    /// the old shape panics naming R1 instead of blocking silently.
    pub fn reap_retired(&self) -> usize {
        let reclaimed = {
            let mut g = self.state.write();
            g.spine.reap_retired()
            // ↑ the write guard dies at this brace, BEFORE the drop below.
        };
        let n = reclaimed.len();
        drop(reclaimed); // ← the blocking teardown, with zero ranked locks held.
        n
    }

    /// Read one live proc's state under whichever guards the configured mode requires —
    /// a **diagnostic window**, so a caller can ask a question about a proc without
    /// reaching into the lock shell or consuming the device. `None` if `pid` is not live.
    ///
    /// Deliberately `&Proc` and deliberately a closure: the borrow cannot escape the
    /// guard, so this cannot become a back door for holding proc state across a lock
    /// boundary (which is how a "just for a moment" accessor turns into an R5 violation).
    pub fn with_proc<R>(&self, pid: ProcId, f: impl FnOnce(&Proc) -> R) -> Option<R> {
        let mut out = None;
        let _ = self.route_act(
            |_| Ok((pid, ())),
            |_, p, ()| {
                out = Some(f(p));
            },
        );
        out
    }

    /// The mutating twin of [`SharedDevice::with_proc`]: run `f` against ONE live proc
    /// under its own rank-1 lock, taking no other proc lock.
    ///
    /// ★ Added for `#102`'s page-table latch, which is the first operation whose *target*
    /// proc is not the proc that produced the work — the guest kernel writes user
    /// processes' page tables. `None` means the proc is gone (retired or never existed),
    /// which every caller treats as "the state this would have touched died with it",
    /// never as an error.
    pub fn with_proc_mut<R>(&self, pid: ProcId, f: impl FnOnce(&mut Proc) -> R) -> Option<R> {
        let mut out = None;
        let _ = self.route_act(
            |_| Ok((pid, ())),
            |_, p, ()| {
                out = Some(f(p));
            },
        );
        out
    }

    /// Every proc the device currently holds live, the system proc first. **Spine op**
    /// (read guard); pairs with [`SharedDevice::with_proc`] so a caller can walk the
    /// whole live set one rank-1 lock at a time — never two at once, which R3 would
    /// refuse.
    #[must_use]
    pub fn live_pids(&self) -> Vec<ProcId> {
        let st = self.state.read();
        core::iter::once(Gpu::SYSTEM_PROC)
            .chain(st.procs.keys().copied())
            .collect()
    }

    /// ★★★★ **§16.25 — every live channel's VA-space resolution, as one census.**
    ///
    /// # Why a census and not just the refused channel's own row
    ///
    /// `[measured 2026-08-08, boot `s23_10a769c_cup2`]` 24 doorbells arrived, **9 were
    /// served and 15 refused `FwdFault::NoVas`**. A refusal that describes only the refused
    /// channel cannot be read: every field on it is equally consistent with "this is how
    /// all channels look here" and with "this channel is the odd one out". The nine served
    /// channels are the **control**, and a field that reads the same on a served channel and
    /// a refused one is not the field that explains the refusal.
    ///
    /// ⊘ Read-only, resolves nothing, and allocates a `Vec` — a **diagnostic** path, called
    /// when a doorbell has already been refused, never on the serving path.
    ///
    /// ★ Walks the live set one rank-1 lock at a time via [`Self::live_pids`] +
    /// [`Self::with_proc`], never two at once (R3).
    #[must_use]
    pub fn channel_vas_census(&self) -> Vec<ChannelVasRow> {
        let mut out = Vec::new();
        for pid in self.live_pids() {
            self.with_proc(pid, |p| {
                for (cid, ch) in &p.channels {
                    out.push(ChannelVasRow {
                        proc: pid,
                        chan: *cid,
                        vchid: ch.vchid,
                        engine: ch.engine,
                        client: ch.key.origin.client.0,
                        handle: ch.key.origin.handle.0,
                        has_pdb: ch.vas_pdb.is_some(),
                        route: ch.vas_route,
                    });
                }
            });
        }
        out
    }

    /// ★★★★ **§16.27 — every object one client namespace holds, by kind.**
    ///
    /// # The one question §16.25 measured but could not answer
    ///
    /// §16.25 established that the walling channel (`c0xc1e00010/0x2`) declares neither an
    /// `hVASpace` nor an `hCtxShare`, and that its parent is a **Device**. RM's answer for
    /// that exact shape is the **device's default VA space**
    /// (`ogkm-580: kernel_channel.c:350-375` → `kernel_ctxshare.c:127` →
    /// `vaspace.c:178` `vaspaceGetByHandleOrDeviceDefault_IMPL`, which resolves the Device
    /// when `hVASpace == NV01_NULL_OBJECT`).
    ///
    /// Which leaves exactly one fork, and it decides the whole shape of the fourth route:
    /// - the namespace **does** hold a `VaSpace` under that Device ⇒ the route is a
    ///   **lookup** we are simply not performing; or
    /// - it holds **none** ⇒ RM created the default VAS implicitly with the Device, it was
    ///   never an `RM_ALLOC` on the wire, and the route must **mint** one.
    ///
    /// ⊘ §16.25's capture could not tell these apart, and ⊘ *"no VASpace appeared in the
    /// refusal string"* is **not** evidence of the second: the refusal never enumerated the
    /// namespace. An absent capture is unmeasured, not empty
    /// (`c_oracle_empty_rows_are_wrong`). This method is what makes the fork measurable.
    ///
    /// ⊘ Read-only, allocates, and is called only from a refusal path.
    #[must_use]
    pub fn namespace_census(&self, client: u32) -> Vec<NamespaceRow> {
        let st = self.state.read();
        let g = &st.spine.rmgraph;
        let mut out: Vec<NamespaceRow> = g
            .nodes()
            .filter(|n| n.key.client.0 == client)
            .map(|n| NamespaceRow {
                kind: n.kind,
                handle: n.key.handle.0,
                parent: n.parent.0,
                pdb: g.pdb_of_resource(n.id()).map(|p| p.0),
            })
            .collect();
        out.sort_by_key(|r| r.handle);
        out
    }

    /// ★★★ **This device's own pushbuffer codec**, for the duration of `f` — a **spine op**
    /// (rank-0 read guard).
    ///
    /// # Why a borrow through a closure, and why it exists at all
    ///
    /// The E10e shell driver (`crate::ceutils`) decodes a CeUtils submission's method words
    /// outside the core, because that channel's addresses resolve through the register
    /// plane rather than through a `Vas`. It must decode them with **this device's** codec:
    /// a second `Arch` constructed in the adapter would be a second description of the wire
    /// format, and the two would agree until the day they did not.
    ///
    /// ⊘ A closure rather than a returned reference, because the codec lives inside the
    /// spine behind the device lock and a `&dyn PushbufferAbi` handed out here would outlive
    /// its guard — the same constraint `decode_pt_writes`'s `fmt` parameter states one seam
    /// over.
    ///
    /// ⚠ **Lock order.** This takes rank 0. A caller holding the register plane's mutex may
    /// call it (plane→core is the established order, set by the command-policy chain); a
    /// caller holding a rank-1 proc lock may not — that is the rank order, unchanged.
    pub fn with_pushbuffer<R>(&self, f: impl FnOnce(&dyn kayfabe_arch::PushbufferAbi) -> R) -> R {
        let g = self.state.read();
        f(g.spine.arch().pushbuffer())
    }

    /// The device-global page-table ownership index's answer for `phys` — **spine op**
    /// (read guard), a diagnostic window over [`kayfabe_core::gpu::Spine::pt_page_owner`].
    ///
    /// ★ E8: exists so a test (and a boot-time diagnostic) can ask whether the PUBLISH
    /// phase actually reached the index, without the caller holding a guard or reaching
    /// into the lock shell. `None` is the ordinary answer for an ordinary data page.
    #[must_use]
    pub fn pt_page_owner(&self, gpu: GpuId, phys: u64) -> Option<(ProcId, Pdb)> {
        self.state.read().spine.pt_page_owner(gpu, phys)
    }

    /// ★★★ **E1's isolate census, over the sharded shell** — `Gpu::isolate_census`'s twin
    /// for a device whose procs live behind rank-1 locks.
    ///
    /// **Spine op shape, one proc lock at a time.** The spine's own counter and the retired
    /// corpses are read under the rank-0 guard (a vacated proc has left its lock cell and
    /// is a bare value inside the spine — see [`SharedDevice::with_retired`]); each live
    /// proc is then visited on its own, never two at once, which is exactly what R3
    /// requires and what [`SharedDevice::live_pids`] + [`SharedDevice::with_proc`] exist
    /// for.
    ///
    /// ⊘ It says **whether** an isolate was materialized, live or refusing, and it can
    /// never say **why** — the census is written by the code under test, so attribution of
    /// a spawn to the guest belongs to an instrument outside the process
    /// (`scripts/bench/e0_isolate_witness.sh`). That sentence is in `IsolateCensus`'s own
    /// docs and is repeated here because this is the entry point a shell will reach for.
    #[must_use]
    pub fn isolate_census(&self) -> kayfabe_isolate::IsolateCensus {
        // ★ The device-global half first, under rank 0 alone: the materialization counter
        // and every retired proc's isolates.
        let (mut census, pids) = {
            let st = self.state.read();
            let c = st.spine.isolate_census_seed();
            let pids: Vec<ProcId> = core::iter::once(Gpu::SYSTEM_PROC)
                .chain(st.procs.keys().copied())
                .collect();
            (c, pids)
        };
        // ★ Then each live proc, ONE rank-1 lock at a time. A proc that retired in the gap
        // is simply absent — its isolates were counted above if it is still a corpse, and
        // if it was reaped it has none.
        for pid in pids {
            self.with_proc(pid, |p| {
                for iso in p.isolates.values() {
                    census.observe(&**iso);
                }
            });
        }
        census
    }

    /// ★ Read window over the **retired-but-unreaped** procs (`Spine::retired_procs`).
    /// **Spine op** (read guard), and no proc lock: a vacated proc has left the lock
    /// cells and is a bare value inside the spine. Needed by the §12.35 teardown audit,
    /// which cannot state "reachable or queued" without seeing the corpses' queues.
    pub fn with_retired<R>(&self, f: impl FnOnce(&[Proc]) -> R) -> R {
        let st = self.state.read();
        f(st.spine.retired_procs())
    }

    /// ★★ **T0's backstop drain** (`l1_os_shell.md` §7.6 T0, gap G2) — release every
    /// host object a `refresh` queued for a **live** proc, for procs that have gone
    /// quiet. Returns how many objects + mappings it disposed of.
    ///
    /// The opportunistic path ([`Staged::check_out`]) covers a proc that keeps issuing
    /// ops: its next verb-issuing op has a worker checked out anyway, so the queue rides
    /// out with it for free. This is the other half — *"the executor as the backstop for
    /// a proc that goes quiet"*. A guest process that frees its last VASpace and then
    /// blocks on a fence would otherwise hold those host objects for as long as it lives,
    /// which on this path is forever: T0 is the one trigger with no process-boundary
    /// backstop (§7.0).
    ///
    /// **Shape: plan (locked) → execute (no locks) → check in, per `(proc, target)`**,
    /// exactly like every other verb-issuing op — never a second reclamation mechanism.
    /// The queue is taken and the worker checked out inside one locked phase; the
    /// disposal runs with zero ranked locks held (R1, enforced by `Worker::execute`); the
    /// worker goes back through [`SharedDevice::return_worker`], which handles a proc that
    /// retired in the gap (G3).
    ///
    /// **Pool-full — or an isolate with a verb still in flight — is a SKIP, not a wait.**
    /// This is a housekeeping sweep, so parking it behind guest traffic would be
    /// backpressure applied in the wrong direction; and the in-flight case must not be
    /// waited out either, because releasing an object a live verb still names is the
    /// use-after-free [`Proc::checkout_with_pending_release`]'s idle test exists to
    /// prevent. Either way the queue simply stays for the next sweep or the next op,
    /// which is what makes this safe to call from any edge and idempotent at a real
    /// quiesce point.
    pub fn drain_pending_releases(&self) -> usize {
        let pids: Vec<ProcId> = {
            let st = self.state.read();
            core::iter::once(Gpu::SYSTEM_PROC)
                .chain(st.procs.keys().copied())
                .collect()
        };
        let mut disposed = 0usize;
        for pid in pids {
            // Which targets owe anything — a read, so a proc with nothing queued (the
            // overwhelming majority) costs one lock and no checkout.
            let targets = self
                .route_act(|_| Ok((pid, ())), |_, p, ()| p.pending_release_targets())
                .unwrap_or_default();
            for gpu in targets {
                // ---- PLAN: take the queue and a worker of THAT isolate, together.
                let mut taken: Option<(Orphans, Worker)> = None;
                let _ = self.route_act(
                    |_| Ok((pid, ())),
                    |_, p, ()| {
                        if let Ok((Some(w), o)) = kayfabe_fwd::checkout_and_drain(p, gpu) {
                            taken = Some((o, w));
                        }
                    },
                );
                let Some((orphans, mut worker)) = taken else {
                    continue; // pool full, or the proc retired — next sweep.
                };
                let n = orphans.free.len() + orphans.unmap.len();
                // ---- EXECUTE: zero locks held.
                let undisposed = kayfabe_fwd::dispose_on(&mut worker, orphans);
                disposed += n - (undisposed.free.len() + undisposed.unmap.len());
                self.return_worker(pid, gpu, worker);
            }
        }
        disposed
    }

    /// How many retired procs are still awaiting a reap — including any a previous
    /// [`SharedDevice::reap_retired`] **deferred** for not being quiesced (§12.16,
    /// G3). **Spine op** (read guard); diagnostics and test assertions.
    #[must_use]
    pub fn retired_len(&self) -> usize {
        self.state.read().spine.retired_len()
    }

    /// Register a completion source (spine mutation — write guard). The
    /// [`kayfabe_core::reactor::WakeRequest`] is discharged into the registry's
    /// latch; the stage-3 shell drains it via [`SharedDevice::take_pending_wake`]
    /// after any entry that may have changed the set.
    pub fn register_source(&self, kind: SourceKind) -> CompletionSource {
        let mut g = self.state.write();
        let (source, wake) = g.spine.sources.register(kind);
        wake.latched();
        source
    }

    /// Drain the registry's pending-wake latch: `true` if the source set changed
    /// since the last drain (registrations above, or retire-path deregistrations
    /// inside [`SharedDevice::apply`]). The stage-3 reactor loop's re-join edge;
    /// exposed now so the seam is visible and testable.
    pub fn take_pending_wake(&self) -> bool {
        self.state
            .write()
            .spine
            .sources
            .take_pending_wake()
            .is_some()
    }

    // ---- Per-proc ops: route under device READ, act under that proc's lock ------

    // ---- ★ The plan / execute / commit driver (R1 + R5, `l1_concurrency.md` §3.3) --

    /// One locked phase: resolve the target proc from the spine, then act on **that
    /// proc only**. Sharded = device read (rank 0) + that proc's mutex (rank 1);
    /// Degenerate = one device write guard reaching the proc via `get_mut`. The mode
    /// never leaks into the caller.
    fn route_act<T, R>(
        &self,
        route: impl FnOnce(&Spine) -> Result<(ProcId, T), FwdFault>,
        act: impl FnOnce(&Spine, &mut Proc, T) -> R,
    ) -> Result<R, FwdFault> {
        match self.mode {
            LockMode::Sharded => {
                let st = self.state.read();
                let (pid, t) = route(&st.spine)?;
                let cell = st.proc_cell(pid).ok_or(FwdFault::RetiredProc(pid))?;
                let mut p = cell.lock();
                Ok(act(&st.spine, &mut p, t))
            }
            LockMode::Degenerate => {
                let mut g = self.state.write();
                let DeviceState {
                    spine,
                    system,
                    procs,
                } = &mut *g;
                let (pid, t) = route(spine)?;
                let p = if pid == Gpu::SYSTEM_PROC {
                    system.get_mut()
                } else {
                    procs
                        .get_mut(&pid)
                        .map(RankedMutex::get_mut)
                        .ok_or(FwdFault::RetiredProc(pid))?
                };
                Ok(act(spine, p, t))
            }
        }
    }

    /// The COMMIT locked phase: re-acquire, then let the core re-validate (R5) and
    /// apply. Keyed on the plan's `ProcId` — a proc that vanished in the gap is
    /// itself the loudest staleness answer, and reaching it through the routing
    /// tables again would only re-derive the same fact more slowly.
    fn commit_phase<P, T>(
        &self,
        pid: ProcId,
        plan: &P,
        reply: Option<VerbReply>,
        commit: impl FnOnce(&Spine, &mut Proc, &P, Option<VerbReply>) -> Result<T, Refusal>,
    ) -> Result<T, Refusal> {
        // Divergent staleness: the proc vanished in the gap. Never retried — MISS
        // = FAULT applies to staleness too (§3.3 R5).
        let gone = || Refusal {
            fault: FwdFault::Stale(kayfabe_fwd::Stale::Proc(pid)),
            orphans: kayfabe_fwd::Orphans::default(),
            retry: false,
        };
        match self.mode {
            LockMode::Sharded => {
                let st = self.state.read();
                let Some(cell) = st.proc_cell(pid) else {
                    return Err(gone());
                };
                let mut p = cell.lock();
                commit(&st.spine, &mut p, plan, reply)
            }
            LockMode::Degenerate => {
                let mut g = self.state.write();
                let DeviceState {
                    spine,
                    system,
                    procs,
                } = &mut *g;
                let p = if pid == Gpu::SYSTEM_PROC {
                    system.get_mut()
                } else {
                    match procs.get_mut(&pid) {
                        Some(c) => c.get_mut(),
                        None => return Err(gone()),
                    }
                };
                commit(spine, p, plan, reply)
            }
        }
    }

    /// Is `pid` still in the live set? A device-read-only probe used to tell a
    /// genuine host failure apart from "the world moved" on the verb-error path.
    fn proc_is_live(&self, pid: ProcId) -> bool {
        match self.mode {
            LockMode::Sharded => self.state.read().proc_cell(pid).is_some(),
            LockMode::Degenerate => self.state.write().proc_cell(pid).is_some(),
        }
    }

    /// Return a checked-out worker to its pool slot (locked bookkeeping), then wake
    /// anyone waiting on pool-full backpressure.
    ///
    /// ★ **G3 (§12.16): a proc that retired in the lock-free gap still gets its worker
    /// back.** The old code dropped the handle when the live-map lookup missed and
    /// called that "the retire path owns the disposition". It does own the *host
    /// objects*; it does not own the *pool slot*, which stays marked checked-out with
    /// nobody holding it. Harmless while the reap trusted the caller — fatal now that
    /// it checks: the isolate would never report itself quiesced, so the proc would be
    /// deferred at every quiesce point for the life of the device and its GPA arena
    /// would never recycle (#80). Found by the §8.4 mean test, which reaped 1 of 2.
    ///
    /// Two-step by necessity: the fast path is device-read + proc lock; the fallback
    /// needs the device **write** lock to reach `Spine::retired`. They run
    /// sequentially, never nested — `route_act` has released both guards before the
    /// fallback takes one (R3: at most one lock per rank, and rank 0 is not held
    /// twice).
    fn return_worker(&self, pid: ProcId, gpu: GpuId, worker: Worker) {
        let mut hold = Some(worker);
        let _ = self.route_act(
            |_| Ok((pid, ())),
            |_, proc, ()| {
                if let Some(w) = hold.take() {
                    kayfabe_fwd::checkin(proc, gpu, w);
                }
            },
        );
        if let Some(w) = hold.take() {
            let mut g = self.state.write();
            g.spine.checkin_retired(pid, gpu, w);
        }
        self.pool.signal_return();
    }

    /// ★ **Stage a failed disposal's residue on its proc** — the two-step shape
    /// [`SharedDevice::return_worker`] uses, and for the same reason
    /// (`l1_os_shell.md` §7.5).
    ///
    /// A verb executes with every lock released, so its proc can retire in the gap —
    /// including *because of* the very escape that produced this residue. The live map
    /// then misses and the handles would be dropped on the floor: outstanding host
    /// objects that nothing in core state, and no release queue, can name. That is
    /// §12.35's UNACCOUNTED class exactly, and it is what the teardown audit fails on.
    ///
    /// So the fast path stages on the live proc, and the fallback reaches
    /// `Spine::retired` under the write lock. Both are pure bookkeeping (no verb), so
    /// neither violates R1; they run sequentially, never nested (R3).
    fn stage_orphans(&self, pid: ProcId, gpu: GpuId, orphans: kayfabe_isolate::Orphans) {
        if orphans.is_empty() {
            return;
        }
        let mut hold = Some(orphans);
        let _ = self.route_act(
            |_| Ok((pid, ())),
            |_, proc, ()| {
                if let Some(o) = hold.take() {
                    proc.stage_release(gpu, o);
                }
            },
        );
        if let Some(o) = hold.take() {
            let mut g = self.state.write();
            if let Some(p) = g.spine.retired_mut(pid) {
                p.stage_release(gpu, o);
            } else {
                // Nothing anywhere can name these: the proc was reaped in the gap, so its
                // isolate died and §7.0's process boundary already freed the lot. That is
                // a disposition, and saying so here is the difference between a leak and
                // a stated one.
                drop(o);
            }
        }
    }

    /// ★ **The cancel capability, handed out** (`l1_os_shell.md` §7.1): slot `worker`'s
    /// [`CancelHandle`], or `None` if that slot has no transaction in flight.
    ///
    /// This is the door the two-stage watchdog holds: a handle armed at checkout, kept
    /// somewhere the *cancelling* thread can reach, and fired later from a thread that
    /// has no `&mut Worker` and could not get one. It is `Send + Sync` and holds no
    /// reference to the worker or the backend, so keeping one across the lock gap is
    /// safe by construction rather than by discipline — and a request naming a txn that
    /// has since ended is simply dropped.
    #[must_use]
    pub fn cancel_handle(
        &self,
        pid: ProcId,
        gpu: GpuId,
        worker: WorkerId,
    ) -> Option<kayfabe_isolate::CancelHandle> {
        let mut out = None;
        let _ = self.route_act(
            |_| Ok((pid, ())),
            |_, proc, ()| {
                out = proc.isolate(gpu).and_then(|iso| iso.cancel_handle(worker));
            },
        );
        out
    }

    /// ★ Cancel ONE in-flight verb (§5.4's founding case, and the watchdog's **first**
    /// expiry — §7.5 step 2). Latched under the proc lock, discharged after it drops.
    ///
    /// `true` means the break signal was delivered to a live transaction. `false` means
    /// there was nothing to cancel *or* the verb finished first — §7.3's fourth row,
    /// which is a normal outcome and emphatically not a failure: the reply names host
    /// objects that now exist and must be committed, never discarded.
    ///
    /// Delivering it is **not** the same as the verb observing it: RM's waits are mostly
    /// uninterruptible (§7.9), so a delivered cancel that the host ignores is the
    /// expected case, not the exception. That is what the second budget escalates out
    /// of, via [`SharedDevice::declare_wedged`].
    pub fn request_cancel(
        &self,
        pid: ProcId,
        gpu: GpuId,
        worker: WorkerId,
        reason: kayfabe_isolate::CancelReason,
    ) -> bool {
        let mut latched: Option<kayfabe_isolate::CancelRequest> = None;
        let _ = self.route_act(
            |_| Ok((pid, ())),
            |_, proc, ()| {
                if let Some(iso) = proc.isolates.get_mut(&gpu) {
                    latched = iso.request_cancel(worker, reason);
                }
            },
        );
        // Guards dropped (R1): firing it is a syscall.
        latched.is_some_and(kayfabe_isolate::CancelRequest::discharge)
    }

    /// ★★ **THE WEDGE ESCAPE — one act, three consequences** (`l1_os_shell.md` §7.5).
    ///
    /// The watchdog's second expiry declares slot `worker` of `(pid, gpu)` **wedged**: a
    /// host thread in uninterruptible sleep that no signal can reach. Then, in ONE
    /// critical section:
    ///
    /// 1. the slot dies permanently ([`Isolate::worker_died`]) — never a respawn;
    /// 2. the **abandon** is latched for the requester parked on it;
    /// 3. the component is condemned ([`Spine::retire_proc`]), which is what makes
    ///    abandoning the reply safe: the desync hazard §7.2 forbids is a *future* reader
    ///    of that channel, and after this there is none.
    ///
    /// ★ **The three must not be reorderable steps**, which is why they share this
    /// function and this guard rather than being a documented sequence at a call site.
    /// The safety of the escape is *conditional on the condemnation*; a caller that did
    /// (2) without (3) would have reintroduced silent cross-transaction corruption, and
    /// nothing would say so.
    ///
    /// The abandon is **discharged after the guard drops** — firing it is a syscall
    /// (R1), exactly like every other cancel. Returns `false` if there was nothing to
    /// wedge (no such proc, target or checked-out slot), which is idempotent by design:
    /// a guest teardown racing the watchdog is an ordinary R5 gap.
    ///
    /// **The honest residual, stated where the code is:** the D-state host thread and its
    /// RM objects leak until the kernel finishes the ioctl — `SIGKILL` does not reap a
    /// task in uninterruptible sleep. What this converts is an unbounded silent stall
    /// into a bounded loud failure plus a leak we can name, count and report.
    pub fn declare_wedged(&self, pid: ProcId, gpu: GpuId, worker: WorkerId) -> bool {
        let cancels = {
            let mut g = self.state.write();
            let st = &mut *g;
            let Some(proc) = st.proc_mut(pid) else {
                return false;
            };
            let Some(iso) = proc.isolates.get_mut(&gpu) else {
                return false;
            };
            let Some(abandon) = iso.abandon(worker) else {
                return false;
            };
            iso.worker_died(worker);
            let mut cancels = kayfabe_isolate::Cancels::new();
            cancels.push(abandon);
            // (3) — the same act. `retire_proc` refuses the SYSTEM proc (§12.26), and a
            // wedged system worker is device-fatal rather than condemnable; the slot is
            // still dead and the requester is still released, which is the containment
            // this escape owes.
            st.spine
                .retire_proc(&mut ExclusiveProcs(&mut st.procs), pid);
            cancels.absorb(st.spine.take_pending_cancels());
            cancels
        };
        // Guards dropped (R1): now the syscall-shaped part may run.
        cancels.discharge_all();
        true
    }

    /// ★ The driver every verb-issuing op runs: **plan+checkout (locked) → execute
    /// (NO locks) → commit+check-in (re-locked, R5)**. Two locked phases per op, the
    /// shape §7.3 describes; the worker returns inside the same critical section that
    /// commits, so the common path takes each rank exactly twice.
    ///
    /// Two things send it back to the top, both holding **zero** locks:
    ///
    /// - **Pool full** — `stage` returned no worker. The thread parks on
    ///   [`PoolGate::wait_for_return`] and re-enters from the very top: re-routing,
    ///   re-planning, re-gating, because the proc may have retired while it waited
    ///   (R5). Backpressure, never a hang, never a spin.
    /// - **Converging staleness** — the commit refused with `retry` (a sibling
    ///   materialized the same host VAS / channel / engine object first). The loser
    ///   releases its duplicate and re-plans against the winner's state. Bounded by
    ///   [`MAX_COMMIT_RETRIES`]: each retry sees a *more* materialized world, so one
    ///   pass normally suffices; exhausting the bound surfaces the fault rather than
    ///   looping, because an unbounded retry is just a spin with extra steps.
    /// - ★★★ **Isolate pending** — R1's spawn deferral. `stage` refused with
    ///   [`FwdFault::IsolatePending`]: this `(proc, gpu)`'s sandbox was decided under a
    ///   write lock elsewhere and has not been installed yet. The thread materializes it
    ///   **itself** ([`SharedDevice::materialize_one`], lock-free) and re-enters from the
    ///   top with full R5 re-validation. DEFER, not FAULT — refusing here would turn a
    ///   legal concurrent state into a guest-visible fault, which is §12.9's "worse bug"
    ///   exactly. Bounded by [`MAX_COMMIT_RETRIES`] on the same counter, and the bound is
    ///   provably generous: after one materialization the pair is installed or the proc
    ///   is gone, and both are terminal.
    fn verb_op<P, T>(
        &self,
        stage: impl Fn() -> Result<Staged<P>, FwdFault>,
        commit: impl Fn(&Spine, &mut Proc, &P, Option<VerbReply>) -> Result<T, Refusal>,
    ) -> Result<T, FwdFault> {
        let mut retries = 0u32;
        loop {
            // Sampled BEFORE any lock is taken, so a return landing in the gap
            // cannot be missed (see `PoolGate`).
            let seen = self.pool.sample();
            let mut staged = match stage() {
                Ok(s) => s,
                // ★★★ R1's spawn deferral, RESOLVED rather than surfaced (see the docs).
                Err(FwdFault::IsolatePending { proc, gpu }) if retries < MAX_COMMIT_RETRIES => {
                    retries += 1;
                    self.materialize_one(proc, gpu);
                    continue;
                }
                Err(e) => return Err(e),
            };
            let Some(verbs) = staged.verbs else {
                // No host work at all (an idempotent replay): commit straight
                // through — the pool is never touched, so no worker to return.
                return self
                    .commit_phase(staged.proc, &staged.plan, None, commit)
                    .map_err(|r| r.fault);
            };
            let Some(mut worker) = staged.worker else {
                self.pool.wait_for_return(staged.gpu, seen);
                continue;
            };
            // ---- EXECUTE: no lock held. `Worker::execute` asserts exactly that. ----
            //
            // ★ T0's drain goes FIRST (§7.6 T0): the queue is what a previous `refresh`
            // could not release because it ran under the device write lock, and this
            // thread is now lock-free with a worker of the right `(proc, gpu)` isolate in
            // hand. Draining before the op's own chain means a map/unmap loop returns
            // host handles at least as fast as it consumes them, which is the whole
            // point — the alternative is a steady state that only ever grows. A failure
            // here is the isolate's own refusal (a retiring proc), where §7.0's
            // process-boundary backstop is the disposition of record.
            let _undisposed =
                kayfabe_fwd::dispose_on(&mut worker, core::mem::take(&mut staged.release));
            let executed = worker.execute(&verbs);
            let gpu = staged.gpu;
            let Ok(reply) = executed else {
                let failure = executed.expect_err("matched Err");
                let err = failure.err;
                // ★ What the worker's own cancel seam OBSERVED, read here — lock-free,
                // on the thread that ran the verb — so `FwdFault::Cancelled` can name
                // the truth (§7.3) without `RmError::Interrupted` growing a payload the
                // core has no business carrying.
                let reason = worker.cancel_observed();
                let wid = worker.id();
                // ★ G4 (§12.16): dispose of the failure's orphans on the SAME worker,
                // still lock-free, BEFORE returning it.
                //
                // ★★ §7.5 — UNLESS the worker is WEDGED. It cannot free anything: it is
                // still inside the ioctl that wedged it, so a disposal attempt is a
                // second wedge, not a cleanup. The chain's intermediates are STAGED on
                // the proc instead, which is what turns "in no `Orphans` and in no core
                // state" (G4's exact words) into a set §12.35's audit can name. Their
                // disposition of record is §7.0: the escape condemns the component and
                // the isolate's namespace death frees the lot.
                let residue = if err == RmError::Wedged {
                    failure.orphans
                } else {
                    kayfabe_fwd::dispose_on(&mut worker, failure.orphans)
                };
                self.stage_orphans(staged.proc, gpu, residue);
                self.return_worker(staged.proc, gpu, worker);
                // ★ R5 applies to the FAILURE path too (a stage-3 finding, §12.10).
                // `Proc::retire` retires the isolate, so a verb held in flight across
                // a retire comes back as an RM refusal — which is loud and
                // mutation-free, but names the wrong cause. Re-validate before
                // surfacing: if the proc vanished, the honest fault is staleness.
                //
                // ★ G4 (§12.16) — and CANCELLATION is tested FIRST, because it is the
                // one case where the proc is typically still live and the old order
                // therefore resolved it to `Rm(Other(..))`: §12.10's wrong-reason
                // conflation, one layer over. A cancelled verb is a fact about the
                // requester, not about the host and not about the proc's existence.
                return Err(match err {
                    RmError::Interrupted => FwdFault::Cancelled {
                        proc: staged.proc,
                        reason: reason.unwrap_or(kayfabe_isolate::CancelReason::GuestSignal),
                    },
                    // ★★ §7.5 — and the wedge is tested first for the same reason
                    // cancellation is: the proc is condemned by the escape, so the
                    // staleness arm below would otherwise re-type "we abandoned this"
                    // as "the world moved", which is true and useless.
                    RmError::Wedged => FwdFault::Wedged {
                        proc: staged.proc,
                        gpu,
                        worker: wid,
                    },
                    e if self.proc_is_live(staged.proc) => FwdFault::Rm(e),
                    _ => FwdFault::Stale(kayfabe_fwd::Stale::Proc(staged.proc)),
                });
            };
            // ---- COMMIT + CHECK-IN, one critical section (§7.3) ----
            let mut hold = Some(worker);
            let committed =
                self.commit_phase(staged.proc, &staged.plan, Some(reply), |sp, pr, pl, rp| {
                    let r = commit(sp, pr, pl, rp);
                    if r.is_ok()
                        && let Some(w) = hold.take()
                    {
                        kayfabe_fwd::checkin(pr, gpu, w);
                    }
                    r
                });
            match committed {
                Ok(v) => {
                    // The worker went back inside the commit section; wake anyone
                    // waiting on pool-full backpressure.
                    self.pool.signal_return();
                    return Ok(v);
                }
                Err(refusal) => {
                    let mut w = hold
                        .take()
                        .expect("a refused commit never checks the worker in");
                    // R5's disposition rule: a refused commit must not leak what it
                    // already allocated. Same worker, still lock-free.
                    //
                    // ★ The LEDGER of this line is pinned, not assumed
                    // (`l1_concurrency.md` §12.28): `tests/retry_ledger.rs` proves that
                    // across N converging re-plans — and across the
                    // [`MAX_COMMIT_RETRIES`] bound being *hit* — every host object a
                    // losing attempt allocated is released exactly once. Deleting this
                    // disposal leaks the attempt's duplicate host VAS, its memory object
                    // and its mapping, **per attempt**, which is what that test's set
                    // equality names.
                    let _undisposed = kayfabe_fwd::dispose_on(&mut w, refusal.orphans);
                    self.return_worker(staged.proc, gpu, w);
                    retries += 1;
                    if refusal.retry && retries < MAX_COMMIT_RETRIES {
                        continue;
                    }
                    return Err(refusal.fault);
                }
            }
        }
    }

    /// ★★★ **The declared facts a doorbell's channel needs to be resolvable at all** —
    /// its owning `hClient`, the `hVASpace` it named, and the GPFIFO ring it declared.
    ///
    /// # Why these three, and why from ONE node
    ///
    /// A GPU virtual address only means something inside an address space, and this port's
    /// only boot-path statement of where an address space's page directories live is keyed
    /// `(hClient, hVASpace)` (`kayfabe_device::gvaspub`; `execution_plane_increments.md`
    /// §14.9's census measured `SET_PAGE_DIRECTORY` at **zero**). Both handles and the ring
    /// address are declared by the **same** `RM_ALLOC` — the channel's — so they are read
    /// off the one graph node that declared them rather than joined from two projections
    /// that could disagree.
    ///
    /// ★ `[measured 2026-08-08]`, boot `run_p2_c89899a`, this join holds from both sides:
    /// the guest's own `channelWaitForFinishPayload` printed `hClient=0xc1e00006
    /// hVASpaceId=0xa` while our device recorded `gvas … hClient 0xc1e00006 hObject
    /// 0x0000000a`, and `workSubmitToken=0x10002` equals the token the doorbell carried.
    ///
    /// ⊘ **Read-only, and it resolves nothing.** It hands out declared facts; the walk that
    /// turns them into an address is `kayfabe_device::RegPlane::resolve_published_va`, and
    /// it needs a separate authorisation (`gmmu_publication_discipline.md` §7 rule 1).
    ///
    /// # Errors
    /// [`FwdFault`] from `route_doorbell`, or [`FwdFault::UnknownVchid`] if the routed
    /// channel is gone; [`FwdFault::NoVas`] if the channel's own graph node is not live, so
    /// the absence is named rather than answered with defaults.
    pub fn ce_channel_facts(
        &self,
        target_gpu: GpuId,
        token: u64,
    ) -> Result<CeChannelFacts, FwdFault> {
        self.route_act(
            |spine| {
                let r = kayfabe_fwd::route_doorbell(spine, target_gpu, token)?;
                Ok((r.proc, r))
            },
            |spine, proc, route| {
                let chan = proc
                    .channels
                    .get(&route.chan)
                    .ok_or(FwdFault::UnknownVchid {
                        gpu: route.gpu,
                        vchid: route.vchid,
                    })?;
                let node = spine
                    .rmgraph
                    .node_of_resource(chan.key)
                    .ok_or(FwdFault::NoVas(route.chan))?;
                // ★★★ **THE RESOLVED VA SPACE**, off the same `Channel::vas_origin` that
                // produced `vas_pdb` — see [`CeChannelFacts::vaspace`] for the boot in which
                // this line's predecessor (`node.facts.h_vaspace`) reported
                // `vas=NONE-DECLARED` for a channel whose `vas_pdb` was `Some`, and cost
                // every UVM doorbell its only ring-reading path.
                //
                // ⊘ A `vas_origin` that no longer resolves in the graph yields `None` here
                // rather than a stale handle: the resource died between the projection and
                // this read, and naming a dead VA space would send `ce_session` looking for
                // a publication that belongs to whatever inherits the handle value.
                let vas_node = chan
                    .vas_origin
                    .and_then(|k| spine.rmgraph.node_of_resource(k));
                Ok(CeChannelFacts {
                    proc: route.proc,
                    chan: route.chan,
                    vchid: route.vchid,
                    // ★★★★ §16.25 — carried off the channel, where the projection put it.
                    // ⊘ NOT re-derived here: this whole struct exists because a second
                    // derivation of the VA space disagreed with the first one and lost the
                    // channel `cuInit` walls on (see [`CeChannelFacts::vaspace`]).
                    vas_route: chan.vas_route,
                    // The namespace the VA SPACE lives in — which is the namespace its
                    // publication was issued in. ⊘ Falls back to the channel's own only
                    // when nothing resolved, so a refusal still names a client.
                    // ★★★★ §16.28 — route 4's answer lives in the channel's OWN namespace
                    // by construction: RM mints the device-default name with
                    // `serverutilGenResourceHandle(hClient, …)` on the client that owns
                    // the Device (`ogkm-580: gpu_vaspace.c:4101`). So the existing
                    // fallback — the channel's own client — is already the right one, and
                    // there is deliberately no third arm here.
                    client: vas_node.map_or(node.key.client.0, |v| v.key.client.0),
                    // ⊘ `None` (an `hVASpace` of 0) is carried as `None` and never folded to
                    // zero: a GSP-managed channel that named no VA space and one that named
                    // handle zero are the same wire byte but different facts, and only the
                    // first is what `Channel::vas_pdb == None` means.
                    // ★★★★ **§16.28 — THE FOURTH ROUTE REACHES THE DISPATCH HERE**, and
                    // it is an `or_else` rather than a second opinion: a channel that
                    // resolved a live VASpace resource keeps that answer untouched, and
                    // only a channel for which every declared route missed *and* whose
                    // parent Device named a default address space gets this one. The two
                    // can never disagree because they are never both `Some`
                    // (`project::resolve_channel_vas` returns at most one of them — route
                    // 4 runs only after routes 1-3 have all produced no node).
                    //
                    // ⊘ Nothing is invented: the value is the handle the guest's own RM
                    // minted, published its page-directory root under, and freed, and it
                    // is used as the key it is — `ce_session` looks the guest's own
                    // publication up under `(hClient, hVASpace)`.
                    vaspace: vas_node
                        .map(|v| v.key.handle.0)
                        .or(chan.vas_device_default.map(|h| h.0)),
                    // ★ Reported SEPARATELY as well as folded above, so a reader can tell
                    // which of the two produced `vaspace` without inferring it from
                    // `vas_route` — the §16.16 rule that two projections of one fact are
                    // printed side by side rather than reconciled in silence.
                    vaspace_device_default: chan.vas_device_default.map(|h| h.0),
                    // ★★★★ §16.16 — the DECLARED handle, read straight off this channel's
                    // own alloc facts. ⊘ Deliberately NOT resolved through the graph and
                    // NOT reconciled with `vaspace` above: the whole point is that it is
                    // the other projection, and a value passed through the same resolver
                    // would be the same projection twice. See
                    // `CeChannelFacts::vaspace_declared`.
                    vaspace_declared: node.facts.h_vaspace.map(|h| h.0),
                    ring_va: node.facts.gp_fifo_ring.map(|r| r.va),
                    ring_entries: node.facts.gp_fifo_ring.map_or(0, |r| r.entries),
                    // ★ Off the SAME node the ring came from, so the two declarations can
                    // never be attributed to different channels.
                    userd: node.facts.userd,
                    vas_pdb: chan.vas_pdb,
                    // ★ Off the SAME proc the channel was routed in, so a bind recorded for
                    // another proc's slot of the same index can never be read as this
                    // channel's — the join is `(proc, chan)`, exactly as `ExecPlane` keys
                    // it. ⊘ Read here rather than joined later for `ce_channel_facts`' own
                    // stated reason: two projections of one fact can disagree.
                    bound_engine: proc.exec.bound.get(&route.chan).copied(),
                    // ★★★★ §16.65 — off the SAME resolved `Channel` as `vas_pdb` two lines
                    // up, so the engine and the address space can never be attributed to
                    // different channels. ⊘ Deliberately NOT derived from `bound_engine`
                    // above: see this field's doc for the measurement that rules that out.
                    engine: chan.engine,
                })
            },
        )?
    }

    /// ★ THE one gated ring path (inherited law 7), route/act split per R4 and
    /// plan/execute/commit per R1.
    ///
    /// **Plan** (device read → proc lock): `route_doorbell` + `plan_doorbell` — the
    /// #14 ring-gate runs HERE, against the same locked snapshot the plan is derived
    /// from, before any host op exists. **Execute** (no locks): materialize /
    /// schedule / ring on a checked-out worker. **Commit** (re-locked): re-resolve
    /// the route and the channel, adopt the host handles, record the submission.
    /// ★★ `vmm` is the caller's **guest-memory port**, and `None` means *"this caller has
    /// none"* — the shell between `realize` and `attach_ram` genuinely has none, and a
    /// doorbell then cannot read a ring out of memory that is not mapped yet. ⊘ It is **not**
    /// a "skip the ring" switch: a caller that holds a port and passes `None` silently
    /// forwards nothing, which is `a_fallback_keyed_on_our_own_ignorance` waiting to happen.
    /// The one production caller (`kayfabe-qemu-raw`'s `SharedDoorbell::ring`) passes its
    /// attached port whenever it has one.
    pub fn doorbell(
        &self,
        vmm: Option<&mut dyn kayfabe_vmm::Vmm>,
        target_gpu: GpuId,
        token: u64,
        working_set: &[GpuVa],
    ) -> Result<DoorbellOutcome, FwdFault> {
        let out = self.verb_op(
            || {
                self.route_act(
                    |spine| {
                        let r = kayfabe_fwd::route_doorbell(spine, target_gpu, token)?;
                        Ok((r.proc, r))
                    },
                    |_spine, proc, route| {
                        let planned = kayfabe_fwd::plan_doorbell(proc, &route, working_set)?;
                        Staged::check_out(proc, planned.plan.cgpu, planned)
                    },
                )?
            },
            kayfabe_fwd::commit_doorbell,
        )?;
        if let Some(vmm) = vmm {
            self.forward_ring(vmm, out.proc, out.chan)?;
        }
        Ok(out)
    }

    /// ★★★ **The guest's ring, forwarded — the doorbell half that was missing.**
    ///
    /// Until this existed, `SERVED` on a forwarding plane meant, in
    /// `execution_plane_increments.md` §15.5's own words, *"we rang a doorbell on a host
    /// channel into which the guest's methods were never copied"*: the whole
    /// parse → plan → execute chain that ends at the only function in this tree which
    /// observes a real host completion — `HostRmBackend::await_semaphore`
    /// (`kayfabe-isolate-host/src/rm.rs:2897`, reached through `ce_copy_outcome` and
    /// `RmBackend::ce_copy`) — had **zero** production callers, so a guest action could not
    /// reach it in any build. `tests/tests/doorbell_reaches_the_completion_observer.rs` is
    /// that statement as a test, and it was RED at the commit before this one.
    ///
    /// # ⚠ AFTER the ring, deliberately, and this is not a stylistic order
    ///
    /// `plan_doorbell` is what materializes the channel's host VAS and host channel on
    /// first submission, and `plan_ce` needs `Vas::host_vas` to point an engine at
    /// anything. Forwarding first would refuse the guest's *first* copy on every channel,
    /// on a plane where that copy is the whole point.
    ///
    /// ⊘ **The host doorbell this follows is not a completion.** It rings the isolate's
    /// host channel, which the guest's methods are never copied into
    /// (`SharedDevice::submit_ring`'s own note); the copy is executed by the isolate on its
    /// own channel and waited for there. So nothing here signals the guest before its work
    /// ran, and nothing here signals the guest at all — see the ⊘ below.
    ///
    /// # ⊘ What this does NOT do — the completion tail
    ///
    /// It does not write the guest's finishPayload and raises no interrupt. That is the
    /// order §15.5 argues for and gives the reason for: *"Adding the completion tail to the
    /// forwarding path would advance the payload for work that never happened — a forged
    /// completion … exactly what the C did."* Wire the ring first, complete second. Arm (a)
    /// — the ops we emulate ourselves — already completes honestly in
    /// `kayfabe_rt::ceutils`; this is arm **(b)** only.
    ///
    /// # ⚠ The #14 ring-gate still sees an EMPTY working set, and that is stated on purpose
    ///
    /// `plan_doorbell` runs `VerbPlan::gated_doorbell` over the `working_set` the caller
    /// passed, and the shell passes `&[]` — `execution_plane_increments.md` §15.5 check 1
    /// names that as the reason the forwarding path was never given the ring
    /// (*"recovering which VAs a submission touches means parsing the ring"*). This wiring
    /// parses the ring **after** that gate, so it does **not** close it. What gates the
    /// operands instead is the plan below: `partition_ce` classifies every span by
    /// representability and `plan_ce` refuses by name (`NoHostVas`, and an unpublished
    /// operand forwards as `Untracked` rather than as a guessed binding). ⊘ Feeding the
    /// recovered VAs *back* into the gate would mean gating on addresses recovered after
    /// the gate ran, which is a different increment and a different argument.
    ///
    /// # The phases, and why the cursor is committed last
    ///
    /// **Read** (rank 0 + the proc's rank 1): the ring, the resume point, and the arch's
    /// entry stride, in one locked look so the three cannot come from different instants.
    /// **Parse** and **forward**: each takes and releases its own locks (see
    /// [`SharedDevice::submit_ring`] — the execute phase runs a host verb and R1 forbids
    /// any ranked lock across it). **Commit**: advance
    /// [`kayfabe_core::gpu::ExecPlane::forwarded`] **only** on success, so a refused
    /// submission leaves the ring exactly where it was and the guest's own retry re-reads
    /// the entry it could not run.
    fn forward_ring(
        &self,
        vmm: &mut dyn kayfabe_vmm::Vmm,
        pid: ProcId,
        cid: ChanId,
    ) -> Result<(), FwdFault> {
        // ---- READ. One locked look: the ring, where we stopped, and the stride.
        let read = self.route_act(
            |_spine| Ok((pid, ())),
            |spine, proc, ()| -> Result<Option<(Vec<u8>, u32, u32)>, FwdFault> {
                let Some(ring) = kayfabe_fwd::read_gpfifo_ring(spine, proc, cid, vmm)? else {
                    return Ok(None);
                };
                let done = proc.exec.forwarded.get(&cid).copied().unwrap_or(0);
                let pb = spine.arch().pushbuffer();
                let at = (done as usize).saturating_mul(pb.gpfifo_entry_stride());
                if at >= ring.len() {
                    return Ok(None);
                }
                let fresh = &ring[at..];
                // ⊘ Only the entries the guest has WRITTEN. The tail of a ring is zeros,
                // and a zero entry is the ring saying "no more work" — not a malformed one.
                let n = kayfabe_fwd::gpfifo_live_entries(pb, fresh);
                if n == 0 {
                    return Ok(None);
                }
                let take = n * pb.gpfifo_entry_stride();
                Ok(Some((
                    fresh[..take].to_vec(),
                    done,
                    u32::try_from(n).unwrap_or(u32::MAX),
                )))
            },
        )??;
        let Some((fresh, done, n)) = read else {
            return Ok(());
        };

        // ---- PARSE, then FORWARD. Each half takes and releases its own locks.
        let parsed = self.parse_pushbuffer(vmm, pid, cid, &fresh)?;
        if !parsed.ce_spans.is_empty() {
            // ★★★ THE OBSERVER. `forward_ce` → `plan_ce` → `Worker::execute`'s `CeSplit`
            // arm → `RmBackend::ce_copy`, whose host implementation waits on the engine's
            // own release semaphore and answers `CE_NEVER_RETIRED` if it never came. The
            // `?` is what makes that verdict the guest's: a doorbell whose copy did not
            // retire must not report `Served` (§14.8).
            self.forward_ce(pid, cid, &parsed.ce_spans)?;
        }

        // ---- COMMIT the cursor, on the success arm and nowhere else.
        self.with_proc_mut(pid, |p| {
            p.exec.forwarded.insert(cid, done.saturating_add(n));
        });
        Ok(())
    }

    /// ★★ Parse the pushbuffer `ring` submitted on channel `cid` of proc `pid`, in the
    /// **route/act** shape: the ring's GPFIFO entries are decoded under the device
    /// **read** lock (rank 0) touching no proc, and the act phase — that proc's lock,
    /// rank 1 — translates each entry's address, reads the method words, and applies
    /// them.
    ///
    /// # ★★★ This is the ONLY in-lock-legal entry point that takes a guest-chosen address
    ///
    /// ⊘ **This paragraph used to open *"Every GPFIFO entry in `ring` names a
    /// guest-physical address"*, and `ogkm` refutes it.** A GA10x GPFIFO entry names a
    /// GPU **virtual** address in the issuing channel's address space — UVM writes
    /// `uvm_pushbuffer_get_gpu_va_for_push(...)` into it (`ogkm-580:
    /// kernel-open/nvidia-uvm/uvm_channel.c:996, 1006`; the field is
    /// `NVC56F_GP_ENTRY0_GET`/`_GET_HI`, `ogkm-580: clc56f.h:270, 272`). The statement was
    /// true of `kayfabe_mocks::MockPushbuffer`, whose invented entry really did carry a
    /// GPA, and that is the whole reason it survived.
    ///
    /// # ★★★ The PHASE MOVED (§8.2.3), and here is exactly what that cost
    ///
    /// Translating the entry's VA needs the issuing channel's `Vas`, which needs the
    /// proc, so `kayfabe_fwd::read_pushbuffer` moved from the route phase into the act
    /// phase. **No new lock is taken.** [`SharedDevice::route_act`] acquires device-read
    /// (rank 0) and then that proc's mutex (rank 1) for one operation; the guest-memory
    /// read moved from between those two acquisitions to after the second. The lock set
    /// and the rank order of the whole entry point are unchanged, and nothing was widened
    /// quietly — this note is the widening, stated.
    ///
    /// The lock argument itself is **unchanged, because it never mentioned a rank.** The
    /// address in each GPFIFO entry is guest-chosen and `Vmm::gpa_read` runs here with a
    /// ranked lock held; that combination is legal **only** because the port refuses a GPA
    /// that does not resolve to host RAM (`kayfabe_vmm::GuestRamMap`) — a backend that
    /// served a device-aimed GPA would take the VMM's global lock beneath one of ours,
    /// which is `l1_os_shell.md` §6.3's ABBA inversion built to order by the guest,
    /// whether the lock above it is rank 0 or rank 1. The refusal surfaces as
    /// [`FwdFault::NonRamGpa`].
    ///
    /// ⊘ **Not split into plan/execute/commit**, deliberately: R1 forces that shape for
    /// *blocking* calls, and `gpa_read` is a bounded copy out of a mapped window. The
    /// split would mean resolving addresses, dropping the lock, then fetching method bytes
    /// through a translation the guest was free to invalidate in the gap — a TOCTOU built
    /// on purpose. See `kayfabe_fwd::read_pushbuffer`.
    ///
    /// It is wired here rather than left to callers precisely because it was *not*
    /// reachable through the shell before: `read_pushbuffer`'s docs claimed *"in L1 this
    /// runs under the device read lock"* while no L1 entry point ran it at all, so the
    /// only path that can construct the hazard existed solely in tests holding a bare
    /// `&mut Gpu` and no lock (`testing_doctrine.md` §1).
    pub fn parse_pushbuffer(
        &self,
        vmm: &mut dyn kayfabe_vmm::Vmm,
        pid: ProcId,
        cid: ChanId,
        ring: &[u8],
    ) -> Result<kayfabe_fwd::PushbufferOutcome, FwdFault> {
        let out = self.route_act(
            // ROUTE phase — rank 0 held, no proc, no guest memory: the arch's GPFIFO
            // entry format and nothing else.
            move |spine| Ok((pid, kayfabe_fwd::pushbuffer_ranges(spine, ring))),
            // ACT phase — rank 0 + this proc's rank 1. Translate through the channel's
            // own address table, read, decode, apply. The read and the apply are one
            // phase because they must see the same table.
            move |spine, proc, ranges| {
                let methods = kayfabe_fwd::read_pushbuffer(spine, proc, cid, vmm, &ranges)?;
                kayfabe_fwd::apply_pushbuffer(spine, proc, cid, methods)
            },
        )??;
        // ★★★ #102/#13 — LATCH phase, and it needs its own pass for a structural reason.
        //
        // The act phase above holds the **issuing** proc's lock, and the owner of a
        // written page-table page is routinely a *different* proc: the guest kernel is
        // what writes a user process's page tables. So the latch cannot happen inside the
        // act — it would need a second rank-1 lock, which R3 refuses. Each owner is
        // visited on its own, one lock at a time, exactly as `live_pids`/`with_proc` are
        // designed to be walked.
        //
        // An owner that retired in the gap is simply skipped: its page tables are gone,
        // and re-attaching a dirty page to whoever inherits its id is the C's
        // never-pruned-table aliasing class.
        for w in &out.pt_writes {
            self.with_proc_mut(w.owner, |p| {
                if let Some(vas) = p.vases.get_mut(&(w.gpu, w.owner_pdb)) {
                    vas.pt_pages.insert(w.page);
                }
            });
        }
        Ok(out)
    }

    /// ★★★ **E6 — issue a partitioned copy-engine request**, in the three phases R1
    /// forces: `kayfabe_fwd::plan_ce` (device read → the submitting proc's lock, and the
    /// worker checked out as the last thing under it) → `Worker::execute` (**no lock
    /// held**) → `kayfabe_fwd::commit_ce` (re-locked, R5).
    ///
    /// `spans` is what [`SharedDevice::parse_pushbuffer`] recovered from the guest's own
    /// ring. It is passed through rather than re-derived: partitioning again here would be
    /// a second answer to a question already answered against a table this call no longer
    /// holds a lock on.
    ///
    /// ⊘ **The route is the caller's `pid`, not a spine lookup**, and that is a real
    /// limitation stated rather than papered over. The submitting proc is whoever owns the
    /// ring that was parsed, and the parse was already addressed that way. A doorbell-driven
    /// caller routes through `kayfabe_fwd::route_doorbell` first and hands the `ProcId` it
    /// resolved; nothing here re-derives it, so nothing here can disagree with it.
    ///
    /// # Errors
    /// [`FwdFault`], by variant — see `kayfabe_fwd::plan_ce` for the three refusals it
    /// owns, plus staleness from the commit.
    pub fn forward_ce(
        &self,
        pid: ProcId,
        cid: ChanId,
        spans: &[kayfabe_fwd::CeSpan],
    ) -> Result<kayfabe_fwd::CeForwarded, FwdFault> {
        self.verb_op(
            || {
                self.route_act(
                    |_spine| Ok((pid, ())),
                    |_spine, proc, ()| {
                        let planned = kayfabe_fwd::plan_ce(proc, cid, spans)?;
                        Staged::check_out(proc, planned.plan.gpu, planned)
                    },
                )?
            },
            |_spine, proc, plan, reply| kayfabe_fwd::commit_ce(proc, plan, reply),
        )
    }

    /// ★★★ **E6 — THE JOIN**: the guest's ring, parsed and then *issued*.
    /// [`SharedDevice::parse_pushbuffer`] followed by [`SharedDevice::forward_ce`].
    ///
    /// Each half takes and releases its own locks, so the two are **not** one critical
    /// section — and they must not be: the forward's execute phase runs a host verb, and
    /// `Worker::execute` panics naming R1 if any ranked lock is held. The gap between them
    /// is therefore real, and what covers it is `commit_ce`'s R5 re-validation, not an
    /// assumption that nothing moved.
    ///
    /// ⊘ **What this does NOT do, stated because a reader will assume it:** it does not
    /// ring anything, and it does not decode page tables. A copy-engine request is executed
    /// by the isolate on its own host channel (`kayfabe_isolate_host::rm::ce_copy`), which
    /// rings its own doorbell; the guest's channel is never given a host ring to replay
    /// into. And the page-table pages the parse witnessed are latched but not decoded —
    /// see [`SharedDevice::decode_pt_writes`], which needs a worker and visits other procs.
    ///
    /// # Errors
    /// [`FwdFault`] from either half.
    pub fn submit_ring(
        &self,
        vmm: &mut dyn kayfabe_vmm::Vmm,
        pid: ProcId,
        cid: ChanId,
        ring: &[u8],
    ) -> Result<(kayfabe_fwd::PushbufferOutcome, kayfabe_fwd::CeForwarded), FwdFault> {
        let parsed = self.parse_pushbuffer(vmm, pid, cid, ring)?;
        let forwarded = self.forward_ce(pid, cid, &parsed.ce_spans)?;
        Ok((parsed, forwarded))
    }

    /// ★★★ **The page-table decode pass** (`#102` stage C3) for one proc, in the three
    /// phases R1 forces — the shell half of `kayfabe_fwd::plan_pt_decode` /
    /// `run_pt_decode` / `commit_pt_decode`.
    ///
    /// Called at the guest's own commit point: the **CE release semaphore**, which is
    /// where the guest declares its page-table writes complete
    /// (`C: nvkvm_gpu_emul.c:8676-8695`). Not at an invalidate — `#102` stage C2 measured
    /// that there is no read-at-invalidate on this path (§13.4), so this pass is the only
    /// thing that turns a witnessed write into a mapping.
    ///
    /// **Plan** (rank 1): drain the proc's dirtied page-table pages. **Execute** (no
    /// lock): decode them over `worker`'s isolate, one round trip per page.
    /// **Commit** (rank 1): re-resolve each `Vas` (R5) and forward-populate.
    ///
    /// ★ **`fmt` is a parameter, and that is R1 made visible.** The format lives in the
    /// spine behind the device read lock, so a `&dyn GmmuFmt` obtained from it cannot
    /// outlive the guard — and the execute phase must run with no guard held. Having to
    /// pass it in is the type system stating the constraint rather than a comment stating
    /// it.
    ///
    /// Returns `None` if `pid` is not live.
    ///
    /// # Panics
    /// If a ranked lock is somehow held across the execute phase: `Worker::fb_read`
    /// asserts it, and the assertion is the reason the read goes through a worker.
    pub fn decode_pt_writes(
        &self,
        pid: ProcId,
        fmt: &dyn kayfabe_arch::GmmuFmt,
        worker: &mut kayfabe_isolate::Worker,
    ) -> Option<kayfabe_fwd::PtDecodeOutcome> {
        // PLAN — rank 1, the owner's lock and no other.
        let plan = self.with_proc_mut(pid, kayfabe_fwd::plan_pt_decode)?;
        // EXECUTE — no lock held. `with_proc_mut` released it above, and the borrow of
        // the plan is of owned data, so nothing keeps a guard alive into here (★ a `Drop`
        // is a call site: `plan` owns `Vec`s of plain values, whose drop touches nothing).
        let results = {
            let mut fb = kayfabe_fwd::IsolateFb::new(worker);
            let r = kayfabe_fwd::run_pt_decode(
                fmt,
                &mut fb,
                &plan.tasks,
                kayfabe_fwd::PT_DECODE_BUDGET,
            );
            (r, fb.transport_error())
        };
        // COMMIT — rank 1 again, re-resolving every target (R5).
        let mut out =
            self.with_proc_mut(pid, |p| kayfabe_fwd::commit_pt_decode(fmt, p, &results.0))?;
        // ★★★ **PUBLISH** — E8, rank 0, and the phase E5 could not have. The pages this
        // pass learned go into the device-global ownership index, so the NEXT guest CE
        // write into one of them is classified as a page-table write instead of forwarded
        // as ordinary data. Without it the decode learns the whole subtree and the index
        // still knows only roots, which is exactly what
        // `the_ce_pt_write_source_can_witness_only_a_root_page_today` measured.
        //
        // ⚠ **The ordering is the increment.** `with_proc_mut` above has returned, so its
        // rank-1 guard is dropped; taking the rank-0 write guard here is an acquisition
        // from nothing, not an inversion. Publishing from inside `commit_pt_decode` — the
        // obvious place — would take rank 0 beneath rank 1 and is the ABBA §9.2 refused to
        // build blind.
        //
        // R5 lives in `Spine::publish_pt_pages`: every `(gpu, pdb)` is re-resolved against
        // `by_pdb` and a page whose address space died, or whose PDB now belongs to some
        // other proc, publishes nothing.
        if !out.learned_pages.is_empty() {
            let mut g = self.state.write();
            let st = &mut *g;
            // Grouped so the R5 re-resolve is paid once per address space rather than once
            // per page; `learned_pages` arrives in task order, which is already grouped,
            // but nothing in the type says so and a fold that assumed it would be a silent
            // dependency on the pass's iteration order.
            let mut by_vas: std::collections::BTreeMap<(GpuId, Pdb), Vec<u64>> =
                std::collections::BTreeMap::new();
            for &(gpu, pdb, page) in &out.learned_pages {
                by_vas.entry((gpu, pdb)).or_default().push(page);
            }
            for ((gpu, pdb), pages) in by_vas {
                let (published, refused) = st.spine.publish_pt_pages(pid, gpu, pdb, pages);
                out.pages_published += published;
                out.pages_publish_refused += refused;
            }
        }
        // ★ A transport failure is surfaced as its own fact rather than blended into the
        // walk faults: `Ok(false)` from the isolate is a statement about the GUEST's page
        // tables, an `Err` is a statement about US, and a caller that cannot tell them
        // apart debugs the wrong plane.
        out.transport = results.1;
        Some(out)
    }

    /// Back `[va, va+len)` in the `(gpu, pdb)` VAS. **Plan**: route via `by_pdb`,
    /// read the Vas's host VAS. **Execute**: host VAS (first touch) + sysmem alloc +
    /// map, lock-free. **Commit**: re-resolve the Vas, adopt the host VAS, carve the
    /// GPA from the proc's own arena, forward-populate the address table.
    pub fn publish_backing(
        &self,
        gpu: GpuId,
        pdb: Pdb,
        va: GpuVa,
        len: u64,
    ) -> Result<Published, FwdFault> {
        self.verb_op(
            || {
                self.route_act(
                    |spine| Ok((kayfabe_fwd::route_pdb(spine, gpu, pdb)?, ())),
                    |_spine, proc, ()| {
                        let planned = kayfabe_fwd::plan_publish(proc, gpu, pdb, va, len)?;
                        Staged::check_out(proc, gpu, planned)
                    },
                )?
            },
            |_spine, proc, plan, reply| kayfabe_fwd::commit_publish(proc, plan, reply),
        )
    }

    /// **Case 1**: forward an engine-object alloc on the channel identified by
    /// `vchid`, same three phases. An idempotent re-send resolves entirely in the
    /// plan phase and issues **no verbs and no checkout at all**.
    pub fn forward_engine_object(
        &self,
        target_gpu: GpuId,
        vchid: VChid,
        class: ClassId,
        params: &[u8],
    ) -> Result<EngineObjectForwarded, FwdFault> {
        self.verb_op(
            || {
                self.route_act(
                    |spine| {
                        let r = kayfabe_fwd::route_engine_object(spine, target_gpu, vchid, class)?;
                        Ok((r.proc, r))
                    },
                    |_spine, proc, route| {
                        let planned = kayfabe_fwd::plan_engine_object(proc, &route, class, params)?;
                        Staged::check_out(proc, planned.plan.cgpu, planned)
                    },
                )?
            },
            kayfabe_fwd::commit_engine_object,
        )
    }

    /// ★★ Apply one context promotion — the **sharded** form of
    /// `kayfabe_core::promote`'s two functions.
    ///
    /// # Why it is two passes and not one
    ///
    /// The proc that *issues* a `GPU_PROMOTE_CTX` and the proc that *owns* the address
    /// space it names are not required to be the same one, and R3 forbids holding two
    /// rank-1 locks. So the route is a rank-0 spine read that touches no proc, and the
    /// apply takes the **owner's** lock alone — the same shape as `#102`'s page-table
    /// latch, and for the same reason.
    ///
    /// This is also §5 of `gpu_promote_ctx.md` discharged: `route_control` answers a
    /// Case-2 control under the read lock *before any `Proc` is touched*, so the harvest
    /// could not live inside it. It is a separate verb, not a widened one.
    ///
    /// A proc that retired in the gap is [`kayfabe_core::promote::PromoteFault::RetiredProc`]
    /// rather than a silent skip: unlike a dirty page-table page (whose owner's tables are
    /// simply gone), a promotion is a control the guest is blocked waiting on, and it has
    /// to be answered.
    ///
    /// # Errors
    ///
    /// [`kayfabe_core::promote::PromoteFault`], by variant.
    pub fn promote_ctx(
        &self,
        p: &kayfabe_core::promote::CtxPromotion,
    ) -> Result<kayfabe_core::promote::PromoteJoin, kayfabe_core::promote::PromoteFault> {
        // ROUTE — rank 0 only. No proc lock is held while this runs.
        //
        // ★★★★★ §16.50 — the GPU-scoped global context-buffer publications are read
        // HERE, in the same rank-0 section, and carried into the act phase as a value.
        // ⊘ They are deliberately NOT reached through a lock taken inside the proc-lock
        // section: that would nest rank 0 inside rank 1 and is exactly what R3 forbids.
        // The map holds at most three entries, so passing it by value costs nothing.
        let (route, mut globals) = {
            let route_in = |spine: &kayfabe_core::gpu::Spine| {
                let route = kayfabe_core::promote::route_promote_ctx(
                    spine,
                    p.client,
                    p.chan_client,
                    p.object,
                )?;
                let globals = spine.global_ctx_phys_for(route.gpu);
                Ok::<_, kayfabe_core::promote::PromoteFault>((route, globals))
            };
            match self.mode {
                LockMode::Sharded => route_in(&self.state.read().spine),
                LockMode::Degenerate => route_in(&self.state.write().spine),
            }?
        };
        // ACT — the OWNING proc's rank-1 lock, alone.
        let out = self
            .with_proc_mut(route.proc, |proc| {
                kayfabe_core::promote::apply_promote_ctx(proc, &route, p, &mut globals)
            })
            .unwrap_or(Err(kayfabe_core::promote::PromoteFault::RetiredProc(
                route.proc,
            )));
        // MERGE — back at rank 0, with the proc lock RELEASED. The order is 0 → 1 → 0 and
        // never nested, so this is a re-acquisition rather than a lock-order inversion.
        //
        // ★ Only on success. A refused promotion published nothing the device should keep:
        // `apply_promote_ctx` stages the whole join over a scratch copy and commits
        // nothing on refusal, and letting its half-built `globals` reach the spine would
        // make a refusal partially take effect — the one asymmetry between this shell and
        // `Gpu::promote_ctx` that would matter.
        //
        // ⚠ **The visibility window is named, not hidden**: a publication made by
        // promotion N becomes joinable by promotion N+1, not by a promotion racing it.
        // That is sound for the shape `[measured 2026-08-09, rev 62e757f, boot
        // s41b_62e757f_twophase]` recorded — the physical published under `ProcId(0)` at
        // driver init, the VA halves declared under `ProcId(2)` at `cuCtxCreate`, many
        // controls later — and its worst case is one extra orphaned round, never a wrong
        // binding.
        if out.is_ok() {
            // ⊘ No `self.mode` branch: the merge is a WRITE in both modes, so the two
            // arms would be identical. Degenerate mode differs only in that the route
            // phase already had to take the write lock.
            self.state
                .write()
                .spine
                .merge_global_ctx_phys(route.gpu, &globals);
        }
        out
    }

    /// ★★★ **#177** — perform the guest's `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`, in the
    /// shell's own two ranks: ROUTE under the device read lock, ACT under the owning
    /// proc's lock alone. Identical in shape to [`Self::promote_ctx`], and for the same
    /// reason — the core provides the two halves so this is a composition, not a second
    /// implementation.
    ///
    /// # Errors
    ///
    /// [`kayfabe_core::gpu::ScheduleFault`], by variant.
    pub fn schedule_channel(
        &self,
        client: kayfabe_arch::ids::HClient,
        object: kayfabe_arch::ids::HObject,
        enable: bool,
    ) -> Result<kayfabe_core::gpu::ScheduleAck, kayfabe_core::gpu::ScheduleFault> {
        let route = {
            let route_in = |spine: &kayfabe_core::gpu::Spine| {
                kayfabe_core::gpu::route_schedule_channel(spine, client, object)
            };
            match self.mode {
                LockMode::Sharded => route_in(&self.state.read().spine),
                LockMode::Degenerate => route_in(&self.state.write().spine),
            }?
        };
        self.with_proc_mut(route.proc, |proc| {
            kayfabe_core::gpu::apply_schedule_channel(proc, &route, enable)
        })
        .ok_or(kayfabe_core::gpu::ScheduleFault::ChannelNotMaterialized { client, object })
    }

    /// ★★★★ **§16.56** — perform the guest's `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` (the TSG
    /// form), in the shell's two ranks: ROUTE under the device read lock, ACT under the
    /// owning proc's lock alone. Identical in shape to [`Self::schedule_channel`], and a
    /// composition of the core's two halves rather than a second implementation.
    ///
    /// ⊘ One proc lock, not one per member: [`kayfabe_core::gpu::route_schedule_group`]
    /// refuses a group whose members span procs, so the whole fan-out is inside a single
    /// rank-1 lock and there is no lock-order question to get wrong.
    ///
    /// # Errors
    ///
    /// [`kayfabe_core::gpu::ScheduleGroupFault`], by variant.
    pub fn schedule_group(
        &self,
        client: kayfabe_arch::ids::HClient,
        object: kayfabe_arch::ids::HObject,
        enable: bool,
    ) -> Result<kayfabe_core::gpu::ScheduleGroupAck, kayfabe_core::gpu::ScheduleGroupFault> {
        let route = {
            let route_in = |spine: &kayfabe_core::gpu::Spine| {
                kayfabe_core::gpu::route_schedule_group(spine, client, object)
            };
            match self.mode {
                LockMode::Sharded => route_in(&self.state.read().spine),
                LockMode::Degenerate => route_in(&self.state.write().spine),
            }?
        };
        let members = route.chans.len() + route.unmaterialized;
        self.with_proc_mut(route.proc, |proc| {
            kayfabe_core::gpu::apply_schedule_group(proc, &route, enable)
        })
        .ok_or(
            kayfabe_core::gpu::ScheduleGroupFault::NoMemberMaterialized {
                client,
                object,
                members,
            },
        )
    }

    /// ★★★★ **§16.59 — verify the guest's `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE`**
    /// (`0x20801210`) — the sharded form of
    /// [`kayfabe_core::gpu::Gpu::set_ctxsw_preemption_mode`].
    ///
    /// ⊘ **Rank 0 only, and no proc lock is taken**, because nothing is recorded: this
    /// control asks about a postcondition the execution plane is unconditionally in, so
    /// the whole answer is a route. See [`kayfabe_core::gpu::CtxswPreemptionAck`].
    ///
    /// # Errors
    ///
    /// [`kayfabe_core::gpu::CtxswPreemptionFault`], by variant.
    pub fn set_ctxsw_preemption_mode(
        &self,
        client: kayfabe_arch::ids::HClient,
        h_channel: kayfabe_arch::ids::HObject,
    ) -> Result<kayfabe_core::gpu::CtxswPreemptionAck, kayfabe_core::gpu::CtxswPreemptionFault>
    {
        let route_in = |spine: &kayfabe_core::gpu::Spine| {
            kayfabe_core::gpu::route_ctxsw_preemption(spine, client, h_channel)
        };
        match self.mode {
            LockMode::Sharded => route_in(&self.state.read().spine),
            LockMode::Degenerate => route_in(&self.state.write().spine),
        }
    }

    /// ★★★ **E9/§13.6 — perform the guest's `NVA06F_CTRL_CMD_BIND`** — the sharded form
    /// of [`kayfabe_core::gpu::Gpu::bind_channel`], under the same two-lock split as
    /// [`Self::schedule_channel`]: route under the device lock (rank 0), record under the
    /// owning proc's lock (rank 1).
    ///
    /// `rm_engine_type` is in **RM engine space** — the policy converts and checks it
    /// against the advertised set before calling; every fault here is about the channel.
    ///
    /// # Errors
    ///
    /// [`kayfabe_core::gpu::BindFault`], by variant.
    pub fn bind_channel(
        &self,
        client: kayfabe_arch::ids::HClient,
        object: kayfabe_arch::ids::HObject,
        rm_engine_type: u32,
    ) -> Result<kayfabe_core::gpu::BindAck, kayfabe_core::gpu::BindFault> {
        let route = {
            let route_in = |spine: &kayfabe_core::gpu::Spine| {
                kayfabe_core::gpu::route_bind_channel(spine, client, object)
            };
            match self.mode {
                LockMode::Sharded => route_in(&self.state.read().spine),
                LockMode::Degenerate => route_in(&self.state.write().spine),
            }?
        };
        self.with_proc_mut(route.proc, |proc| {
            kayfabe_core::gpu::apply_bind_channel(proc, &route, rm_engine_type)
        })
        .ok_or(kayfabe_core::gpu::BindFault::ChannelNotMaterialized { client, object })
    }

    /// Route a `GSP_RM_CONTROL` through the Case-1/Case-2 split. Case 2 is ACKed
    /// under the device read lock and never leaves the process; Case 1 runs the same
    /// three phases, with `payload` written back in the commit.
    pub fn route_control(
        &self,
        target_gpu: GpuId,
        pid: ProcId,
        obj: HostHandle,
        cmd: ControlCmd,
        payload: &mut [u8],
    ) -> Result<ControlRoute, FwdFault> {
        {
            let ack = match self.mode {
                LockMode::Sharded => kayfabe_fwd::classify_control(&self.state.read().spine, cmd),
                LockMode::Degenerate => {
                    kayfabe_fwd::classify_control(&self.state.write().spine, cmd)
                }
            };
            if let ControlRoute::AckOnly = ack {
                return Ok(ControlRoute::AckOnly);
            }
        }
        // The commit writes the host's answer back into the caller's buffer, so the
        // payload rides the plan by value and returns by value — a plan may not
        // borrow across the lock gap (R5's "IDs, never held references").
        let out: Vec<u8> = self.verb_op(
            || {
                self.route_act(
                    |_| Ok((pid, ())),
                    |_spine, proc, ()| {
                        let planned =
                            kayfabe_fwd::plan_control(proc, target_gpu, obj, cmd, payload)?;
                        Staged::check_out(proc, target_gpu, planned)
                    },
                )?
            },
            |_spine, proc, plan, reply| {
                let mut buf = vec![0u8; payload.len()];
                kayfabe_fwd::commit_control(proc, plan, reply, &mut buf)?;
                Ok(buf)
            },
        )?;
        payload.copy_from_slice(&out);
        Ok(ControlRoute::Forwarded)
    }

    /// ★ Retire proc `pid` out of band — the §7.3 worker-death consequence, and the
    /// staleness canaries' lever. **Spine op** (write guard). `true` if it was live.
    /// ★ **And it cancels** (§15 amendment 4): every verb this proc still has in flight
    /// is work whose requester is gone. Latched under the guard, discharged after it
    /// drops.
    pub fn retire_proc(&self, pid: ProcId) -> bool {
        let (out, cancels) = {
            let mut g = self.state.write();
            let st = &mut *g;
            let out = st
                .spine
                .retire_proc(&mut ExclusiveProcs(&mut st.procs), pid);
            (out, st.spine.take_pending_cancels())
        };
        cancels.discharge_all();
        out
    }

    /// Resolve `va` in the `(target, pdb)` VAS — read-only (route + `resolve_in`).
    /// Sharded: device read + proc lock (the proc lock is held for the µs lookup
    /// because the sharded procs sit in lock cells; the core's lock-free
    /// `&Gpu`-read shape returns in stage 3 if a measured path needs it).
    /// Degenerate: write guard, per the mode's every-op-writes rule.
    pub fn resolve(&self, target: GpuId, pdb: Pdb, va: GpuVa) -> Result<(Binding, u64), FwdFault> {
        match self.mode {
            LockMode::Sharded => {
                let st = self.state.read();
                let pid = kayfabe_fwd::route_pdb(&st.spine, target, pdb)?;
                let cell = st.proc_cell(pid).ok_or(FwdFault::RetiredProc(pid))?;
                let proc = cell.lock();
                kayfabe_fwd::resolve_in(&proc, target, pdb, va)
            }
            LockMode::Degenerate => {
                let mut g = self.state.write();
                let st = &mut *g;
                let pid = kayfabe_fwd::route_pdb(&st.spine, target, pdb)?;
                let proc = st.proc_mut(pid).ok_or(FwdFault::RetiredProc(pid))?;
                kayfabe_fwd::resolve_in(proc, target, pdb, va)
            }
        }
    }

    /// The #14 ring-gate as a read-only query (`gate_working_set_in`): every VA of
    /// `working_set` must resolve host-published in `cid`'s VAS. Same lock shape
    /// as [`SharedDevice::resolve`].
    pub fn gate_working_set(
        &self,
        pid: ProcId,
        cid: ChanId,
        working_set: &[GpuVa],
    ) -> Result<(), FwdFault> {
        match self.mode {
            LockMode::Sharded => {
                let st = self.state.read();
                let cell = st.proc_cell(pid).ok_or(FwdFault::RetiredProc(pid))?;
                let proc = cell.lock();
                kayfabe_fwd::gate_working_set_in(&proc, cid, working_set)
            }
            LockMode::Degenerate => {
                let mut g = self.state.write();
                let st = &mut *g;
                let proc = st.proc_mut(pid).ok_or(FwdFault::RetiredProc(pid))?;
                kayfabe_fwd::gate_working_set_in(proc, cid, working_set)
            }
        }
    }

    /// ★ One completion-source signal, dispatched and applied in ONE critical
    /// section (`SourceRegistry::dispatch` is a spine read; the Observe effect is
    /// a per-proc act — doing both under one hold closes the route→act staleness
    /// gap, so no R5 re-validation site exists here to forget).
    ///
    /// **Sharded:** device read (dispatch) → owning proc's lock (observe).
    /// **Degenerate:** one write guard.
    ///
    /// A [`SourceFault`] — unknown, stale, or retired-proc source — is returned
    /// typed and mutates nothing (MISS = FAULT; retire deregisters every source
    /// inside the same apply critical section, so a late signal resolves to
    /// nothing rather than to dead state).
    ///
    /// **Deliberately does NOT run the §5.2 pump edge:** posting a batch requires
    /// encoding it on the GSP queue and raising the IRQ (`Vmm::raise_irq`) — the
    /// shell's seam, stage 3. Pumping here without a deliverer would open a
    /// drain-gated batch nobody ever drains and wedge the delivery plane; the
    /// observation is complete in itself and the owner's own poll
    /// ([`SharedDevice::completion_poll`]) re-posts regardless (the F2 fix).
    pub fn signal_source(&self, source: CompletionSource) -> SignalOutcome {
        let outcome = self.dispatch_source(source);
        // ★ The §7.3 worker-death consequence, applied OUTSIDE the dispatch critical
        // section: retiring a proc is a spine WRITE, and taking the write lock while
        // the dispatch guard is alive would be an R3 panic (rank 0 twice) — and a
        // deadlock in any `RwLock` that does not upgrade. So the dispatch decides, the
        // guards drop, and the consequence runs from rank-clean state. The gap is an
        // R5 site like any other: both steps are idempotent and simply find nothing
        // if a guest teardown won the race.
        if let SignalOutcome::WorkerDied { proc, gpu, worker } = outcome {
            self.kill_worker_slot(proc, gpu, worker);
            // ★★ §7.5's pairing, applied to the HUP flavour of T5 — and it was MISSING.
            //
            // `kill_worker_slot` marks the slot dead, which makes it invisible to
            // `checked_out()`, so the `request_cancel_all` inside the retire below finds
            // NOTHING to cancel. A thread parked in that worker's verb therefore got no
            // signal at all: on a real socket the HUP itself ends its read, but nothing
            // in this design said so, and nothing here would have released it.
            //
            // The truth is exactly §7.5's: the reply is never coming, so the requester is
            // ABANDONED — and that is safe here for the one reason it is ever safe, which
            // is that the slot is already dead and the component is condemned by the
            // retire two lines down. The three are one act.
            self.abandon_worker(proc, gpu, worker);
            // ★ §12.26 — the system component is UNCONDEMNABLE. The slot still dies
            // (never a respawn), but the retire/condemn consequence is refused and the
            // outcome is re-typed to the device-scoped fault it actually is. Previously
            // this called `retire_proc(SYSTEM_PROC)`, which missed the `ProcSet` and
            // answered `false` into a discarded result: the loudest rule in the design,
            // silently absent for the one proc that cannot recover from it.
            if proc == Gpu::SYSTEM_PROC {
                return SignalOutcome::DeviceFatal { gpu, worker };
            }
            self.retire_proc(proc);
        }
        outcome
    }

    /// ★ Release the thread parked in a **dead** slot's verb (§7.5). Latched under the
    /// proc lock, discharged after it drops — firing it is a syscall (R1).
    ///
    /// Ordered AFTER [`SharedDevice::kill_worker_slot`] deliberately: the slot must be
    /// dead before its requester is released, or a returning worker could be checked in
    /// as idle and handed to a new op.
    fn abandon_worker(&self, pid: ProcId, gpu: GpuId, worker: WorkerId) {
        let mut latched: Option<kayfabe_isolate::CancelRequest> = None;
        let _ = self.route_act(
            |_| Ok((pid, ())),
            |_, proc, ()| {
                if let Some(iso) = proc.isolates.get_mut(&gpu) {
                    latched = iso.abandon(worker);
                }
            },
        );
        if let Some(req) = latched {
            req.discharge();
        }
    }

    /// Mark one pool slot permanently dead so it is never checked out again
    /// (§7.3) — **never a respawn**.
    fn kill_worker_slot(&self, pid: ProcId, gpu: GpuId, worker: WorkerId) {
        let _ = self.route_act(
            |_| Ok((pid, ())),
            |_, proc, ()| {
                if let Some(iso) = proc.isolates.get_mut(&gpu) {
                    iso.worker_died(worker);
                }
            },
        );
    }

    /// The dispatch critical section itself (see [`SharedDevice::signal_source`]).
    fn dispatch_source(&self, source: CompletionSource) -> SignalOutcome {
        match self.mode {
            LockMode::Sharded => {
                let st = self.state.read();
                let dispatch = match st.spine.sources.dispatch(source) {
                    Ok(d) => d,
                    Err(f) => return SignalOutcome::Fault(f),
                };
                match dispatch {
                    Dispatch::Observe { proc, gpu, ev } => {
                        let cell = st.proc_cell(proc).expect(
                            "registry routes only to live procs: deregister_proc runs \
                             inside the retiring critical section",
                        );
                        let mut p = cell.lock();
                        Self::observe_into(&mut p, proc, gpu, ev)
                    }
                    Dispatch::WorkerDied { proc, gpu, worker } => {
                        SignalOutcome::WorkerDied { proc, gpu, worker }
                    }
                    Dispatch::CrossSignal { from, to } => SignalOutcome::CrossSignal { from, to },
                    Dispatch::Wake => SignalOutcome::Wake,
                }
            }
            LockMode::Degenerate => {
                let mut g = self.state.write();
                let st = &mut *g;
                let dispatch = match st.spine.sources.dispatch(source) {
                    Ok(d) => d,
                    Err(f) => return SignalOutcome::Fault(f),
                };
                match dispatch {
                    Dispatch::Observe { proc, gpu, ev } => {
                        let p = st.proc_mut(proc).expect(
                            "registry routes only to live procs: deregister_proc runs \
                             inside the retiring critical section",
                        );
                        Self::observe_into(p, proc, gpu, ev)
                    }
                    Dispatch::WorkerDied { proc, gpu, worker } => {
                        SignalOutcome::WorkerDied { proc, gpu, worker }
                    }
                    Dispatch::CrossSignal { from, to } => SignalOutcome::CrossSignal { from, to },
                    Dispatch::Wake => SignalOutcome::Wake,
                }
            }
        }
    }

    /// The Observe act phase, shared between modes: land `ev` in `proc`'s OWN
    /// queue (per-proc by construction — the F2 shape has nowhere to exist).
    fn observe_into(p: &mut Proc, proc: ProcId, gpu: GpuId, ev: OsEventRef) -> SignalOutcome {
        match p.completion.observe(ev) {
            Ok(()) => SignalOutcome::Observed { proc, gpu, ev },
            Err(err) => SignalOutcome::ObserveRefused { proc, ev, err },
        }
    }
}

/// ★★★★ **§16.27 — one object in a client namespace**, as reported by
/// [`SharedDevice::namespace_census`]. ⊘ Report-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamespaceRow {
    /// What kind of RM object it is — the field that decides the fork: is there a
    /// `VaSpace` in this namespace at all?
    pub kind: kayfabe_arch::ObjectKind,
    /// Its origin `hObject`.
    pub handle: u32,
    /// Its declared parent handle — so a `VaSpace` can be attributed to the same Device
    /// the walling channel is parented on, rather than merely to the same client.
    pub parent: u32,
    /// Its PDB, if it has one. ★ A `VaSpace` with `pdb: None` is a *different* answer from
    /// no `VaSpace` at all: it exists but has never been bound by `SET_PAGE_DIRECTORY`, so
    /// the fourth route would resolve it and still address nothing.
    pub pdb: Option<u64>,
}

impl NamespaceRow {
    /// Is this the kind the §16.27 fork turns on?
    ///
    /// ⊘ A method rather than letting the reporter match on [`Self::kind`] itself: the
    /// reporting crate (`kayfabe-qemu-raw`) does not depend on `kayfabe-arch`, and the
    /// alternative — comparing `format!("{:?}", kind)` against the string `"VaSpace"` —
    /// would make a *rename* silently turn every namespace into `NO-VASPACE-IN-NAMESPACE`,
    /// i.e. silently answer the open question with the wrong fork.
    #[must_use]
    pub fn is_vaspace(&self) -> bool {
        self.kind == kayfabe_arch::ObjectKind::VaSpace
    }
}

/// ★★★★ **One live channel's addressing** — re-exported from `kayfabe_core` rather than
/// declared here.
///
/// ⊘ It used to be a second struct with the same eight fields, formatted by a second
/// implementation in `kayfabe_qemu_raw`. Two computations that agree today are not
/// corroboration; they are a drift waiting for somebody to read a formatting difference as
/// a fact about the guest (`measure_at_the_boundary_not_inside`). The **sources** stay
/// separate — this one walks the sharded shell under ranked locks, which a whole-`Gpu`
/// walk may not — but the row and the format are `kayfabe_core`'s, once.
pub use kayfabe_core::gpu::VasCensusRow as ChannelVasRow;

/// ★★★ **What a doorbell's channel DECLARED about its own addressing** — the three facts a
/// published-VA-space walk needs, plus the routing identities that named them.
///
/// ⊘ Every field is a *declared* fact read off the channel's own `RM_ALLOC`; nothing here is
/// resolved, validated or defaulted. A resolver that wanted a fourth fact should read it
/// from the same node rather than infer it from these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeChannelFacts {
    /// The owning proc the token routed to.
    pub proc: ProcId,
    /// The channel it routed to.
    pub chan: ChanId,
    /// The decoded vChid.
    pub vchid: VChid,
    /// ★★★★ **§16.25 — which of the three declared-fact routes resolved (or failed to
    /// resolve) this channel's VA space, and what each route that ran actually hit.**
    ///
    /// [`Self::vaspace`] and [`Self::vaspace_declared`] are two projections of *the answer*;
    /// this is the projection of *the search*. `vas=NONE-DECLARED dec=NONE` says the channel
    /// declared no `hVASpace` and none resolved — but three different searches produce that
    /// same pair, and until this field existed a refusal could not say which one ran.
    ///
    /// ⊘ Report-only, and carried rather than re-derived — see the assignment site.
    pub vas_route: kayfabe_core::project::VasRoutes,
    /// `hClient` — **the namespace the channel's RESOLVED VA space lives in**, which is
    /// the namespace its publication was issued in (`kayfabe_device::gvaspub`'s key is the
    /// control RPC's `hClient`/`hObject` pair). Falls back to the channel's own namespace
    /// when no VA space resolved, so a report always names somebody.
    ///
    /// ⊘ It used to be *"the namespace the channel … live[s] in"*, unconditionally. That
    /// is the same namespace only while the channel declares its own `hVASpace`; a channel
    /// that inherits one through a CtxShare or TSG owned by a different client would have
    /// been looked up under the wrong `hClient` — a publication miss dressed as a channel
    /// with no published root.
    pub client: u32,
    /// ★★★ **The RESOLVED `hVASpace`** — the origin handle of the VASpace resource
    /// `project::resolve_channel_vas` bound this channel to, through the declared
    /// precedence (own `hVASpace` → CtxShare's → parent TSG's). `None` = no declared path
    /// resolved, i.e. genuinely GSP-managed with no VA space.
    ///
    /// # ⊘⊘ It used to be `node.facts.h_vaspace`, and THAT LOST THE UVM CHANNEL
    ///
    /// `[measured 2026-08-09, boot `msr2_319d29a`, binary stamped
    /// `kayfabe-rev:319d29a3…`]`. The channel `cuInit` walls on printed
    ///
    /// ```text
    /// doorbells: 5 arrived, 4 served, 1 REFUSED by name; last token 0x00010003
    ///   first doorbell refusal [FwdFault::IsolateRetired] … | c=0xc1d0000a
    ///       vas=NONE-DECLARED ring=0x121010000
    /// ```
    ///
    /// A UVM channel declares **no `hVASpace` of its own** — it inherits it — so this field
    /// was `None`, `SharedDoorbell::try_ce_submission` returned `None` at `facts.vaspace?`
    /// **before reading a byte of the ring**, and the doorbell fell through to a forwarding
    /// path that reads no ring at all. Meanwhile [`CeChannelFacts::vas_pdb`], derived from
    /// the **resolved** node, was `Some` — the core knew this channel's address space by a
    /// route this field did not take.
    ///
    /// ⇒ Two projections of one fact, disagreeing, with the weaker one on the load-bearing
    /// path. It is now the resolved one, so the two cannot disagree: both come from
    /// `Channel::vas_origin`.
    pub vaspace: Option<u32>,
    /// ★★★★ **§16.16 — the handle the CHANNEL ITSELF DECLARED**, verbatim off its alloc
    /// params (`AllocFacts::h_vaspace`), printed **beside** the resolved
    /// [`CeChannelFacts::vaspace`] and never instead of it.
    ///
    /// # ⊘ Why both, when the doc above explains that the declared one LOST the UVM channel
    ///
    /// Both statements are true and they are about different jobs. On the **load-bearing**
    /// path the resolved handle is the right one, for exactly the reason that doc gives: a
    /// UVM channel declares no `hVASpace` of its own and inheriting it is what makes the
    /// ring readable at all. ⊘ Nothing here changes that — this field feeds **no**
    /// decision, only the report.
    ///
    /// ★ Its job is the **audit**. `vaspace` is *derived* — inherited through CtxShare or
    /// the parent TSG by `project::resolve_channel_vas` — and a derivation cannot be
    /// checked by printing its own output. `[measured 2026-08-09, boot `msr2_319d29a`]`
    /// this very attribution was already wrong once on this very channel, in the other
    /// direction. So the two projections are printed **side by side** and a reader compares
    /// them, which is the property that has kept this campaign honest; and the interesting
    /// case is not disagreement but `dec=NONE-DECLARED` with `vas=Some`, which says in one
    /// glance that **every byte of the walk rests on an inheritance we chose**, not on
    /// anything the guest wrote down.
    ///
    /// ⊘ `None` (or a declared handle of zero) is carried as `None`, never folded — same
    /// argument as [`CeChannelFacts::vaspace`]'s.
    pub vaspace_declared: Option<u32>,
    /// ★★★★ **§16.28 — route 4's answer on its own**: the `hVASpace` naming the parent
    /// **Device's** default address space, or `None` if this channel did not take that
    /// route. When it is `Some`, [`Self::vaspace`] equals it and [`Self::vas_pdb`] is
    /// `None` — see [`kayfabe_core::project::ChannelFacts::vas_device_default`] for why
    /// that combination is the correct shape rather than a half-resolved one.
    pub vaspace_device_default: Option<u32>,
    /// `gpFifoOffset` — a **GPU VIRTUAL** address (`ogkm-580: ctrl2080fifo.h:809`).
    /// `None` = the channel's alloc params declared no ring at all, which is different
    /// from `Some(0)` (a ring the driver deliberately declares at zero for its golden-context
    /// channel, `ogkm-580: kernel_graphics.c:2420-2424`).
    pub ring_va: Option<u64>,
    /// `gpFifoEntries` that came with [`Self::ring_va`], or `0`.
    pub ring_entries: u32,
    /// ★★★★ **§16.16 — the channel's declared USERD**, `hUserdMemory[0]` and
    /// `userdOffset[0]`, verbatim off the same alloc params [`Self::ring_va`] comes from.
    ///
    /// ⊘ `None` = *this port could not read the field* (an unpinned driver boundary, or
    /// params that stopped short), **never** *"the channel declared none"* — a declared
    /// handle of zero arrives as `Some` with `handle == 0`. See
    /// [`kayfabe_core::rmgraph::DeclaredUserd`] for why zero is a declaration, and
    /// `kayfabe_abi::notifier::ChannelUserdWire` for why USERD is the canary the ring
    /// cannot be: it comes from the same params but has **recognisable** content, so its
    /// null discriminates where the ring's does not.
    pub userd: Option<kayfabe_core::rmgraph::DeclaredUserd>,
    /// The channel's bound page-directory base, if the port has one. `None` is the
    /// `FwdFault::NoVas` state.
    pub vas_pdb: Option<Pdb>,
    /// ★★★ **§14.18 — the engine the guest bound this channel to** (`NVA06F_CTRL_CMD_BIND`,
    /// `0xa06f0104`), in **`RM_ENGINE_TYPE`** space, or `None` if it never sent one.
    ///
    /// ⊘ The one fact §14.18 named as owed: *"the doorbell path must be able to name the
    /// engine the ringing channel was bound to — `Gpu::bind_channel` records it, the
    /// doorbell path does not read it"*. A completion cannot be announced without it,
    /// because the vector that announces it is a property of the engine.
    ///
    /// ⚠ Read from `kayfabe_core::gpu::ExecPlane::bound`, which stores **RM** engine space
    /// on purpose; it is not the raw `engineType` off the wire, and the two collide above
    /// `0x12`. ⊘ `None` is carried as `None` and never folded to an engine id: a channel
    /// with no bind and a channel bound to engine zero would otherwise be the same fact,
    /// and only the first means *"this device may not announce a completion here"*.
    ///
    /// ⚠ It is a *declared* fact like every other field here — the guest said it, we
    /// recorded it. It is not a claim that any engine ran.
    pub bound_engine: Option<u32>,
    /// ★★★★ **§16.65 — the channel's own [`EngineKind`]**, carried off the same resolved
    /// [`kayfabe_core::gpu::Channel`] that produced [`Self::vas_pdb`], so the doorbell path
    /// can finally name the engine a ringing channel belongs to.
    ///
    /// # ⊘ THE ROUTING KEY IS THIS FIELD AND NOT [`Self::bound_engine`]
    ///
    /// The two look interchangeable and are not, and picking the wrong one is a silent
    /// misroute rather than a build error. `bound_engine` is a **sparse projection of one
    /// wire message**: `NVA06F_CTRL_CMD_BIND` (`0xa06f0104`) occurs `[measured 2026-08-10,
    /// boot s48 census]` **2× per boot, both with `result 0xffffffff`** — nothing answered
    /// — against **14 live channels**. Routing on it would leave twelve channels with
    /// `None` and route them by a default, which is the shape this campaign names *"a
    /// fallback keyed on our own ignorance"*.
    ///
    /// ★ This field, by contrast, is **total**: it is the channel class's declared kind
    /// (`kayfabe_core::project`), refined by the engine object the guest allocated on the
    /// channel. Every channel has one because every channel was allocated with a class.
    ///
    /// ⊘ Both are still carried, side by side and never reconciled — the §16.16 rule. A
    /// boot in which `engine == Ce` and `bound_engine == None` is the *normal* shape here,
    /// and reading it as a disagreement would be reading a sparse instrument as a census.
    pub engine: EngineKind,
}

/// ★★★★ **§16.65 — WHICH EXECUTOR OWNS A RUNG DOORBELL**, decided from the channel's own
/// [`EngineKind`] and from nothing else.
///
/// # ⊘ Why this exists as a value rather than an `if` at the one call site
///
/// `[measured 2026-08-10, boots s49/s50]` the shim's CPU copy-engine executor claimed
/// **every** doorbell that had a VA space and a ring, `Ce` and `GrCompute` alike, because
/// the only gate in front of it asked about the *isolate plane* and not about the *engine*.
/// The visible symptom was `FwdFault::SubmissionHasNoLaunch { methods: 3, opaque: 2 }` —
/// a **GR** pushbuffer being decoded by the **CE** codec, correctly declining to find a CE
/// launch in it. ⊘ Nothing was forged (the codec is class-gated, so a GR ring decodes to
/// `Opaque`), but those doorbells reached the wrong executor and could never reach a right
/// one, and the refusal they got named the *pushbuffer's* shape rather than the *routing*
/// mistake that put them there.
///
/// ★ So the statement is *"this channel's work belongs to executor X"*, made once, keyed on
/// the one total field ([`CeChannelFacts::engine`]) — never on the sparse
/// [`CeChannelFacts::bound_engine`], whose own doc carries the measurement that rules it
/// out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorbellRoute {
    /// The shell's own CPU copy-engine executor (`kayfabe_rt::ceutils`). The copy IS the
    /// workload and its operands are in memory this process holds, so it can run here.
    CpuCe,
    /// ★ A GR context — compute or graphics. **Nothing serves this yet**, and it is a
    /// distinct variant rather than folded into [`DoorbellRoute::Unserved`] because the two
    /// are different states of knowledge: GR is the *destination the ladder is walking
    /// toward* (`ce_executor_tree.md`; it still needs a host channel that SHADOWS the
    /// guest's and the `OS_DESCRIPTOR` primitive), while `Unserved` is an engine nobody has
    /// designed a path for. Collapsing them would make the census unable to say how much
    /// traffic is waiting on work that is planned.
    HostGr,
    /// NVENC, NVDEC, or an engine the core routes but does not interpret. No executor.
    Unserved,
}

/// How many buckets a per-engine doorbell census has — [`EngineKind::ALL`]'s length.
pub const ENGINE_KIND_COUNT: usize = EngineKind::ALL.len();

/// The census's bucket labels, in [`EngineKind::ALL`] order.
///
/// ⊘ Surfaced here so the shim can *label* its histogram without taking a normal dependency
/// on `kayfabe-arch` — see [`CeChannelFacts::engine_index`].
#[must_use]
pub fn engine_kind_names() -> [&'static str; ENGINE_KIND_COUNT] {
    EngineKind::ALL.map(EngineKind::name)
}

/// ★★★★ **§16.65 — THE ROUTING STATEMENT ITSELF**, over an [`EngineKind`] and nothing else.
///
/// # ⊘ Why it is a free function and not only a method on [`CeChannelFacts`]
///
/// The decision depends on exactly one field, and `CeChannelFacts` is a twenty-field
/// structure that can only be built by resolving a live channel out of a realized device.
/// A decision reachable only through its own preconditions is a decision **nothing tests**
/// — `[audited 2026-08-10]` `SharedDoorbell::try_ce_submission`, which is where this
/// decision is acted on, had **no test coverage of any kind**. Splitting the pure half out
/// is the same move `shim_logic.rs` makes for `isolate_plane_from`, and for the same
/// reason: the half that can be quantified over should be.
///
/// ⊘ The `match` is exhaustive with no `_` arm, so a new [`EngineKind`] variant fails this
/// build until somebody says which executor owns it. A default arm would hand it to
/// whichever executor answered first, which is the §16.65 defect restated.
#[must_use]
pub fn route_of_engine(engine: EngineKind) -> DoorbellRoute {
    match engine {
        EngineKind::Ce => DoorbellRoute::CpuCe,
        EngineKind::GrCompute | EngineKind::GrGraphics => DoorbellRoute::HostGr,
        EngineKind::NvEnc | EngineKind::NvDec | EngineKind::Other => DoorbellRoute::Unserved,
    }
}

impl CeChannelFacts {
    /// The executor this channel's doorbells belong to — [`route_of_engine`] of
    /// [`CeChannelFacts::engine`]. ⊘ Delegates rather than re-deciding: two copies of one
    /// rule is how the §16.64 probe came to contradict the serving path beside it.
    #[must_use]
    pub fn route(&self) -> DoorbellRoute {
        route_of_engine(self.engine)
    }

    /// This channel's histogram bucket — [`EngineKind::index`], surfaced so the shim can
    /// tally a per-engine doorbell census **without naming an architecture crate**
    /// (`crates/kayfabe-qemu-raw/Cargo.toml`: `kayfabe-arch` is deliberately not a normal
    /// dependency of the shim, and this rung does not make it one).
    #[must_use]
    pub fn engine_index(&self) -> usize {
        self.engine.index()
    }

    /// This channel's engine name, for the same reason and under the same constraint as
    /// [`CeChannelFacts::engine_index`].
    #[must_use]
    pub fn engine_name(&self) -> &'static str {
        self.engine.name()
    }
}
