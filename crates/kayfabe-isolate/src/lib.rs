//! # kayfabe-isolate — the per-process unprivileged-sandbox port
//!
//! Two abstract seams live here:
//!
//! - [`RmBackend`] — **RM verbs, not ioctls** (arch doc §4.2, crate `kayfabe-rm` folded
//!   into this crate until the wire protocol lands): the unprivileged host-RM
//!   operation surface an isolate can issue. Abstract by design — the Windows-host
//!   door stays open, and NO real NVOS struct/ioctl number appears here (Axis A is
//!   quarantined to `kayfabe-abi`).
//! - [`Isolate`] / [`IsolateFactory`] — the per-guest-process sandboxed host worker.
//!   One isolate per `Proc` (`session_id == ProcId`), giving **blast-radius
//!   containment**: a bug forwarding process A cannot touch process B's host
//!   handles/mappings (threat-model boundary 2, arch doc §4.3.5).
//!
//! The real implementation (spawn, `CLONE_NEW*` namespaces, pivot_root, seccomp,
//! socket wire protocol — the Mode-1 stub posture) is an adapter crate concern.
//! This crate is pure interface + value types so the core and its tests never touch
//! an OS. `kayfabe-mocks::{MockRmBackend, MockIsolate}` are the test impls.
//!
//! ## ★ R1's teeth live here (`l1_concurrency.md` §3.3, stage 3)
//!
//! A host RM verb is the archetypal blocking call, so **R1 — no blocking call under
//! ANY lock, ever — must be asserted at this port's door, not at a wrapper someone
//! must remember to use** (the §12.6 gap stage 3 closes). Two mechanisms, together:
//!
//! - **Ownership shape.** [`RmBackend`] is not reachable from an [`Isolate`] by
//!   reference. It lives inside the isolate's bounded pool of [`Worker`]s, and
//!   [`Isolate::checkout`] **moves a worker OUT** to the calling thread (§7.3). A
//!   locked core phase therefore has nothing to call: it can *emit* a [`VerbPlan`]
//!   and check a worker out, and that is all.
//! - **Runtime assert.** [`Worker::execute`] (and the [`Worker::with_rm`] escape
//!   hatch) call [`kayfabe_util::lockwitness::assert_lock_free`] before touching the
//!   backend. Holding any ranked lock at a verb is an immediate panic naming R1.
//! - **★ The other blocking thing, and it is not a verb** (`l1_concurrency.md`
//!   §12.16, gap G3b): an isolate's `Drop` is `waitpid` + namespace teardown, run by
//!   the compiler at a point no call site names, and the verb assert cannot see it.
//!   [`IsolateBox`] — the only way core state owns an [`Isolate`] — asserts the same
//!   invariant on the drop side. `Spine::reap_retired` was performing exactly that
//!   drop under the device write lock, and nothing could notice.
//!
//! Full compile-time enforcement of "no guard is alive on this thread" is not
//! expressible in safe Rust; the ownership shape makes violations contortions
//! instead of accidents, and the assert is the real teeth.
//!
//! ## Concurrency (decision #17)
//!
//! [`Isolate`] and [`IsolateFactory`] are **`Send + Sync` supertraits**: the core
//! *stores* them (each `Proc` owns its isolates inside [`IsolateBox`], the `Gpu` owns the
//! factory), so they inherit the core's shareability requirement. Their `&self`
//! surface is pure reads (`id`/`is_retired`); every mutation takes `&mut self`, so
//! exclusivity comes from the caller's borrow, as everywhere in the core.
//! [`RmBackend`] is `Send + Sync` **because the pool stores it**, not because any
//! call path shares one: it is reachable exclusively through a [`Worker`] the caller
//! `&mut`-owns, so a shared reference never exists. The bound is nonetheless
//! load-bearing and not droppable — an [`Isolate`] owns N idle [`Worker`]s, a `Proc`
//! owns the isolate, and the core's `Gpu` is `Sync`, so every boxed backend sitting
//! in a pool slot must be `Sync` for that chain to hold. (Before the §7.2 pool, no
//! `Box<dyn RmBackend>` was ever *stored* in core state, and the crate carried a
//! `Send`-only exception here; storing the pool is what cashed it in. Cost to real
//! impls: no `Rc`/`Cell` in a backend's private state — which the `&mut`-only
//! surface never needed anyway.)

pub use kayfabe_arch::ids::ControlCmd;
use kayfabe_arch::ids::{ClassId, EngineKind, GpuId};
use kayfabe_vmm::SurfaceHandle;

/// A host-side RM object handle, scoped to ONE isolate's handle namespace.
///
/// Handles from different isolates must never be interchangeable — the mock backend
/// enforces this in tests (boundary-2 blast-radius assertion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostHandle(pub u64);

