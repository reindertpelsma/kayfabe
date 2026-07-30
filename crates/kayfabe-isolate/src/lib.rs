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
use kayfabe_arch::ids::{ClassId, EngineKind, GpuId, GpuVa};
use kayfabe_vmm::SurfaceHandle;

/// A host-side RM object handle — **a raw value plus the isolate whose RM client
/// namespace it lives in** (`l1_concurrency.md` §12.26).
///
/// ## Why the isolate travels with the handle
///
/// A handle value alone is not an identity: RM mints client-scoped handles from one
/// shared base (`RS_CLIENT_HANDLE_BASE` = `0xC1D00000`:
/// `ogkm-610: src/nvidia/generated/g_resserv_nvoc.h:173`, `ogkm-580: :188` — the
/// **value** is identical, only the line moved; one `serverSetClientHandleBase` for the
/// whole driver, `ogkm-610:`/`ogkm-580: src/nvidia/src/kernel/rmapi/rmapi.c:105`, same
/// line at both), so isolate A's handle `0x…07` and isolate B's handle `0x…07`
/// are *both live and unrelated*. Using one on the other's connection therefore does
/// **not** fault — it names a different, live object. A free would destroy a bystander;
/// an unmap would tear down a bystander's mapping. That is the cross-namespace reach
/// [`kayfabe_mocks::HostLedger::free_of_unknown`] was added to detect, and the mock
/// can only detect it because the mock namespaces its fake handle *values*. A real host
/// does not, which is exactly why the fact has to live in the type rather than in the
/// backend's luck.
///
/// This is the same discipline [`kayfabe_core::gpa::GpaBlock`] already uses one plane
/// over — *"a block names the exact `ArenaId` it came from, so freeing it into a
/// different proc's arena is a loud refusal, not a silent double-issue"* (§12.20) —
/// applied to the object plane. It is the recorded-fact shape of §12.25, not a
/// branded-lifetime scheme: the fact is written exactly once, by the only party that
/// can know it (the backend that minted the handle), and [`Worker::execute`] is the one
/// consumer that must read it.
///
/// The raw value is deliberately still `u64` and still opaque: it is the host's, and
/// nothing in the core interprets it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostHandle {
    /// The isolate whose RM client namespace this handle lives in. Ordered first so
    /// a `BTreeSet<HostHandle>` groups by namespace.
    isolate: IsolateId,
    raw: u64,
}

impl HostHandle {
    /// RM's `NV01_NULL_OBJECT` — the namespace-free "no parent / no object" value.
    /// Belongs to no isolate, so the [`Worker::execute`] foreign-handle gate exempts
    /// it (a null parent is legal on every connection).
    pub const NULL: HostHandle = HostHandle {
        isolate: IsolateId::NONE,
        raw: 0,
    };

    /// Stamp `raw` as belonging to `isolate`'s namespace. **Backends only** — this is
    /// the mint, and calling it anywhere else fabricates a provenance claim.
    #[must_use]
    pub const fn new(isolate: IsolateId, raw: u64) -> Self {
        HostHandle { isolate, raw }
    }

    /// The isolate whose namespace this handle lives in.
    #[must_use]
    pub const fn isolate(self) -> IsolateId {
        self.isolate
    }

    /// The host's opaque handle value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.raw
    }

    /// Is this handle usable on `isolate`'s connection? [`Self::NULL`] is usable
    /// everywhere; every other handle belongs to exactly one namespace.
    ///
    /// ★ N3: the comparison is on the WHOLE [`IsolateId`], i.e. on `(proc, GPU)`. It
    /// used to compare the proc half only, which made a handle minted on one of a
    /// proc's GPUs indistinguishable — *to this function* — from the same value on
    /// another of its GPUs. See [`IsolateId`] for why that is a bystander hit rather
    /// than a harmless alias.
    #[must_use]
    pub const fn belongs_to(self, isolate: IsolateId) -> bool {
        if self.raw == 0
            && self.isolate.proc == IsolateId::NONE.proc
            && self.isolate.gpu.0 == IsolateId::NONE.gpu.0
        {
            return true;
        }
        self.isolate.proc == isolate.proc && self.isolate.gpu.0 == isolate.gpu.0
    }
}

