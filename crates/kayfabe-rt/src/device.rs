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
//!   RankedRwLock<DeviceState>                       // rank 1 — the device lock
//!     DeviceState {
//!         spine:  Spine,                            // device-global, guarded by rank 1
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
//!   device *write* guard, procs via [`ExclusiveProcs`]. One lock, rank 1.
//! - **per-proc op** (doorbell, publish, resolve, gate, source dispatch) — device
//!   *read* guard (rank 1) → look up that proc's `RankedMutex` → `lock()` it
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

use kayfabe_arch::ids::{
    ClassId, ControlCmd, EngineKind, GpuId, GpuVa, HClient, HObject, Pdb, VChid,
};
use kayfabe_completion::{CompletionError, OsEventRef, PostBatch};
use kayfabe_core::gpu::{Gpu, GpuError, PendingSpawn, PendingSpawns, Proc, ProcSet, Spine};
use kayfabe_core::reactor::{CompletionSource, Dispatch, SourceFault, SourceKind};
use kayfabe_core::rmgraph::RmEvent;
use kayfabe_core::{ChanId, ProcId};
use kayfabe_fwd::{
    ControlRoute, DoorbellOutcome, EngineObjectForwarded, FB_LEAF_GRANULE, FwdFault, Orphans,
    Planned, Published, Refusal,
};
use kayfabe_isolate::{
    HostHandle, IsolateBox, IsolateFactory, IsolateId, RmError, VerbPlan, VerbReply, Worker,
    WorkerId,
};
use kayfabe_mmu::Binding;
use kayfabe_util::Instant;

use crate::lock::{BlockingSection, LockRank, RankedMutex, RankedRwLock};

/// `Y`/`N` for a report. ⊘ Two characters and not `true`/`false`: a census line packs a
/// dozen predicates and a reader scanning a column wants them the same width.
fn yn(b: bool) -> &'static str {
    if b { "Y" } else { "N" }
}

/// Which lock configuration a [`SharedDevice`] runs (§8.2 / P5: BOTH are tested
/// from day one; the mode must never be observable through the API).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    /// Every op takes the device **write** lock; procs are reached via `get_mut`.
    /// Bit-for-bit the stress-proven single-lock shape — L1-M1's shipping
    /// configuration (§3.4 staging).
    Degenerate,
    /// Spine ops write-lock the device; per-proc ops take device-read (rank 1)
    /// then that proc's mutex (rank 1). The #14-gate configuration.
    Sharded,
}

/// The rank-1-guarded state: the spine plus the per-proc lock cells.
struct DeviceState {
    /// Device-global spine (graph, routing maps, targets, delivery, sources).
    spine: Spine,
    /// The system proc's rank-1 cell (kernel RM / scrubber / CeUtils traffic).
    system: RankedMutex<Proc>,
    /// One rank-1 cell per derived user proc.
    procs: BTreeMap<ProcId, RankedMutex<Proc>>,
    /// ★★★★★ **§16.96 — the engine-object forward latch.** See
    /// [`SharedDevice::forward_engine_object_deferring`].
    ///
    /// ⊘ It lives HERE, beside the spine, rather than travelling through
    /// `CommandPolicy`: the frame that can safely issue the verb (`Regs::write`) already
    /// holds an `Arc<SharedDevice>`, so nothing has to cross the `kayfabe-gsp` port
    /// (§16.90's blocker, dissolved the same way §16.91 dissolved it for spawns).
    ///
    /// ⚠ **Bounded** by [`MAX_PENDING_ENGINE_FORWARDS`], and the bound REFUSES BY NAME.
    pending_engine_forwards: Vec<PendingEngineForward>,
}

/// ★★★★★ **§16.96** — one Case-1 engine-object alloc the plane decided under its **rank-0**
/// mutex and therefore could not forward there.
///
/// Plain data, by value: a latch outlives the guard that made it, so it may not borrow.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingEngineForward {
    /// The alloc's `hClient`.
    client: HClient,
    /// The alloc's `hParent` — the channel the object is created on.
    parent: HObject,
    /// The engine-object class.
    class: ClassId,
    /// The guest's own declared params window, already bounded by the decode.
    params: Vec<u8>,
}

/// ★★ **§16.96 — the latch's bound**, and it exists because the population being 1 is *"a
/// property of the guest, not of the protocol"* (`kayfabe-gsp/src/boot.rs:1291-1294`).
///
/// The guest is synchronous under the GPU lock, so today exactly one command is in flight
/// per register write and the latch holds at most one entry. ⊘ That is an observation about
/// **this** guest. `cap1b` reaches a real queue-full at txn 1028, which is the same shape of
/// assumption failing on the status ring. ⇒ the latch **refuses by name** at the bound
/// ([`ForwardAdmission::LatchFull`]) instead of growing without limit on a guest that
/// batches.
///
/// The number is generous rather than tight on purpose: a bound that fires in normal
/// operation would be a second failure mode, and a bound that never fires is still the
/// difference between a named refusal and an unbounded `Vec` fed by guest traffic.
pub const MAX_PENDING_ENGINE_FORWARDS: usize = 64;

/// ★★★★★ **§16.96 — what [`SharedDevice::forward_engine_object_deferring`] did with a
/// request.** ⊘ Three outcomes and not a `Result<(), _>`, because ADMITTED and SERVED are
/// different gates and this call can only ever report the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardAdmission {
    /// ★ **ADMITTED, NOT SERVED.** The request is latched; the host verb has **not** run
    /// and its outcome is not known here. [`SharedDevice::run_pending_engine_forwards`]
    /// runs it, lock-free, and reports.
    Latched {
        /// How many requests are now latched, this one included.
        pending: usize,
    },
    /// The class gate refused: the arch does not know this class as an engine object.
    /// **Nothing was latched and nothing will run** — the same silent, cheap exit
    /// `kayfabe_fwd::route_engine_object_by_parent` takes first, kept first here so the
    /// overwhelming majority of allocs (clients, devices, memory, VA spaces) never touch
    /// the latch at all.
    NotAnEngine(ClassId),
    /// ★★ The latch is **full** and this request was **refused by name** rather than
    /// growing it. See [`MAX_PENDING_ENGINE_FORWARDS`].
    LatchFull {
        /// How many requests were already latched.
        pending: usize,
        /// The bound that refused.
        bound: usize,
    },
}

/// ★★★★★ **w288 — the VMM's answer to *"where are THIS latched forward's channel's error
/// notifier pages?"***, keyed by the same triple the latch carries.
///
/// ⊘ It is a **grant** and not a guest-physical address, because a GPA is a number this
/// crate could be tempted to re-derive a file offset from, and re-deriving it is the `-m 8G`
/// bug `kayfabe_vmm_qemu::layout` exists to refuse. The VMM resolved it against its own
/// stated layout, refused by name if it could not, and what crosses is the result.
///
/// ⊘ The key is the alloc's identity, not the channel's: at the moment the caller builds
/// this, the host channel does not exist yet — that is the whole point of building it before
/// the drain rather than after.
#[derive(Debug, Clone, Copy)]
pub struct EngineNotifierGrant {
    /// The latched alloc's owning `hClient`.
    pub client: HClient,
    /// The latched alloc's `hParent` — the guest channel the object is allocated on.
    pub parent: HObject,
    /// The latched alloc's class.
    pub class: ClassId,
    /// The guest's own notifier pages, as the VMM derived them.
    pub grant: kayfabe_isolate::GuestRamGrant,
}

/// ★★★ **§16.96 — one latched forward, RUN.** What
/// [`SharedDevice::run_pending_engine_forwards`] reports per request.
///
/// ⊘ The request's own identifying fields travel back with the outcome because the caller
/// that reports them (`Regs::write`, six crates out) never saw the request: it did not
/// decide the forward, it only owns the frame that may issue it.
#[derive(Debug, Clone)]
pub struct EngineForwardRun {
    /// The alloc's `hClient`.
    pub client: HClient,
    /// The alloc's `hParent`.
    pub parent: HObject,
    /// The engine-object class.
    pub class: ClassId,
    /// How many bytes of params the guest declared (the bytes themselves are consumed).
    pub params_len: usize,
    /// What the host verb did.
    pub out: Result<EngineObjectForwarded, FwdFault>,
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
/// [`SharedDevice::forward_ring`]'s phase-1 result: a plan to fetch, or a named absence.
///
/// ⊘ Boxed: a `RingPlan` carries the ring's translated runs and this crosses a closure
/// boundary on the doorbell's hot path.
#[derive(Debug)]
enum RingPlanned {
    /// The ring resolved; phase 2 fetches its bytes with **no** lock held.
    Plan(Box<kayfabe_fwd::RingPlan>),
    /// One of the six named absences — nothing to fetch.
    NoRing(kayfabe_fwd::RingLook),
}

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
    /// ★★★★★ **Route B's switch, and it is a PRESENCE, not a boolean** — the framebuffer
    /// source this device answers vidmem ring reads from.
    ///
    /// ⊘ Unset by default and unset in every existing constructor, so a device that nobody
    /// registers a source with refuses vidmem ranges **exactly** as it did before route B
    /// existed. There is no flag to get wrong: `kayfabe_fwd::read_gpfifo_ring` derives its
    /// route from whether it was handed a reader.
    ///
    /// ⚠ **Read outside the ranked locks only** (§16.87): the production source takes the
    /// plane's rank-0 mutex, which may not be acquired beneath ranks 1-2.
    fb: std::sync::OnceLock<Arc<dyn kayfabe_fwd::FbSource>>,
    /// `[w281]` The PUSHBUFFER's vidmem route — [`SharedDevice::set_pushbuffer_vidmem`].
    /// ⊘ Separate from `fb` on purpose: supply and route are different questions.
    pb_vidmem: std::sync::atomic::AtomicBool,
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

/// ★★★★★ **w317 — what one [`SharedDevice::drain_retired_budgeted`] call did.**
///
/// ⊘ **Counted, never timed**, for [`PoolWaits`]'s reason exactly: there is no clock in this
/// crate. The *caller* owns the wall clock (it hands the deadline in), so the caller is also
/// the only party that can honestly report a duration — and it does, on the `DRAIN-TIMING`
/// boot line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetiredDrainStats {
    /// Host objects, GPU mappings and guest-RAM windows actually disposed of.
    pub disposed: usize,
    /// Disposals the host **refused**, and which are therefore gone from the queue without
    /// having been done. ★ Their disposition of record is the isolate's own death (§7.0);
    /// they are counted rather than re-staged because re-staging a permanently-refusing
    /// object is the one shape that would make this drain non-terminating.
    pub residue: usize,
    /// Plan→execute→check-in turns. `turns × chunk` bounds `disposed + residue`.
    pub turns: usize,
    /// ★ **The budget ran out with work still queued.** This is the flag that says the
    /// remainder is riding to the next trap — i.e. the mechanism working, not failing. A
    /// boot where it is *always* set and `SharedDevice::retired_len` never falls is outcome
    /// (B): the cost moved rather than went away.
    pub budget_hit: bool,
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

/// ★★★★ **G1 — what one attribution pass over the CPU transport's pages did.**
///
/// See [`SharedDevice::witness_cpu_pt_pages`]. Every page handed in lands in exactly one of
/// [`Self::latched`], [`Self::vas_gone`] or [`Self::unattributed`], and those three mean
/// three different things about three different components: the page reached its `Vas`; the
/// index named an owner whose address space is gone (**ours**, R5); the index does not know
/// this page yet (**not yet**, and it must be requeued).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuPtWitness {
    /// Pages inserted into an owning `Vas`'s dirty set.
    pub latched: usize,
    /// Pages the index attributed and whose `Vas` had gone by the time the lock was taken.
    pub vas_gone: usize,
    /// ★★★ Pages no owner could be derived for, **carried back to be requeued**. Reported
    /// as addresses rather than counted so a caller can put them back and a test can name
    /// one — see [`SharedDevice::witness_cpu_pt_pages`] for why dropping them destroys the
    /// witness.
    pub unattributed: Vec<u64>,
    /// Procs that got at least one page, ascending — the population a decode pass must now
    /// be run for.
    pub procs: Vec<ProcId>,
}

/// ★★★★ What [`SharedDevice::forward_ring`]'s locked read phase found, **named**.
///
/// ⊘ The three non-`Fresh` variants are the arms on which a doorbell is reported `Served`
/// and **nothing is handed to any engine**. They were one `Ok(None)` until §16.70, which is
/// why `[measured 2026-08-10, boot p2_29e7c25_planereal]`'s `3 forwarded (host channel
/// rung)` could not be read either way — see [`SharedDevice::forward_ring`]'s own note.
enum RingRead {
    /// Entries the guest has written and this port has not yet forwarded.
    Fresh {
        /// The fresh entries' bytes.
        fresh: Vec<u8>,
        /// The resume cursor these entries start at.
        done: u32,
        /// How many entries.
        n: u32,
        /// How many bytes of ring were readable in total.
        bytes: usize,
    },
    /// [`kayfabe_fwd::read_gpfifo_ring`] found no ring, and said which of its six absences.
    NoRing(kayfabe_fwd::RingLook),
    /// The resume cursor is at or past the readable extent of the ring.
    CursorPastEnd {
        /// The resume cursor.
        done: u32,
        /// Readable ring bytes.
        bytes: usize,
    },
    /// The ring was read and the entries from the cursor on are all zero — the ring's own
    /// way of saying "no more work". ⊘ A real answer, not a failure to look.
    NoLiveEntries {
        /// The resume cursor.
        done: u32,
        /// Readable ring bytes.
        bytes: usize,
    },
}

/// ★★★★ **§16.71 — WHICH OBJECT the forwarding resolver resolved**, printed beside the
/// ring outcome so a reader can join it to the other projection's
/// [`CeChannelFacts::chan_key`].
///
/// # ⊘ The question this exists to make answerable
///
/// `[measured 2026-08-10]` §16.70.6 carried two ring addresses out of two boots —
/// `0x120064000` (control, CPU executor) and `0x420064000` (real plane, forwarding path) —
/// and recorded that it *could not say* whether RM had placed one channel's ring
/// differently or the two resolvers were reading **different channels**. Both numbers were
/// printed bare. ⇒ The log could not answer it *in principle*, not merely in fact, and no
/// amount of re-reading it would have helped.
///
/// ⊘ **Absence is carried as absence.** `key: None` means the channel record or its graph
/// node did not resolve at the instant the ring was read — which is a **lifetime** answer,
/// distinct from *"they are different channels"*, and the two must not print the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RingWho {
    /// The RM-graph node the ring was read off, as `(client, handle)`, or `None` if it did
    /// not resolve.
    key: Option<(u32, u32)>,
    /// The channel's bound page-directory base — the table `binding_at` was asked in.
    pdb: Option<u64>,
}

impl std::fmt::Display for RingWho {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.key {
            Some((c, h)) => write!(f, "key=0x{c:x}:0x{h:x}")?,
            // ⊘ Named, never blank: a channel whose node is gone is a different fact from a
            // channel whose node names a different object.
            None => write!(f, "key=UNRESOLVED-AT-RING-READ")?,
        }
        match self.pdb {
            Some(p) => write!(f, " pdb=0x{p:x}"),
            None => write!(f, " pdb=NONE"),
        }
    }
}

impl RingRead {
    /// A tag naming this outcome and carrying its numbers, for the diagnostic line.
    fn tag(&self) -> String {
        match self {
            RingRead::Fresh { done, n, bytes, .. } => {
                format!("RING bytes={bytes} cursor={done} live={n}")
            }
            RingRead::NoRing(look) => match look {
                kayfabe_fwd::RingLook::RingDeclaredEmpty { va, entries } => {
                    format!("{} va={va:#x} entries={entries}", look.tag())
                }
                kayfabe_fwd::RingLook::RingVaUnbound { va }
                | kayfabe_fwd::RingLook::RingMappedZero { va } => {
                    format!("{} va={va:#x}", look.tag())
                }
                other => other.tag().to_owned(),
            },
            RingRead::CursorPastEnd { done, bytes } => {
                format!("CURSOR-PAST-END cursor={done} bytes={bytes}")
            }
            RingRead::NoLiveEntries { done, bytes } => {
                format!("NO-LIVE-ENTRIES cursor={done} bytes={bytes}")
            }
        }
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
            fb: std::sync::OnceLock::new(),
            pb_vidmem: std::sync::atomic::AtomicBool::new(false),
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
                    pending_engine_forwards: Vec::new(),
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
            // ⊘ NAMED rather than swallowed by a `..`: a bare [`Gpu`] has no register plane,
            // so it has no lock to defer out of and nowhere for a latched forward to go. Any
            // request still latched here was decided by a plane that is being dismantled.
            // ⚠ Dropping it is correct and is NOT silent: [`EngineForwardRun`] never
            // existed for it, so no census row claims the verb ran.
            pending_engine_forwards: _dismantled,
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
        //
        // ⚠ **"Guards dropped" here means THIS FUNCTION'S guards.** `[measured 2026-08-11,
        // §16.88]` a caller six crates up (`RegPlane::write`, holding the plane's rank-0
        // mutex) reaches this through the GSP command policy, and for that caller this line
        // is an R1 violation that aborts QEMU on the guest's first `GSP_RM_ALLOC`.
        // ★ A correctness claim scoped to *my* locks reads exactly like one scoped to *all*
        // locks, and only the second is R1. ⇒ such a caller must use
        // [`SharedDevice::apply_deferring`] and drain with [`SharedDevice::materialize_pending`].
        // ⊘ This door is kept, unchanged, because lock-free callers exist and are correct;
        // `materialize` asserts, so a caller that is wrong about itself is refused by name.
        self.materialize(spawns);
        out
    }

    /// ★★★★★ **`apply`, for a caller that is UNDER A LOCK IT DID NOT TAKE** — decides the
    /// spawns and **leaves them latched** instead of running them.
    ///
    /// # ⊘ Why this leaves the queue where it is rather than handing it back
    ///
    /// The latch already lives in the spine (`Spine::take_pending_spawns`), and the frame that
    /// can safely spawn — the shim's register-write path — already holds an
    /// `Arc<SharedDevice>`. ⇒ **nothing needs to travel.** An earlier design carried the batch
    /// outward through `CommandPolicy`, which forced `kayfabe-core` vocabulary across a
    /// `kayfabe-gsp` **port** and needed either a new crate edge or a second identity
    /// vocabulary for `(Proc, GpuId)` (§16.90). Leaving the batch in `core` and letting the
    /// outermost frame **ask** costs neither.
    ///
    /// ⊘ Cancels still discharge here: firing an eventfd is a syscall but a **non-blocking**
    /// one, and `l1_os_shell.md` §4.5 enumerates it as the permitted exception. A `clone` +
    /// `execveat` is not on that list and cannot be added to it.
    ///
    /// ⚠ The caller **must** drain with [`Self::materialize_pending`] once its own locks are
    /// down, or the spawn waits for the next register write that does.
    pub fn apply_deferring(&self, ev: RmEvent) -> Result<(), GpuError> {
        let (out, cancels) = {
            let mut g = self.state.write();
            let st = &mut *g;
            let out = st
                .spine
                .apply(st.system.get_mut(), &mut ExclusiveProcs(&mut st.procs), ev);
            (out, st.spine.take_pending_cancels())
        };
        cancels.discharge_all();
        out
    }

    /// ★★★ **Drain and run whatever [`Self::apply_deferring`] latched** — the pull half.
    ///
    /// Idempotent and cheap when there is nothing latched: one rank-1 acquisition that moves a
    /// `Vec` and returns. ⚠ **Call with every ranked lock down.** `materialize` asserts it, so
    /// a caller that is wrong is refused by name rather than spawning under a lock.
    pub fn materialize_pending(&self) {
        let spawns = {
            let mut g = self.state.write();
            g.spine.take_pending_spawns()
        };
        if spawns.is_empty() {
            return;
        }
        self.materialize(spawns);
    }