/// Errors an RM verb can return, in core terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RmError {
    /// The op requires privilege the isolate deliberately lacks. This is
    /// **"wrong layer," never "gain privilege"** (lesson L2: a Case-2 GSP-internal
    /// control replayed on the host gets exactly this) — callers must treat it as a
    /// design error in the forwarding decision, not retry with more privilege.
    InsufficientPermissions,
    /// Unknown handle in this isolate's namespace.
    BadHandle(HostHandle),
    /// Host resource exhaustion.
    NoMemory,
    /// ★ **The verb was CANCELLED — it did not fail** (`l1_concurrency.md` §5.4 and
    /// §12.16, gap G4).
    ///
    /// The archetypal cause is the one §5.4 designs for: a guest thread blocked in a
    /// forwarded op dies or takes a signal, so the requester interrupts the in-flight
    /// verb rather than wedging until the host ioctl finishes on its own.
    ///
    /// **Measured in the C, not invented here** (issue #73). The host stub installs a
    /// SIGUSR1 handler *without* `SA_RESTART` precisely so a blocked
    /// `ioctl()` on `/dev/nvidia*` returns `-EINTR` rather than auto-restarting
    /// (`C: src/stub/nvkvm_stub.c:699-708`, `:2669-2678`); the interrupt itself
    /// arrives out of band as a command (`ISOLATE_CMD_INTERRUPT`,
    /// `C: src/common/nvkvm_isolate_proto.h:53,122-131`) and the worker then answers
    /// **on the ordinary reply path** carrying `retval = -EINTR`. There is no separate
    /// "interrupted" reply message in the C's wire protocol, which is exactly why this
    /// is an [`RmError`] variant and not a new [`VerbReply`]: at this port an
    /// interrupted verb *is* a verb that came back, with a distinguishable status.
    ///
    /// **The worker survives it.** The C's stub clears its in-flight txn and loops
    /// (`C: src/stub/nvkvm_stub.c:1276-1281`), and its framing treats `-EINTR` as
    /// resumable (`:569-571`). So the unwind CAN still run on this worker — which is
    /// what makes [`VerbFailure::orphans`] meaningful here, and what distinguishes
    /// cancellation from worker *death* ([`Isolate::worker_died`], §7.3), where the
    /// verb never returns at all.
    ///
    /// **Never retry it as if it were transient.** A cancellation is a fact about the
    /// requester, not about the host: retrying re-issues work whose requester is
    /// gone. It is §12.9's *third* staleness shape — non-retryable and
    /// orphan-carrying — and the fwd plane surfaces it as
    /// `FwdFault::Cancelled`, never as an RM failure (that conflation is §12.10 one
    /// layer over).
    Interrupted,
    /// Any other backend-reported failure (opaque status for diagnostics).
    Other(u32),
}

/// # The unprivileged host-RM verb surface
///
/// The complete vocabulary of host operations the forwarding plane may request.
/// Everything here must be issuable by an **unprivileged** host process — that
/// unprivilege, not the keying, is the load-bearing host security boundary
/// (lesson L8). There is deliberately no verb that could express a privileged
/// GSP-internal replay.
///
/// Two verb tiers, both abstract:
///
/// - **Generic verbs** (`alloc`/`free`/`control`) — ABI-typed passthrough for
///   Case-1 shadow-forwarding, parameter encoding supplied by the ABI adapter.
/// - **Intent verbs** (`alloc_vaspace`/`alloc_sysmem`/`alloc_channel`/`schedule`/…)
///   — named *intents* the adapter lowers to the correct per-version NVOS sequence
///   (lesson L2: translate guest intent; class/param selection is the adapter's
///   Axis-A job, never the core's).
///
/// Object-safe; implemented by the linux-ioctl adapter inside the sandbox, and by
/// `MockRmBackend` in tests (scriptable failures, recorded verb log).
///
/// `Send + Sync` — see the crate docs: only ever reached via `&mut`, so shared
/// cross-thread references are unrepresentable, but the pool *stores* boxed
/// backends inside a `Sync` `Proc`, which makes the bound structural.
pub trait RmBackend: Send + Sync {
    /// Allocate an RM object of `class` under `parent`. `params` is an opaque,
    /// already-encoded parameter blob (encoding is the ABI adapter's job).
    fn alloc(
        &mut self,
        parent: HostHandle,
        class: ClassId,
        params: &[u8],
    ) -> Result<HostHandle, RmError>;

    /// Intent verb: allocate a fresh **host GPU virtual address space** for one
    /// guest `Vas`. Per-Vas host VAS separation is the proven #14 fix: two guest
    /// processes' identical guest VAs publish into *different* host VASes and
    /// cannot collide.
    fn alloc_vaspace(&mut self) -> Result<HostHandle, RmError>;

    /// Intent verb: allocate `len` bytes of host-visible system memory.
    fn alloc_sysmem(&mut self, len: u64) -> Result<HostHandle, RmError>;

    /// Intent verb: allocate a host GPU channel bound to host VAS `vas`, on the
    /// runlist/engine named by `engine` — the channel's graph-derived [`EngineKind`],
    /// which the adapter lowers to the host `NV_CHANNEL_ALLOC_PARAMS` engine type.
    /// The engine is declared HERE because the adapter cannot invent it: an
    /// engine-blind channel alloc is the C's proven wrong-runlist bug class
    /// (`dma_copy_class_alloc_params`: `engineType=0` → wrong runlist →
    /// cuCtxCreate 401 — seam audit GR-1). Returns
    /// `(channel_handle, host_work_submit_token)`.
    fn alloc_channel(
        &mut self,
        vas: HostHandle,
        engine: EngineKind,
    ) -> Result<(HostHandle, u64), RmError>;

    /// Intent verb: allocate an **engine object** (compute / graphics / CE / NVENC)
    /// of `class` on host channel `chan` — the Case-1 forward that makes the host
    /// kernel-RM build and self-promote its OWN context (golden ctx included, on real
    /// silicon). `params` is the ABI-lowered alloc blob (Axis A: `IS_EXTERNALLY_OWNED`
    /// already stripped, etc.). NOTE the anti-bolt-on property: this is *almost* the
    /// generic [`RmBackend::alloc`] with `parent = chan`; it is named only to state
    /// the intent — the host verb surface does NOT grow to add an engine.
    fn alloc_engine_object(
        &mut self,
        chan: HostHandle,
        class: ClassId,
        params: &[u8],
    ) -> Result<HostHandle, RmError>;