impl core::fmt::Debug for HostHandle {
    /// Compact and namespace-first, because every assertion that involves a handle is
    /// really an assertion about *which* namespace it came from.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if *self == HostHandle::NULL {
            return f.write_str("HostHandle(NULL)");
        }
        write!(f, "HostHandle({:?}:{:#x})", self.isolate, self.raw)
    }
}

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
    /// ★★ **The worker never replied and the requester was RELEASED without one** —
    /// §7.5's D-state escape (`l1_os_shell.md` §7.5, the two-stage watchdog).
    ///
    /// **This is not a reply and no real backend returns it.** A host thread in
    /// uninterruptible sleep cannot be signalled awake — RM's waits are `down_write`s
    /// and busy-polls with no signal check: `_kgspRpcRecvPoll`'s loop ends in
    /// `osSpinLoop()` and tests no signal at either tag
    /// (`ogkm-610: .../gpu/gsp/kernel_gsp.c:2963-3060`, `ogkm-580: :2392-2479`. The two
    /// differ *inside* the loop — 610 classifies the timeout through
    /// `_kgspClassifyGspTimeout`/heartbeats where 580 counts three back-to-back timeouts
    /// and marks the GPU for reset — but neither tag makes the wait interruptible, which
    /// is the only part this variant rests on),
    /// which is exactly why [`RmError::Interrupted`] is a *best effort* and this variant
    /// has to exist beside it. When the watchdog's second budget expires the shell
    /// declares the worker **wedged** and, in ONE act, kills the slot, condemns the
    /// component and abandons the reply; this value is what the abandoned requester
    /// carries out. In the mock, "the socket" is a condvar and the abandon signal is the
    /// same condvar (§7.5), so the whole path is deterministic with no sleeps.
    ///
    /// ## ★ The consequence that makes it different from every other `RmError`
    ///
    /// **The unwind CANNOT run.** Every other failure — cancellation included — comes
    /// back on a worker that is still alive, so [`Worker::execute`] frees what the chain
    /// already allocated before it returns. A wedged worker cannot issue a `free`: it is
    /// still inside the host ioctl that wedged it. So the chain's intermediates come out
    /// **untouched** in [`VerbFailure::orphans`], which is the G4 premise verbatim
    /// (*"a worker that died mid-chain cannot run the unwind, and the handles it already
    /// minted are in no `Orphans` and in no core state"*). The caller must **stage** them
    /// — it must not try to dispose of them on this worker, and it must not drop them.
    ///
    /// Their disposition of record is §7.0's process boundary: the escape kills the
    /// isolate, and the kernel frees the whole RM client tree. That is a **stated**
    /// disposition, not a leak, and the honest residual §7.5 names is that the D-state
    /// host thread itself leaks until the kernel finishes its ioctl — *"what we convert
    /// is unbounded silent stall → bounded loud failure plus a leak we can name."*
    Wedged,
    /// ★ **The plan named a handle from ANOTHER isolate's namespace** — refused
    /// before a single verb ran (`l1_concurrency.md` §12.26).
    ///
    /// Not a host failure and not a guest fault: it is *our* invariant breaking. A
    /// [`HostHandle`] belongs to exactly one isolate's RM client namespace, and the
    /// same raw value is live-and-different in every other one, so issuing this verb
    /// would have operated on a **bystander object** — the cross-namespace reach
    /// boundary 2 exists to forbid. The gate runs first, so nothing was allocated and
    /// the accompanying [`VerbFailure::orphans`] is empty by construction.
    ForeignHandle {
        /// The offending handle, carrying the namespace it actually belongs to.
        handle: HostHandle,
        /// The isolate this worker would have issued the verb on.
        worker_isolate: IsolateId,
    },
    /// ★★★ **ADDRESS IDENTITY REFUSED** (`#102`, `eight_blockers_resolved.md` §1) — the
    /// backend was asked to map at a specific host GPU VA and produced a different one.
    ///
    /// A forwarded pushbuffer names **guest** virtual addresses. For the host GPU's MMU
    /// to resolve one, the mapping must exist **at that same address** in the channel's
    /// host VAS. So [`RmBackend::map_gpu_va`] takes the address as an argument and the
    /// real backend sets `DMA_OFFSET_FIXED_TRUE` (bit 15, `0x8000`; `dmaOffset` becomes
    /// **[IN]** — `C: nvkvm_gpu_emul.c:7663-7692`, *"the irreducible primitive the whole
    /// data plane rests on"*). A backend that ignores the flag and picks its own
    /// placement produces a mapping that **looks published and cannot be addressed**:
    /// the guest's copy is forwarded, the host MMU walks for the guest VA, finds
    /// nothing, and the run dies as `Xid 31 FAULT_PDE`. That is exactly the failure this
    /// error exists to convert into a loud, local refusal.
    ///
    /// It is *our* invariant breaking, not a guest fault and not a host resource
    /// condition: it means the placement request was silently downgraded. The verb chain
    /// unwinds the mapping and everything under it before surfacing this.
    PlacementRefused {
        /// The host GPU VA the plan required (the guest VA, by identity).
        want: u64,
        /// The host GPU VA the backend actually produced.
        got: u64,
    },
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

    /// Map `len` bytes of `memory` into the host GPU VA space owned by `vas`
    /// **at `at`**, returning the host GPU VA actually achieved.
    ///
    /// ★★★ **`at` is not a hint** (`#102`). The caller passes the *guest* VA and the
    /// backend must place the mapping there — the real backend by setting
    /// `DMA_OFFSET_FIXED_TRUE` (bit 15, `0x8000`) so `dmaOffset` is **[IN]** rather than
    /// [OUT] (`C: nvkvm_gpu_emul.c:7663-7692`). Address identity is what makes a
    /// forwarded pushbuffer resolve at all: the guest's commands carry guest VAs, and
    /// the host MMU walks the host VAS for exactly those numbers. A backend free to
    /// choose its own placement produces a binding that is published and unaddressable.
    ///
    /// This used to take no address at all while [`RmBackend::unmap_gpu_va`] took one —
    /// the asymmetry was the tell (`eight_blockers_resolved.md` §1).
    ///
    /// ★★ Address identity does **not** weaken #14. Two guest processes' identical guest
    /// VAs now land at the *same* host VA — inside *different* host VASes, one per
    /// `Vas`, on different isolates. Per-address-**space** separation is #14's proven
    /// fix; per-*address* separation never was, and asserting it was a wrong reading.
    ///
    /// # Errors
    /// [`RmError::PlacementRefused`] if the backend could not honour `at`. Returning a
    /// different VA is a contract violation the caller ([`Worker::execute`]) converts
    /// into that error and unwinds — it must never be adopted.
    fn map_gpu_va(
        &mut self,
        vas: HostHandle,
        memory: HostHandle,
        len: u64,
        at: GpuVa,
    ) -> Result<u64, RmError>;

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

/// ★★ Session identity of an isolate — the **`(Proc, GpuId)` pair**, because that is
/// what an isolate *is* (`mode2_multiprocess_isolate.md`; MG-5, decision #29/#30: one
/// sandboxed host process per `(Proc, GpuId)`), and an identity must name the thing it
/// identifies.
///
/// ## Why the GPU is part of the identity, and not a field beside it (N3)
///
/// It used to be `IsolateId(ProcId)` alone, with the target GPU carried separately —
/// and a proc that legally spans two GPUs (one guest process with a `Device` on each
/// `deviceInstance`) then had **two isolates wearing one id**. Two consequences, both
/// measured before this changed:
///
/// 1. [`HostHandle::belongs_to`] answered `true` for a handle minted in the proc's GPU0
///    isolate presented on its GPU1 connection, so [`Worker::execute`]'s foreign-handle
///    gate — documented as "the ONE place the `(Proc, GpuId)`-scoped-handle rule is
///    enforced" — enforced only the `Proc` half. Nothing downstream closes that: the
///    two isolates are two host processes with two RM clients, and RM mints handles for
///    every client from ONE base (`ogkm-610: src/nvidia/generated/g_resserv_nvoc.h:173`,
///    `ogkm-580: :188` — same `0xC1D00000` at both), so
///    the same raw value names a **different live object** in the other one. The verb
///    would have run on a bystander. (In the mock it is caught by `MockRmBackend`'s
///    per-namespace validity check — the exact "backend's luck" [`HostHandle`]'s own
///    docs say a real host does not provide.)
/// 2. Every per-isolate *account* keyed on this id merged the two: the mock's
///    `HostLedger`, its cancel census, its verb log queries. An instrument that cannot
///    separate the two isolates cannot witness a fix to (1) either.
///
/// This is the [`crate::HostHandle`] / `GpaBlock` / `ClientKey` discipline applied to
/// the isolate itself: **derived from observed protocol facts** — a `ProcId` is the
/// label of a dup-connected component of declared clients, a [`GpuId`] is a `Device`'s
/// declared `deviceInstance` — never from allocation order (decision #4).
///
/// The `proc` field is a bare `u32` rather than `kayfabe_core::ProcId` only because
/// this crate sits *below* the core in the dependency graph; [`kayfabe_core`] is the
/// one minting site and it passes `pid.0`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IsolateId {
    /// The owning proc's [`kayfabe_core::ProcId`] value. Ordered FIRST so a
    /// `BTreeMap<IsolateId, _>` groups a proc's targets together.
    proc: u32,
    /// The GPU target this isolate is the sandbox for.
    gpu: GpuId,
}

