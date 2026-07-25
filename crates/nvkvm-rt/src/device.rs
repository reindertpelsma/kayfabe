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
//!   route/act split (R4) the core's `nvkvm-fwd` signatures already factor.
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

use nvkvm_arch::ids::{GpuId, GpuVa, Pdb};
use nvkvm_completion::{CompletionError, OsEventRef, PostBatch};
use nvkvm_core::gpu::{Gpu, GpuError, Proc, ProcSet, Spine};
use nvkvm_core::reactor::{CompletionSource, Dispatch, SourceFault, SourceKind};
use nvkvm_core::rmgraph::RmEvent;
use nvkvm_core::{ChanId, ProcId};
use nvkvm_fwd::{DoorbellOutcome, FwdFault, Published};
use nvkvm_isolate::WorkerId;
use nvkvm_mmu::Binding;
use nvkvm_util::Instant;

use crate::lock::{LockRank, RankedMutex, RankedRwLock};

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
    /// A worker of `proc`'s `gpu` isolate died. Stage 2 surfaces the typed fact;
    /// the §5.4 teardown consequence (interrupt in-flight, retire) is stage 3's —
    /// never a silent respawn.
    WorkerDied {
        /// The owning proc.
        proc: ProcId,
        /// The isolate's GPU target.
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

    /// Consume the device back into a plain [`Gpu`] (tests: the lock-mode
    /// differential snapshots the reassembled core). Ownership-based — no
    /// acquisition.
    #[must_use]
    pub fn into_gpu(self) -> Gpu {
        let DeviceState {
            spine,
            system,
            procs,
        } = self.state.into_inner();
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
    pub fn reap_retired(&self) -> usize {
        self.state.write().spine.reap_retired()
    }

    /// Register a completion source (spine mutation — write guard). The
    /// [`nvkvm_core::reactor::WakeRequest`] is discharged into the registry's
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

    /// ★ THE one gated ring path (inherited law 7), route/act split per R4.
    ///
    /// **Sharded:** ROUTE under the device *read* guard (`route_doorbell` — pure
    /// spine read), then ACT under the owning proc's rank-1 lock
    /// (`exec_doorbell`: ring-gate → lazy materialize/schedule → ring).
    /// **Degenerate:** the same two phases under one device write guard.
    pub fn doorbell(
        &self,
        target_gpu: GpuId,
        token: u64,
        working_set: &[GpuVa],
    ) -> Result<DoorbellOutcome, FwdFault> {
        match self.mode {
            LockMode::Sharded => {
                let st = self.state.read();
                let route = nvkvm_fwd::route_doorbell(&st.spine, target_gpu, token)?;
                let cell = st
                    .proc_cell(route.proc)
                    .ok_or(FwdFault::RetiredProc(route.proc))?;
                let mut proc = cell.lock();
                nvkvm_fwd::exec_doorbell(&mut proc, &route, working_set)
            }
            LockMode::Degenerate => {
                let mut g = self.state.write();
                let st = &mut *g;
                let route = nvkvm_fwd::route_doorbell(&st.spine, target_gpu, token)?;
                let proc = st
                    .proc_mut(route.proc)
                    .ok_or(FwdFault::RetiredProc(route.proc))?;
                nvkvm_fwd::exec_doorbell(proc, &route, working_set)
            }
        }
    }

    /// Back `[va, va+len)` in the `(gpu, pdb)` VAS: route via `by_pdb` (device
    /// read), act via `publish_backing` under the owning proc's lock (carve arena,
    /// host alloc+map, forward-populate). Degenerate: both phases under the write
    /// guard.
    pub fn publish_backing(
        &self,
        gpu: GpuId,
        pdb: Pdb,
        va: GpuVa,
        len: u64,
    ) -> Result<Published, FwdFault> {
        match self.mode {
            LockMode::Sharded => {
                let st = self.state.read();
                let pid = nvkvm_fwd::route_pdb(&st.spine, gpu, pdb)?;
                let cell = st.proc_cell(pid).ok_or(FwdFault::RetiredProc(pid))?;
                let mut proc = cell.lock();
                nvkvm_fwd::publish_backing(&mut proc, gpu, pdb, va, len)
            }
            LockMode::Degenerate => {
                let mut g = self.state.write();
                let st = &mut *g;
                let pid = nvkvm_fwd::route_pdb(&st.spine, gpu, pdb)?;
                let proc = st.proc_mut(pid).ok_or(FwdFault::RetiredProc(pid))?;
                nvkvm_fwd::publish_backing(proc, gpu, pdb, va, len)
            }
        }
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
                let pid = nvkvm_fwd::route_pdb(&st.spine, target, pdb)?;
                let cell = st.proc_cell(pid).ok_or(FwdFault::RetiredProc(pid))?;
                let proc = cell.lock();
                nvkvm_fwd::resolve_in(&proc, target, pdb, va)
            }
            LockMode::Degenerate => {
                let mut g = self.state.write();
                let st = &mut *g;
                let pid = nvkvm_fwd::route_pdb(&st.spine, target, pdb)?;
                let proc = st.proc_mut(pid).ok_or(FwdFault::RetiredProc(pid))?;
                nvkvm_fwd::resolve_in(proc, target, pdb, va)
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
                nvkvm_fwd::gate_working_set_in(&proc, cid, working_set)
            }
            LockMode::Degenerate => {
                let mut g = self.state.write();
                let st = &mut *g;
                let proc = st.proc_mut(pid).ok_or(FwdFault::RetiredProc(pid))?;
                nvkvm_fwd::gate_working_set_in(proc, cid, working_set)
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