    /// Intent verb: make `chan` runnable (the GPFIFO_SCHEDULE intent). Per-proc,
    /// never a one-shot: #12's CTX2 rang off-runlist because scheduling was a
    /// sticky global in the C.
    fn schedule(&mut self, chan: HostHandle) -> Result<(), RmError>;

    /// Free an RM object (and its subtree, per RM semantics).
    fn free(&mut self, obj: HostHandle) -> Result<(), RmError>;

    /// Issue a control command on an object; `payload` is read and written in place.
    fn control(
        &mut self,
        obj: HostHandle,
        cmd: ControlCmd,
        payload: &mut [u8],
    ) -> Result<(), RmError>;

    /// Map `len` bytes of `memory` into the host GPU VA space owned by `vas`,
    /// returning the host GPU VA. The isolate picks/owns the actual placement —
    /// per-Vas host-VAS separation is what makes two guest processes' identical
    /// guest VAs land in disjoint host mappings (#14's proven fix).
    fn map_gpu_va(&mut self, vas: HostHandle, memory: HostHandle, len: u64)
    -> Result<u64, RmError>;

    /// Unmap a previous [`RmBackend::map_gpu_va`].
    fn unmap_gpu_va(&mut self, vas: HostHandle, gpu_va: u64) -> Result<(), RmError>;

    /// Ring the host work-submit doorbell with an (already host-translated) token.
    fn ring_doorbell(&mut self, host_token: u64) -> Result<(), RmError>;

    /// Intent verb: export the host memory object `memory` (a render target in host
    /// VRAM) as a presentable [`SurfaceHandle`] — the **producer half of the display
    /// seam** (`execution_plane.md` §3.3, seam audit GR-2b). The C proved this runs
    /// in the ISOLATE (stub `PRIME_HANDLE_TO_FD` dma-buf export, session-owned —
    /// `present_path_b_done`); the flow is one-way guest→host. The consumer half is
    /// `Present::present`. Anti-bolt-on note: this is the ONE named display verb —
    /// the verb surface does not grow per engine.
    fn export_surface(&mut self, memory: HostHandle) -> Result<SurfaceHandle, RmError>;
}

/// Session identity of an isolate. Equals the owning `ProcId` by construction
/// (arch doc §4.3.4; the isolate infra supports 4096 sessions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IsolateId(pub u32);

/// One worker slot inside an isolate's **bounded pool** (`l1_concurrency.md` §7.2,
/// decision #37).
///
/// The pool exists because a single-in-flight worker per isolate serializes a guest
/// process's *own* threads behind each other (the #37 intra-proc blocking gap); N
/// workers per `(Proc, GpuId)` isolate let sibling guest threads have verbs in flight
/// concurrently, while each individual worker stays strictly single-in-flight (a
/// property the type system gives for free: a worker is reached only by `&mut`).
///
/// Dense and **scoped to its owning isolate** — `WorkerId(0)` of proc A's GPU0 isolate
/// and `WorkerId(0)` of proc B's GPU1 isolate are unrelated identities, exactly like
/// [`HostHandle`]. Anything keyed on a worker must carry the `(ProcId, GpuId)` pair
/// alongside it (see `kayfabe_core::reactor::SourceKind::Worker`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkerId(pub u32);

/// The bounded pool's default size (`l1_concurrency.md` §7.2, "Calibration").
///
/// **A tuning constant, not a design question.** The design is explicit that the pool
/// is *statically* sized first and grows dynamically only when a measured workload
/// proves the bound hurts — a spawn/reap policy, thundering-herd wakeups and
/// worker-lifetime races are all cost with no demonstrated benefit. Order of the vCPU
/// count; four is a small commodity guest.
pub const DEFAULT_POOL_WORKERS: usize = 4;

// =================================================================================
// The verb PLAN — what a locked core phase emits instead of calling (R1's
// "consequence for the core shape", `l1_concurrency.md` §3.3)
// =================================================================================

/// A freshly allocated host channel: `(handle, host work-submit token)`.
pub type ChannelHandles = (HostHandle, u64);