impl IsolateId {
    /// The namespace-free sentinel [`HostHandle::NULL`] wears — belongs to no proc and
    /// no GPU, so it can never compare equal to a real isolate's id.
    pub const NONE: IsolateId = IsolateId {
        proc: u32::MAX,
        gpu: GpuId(u32::MAX),
    };

    /// The isolate of `(proc, gpu)`. **The only constructor** — an `IsolateId` that
    /// names a proc without naming its target is not representable.
    #[must_use]
    pub const fn new(proc: u32, gpu: GpuId) -> Self {
        IsolateId { proc, gpu }
    }

    /// The owning proc's id value.
    #[must_use]
    pub const fn proc(self) -> u32 {
        self.proc
    }

    /// The GPU target this isolate serves.
    #[must_use]
    pub const fn gpu(self) -> GpuId {
        self.gpu
    }
}

impl core::fmt::Debug for IsolateId {
    /// `iso1/gpu0` — both halves always, because a diagnostic that prints only the proc
    /// is how the collapse this type fixed stayed invisible.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if *self == IsolateId::NONE {
            return f.write_str("iso(NONE)");
        }
        write!(f, "iso{}/gpu{}", self.proc, self.gpu.0)
    }
}

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
/// worker-lifetime races are all cost with no demonstrated benefit.
///
/// ★ **Deliberately 2–4, and deliberately NOT scaled to the vCPU count** (§12.26). The
/// old rationale here read "order of the vCPU count", which implies the pool buys wire
/// concurrency. It does not: RM serializes **every** ioctl-reachable path on the
/// per-client WRITE lock
/// (`ogkm-610:`/`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_server.c:778` and
/// seven siblings, asserted at `:786-788` — eight `_serverLockClientWithLockInfo(…,
/// LOCK_ACCESS_WRITE, …)` sites at **both** tags, seven of the eight at identical lines;
/// only the last moved, `ogkm-610: :2546` / `ogkm-580: :2468`) and takes the **global**
/// API lock in WRITE for every alloc/free
/// (`ogkm-610:`/`ogkm-580: .../rmapi/rmapi.c:53-58`, `:535`, same lines at both;
/// `ogkm-610: .../rmapi/alloc_free.c:1714-1718`, `ogkm-580: :1692-1696`), held across
/// the GSP RPC. There is **no version seam in RM's locking**, which is what makes the
/// pool-size argument below version-independent. What the pool actually
/// buys is **liveness/latency isolation** — a six-second verb must not make a sibling
/// guest thread's independent verb *appear* to hang — which is the §3.5 invariant, and
/// which saturates at a handful of workers. Past that, each extra worker is one more
/// host thread parked in D state on the same uninterruptible `down_write`.
pub const DEFAULT_POOL_WORKERS: usize = 4;

// =================================================================================
// ★★ CANCELLATION — the seam (`l1_os_shell.md` §7.1–§7.5, decision 12)
// =================================================================================

/// ★ **Why** a verb was cancelled — §7.3's *"a fault must name the truth, not the
/// symptom"* applied to cancellation itself.
///
/// It is carried all the way out to `FwdFault::Cancelled` on purpose. A cancelled verb
/// that surfaced a bare "it refused" would be indistinguishable from a host failure, and
/// a canary asserting only *"it refused"* would pass for the wrong reason — §12.10's
/// lesson, one layer over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CancelReason {
    /// The requesting guest process is going away (§7.6 T2) — it exited, was killed, or
    /// its client root was freed while a verb was in flight. The overwhelmingly common
    /// case, and the one the guest kernel drives: on process death it frees the client
    /// tree itself (measured: 178 `fn=10` RM-FREE RPCs, `mode2_bench_lifecycle.md` §5).
    ProcExit,
    /// The whole device is being torn down or reset (§7.6 T4/T7).
    DeviceReset,
    /// The two-stage verb watchdog's **first** expiry (§7.5): the verb outlived
    /// `VERB_BUDGET`, which is sized against RM's own 6 s GSP-RPC timeout and not
    /// against any measured unwind. The overwhelmingly common outcome is that the verb
    /// was merely slow and the interrupt lands.
    Watchdog,
    /// The guest thread that requested the verb took a signal or died (§5.4's founding
    /// case — the C's `#73` signal-interruptible forwarded ioctl).
    GuestSignal,
}

/// ★ A **per-checkout transaction id**. `l1_concurrency.md` §7.2: *"txn ids exist only
/// for this"* — and this is the only place they appear.
///
/// A [`CancelHandle`] is armed for exactly one txn, and a request naming a stale one is
/// **dropped**. That is the C's refinement 4 verbatim (*"main thread only signals the
/// worker if it is still on that txn_id"*), and the C needed it because without it a
/// cancel races the completion and lands on an unrelated later operation — the sharpest
/// bug in this whole area, because the damage is done to an innocent op.
///
/// Minted by the isolate at [`Isolate::checkout`], monotonic per worker slot and never
/// reused, so §7.7(i)'s never-recycled-mint argument covers it too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Txn(pub u64);

