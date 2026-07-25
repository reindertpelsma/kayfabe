//! # nvkvm-isolate — the per-process unprivileged-sandbox port
//!
//! Two abstract seams live here:
//!
//! - [`RmBackend`] — **RM verbs, not ioctls** (arch doc §4.2, crate `nvkvm-rm` folded
//!   into this crate until the wire protocol lands): the unprivileged host-RM
//!   operation surface an isolate can issue. Abstract by design — the Windows-host
//!   door stays open, and NO real NVOS struct/ioctl number appears here (Axis A is
//!   quarantined to `nvkvm-abi`).
//! - [`Isolate`] / [`IsolateFactory`] — the per-guest-process sandboxed host worker.
//!   One isolate per `Proc` (`session_id == ProcId`), giving **blast-radius
//!   containment**: a bug forwarding process A cannot touch process B's host
//!   handles/mappings (threat-model boundary 2, arch doc §4.3.5).
//!
//! The real implementation (spawn, `CLONE_NEW*` namespaces, pivot_root, seccomp,
//! socket wire protocol — the Mode-1 stub posture) is an adapter crate concern.
//! This crate is pure interface + value types so the core and its tests never touch
//! an OS. `nvkvm-mocks::{MockRmBackend, MockIsolate}` are the test impls.
//!
//! ## Concurrency (decision #17)
//!
//! [`Isolate`] and [`IsolateFactory`] are **`Send + Sync` supertraits**: the core
//! *stores* them (each `Proc` owns its `Box<dyn Isolate>`, the `Gpu` owns the
//! factory), so they inherit the core's shareability requirement. Their `&self`
//! surface is pure reads (`id`/`is_retired`); every mutation takes `&mut self`, so
//! exclusivity comes from the caller's borrow, as everywhere in the core.
//! [`RmBackend`] is the **one documented `Send`-only exception**: it is reachable
//! exclusively through [`Isolate::rm`]`(&mut self)`, so a shared reference to one
//! never exists — requiring `Sync` would constrain real impls (socket buffers to the
//! sandboxed worker) for a capability no call path can use.

pub use nvkvm_arch::ids::ControlCmd;
use nvkvm_arch::ids::{ClassId, EngineKind, GpuId};
use nvkvm_vmm::SurfaceHandle;

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
/// `Send` (not `Sync`) — the documented exception, see crate docs: only ever
/// reached via `&mut`, so shared cross-thread references are unrepresentable.
pub trait RmBackend: Send {
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

/// # One per-process sandboxed host worker
///
/// Owns a private host-RM connection ([`RmBackend`]) with its own handle namespace,
/// fd table, and host process. Lifecycle: created at the process's earliest
/// unambiguous signal (the DUP_OBJECT dup-src registration — arch doc §4.3.4),
/// retired in two stages (`retire()` → drop) so cross-teardown consumption is
/// impossible by construction (lesson L10).
///
/// `Send + Sync`: owned by a `Proc` inside the shared `Gpu` (crate docs, #17).
pub trait Isolate: Send + Sync {
    /// This isolate's session id (== `ProcId`).
    fn id(&self) -> IsolateId;

    /// The unprivileged RM verb surface of this isolate. All host ops for the
    /// owning `Proc` go through here and nowhere else.
    fn rm(&mut self) -> &mut dyn RmBackend;

    /// Stage 1 of teardown: stop accepting new ops, begin quiescing in-flight work.
    /// Heavy state is reaped at the proven quiesce point, not here (lesson L10).
    fn retire(&mut self);

    /// True once `retire()` has been called (a retired isolate must refuse ops).
    fn is_retired(&self) -> bool;
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
nvkvm_util::assert_send_sync!(
    HostHandle,
    RmError,
    IsolateId,
    dyn Isolate,
    dyn IsolateFactory
);
nvkvm_util::assert_send!(dyn RmBackend);