/// ★ A **typed verb chain** — the description of host work a locked core phase
/// emits, executed later by [`Worker::execute`] with no lock held.
///
/// Deliberately NOT a resumable continuation machine. Every site's verbs are
/// data-dependent only on *each other* (host VAS handle → memory handle → mapped VA),
/// never on core state read between two verbs, so a plain chain suffices and the
/// execution step can thread its own intermediate results. If a future site genuinely
/// needs to consult core state mid-chain, that is a design change to argue in the doc
/// — never a hidden lock acquisition inside execution.
///
/// Owned payloads (`Vec<u8>`) rather than borrows: a plan outlives the lock scope
/// that produced it by construction, so it cannot hold a reference into core state.
/// Control/alloc blobs are small; the copy is the price of the invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerbPlan {
    /// `publish_backing`'s chain: (optionally) allocate the Vas's own host VAS, then
    /// allocate sysmem and map it into that VAS.
    Publish {
        /// The Vas's already-materialized host VAS, or `None` to allocate one.
        host_vas: Option<HostHandle>,
        /// Bytes to allocate and map.
        len: u64,
    },
    /// The doorbell chain: (optionally) host VAS → (optionally) host channel →
    /// (optionally) schedule → ring.
    Doorbell {
        /// The Vas's host VAS, or `None` to allocate one (only consulted when
        /// `channel` is `None`).
        host_vas: Option<HostHandle>,
        /// The channel's already-materialized host handles, or `None` to allocate.
        channel: Option<ChannelHandles>,
        /// The channel's graph-derived engine (GR-1: the adapter cannot invent the
        /// runlist, so the core declares it).
        engine: EngineKind,
        /// Whether this submission must make the channel runnable first.
        schedule: bool,
    },
    /// The Case-1 engine-object chain: (optionally) host VAS → (optionally) host
    /// channel → engine-object alloc.
    EngineObject {
        /// The Vas's host VAS, or `None` to allocate one (only when `channel` is
        /// `None`).
        host_vas: Option<HostHandle>,
        /// The channel's host handles, or `None` to materialize it first.
        channel: Option<ChannelHandles>,
        /// The channel's engine (rides the channel alloc, GR-1).
        engine: EngineKind,
        /// The engine-object class.
        class: ClassId,
        /// The ABI-lowered alloc blob.
        params: Vec<u8>,
    },
    /// One Case-1 control, payload carried by value in and out.
    Control {
        /// The control object, in this isolate's namespace.
        obj: HostHandle,
        /// The command.
        cmd: ControlCmd,
        /// The in/out payload.
        payload: Vec<u8>,
    },
    /// ★ The disposition of host objects a **refused commit** could not adopt
    /// (`l1_concurrency.md` §3.3 R5: a commit whose target vanished must not
    /// silently leak what it already allocated). Runs on the same worker, still
    /// lock-free, before the worker is checked back in.
    Release {
        /// `(host VAS, host GPU VA)` pairs to unmap first.
        unmap: Vec<(HostHandle, u64)>,
        /// Objects to free, in the given order.
        free: Vec<HostHandle>,
    },
}

/// What one [`VerbPlan`] produced — the typed reply a commit phase re-enters with.
///
/// `host_vas` / `channel` fields carry only what this execution **freshly allocated**
/// (`None` = the plan reused what core state already held), which is exactly what the
/// commit must adopt or orphan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerbReply {
    /// [`VerbPlan::Publish`]'s reply.
    Published {
        /// Freshly allocated host VAS, if the plan asked for one.
        host_vas: Option<HostHandle>,
        /// The allocated host memory object.
        memory: HostHandle,
        /// The host GPU VA it was mapped at.
        host_va: u64,
    },
    /// [`VerbPlan::Doorbell`]'s reply.
    Doorbell {
        /// Freshly allocated host VAS, if any.
        host_vas: Option<HostHandle>,
        /// Freshly allocated host channel, if any.
        channel: Option<ChannelHandles>,
        /// Whether the schedule verb ran.
        scheduled: bool,
    },
    /// [`VerbPlan::EngineObject`]'s reply.
    EngineObject {
        /// Freshly allocated host VAS, if any.
        host_vas: Option<HostHandle>,
        /// Freshly allocated host channel, if any.
        channel: Option<ChannelHandles>,
        /// The host engine object.
        object: HostHandle,
    },
    /// [`VerbPlan::Control`]'s reply — the payload as the host wrote it back.
    Control {
        /// The written-back payload.
        payload: Vec<u8>,
    },
    /// [`VerbPlan::Release`]'s reply.
    Released,
}

/// ★ Host objects that exist and that the core could not adopt — **the record a
/// failed or refused operation leaves behind** (`l1_concurrency.md` §12.16, gap G4).
///
/// Two producers, both of which used to lose it:
///
/// - a **refused commit** (R5) whose execute phase already allocated — the caller
///   runs [`Orphans::release_plan`] on the SAME worker, still lock-free, before
///   checking it back in;
/// - a **mid-chain verb failure** — [`VerbFailure::orphans`], which carries whatever
///   the worker's own unwind could not dispose of.
///
/// ★ **`#[must_use]`, and it earns it.** Dropping an `Orphans` on the floor silently
/// leaks every host object it names — the exact defect this type exists to record —
/// and the compiler is the only thing that reliably notices. (Same reasoning that
/// gave `kayfabe_core::reactor::WakeRequest` its teeth.)
///
/// **Order is unmap-then-free, and that is RM's rule, not our preference.** RM frees
/// children and dependents ahead of parents (`ogkm:
/// src/nvidia/src/libraries/resserv/src/rs_server.c:963-981`,
/// `.../rs_client.c:1086-1122`) and auto-unmaps a resource's inter-mappings inside
/// `clientFreeResource_IMPL` *before* `objDelete` (`.../rs_client.c:830-849`). So RM
/// itself leaks nothing if we free a mapped object — but any **external mirror** of
/// that mapping (ours: the address table's `HostBacking` entries, gap G1) goes stale,
/// which is why the plan states the unmaps first and means it. The map/unmap ABI pair
/// is `NVOS46`/`NVOS47` respectively (`gvisor: pkg/sentry/devices/nvproxy/version.go:176-177`).
#[must_use = "an Orphans that is neither released nor recorded is a silent host-object \
              leak — that is the whole defect this type exists to make impossible. \
              Run `release_plan()` on a checked-out worker, or hand it onward."]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Orphans {
    /// `(host VAS, host GPU VA)` mappings to undo first.
    pub unmap: Vec<(HostHandle, u64)>,
    /// Objects to free.
    pub free: Vec<HostHandle>,
}

impl Orphans {
    /// True if there is nothing to dispose of.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.unmap.is_empty() && self.free.is_empty()
    }

    /// The verb chain that disposes of these orphans.
    #[must_use]
    pub fn release_plan(&self) -> VerbPlan {
        VerbPlan::Release {
            unmap: self.unmap.clone(),
            free: self.free.clone(),
        }
    }
}