/// What a discharged [`CancelRequest`] asks the isolate to do out of band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signal {
    /// Interrupt the in-flight ioctl (§7.2's break signal).
    Interrupt(CancelReason),
    /// Abandon the reply (§7.5's escape) — only ever paired with condemnation.
    Abandon,
}

/// ★ The **out-of-band delivery seam** (`l1_os_shell.md` §7.2).
///
/// Cancellation is *never* delivered on the request/reply channel: that channel is
/// 1-deep and desynchronising it means the next checkout of the worker reads the
/// previous transaction's reply as its own — *"ours would be worse than the C's
/// use-after-free: silent cross-transaction corruption."* So the real adapter writes a
/// byte on the worker's **control pipe** and the isolate's control thread `tgkill`s the
/// worker thread, whose handler is installed **without `SA_RESTART`** (with it, the host
/// kernel silently restarts the ioctl, `EINTR` never surfaces, and *"cancellation appears
/// to work and does nothing"*).
///
/// That is all OS, so it lives behind this port. The core and its tests see only:
/// *deliver a fact to a txn, and ask what the verb actually observed*.
pub trait CancelSink: Send + Sync + core::fmt::Debug {
    /// Deliver a break signal for `txn`. Returns `true` if it was armed — `false` means
    /// the txn was **stale** (the verb already completed and the worker was checked back
    /// in), which is not an error: §7.3's fourth row, *"the verb finished first"*.
    ///
    /// Must not block. Called with **no lock held** — [`CancelRequest::discharge`]
    /// asserts exactly that, because firing a cancel is a syscall and §6.2 forbids
    /// syscalls under locks.
    fn deliver(&self, txn: Txn, reason: CancelReason) -> bool;

    /// §7.5's escape: release the requester **without a reply**. Same staleness rule and
    /// same lock-freedom requirement as [`CancelSink::deliver`].
    ///
    /// Safe only because the slot is retired in the same act, so no future reader of that
    /// channel exists (§7.2). An implementation that abandons without the condemnation
    /// has reintroduced the desync hazard.
    fn abandon(&self, txn: Txn) -> bool;

    /// What the verb **actually observed**, if anything — read by the executing thread
    /// itself, lock-free, straight after [`Worker::execute`] returns.
    ///
    /// Deliberately *observed* and not *requested*: a cancel that was delivered but that
    /// the host ioctl never noticed (RM's waits are mostly uninterruptible) is a request
    /// that lost, and reporting it as the cause of a verb that succeeded would be a lie
    /// in the one place §7.3 says must carry the truth.
    fn observed(&self) -> Option<CancelReason>;
}

/// ★ **The cancel capability, separated from the `&mut Worker`** (`l1_os_shell.md` §7.1).
///
/// The thread that could cancel is never the thread that holds the worker — the holder
/// is blocked inside the verb. So the capability must be reachable *without* a reference
/// to the worker or its backend, which §12.8 deliberately made unrepresentable.
///
/// ```text
///   Isolate::checkout()  -> Worker         (moves the backend OUT)
///                        +  CancelHandle   (stays in the pool slot, under the proc lock)
/// ```
///
/// `Send + Sync`, and it holds **no** reference to the [`Worker`] or the [`RmBackend`] —
/// it identifies `(isolate, worker, txn)` and owns one delivery sink, and nothing else.
#[derive(Debug, Clone)]
pub struct CancelHandle {
    isolate: IsolateId,
    worker: WorkerId,
    txn: Txn,
    sink: std::sync::Arc<dyn CancelSink>,
}

impl CancelHandle {
    /// Arm a handle for `txn` on slot `worker` of `isolate` (isolate implementations
    /// only — minting one elsewhere fabricates a cancellation authority).
    #[must_use]
    pub fn new(
        isolate: IsolateId,
        worker: WorkerId,
        txn: Txn,
        sink: std::sync::Arc<dyn CancelSink>,
    ) -> Self {
        CancelHandle {
            isolate,
            worker,
            txn,
            sink,
        }
    }

    /// The isolate whose pool this slot belongs to.
    #[must_use]
    pub fn isolate(&self) -> IsolateId {
        self.isolate
    }

    /// The pool slot.
    #[must_use]
    pub fn worker(&self) -> WorkerId {
        self.worker
    }

    /// The transaction this handle is armed for.
    #[must_use]
    pub fn txn(&self) -> Txn {
        self.txn
    }

    /// **Latch** an interrupt for this txn. Pure bookkeeping — nothing is signalled
    /// until [`CancelRequest::discharge`], which is what makes this legal under the proc
    /// lock (§7.1: *"firing a cancel is a syscall, and §6.2 forbids syscalls under
    /// locks"*).
    pub fn request(&self, reason: CancelReason) -> CancelRequest {
        self.at(Signal::Interrupt(reason))
    }

    /// Latch §7.5's **abandon**. Only legal as half of the wedge escape, whose other
    /// half — killing the slot and condemning the component — must happen in the *same*
    /// act; see [`CancelSink::abandon`].
    pub fn abandon(&self) -> CancelRequest {
        self.at(Signal::Abandon)
    }

    fn at(&self, signal: Signal) -> CancelRequest {
        CancelRequest {
            isolate: self.isolate,
            worker: self.worker,
            txn: self.txn,
            signal,
            sink: std::sync::Arc::clone(&self.sink),
        }
    }
}

/// ★ A **latched** cancellation — the same two-step shape as
/// `kayfabe_core::reactor::WakeRequest`, and for the same reason (§7.1: *"the mechanism
/// already exists for wake and timer; cancel is the third user, which is the argument
/// that it is the right mechanism rather than a third one"*).
///
/// `#[must_use]`, and it earns it exactly as `Orphans` does: a latched cancel that is
/// never discharged is **a cancellation that silently did not happen** — the failure mode
/// §7.2 refinement 1 names in so many words. The compiler is the only thing that
/// reliably notices.
#[must_use = "a latched cancel that is never discharged is a cancellation that silently \
              did not happen — discharge it (with no lock held) or hand it onward."]
#[derive(Debug, Clone)]
pub struct CancelRequest {
    isolate: IsolateId,
    worker: WorkerId,
    txn: Txn,
    signal: Signal,
    sink: std::sync::Arc<dyn CancelSink>,
}