    /// ★★★★★ **§16.96 — [`Self::forward_engine_object_by_parent`], for a caller that is
    /// UNDER A LOCK IT DID NOT TAKE.** Decides nothing, issues nothing, **latches**.
    ///
    /// # ⊘ Why this exists, measured
    ///
    /// `[measured 2026-08-11, §16.91, `traces/boots/w239/`]` the guest's first
    /// `GSP_RM_ALLOC` for an engine object reaches `ObjectModel::forward_engine_object`
    /// **six crates inside `RegPlane::write`**, which holds `LockRank::Plane` (rank 0), and
    /// the direct call issues a host RM ioctl there. QEMU aborts:
    ///
    /// ```text
    /// R1 no-blocking-under-lock violation: issuing a host RM verb while holding rank(s) [0]
    /// ```
    ///
    /// # ★★★ Why LATCHING is legal here, and the claim it corrects
    ///
    /// §16.91 concluded a forwarded RM verb *"can never be latched, because its result is
    /// the answer"*. ⊘ **That is false of this verb**, and the refutation is at its only
    /// production call site: `kayfabe_rmrpc::Bridge::deliver` **discards the result on
    /// purpose**, with the sentence *"⚠ the guest's answer does NOT change, and that is a
    /// decision, not an oversight"* — turning a host-side refusal into an alloc failure
    /// would fail `cuCtxCreate` outright and a boot measuring the forward would silently be
    /// measuring that instead. ⇒ **nothing in the guest's reply depends on this verb**, so
    /// it is exactly the *fire-and-forget* shape the spawn deferral already handles, and
    /// §16.91's own general rule — *work decided under a lock can be deferred only if
    /// nothing in the response depends on it* — **admits it**.
    ///
    /// ⇒ no third `CommandPolicy` outcome, no reply memo, no re-service door and no second
    /// decode of the command are needed (§16.94/§16.95's design solved a harder problem than
    /// the tree has).
    ///
    /// # ⊘ Ordering, which IS load-bearing
    ///
    /// `Bridge::deliver` runs `apply` **before** this, because the forward routes through
    /// the channel the alloc names and `Spine::by_vchid` is rebuilt by the projection
    /// `apply` runs. Latching preserves that: the apply still happens first, under the lock,
    /// and the verb happens strictly later. ⚠ The drain also runs **after**
    /// [`Self::materialize_pending`], so an isolate this same register write decided is
    /// installed before a forward that needs it — strictly better than the direct call,
    /// which could only meet a `FwdFault::IsolatePending`.
    ///
    /// ⚠ The caller **must** drain with [`Self::run_pending_engine_forwards`] once its own
    /// locks are down, or the forward waits for the next register write that does.
    pub fn forward_engine_object_deferring(
        &self,
        client: HClient,
        parent: HObject,
        class: ClassId,
        params: &[u8],
    ) -> ForwardAdmission {
        let mut g = self.state.write();
        // ★ THE CLASS GATE RUNS FIRST — the same gate, in the same position, that
        // `kayfabe_fwd::route_engine_object_by_parent` opens with, and for the reason its
        // own `[measured 2026-08-10]` comment gives: every non-engine alloc must exit
        // before anything else happens to it. Here it has a second job — it is what keeps
        // the latch bounded in practice, because client/device/memory/VASpace allocs are
        // the overwhelming majority and none of them is ever latched.
        //
        // ⊘ The result is discarded: the route asks the same question again and remains
        // the one authority for the ANSWER. This call is the GATE.
        if g.spine.arch().engine_of_object(class).is_none() {
            return ForwardAdmission::NotAnEngine(class);
        }
        let pending = g.pending_engine_forwards.len();
        if pending >= MAX_PENDING_ENGINE_FORWARDS {
            // ★★ REFUSE BY NAME. See [`MAX_PENDING_ENGINE_FORWARDS`] for why a bound is
            // owed at all when the observed population is one.
            return ForwardAdmission::LatchFull {
                pending,
                bound: MAX_PENDING_ENGINE_FORWARDS,
            };
        }
        g.pending_engine_forwards.push(PendingEngineForward {
            client,
            parent,
            class,
            params: params.to_vec(),
        });
        ForwardAdmission::Latched {
            pending: pending + 1,
        }
    }

    /// ★★★★★ **WHICH CHANNELS ARE ABOUT TO HAVE A HOST CHANNEL BORN FOR THEM** — the latch,
    /// READ and not taken.
    ///
    /// # ⊘ Why a peek exists at all, when a drain is right there
    ///
    /// `commit_engine_object` is where a `GrCompute` channel's **host** channel is
    /// materialized, and `alloc_channel_in`'s guest-ring arm `narrow()`s the ring's memory
    /// handle — so any object over the guest's ring must be minted **before** the drain, not
    /// during it and not after. This is the only instant at which *"a host channel is about
    /// to be born for guest channel X"* is knowable and X is still un-born.
    ///
    /// ⊘ **It takes nothing and mutates nothing.** A caller that drained here to inspect the
    /// entries would have run the forwards it meant to prepare for.
    ///
    /// ⚠ Returns `(client, parent, class)` and **not** the params: nothing upstream of the
    /// forward has any business reading the guest's alloc blob, and a peek that handed it
    /// out would be an inspection surface wearing a routing name.
    #[must_use]
    pub fn peek_pending_engine_forwards(&self) -> Vec<(HClient, HObject, ClassId)> {
        self.state
            .read()
            .pending_engine_forwards
            .iter()
            .map(|p| (p.client, p.parent, p.class))
            .collect()
    }

    /// ★★★ **§16.96 — drain and run whatever [`Self::forward_engine_object_deferring`]
    /// latched** — the pull half, and the mirror of [`Self::materialize_pending`].
    ///
    /// Idempotent and cheap when there is nothing latched: one rank-1 acquisition that
    /// moves an empty `Vec` and returns — the same cost `materialize_pending` already pays
    /// on every register write.
    ///
    /// ⚠ **Call with every ranked lock down.** `Worker::execute` asserts exactly that
    /// (`assert_lock_free("issuing a host RM verb")`), so a caller that is wrong about
    /// itself is refused **by name, at the drain**, rather than six crates away.
    ///
    /// ⊘ Returns one row per request **in the order the guest declared them**, so the
    /// caller can report outcomes it never saw the requests for. A refusal is a row like
    /// any other: this drain never swallows one, because the census of what the host said
    /// is the only reason the forward is observable at all.
    ///
    /// # ★★★★★ w288 — `err_notifier_grants`, and why the VMM hands them to the DRAIN
    ///
    /// Each entry names *"for the channel this `(client, parent, class)` alloc lands on, the
    /// guest's own error-notifier pages, as a slice of the guest-RAM block"*. Only the VMM
    /// may derive a [`kayfabe_isolate::GuestRamGrant`], so it cannot be computed in here or
    /// anywhere below; the caller resolves the channel's declared notifier GPA through its
    /// **own** stated layout and passes the result down.
    ///
    /// ⊘ Matched by key, and an entry with no match is simply not applied. A latch whose
    /// channel the caller could not resolve gets `None` — a channel born **without** a
    /// notifier, which is exactly the pre-w288 behaviour and never a silent substitution of
    /// somebody else's pages. ⚠ Positional matching was deliberately not used: the latch can
    /// grow between a caller's peek and this drain, and a positional scheme would then apply
    /// each grant to the *wrong* channel — the one failure mode worse than applying none.
    #[must_use]
    pub fn run_pending_engine_forwards(
        &self,
        err_notifier_grants: &[EngineNotifierGrant],
    ) -> Vec<EngineForwardRun> {
        let batch = {
            let mut g = self.state.write();
            core::mem::take(&mut g.pending_engine_forwards)
        };
        batch
            .into_iter()
            .map(|p| {
                let grant = err_notifier_grants
                    .iter()
                    .find(|g| g.client == p.client && g.parent == p.parent && g.class == p.class)
                    .map(|g| g.grant);
                EngineForwardRun {
                    client: p.client,
                    parent: p.parent,
                    class: p.class,
                    params_len: p.params.len(),
                    out: self.forward_engine_object_by_parent(
                        p.client, p.parent, p.class, &p.params, grant,
                    ),
                }
            })
            .collect()
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
                // ⊘ Not this op's business; named because the destructure is exhaustive
                // by design (a field added and not considered is a compile error here).
                pending_engine_forwards: _,
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

    /// ★★★★★ **w317 — the reap under [`ReapPolicy::HoldUndrained`]:** reap only the procs
    /// whose staged disposal queue [`SharedDevice::drain_retired_budgeted`] has already
    /// emptied, and hold the rest for the next quiesce edge.
    ///
    /// Returns `(reaped, deferred_for_drain)`. ★ **The pair is the point.** Under the budget,
    /// "nothing was reaped" stops being one fact: a trap that reaps zero because nothing died
    /// and a trap that reaps zero because a proc's queue is not draining are the same number
    /// on [`SharedDevice::reap_retired`]'s signature, and the second is precisely this rung's
    /// pre-registered outcome (B) — the bound having **moved** cost rather than removed it.
    /// `a_count_cannot_see_a_substitution`.
    ///
    /// ⚠ Call it **after** the drain, never instead of it. On its own this only defers, and a
    /// deferral with nothing draining it is the leak with extra steps.
    pub fn reap_retired_held(&self) -> (usize, usize) {
        let reclaimed = {
            let mut g = self.state.write();
            g.spine
                .reap_retired_with(kayfabe_core::gpu::ReapPolicy::HoldUndrained)
            // ↑ the write guard dies at this brace, BEFORE the drop below.
        };
        let n = reclaimed.len();
        let d = reclaimed.deferred_for_drain();
        drop(reclaimed); // ← the blocking teardown, with zero ranked locks held.
        (n, d)
    }

    /// ★★★★★ **w317 — THE BUDGETED DRAIN: dispose of retired procs' staged host objects a
    /// bounded slice at a time, instead of all of them inside one guest MMIO trap.**
    ///
    /// # The number this exists for
    ///
    /// `[measured 2026-08-14 (w314), bench vh, real GA106, n=4 per arm, non-overlapping
    /// ranges]` `Regs::write`'s reap halted **every vCPU and QEMU's main loop** — they all
    /// serialise on the BQL — for **2.65–2.92 s** on clean master and up to **3.70 s** with
    /// w310's pin release, against `scrubberDestruct`'s **4 000 ms**
    /// (`ce_utils.c:349`). That is `INLINE-SAFE` clause (b)
    /// (`blocking_and_completion_model.md` §1) failing at 92.6 % of budget on a *green* boot
    /// of the *standard* workload.
    ///
    /// # The shape — deferral was already there; only the bound was missing
    ///
    /// w303 armed the reap at the **GSP re-handshake edge**, and that edge **recurs**: a proc
    /// deferred for not being quiesced is retried at the guest's next MMIO write. This adds
    /// nothing to that structure. It drains a bounded slice, and `Spine::reap_retired` holds
    /// the proc back while any drainable remainder exists — so the unbounded burst in
    /// [`kayfabe_core::gpu::Proc`]'s `Drop` is never reached with a full queue.
    ///
    /// # ⊘ Why the budget is a CLOSURE and not a `Duration`
    ///
    /// This crate is *pure std with no OS waiting primitives*, and the core it drives has no
    /// clock at all (§8.3). Handing the deadline in as `over_budget` keeps the wall clock in
    /// the shell where it belongs **and** makes the bound testable offline: a closure that
    /// returns `true` after k turns exercises the exact control flow a 40 ms deadline does,
    /// with no sleep and no flake. ★ A budget that can only be checked on the bench is a
    /// budget nobody re-checks.
    ///
    /// `chunk` is a **granularity**, not the budget: it caps how far past the deadline one
    /// turn can carry us. The bound delivered is `budget + (chunk × per-disposal cost)`.
    ///
    /// # ⚠ What it does NOT do
    ///
    /// It does not make the disposal asynchronous, and it does not reduce the total work: the
    /// same objects are freed by the same verbs on the same thread. It converts **one 3.7 s
    /// stall** into **many bounded ones with the guest running in between**. That is a clause
    /// (b) fix and nothing else; clause (b) is the clause that was failing.
    pub fn drain_retired_budgeted(
        &self,
        chunk: usize,
        mut over_budget: impl FnMut() -> bool,
    ) -> RetiredDrainStats {
        let mut stats = RetiredDrainStats::default();
        loop {
            // ---- PLAN: pure state, under the device write guard. No verb here.
            let planned = {
                let mut g = self.state.write();
                g.spine.plan_retired_drain(chunk)
                // ↑ the write guard dies at this brace, BEFORE the verb below.
            };
            let Some(kayfabe_core::gpu::RetiredDrain {
                pid,
                gpu,
                mut worker,
                orphans,
            }) = planned
            else {
                break; // nothing drainable — the overwhelming majority of traps.
            };
            let n = orphans.len();
            // ---- EXECUTE: zero ranked locks held (R1, asserted by `Worker::execute`).
            let undisposed = kayfabe_fwd::dispose_on(&mut worker, orphans);
            stats.residue += undisposed.len();
            stats.disposed += n - undisposed.len();
            stats.turns += 1;
            // ⊘ The residue is DROPPED, exactly as `Proc::drop` and
            // `SharedDevice::drain_pending_releases` already drop theirs. Re-staging it
            // would be the one shape that breaks this rung's termination argument: a
            // permanently-refusing object would be split off and put back forever, and the
            // proc would never reap. It is counted instead, which is what makes
            // "the drain is not making progress" a readable number rather than a hang.
            self.return_worker(pid, gpu, worker);
            if over_budget() {
                stats.budget_hit = true;
                break;
            }
        }
        stats
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
                for ch in p.channels.values() {
                    // ⊘ **THROUGH `of`, not hand-built.** This site used to construct the
                    // row field by field beside `VasCensusRow::of`, which builds the same
                    // row from the same `Channel` — the second computation
                    // [`ChannelVasRow`]'s own re-export doc says was deleted (*"Two
                    // computations that agree today are not corroboration; they are a
                    // drift waiting for somebody to read a formatting difference as a fact
                    // about the guest"*). It was deleted for the FORMATTER and not for the
                    // ROW. `[measured 2026-08-11]` the two agreed on all eight fields; the
                    // ninth (`kind`) is what made the duplication cost something, because
                    // one of the two would have had to re-derive it from `proc`.
                    //
                    // ★ `ch.id` is the `ChanId` the map is keyed by — `of` reads it off the
                    // channel, which is the same value, from the object rather than from
                    // the index.
                    out.push(ChannelVasRow::of(pid, ch));
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
                // ★ **w310** — `Orphans::len()` and not a hand-rolled sum of two fields: the
                // hand-rolled version silently omitted `guest_ram` the moment a third kind
                // existed, and *"how much did this drain dispose of"* would have under-read
                // by exactly the kind this rung added.
                let n = orphans.len();
                // ---- EXECUTE: zero locks held.
                let undisposed = kayfabe_fwd::dispose_on(&mut worker, orphans);
                disposed += n - undisposed.len();
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

    /// ★★★ **w310 — guest-RAM pin reclaim, from procs that have already vacated.**
    ///
    /// **Spine op** (read guard only), which is why it is the *gone* half and not the whole
    /// device total: summing live procs would take one rank-1 proc lock each, and this is
    /// called from `Regs::write` where the point of the frame is that it holds nothing.
    ///
    /// It is not a partial answer for long: `Spine::vacate` absorbs a proc's **cumulative**
    /// tally — including pins released at VAS deaths while it was still alive — so every
    /// release a dead proc ever made lands here. A live proc's own running tally is read
    /// per-proc through [`SharedDevice::pin_reclaim_of`].
    ///
    /// ⊘ **Monotone. Grade it as a FLOOR, never as an exact value** — the mistake w304's
    /// criterion (E) was rewritten for.
    #[must_use]
    pub fn pin_reclaim_gone(&self) -> kayfabe_core::gpu::PinReclaim {
        self.state.read().spine.pin_reclaim_gone()
    }

    /// One live proc's cumulative guest-RAM pin reclaim tally. See
    /// [`SharedDevice::pin_reclaim_gone`].
    #[must_use]
    pub fn pin_reclaim_of(&self, pid: ProcId) -> kayfabe_core::gpu::PinReclaim {
        self.with_proc_mut(pid, |p| p.pin_reclaim)
            .unwrap_or_default()
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
                    // ⊘ Not this op's business; see `materialize`'s destructure.
                    pending_engine_forwards: _,
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
                    // ⊘ Not this op's business; see `materialize`'s destructure.
                    pending_engine_forwards: _,
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
            // ★★★★★ **w323 — THE TRAP WITNESS, MINTED AT THE ONE PLACE THAT ISSUES.**
            //
            // `assert_lock_free` (inside `execute`) answers *"what do I hold"*. This answers
            // *"where am I"* — `INLINE-SAFE` clauses (a)/(b), which had no mechanism until
            // this rung. `at_a_host_verb` takes the honest branch: a `claim` on the
            // publication worker, a **counted** `inline_under_bql` on a vCPU inside a guest
            // trap. ⇒ `kayfabe_util::trapwitness::inline_exceptions()` is a boot-visible
            // count of how many host verbs still ran with the BQL held, and driving it to
            // zero is what "publication is off the BQL" means, measured rather than claimed.
            let off = kayfabe_util::trapwitness::OffTrap::at_a_host_verb(
                "kayfabe_rt::SharedDevice::verb_op — the execute phase",
            );
            let executed = worker.execute(&verbs, &off);
            let gpu = staged.gpu;
            let Ok(reply) = executed else {
                let failure = executed.expect_err("matched Err");
                let err = failure.err;
                // ★ §16.105 — read BEFORE `failure.orphans` is moved out below. See
                // [`kayfabe_isolate::VerbFailure::on`] for why nothing downstream can
                // re-derive it.
                let on = failure.on;
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
                    e if self.proc_is_live(staged.proc) => FwdFault::Rm { err: e, on },
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
                channel_facts_from(spine, proc, route.proc, route.chan, route.gpu, route.vchid)
            },
        )?
    }

    /// ★★★★★ **THE SAME DECLARED FACTS, ROUTED BY THE ENGINE-OBJECT'S PARENT INSTEAD OF BY
    /// A DOORBELL TOKEN** — because the channel's ring has to be reachable **before** the
    /// first doorbell, and a token does not exist yet at that moment.
    ///
    /// # ⊘ Why a second entry point and not a second derivation
    ///
    /// The body is [`Self::ce_channel_facts`]' body, called — not copied. The two differ in
    /// exactly one thing, the **route**, and they are deliberately the two routes the tree
    /// already owns: `route_doorbell` (by `(GpuId, VChid)` off a work-submit token) and
    /// `route_engine_object_by_parent` (by the parent handle the guest's own alloc named).
    /// Everything downstream — the resolved VA space, the declared ring, the pdb — is read
    /// off the one graph node that declared it, exactly once, in `channel_facts_from`.
    ///
    /// ★★★ **This is the hop the ordering fact demands.** `GuestRing::memory` is a
    /// `HostHandle` and `alloc_channel_in`'s guest arm `narrow()`s it, so the host object
    /// over the guest's ring must EXIST before the host channel is born — and the host
    /// channel is born on the engine-object path (`commit_engine_object`), upstream in time
    /// of every doorbell. ⊘ Binding may be late (R31 arm C: an unmapped `gpFifoOffset` was
    /// **accepted**); minting may not.
    ///
    /// # Errors
    /// Whatever `route_engine_object_by_parent` refuses with — including
    /// [`FwdFault::NotAnEngine`] for a class that was never an engine object — then
    /// [`FwdFault::UnknownVchid`] / [`FwdFault::NoVas`] exactly as
    /// [`Self::ce_channel_facts`].
    pub fn engine_object_channel_facts(
        &self,
        client: HClient,
        parent: HObject,
        class: ClassId,
    ) -> Result<CeChannelFacts, FwdFault> {
        self.route_act(
            |spine| {
                let r = kayfabe_fwd::route_engine_object_by_parent(spine, client, parent, class)?;
                Ok((r.proc, r))
            },
            |spine, proc, route| {
                channel_facts_from(spine, proc, route.proc, route.chan, route.gpu, route.vchid)
            },
        )?
    }
}

/// The declared facts of one already-routed channel, read off the **one** graph node that
/// declared them. Shared by [`SharedDevice::ce_channel_facts`] and
/// [`SharedDevice::engine_object_channel_facts`] so the two routes can never come to
/// disagree about what a channel declared (`two_projections_of_one_fact_disagreeing`).
fn channel_facts_from(
    spine: &Spine,
    proc: &Proc,
    pid: ProcId,
    cid: ChanId,
    gpu: GpuId,
    vchid: VChid,
) -> Result<CeChannelFacts, FwdFault> {
    /// The four routing facts both entry points arrive with, named so the body below reads
    /// identically whichever route produced them.
    struct Routed {
        proc: ProcId,
        chan: ChanId,
        gpu: GpuId,
        vchid: VChid,
    }
    let route = Routed {
        proc: pid,
        chan: cid,
        gpu,
        vchid,
    };
    {
        {
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
                // ★★★★★ Off the SAME resolved `Channel` as `vas_pdb` and `engine`
                // below — the declared kind, never `route.proc == SYSTEM_PROC`
                // recomputed here. See [`CeChannelFacts::kind`].
                kind: chan.kind,
                // ★★★★★ w288 — off the SAME resolved `Channel` as `kind` above. ⊘ NOT
                // re-derived from the graph node's alloc facts: this struct exists because a
                // second derivation of one fact disagreed with the first
                // (`two_projections_of_one_fact_disagreeing`).
                error_notifier: chan.error_notifier,
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
                // ★ Off the SAME `node` the ring below is read from — see
                // [`CeChannelFacts::chan_key`]. ⊘ NOT `chan.key`: the point is to name
                // the object this struct's facts actually came out of, and a key read
                // off the channel record rather than off the resolved node would be a
                // different statement wearing the same name.
                chan_key: (node.key.client.0, node.key.handle.0),
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
        }
    }
}

/// ★★★★★ **The publication census of ONE `Vas`** — [`SharedDevice::vas_publish_census`]'s
/// answer, taken before any host verb exists.
///
/// ★ Every row of the table lands in **exactly one** bucket, and
/// `already_host + already_pinned + guest_ram + not_vidmem + not_granular + candidates_total
/// == total`. That
/// identity is asserted by `tests/tests/publish_census.rs`, and it is what makes a short
/// `candidates` list readable as *"most rows are refused, and here is which gate"* rather
/// than as *"the walk stopped early"*. A census whose buckets did not sum could report a
/// comfortable zero for a class it simply never reached.
///
/// ⊘ **The counts are TOTAL; [`Self::candidates`] is CAPPED.** [`Self::candidate_bytes`] and
/// [`Self::capped`] together describe the qualifying set in full, so a reader must never
/// infer it from the list's length — that is the `dlen=0` mistake in list form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublishCensus {
    /// Every row in the table.
    pub total: usize,
    /// Rows already carrying a [`kayfabe_mmu::HostBacking`] — the idempotent steady state.
    pub already_host: usize,
    /// ★★★★★ Rows **covered by a live guest-RAM pin** — mapped in the host VAS at the
    /// guest's own VA by a real `OS_DESCRIPTOR`, and recorded in
    /// [`kayfabe_core::gpu::Vas::guest_ram_pins`] rather than in `Binding::host`.
    ///
    /// ⊘ Counted apart from [`Self::already_host`] and from [`Self::guest_ram`] because it is
    /// a **third** state and the two it sits between mean opposite things: `already_host` is
    /// "mapped and in the field", `guest_ram` is "not mapped, and the FB verb is the wrong
    /// one for it". Merging this into either would report a mapped range as unmapped or hide
    /// which of the two disjoint records holds it.
    pub already_pinned: usize,
    /// ⊘ Rows over the guest's own pages. **Excluded, not refused**: they are the guest-RAM
    /// pin's population, a different verb with a different lifetime, and a second
    /// establishment over them is the `0x51` collision that cannot be told from exhaustion.
    pub guest_ram: usize,
    /// Rows whose aperture is not `Vidmem` — no framebuffer to join (`FbLeafDisagrees`).
    ///
    /// ⊘⊘ **UNREACHABLE FROM GUEST-DECLARED ROWS, AND THAT IS PINNED, NOT ASSUMED.**
    /// `Binding::declared_by_guest` maps every non-`Vidmem` aperture a guest may declare
    /// onto a guest-RAM kind and refuses `Aperture::Peer` outright, so such a row is caught
    /// by [`Self::guest_ram`] one arm earlier. The bucket is kept because the gate it names
    /// is real and another populate source could reach it — but it is asserted **zero** by
    /// `tests/tests/publish_census.rs`, because a dead bucket nobody pins reads as a
    /// measured zero and would send a reader chasing the wrong gate.
    pub not_vidmem: usize,
    /// ★★★ Rows RM's 64 KiB fixed-placement granularity cannot cover exactly
    /// (`FbLeafGranularity`). **This is the number that decides the rung**: it is how much
    /// of the table the proven verb structurally cannot take.
    pub not_granular: usize,
    /// The bytes behind [`Self::not_granular`] — the same fact weighted by size, because a
    /// count of 4 KiB rows and a count of 2 MiB rows are not comparable quantities.
    pub not_granular_bytes: u64,
    /// `(va, len, phys)` of rows that pass every gate — **capped**, see [`Self::capped`].
    pub candidates: Vec<(u64, u64, u64)>,
    /// Bytes behind ALL qualifying rows, including those past the cap.
    pub candidate_bytes: u64,
    /// How many qualifying rows were dropped from [`Self::candidates`] by the cap.
    pub capped: usize,
}

impl PublishCensus {
    /// Qualifying rows, cap included — `candidates.len() + capped`.
    #[must_use]
    pub fn candidates_total(&self) -> usize {
        self.candidates.len() + self.capped
    }