/// ★ What a [`Worker::execute`] failure actually leaves behind: **why it failed, and
/// what still exists because of it** (`l1_concurrency.md` §12.16, gap G4).
///
/// `execute` used to return a bare [`RmError`] and promise all-or-nothing by unwinding
/// internally. The promise was overstated in two ways, and both are now expressible:
///
/// 1. The unwind's own `free`s were `let _ = …` — a failure to dispose of a partially
///    built chain was swallowed with no record anywhere. Now every object the unwind
///    could not free lands in [`VerbFailure::orphans`].
/// 2. Cancellation ([`RmError::Interrupted`]) is the entire premise of §5.4, and a
///    cancelled chain is precisely a chain whose all-or-nothing cannot be assumed.
///
/// ## ★ What `orphans` does and does not enumerate — a named unknown, not a guess
///
/// It enumerates every host object **whose handle this execution received** and could
/// not dispose of. It cannot enumerate an object the host may have created for a verb
/// whose reply never arrived — an interrupted alloc.
///
/// The C never settled that, and it is honest to say so rather than assert an answer:
/// its stub records nothing on a non-zero return
/// (`C: src/qemu/nvkvm_isolate_handlers.c:1444-1445`, `:1497-1501` — bookkeeping gated
/// on `ret == 0 && nvstatus == 0`), its guest discards the reply entirely on the
/// interrupt path (`C: src/guest/nvkvm_virtio.c:461-471`), and there is no
/// reconciliation code anywhere in the C for an alloc that may have landed. Compounding
/// it, most RM waits are *not* interruptible in the first place (`ogkm:
/// kernel-open/nvidia/nv.c` carries only a handful of `*_interruptible` waits), so a
/// cancelled alloc plausibly completed. The C's only disposition for such an object is
/// bulk: the #80 session reaper force-closing the isolate's host fds
/// (`C: src/qemu/virtio_nvgpu.c:100-118`).
///
/// **OPEN QUESTION, needs a bench experiment, must not be reasoned about:** does an
/// interrupted `NV_ESC_RM_ALLOC` leave the object created, partially created, or
/// absent? Until that is measured, the design must keep isolate-session death as the
/// backstop disposition and must not claim per-object completeness.
#[must_use = "a VerbFailure names host objects that still exist — dropping it \
              without releasing or recording its `orphans` is a leak."]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerbFailure {
    /// Why the chain stopped.
    pub err: RmError,
    /// Host objects this execution allocated and could not dispose of (see above for
    /// exactly what this can and cannot cover).
    pub orphans: Orphans,
}

impl VerbFailure {
    /// A failure that left nothing behind (the chain failed on its first verb, or the
    /// unwind disposed of everything).
    pub fn bare(err: RmError) -> Self {
        VerbFailure {
            err,
            orphans: Orphans::default(),
        }
    }
}

impl From<RmError> for VerbFailure {
    fn from(err: RmError) -> Self {
        VerbFailure::bare(err)
    }
}

/// ★ A checked-out pool worker: **the one door to a host RM verb**.
///
/// Obtained only from [`Isolate::checkout`], which moves it OUT of the isolate's pool
/// (§7.3). While it is out, exactly one thread `&mut`-owns it — so
/// single-in-flight-per-worker stays the borrow checker's guarantee, N times over,
/// and there is no shared in-flight slot table and no txn demux anywhere (§11 B6:
/// concurrency comes from channel COUNT, never from multiplexing one channel).
///
/// `Send`, not `Sync` — it migrates to whichever thread checked it out, but a shared
/// reference to one is unrepresentable.
pub struct Worker {
    id: WorkerId,
    backend: Box<dyn RmBackend>,
}

impl core::fmt::Debug for Worker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Worker").field("id", &self.id).finish()
    }
}

impl Worker {
    /// Wrap a backend as pool slot `id` (isolate implementations only).
    #[must_use]
    pub fn new(id: WorkerId, backend: Box<dyn RmBackend>) -> Self {
        Worker { id, backend }
    }

    /// This worker's slot in its isolate's pool.
    #[must_use]
    pub fn id(&self) -> WorkerId {
        self.id
    }