impl CancelRequest {
    /// The isolate this request names.
    #[must_use]
    pub fn isolate(&self) -> IsolateId {
        self.isolate
    }

    /// The pool slot this request names.
    #[must_use]
    pub fn worker(&self) -> WorkerId {
        self.worker
    }

    /// The transaction this request is armed for; a delivery naming a stale one is
    /// dropped by the sink.
    #[must_use]
    pub fn txn(&self) -> Txn {
        self.txn
    }

    /// The reason, for an interrupt; `None` for §7.5's abandon.
    #[must_use]
    pub fn reason(&self) -> Option<CancelReason> {
        match self.signal {
            Signal::Interrupt(r) => Some(r),
            Signal::Abandon => None,
        }
    }

    /// True if this is §7.5's abandon rather than an ordinary interrupt.
    #[must_use]
    pub fn is_abandon(&self) -> bool {
        matches!(self.signal, Signal::Abandon)
    }

    /// ★ **Fire it.** Returns `true` if the txn was still current — `false` means the
    /// verb finished first (§7.3's fourth row), which is a normal outcome and never a
    /// failure.
    ///
    /// # Panics
    /// If this thread holds any ranked lock. Firing a cancel is a syscall (a pipe write
    /// plus a `tgkill`), and R1 admits no exception for it; this assert is why the
    /// latch/discharge split is structural rather than advisory.
    pub fn discharge(self) -> bool {
        kayfabe_util::lockwitness::assert_lock_free("discharging a cancel request");
        match self.signal {
            Signal::Interrupt(reason) => self.sink.deliver(self.txn, reason),
            Signal::Abandon => self.sink.abandon(self.txn),
        }
    }
}

/// ★ A batch of latched cancels on their way out of a locked phase — `#[must_use]` on
/// the **collection**, because `Vec<CancelRequest>` is not (dropping the `Vec` drops
/// every request without a single warning).
///
/// This is the shape §15 amendment 4 asks for: *"retire **requests** cancellation for
/// every checked-out worker (latched, discharged lock-free)"*.
#[must_use = "latched cancels that are never discharged are cancellations that silently \
              did not happen — call `discharge_all()` with no lock held, or `absorb` \
              them into a batch that will be."]
#[derive(Debug, Clone, Default)]
pub struct Cancels(Vec<CancelRequest>);

impl Cancels {
    /// An empty batch.
    pub fn new() -> Self {
        Cancels(Vec::new())
    }

    /// Latch one more.
    pub fn push(&mut self, req: CancelRequest) {
        self.0.push(req);
    }

    /// Take `other`'s requests into this batch — the way a locked phase hands its
    /// latches up to the shell that will discharge them.
    pub fn absorb(&mut self, other: Cancels) {
        self.0.extend(other.0);
    }

    /// Read the latched requests without consuming them (assertions and diagnostics).
    pub fn requests(&self) -> &[CancelRequest] {
        &self.0
    }

    /// ★ Fire every latched request. Returns how many were **delivered** — i.e. how many
    /// named a txn that was still current; the rest lost the race with their own verb's
    /// completion, which is §7.3's fourth row and not a failure.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (see [`CancelRequest::discharge`]).
    pub fn discharge_all(self) -> usize {
        self.0
            .into_iter()
            .map(CancelRequest::discharge)
            .filter(|&delivered| delivered)
            .count()
    }
}

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
    /// allocate sysmem and map it into that VAS **at the guest VA** (`#102`).
    Publish {
        /// The Vas's already-materialized host VAS, or `None` to allocate one.
        host_vas: Option<HostHandle>,
        /// Bytes to allocate and map.
        len: u64,
        /// ★★★ The guest VA this range must be addressable at. Address identity: the
        /// mapping is placed HERE, not wherever the host driver would have chosen.
        at: GpuVa,
    },
    /// The doorbell chain: (optionally) host VAS → (optionally) host channel →
    /// (optionally) schedule → ring.
    ///
    /// ★★ **`#[non_exhaustive]`, and that is the #14 ring-gate's teeth** (added
    /// 2026-07-27; `ARCHITECTURE.md` invariant 5). This variant has **no struct
    /// expression outside this crate** — `VerbPlan::Doorbell { … }` written anywhere else
    /// is a compile error (E0639), pinned by `tests/ui/ungated_doorbell.rs`. The only
    /// way to obtain one is [`VerbPlan::gated_doorbell`], **which runs the gate**.
    ///
    /// Before this, the invariant was a property of the production *call graph* — true,
    /// but unenforced: `kayfabe_fwd::plan_doorbell` was the only thing that built one,
    /// while nothing stopped a caller hand-building the variant and handing it to a
    /// checked-out [`Worker`], whose own gate is the foreign-handle check and **not** the
    /// #14 working-set check. That is a reachable door — `tests/tests/cross_proc_lifetime.rs`
    /// went through it — and a boundary that depends on nobody noticing it is not a
    /// boundary. It is the only variant with this shape: every other variant's gate is
    /// [`Worker::execute`]'s central foreign-handle check, which runs whoever built the
    /// plan.
    #[non_exhaustive]
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

/// ★★ The #14 ring-gate's view of **one channel's `Vas`** — the address-plane
/// abstraction [`VerbPlan::gated_doorbell`] runs the gate over.
///
/// This crate cannot name `kayfabe_mmu::AddressTable` (the mmu depends on *this* crate,
/// not the other way round — see `kayfabe-mmu`'s `Cargo.toml`), and it must not: the
/// isolate port has no business knowing the shape of a page table. What it *can* insist
/// on is that a ring plan is never built without an address plane being consulted, and
/// that is exactly this one predicate.
///
/// Implemented in `kayfabe-fwd` over `(&AddressTable, Pdb)` — the channel's own `Vas`,
/// keyed by PDB, which is what makes two guest processes' *identical* guest VAs resolve
/// into disjoint host VASes (#14's proven fix). Deliberately **not** `Send + Sync`: it is
/// an argument borrowed for the duration of one call, never core-stored state.
pub trait RingWorkingSet {
    /// Is `va` forward-populated in **this channel's own `Vas`** *and* host-published
    /// into that `Vas`'s own host VAS?
    ///
    /// `false` is the total answer for *both* misses — no mapping at all, and a mapping
    /// with no host publication. The caller (`kayfabe-fwd`) owns the exact fault
    /// vocabulary and re-derives which one it was from the offending VA in
    /// [`UngatedVa`], so this predicate stays a predicate and the two crates cannot
    /// drift into two classifications of the same miss.
    fn is_host_published(&self, va: GpuVa) -> bool;
}