    /// ★ The bucket identity, as a value rather than as a comment. `false` means a row was
    /// counted twice or not at all, and the caller must print that instead of the census.
    #[must_use]
    pub fn buckets_sum(&self) -> bool {
        self.already_host
            + self.already_pinned
            + self.guest_ram
            + self.not_vidmem
            + self.not_granular
            + self.candidates_total()
            == self.total
    }
}

impl SharedDevice {
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
    /// ★★★★★ **Register the framebuffer source that turns route B on** — once, and only
    /// once, for this device's life.
    ///
    /// # ⊘ This is the ENTIRE switch, and it is a presence rather than a boolean
    ///
    /// `kayfabe_fwd::read_gpfifo_ring` derives its route from whether it was handed a
    /// reader, so a device nobody calls this on refuses vidmem ring ranges **exactly** as it
    /// did before route B existed. ⇒ *A default-off flag that is not off by construction is
    /// a default-on flag with a comment.* There is no boolean to read the wrong way, and no
    /// call site that can forget to check one.
    ///
    /// ⚠ **The source is consulted with NO ranked lock held** (§16.87). The production one
    /// takes the plane's rank-0 mutex, which may not be acquired beneath the core's ranks
    /// 1-2 — that inversion is what `check_acquire` now refuses by name.
    ///
    /// # Errors
    /// Returns the source back if one was already registered. ⊘ Not a panic and not a
    /// silent overwrite: a second registration means two subsystems each believe they own
    /// the framebuffer answer, and the caller is the only one who can say which.
    pub fn set_fb_source(
        &self,
        src: Arc<dyn kayfabe_fwd::FbSource>,
    ) -> Result<(), Arc<dyn kayfabe_fwd::FbSource>> {
        self.fb.set(src)
    }