    /// ★ Run `plan`'s verb chain. **Asserts R1 first** — invoking a host verb with
    /// any ranked lock held panics naming R1 (crate docs).
    ///
    /// Chains its own intermediate results with **zero core access**. On a mid-chain
    /// failure it releases what it already allocated on this same worker, then returns
    /// a [`VerbFailure`] carrying both the error and **whatever the release could not
    /// dispose of** (§12.16, G4). The all-or-nothing promise is thereby made checkable
    /// instead of asserted: when the residue is empty it held; when it is not, the
    /// caller has the list rather than a swallowed `let _ =`.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1).
    pub fn execute(&mut self, plan: &VerbPlan) -> Result<VerbReply, VerbFailure> {
        kayfabe_util::lockwitness::assert_lock_free("issuing a host RM verb");
        let rm = &mut *self.backend;
        match plan {
            VerbPlan::Publish { host_vas, len } => {
                let (vas, fresh_vas) = match *host_vas {
                    Some(h) => (h, None),
                    None => {
                        let h = rm.alloc_vaspace()?;
                        (h, Some(h))
                    }
                };
                let memory = match rm.alloc_sysmem(*len) {
                    Ok(m) => m,
                    Err(e) => return Err(unwind(rm, fresh_vas.into_iter().collect(), e)),
                };
                let host_va = match rm.map_gpu_va(vas, memory, *len) {
                    Ok(va) => va,
                    Err(e) => {
                        let mut orphans = vec![memory];
                        orphans.extend(fresh_vas);
                        return Err(unwind(rm, orphans, e));
                    }
                };
                Ok(VerbReply::Published {
                    host_vas: fresh_vas,
                    memory,
                    host_va,
                })
            }
            VerbPlan::Doorbell {
                host_vas,
                channel,
                engine,
                schedule,
            } => {
                let (chan, fresh_vas, fresh_chan) = match *channel {
                    Some(c) => (c, None, None),
                    None => {
                        let (vas, fresh_vas) = match *host_vas {
                            Some(h) => (h, None),
                            None => {
                                let h = rm.alloc_vaspace()?;
                                (h, Some(h))
                            }
                        };
                        match rm.alloc_channel(vas, *engine) {
                            Ok(c) => (c, fresh_vas, Some(c)),
                            Err(e) => {
                                return Err(unwind(rm, fresh_vas.into_iter().collect(), e));
                            }
                        }
                    }
                };
                let unwind_set = || {
                    let mut v: Vec<HostHandle> = Vec::new();
                    if let Some((h, _)) = fresh_chan {
                        v.push(h);
                    }
                    v.extend(fresh_vas);
                    v
                };
                if *schedule && let Err(e) = rm.schedule(chan.0) {
                    return Err(unwind(rm, unwind_set(), e));
                }
                if let Err(e) = rm.ring_doorbell(chan.1) {
                    return Err(unwind(rm, unwind_set(), e));
                }
                Ok(VerbReply::Doorbell {
                    host_vas: fresh_vas,
                    channel: fresh_chan,
                    scheduled: *schedule,
                })
            }
            VerbPlan::EngineObject {
                host_vas,
                channel,
                engine,
                class,
                params,
            } => {
                let (chan, fresh_vas, fresh_chan) = match *channel {
                    Some(c) => (c, None, None),
                    None => {
                        let (vas, fresh_vas) = match *host_vas {
                            Some(h) => (h, None),
                            None => {
                                let h = rm.alloc_vaspace()?;
                                (h, Some(h))
                            }
                        };
                        match rm.alloc_channel(vas, *engine) {
                            Ok(c) => (c, fresh_vas, Some(c)),
                            Err(e) => {
                                return Err(unwind(rm, fresh_vas.into_iter().collect(), e));
                            }
                        }
                    }
                };
                match rm.alloc_engine_object(chan.0, *class, params) {
                    Ok(object) => Ok(VerbReply::EngineObject {
                        host_vas: fresh_vas,
                        channel: fresh_chan,
                        object,
                    }),
                    Err(e) => {
                        let mut orphans: Vec<HostHandle> = Vec::new();
                        if let Some((h, _)) = fresh_chan {
                            orphans.push(h);
                        }
                        orphans.extend(fresh_vas);
                        Err(unwind(rm, orphans, e))
                    }
                }
            }
            VerbPlan::Control { obj, cmd, payload } => {
                let mut payload = payload.clone();
                rm.control(*obj, *cmd, &mut payload)?;
                Ok(VerbReply::Control { payload })
            }
            VerbPlan::Release { unmap, free } => {
                // ★ G4 (§12.16): still best-effort — this IS the failure path and a
                // refusal must not abort the rest of the disposal — but no longer
                // SILENT. Every unmap/free that fails is carried out in the returned
                // `VerbFailure::orphans`, so "we could not dispose of this" becomes a
                // value the caller holds instead of a `let _ =` nobody can audit.
                // Unmaps first, then frees: RM auto-unmaps a resource's inter-mappings
                // inside `clientFreeResource_IMPL` before `objDelete` (`ogkm:
                // src/nvidia/src/libraries/resserv/src/rs_client.c:830-849`), so the
                // order does not protect RM — it protects OUR mirror of the mapping.
                let mut residue = Orphans::default();
                let mut first: Option<RmError> = None;
                for &(vas, va) in unmap {
                    if let Err(e) = rm.unmap_gpu_va(vas, va) {
                        first.get_or_insert(e);
                        residue.unmap.push((vas, va));
                    }
                }
                for &obj in free {
                    if let Err(e) = rm.free(obj) {
                        first.get_or_insert(e);
                        residue.free.push(obj);
                    }
                }
                match first {
                    None => Ok(VerbReply::Released),
                    Some(err) => Err(VerbFailure {
                        err,
                        orphans: residue,
                    }),
                }
            }
        }
    }

    /// Escape hatch for adapter/test code that needs the raw verb surface — scoped
    /// to a closure so no bare `&mut dyn RmBackend` escapes into a caller that might
    /// then take a lock. **Asserts R1** exactly like [`Worker::execute`].
    ///
    /// Production forwarding paths use [`VerbPlan`]; this exists so the port stays
    /// usable for bring-up probes without reopening a door that skips the assert.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1).
    pub fn with_rm<R>(&mut self, f: impl FnOnce(&mut dyn RmBackend) -> R) -> R {
        kayfabe_util::lockwitness::assert_lock_free("issuing a host RM verb");
        f(&mut *self.backend)
    }
}

/// Release `orphans` (newest first) after a mid-chain verb failure, then surface the
/// ORIGINAL error — the cleanup's own failures never *replace* the cause.
///
/// ★ G4 (§12.16): they are no longer *discarded* either. An object the unwind could
/// not free is exactly the thing that was previously "in no `Orphans`, in no core
/// state, enumerable from nothing", so it comes back in [`VerbFailure::orphans`]. On a
/// [`RmError::Interrupted`] chain the unwind still runs — the C's stub survives its own
/// `-EINTR` and keeps serving (`C: src/stub/nvkvm_stub.c:1276-1281`) — and any verb
/// that fails during it lands in the residue.
fn unwind(rm: &mut dyn RmBackend, orphans: Vec<HostHandle>, err: RmError) -> VerbFailure {
    let mut residue = Orphans::default();
    for obj in orphans {
        if rm.free(obj).is_err() {
            residue.free.push(obj);
        }
    }
    VerbFailure {
        err,
        orphans: residue,
    }
}