/// The #14 ring-gate's refusal: the **first** working-set VA that is not host-published
/// in the ringing channel's own `Vas`.
///
/// Carries the VA rather than a bare "no", so the caller's fault names the address the
/// guest actually asked for — MISS = FAULT, loud and exact, never a cross-proc
/// content-pick (the confused deputy #14 designed out).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UngatedVa(pub GpuVa);

impl VerbPlan {
    /// ★★★ **THE ONLY constructor of [`VerbPlan::Doorbell`], and it IS the #14
    /// ring-gate.** (`ARCHITECTURE.md` invariant 5, `execution_plane.md` §2.4.)
    ///
    /// Every `va` in `working_set` must be host-published in `vas` — the ringing
    /// channel's *own* `Vas` — or this returns [`UngatedVa`] naming the first one that
    /// is not, and **no plan exists to hand a [`Worker`]**. The gate therefore cannot be
    /// forgotten, skipped, or reordered after a host op: there is no plan to execute
    /// until it has passed, and the variant it produces has no struct expression outside
    /// this crate.
    ///
    /// ## What this does and does not prove
    ///
    /// It proves the gate **ran**, over whatever address plane the caller supplied, for
    /// every VA the submission claimed. It cannot prove the address plane is the real
    /// one — a caller free to implement [`RingWorkingSet`] is equally free to fabricate a
    /// `Proc`, and Rust's privacy unit is the crate, so "only `kayfabe-fwd` may call
    /// this" is not expressible in the type system. What changed is the failure *mode*:
    /// bypassing the gate is no longer *omission* (build the struct, forget the check),
    /// which is what an ordinary edit looks like, but *commission* (write a lying
    /// address plane), which is what a review notices.
    ///
    /// An **empty** `working_set` passes, and that is correct rather than a hole: the
    /// working set is what the submission *claims to touch*, a GSP-managed
    /// system-routed channel legitimately claims nothing, and the same empty set passes
    /// through `kayfabe_fwd::plan_doorbell` today. The gate's content is *"every claimed
    /// VA is published in THIS Vas"*, never *"something was claimed"*.
    ///
    /// # Errors
    /// [`UngatedVa`] — the first working-set VA with no host publication in `vas`.
    pub fn gated_doorbell(
        vas: &dyn RingWorkingSet,
        working_set: &[GpuVa],
        host_vas: Option<HostHandle>,
        channel: Option<ChannelHandles>,
        engine: EngineKind,
        schedule: bool,
    ) -> Result<VerbPlan, UngatedVa> {
        if let Some(&va) = working_set.iter().find(|&&va| !vas.is_host_published(va)) {
            return Err(UngatedVa(va));
        }
        Ok(VerbPlan::Doorbell {
            host_vas,
            channel,
            engine,
            schedule,
        })
    }

    /// Every [`HostHandle`] this plan would issue a verb *against* — the input side
    /// only (handles the chain will mint do not exist yet, and are this isolate's by
    /// construction).
    ///
    /// This exists so the foreign-handle gate is ONE central enumeration rather than a
    /// `matches!` each new variant could forget: adding a variant that carries a handle
    /// and not listing it here is the mistake, and it is a mistake in exactly one file.
    #[must_use]
    pub fn handles(&self) -> Vec<HostHandle> {
        match self {
            VerbPlan::Publish { host_vas, .. } => host_vas.iter().copied().collect(),
            VerbPlan::Doorbell {
                host_vas, channel, ..
            }
            | VerbPlan::EngineObject {
                host_vas, channel, ..
            } => host_vas
                .iter()
                .copied()
                .chain(channel.map(|(h, _)| h))
                .collect(),
            VerbPlan::Control { obj, .. } => vec![*obj],
            VerbPlan::Release { unmap, free } => unmap
                .iter()
                .map(|&(vas, _)| vas)
                .chain(free.iter().copied())
                .collect(),
        }
    }
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
/// children and dependents ahead of parents
/// (`ogkm-610:`/`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_server.c:963-981` —
/// byte-identical, same lines; and `ogkm-610: .../rs_client.c:1086-1122`,
/// `ogkm-580: :1085-1121`) and auto-unmaps a resource's inter-mappings inside
/// `clientFreeResource_IMPL` *before* `objDelete`
/// (`ogkm-610:`/`ogkm-580: .../rs_client.c:830-849` — same lines, byte-identical at both
/// tags: the unmaps are `:835-837`, `objDelete` is `:849`). So RM
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
/// it, most RM waits are *not* interruptible in the first place
/// (`ogkm-610:`/`ogkm-580: kernel-open/nvidia/nv.c` carries exactly **six**
/// `*_interruptible` call sites at each tag, in the same three functions — the PM-lock
/// read and the open-complete wait — out of 6 533 / 6 312 lines respectively), so a
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
/// `Send + Sync` — compile-time-asserted at the bottom of this file, with everything
/// else the core stores (decision #17). ★ **corrected 2026-07-27**: this said *"`Send`,
/// not `Sync`"*, which the assertion in the same file refuted. The sentence was
/// describing the *usage* shape, and that part is still true and is the load-bearing
/// one: a worker is reached **only** by `&mut`, so a shared reference to one never
/// exists on any path, and single-in-flight-per-worker is the borrow checker's
/// guarantee rather than a bound's. The `Sync` bound is nonetheless real and not
/// droppable — an idle worker *sits in* its isolate's pool, inside a `Proc`, inside the
/// `Sync` `Gpu` — so a backend may hold no `Rc`/`Cell` in its private state.
pub struct Worker {
    isolate: IsolateId,
    id: WorkerId,
    backend: Box<dyn RmBackend>,
    /// The out-of-band cancel seam this worker shares with its pool slot's
    /// [`CancelHandle`]. `None` for a backend with no cancellation support (bring-up
    /// probes) — such a worker is simply never interruptible, which is honest and
    /// visible rather than silently doing nothing.
    cancel: Option<std::sync::Arc<dyn CancelSink>>,
    txn: Txn,
}

impl core::fmt::Debug for Worker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Worker")
            .field("isolate", &self.isolate)
            .field("id", &self.id)
            .field("txn", &self.txn)
            .finish()
    }
}

