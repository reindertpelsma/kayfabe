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

use nvkvm_arch::ids::ClassId;

/// A host-side RM object handle, scoped to ONE isolate's handle namespace.
///
/// Handles from different isolates must never be interchangeable — the mock backend
/// enforces this in tests (boundary-2 blast-radius assertion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostHandle(pub u64);

/// An opaque RM control command identifier. Values are Axis-A (versioned, codegen'd
/// in `nvkvm-abi`); the core routes them, the backend interprets them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlCmd(pub u32);

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
pub trait RmBackend {
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

    /// Intent verb: allocate a host GPU channel bound to host VAS `vas`.
    /// Returns `(channel_handle, host_work_submit_token)`.
    fn alloc_channel(&mut self, vas: HostHandle) -> Result<(HostHandle, u64), RmError>;

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
    fn map_gpu_va(
        &mut self,
        vas: HostHandle,
        memory: HostHandle,
        len: u64,
    ) -> Result<u64, RmError>;

    /// Unmap a previous [`RmBackend::map_gpu_va`].
    fn unmap_gpu_va(&mut self, vas: HostHandle, gpu_va: u64) -> Result<(), RmError>;

    /// Ring the host work-submit doorbell with an (already host-translated) token.
    fn ring_doorbell(&mut self, host_token: u64) -> Result<(), RmError>;
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
pub trait Isolate {
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

/// Spawns isolates. The composition root holds one; per-`Proc` isolates are created
/// through it so the core never knows *how* a sandbox is made.
pub trait IsolateFactory {
    /// Spawn (or lazily reserve) the isolate for session `id`.
    fn spawn(&mut self, id: IsolateId) -> Box<dyn Isolate>;
}