/// # One per-process sandboxed host worker pool
///
/// Owns a private host-RM connection with its own handle namespace, fd table, and
/// host process. Lifecycle: created at the process's earliest unambiguous signal (the
/// DUP_OBJECT dup-src registration — arch doc §4.3.4), retired in two stages
/// (`retire()` → drop) so cross-teardown consumption is impossible by construction
/// (lesson L10).
///
/// The isolate remains ONE sandboxed process per `(Proc, GpuId)` — the sandbox, the
/// RM client and the handle namespace are per-process identities and stay singular.
/// [`Worker`]s are slots inside it, each with its own 1-deep request/reply channel
/// (`l1_concurrency.md` §7.2); they share only the RM connection, which is
/// kernel-mediated. A handle minted on one worker is therefore valid on its siblings.
///
/// `Send + Sync`: owned by a `Proc` inside the shared `Gpu` (crate docs, #17).
pub trait Isolate: Send + Sync {
    /// This isolate's session id (== `ProcId`).
    fn id(&self) -> IsolateId;

    /// How many worker slots this isolate's bounded pool has (statically sized,
    /// §7.2). Never changes over the isolate's life.
    fn pool_size(&self) -> usize;

    /// How many workers are currently checked IN (available).
    fn idle_workers(&self) -> usize;

    /// ★ Check a worker OUT: mark a slot busy and move its handle to the calling
    /// thread (§7.3). Runs under device-read + proc lock — pool *bookkeeping* only,
    /// no verb.
    ///
    /// `None` means **backpressure, not failure**: the pool is saturated (or the
    /// isolate is retired and refuses new checkouts, §5.4). The caller must release
    /// ALL locks, wait, and re-enter from the top with full R5 re-validation — never
    /// spin, never wait under a lock.
    fn checkout(&mut self) -> Option<Worker>;

    /// Return a checked-out worker to its slot. Pool bookkeeping; runs under the
    /// proc lock alongside the commit phase.
    fn checkin(&mut self, worker: Worker);

    /// A worker died out of band (its reactor source signalled HUP, §7.3). Retires
    /// the slot permanently — **never a respawn**: a worker that died mid-verb may
    /// have left host state the core cannot reason about. Returns `true` if the slot
    /// was known and is now dead.
    fn worker_died(&mut self, worker: WorkerId) -> bool;

    /// ★ How many of this isolate's workers are **checked OUT right now**
    /// (`l1_concurrency.md` §12.16, gap G3) — the quantity [`Isolate::is_quiesced`]
    /// is defined on, and the one the core must ASK for rather than derive.
    ///
    /// **Why the trait must answer this and the core must not compute it.**
    /// `pool_size() - idle_workers()` looks like the same number and is not: a slot
    /// that died out of band (§7.3) is neither idle nor checked out, and it can never
    /// become either — "no resurrect". Deriving in-flight by subtraction would count
    /// every dead slot as a live round trip, so an isolate that lost one worker would
    /// report itself busy forever, defer its reap forever, and leak its GPA arena
    /// forever (the #80 class the reap exists to prevent). The implementation knows
    /// which slots are `Dead`; the core does not, and must not have to.
    fn in_flight(&self) -> usize;

    /// ★ **QUIESCED — the per-isolate SAFETY PRECONDITION for reaping** (§12.16, G3).
    ///
    /// Defined exactly, and narrowly: *no worker of this isolate is checked out.*
    /// Equivalently, every slot is idle or permanently dead, so no thread anywhere
    /// holds a [`Worker`] whose backend is this isolate's RM connection, and no verb
    /// of this isolate can still be in flight or still land. Dropping it therefore
    /// cannot tear a sandbox down underneath a live connection.
    ///
    /// ## ★ This is NOT "the device is quiescent" — do not conflate them
    ///
    /// The device-level quiesce point is a **protocol event the guest sends**, not
    /// anything inferable from worker counts and emphatically not a timer. The C
    /// measured it: `UNLOADING_GUEST_DRIVER` (GSP RPC fn=47) is emitted on **both** a
    /// real driver unload *and* a GPU-idle release when the last context exits
    /// (`C: src/qemu/nvkvm_gpu_emul.c:2450-2462`), and the reap runs at the
    /// **re-handshake** that follows it — the status-queue tx-header write — which the
    /// C names in so many words: *"the re-handshake = the quiesced point (GPU was
    /// idle-released; next context boots). Purge dead-client resolution/backing state
    /// now — never at the free."* (`C: src/qemu/nvkvm_gpu_emul.c:3458-3461`, the #14
    /// P0 fix; reaping at the client-root free instead hung the dying context's
    /// residual polls — lesson L10).
    ///
    /// So there are two distinct questions and this predicate answers only the second:
    ///
    /// - **When may the reap be attempted?** The adapter's lifecycle decision, driven
    ///   by fn-47 / the re-handshake. Belongs to L1-M2, not here, not to the core.
    /// - **Is attempting it safe for THIS isolate right now?** This predicate. The
    ///   core checks it because the adapter's edge is device-wide while the hazard is
    ///   per-`(Proc, GpuId)`: a guest process can have a verb in flight across another
    ///   process's idle-release.
    ///
    /// ## Two more things it deliberately does not mean
    ///
    /// - Not "the sandbox has exited". The adapter's `waitpid` + namespace teardown
    ///   happens **in `Drop`**, after this predicate opens the gate.
    /// - Not "every host object has been reclaimed". Reclamation is a separate
    ///   obligation with a separate ledger (G1/G2); a quiesced isolate can still own
    ///   host objects, and dropping it is what disposes of them via the session's
    ///   namespace death — the C's only backstop too (`C: src/qemu/virtio_nvgpu.c:100-118`,
    ///   the #80 session reaper force-closing the session's host fds).
    ///
    /// Getting the gate wrong is asymmetric, which is why the core **checks** rather
    /// than trusting a declaration: reaping too early tears the sandbox down under a
    /// live connection (a use-after-free); reaping too late leaks until the next
    /// quiesce point — which is the residual the C also carried and named
    /// (`C: docs/design/mode2_multiprocess_refactor_plan.md:539-541`, "mid-life
    /// multi-proc churn … keeps the pre-P0 leak-until-idle behavior"). Default:
    /// `in_flight() == 0`.
    fn is_quiesced(&self) -> bool {
        self.in_flight() == 0
    }