impl Worker {
    /// Wrap a backend as pool slot `id` of `isolate` (isolate implementations only).
    /// The `isolate` argument is what makes the foreign-handle gate in
    /// [`Worker::execute`] possible — a worker must know whose namespace it speaks.
    ///
    /// **Not cancellable** — see [`Worker::with_cancel`].
    #[must_use]
    pub fn new(isolate: IsolateId, id: WorkerId, backend: Box<dyn RmBackend>) -> Self {
        Worker {
            isolate,
            id,
            backend,
            cancel: None,
            txn: Txn(0),
        }
    }

    /// As [`Worker::new`], with the out-of-band cancel seam its pool slot's
    /// [`CancelHandle`] signals through (§7.1).
    #[must_use]
    pub fn with_cancel(
        isolate: IsolateId,
        id: WorkerId,
        backend: Box<dyn RmBackend>,
        cancel: std::sync::Arc<dyn CancelSink>,
    ) -> Self {
        Worker {
            isolate,
            id,
            backend,
            cancel: Some(cancel),
            txn: Txn(0),
        }
    }

    /// Stamp this checkout's transaction id (isolate implementations only — called from
    /// [`Isolate::checkout`], which mints it). A cancel armed for an earlier txn is
    /// dropped by the sink, which is the whole point of the id existing.
    pub fn begin_txn(&mut self, txn: Txn) {
        self.txn = txn;
    }

    /// This checkout's transaction id.
    #[must_use]
    pub fn txn(&self) -> Txn {
        self.txn
    }

    /// ★ What a cancellation actually **did** to the verb that just ran, read lock-free
    /// by the executing thread itself right after [`Worker::execute`] returns.
    ///
    /// This is how `FwdFault::Cancelled` gets its `reason` without the shell re-taking a
    /// lock to ask the isolate — and without [`RmError::Interrupted`] growing a payload,
    /// which §7.3 deliberately refuses (*"the txn is L1's business, not the core's"*).
    #[must_use]
    pub fn cancel_observed(&self) -> Option<CancelReason> {
        self.cancel.as_ref().and_then(|c| c.observed())
    }

    /// This worker's slot in its isolate's pool.
    #[must_use]
    pub fn id(&self) -> WorkerId {
        self.id
    }

