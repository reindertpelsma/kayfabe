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
use std::sync::{Condvar, Mutex};

use kayfabe_arch::ids::{ClassId, ControlCmd, GpuId, GpuVa, Pdb, VChid};
use kayfabe_completion::{CompletionError, OsEventRef, PostBatch};
use kayfabe_core::gpu::{Gpu, GpuError, Proc, ProcSet, Spine};
use kayfabe_core::reactor::{CompletionSource, Dispatch, SourceFault, SourceKind};
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_core::{ChanId, ProcId};
use kayfabe_fwd::{
    ControlRoute, DoorbellOutcome, EngineObjectForwarded, FwdFault, Planned, Published, Refusal,
};
use kayfabe_isolate::{HostHandle, RmError, VerbPlan, VerbReply, Worker, WorkerId};
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
    /// (`ogkm: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:2779-2789`) — **device** scope,
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
}

impl<P> Staged<P> {
    /// Finish a plan phase by checking a worker out of `proc`'s `gpu` isolate — the
    /// last thing that happens under the lock, per §7.3.
    fn check_out(proc: &mut Proc, gpu: GpuId, planned: Planned<P>) -> Result<Self, FwdFault> {
        let worker = match planned.verbs {
            Some(_) => kayfabe_fwd::checkout(proc, gpu)?,
            None => None,
        };
        Ok(Staged {
            proc: proc.id,
            gpu,
            plan: planned.plan,
            verbs: planned.verbs,
            worker,
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
        SharedDevice {
            mode,
            pool: PoolGate::default(),
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
    pub fn apply(&self, ev: RmEvent) -> Result<(), GpuError> {
        let mut g = self.state.write();
        let st = &mut *g;
        st.spine
            .apply(st.system.get_mut(), &mut ExclusiveProcs(&mut st.procs), ev)
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
            let staged = stage()?;
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
            let executed = worker.execute(&verbs);
            let gpu = staged.gpu;
            let Ok(reply) = executed else {
                let failure = executed.expect_err("matched Err");
                let err = failure.err;
                // ★ G4 (§12.16): dispose of the failure's orphans on the SAME worker,
                // still lock-free, BEFORE returning it. The residue is a named value —
                // its core-side ledger is L1-M2's (see `kayfabe_fwd::dispose_on`).
                let _undisposed = kayfabe_fwd::dispose_on(&mut worker, failure.orphans);
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
                    RmError::Interrupted => FwdFault::Cancelled { proc: staged.proc },
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

    /// ★ THE one gated ring path (inherited law 7), route/act split per R4 and
    /// plan/execute/commit per R1.
    ///
    /// **Plan** (device read → proc lock): `route_doorbell` + `plan_doorbell` — the
    /// #14 ring-gate runs HERE, against the same locked snapshot the plan is derived
    /// from, before any host op exists. **Execute** (no locks): materialize /
    /// schedule / ring on a checked-out worker. **Commit** (re-locked): re-resolve
    /// the route and the channel, adopt the host handles, record the submission.
    pub fn doorbell(
        &self,
        target_gpu: GpuId,
        token: u64,
        working_set: &[GpuVa],
    ) -> Result<DoorbellOutcome, FwdFault> {
        self.verb_op(
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
        )
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
    pub fn retire_proc(&self, pid: ProcId) -> bool {
        let mut g = self.state.write();
        let st = &mut *g;
        st.spine
            .retire_proc(&mut ExclusiveProcs(&mut st.procs), pid)
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