    /// Stage 1 of teardown: stop accepting new ops, begin quiescing in-flight work.
    /// Heavy state is reaped at the proven quiesce point, not here (lesson L10).
    fn retire(&mut self);

    /// True once `retire()` has been called (a retired isolate must refuse ops).
    fn is_retired(&self) -> bool;
}

/// ★ **The only way core state owns an [`Isolate`] — and the door R1 is asserted at
/// on the DROP side** (`l1_concurrency.md` §12.16, gap G3b).
///
/// `Worker::execute` gave R1 teeth for *verbs* (§12.8). It gave none for the other
/// blocking thing an isolate does, and that thing is not a verb: a real isolate's
/// `Drop` is `waitpid` + namespace teardown + fd close — a blocking syscall, run by
/// the compiler at a point no call site names. `Spine::reap_retired` used to perform
/// exactly that drop **inside the device write guard**, and nothing anywhere could
/// notice. That is §12.6's shape verbatim ("an assert guarding a wrapper rather than
/// the thing"), one layer over.
///
/// So every isolate the core stores lives in this newtype, and its `Drop` asserts
/// lock-freedom the same way a verb does. It is not decoration: it is the *only*
/// mechanism, because `Drop` cannot be implemented on the `dyn Isolate` trait itself
/// and an adapter's own `Drop` cannot be relied on to exist (a mock has none, and the
/// mock is what the core is tested against).
///
/// **Why it is sound to panic here.** A panic in `Drop` during an unwind aborts the
/// process, which would replace a real failure's message with a bare abort. The
/// assert is therefore skipped while this thread is already panicking
/// (`std::thread::panicking`) — the standard guard-in-`Drop` discipline. The cost is
/// exact and small: an isolate dropped under a lock *on an unwinding path* is not
/// caught. The unwinding path is not where reclamation is designed, and every
/// non-unwinding drop — which is all of production's and all of the suite's green
/// path — is.
pub struct IsolateBox(Box<dyn Isolate>);

impl IsolateBox {
    /// Take ownership of a freshly spawned isolate.
    #[must_use]
    pub fn new(isolate: Box<dyn Isolate>) -> Self {
        IsolateBox(isolate)
    }
}

impl core::fmt::Debug for IsolateBox {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IsolateBox")
            .field("id", &self.0.id())
            .field("retired", &self.0.is_retired())
            .field("in_flight", &self.0.in_flight())
            .finish()
    }
}

impl core::ops::Deref for IsolateBox {
    type Target = dyn Isolate;
    fn deref(&self) -> &(dyn Isolate + 'static) {
        &*self.0
    }
}

impl core::ops::DerefMut for IsolateBox {
    fn deref_mut(&mut self) -> &mut (dyn Isolate + 'static) {
        &mut *self.0
    }
}

impl Drop for IsolateBox {
    /// # Panics
    /// If this thread holds any ranked lock (R1) — see the type docs.
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        kayfabe_util::lockwitness::assert_lock_free(
            "dropping an isolate (sandbox teardown: waitpid + namespace unwind + fd close)",
        );
    }
}

/// Spawns isolates. The composition root holds one; per-`(Proc, GpuId)` isolates are
/// created through it so the core never knows *how* a sandbox is made.
///
/// `Send + Sync`: owned by the shared `Gpu` (crate docs, #17); spawning takes `&mut`.
pub trait IsolateFactory: Send + Sync {
    /// Spawn (or lazily reserve) the isolate for session `(id, gpu)` — one sandboxed
    /// host worker **per guest process per target GPU** (`multi_gpu_and_mig.md` item
    /// 3: a proc spanning two GPUs gets distinct isolates, so a bug forwarding its
    /// GPU0 traffic cannot reach its GPU1 host handles — the #14 blast-radius
    /// boundary lifted onto the GPU axis).
    fn spawn(&mut self, id: IsolateId, gpu: GpuId) -> Box<dyn Isolate>;
}

// The concurrency contract, compile-time-asserted (decision #17). `dyn RmBackend`
// is the one documented Send-only exception (crate docs).
kayfabe_util::assert_send_sync!(
    HostHandle,
    RmError,
    IsolateId,
    WorkerId,
    VerbPlan,
    VerbReply,
    Orphans,
    VerbFailure,
    dyn Isolate,
    dyn IsolateFactory,
    IsolateBox
);
// The backend and the `Worker` that owns one: `Send + Sync` because pool slots live
// inside the `Sync` core (crate docs), even though no call path ever shares one.
kayfabe_util::assert_send_sync!(dyn RmBackend, Worker);