    /// The isolate whose RM client namespace this worker speaks for.
    #[must_use]
    pub fn isolate(&self) -> IsolateId {
        self.isolate
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
    /// ★ **The foreign-handle gate runs FIRST** (`l1_concurrency.md` §12.26): a plan
    /// naming a handle from another isolate's namespace is refused with
    /// [`RmError::ForeignHandle`] before any verb runs, so nothing is allocated and the
    /// returned [`VerbFailure::orphans`] is empty. This is the ONE place the
    /// `(Proc, GpuId)`-scoped-handle rule is enforced, and it covers every plan shape
    /// including [`VerbPlan::Release`] — the disposal path, which is where a
    /// cross-namespace handle would otherwise `free` a **bystander** object.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1).
    pub fn execute(&mut self, plan: &VerbPlan) -> Result<VerbReply, VerbFailure> {
        kayfabe_util::lockwitness::assert_lock_free("issuing a host RM verb");
        if let Some(&handle) = plan.handles().iter().find(|h| !h.belongs_to(self.isolate)) {
            return Err(VerbFailure::bare(RmError::ForeignHandle {
                handle,
                worker_isolate: self.isolate,
            }));
        }
        let rm = &mut *self.backend;
        match plan {
            VerbPlan::Publish { host_vas, len, at } => {
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
                let host_va = match rm.map_gpu_va(vas, memory, *len, *at) {
                    Ok(va) => va,
                    Err(e) => {
                        let mut orphans = vec![memory];
                        orphans.extend(fresh_vas);
                        return Err(unwind(rm, orphans, e));
                    }
                };
                // ★★★ #102 — ADDRESS IDENTITY, checked at the seam that crosses into the
                // untrusted-by-construction backend. A backend that ignores the
                // fixed-offset request hands back a mapping that is published and
                // **unaddressable**: the forwarded pushbuffer names `at`, the host MMU
                // walks for `at`, and finds nothing (Xid 31 FAULT_PDE). Undo the mapping
                // and everything under it rather than adopting a binding whose host VA is
                // a lie. This is the ONE place a placement can be downgraded silently, so
                // it is the one place the check belongs.
                if host_va != at.0 {
                    let _ = rm.unmap_gpu_va(vas, host_va);
                    let mut orphans = vec![memory];
                    orphans.extend(fresh_vas);
                    return Err(unwind(
                        rm,
                        orphans,
                        RmError::PlacementRefused {
                            want: at.0,
                            got: host_va,
                        },
                    ));
                }
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
                // inside `clientFreeResource_IMPL` before `objDelete`
                // (`ogkm-610:`/`ogkm-580:`
                // `src/nvidia/src/libraries/resserv/src/rs_client.c:830-849` — same
                // lines at both tags), so the order does not protect RM — it protects
                // OUR mirror of the mapping.
                //
                // ★★ §7.5 — one exception to "best effort": [`RmError::Wedged`] stops
                // the loop. A wedged worker will answer every remaining verb the same
                // way, so grinding through the list buys nothing and models the wrong
                // thing (the real worker is not answering at all). The untried remainder
                // is residue exactly like the tried-and-failed part, so the RESULT is
                // identical — what differs is that we do not pretend to have asked.
                let mut residue = Orphans::default();
                let mut first: Option<RmError> = None;
                for (i, &(vas, va)) in unmap.iter().enumerate() {
                    match rm.unmap_gpu_va(vas, va) {
                        Ok(()) => {}
                        Err(RmError::Wedged) => {
                            residue.unmap.extend_from_slice(&unmap[i..]);
                            residue.free.extend_from_slice(free);
                            return Err(VerbFailure {
                                err: RmError::Wedged,
                                orphans: residue,
                            });
                        }
                        Err(e) => {
                            first.get_or_insert(e);
                            residue.unmap.push((vas, va));
                        }
                    }
                }
                for (i, &obj) in free.iter().enumerate() {
                    match rm.free(obj) {
                        Ok(()) => {}
                        Err(RmError::Wedged) => {
                            residue.free.extend_from_slice(&free[i..]);
                            return Err(VerbFailure {
                                err: RmError::Wedged,
                                orphans: residue,
                            });
                        }
                        Err(e) => {
                            first.get_or_insert(e);
                            residue.free.push(obj);
                        }
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
    // ★★ §7.5 — a WEDGED worker cannot run its own unwind. It is still inside the host
    // ioctl that wedged it; issuing a `free` on it would either block forever behind the
    // same uninterruptible wait or desynchronise a channel whose reply was abandoned.
    // So the intermediates come out UNTOUCHED, which is G4's premise made a value: the
    // caller must stage them, and their disposition of record is §7.0's process
    // boundary. Attempting the frees here is not merely useless — it is the shape that
    // turns a bounded loud failure into a second wedge.
    if err == RmError::Wedged {
        return VerbFailure {
            err,
            orphans: Orphans {
                unmap: Vec::new(),
                free: orphans,
            },
        };
    }
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

    /// ★ Which pool slots are **checked out right now**, by id — §7.6 T2's *"for every
    /// checked-out worker, `request_cancel(ProcExit)`"* needs the list, not the count.
    ///
    /// The same argument as [`Isolate::in_flight`] applies: the implementation knows
    /// which slots are `Dead` and the core does not, so deriving this by subtraction
    /// from the pool width would name slots that can never answer a cancel.
    fn checked_out(&self) -> Vec<WorkerId>;

    /// ★ The [`CancelHandle`] of slot `worker`, or `None` if that slot is not checked out
    /// (there is nothing to cancel) or does not exist.
    ///
    /// Reachable through `&self`, under the proc lock, **without touching the
    /// [`Worker`]** — which is the entire point of §7.1: the thread that could cancel is
    /// never the thread that holds the worker, because the holder is blocked inside the
    /// verb.
    fn cancel_handle(&self, worker: WorkerId) -> Option<CancelHandle>;

    /// **Latch** an interrupt for slot `worker` (§7.1). Nothing is signalled here: the
    /// returned [`CancelRequest`] is discharged by the shell after the guards drop.
    ///
    /// `None` if the slot is not checked out — a cancel with nothing to cancel is a
    /// no-op, not a failure.
    fn request_cancel(&mut self, worker: WorkerId, reason: CancelReason) -> Option<CancelRequest> {
        self.cancel_handle(worker).map(|h| h.request(reason))
    }

    /// ★ Latch an interrupt for **every** checked-out worker — the door
    /// `Proc::retire`/`Proc::vacate` use (§7.6 T2, §15 amendment 4).
    fn request_cancel_all(&mut self, reason: CancelReason) -> Cancels {
        let slots = self.checked_out();
        let mut out = Cancels::new();
        for w in slots {
            if let Some(h) = self.cancel_handle(w) {
                out.push(h.request(reason));
            }
        }
        out
    }

    /// ★ §7.5's escape, latched: release slot `worker`'s requester **without a reply**.
    ///
    /// The caller must kill the slot ([`Isolate::worker_died`]) and condemn the component
    /// in the *same act*; abandoning without that reintroduces the channel-desync hazard
    /// §7.2 forbids, because a future reader of that channel would misread the stale
    /// reply. The two must not be reorderable steps.
    fn abandon(&mut self, worker: WorkerId) -> Option<CancelRequest> {
        self.cancel_handle(worker).map(|h| h.abandon())
    }

    /// A worker died out of band (its reactor source signalled HUP, §7.3). Retires
    /// the slot permanently — **never a respawn**. Respawning the slot would be
    /// pointless anyway: the whole component is condemned by the same event
    /// (`l1_concurrency.md` §12.13), because the guest's published data lived in host
    /// memory owned by this isolate's RM client and died with it, so there is nothing
    /// for a fresh worker to serve except zeroes. Returns `true` if the slot was known
    /// and is now dead.
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
    /// Spawn (or lazily reserve) the isolate for session `id` — one sandboxed host
    /// worker **per guest process per target GPU** (`multi_gpu_and_mig.md` item 3: a
    /// proc spanning two GPUs gets distinct isolates, so a bug forwarding its GPU0
    /// traffic cannot reach its GPU1 host handles — the #14 blast-radius boundary
    /// lifted onto the GPU axis).
    ///
    /// ★ N3: the target GPU rides **inside** [`IsolateId`] rather than beside it. This
    /// used to be `spawn(id, gpu)`, which made "the id says one target, the sandbox was
    /// built for another" a representable state — and the handles the sandbox then
    /// minted would be stamped with the wrong namespace, which is precisely the fact
    /// [`Worker::execute`]'s gate reads. One argument, no disagreement possible.
    fn spawn(&mut self, id: IsolateId) -> Box<dyn Isolate>;
}

// The concurrency contract, compile-time-asserted (decision #17).
//
// ★ **corrected 2026-07-27.** This comment said `dyn RmBackend` "is the one documented
// Send-only exception (crate docs)" — twenty lines above the `assert_send_sync!` that
// asserts the opposite. It is not an exception and has not been one since the §7.2
// worker pool: an `Isolate` OWNS N idle `Worker`s, a `Proc` owns the isolate, and the
// core's `Gpu` is `Sync`, so every boxed backend sitting in a pool slot must be `Sync`
// for that chain to hold. `RmBackend`'s own supertrait bound (`Send + Sync`, above)
// has been the truth all along. **This workspace has NO Send-only exception**; the two
// `assert_send!` sites that exist (`dyn Vmm`, `Reactor`) are ARGUMENT-passed ports the
// core never stores, which is a different property entirely.
//
// `RingWorkingSet` is deliberately absent for that same reason: it is borrowed for the
// duration of one `VerbPlan::gated_doorbell` call and never stored, so bounding it would
// price a property nothing needs onto every address plane that wants to be gateable.
kayfabe_util::assert_send_sync!(
    HostHandle,
    RmError,
    IsolateId,
    WorkerId,
    Txn,
    CancelReason,
    CancelHandle,
    CancelRequest,
    Cancels,
    dyn CancelSink,
    VerbPlan,
    UngatedVa,
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