    /// ★★★★★ `[w281]` **Arm the PUSHBUFFER's vidmem route — its OWN flag, never route B's.**
    ///
    /// `w279`'s result ruled this explicitly: *"widen it — as its **own** flag, never folded
    /// into route B"*. Route B is [`SharedDevice::set_fb_source`], a **supply**: it says
    /// whose bytes we may serve. This is a **route**: it says which read may consume them.
    /// Folding them together would make a boot unable to say whether a byte reached the
    /// decoder through the ring's widening or the pushbuffer's — the attribution `w279`
    /// paid a whole rung to keep.
    ///
    /// ⊘ **It is NECESSARY-NOT-SUFFICIENT on its own.** With no [`kayfabe_fwd::FbSource`]
    /// registered there is nothing to read, so a vidmem run still raises
    /// [`FwdFault::PushbufferAperture`] — the same shape as *"route B is unreachable with
    /// the witness disarmed"*. Both must be on; each is asserted separately in a boot's log.
    pub fn set_pushbuffer_vidmem(&self, on: bool) {
        self.pb_vidmem
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether the pushbuffer's vidmem route is armed. ⊘ Read per parse rather than cached,
    /// so a log line and the act cannot disagree about what this boot ran with.
    #[must_use]
    pub fn pushbuffer_vidmem(&self) -> bool {
        self.pb_vidmem.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// attached port whenever it has one.
    pub fn doorbell(
        &self,
        vmm: Option<&mut dyn kayfabe_vmm::Vmm>,
        target_gpu: GpuId,
        token: u64,
        working_set: &[GpuVa],
        err_notifier_grant: Option<kayfabe_isolate::GuestRamGrant>,
    ) -> Result<DoorbellOutcome, FwdFault> {
        let out = self.verb_op(
            || {
                self.route_act(
                    |spine| {
                        let r = kayfabe_fwd::route_doorbell(spine, target_gpu, token)?;
                        Ok((r.proc, r))
                    },
                    |_spine, proc, route| {
                        // ★★★★★ **w288 — THE VMM'S GRANT, PASSED THROUGH.** Nothing in this
                        // crate derives it and nothing here checks it: only the VMM may mint
                        // a `GuestRamGrant`, and `plan_doorbell` still gates it on the
                        // channel's own declaration.
                        let planned = kayfabe_fwd::plan_doorbell(
                            proc,
                            &route,
                            working_set,
                            err_notifier_grant,
                        )?;
                        Staged::check_out(proc, planned.plan.cgpu, planned)
                    },
                )?
            },
            kayfabe_fwd::commit_doorbell,
        )?;
        // ★★★★★ **THE CONTENT FORWARD IS ASKED FOR BY ENGINE** — see
        // [`ring_content_is_forwardable`], which carries the ruling and its reason.
        //
        // ⊘ Two conditions, and they are different questions. `vmm` is *"does this caller
        // hold a guest-memory port at all"*; the predicate is *"is parsing this ring with
        // the copy-engine codec the right verb for this engine"*. A GR doorbell answers
        // **no** to the second, so `forward_ring`'s `?` can never turn a rung host doorbell
        // into a `Refused` report — which is the property
        // `tests/tests/doorbell_is_forwarded_without_reading_the_ring.rs` pins and the GR
        // route depends on.
        //
        // ⊘ It is checked HERE and not inside `forward_ring`, deliberately: that function's
        // own doctrine is that *every* arm which forwards nothing must be NAMED, and it
        // already has eight paths to `Ok(())`. A ninth, whose meaning is *"this was never
        // this function's kind of work"*, belongs at the call site where the alternative is
        // visible, not inside the thing being skipped.
        // ★★★★★ **w287 — AND THE KIND.** `out.kind` is carried off the same `chan` as
        // `out.engine`; a `Passthrough` channel's ring is the guest's and is never read here.
        if let Some(vmm) = vmm
            && ring_content_is_forwardable(out.engine, out.kind)
        {
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
    /// (`kayfabe-isolate-host/src/rm.rs:3260-3279`, reached through `ce_copy_outcome` and
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
        //
        // ★★★★ **§16.70 — THE ARM IS NAMED, always, including every arm that forwards
        // NOTHING.** `[measured 2026-08-10, boot p2_29e7c25_planereal]` three doorbells were
        // reported `3 forwarded (host channel rung)` and the guest's scrubber then timed out
        // on a completion that never came. Eight distinct paths through this function reach
        // `Ok(())` — six inside `read_gpfifo_ring`, plus the cursor-past-end and
        // no-live-entries arms here, plus an empty span list below — and **every one of them
        // makes the doorbell report `Served` with zero bytes handed to any engine**. So a
        // reader of `3 forwarded` could not tell *"the GPU ran it and we lost the
        // completion"* from *"the channel was rung with nothing in it"*
        // (`execution_plane_increments.md` §16.69.5). ⊘ `3 forwarded` counts the doorbells
        // the PORT rang; it has never counted work.
        // ★★★★★ **THREE PHASES, and the split is FORCED by the lock order (§16.87).**
        //
        // The ring may live in the **emulated framebuffer** (the 8 `proc 2` doorbells do),
        // and the only seam that serves those bytes takes the plane's mutex — now
        // `LockRank::Plane`, rank 0. `route_act` holds ranks 1 and 2, so reading the
        // framebuffer inside it is `core → plane`, which `check_acquire` refuses **by name**.
        // ⇒ PLAN under the locks, FETCH with every guard dropped, ACT under them again.
        //
        // ⚠ **`done` is deliberately re-read in phase 3, not carried from phase 1.** Carrying
        // it would make the cursor a value from an instant before an unlocked window — R5's
        // "re-acquire and RE-VALIDATE" is the rule, and the widened window is exactly what
        // makes it load-bearing here rather than pedantic.
        let fb_on = self.fb.get().is_some();
        let planned = self.route_act(
            |_spine| Ok((pid, ())),
            |spine, proc, ()| -> Result<(RingPlanned, RingWho), FwdFault> {
                // ★★★★ §16.71 — **WHO the forwarding path resolved**, taken in the SAME
                // locked look, from the SAME two lookups `read_gpfifo_ring` performs
                // (`proc.channels[cid]` → `rmgraph.node_of_resource(chan.key)`), so the
                // identity and the ring address below cannot come from different instants.
                //
                // ⊘ This is a JOIN KEY, not corroboration. §16.70.6 recorded two ring
                // addresses for what it called one token and could not say whether the two
                // resolvers were looking at one channel or two, because **neither side ever
                // printed which object it had resolved**. Two of our own computations
                // agreeing proves nothing (`measure_at_the_boundary_not_inside`); what this
                // buys is that the *other* machinery's line
                // (`CeChannelFacts::chan_key`, produced under a different lock acquisition
                // at a different instant) can be joined to this one by a reader.
                let who = RingWho {
                    key: proc
                        .channels
                        .get(&cid)
                        .and_then(|c| spine.rmgraph.node_of_resource(c.key))
                        .map(|n| (n.key.client.0, n.key.handle.0)),
                    pdb: proc.channels.get(&cid).and_then(|c| c.vas_pdb).map(|p| p.0),
                };
                // ⊘⊘ **CORRECTED `[w300, 2026-08-13]`, above the text it corrects: THIS
                // CLOSURE NO LONGER READS THE FRAMEBUFFER, AND ROUTE B IS WIRED.** The
                // paragraph below (`[w235, 2026-08-11]`) said route B was *not* wired here
                // because reading it would invert the lock order. That was true when
                // written and was fixed four commits later: `Mutex<PlaneState>` is now
                // `RankedMutex`/`LockRank::Plane` (w236 `56269390`) and `forward_ring` was
                // split into **PLAN / FETCH / ACT** (w237 `7107bba5`) — this closure only
                // PLANS; the bytes are fetched below with every ranked guard dropped.
                // ⇒ ★ The reasoning below is still exactly **why the three phases exist**,
                // so it is kept; only its status word ("NOT wired") was false.
                // ⚠ And it is no longer true that "no existing gate would have caught it":
                // `check_acquire` refuses `core → plane` by name, always-on. What is STILL
                // true is the *scope* of that enforcement — it covers **ranked** locks only.
                // See `tests/tests/unranked_locks.rs` for which clause of
                // `blocking_and_completion_model.md` §1's INLINE-SAFE predicate is enforced
                // here and which is merely enumerated.
                //
                // ⊘ [SUPERSEDED STATUS, LIVE REASONING] The 8 `proc 2`
                // doorbells have their ring in the **emulated framebuffer**, so this call
                // answers `PushbufferAperture` for them. The bytes exist and the descent
                // prints them; the only seam that serves them
                // (`kayfabe_device::RegPlane::pt_bytes`) takes the plane's FSM mutex **per
                // read**, and this closure runs under `route_act`'s **rank-0 device read
                // lock + rank-1 proc mutex**. Taking the plane mutex here is the exact
                // inversion of the established `plane → core` order, whose ABBA partner is
                // already shipping (the policy chain takes core locks under `state.lock()`
                // on another vCPU's MMIO trap) — so a guest that rings a doorbell on one
                // vCPU while touching a register on another **builds the deadlock itself**.
                // ⇒ `kayfabe_fwd::plan_gpfifo_ring` / `fetch_ring_bytes` are the R1-correct
                // shape this needs (plan under the locks, bytes with every guard dropped).
                match kayfabe_fwd::plan_gpfifo_ring(spine, proc, cid, fb_on)? {
                    kayfabe_fwd::RingPlanLook::Planned(plan) => {
                        Ok((RingPlanned::Plan(Box::new(plan)), who))
                    }
                    kayfabe_fwd::RingPlanLook::Absent(a) => Ok((RingPlanned::NoRing(a), who)),
                }
            },
        )?;
        let (planned, who) = planned?;
        let plan = match planned {
            RingPlanned::Plan(p) => p,
            RingPlanned::NoRing(a) => {
                // ⊘ The SAME line the fall-through below prints, for the same reason: the
                // arm that forwards nothing is the arm a counter cannot see.
                let look = RingRead::NoRing(a);
                eprintln!(
                    "kayfabe: FWD-RING proc={} chan={} {who} {} → NOTHING FORWARDED (the \
                     doorbell still reports SERVED)",
                    pid.0,
                    cid.0,
                    look.tag(),
                );
                return Ok(());
            }
        };

        // ---- FETCH. ⊘ NO ranked lock is held here, and that is the whole point: this is
        // where the plane's rank-0 mutex may be taken, above the core's ranks in the order.
        let ring = {
            let src = self.fb.get().cloned();
            let mut borrowed = src.as_deref().map(kayfabe_fwd::FbSourceRef);
            let dynref: Option<&mut dyn kayfabe_fwd::FbBytes> = match borrowed.as_mut() {
                Some(b) => Some(b),
                None => None,
            };
            kayfabe_fwd::fetch_ring_bytes(&plan, vmm, dynref)?
        };

        // ---- ACT. Back under the locks, with the cursor RE-READ.
        let look = self.route_act(
            |_spine| Ok((pid, ())),
            |spine, proc, ()| -> Result<(RingRead, RingWho), FwdFault> {
                let done = proc.exec.forwarded.get(&cid).copied().unwrap_or(0);
                let pb = spine.arch().pushbuffer();
                let at = (done as usize).saturating_mul(pb.gpfifo_entry_stride());
                let bytes = ring.len();
                if at >= bytes {
                    return Ok((RingRead::CursorPastEnd { done, bytes }, who));
                }
                let fresh = &ring[at..];
                // ⊘ Only the entries the guest has WRITTEN. The tail of a ring is zeros,
                // and a zero entry is the ring saying "no more work" — not a malformed one.
                let n = kayfabe_fwd::gpfifo_live_entries(pb, fresh);
                if n == 0 {
                    return Ok((RingRead::NoLiveEntries { done, bytes }, who));
                }
                let take = n * pb.gpfifo_entry_stride();
                Ok((
                    RingRead::Fresh {
                        fresh: fresh[..take].to_vec(),
                        done,
                        n: u32::try_from(n).unwrap_or(u32::MAX),
                        bytes,
                    },
                    who,
                ))
            },
        )??;
        let (look, who) = look;
        let RingRead::Fresh {
            fresh,
            done,
            n,
            bytes,
        } = look
        else {
            // ⊘ Printed on the arm that forwards NOTHING, which is the arm a counter cannot
            // see. The absence carries its own name; nothing here decides what it means.
            eprintln!(
                "kayfabe: FWD-RING proc={} chan={} {who} {} → NOTHING FORWARDED (the \
                 doorbell still reports SERVED)",
                pid.0,
                cid.0,
                look.tag(),
            );
            return Ok(());
        };

        // ---- PARSE, then FORWARD. Each half takes and releases its own locks.
        let parsed = self.parse_pushbuffer(vmm, pid, cid, &fresh)?;
        let spans = parsed.ce_spans.len();
        if !parsed.ce_spans.is_empty() {
            // ★★★ THE OBSERVER. `forward_ce` → `plan_ce` → `Worker::execute`'s `CeSplit`
            // arm → `RmBackend::ce_copy`, whose host implementation waits on the engine's
            // own release semaphore and answers `CE_NEVER_RETIRED` if it never came. The
            // `?` is what makes that verdict the guest's: a doorbell whose copy did not
            // retire must not report `Served` (§14.8).
            let fwd = self.forward_ce(pid, cid, &parsed.ce_spans)?;
            eprintln!(
                "kayfabe: FWD-RING proc={} chan={} {who} RING bytes={bytes} cursor={done} \
                 live={n} spans={spans} → host_ce={} ours={} (each host_ce sub-copy has \
                 its own CE-SUBMIT line from the isolate)",
                pid.0, cid.0, fwd.host_ce, fwd.ours,
            );
        } else {
            // ⊘ A ring that decoded to no copy-engine operand at all. The bytes were read
            // and parsed; nothing in them was work an engine could be pointed at.
            eprintln!(
                "kayfabe: FWD-RING proc={} chan={} {who} RING bytes={bytes} cursor={done} \
                 live={n} spans=0 → NOTHING FORWARDED (the ring decoded to no CE span; \
                 the doorbell still reports SERVED)",
                pid.0, cid.0,
            );
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
    /// # ⊘⊘⊘ CORRECTED `[w281, 2026-08-12]` — IT **IS** SPLIT NOW, AND A LOCK RANK FORCED IT
    ///
    /// The paragraph below said the split would be *"a TOCTOU built on purpose"* and
    /// declined it. That reasoning stood while this path read **guest RAM only**. Reading a
    /// **vidmem** pushbuffer needs `kayfabe_fwd::FbBytes`, whose production implementation
    /// takes the plane's mutex — `LockRank::Plane`, **rank 0** — and this closure holds
    /// ranks 1 and 2. `check_acquire` refuses that `core → plane` acquisition **by name**,
    /// so the unsplit shape does not merely risk a deadlock: it **cannot run**.
    ///
    /// ⇒ Three phases, `forward_ring`'s exactly: `plan_pushbuffer` under the locks →
    /// `fetch_pushbuffer` with **every ranked guard dropped** → `decode_pushbuffer` +
    /// `apply_pushbuffer` under them again. ⚠ The TOCTOU is **accepted, not dissolved**:
    /// between plan and fetch the guest may invalidate a translation, so the bytes may come
    /// from a page it named and has since unmapped. Bounded, because the runs come from
    /// that channel's own table under the lock and are never recomputed outside it — a
    /// stale read of the guest's own memory, never a read of memory it never owned. The
    /// ring has carried this exposure since `w235`.
    ///
    /// ⊘ The superseded reasoning, kept because its *scope* is what changed:
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
        // ★★★ `[w281]` The route's arm, read ONCE per parse and printed with the plan, so a
        // grader reads the flag this act ran under rather than the one the process booted
        // with. ⊘ `pb_vidmem` alone is not enough: with no `FbSource` registered there is
        // nothing to read and a vidmem run still refuses. Both are asserted below.
        let pb_vidmem = self.pushbuffer_vidmem();
        // ---- PLAN. Ranks 0+1 held; no byte of guest memory or framebuffer is touched.
        let plan = self.route_act(
            // ROUTE phase — rank 0 held, no proc, no guest memory: the arch's GPFIFO
            // entry format and nothing else.
            move |spine| Ok((pid, kayfabe_fwd::pushbuffer_ranges(spine, ring))),
            // PLAN phase — rank 0 + this proc's rank 1. Translate through the channel's
            // own address table. The refusals (MISS, wrong aperture, over-fragmented) are
            // all raised HERE, under the lock, before anything is read.
            move |_spine, proc, ranges| kayfabe_fwd::plan_pushbuffer(proc, cid, &ranges, pb_vidmem),
        )??;
        // ---- FETCH. ⊘ NO ranked lock is held here — the whole reason for the split.
        let bytes = {
            let src = self.fb.get().cloned();
            let mut borrowed = src.as_deref().map(kayfabe_fwd::FbSourceRef);
            let dynref: Option<&mut dyn kayfabe_fwd::FbBytes> = match borrowed.as_mut() {
                Some(b) => Some(b),
                None => None,
            };
            // ★★ Printed on the arm that READS OUR OWN FRAMEBUFFER, and only there: a route
            // that is armed and never taken must not read as a route that was taken.
            if plan.touches_fb() {
                eprintln!(
                    "kayfabe: FWD-PUSHBUF proc={} chan={} ranges={} → VIDMEM RUNS PLANNED \
                     (pb_vidmem={pb_vidmem} fb_source={}) — the pushbuffer is read out of \
                     OUR OWN framebuffer, not guest RAM",
                    pid.0,
                    cid.0,
                    plan.len(),
                    dynref.is_some(),
                );
            }
            kayfabe_fwd::fetch_pushbuffer(&plan, vmm, dynref)?
        };
        // ---- ACT. Back under the locks. Decode is pure; apply needs the proc.
        let out = self.route_act(
            move |spine| Ok((pid, kayfabe_fwd::decode_pushbuffer(spine, &bytes))),
            move |spine, proc, methods| kayfabe_fwd::apply_pushbuffer(spine, proc, cid, methods),
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
        let mut fb = kayfabe_fwd::IsolateFb::new(worker);
        let mut out = self.decode_pt_writes_from(pid, fmt, &mut fb)?;
        // ★ A transport failure is surfaced as its own fact rather than blended into the
        // walk faults: `Ok(false)` from the isolate is a statement about the GUEST's page
        // tables, an `Err` is a statement about US, and a caller that cannot tell them
        // apart debugs the wrong plane. ⊘ Read HERE and not inside the shared body: it is
        // a property of *this* byte source, and the plane's store has no socket to break.
        out.transport = fb.transport_error();
        Some(out)
    }

    /// ★★★★ **G3 — the same pass, over a byte source the CALLER chooses.**
    ///
    /// [`SharedDevice::decode_pt_writes`] is this with `kayfabe_fwd::IsolateFb` supplied;
    /// everything that method's docs say about the three phases, the locks and the fourth
    /// PUBLISH phase applies here unchanged.
    ///
    /// # ⊘ Why the source is a parameter at all — the seam, stated
    ///
    /// **Which store is authoritative for a page-table decode is not one answer.** §12.2's
    /// fabricated aperture is memory *we* wrote on behalf of a copy *we* performed, and it
    /// lives in the isolate; the Mode-2 emulated plane's page tables are written by the
    /// **guest's own CPU** through the emulated BAR2 window and live in the device's
    /// `kayfabe_device::FbStore`. `[measured 2026-08-10, boot `w208_797a6bc_real`]` names
    /// which one the walling ring's tree is in and does not leave it to be argued: all five
    /// of its page-table pages carry `/byBAR2`, and the same boot's census reads `EXEC 0`.
    ///
    /// ⚠ Getting this wrong is *a self-consistent wrong store* — a writer and a reader that
    /// agree and are both wrong — so the caller states its source and the pass does not
    /// pick one.
    ///
    /// ⊘ [`kayfabe_fwd::PtDecodeOutcome::transport`] is left `None` here. A transport
    /// failure is a property of the source, `FbRead` has no channel to report one, and
    /// inventing an absence would be worse than the honest `None` a caller can override.
    ///
    /// Returns `None` if `pid` is not live.
    ///
    /// # Panics
    /// If a ranked lock is somehow held across the execute phase and the source asserts it
    /// (`kayfabe_isolate::Worker::fb_read` does).
    pub fn decode_pt_writes_from(
        &self,
        pid: ProcId,
        fmt: &dyn kayfabe_arch::GmmuFmt,
        fb: &mut dyn kayfabe_mmu::walker::FbRead,
    ) -> Option<kayfabe_fwd::PtDecodeOutcome> {
        self.decode_pt_writes_revoking(pid, fmt, fb, kayfabe_mmu::reach::PublishedUnbind::Refuse)
    }

    /// \u{2605}\u{2605}\u{2605}\u{2605}\u{2605} **w329 - [`SharedDevice::decode_pt_writes_from`] with the
    /// host-published unbind policy as a parameter.**
    ///
    /// \u{26a0} **The caller MUST dispose of [`kayfabe_fwd::PtDecodeOutcome::revoked`]**, and it must
    /// do BOTH halves: `kayfabe_device::plane::RegPlane::release_fb_join` for the guest's view and
    /// [`SharedDevice::revoke_published_fb_leaf`] for the host object. One without the other is
    /// either a store serving bytes out of an unmapped region (`SIGBUS` in the VMM) or a host
    /// object nothing names (a leak) - which is why the two are named together at every site.
    pub fn decode_pt_writes_revoking(
        &self,
        pid: ProcId,
        fmt: &dyn kayfabe_arch::GmmuFmt,
        fb: &mut dyn kayfabe_mmu::walker::FbRead,
        revoke: kayfabe_mmu::reach::PublishedUnbind,
    ) -> Option<kayfabe_fwd::PtDecodeOutcome> {
        // PLAN — rank 1, the owner's lock and no other.
        let plan = self.with_proc_mut(pid, kayfabe_fwd::plan_pt_decode)?;
        // EXECUTE — no lock held. `with_proc_mut` released it above, and the borrow of
        // the plan is of owned data, so nothing keeps a guard alive into here (★ a `Drop`
        // is a call site: `plan` owns `Vec`s of plain values, whose drop touches nothing).
        let results = (
            kayfabe_fwd::run_pt_decode(fmt, fb, &plan.tasks, kayfabe_fwd::PT_DECODE_BUDGET),
            None,
        );
        // COMMIT — rank 1 again, re-resolving every target (R5).
        let mut out = self.with_proc_mut(pid, |p| {
            kayfabe_fwd::commit_pt_decode_revoking(fmt, p, &results.0, revoke)
        })?;
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
        // ⊘ `transport` stays as `commit_pt_decode` left it. It is a property of the byte
        // source, which this body does not own; the caller that chose the source sets it.
        out.transport = results.1;
        Some(out)
    }

    /// ★★★★★ **THE WHOLE-VAS SWEEP** — the C's `enum_gr_sysmem` (`C: nvkvm_gpu_emul.c:583-591`)
    /// on this port's three-phase shape.
    ///
    /// Identical in structure to [`SharedDevice::decode_pt_writes_from`] — plan under rank 1,
    /// execute under no lock, commit under rank 1, publish under rank 0 — and different in
    /// exactly two places, both of which are the rung:
    ///
    /// 1. the plan seeds **one root task per stale address space**
    ///    (`kayfabe_fwd::plan_pt_sweep`) instead of draining a dirty set;
    /// 2. the commit admits every page the descent reached
    ///    (`kayfabe_mmu::reach::ReachShadow::witness_swept`), because a root descent reaches
    ///    pages nobody was seen to write and the witness gate would otherwise publish **zero**
    ///    — `[measured, w275]`.
    ///
    /// ⊘ **It is not a second address plane and it resolves nothing.** Same walker, same
    /// aperture-checked byte source, same one authoritative table, same refusal vocabulary; a
    /// miss is still a fault and the table is still never reverse-resolved.
    ///
    /// ⚠ **Read `witness_swept` before calling this.** It relaxes a deliberate correctness gate
    /// and carries an accepted residual (owner ruling, 2026-08-12), whose bound is the
    /// dirty-driven re-sweep — half of which lives in `kayfabe_fwd::plan_pt_decode`. A caller
    /// that runs this sweep without also running the decode pass has the relaxation and not its
    /// mitigation.
    ///
    /// Returns `None` if `pid` is not live.
    ///
    /// # Panics
    /// If a ranked lock is held across the execute phase and the source asserts it.
    pub fn sweep_pt_tables_from(
        &self,
        pid: ProcId,
        fmt: &dyn kayfabe_arch::GmmuFmt,
        fb: &mut dyn kayfabe_mmu::walker::FbRead,
    ) -> Option<(kayfabe_fwd::PtSweepPlan, kayfabe_fwd::PtDecodeOutcome)> {
        self.sweep_pt_tables_revoking(pid, fmt, fb, kayfabe_mmu::reach::PublishedUnbind::Refuse)
    }

    /// \u{2605}\u{2605}\u{2605}\u{2605}\u{2605} **w329 - [`SharedDevice::sweep_pt_tables_from`] with the
    /// host-published unbind policy as a parameter.** Same obligation on the caller as
    /// [`SharedDevice::decode_pt_writes_revoking`].
    pub fn sweep_pt_tables_revoking(
        &self,
        pid: ProcId,
        fmt: &dyn kayfabe_arch::GmmuFmt,
        fb: &mut dyn kayfabe_mmu::walker::FbRead,
        revoke: kayfabe_mmu::reach::PublishedUnbind,
    ) -> Option<(kayfabe_fwd::PtSweepPlan, kayfabe_fwd::PtDecodeOutcome)> {
        // PLAN — rank 1.
        let plan = self.with_proc_mut(pid, kayfabe_fwd::plan_pt_sweep)?;
        if plan.tasks.is_empty() {
            // ⊘ Returned rather than skipped, and with the plan attached: "every address space
            // was current" is a result, and it is the one a reader would otherwise confuse with
            // "the sweep did not run".
            return Some((plan, kayfabe_fwd::PtDecodeOutcome::default()));
        }
        // EXECUTE — no lock. ★ The budget is charged PER TASK; see `PT_SWEEP_BUDGET` for why
        // reusing the decode pass's run-wide budget would divide the C's number by a
        // guest-chosen quantity.
        let results = kayfabe_fwd::run_pt_sweep(fmt, fb, &plan.tasks, kayfabe_fwd::PT_SWEEP_BUDGET);
        // COMMIT — rank 1, re-resolving every target (R5).
        let mut out = self.with_proc_mut(pid, |p| {
            kayfabe_fwd::commit_pt_sweep_revoking(fmt, p, &results, revoke)
        })?;
        // PUBLISH — rank 0, from nothing, exactly as the decode pass does. A sweep learns far
        // more pages than a dirty drain, so this is the phase that makes the NEXT guest CE
        // write into any of them classify as a page-table write.
        if !out.learned_pages.is_empty() {
            let mut g = self.state.write();
            let st = &mut *g;
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
        Some((plan, out))
    }

    /// ★★ **How many page-table pages this proc holds that ONLY a sweep admitted**, summed
    /// over its address spaces.
    ///
    /// ⊘ The number that separates two readings of `swept_binds=0`, which look identical:
    /// *"the witness transport already covered every root-reachable page"* (`0` here — a
    /// statement about our transport) from *"those pages held no bindable leaves"* (`>0` here
    /// — a statement about the guest). `[measured, w276_on]` a boot printed `swept_binds=0`
    /// and could say neither.
    #[must_use]
    pub fn vas_swept_only(&self, pid: ProcId) -> usize {
        self.with_proc_mut(pid, |p| {
            p.vases.values().map(|v| v.reach.swept_only_len()).sum()
        })
        .unwrap_or_default()
    }

    /// ★★★ **ARM 2.1's raw material** — every address space's coalesced reachable VA ranges,
    /// one string per `Vas`.
    ///
    /// This is what makes *"is the faulting VA described by the guest?"* answerable **offline,
    /// from the boot log**, without knowing the address in advance. The wall's VA is ASLR'd and
    /// arrives from host `dmesg` after the fact, so a probe that had to be told the address up
    /// front could never be armed in time.
    ///
    /// ⚠ `cap` truncates the range list and the caller must print that it did. A truncated list
    /// read as complete turns *"not in the list"* — the answer arm 2.1 rests on — into a lie.
    #[must_use]
    pub fn vas_reachable_ranges(&self, pid: ProcId, cap: usize) -> Vec<String> {
        self.with_proc_mut(pid, |p| {
            p.vases
                .iter()
                .map(|(&(gpu, pdb), vas)| {
                    let r = vas.reach.reachable_ranges();
                    let shown: Vec<String> = r
                        .iter()
                        .take(cap)
                        .map(|(va, len)| format!("0x{va:x}+0x{len:x}"))
                        .collect();
                    format!(
                        "[proc={} gpu={} pdb=0x{:x} sweeps={} trunc={} runs={} {}{}]",
                        pid.0,
                        gpu.0,
                        pdb.0,
                        vas.sweep.sweeps,
                        u8::from(vas.sweep.truncated),
                        r.len(),
                        shown.join(","),
                        if r.len() > cap {
                            format!(" ⚠⚠ CAPPED at {cap} of {} runs — INCOMPLETE", r.len())
                        } else {
                            String::new()
                        }
                    )
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// ★★★★★ **WHAT OUR OWN ADDRESS TABLE HOLDS**, coalesced, one string per `Vas` — the
    /// counterpart to [`SharedDevice::vas_reachable_ranges`], and the instrument that makes
    /// the **third outcome** answerable offline.
    ///
    /// # ⊘⊘ The gap it closes, stated as the measurement that exposed it
    ///
    /// `[measured, w276b_on]` hardware faulted at a VA that the guest's own tables **describe**
    /// (`GUEST-DESCRIBES` names a run based exactly there) and that appears in **no**
    /// `refused_vas` list. From those two facts alone there are still three live readings and
    /// the log could not choose between them:
    ///
    /// 1. **it is bound** — the table already holds it, the mirror is right, and the fault is
    ///    about something other than this VA's mapping (reachability, aperture, the host
    ///    channel's own VAS);
    /// 2. **it was refused** — but off the end of a capped refusal list;
    /// 3. **it was dropped** — decoded and then lost, which is
    ///    [`kayfabe_mmu::reach::Settlement::shape_collisions`].
    ///
    /// A refusal list can never answer (1). Only the table itself can, and *"is this VA in the
    /// table"* is exactly what a coalesced dump of the table makes joinable against an address
    /// that is **ASLR'd and only arrives after the hang**. Same design as
    /// [`SharedDevice::vas_reachable_ranges`]: print unconditionally, join offline, never let
    /// the device learn the address.
    ///
    /// ⚠ `cap` truncates and the caller **must** print that it did — an absent range read as
    /// *"not bound"* is the `dlen=0` mistake, and this row is the one a story turns on.
    ///
    /// ⊘ Coalescing merges only **VA-contiguous** bindings, exactly as the reachable dump does,
    /// so the two lists are comparable run for run. It deliberately does **not** require the
    /// physical addresses to be contiguous: the question a reader brings here is *"is there a
    /// hole"*, and a run that is contiguous in VA and scattered in FB is still not a hole.
    #[must_use]
    pub fn vas_table_ranges(&self, pid: ProcId, cap: usize) -> Vec<String> {
        self.with_proc_mut(pid, |p| {
            p.vases
                .iter()
                .map(|(&(gpu, pdb), vas)| {
                    let mut runs: Vec<(u64, u64)> = Vec::new();
                    for (va, len, _b) in vas.table.iter() {
                        match runs.last_mut() {
                            Some((s, l)) if *s + *l == va => *l += len,
                            _ => runs.push((va, len)),
                        }
                    }
                    let shown: Vec<String> = runs
                        .iter()
                        .take(cap)
                        .map(|(va, len)| format!("0x{va:x}+0x{len:x}"))
                        .collect();
                    format!(
                        "[proc={} gpu={} pdb=0x{:x} rows={} runs={} {}{}]",
                        pid.0,
                        gpu.0,
                        pdb.0,
                        vas.table.iter().count(),
                        runs.len(),
                        shown.join(","),
                        if runs.len() > cap {
                            format!(" ⚠⚠ CAPPED at {cap} of {} runs — INCOMPLETE", runs.len())
                        } else {
                            String::new()
                        }
                    )
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// ★★★★★ **WHAT IS ACTUALLY IN THE HOST VAS** — the subset of [`Self::vas_table_ranges`]
    /// whose bindings carry a [`kayfabe_mmu::HostBacking`], coalesced the same way.
    ///
    /// # ⊘ The gap it closes, and it is the one `w289cup2` ended in
    ///
    /// `[measured, boot w289cup2]` hardware faulted `ENGINE GRAPHICS ... FAULT_PDE` at a VA that
    /// **both** `GUEST-DESCRIBES` and `TABLE-DESCRIBES` name as a run based exactly there
    /// (`0x7ff6a6e00000+0x200000`, proc=2 pdb=0x201000). From those two rows the fault has no
    /// reading at all: our mirror is right and hardware still missed above the leaf.
    ///
    /// ★ The missing distinction is that **`TABLE-DESCRIBES` is a claim about OUR shadow, not
    /// about the host page tables the GPU actually walks.** A row with `host: None` was declared
    /// by the guest and never materialized — no host object, no `map_dma`, therefore no host
    /// PDE. A `FAULT_PDE` over a VA our table holds is exactly what a fully-unpublished run
    /// produces, and no instrument in the tree could say so, because *"bound"* and
    /// *"published"* were printed as one number.
    ///
    /// ⇒ Read beside `TABLE-DESCRIBES`: **same run in both ⇒ the mapping is live host-side**;
    /// **present in `TABLE-DESCRIBES` and absent here ⇒ declared only, and the GPU cannot see
    /// it.** That is the third outcome `vas_table_ranges`'s own doc could not reach.
    ///
    /// ⚠ Same cap discipline, and the caller must print that it truncated: an absent run read as
    /// *"not published"* would be the `dlen=0` mistake pointing the other way.
    ///
    /// # ⊘⊘ CORRECTED 2026-08-13 (w290) — **THIS ROW ALONE UNDER-REPORTS, AND MY OWN FINDING
    /// # WAS TOO STRONG BECAUSE OF IT**
    ///
    /// `w290` reported `host_rows=4 of 16425` and read it as *"the host VAS the GPU walks is
    /// empty"*. **That reading counts only ONE of the two records this port keeps of host-side
    /// mapping state.** `commit_pin_guest_ram` (`kayfabe-fwd/src/lib.rs:1886-1893`) inserts
    /// into [`kayfabe_core::gpu::Vas::guest_ram_pins`] and **never calls `table.bind` and
    /// never sets `Binding::host`** — so a guest-RAM range that IS mapped in the host VAS, at
    /// the guest's own VA, by a real `OS_DESCRIPTOR`, is **invisible to `Binding::host`**.
    ///
    /// ⇒ ★★★★★ The host mapping state lives in **two disjoint places**, and an instrument
    /// reading one of them answers a confident wrong zero. That is
    /// `a_second_source_of_truth_beside_a_complete_value` in the one row a story turns on —
    /// and it is exactly the gap the owner names from the C, which carried **one** offset in
    /// its object structure for this.
    ///
    /// ⇒ This row therefore prints **both**: `host_rows`/runs from `Binding::host`, **and**
    /// `pins=` from `guest_ram_pins`. ⚠ A reader must join them; neither alone is the host
    /// VAS. ⊘ They are NOT merged into one number here, because a sum would hide which
    /// record a range lives in.
    ///
    /// # ⊘⊘ CORRECTED 2026-08-14 (w310) — **"THE PIN MAP IS RECLAIMED SEPARATELY" WAS FALSE**
    ///
    /// This paragraph used to justify keeping the two numbers apart by saying they *"have
    /// different lifetimes (`stage_dropped_vases` reclaims the first; the pin map is
    /// reclaimed **separately**)"*. There was no separate reclaim. `Vas::guest_ram_pins` had
    /// **no `remove`, `retain`, `clear` or `drain` anywhere in the tree**
    /// (`docs/audits/w301_cancellation_error_leaks.md` §3.2), so the map was dropped with its
    /// `Vas` and its handles were lost. ⇒ the sentence read as a design decision and was in
    /// fact a **description of the leak**, which is why nobody chasing it started here.
    /// ★ Same class this tree keeps paying for: *a correct-sounding doc is the last place a
    /// reader looks for a missing mechanism.*
    ///
    /// It is true **now**: `Spine::stage_dropped_vases` reclaims both, the pin first and the
    /// row deduped against it. The conclusion (keep the numbers apart) survives; only its
    /// stated reason changed.
    #[must_use]
    pub fn vas_published_ranges(&self, pid: ProcId, cap: usize) -> Vec<String> {
        self.with_proc_mut(pid, |p| {
            p.vases
                .iter()
                .map(|(&(gpu, pdb), vas)| {
                    let mut runs: Vec<(u64, u64)> = Vec::new();
                    let mut rows = 0usize;
                    for (va, len, b) in vas.table.iter() {
                        if b.host().is_none() {
                            continue;
                        }
                        rows += 1;
                        match runs.last_mut() {
                            Some((s, l)) if *s + *l == va => *l += len,
                            _ => runs.push((va, len)),
                        }
                    }
                    let shown: Vec<String> = runs
                        .iter()
                        .take(cap)
                        .map(|(va, len)| format!("0x{va:x}+0x{len:x}"))
                        .collect();
                    // ★★★★★ THE SECOND RECORD — see this function's 2026-08-13 correction.
                    let pins: Vec<String> = vas
                        .guest_ram_pins
                        .iter()
                        .take(cap)
                        .map(|(va, p)| format!("0x{va:x}+0x{:x}", p.len))
                        .collect();
                    // ★★★★★ **THE MERGE'S OWN FALSIFIER, CHECKED RATHER THAN PRINTED.**
                    //
                    // Two records that must agree, compared against each other — not reported
                    // side by side for a reader to compare. This is the fix for the class that
                    // made `host_rows=4` wrong: a number that looked complete because nothing
                    // was standing beside it. Every guest-RAM pin whose grant matches exactly
                    // one row upgrades that row, so **each such pin contributes exactly one
                    // `host_rows`**.
                    //
                    // ⊘⊘⊘ **CORRECTED 2026-08-14 (w296) — READ THIS BEFORE THE PARAGRAPH
                    // BELOW, WHICH IS TRUE OF THE MECHANISM AND WRONG ABOUT THE VERDICT.**
                    // `[measured, tests/tests/publish_census.rs::
                    // the_merge_equality_can_be_false_and_says_which_pin_no_row_records]`
                    // `MERGE-AGREES=false` is **ALSO THE DESIGNED OUTCOME OF A LEGITIMATE
                    // PIN**, not only of a defect. `commit_pin_guest_ram`'s merge is bounded
                    // to a row whose extent matches the grant EXACTLY
                    // (`kayfabe-fwd/src/lib.rs:1930-1932`), and that bound is deliberate:
                    // one host handle written into N rows would be freed N times by
                    // `Spine::stage_dropped_vases`, a DOUBLE FREE strictly worse than the
                    // leak the merge closes. ⇒ Every **run pin** (legs 4-6) reports `false`
                    // by construction, and a reader who takes `false` as "a bug" on a boot
                    // full of run pins will chase nothing.
                    // ⇒ ★ Read it as *"how many pins are NOT reflected as rows"*, and join it
                    // with `bound_into_table` on the pin's own reply — which is the field
                    // that says whether THIS pin was one of them.
                    //
                    // ⊘ `>=` and not `==`, and the asymmetry is the honest one: `host_rows`
                    // legitimately EXCEEDS `pins` by the framebuffer joins (leg 7/8), which
                    // carry a backing and no pin. What must never happen is a pin with no row
                    // — that is a mapping the field cannot see, which is exactly the defect.
                    // ⇒ `MERGE-AGREES=false` means a pin exists that no row records.
                    let agrees = rows >= vas.guest_ram_pins.len();
                    format!(
                        "[proc={} gpu={} pdb=0x{:x} host_rows={} of {} runs={} {}{} \
                         pins={}=[{}{}] MERGE-AGREES={agrees}]",
                        pid.0,
                        gpu.0,
                        pdb.0,
                        rows,
                        vas.table.iter().count(),
                        runs.len(),
                        shown.join(","),
                        if runs.len() > cap {
                            format!(" ⚠⚠ CAPPED at {cap} of {} runs — INCOMPLETE", runs.len())
                        } else {
                            String::new()
                        },
                        vas.guest_ram_pins.len(),
                        pins.join(","),
                        if vas.guest_ram_pins.len() > cap {
                            " ⚠⚠CAPPED"
                        } else {
                            ""
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// ★★★ **w291 — the GUEST-RAM rows a bounded pin-rate measurement would take**, as
    /// `(va, gpa, len)`, oldest-first and capped.
    ///
    /// ⊘ A **measurement input, not the merge.** It selects rows whose binding is guest RAM,
    /// is not already covered by a live pin, and carries a non-zero length. Nothing here
    /// writes `Binding::host` and nothing changes the table — the caller pins through the
    /// existing `pin_guest_ram` verb, whose existing record is `Vas::guest_ram_pins`.
    ///
    /// ⚠ `Binding::phys` for a guest-RAM row **is a guest-physical address** (that field's own
    /// doc), which is what the VMM's `resolve_guest_ram` needs. It is returned rather than
    /// re-derived so the caller cannot resolve a different address than the one classified.
    #[must_use]
    pub fn vas_guest_ram_rows(
        &self,
        pid: ProcId,
        gpu: GpuId,
        pdb: Pdb,
        cap: usize,
    ) -> Vec<(u64, u64, u64)> {
        self.with_proc_mut(pid, |p| {
            let Some(vas) = p.vases.get(&(gpu, pdb)) else {
                return Vec::new();
            };
            vas.table
                .iter()
                .filter(|(va, len, b)| {
                    *len > 0
                        && b.host().is_none()
                        && b.is_guest_ram()
                        && !vas
                            .guest_ram_pins
                            .iter()
                            .any(|(pva, pin)| *pva <= *va && *va + *len <= pva + pin.len)
                })
                .take(cap)
                .map(|(va, len, b)| (va, b.phys(), len))
                .collect()
        })
        .unwrap_or_default()
    }

    /// Every `(gpu, pdb)` this proc holds a `Vas` for — the keys
    /// [`SharedDevice::vas_publish_census`] and the publication pass iterate.
    ///
    /// ⊘ A list rather than a callback because the publication that follows takes the
    /// device's locks again per leaf: holding the proc across the whole pass would put a
    /// round trip to another process under a rank-0 lock.
    #[must_use]
    pub fn vas_keys(&self, pid: ProcId) -> Vec<(GpuId, Pdb)> {
        self.with_proc_mut(pid, |p| p.vases.keys().copied().collect())
            .unwrap_or_default()
    }

    /// ★★★★★ **w318 — THE ARMING EDGE FOR THE PUBLICATION PASS**, read without walking a
    /// single row. See [`kayfabe_core::gpu::Vas::publish_epoch`] for what the two terms are
    /// and what they deliberately do not cover.
    ///
    /// ⊘ `None` means **there is no such `Vas`**, which is a different fact from *"it has
    /// not changed"* and must not be cached as one: a key that appears later has no prior
    /// epoch to compare against, so its first pass runs.
    #[must_use]
    pub fn vas_publish_epoch(&self, pid: ProcId, gpu: GpuId, pdb: Pdb) -> Option<(u64, usize)> {
        self.with_proc_mut(pid, |p| {
            p.vases.get(&(gpu, pdb)).map(kayfabe_core::gpu::Vas::publish_epoch)
        })
        .flatten()
    }

    /// ★★★★★ **WHICH ROWS COULD BE PUBLISHED, AND BY NAME WHY THE REST COULD NOT** — w290's
    /// publication census, taken over one `Vas` before any host verb is issued.
    ///
    /// # ⊘ The gate that is NOT ours, and it is the one that decides the shape of this rung
    ///
    /// `plan_back_fb_leaf` — the proven publication verb — refuses a leaf on **three**
    /// grounds before it asks the host anything, and two of them are RM's, not ours:
    ///
    /// | refusal | source | why it cannot be relaxed here |
    /// |---|---|---|
    /// | `FbLeafGranularity` | `kayfabe-fwd/src/lib.rs:2244-2247` — *"RM places a fixed mapping in 64 KiB granules"* | it is RM's placement granularity; a 4 KiB row **cannot** be covered exactly |
    /// | `FbLeafDisagrees` | `:2360-2366` | a row whose aperture is not `Vidmem` has no framebuffer to join |
    /// | `FbLeafExtent` | `:2352-2358` | the leaf must be **exactly one table row**, start and length |
    ///
    /// ⇒ ★★★ **THE BRIEF'S "publish extents, coalesce by RUN" IS BLOCKED BY THE THIRD ROW,
    /// AND IS REQUIRED BY THE FIRST.** A 4 MiB run is a whole number of 64 KiB granules and
    /// would sail through the granularity gate; the 4 KiB rows it is made of cannot. But
    /// `FbLeafExtent` refuses any request that is not exactly one row, so the proven verb
    /// **cannot be handed a run**. This census reports both facts as counts rather than
    /// guessing which dominates: `not_granular` is how many rows the run-coalescing would
    /// have rescued, and it is measured, not estimated.
    ///
    /// ⊘ `guest_ram` rows are counted and **excluded, not refused**: they are leg 6's
    /// population and are served by the guest-RAM pin, which is a different verb with a
    /// different lifetime. Publishing them here would be a second establishment over pages
    /// that already have one — the `0x51` collision that cannot be told from exhaustion.
    #[must_use]
    pub fn vas_publish_census(
        &self,
        pid: ProcId,
        gpu: GpuId,
        pdb: Pdb,
        cap: usize,
    ) -> PublishCensus {
        self.with_proc_mut(pid, |p| {
            let mut c = PublishCensus::default();
            let Some(vas) = p.vases.get(&(gpu, pdb)) else {
                return c;
            };
            for (va, len, b) in vas.table.iter() {
                c.total += 1;
                if b.host().is_some() {
                    c.already_host += 1;
                } else if vas
                    .guest_ram_pins
                    .iter()
                    .any(|(pva, p)| *pva <= va && va + len <= pva + p.len)
                {
                    // ★★★★★ THE SECOND RECORD, counted separately — see
                    // `vas_published_ranges`' 2026-08-13 correction. A row covered by a live
                    // guest-RAM pin IS mapped in the host VAS at the guest's own VA; it just
                    // is not recorded in `Binding::host`. Folding it into `guest_ram` would
                    // report a mapped range as unmapped, which is the whole defect.
                    c.already_pinned += 1;
                } else if b.is_guest_ram() {
                    c.guest_ram += 1;
                } else if b.aperture() != kayfabe_arch::Aperture::Vidmem {
                    c.not_vidmem += 1;
                } else if len < FB_LEAF_GRANULE
                    || len % FB_LEAF_GRANULE != 0
                    || va % FB_LEAF_GRANULE != 0
                {
                    c.not_granular += 1;
                    c.not_granular_bytes += len;
                } else {
                    c.candidate_bytes += len;
                    if c.candidates.len() < cap {
                        c.candidates.push((va, len, b.phys()));
                    } else {
                        c.capped += 1;
                    }
                }
            }
            c
        })
        .unwrap_or_default()
    }

    /// ★★★★★ **w329 leg 2 — SUPERSEDE the stale join of a recycled framebuffer frame.**
    ///
    /// # ⊘⊘⊘ WHY THIS EXISTS: THE TRIGGER THE SOURCE NOMINATES IS NOT THE EVENT THAT OCCURS
    ///
    /// `SharedDoorbell::join_operand_fb_leaves`' cleanup table names the ending event as *"the
    /// guest's own free/unmap of the range, seen as the page-table leaf ceasing to bind"*, and
    /// `PublishedUnbind::RevokeWholeJoins` wires exactly that. `[measured 2026-08-15, boot
    /// `w329a1`]` **it fires eight times in a whole `28,31` run and the failure survives**,
    /// because CUDA's suballocator **does not unmap on `cuMemFree`**: the boot's own
    /// `GUEST-DESCRIBES` census ends with **one 140 MiB run**, `0x7af90e000000+0x8c00000`,
    /// which contains the freed buffer *and* the new one. The guest re-points the **physical**
    /// frame into a new VA and leaves the old VA's PTE naming it.
    ///
    /// ⇒ The event that actually occurs is **a NEW leaf naming a framebuffer frame we already
    /// joined for a DIFFERENT VA in the same address space** — an alias the guest created and
    /// only one half of which it will ever use again.
    ///
    /// # ★★★ WHAT IS SAFE HERE, AND WHAT IS NOT — stated, because this is the risky half
    ///
    /// - **The ownership argument is unchanged** and is
    ///   [`SharedDevice::revoke_published_fb_leaf`]'s, verbatim: the row selected is the only
    ///   one that names the object (`frees_object()`, `JoinsGuestWindow`, exact extent), and
    ///   removing it is what creates the obligation the caller then discharges.
    /// - ⊘ **Scoped to ONE address space.** A join owned by another `Vas` is left alone and
    ///   the old refusal stands: another proc's row is another isolate's object, and *"the
    ///   guest re-pointed it"* is not a statement anyone can make across that boundary.
    /// - ⚠ **What is NOT proven: that the old VA is dead.** The guest describes both. The
    ///   device can serve only one — one frame carries one join — so today it serves the OLD
    ///   VA and starves the new one, and this makes it serve the NEW one and starve the old.
    ///   **Neither is correct in general.** The newest is chosen for the reason
    ///   `ReachShadow::settle` already chooses it for shape collisions: the guest's most recent
    ///   page-table write is its most recent statement about what that frame is for. An engine
    ///   still pointed at the old VA takes a **contained** GPU fault, which is the map/revoke
    ///   asymmetry's cheap side.
    ///
    /// Returns the row that was removed, or `None` when no qualifying row in this address
    /// space names `phys` — which the caller must read as *"do not release anything"*.
    pub fn supersede_joined_fb_leaf(
        &self,
        gpu: GpuId,
        pdb: Pdb,
        phys: u64,
        keep_va: GpuVa,
    ) -> Option<kayfabe_fwd::RevokedLeaf> {
        let pid = self
            .route_act(
                |spine| Ok((kayfabe_fwd::route_pdb(spine, gpu, pdb)?, ())),
                |_spine, proc, ()| proc.id,
            )
            .ok()?;
        self.with_proc_mut(pid, |p| {
            let vas = p.vases.get_mut(&(gpu, pdb))?;
            // ⊘ A scan of THIS address space's own rows for a framebuffer offset, not a
            // reverse resolution of a host address to a guest VA: the join is keyed by
            // `phys` at every other site too (`FbStore::install_join`,
            // `FbStore::release_join`), so this asks the table the same question in the same
            // key. `vas_publish_census` walks the identical iterator every doorbell.
            let hit = vas.table.iter().find_map(|(va, len, b)| {
                let h = b.host()?;
                (b.phys() == phys
                    && va != keep_va.0
                    && h.frees_object()
                    && h.bytes() == kayfabe_mmu::BackingBytes::JoinsGuestWindow)
                    .then_some((va, len, h.host_va(), h.memory()))
            })?;
            let (va, len, host_va, memory) = hit;
            vas.table.unbind(GpuVa(va));
            // ★★★ AND THE SHADOW IS TOLD. A table without the row and a shadow that still
            // claims it is a hole: the next `settle` would compare `published == desired`,
            // propose nothing, and the VA would resolve `Miss` forever. Telling it makes the
            // next pass propose the bind again — as a row with no host object, which is the
            // truth.
            vas.reach.confirm_unbind(GpuVa(va));
            Some(kayfabe_fwd::RevokedLeaf {
                gpu,
                pdb,
                va: GpuVa(va),
                len,
                phys,
                host_va,
                memory,
            })
        })
        .flatten()
    }

    /// ★★★★★ **w363 — EVERY FRAMEBUFFER JOIN A *RETIRED* PROC STILL OWNS.**
    ///
    /// # The defect this exists for, measured
    ///
    /// `[measured w361/w362, 2026-08-20, real GA106]` the **first** CUDA process in a boot
    /// gets its GR context framebuffer leaves joined and **8/8 completions**; **every later
    /// one** gets `THE INSTALL REFUSED … already joined` on the same three frames
    /// (`0x400000` `SET_VALID_SPAN_OVERFLOW_AREA`, `0x600000` `SET_TEX_HEADER_POOL`,
    /// `0x800000` `SET_TEX_SAMPLER_POOL`), **zero** joins, and therefore **0/8** completions
    /// — libcuda then spins forever. The effect is ORDINAL: the predecessor's *kind* is
    /// irrelevant (a second `torch` with nothing between it and the first fails identically).
    ///
    /// # Why the existing takeover cannot reach it
    ///
    /// [`SharedDevice::supersede_joined_fb_leaf`] routes by the **caller's own** `(gpu, pdb)`
    /// and searches only `p.vases[&(gpu, pdb)]`. A join left behind by a **different,
    /// exited** process lives in a different VAS under a different PDB, so the takeover path
    /// **cannot reach it by construction** — no amount of arming helps.
    ///
    /// # Why release-on-death and not a cross-process takeover
    ///
    /// Two guest processes do not have to trust each other, so reaching into a **live**
    /// peer's table to steal its backing is exactly the cross-process leakage this design
    /// forbids. A **dead** proc is different: nothing can still be reading through it, and
    /// real RM does precisely this at `fd` close — which is why the Mode-1 sibling has no
    /// analog of this bug. It **delegates** lifecycle to RM; Mode 2 **models** it, so Mode 2
    /// owes the implicit free.
    ///
    /// ⊘ **Read-only, and that is sufficient.** The rows are not unbound here: the caller
    /// runs this immediately before the reap that **drops** these procs, so their tables die
    /// with them. What the caller must still do, in this order, is
    /// [`kayfabe_device::plane::RegPlane::release_fb_join`] (the guest's view stops being
    /// served out of the join) and *then*
    /// [`SharedDevice::revoke_published_fb_leaf`] + [`SharedDevice::drain_pending_releases`]
    /// (the host half). Guest view first is the same ordering the supersede path states.
    #[must_use]
    pub fn retired_fb_joins(&self) -> Vec<kayfabe_fwd::RevokedLeaf> {
        self.retired_fb_join_census().1
    }

    /// ★★ The same sweep, but it also answers **how many corpses it looked at** — so a caller
    /// can tell *"nothing was retired"* from *"retired procs held no join rows"*. A count that
    /// conflates those two is the failure class this campaign has now paid for three times in
    /// one day (a global print cap, an append-only watch list, and this).
    #[must_use]
    pub fn retired_fb_join_census(&self) -> (usize, Vec<kayfabe_fwd::RevokedLeaf>) {
        self.with_retired(|corpses| {
            let n = corpses.len();
            let mut out = Vec::new();
            for p in corpses {
                for ((gpu, pdb), vas) in p.vases.iter() {
                    for (va, len, b) in vas.table.iter() {
                        let Some(h) = b.host() else { continue };
                        if h.frees_object()
                            && h.bytes() == kayfabe_mmu::BackingBytes::JoinsGuestWindow
                        {
                            out.push(kayfabe_fwd::RevokedLeaf {
                                gpu: *gpu,
                                pdb: *pdb,
                                va: GpuVa(va),
                                len,
                                phys: b.phys(),
                                host_va: h.host_va(),
                                memory: h.memory(),
                            });
                        }
                    }
                }
            }
            (n, out)
        })
    }

    /// ★★★★★ **w364 — WHO, IF ANYONE, STILL NAMES THIS FRAMEBUFFER FRAME?**
    ///
    /// Returns `(live_rows, retired_rows)`: how many `JoinsGuestWindow` rows across **live**
    /// procs' address tables, and across **retired-but-unreaped** ones, name `phys`.
    ///
    /// # What it is for
    ///
    /// `[measured w361–w363, real GA106]` the framebuffer store can hold a join at `phys`
    /// that **no table row names at all** — the owning proc exited, and
    /// `drain_retired_budgeted` unbound its rows before anything gave the store's half back.
    /// The next process's `install_join` then refuses `ALREADY_JOINED` forever
    /// (`refused=48 joined=1` per frame, three frames, every boot), its GR context never gets
    /// host backing, and its completions never land: `0/8` against the first proc's `8/8`.
    ///
    /// # Why this predicate and not a cross-process takeover
    ///
    /// `live_rows == 0` is the **proof of orphanhood**. A join nobody names cannot be read
    /// through by anybody, so reclaiming it takes nothing from anyone — no trust argument
    /// between mutually-untrusting guest processes is required, which is exactly what a
    /// takeover that reached into a **live** peer's table would need and could not have.
    /// ⊘ `live_rows > 0` is a genuine conflict between two living processes over one frame
    /// and must stay refused **by name**; it is not this function's business to resolve.
    ///
    /// ⚠ **Both halves are returned because a single number cannot separate the two cases.**
    /// `(0, 0)` is an orphan; `(0, n)` is a corpse whose rows are still standing and whose
    /// join the retired sweep should take; `(n, _)` is live and must be refused. Collapsing
    /// these into one count is the failure class this campaign paid for three times in one
    /// day — most recently in the fix for it.
    #[must_use]
    pub fn fb_join_namers(&self, phys: u64) -> (usize, usize) {
        let mut live = 0usize;
        for pid in self.live_pids() {
            self.with_proc(pid, |p| {
                for vas in p.vases.values() {
                    for (_va, _len, b) in vas.table.iter() {
                        if b.phys() == phys
                            && b.host().is_some_and(|h| {
                                h.frees_object()
                                    && h.bytes()
                                        == kayfabe_mmu::BackingBytes::JoinsGuestWindow
                            })
                        {
                            live += 1;
                        }
                    }
                }
            });
        }
        let retired = self
            .retired_fb_join_census()
            .1
            .iter()
            .filter(|r| r.phys == phys)
            .count();
        (live, retired)
    }

    /// ★★★★★ **THE PARKED PROMOTE HALVES, BY IDENTITY** — every entry of
    /// [`kayfabe_core::gpu::Vas::promote_halves`] rendered with its `buffer_id`, which half
    /// arrived, and the address it carries.
    ///
    /// # ⊘ A TALLY CANNOT ANSWER THE QUESTION THE PARK IS READ FOR
    ///
    /// The shipped instruments print `parked=N` and `orphans(awaiting_va=A,awaiting_phys=B)`
    /// — three counts. `[measured, boot w289cup2]` cup2's own VAS ended at
    /// `orphans(awaiting_va=0,awaiting_phys=9)`: **nine context buffers declared at a VA with
    /// nothing bound behind them**, and hardware faulted at a VA the log could not test against
    /// any of them, because not one of the nine addresses was printed. ⇒ *"does a parked half
    /// cover the faulting VA"* — the only question a park is ever read to answer — was
    /// unanswerable from a complete, healthy-looking log. Same class as
    /// `a_count_cannot_see_a_substitution`.
    ///
    /// ★ The **known-positive rides the same instrument**: a `buffer_id` that JOINED is absent
    /// from this list and present in `bound=[…]`, which is [`kayfabe_core::gpu::Vas::promote_bound`].
    /// Both are printed, so *"everything is parked"* is distinguishable from *"the enumerator
    /// only knows how to print parks"* — the failure that killed `GET_PTE_INFO` this week.
    /// `[measured, boot w289cup2]` that boot reached `CUMULATIVE … joined=4 joined_global=1`, so
    /// a non-empty `bound=[…]` is a **reading this boot can produce**, not a hypothetical.
    ///
    /// ⚠ Uncapped by construction: the key is `NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_*`, a
    /// fixed RM enum with thirteen members, so the row count is bounded by the ABI and not by
    /// guest behaviour. Nothing here can truncate and read as complete.
    #[must_use]
    pub fn vas_promote_halves(&self, pid: ProcId) -> Vec<String> {
        self.with_proc_mut(pid, |p| {
            p.vases
                .iter()
                .map(|(&(gpu, pdb), vas)| {
                    let halves: Vec<String> = vas
                        .promote_halves
                        .iter()
                        .map(|(bid, h)| match h {
                            kayfabe_core::promote::ParkedHalf::AwaitingPhysical { va } => {
                                format!("{{bid={bid:#x} AwaitingPhysical va=0x{:x}}}", va.0)
                            }
                            kayfabe_core::promote::ParkedHalf::AwaitingVa {
                                phys,
                                len,
                                aperture,
                            } => format!(
                                "{{bid={bid:#x} AwaitingVa phys=0x{phys:x} len=0x{len:x} \
                                 ap={aperture:?}}}"
                            ),
                        })
                        .collect();
                    let bound: Vec<String> = vas
                        .promote_bound
                        .iter()
                        .map(|va| format!("0x{va:x}"))
                        .collect();
                    format!(
                        "[proc={} gpu={} pdb=0x{:x} parked={} {} bound={}=[{}]]",
                        pid.0,
                        gpu.0,
                        pdb.0,
                        halves.len(),
                        halves.join(" "),
                        bound.len(),
                        bound.join(","),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// ★★★ **ARM 2.1 — WHAT THE GUEST ITSELF SAYS ABOUT A VIRTUAL ADDRESS**, asked of the
    /// swept picture of every one of `pid`'s address spaces.
    ///
    /// # ⊘ The question this exists to make answerable, and why it can kill a story
    ///
    /// The wall's host-side `Xid 31` names a faulting VA. Two readings are consistent with it
    /// and they demand opposite work:
    ///
    /// - **the guest describes that VA and our mirror missed it** ⇒ the sweep is the fix;
    /// - **the guest never described it** ⇒ mirroring cannot bind what does not exist, and the
    ///   sweep story is dead.
    ///
    /// A bind census cannot separate them: it reports OUR table, and both readings produce the
    /// same absence in it. This reports the **guest's own tables**, as walked from the guest's
    /// own installed page-directory root — so an absence here is a statement about the guest.
    ///
    /// ⚠ It is only as complete as the sweep that fed the shadow. A `truncated` sweep produced
    /// **no** leaves, so "absent" from a truncated picture is not evidence of anything; the
    /// caller must print the sweep's own state beside this. ⊘ And it reads the shadow rather
    /// than re-walking, deliberately: a second walk is a second reader that can disagree with
    /// the one the port acted on.
    #[must_use]
    pub fn guest_leaf_census(&self, pid: ProcId, va: kayfabe_arch::ids::GpuVa) -> String {
        let Some(rows) = self.with_proc_mut(pid, |p| {
            let mut rows: Vec<String> = Vec::new();
            for (&(gpu, pdb), vas) in &p.vases {
                let hit = vas.reach.leaf_covering(va);
                rows.push(format!(
                    "gpu={} pdb=0x{:x} sweeps={} trunc={} dirty={} pages={} swept_only={} → {}",
                    gpu.0,
                    pdb.0,
                    vas.sweep.sweeps,
                    u8::from(vas.sweep.truncated),
                    u8::from(vas.sweep.dirty),
                    vas.reach.len(),
                    vas.reach.swept_only_len(),
                    match hit {
                        Some(l) => format!(
                            "LEAF-PRESENT va=0x{:x} size={:?} phys=0x{:x} ap={:?} ro={}",
                            l.va.0, l.size, l.phys, l.aperture, l.read_only
                        ),
                        None => "LEAF-ABSENT (this address space's own tables, as swept, \
                                 describe no mapping covering it)"
                            .to_string(),
                    }
                ));
            }
            rows
        }) else {
            return format!("GUEST-LEAF va=0x{:x} → NO SUCH PROC", va.0);
        };
        if rows.is_empty() {
            return format!(
                "GUEST-LEAF va=0x{:x} proc={} → NO ADDRESS SPACES. ⊘ That is not LEAF-ABSENT: \
                 nothing was asked.",
                va.0, pid.0
            );
        }
        format!(
            "GUEST-LEAF va=0x{:x} proc={} over {} address space(s): {}",
            va.0,
            pid.0,
            rows.len(),
            rows.join(" | ")
        )
    }

    /// ★★★★★ **§16.82 — WHY one VA is not bound, asked of the VAS that would have to bind
    /// it, on the same line as the key.**
    ///
    /// # ⊘ The question this exists to make un-guessable
    ///
    /// `[measured 2026-08-11, boot `w232c_6fcedac`]` eight doorbells read
    /// `RING-VA-UNBOUND va=0x200224000 → NOTHING FORWARDED` while, **in the same string**,
    /// the published-root descent resolved that exact VA (`rng=V:0x1024000`) and read its
    /// bytes. Two resolvers, one address, opposite answers — and nothing printed said which
    /// of the four possible causes it was:
    ///
    /// | cause | the field that shows it |
    /// |---|---|
    /// | the VAS does not exist for this `(gpu, pdb)` | `vas=ABSENT` |
    /// | it exists and **nothing was ever decoded into it** | `rows=0 shadow=0 wit=0 meta=0` |
    /// | it was decoded and this page was **never witnessed** | `root_wit=N` with `wit>0` |
    /// | it is bound, and the *reader* refused it | `hit=0x…/<aperture>` |
    ///
    /// ⊘ **It computes no verdict and prints no adjective.** Every field is a structure's own
    /// answer about itself; the reader does the classifying. That is deliberate — `RingLook`'s
    /// own doc states the rule (*"the tag is the variant, never a derived judgement"*), and
    /// this rung exists because a *name* (`RING-VA-UNBOUND`) was read as a diagnosis.
    ///
    /// ★★ **`wit_sample` is the negative control, and it is why the line is trustworthy.**
    /// `root_wit=N` printed alone cannot be told from a predicate that is incapable of
    /// answering `Y`. The sample comes out of the **same** `BTreeSet` the predicate reads, so
    /// a non-empty sample beside a `N` is the set saying *no, about this page* — the fail-arm
    /// returning the other direction's pattern rather than zeros.
    ///
    /// # ⚠ Locks
    ///
    /// One [`SharedDevice::route_act`]: device-read (rank 0) and this proc's mutex (rank 1),
    /// in that order, both released on return. `Spine::pt_page_owner` is asked **inside** the
    /// same acquisition, so the ownership answer and the `Vas`'s own state cannot come from
    /// two different instants — the join rule §16.71 states.
    ///
    /// Returns a `vas=NO-PROC` line rather than `None` when `pid` is not live: a pass that
    /// found nothing and a pass that did not run are different facts about the boot.
    #[must_use]
    pub fn vas_bind_census(&self, pid: ProcId, gpu: GpuId, pdb: Pdb, va: GpuVa) -> String {
        /// How many witnessed pages to name. Enough to prove the set is non-empty and to
        /// let a reader recognise the family; not a dump of a guest-sized structure.
        const SAMPLE: usize = 4;
        let root = pdb.0 & !0xfff;
        // ★★★★★ **R1 — NOTHING IS FORMATTED UNDER THE LOCK.** The closure returns plain
        // `Copy` scalars and one fixed-size array; every `format!` runs after the guard is
        // dropped.
        //
        // ⊘ **This was not the shape it was written in**, and the correction is the point:
        // it built its `String` *inside* `with_proc`, i.e. it ran a heap allocation beneath
        // a rank-1 guard on the vCPU's own MMIO trap path. `[measured 2026-08-11, r33]` the
        // sibling instance of exactly that — a guard alive inside an `eprintln!` — was a
        // real R1 violation, not a lock awaiting a ruling, because the *global stderr lock*
        // and an allocation both ran under it. ⚠ And this tree's lock witness masks only
        // **ranked** locks, so the process-global ones are invisible to it: the assertion
        // that would have caught this does not exist and cannot be relied on.
        //
        // ★ The rule this encodes, general and cheap: **gather scalars under the lock,
        // render outside it.** It costs one struct and removes a whole class.
        struct Row {
            present: bool,
            rows: usize,
            hit: Option<(u64, kayfabe_arch::Aperture, u64, u64)>,
            root_dirty: bool,
            root_wit: bool,
            root_meta: bool,
            dirty: usize,
            meta: usize,
            shadow: usize,
            wit: usize,
            published: usize,
            wit_sample: [Option<u64>; SAMPLE],
            proc_vases: usize,
            vas_sample: [Option<(u32, u64)>; SAMPLE],
        }
        let out = self.with_proc(pid, |p| {
            let mut r = Row {
                present: false,
                rows: 0,
                hit: None,
                root_dirty: false,
                root_wit: false,
                root_meta: false,
                dirty: 0,
                meta: 0,
                shadow: 0,
                wit: 0,
                published: 0,
                wit_sample: [None; SAMPLE],
                proc_vases: p.vases.len(),
                vas_sample: [None; SAMPLE],
            };
            let Some(vas) = p.vases.get(&(gpu, pdb)) else {
                // ⊘ Which address spaces this proc DOES have, because "absent" without the
                // population it is absent from is the `an_absence_from_a_filtered_view` shape.
                for (slot, (g, d)) in r.vas_sample.iter_mut().zip(p.vases.keys()) {
                    *slot = Some((g.0, d.0));
                }
                return r;
            };
            r.present = true;
            r.rows = vas.table.iter().count();
            r.hit = vas.table.binding_at(va).map(|(start, len, b)| {
                (
                    b.phys().wrapping_add(va.0 - start),
                    b.aperture(),
                    start,
                    len,
                )
            });
            r.root_dirty = vas.pt_pages.contains(&root);
            r.root_wit = vas.reach.is_witnessed(root);
            r.root_meta = vas.pt_meta.contains_key(&root);
            r.dirty = vas.pt_pages.len();
            r.meta = vas.pt_meta.len();
            r.shadow = vas.reach.len();
            r.wit = vas.reach.witnessed_len();
            r.published = vas.reach.published_len();
            for (slot, page) in r
                .wit_sample
                .iter_mut()
                .zip(vas.reach.witnessed_sample(SAMPLE))
            {
                *slot = Some(page);
            }
            r
        });
        // ---- RENDER, with no guard held ------------------------------------------------
        let list = |xs: &[Option<u64>]| {
            xs.iter()
                .flatten()
                .map(|v| format!("0x{v:x}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let out = out.map(|r| {
            if !r.present {
                let sample = r
                    .vas_sample
                    .iter()
                    .flatten()
                    .map(|(g, d)| format!("g{g}:0x{d:x}"))
                    .collect::<Vec<_>>()
                    .join(",");
                return format!(" vas=ABSENT proc_vases={} sample=[{sample}]", r.proc_vases);
            }
            let hit = r.hit.map_or_else(
                || "NONE".to_string(),
                |(phys, ap, start, len)| format!("0x{phys:x}/{ap:?}/start0x{start:x}/len0x{len:x}"),
            );
            format!(
                " vas=PRESENT rows={} hit={hit} root=0x{root:x} root_dirty={} root_wit={} \
                 root_meta={} dirty={} meta={} shadow={} wit={} published={} \
                 wit_sample=[{}]",
                r.rows,
                yn(r.root_dirty),
                yn(r.root_wit),
                yn(r.root_meta),
                r.dirty,
                r.meta,
                r.shadow,
                r.wit,
                r.published,
                list(&r.wit_sample),
            )
        });
        // ⊘ The owner index is device-global (rank 0) and is asked in its own acquisition
        // **after** the proc lock has been released — never beneath it.
        // ⊘ R1 again: the guard's scope ends with the `let`, and the `format!` is outside
        // it. Same rule as the block above, applied to the rank-0 read.
        let owner_ids = {
            let g = self.state.read();
            g.spine.pt_page_owner(gpu, root)
        };
        let owner = owner_ids.map_or_else(
            || "NONE".to_string(),
            |(o, d)| format!("p{}:0x{:x}", o.0, d.0),
        );
        match out {
            Some(s) => format!(
                "VAS-BIND-CENSUS proc={} gpu={} pdb=0x{:x} va=0x{:x}{s} root_owner={owner}",
                pid.0, gpu.0, pdb.0, va.0
            ),
            None => format!(
                "VAS-BIND-CENSUS proc={} gpu={} pdb=0x{:x} va=0x{:x} vas=NO-PROC \
                 (the proc is retired or never existed) root_owner={owner}",
                pid.0, gpu.0, pdb.0, va.0
            ),
        }
    }

    /// ★★★★ **G1's consumer side — attribute the pages the guest's CPU wrote, and latch
    /// each one onto the `Vas` that owns it.**
    ///
    /// This is `kayfabe_fwd::latch_pt_writes` for the **other transport**. The CE path
    /// arrives with an owner already resolved (`CeOperands::PhysOperand` asked
    /// `classify_ce` under the same lock as the parse); a CPU window write arrives as a
    /// bare framebuffer address, so ownership is asked here —
    /// [`kayfabe_core::gpu::Spine::pt_page_owner`], the same declared-then-discovered index
    /// the CE path consults.
    ///
    /// # ⊘ An unattributed page is RETURNED, never dropped
    ///
    /// A page the index cannot name an owner for is not a page that was not written. It is
    /// most often a page-table page whose parent has not been decoded yet — the index knows
    /// **roots** by declaration and everything deeper only after a decode published it — so
    /// the honest answer is *"not yet"*, and [`CpuPtWitness::unattributed`] carries it back
    /// for the caller to requeue. Dropping it would destroy the witness
    /// (`reachability_on_transition.md` §2.2: a leaf binds only if the guest was **seen** to
    /// write its page) and the page would then never bind, which is exactly the standing
    /// residue this rung exists to close.
    ///
    /// # The locks
    ///
    /// Rank 0 for the whole attribution (one read guard, released before anything else),
    /// then **one proc lock at a time** for the latch — R3, and the same shape
    /// `SharedDevice::parse_pushbuffer`'s latch phase uses, because the owner of a written
    /// page is routinely not the proc that is submitting.
    pub fn witness_cpu_pt_pages(&self, gpu: GpuId, pages: &[u64]) -> CpuPtWitness {
        let mut out = CpuPtWitness::default();
        // ROUTE — rank 0, and released before any proc lock is taken.
        let owned: Vec<(u64, ProcId, Pdb)> = {
            let g = self.state.read();
            pages
                .iter()
                .filter_map(|&p| {
                    let page = p & !0xfff;
                    g.spine
                        .pt_page_owner(gpu, page)
                        .map(|(pid, pdb)| (page, pid, pdb))
                })
                .collect()
        };
        let claimed: std::collections::BTreeSet<u64> = owned.iter().map(|&(p, ..)| p).collect();
        // ⊘ Through a set, not a `dedup()`: `dedup` only collapses *adjacent* equals, so an
        // unsorted input would requeue the same page twice and the count a report prints
        // would be of rows rather than of pages.
        out.unattributed = pages
            .iter()
            .map(|&p| p & !0xfff)
            .filter(|p| !claimed.contains(p))
            .collect::<std::collections::BTreeSet<u64>>()
            .into_iter()
            .collect();
        // LATCH — grouped by owner so each proc's rank-1 lock is taken once, never twice
        // and never two at a time.
        let mut by_proc: std::collections::BTreeMap<ProcId, Vec<(Pdb, u64)>> =
            std::collections::BTreeMap::new();
        for (page, pid, pdb) in owned {
            by_proc.entry(pid).or_default().push((pdb, page));
        }
        for (pid, rows) in by_proc {
            let rows_len = rows.len();
            let latched = self.with_proc_mut(pid, |p| {
                let mut n = 0;
                for (pdb, page) in rows {
                    // ⊘ The `Vas` may have gone between the index read and this lock (R5).
                    // A page whose address space died is dropped here rather than
                    // re-attached to whatever inherited the PDB — the C's never-pruned-table
                    // aliasing class, refused the same way `latch_pt_writes` refuses it.
                    if let Some(vas) = p.vases.get_mut(&(gpu, pdb)) {
                        vas.pt_pages.insert(page);
                        n += 1;
                    }
                }
                n
            });
            // ⊘ Counted per PAGE, not per proc: "attributed but not latched" is a different
            // fact from `unattributed` (which is *"the index does not know this page"*), and
            // a per-proc tally would report one for a proc that lost 400 of 401 `Vas`es.
            let n = latched.unwrap_or(0);
            out.latched += n;
            out.vas_gone += rows_len - n;
            if n > 0 {
                out.procs.push(pid);
            }
        }
        out
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

    /// ★★★★★ **Pin the GUEST's own pages at `va` in the `(gpu, pdb)` VAS.** Same three
    /// phases as [`SharedDevice::publish_backing`], and it is deliberately a sibling of it
    /// rather than a mode of it — see [`kayfabe_isolate::VerbPlan::PinGuestRam`].
    ///
    /// ⊘ `grant` is the **VMM's** instruction and is carried through untouched. Nothing in
    /// this crate or below derives it, checks it, or clamps it; the only party that can is
    /// the one holding the hypervisor's stated layout.
    ///
    /// # Errors
    /// Whatever [`kayfabe_fwd::plan_pin_guest_ram`] refuses with, or the host's own
    /// refusal — including [`kayfabe_isolate::RmError::PlacementRefused`] when the fixed
    /// map did not land where it was asked to.
    pub fn pin_guest_ram(
        &self,
        gpu: GpuId,
        pdb: Pdb,
        va: GpuVa,
        grant: kayfabe_isolate::GuestRamGrant,
    ) -> Result<kayfabe_fwd::GuestRamPinned, FwdFault> {
        self.verb_op(
            || {
                self.route_act(
                    |spine| Ok((kayfabe_fwd::route_pdb(spine, gpu, pdb)?, ())),
                    |_spine, proc, ()| {
                        let planned = kayfabe_fwd::plan_pin_guest_ram(proc, gpu, pdb, va, grant)?;
                        Staged::check_out(proc, gpu, planned)
                    },
                )?
            },
            |_spine, proc, plan, reply| kayfabe_fwd::commit_pin_guest_ram(proc, plan, reply),
        )
    }

    /// ★★★★★ **THE SECOND CROSSING — back one framebuffer leaf with real host vidmem.**
    /// Same three phases as [`SharedDevice::publish_backing`], and a sibling of it for the
    /// reason [`kayfabe_isolate::VerbPlan::PublishVidmem`] gives.
    ///
    /// `(va, len, phys)` are the guest's OWN page-table walk's answer for this leaf, and
    /// they are carried whole rather than re-derived: `phys` exists here so the commit can
    /// check the walk against the address table and refuse the disagreement by name
    /// ([`kayfabe_fwd::FwdFault::FbLeafDisagrees`]).
    ///
    /// # Errors
    /// Whatever [`kayfabe_fwd::plan_back_fb_leaf`] refuses with, or the host's own refusal
    /// — including [`kayfabe_isolate::RmError::PlacementRefused`] when the fixed map did
    /// not land where it was asked to, and `Rm(NoMemory)` (`0x51`), which ⊘ **is
    /// collision-or-exhaustion and cannot be told apart** — see the C's R2.
    /// ★★★★★ §5.12 — `how` selects the chain, and the two are **alternatives**: see
    /// [`kayfabe_fwd::FbLeafBacking`]. ⊘ Nothing here retries one as the other.
    pub fn back_fb_leaf(
        &self,
        gpu: GpuId,
        pdb: Pdb,
        va: GpuVa,
        len: u64,
        phys: u64,
        how: kayfabe_fwd::FbLeafBacking,
    ) -> Result<kayfabe_fwd::FbLeafBacked, FwdFault> {
        self.verb_op(
            || {
                self.route_act(
                    |spine| Ok((kayfabe_fwd::route_pdb(spine, gpu, pdb)?, ())),
                    |_spine, proc, ()| {
                        let planned =
                            kayfabe_fwd::plan_back_fb_leaf(proc, gpu, pdb, va, len, phys, how)?;
                        Staged::check_out(proc, gpu, planned)
                    },
                )?
            },
            |_spine, proc, plan, reply| kayfabe_fwd::commit_back_fb_leaf(proc, plan, reply),
        )
    }

    /// ★★★★★ **§5.12 — BIND a joined leaf, AFTER its view is installed.** The second half of
    /// [`SharedDevice::back_fb_leaf`]'s `Joined` arm; see [`kayfabe_fwd::adopt_joined_fb_leaf`]
    /// for why the two halves exist and what sits between them.
    ///
    /// ⊘ **Not a [`SharedDevice::verb_op`]**, and for the same reason `fb_join_peek` is not:
    /// it issues no host verb, so there is no plan to stage, no worker to check out and
    /// nothing to execute. What it *does* need is `verb_op`'s disposal discipline on the
    /// refusal path, which it gets explicitly — a refused adopt has host objects nobody else
    /// can name, and dropping them is `§12.35`'s UNACCOUNTED class.
    ///
    /// # ⚠ The caller's obligation, and it is not optional
    ///
    /// `host_va` and `memory` must be the ones [`SharedDevice::back_fb_leaf`] answered with,
    /// carried across the install. A caller that never reaches this call — because the
    /// descriptor did not cross, or the `mmap` failed, or the plane refused the install —
    /// still holds a fixed host mapping at the guest's own VA. It must be released, or the
    /// next ask re-plans as a first join and RM answers the second fixed map with `0x51`.
    ///
    /// # Errors
    /// [`kayfabe_fwd::FwdFault`] — the R5 vocabulary, at this later instant. ★ The refusal's
    /// orphans are STAGED on the proc here, so the caller is told what happened without also
    /// being made responsible for the unmap.
    pub fn adopt_joined_fb_leaf(
        &self,
        gpu: GpuId,
        pdb: Pdb,
        leaf: kayfabe_fwd::FbLeafRange,
        backed: &kayfabe_fwd::FbLeafBacked,
    ) -> Result<(), FwdFault> {
        let kayfabe_fwd::FbLeafRange { va, len, phys } = leaf;
        let (host_va, memory) = (backed.host_va, backed.memory);
        let pid = self.route_act(
            |spine| Ok((kayfabe_fwd::route_pdb(spine, gpu, pdb)?, ())),
            |_spine, proc, ()| proc.id,
        )?;
        let plan = kayfabe_fwd::BackFbLeafPlan {
            proc: pid,
            gpu,
            pdb,
            va,
            len,
            phys,
            // ⊘ Re-read inside the adopt from the `Vas`, never from this field — see the
            // core function. Carried only because the plan type has it.
            host_vas: None,
            existing: None,
            how: kayfabe_fwd::FbLeafBacking::Joined,
        };
        let adopted = self.route_act(
            |_| Ok((pid, ())),
            |_spine, proc, ()| kayfabe_fwd::adopt_joined_fb_leaf(proc, &plan, host_va, memory),
        )?;
        match adopted {
            Ok(()) => Ok(()),
            Err(r) => {
                self.stage_orphans(pid, gpu, r.orphans);
                Err(r.fault)
            }
        }
    }

    /// ★★★★★ **GIVE BACK a join whose view never got installed** — the obligation
    /// [`SharedDevice::adopt_joined_fb_leaf`] names, as a call.
    ///
    /// Between `back_fb_leaf(Joined)` and the adopt there are three places the shell can
    /// fail — the descriptor may not cross, the `mmap` may fail, the plane may refuse the
    /// install — and at every one of them the isolate is holding a **fixed mapping at the
    /// guest's own VA** that no core state names. ⊘ Binding it anyway is not an option: the
    /// row would declare `JoinsGuestWindow` over a window that was never re-pointed, which
    /// is the exact falsehood the two-call split exists to prevent.
    ///
    /// ⇒ So the mapping is **released**, and the next ask re-plans as a first join rather
    /// than colliding with half of one (RM answers a second fixed map at an occupied address
    /// `0x51`, which ⊘ cannot be told apart from real exhaustion).
    ///
    /// ★ The unmap is staged rather than performed: this thread holds no worker, and
    /// [`kayfabe_fwd::checkout_and_drain`] runs the queue on the next verb that does. That is
    /// T0's own mechanism, not a new one.
    ///
    /// ⊘ **Nothing to report.** A proc that has gone in the gap took its isolate with it and
    /// §7.0's process boundary freed the lot; there is no failure this can surface that the
    /// caller could act on.
    pub fn release_unadopted_fb_leaf(
        &self,
        gpu: GpuId,
        pdb: Pdb,
        host_va: u64,
        memory: kayfabe_isolate::HostHandle,
    ) {
        self.stage_fb_leaf_release(gpu, pdb, host_va, memory);
    }

    /// ★★★★★ **w329 — GIVE BACK a join that WAS adopted, because the guest stopped naming it.**
    ///
    /// [`SharedDevice::release_unadopted_fb_leaf`]'s twin, and it is a **second name for one
    /// body on purpose**: the two differ in nothing the host can see and in everything a reader
    /// needs. The unadopted case is *"the crossing failed halfway, undo it"*; this is *"the
    /// crossing succeeded, was used, and the guest has since unmapped the range"* — the
    /// reclamation `w327` measured the absence of, and the case
    /// `kayfabe_mmu::reach::PublishedUnbind::RevokeWholeJoins` produces.
    ///
    /// # ★★★ THE OWNERSHIP ARGUMENT, because this is where a double free would live
    ///
    /// - **Who owns the object at this instant.** Nobody. `apply_settlement_as` removed the one
    ///   `Binding` that named it from the one `AddressTable` that held it, in the same statement
    ///   that produced the row this call is servicing. There is no window in which two parties
    ///   believe they own it, because the table row and the caller's obligation are created and
    ///   destroyed by the same statement.
    /// - **What proves no other row references it.** Four independent facts, each checkable:
    ///   (1) the object was minted per-leaf by `back_fb_leaf(Joined)` and bound as
    ///   `HostBacking::whole`, so `frees_object()` is true and it is **not** an arena slice
    ///   serving siblings at other offsets — the exact predicate `unpublish_backing` gates its
    ///   own `free` on, for the double-free this tree already paid for; (2) `bind_backed_fb_leaf`
    ///   refuses to bind over a row that already carries a host object
    ///   (`GuestRamAddressTaken`), so one object can never be reached from two VAs in one `Vas`;
    ///   (3) `AddressTable::bind` refuses overlaps, so no wider row covers it either;
    ///   (4) `HostHandle` is `(proc, gpu)`-scoped and the table is per-`Vas`, so another address
    ///   space's row for the same framebuffer offset is a *different* object from a *different*
    ///   `back_fb_leaf` call — and `SparseFb::install_join` refuses any second join over the
    ///   same range, so that second object can never have existed.
    /// - **A partial extent.** Cannot arise, and is refused twice over rather than handled:
    ///   `install_join` refuses **any** overlap so a join is always a whole leaf, and
    ///   `apply_settlement_as` revokes only a row whose tabled start equals the proposed VA.
    ///
    /// ⚠ **This call is HALF of the release.** The other half is
    /// `kayfabe_device::plane::RegPlane::release_fb_join`, and the guest's view must go **first**
    /// — a store still serving bytes out of a region this call is about to unmap is a `SIGBUS`
    /// with no other detector.
    pub fn revoke_published_fb_leaf(
        &self,
        gpu: GpuId,
        pdb: Pdb,
        host_va: u64,
        memory: kayfabe_isolate::HostHandle,
    ) {
        self.stage_fb_leaf_release(gpu, pdb, host_va, memory);
    }

    /// The staging body both framebuffer-leaf release verbs share. ⊘ One body rather than two,
    /// because *"which host VAS was this mapped in"* is read from the `Vas` and a second copy of
    /// that read is a second answer that can disagree with the first.
    fn stage_fb_leaf_release(
        &self,
        gpu: GpuId,
        pdb: Pdb,
        host_va: u64,
        memory: kayfabe_isolate::HostHandle,
    ) {
        let Ok(pid) = self.route_act(
            |spine| Ok((kayfabe_fwd::route_pdb(spine, gpu, pdb)?, ())),
            |_spine, proc, ()| proc.id,
        ) else {
            return;
        };
        // The host VAS the mapping lives in — read from the `Vas`, the same authority the
        // adopt reads it from, so an unmap can never name a VAS the map was not made in.
        let host_vas = self.route_act(
            |_| Ok((pid, ())),
            |_spine, proc, ()| proc.vases.get(&(gpu, pdb)).and_then(|v| v.host_vas),
        );
        let orphans = match host_vas {
            Ok(Some(vas)) => kayfabe_isolate::Orphans {
                unmap: vec![(vas, host_va)],
                free: vec![memory],
                guest_ram: Vec::new(),
            },
            // ⊘ No VAS means no mapping to unmap — but the OBJECT still exists and is still
            // ours to free. Freeing it without the unmap is correct here and not a shortcut:
            // RM tears the mapping down with the address space it was made in.
            _ => kayfabe_isolate::Orphans {
                unmap: Vec::new(),
                free: vec![memory],
                guest_ram: Vec::new(),
            },
        };
        self.stage_orphans(pid, gpu, orphans);
    }

    /// ★★★ **The joined-leaf instrument, through the core** — [`kayfabe_isolate::Worker`]'s
    /// `fb_join_peek`, routed to the isolate that owns `(pid, gpu)`.
    ///
    /// ⊘ Not a `VerbPlan`, and it does not go through [`SharedDevice::verb_op`]: it acquires
    /// nothing and names no [`kayfabe_isolate::HostHandle`], so there is no plan to commit,
    /// no orphan set to unwind and no R5 re-validation to do — which is exactly why
    /// [`kayfabe_isolate::Worker::fb_join_peek`] sits beside `execute` rather than inside it.
    /// What it *does* keep is the checkout: a worker of the right isolate, borrowed under the
    /// proc lock and used with **every lock released**, so R1's assertion inside the worker
    /// has something true to assert.
    ///
    /// ⚠ **Pool-full is a refusal here, not a park.** A verb that cannot get a worker must
    /// wait, because dropping it would drop guest work. An instrument that cannot get one has
    /// nothing to lose by saying so, and a diagnostic that blocked the caller on pool
    /// availability would change the timing of the thing it is measuring.
    ///
    /// # Errors
    /// [`FwdFault`] when there is no live isolate to ask or the pool has no idle worker;
    /// otherwise the isolate's own refusal. ⊘ `Ok(false)` — *"no joined leaf covers that
    /// range"* — is **not** an error.
    pub fn fb_join_peek(
        &self,
        pid: ProcId,
        gpu: GpuId,
        phys: u64,
        buf: &mut [u8],
        poke: Option<u32>,
    ) -> Result<bool, FwdFault> {
        let mut worker = self
            .route_act(
                |_| Ok((pid, ())),
                |_, proc, ()| kayfabe_fwd::checkout_and_drain(proc, gpu).map(|(w, _)| w),
            )??
            .ok_or(FwdFault::NoTarget { proc: pid, gpu })?;
        // ---- No lock held. `Worker::fb_join_peek` asserts exactly that. ----
        let out = worker.fb_join_peek(phys, buf, poke);
        self.return_worker(pid, gpu, worker);
        // ⊘ `FwdFault::Rm` gained a second field (`on`) after this branch forked; the
        // instrument names no object, so `None` is the truthful value rather than a
        // placeholder.
        out.map_err(|err| FwdFault::Rm { err, on: None })
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
        err_notifier_grant: Option<kayfabe_isolate::GuestRamGrant>,
    ) -> Result<EngineObjectForwarded, FwdFault> {
        self.verb_op(
            || {
                self.route_act(
                    |spine| {
                        let r = kayfabe_fwd::route_engine_object(spine, target_gpu, vchid, class)?;
                        Ok((r.proc, r))
                    },
                    |spine, proc, route| {
                        // ★★★★★ LEG A2 — the spine is now READ here, for the channel's own
                        // declared `gpFifoOffset`. See `plan_engine_object`.
                        let planned = kayfabe_fwd::plan_engine_object(
                            spine,
                            proc,
                            &route,
                            class,
                            params,
                            // ★★★★★ w288 — the VMM's grant, passed through untouched.
                            err_notifier_grant,
                        )?;
                        Staged::check_out(proc, planned.plan.cgpu, planned)
                    },
                )?
            },
            kayfabe_fwd::commit_engine_object,
        )
    }

    /// ★★★★★ **§16.80** — [`Self::forward_engine_object`] keyed on the handles a
    /// `GSP_RM_ALLOC` carries (`hClient` / `hParent`) instead of on `(GpuId, VChid)`,
    /// which is what a *doorbell* has. Same three phases; only the rank-0 route differs.
    ///
    /// This is the entry point the RPC path uses, and its absence is the whole reason
    /// `forward_engine_object` had no production caller: the Case-1 forward was built
    /// against the doorbell's key and the wire never speaks that key.
    ///
    /// # Errors
    /// [`FwdFault`], by variant.
    pub fn forward_engine_object_by_parent(
        &self,
        client: HClient,
        parent: HObject,
        class: ClassId,
        params: &[u8],
        err_notifier_grant: Option<kayfabe_isolate::GuestRamGrant>,
    ) -> Result<EngineObjectForwarded, FwdFault> {
        self.verb_op(
            || {
                self.route_act(
                    |spine| {
                        let r = kayfabe_fwd::route_engine_object_by_parent(
                            spine, client, parent, class,
                        )?;
                        Ok((r.proc, r))
                    },
                    |spine, proc, route| {
                        // ★★★★★ LEG A2 — the spine is now READ here, for the channel's own
                        // declared `gpFifoOffset`. See `plan_engine_object`.
                        let planned = kayfabe_fwd::plan_engine_object(
                            spine,
                            proc,
                            &route,
                            class,
                            params,
                            // ★★★★★ w288 — the VMM's grant, passed through untouched.
                            err_notifier_grant,
                        )?;
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

    /// ★★★★★ **w303 — the host-reachability census for one channel group.** ROUTE (rank
    /// 0) then CENSUS (rank 1, one proc), the same two-step
    /// [`SharedDevice::schedule_group`] takes — deliberately, so a `PREEMPT` and a
    /// `GPFIFO_SCHEDULE` naming one TSG can never be attributed to different groups.
    ///
    /// Pure read: no verb is issued, nothing is scheduled or staged. See
    /// [`kayfabe_core::gpu::GroupHostTwins`].
    ///
    /// # Errors
    /// [`kayfabe_core::gpu::ScheduleGroupFault`], by variant.
    pub fn group_host_twins(
        &self,
        client: kayfabe_arch::ids::HClient,
        object: kayfabe_arch::ids::HObject,
    ) -> Result<kayfabe_core::gpu::GroupHostTwins, kayfabe_core::gpu::ScheduleGroupFault> {
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
        self.with_proc(route.proc, |proc| {
            kayfabe_core::gpu::census_group_host_twins(proc, &route)
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

    /// ★★★★★ **w346 — RELAY ONE CONTROL TO THE HOST'S OWN SUBDEVICE.**
    ///
    /// For the GSS-legacy family that asks **the part about itself** — a CUDA major
    /// version, a capability mask — and so twins no guest object at all.
    ///
    /// # ⊘ Why `SYSTEM_PROC`, and why that is the SAFE choice rather than the lazy one
    ///
    /// Every other route in this port reaches a proc **through a GPU object**
    /// (`by_pdb`, `by_chan`). A control that names no object has no such route, so the proc
    /// must be chosen rather than derived — and the choice is load-bearing for isolation:
    ///
    /// - **the asking process's own isolate** — no route exists to find it from a bare
    ///   `hClient`, and building one would be inventing an index for this one verb;
    /// - **any live proc** — ⊘ **REFUSED.** That would answer one guest process's question
    ///   inside **another tenant's** isolate. Two processes in one guest do not have to
    ///   trust each other, and this is exactly the cross-process leakage that rules out;
    /// - **`SYSTEM_PROC`** — the device's own client, which is the isolate whose subdevice
    ///   this is. It is neither the asker's nor a peer's.
    ///
    /// ⚠ **The cost, named:** one isolate's health then gates this family for every guest
    /// process. That is a real coupling and it is the price of not crossing tenants.
    ///
    /// ⊘ **No `classify_control` here.** The Case-1/Case-2 split asks whether a control's
    /// host effect is already achieved; these have no local effect to have achieved, and
    /// the caller has already decided this id is one to forward. Running the classifier
    /// would let a Case-2 row silently turn a forward into an ack.
    ///
    /// # Errors
    /// [`FwdFault`] by variant — a retired proc, a target with no isolate, or the host's
    /// own refusal. ⚠ **`payload` is left untouched on every failure arm**: a zero-filled
    /// `NV_OK` is precisely the answer the CUDA runtime reads as real data and dies on
    /// (`C: nvkvm_gpu_emul.c:3334-3350`), which is the defect this whole verb exists to
    /// avoid re-creating.
    pub fn route_subdevice_control(
        &self,
        target_gpu: GpuId,
        cmd: ControlCmd,
        payload: &mut [u8],
    ) -> Result<ControlRoute, FwdFault> {
        let pid = kayfabe_core::gpu::Gpu::SYSTEM_PROC;
        let out: Vec<u8> = self.verb_op(
            || {
                self.route_act(
                    |_| Ok((pid, ())),
                    |_spine, proc, ()| {
                        let planned =
                            kayfabe_fwd::plan_subdevice_control(proc, target_gpu, cmd, payload)?;
                        Staged::check_out(proc, target_gpu, planned)
                    },
                )?
            },
            |_spine, proc, plan, reply| {
                let mut buf = vec![0u8; payload.len()];
                kayfabe_fwd::commit_subdevice_control(proc, plan, reply, &mut buf)?;
                Ok(buf)
            },
        )?;
        payload.copy_from_slice(&out);
        Ok(ControlRoute::Forwarded)
    }

    /// ★★★★★ **w288 TIER 2 — RELAY ONE CHANNEL CONTROL TO THE SAME CHANNEL ON THE HOST**,
    /// one guest ask to exactly one host issue, and the reply back verbatim.
    ///
    /// The guest names its own channel by `(hClient, hObject)`; this resolves that to the
    /// **host** channel handle the doorbell path already materialized and issues the SAME
    /// `cmd` with the SAME `payload` against it, writing the host's answer back in place.
    ///
    /// # ⊘⊘ ONE ASK, ONE ISSUE — and for `NV906F_CTRL_CMD_GET_MMU_FAULT_INFO` that is a
    /// # correctness requirement, not an efficiency one
    ///
    /// That control's record is **cleared by reading it**
    /// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl906f.h`). ⇒ Nothing may call this
    /// speculatively, on a timer, or "to warm a cache": a read nobody asked for consumes the
    /// record, and the party that did ask then gets a well-formed all-zero answer that reads
    /// exactly like *"no fault"*. This method is therefore driven by the guest's own RPC and
    /// by nothing else.
    ///
    /// # ⚠ RELAY, NEVER SYNTHESISE
    ///
    /// Every failure is a **named** variant of [`kayfabe_fwd::ChannelControlRelayFault`] and the caller's
    /// `payload` is left untouched on every one of them. There is deliberately no arm that
    /// zero-fills, no arm that invents an address, and no arm that answers `NV_OK` with a
    /// body we composed — a fabricated fault record is worse than no answer, because it is
    /// shaped exactly like a pass.
    ///
    /// # Errors
    /// [`kayfabe_fwd::ChannelControlRelayFault`], by variant.
    pub fn relay_channel_control(
        &self,
        client: HClient,
        object: HObject,
        cmd: ControlCmd,
        payload: &mut [u8],
    ) -> Result<kayfabe_fwd::ChannelControlRelay, kayfabe_fwd::ChannelControlRelayFault> {
        // ---- 1. ROUTE, rank 0. The SAME route the guest's own bind takes, so a control and
        //         a bind can never be attributed to different channels.
        let route = {
            let route_in = |spine: &kayfabe_core::gpu::Spine| {
                kayfabe_core::gpu::route_bind_channel(spine, client, object)
            };
            match self.mode {
                LockMode::Sharded => route_in(&self.state.read().spine),
                LockMode::Degenerate => route_in(&self.state.write().spine),
            }
            .map_err(kayfabe_fwd::ChannelControlRelayFault::NotRoutable)?
        };
        // ---- 2. THE HOST TWIN, read off the channel the route resolved. ⊘ `None` is a
        //         refusal BY NAME and never a fabricated answer: a guest channel with no host
        //         channel has nothing whose fault record could be read, and saying so is a
        //         finding — it means the birth path never ran for this channel.
        let found = self
            .with_proc(route.proc, |proc| {
                proc.channels
                    .get(&route.chan)
                    .map(|c| (c.gpu, c.host_channel))
            })
            .flatten();
        let Some((cgpu, host_channel)) = found else {
            return Err(kayfabe_fwd::ChannelControlRelayFault::ChannelGone {
                proc: route.proc,
                chan: route.chan,
            });
        };
        let Some(host_chan) = host_channel else {
            return Err(kayfabe_fwd::ChannelControlRelayFault::NoHostChannel {
                proc: route.proc,
                chan: route.chan,
            });
        };
        // ---- 3. ISSUE IT, once, on that channel's own isolate.
        //
        // ⊘ `AckOnly` is REFUSED here rather than reported as success. The classifier's
        // ack-only arm leaves the payload untouched, so relaying it would hand the guest
        // whatever it sent us — for a fault-info read, a buffer of zeros wearing an `NV_OK`.
        // That is the fabrication this whole verb exists to make unrepresentable.
        match self.route_control(cgpu, route.proc, host_chan, cmd, payload) {
            Ok(ControlRoute::Forwarded) => Ok(kayfabe_fwd::ChannelControlRelay {
                proc: route.proc,
                chan: route.chan,
                gpu: cgpu,
                host_chan,
            }),
            Ok(ControlRoute::AckOnly) => {
                Err(kayfabe_fwd::ChannelControlRelayFault::ClassifiedAckOnly {
                    proc: route.proc,
                    chan: route.chan,
                })
            }
            Err(f) => Err(kayfabe_fwd::ChannelControlRelayFault::HostRefused(f)),
        }
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
    /// ★★★★★ **What this channel IS to the guest** — carried off the same resolved
    /// [`kayfabe_core::gpu::Channel`] that produced [`Self::vas_pdb`] and
    /// [`Self::engine`], so the kind and the address space can never be attributed to
    /// different channels. See [`kayfabe_core::channel_kind`] for the model.
    ///
    /// # ⊘ It is the CARRIED fact, and it deliberately replaces a re-derivation
    ///
    /// [`Self::proc`] is still here and still correct, and `proc == Gpu::SYSTEM_PROC`
    /// still computes the same answer — the projection that assigns a channel's `ProcId`
    /// and the one that assigns its kind are the **same pass over the same
    /// `ProcBoundary`** (`Gpu::sync_proc_to_boundary`), so they cannot disagree. What
    /// changed is which of the two the load-bearing gate reads: a consumer that reaches
    /// for `proc` and compares it to a reserved constant is re-deriving a declared fact,
    /// and this tree has paid for that shape more than once
    /// (`two_projections_of_one_fact_disagreeing`; and for this exact axis, 12 boots).
    pub kind: kayfabe_core::channel_kind::GuestChannelKind,
    /// ★★★★★ **w288 — WHERE THIS CHANNEL ASKED TO BE TOLD ABOUT ITS OWN DEATH**, carried
    /// off the same resolved [`kayfabe_core::gpu::Channel`] as [`Self::kind`] and
    /// [`Self::vas_pdb`], so the notifier and the address space can never be attributed to
    /// different channels.
    ///
    /// The three states are the guest's, not ours, and they are kept apart because they lead
    /// a reader to different files: `Sysmem { gpa }` is servable (an
    /// `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` can be built over those pages), `Unreachable` is a
    /// gap in **us**, and `None` is the guest waiving error reporting altogether. ⊘ Folding
    /// the last two together is the collapse `kayfabe_arch::fault::ErrorNotifier`'s own docs
    /// refuse.
    ///
    /// ⊘ Reported, never resolved: turning the GPA into a file offset is the **VMM's** job
    /// and its alone (`kayfabe_isolate::GuestRamGrant::originated_by_the_vmm`).
    pub error_notifier: Option<kayfabe_arch::fault::ErrorNotifier>,
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
    /// ★★★★ **§16.71 — the RM-graph node this whole struct was read off**, as
    /// `(client, handle)`.
    ///
    /// # ⊘ Why an identity, and why on this struct
    ///
    /// `[measured 2026-08-10, boots `w205_227194f_ctl` / `_real`]` two ring addresses were
    /// carried out of this campaign for what §16.70.6 called *"one token"* —
    /// `0x120064000` on the control and `0x420064000` on the real plane — and the record
    /// could not say whether that was RM placing one channel's ring differently or the two
    /// paths reading **different channels**. Neither number was printed with anything that
    /// names *which object it belongs to*, so the question was unanswerable from the log
    /// rather than merely unanswered.
    ///
    /// ⊘ This field decides **nothing**; it is the join key a reader needs to compare this
    /// projection with [`kayfabe_fwd::read_gpfifo_ring`]'s. It is the key of the very node
    /// [`Self::ring_va`] is read from, on the same line of the same locked look, so a ring
    /// address and the object that declared it can never be attributed to different
    /// channels.
    pub chan_key: (u32, u32),
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
/// `[measured 2026-08-10, boot `s51_d502ac6_engroute`]` it was **86 doorbells wide**:
/// `GrCompute=86 Ce=362`, and `86 + 362 = 448`. ⊘ Nothing was forged (the codec is
/// class-gated, so a GR ring decodes to `Opaque`), but those doorbells reached the wrong
/// executor and could never reach a right one.
///
/// ⊘ **What this doc used to claim as the visible symptom was a DIFFERENT doorbell's**:
/// `SubmissionHasNoLaunch { methods: 3, opaque: 2 }` was named here as *"a GR pushbuffer
/// being decoded by the CE codec"*, and the refusal's own printed pushbuffer says
/// `SET_OBJECT → AMPERE_DMA_COPY_B` — a CE push, on a CE channel, at the CE executor.
/// `w202` neither moved it nor could have. It was §16.66's four-word semaphore release.
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

/// ★★★★★ **What the SHELL'S doorbell port does with a doorbell of a given route** — the
/// pure half of the decision `kayfabe-qemu-raw`'s `SharedDoorbell::try_ce_submission`
/// makes, split out here for [`route_of_engine`]'s stated reason: *"the half that can be
/// quantified over should be."*
///
/// ⊘ **Three answers and not two.** Before the GR passthrough route existed the shim's
/// decision was a `bool` spelled `route != DoorbellRoute::CpuCe`, which forced `HostGr` and
/// `Unserved` into one bucket. They are not one bucket: `Unserved` is *"nobody has designed
/// a path for this engine"*, and `HostGr` is *"the core's ring path serves this"* — the same
/// distinction [`DoorbellRoute`] itself exists to preserve, and collapsing it one layer down
/// was what made the route unopenable without touching the refusal's own name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellDisposition {
    /// The shell's own CPU copy-engine executor (`kayfabe_rt::ceutils`) may serve it —
    /// subject to the *other* gates the shim applies after this one. ⊘ Not *"it will be
    /// served here"*: this answer only says the routing fact does not exclude it.
    MayServeLocally,
    /// ★★★ **PASSTHROUGH.** Hand the doorbell to the core — [`SharedDevice::doorbell`],
    /// which routes the guest token, materializes/schedules the host channel and rings the
    /// **host** token. ⊘ The shell runs no executor for it and interprets none of its
    /// bytes.
    HandToCore,
    /// Refuse by name. Nothing in this process is an executor for this engine and the core
    /// has no ring path for it either.
    RefuseByRoute,
}

/// ★★★★★ **THE SHELL'S DISPOSITION**, over a [`DoorbellRoute`] and one arming flag.
///
/// # ⚠ `gr_passthrough` re-opens a path that was CLOSED ON EVIDENCE
///
/// §16.65 replaced a `HostGr` fall-through to the core with a named refusal, because
/// `execution_plane_increments.md` §15.5 had measured what the fall-through achieved:
/// *"we rang a doorbell on a host channel into which the guest's methods were never
/// copied"*, and *"a true refusal outranks a forwarded no-op."* That measurement has not
/// been overturned — see `docs/design/gr_doorbell_passthrough.md` §0.3, which states at the
/// code why a routed GR doorbell **still** cannot make the host engine fetch the guest's
/// ring (the host channel's ring and its `GP_PUT` are both ours).
///
/// ⇒ The flag exists so that re-opening it is a **deliberate, armed, printed, controlled**
/// choice with a default that is byte-identical to today, rather than a silent flip. What
/// the armed arm buys is the **transport**: the first `ring_doorbell` ever issued for a GR
/// host token, which is the standing debt `RESUME_HERE_2026_08_11.md` §3 records.
///
/// ⊘ The `match` is exhaustive with no `_` arm, for [`route_of_engine`]'s reason: a new
/// [`DoorbellRoute`] variant fails this build until somebody says what the shell does with
/// it.
#[must_use]
pub fn shell_disposition(route: DoorbellRoute, gr_passthrough: bool) -> ShellDisposition {
    match route {
        DoorbellRoute::CpuCe => ShellDisposition::MayServeLocally,
        DoorbellRoute::HostGr if gr_passthrough => ShellDisposition::HandToCore,
        DoorbellRoute::HostGr | DoorbellRoute::Unserved => ShellDisposition::RefuseByRoute,
    }
}

/// ★★★★ **Whether the guest's ring CONTENT may be forwarded for this engine** — i.e.
/// whether [`SharedDevice::forward_ring`], which parses the ring with the **copy-engine**
/// codec and plans `ce_copy`s, is the right verb for a doorbell on it.
///
/// # ⊘ Why this is derived from [`route_of_engine`] and is not a second table
///
/// It is the same question that decides the executor, asked one layer up, and this project
/// has measured twice what two tables for one question cost (§16.64's probe contradicting
/// the serving path beside it). ⇒ One authority, two callers.
///
/// # ⚠ Why a GR doorbell must not run it — this is a RULING, not an optimisation
///
/// The copy-engine codec is class-gated, so every span of a GR pushbuffer decodes `Opaque`
/// and the forward moves nothing. But `forward_ring` can return `Err`, and
/// [`SharedDevice::doorbell`] propagates it — so a GR doorbell that rang the host channel
/// **successfully** would be reported `Refused`, and *whether the doorbell was forwarded*
/// would depend on *whether its ring parsed*. That is exactly the property
/// `tests/tests/doorbell_is_forwarded_without_reading_the_ring.rs` exists to forbid, and
/// the rung brief states it as a requirement: *"ring resolution / pushbuffer reads / method
/// decode … must never gate whether the doorbell is forwarded."*
///
/// ⊘ **Not solved by passing `vmm: None` from the shim.** That parameter's own doc names
/// that move: *"a caller that holds a port and passes `None` silently forwards nothing."*
/// The decision belongs to the engine, by name, here.
/// # ★★★★★ w287 — AND THE KIND, because the two dispositions are MUTUALLY EXCLUSIVE
///
/// This used to take `engine` alone, which made the predicate *"is this CE"*, full stop. That
/// is not enough to decide the question it is actually asked, and `[measured 2026-08-13,
/// w283d, real GA106]` says so: one CE doorbell rang the **adopted** channel (`host_token=0x6`)
/// and then also decoded that channel's guest ring and ran `ce_copy` on `host_token=0x7`.
///
/// ⊘ **`ce_copy` DRIVES a channel; adoption means the GUEST drives it.** We cannot both write
/// the ring and let the guest own it, and we cannot both ring the doorbell and expect hardware
/// to advance a cursor we are also advancing. On a [`GuestChannelKind::Passthrough`] channel
/// the engine fetches the guest's pushbuffer **directly** and this codec must not run — not as
/// an optimisation, but because running it is the fallback that has made every green run
/// unable to distinguish *"passthrough worked"* from *"the decode-and-re-emit caught it"*.
///
/// ⇒ The owner's ruling, as a total function: `KERNEL` → emulate (our scratchpad ring, our
/// codec); `USER`/`ADMIN` → passthrough (their ring, untouched). [`GuestChannelKind`] is
/// exactly that axis, resolved once per proc boundary and **carried**, never re-derived.
#[must_use]
pub fn ring_content_is_forwardable(
    engine: EngineKind,
    kind: kayfabe_core::channel_kind::GuestChannelKind,
) -> bool {
    use kayfabe_core::channel_kind::GuestChannelKind;
    matches!(route_of_engine(engine), DoorbellRoute::CpuCe)
        && matches!(kind, GuestChannelKind::Emulated)
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
