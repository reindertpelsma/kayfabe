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
//!   One isolate per `Proc` (`session_id == ProcId`).
//!
//! ## ★★★ Why one isolate per guest process — read this before changing the granularity
//!
//! ⊘ **Not primarily for blast radius.** The founding reason is **VA identity**:
//! NVIDIA's driver, and UVM especially, treats the address a buffer has in the
//! *calling process* as *the* address. `nvproxy` gets that for free because a guest
//! process **is** a host process; in a VM it is not, and that is exactly why porting
//! `nvproxy` to QEMU was believed impossible. One host process per guest process
//! restores the identity, and `#14` **is** the collision that follows when it is lost;
//! `#102` is the tree asserting its negation.
//!
//! ★ **State it precisely, because the loose version is refutable.** An address passed
//! as an *argument* IS re-addressable — every NVIDIA object can be shared out of an
//! isolate and re-mapped at an arbitrary VMM address, which is how Mode 2 passed LLM
//! compute. What cannot be re-addressed is an address the driver takes **implicitly
//! from the calling process**: `mm` is not a parameter. UVM is the clean case — managed
//! memory's contract is CPU VA == GPU VA and its `va_space` is bound to the caller's
//! `mm`, so two guest processes with overlapping managed VAs cannot both be registered
//! in one host `mm`. ⊘ The GPU-VAS argument does *not* carry this on its own: one RM
//! client may own many `VASpace` objects.
//!
//! ⊘ **CORRECTED — do not repeat the strong version of this.** An earlier revision
//! said UVM's `mm` binding is enforced by the driver *by error code*, citing the C's
//! `SCM_RIGHTS` rejection. The rejection was real, but `uvm_api_mm_initialize`
//! contains **no `current->mm` comparison** in 580, 610 or 575
//! (`ogkm-580: kernel-open/nvidia-uvm/uvm.c:59-137`) — the C's stated mechanism is a
//! reconstruction. The `mm` is captured at `UVM_INITIALIZE`
//! (`uvm_va_space_mm.c:195`) and is **skipped entirely** under
//! `UVM_INIT_FLAGS_MULTI_PROCESS_SHARING_MODE` (`uvm_va_space_mm.c:172-179`) — which
//! gVisor's `nvproxy` forcibly sets on every app precisely to share one host UVM fd
//! (`gvisor/pkg/sentry/devices/nvproxy/uvm.go:188-200`).
//!
//! ⇒ What survives is an **allocator** argument, not a kernel-refusal one: UVM VA
//! ranges are identity-mapped and `MAP_EXTERNAL_ALLOCATION` carries raw VAs, so one
//! shared `va_space` means one flat host VA allocator in which two guest processes at
//! the same VA collide — `#14` again. ★ The strongest argument is therefore the RM
//! **namespace** one below (a new host process with a new `hClient` is separation RM
//! performs, not separation we assert). Full treatment, including what was
//! overstated: `docs/design/isolate_founding_rationale.md` §1c.
//!
//! ⇒ **Coarsening this granularity — one isolate shared by two guest processes —
//! reintroduces `#14` no matter what it does for security.** Blast-radius containment
//! (a bug forwarding process A cannot touch process B's host handles/mappings —
//! threat-model boundary 2, arch doc §4.3.5) is real and is the *third* reason, not the
//! first. Full ordering and its history: `docs/design/isolate_founding_rationale.md`.
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
use kayfabe_vmm::{Prot, SurfaceHandle};

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
    /// ★★★ **THE NAMED BOUNDARY of the isolate-performs-the-mapping ruling** — the bytes
    /// asked for are the real device's, and there is no way to hand them to the VMM as
    /// *memory* ([`RmBackend::export_backing`], `isolate_vmm_fd_crossing.md` §12).
    ///
    /// This is not "not built yet" and it is not a host failure. It is the isolate saying
    /// *"I performed no mapping, because the only thing I could have handed you is a
    /// descriptor you could `ioctl`, and that is the thing this verb exists to stop
    /// crossing."* Three facts, together, make it a decision rather than a gap:
    ///
    /// 1. The only object whose `mmap` yields a host GPU BAR page is `/dev/nvidia<N>`
    ///    carrying a registered mapping context — a **character device**, and RM assigns
    ///    `secInfo.privLevel` from `osIsAdministrator()` at the top of **every** escape
    ///    (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:304`, sole occurrence), so
    ///    the same descriptor is unprivileged in the isolate and privileged in a root VMM
    ///    (`guest_blast_radius.md` F14).
    /// 2. NVIDIA's own dma-buf export — the one route that would cross a *non*-RM
    ///    descriptor — hard-gates CPU mapping to integrated parts:
    ///    `*pbCanMmap = pGpu->getProperty(pGpu, PDB_PROP_GPU_ZERO_FB)`
    ///    (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osapi.c:5609`), and
    ///    `nv_dma_buf_mmap` refuses when it is false
    ///    (`ogkm-580: kernel-open/nvidia/nv-dmabuf.c:1246-1250`). On every discrete part
    ///    this project targets, a dma-buf of device memory **cannot be mapped by the CPU**
    ///    at all.
    /// 3. Our own memory plane already refuses the result independently:
    ///    `kayfabe_linux_raw::GuestWindow::place` rejects `Backing::DeviceFile` with
    ///    `RawError::DeviceBackingNotPlaceable`.
    ///
    /// ⊘ **A caller must never downgrade this to "map it somewhere else".** The bytes are
    /// on the card; a mapping of different bytes is worse than no mapping, which is the
    /// same judgement `kayfabe_vmm_qemu::viewer_install` already makes when it refuses to
    /// install rather than approximate.
    NotExportableAsMemory {
        /// The RM object whose pages were asked for.
        memory: HostHandle,
    },
    /// ★★★ **This isolate has no guest-RAM descriptor**, so a [`GuestRamGrant`] cannot be
    /// honoured — and it is a *deployment* fact, not a bug and not a host resource
    /// condition (`mode2_isolate_memory_boundary.md` §2, `guest_ram_crossing.md` §1).
    ///
    /// Guest RAM is only shareable when the VM was **launched** with a shared, fd-backed
    /// memory block (`memory-backend-memfd,share=on`). No code gate can observe how the
    /// operator started the VM — the same argument
    /// [`kayfabe_linux_raw::Backing::PrivateAnonymous`] already writes down for the VMM's
    /// own side of the same fact — so the only available mechanism is a **loud refusal at
    /// the first grant**.
    ///
    /// ⊘ A caller must never fall back to copying the range instead. The guest **polls**
    /// its completion semaphore out of its own RAM and advances `GP_PUT` in its own ring;
    /// a poll has no trigger point at which a copy-back could happen, so a copying fallback
    /// is not a degraded version of this — it is a different, wrong mechanism that looks
    /// like it works until the guest waits forever.
    GuestRamUnavailable,
    /// Any other backend-reported failure (opaque status for diagnostics).
    Other(u32),
}

/// ★★★★★ **An instruction to map a slice of GUEST RAM into an isolate** — and the whole
/// point is *who wrote the numbers in it*.
///
/// `mode2_isolate_memory_boundary.md` §3, the load-bearing rule: **the VMM originates the
/// numbers; it never validates numbers the isolate proposed.** If the isolate could say
/// *"I would like offset X length Y"* and the VMM checked *"is that inside guest RAM?"*,
/// the check would be **circular** — it validates a request against itself, which is
/// exactly [an echo is unverifiable by its reply]. `(offset, len)` must come from the VMM's
/// own address-table derivation, and this type exists so that the shape of the call
/// enforces it rather than a comment asking for it.
///
/// ## ★ Why this is a SHAPE and had to land before the first caller
///
/// §5 of that page: *"Checks can be added later; shapes cannot."* If the crossing lands
/// without this, every call site is written assuming *"I can map what I need"*, and
/// retrofitting means auditing and rewriting all of them. The seccomp enforcement — fd
/// pinning, the filter, `SECCOMP_RET_USER_NOTIF` on `mmap`, the `munmap` confirmation —
/// lands **behind** this interface without touching a single call site, because the shape
/// already forced everything through one door.
///
/// ## ⊘ What this rung deliberately does NOT dictate, and why that is not a hole
///
/// The **host virtual address**. §3's "Matching" paragraph wants `addr` in the match set
/// too, and it is not here yet because dictating a host VA requires a *host-private VA
/// reservation in the isolate's address space* — `kayfabe_linux_raw` has no
/// `MAP_FIXED_NOREPLACE` anywhere by deliberate policy, and
/// [`kayfabe_linux_raw::Reservation::map_fixed_in`] is the only sanctioned way to place a
/// mapping at a chosen address. That reservation is designed and not built
/// (`guest_ram_crossing.md` §3).
///
/// ★ And the omission is **sound rather than merely tolerable**, which is the part worth
/// stating: the circularity §3 forbids is about *which guest memory* an isolate may reach.
/// The host VA says where in the isolate's own address space the pages land, which
/// authorizes nothing — an isolate that could pick its own host VA still cannot pick which
/// guest bytes are there. So the authorization is complete as written, and the host VA
/// tightens the *seccomp match* later rather than plugging a gap now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRamGrant {
    offset: u64,
    len: u64,
    prot: Prot,
}

impl GuestRamGrant {
    /// Mint a grant. **The name is the contract**: a caller that is not the VMM deriving
    /// these numbers from its own region map is misusing this, and the spelling is meant to
    /// be uncomfortable to write anywhere else.
    ///
    /// ⊘ There is deliberately no `from_request`, no `validate`, and no constructor that
    /// takes numbers the isolate sent. Adding one re-opens §3's circularity.
    #[must_use]
    pub fn originated_by_the_vmm(offset: u64, len: u64, prot: Prot) -> Self {
        GuestRamGrant { offset, len, prot }
    }

    /// Byte offset into the guest-RAM block.
    #[must_use]
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Length in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the isolate may write these pages.
    ///
    /// ★ Carried explicitly because authorizing read-only and receiving `PROT_WRITE` is a
    /// **silent escalation** (§3, "Matching"). A ring the isolate only reads must be mapped
    /// read-only *in the isolate*, which is
    /// [`kayfabe_linux_raw::HostProt`]'s whole reason for being a distinct type from this
    /// one.
    #[must_use]
    pub fn prot(&self) -> Prot {
        self.prot
    }

    /// Whether the grant names no bytes at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// What the isolate made of a [`GuestRamGrant`]: **a named mapping**, not an address.
///
/// ⊘⊘ **It deliberately does not carry a host virtual address**, and the reason is a
/// boundary rather than taste. `kayfabe_linux_raw::MappedRegion::addr_at` is `pub(crate)`:
/// no representation of a host mapping's address crosses that crate, because the one
/// consumer that needs one (`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`, which hands RM an address
/// it then `pin_user_pages`-walks) is served by patching the value into the ioctl argument
/// *inside* the raw crate and scrubbing it back out. Reporting a host VA up to the VMM
/// would have punched a hole in that for a number the VMM has no use for — it cannot
/// dereference an address in another process, and it does not choose this one.
///
/// ★ So the mapping is named by a [`HostHandle`], like every other isolate-side object.
/// That is not a stand-in for the address: it buys the cross-isolate check for free, since
/// [`Worker::execute`]'s foreign-handle gate already refuses a handle minted by one
/// isolate presented on another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRamMapped {
    /// The isolate's name for this mapping.
    pub region: HostHandle,
    /// Length in bytes, as granted.
    pub len: u64,
}

/// ★★★ **What the VMM is asking the isolate to make installable** —
/// [`RmBackend::export_backing`]'s argument (`isolate_vmm_fd_crossing.md` §12).
///
/// Two variants, and the second one **always refuses**. That is the point: the boundary
/// of the owner's decision (b) is a *typed* fact with a test that watches it fire, not a
/// paragraph. A request shape that could only ever succeed would leave the incomplete
/// half of (b) expressible nowhere, and an unexpressible boundary is the shape that gets
/// silently crossed later by someone adding "just one" backing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSource {
    /// ★ **Memory we FABRICATE** — an emulated device's framebuffer/instance window, a
    /// fabricated aperture, any range whose bytes exist only because we wrote them.
    ///
    /// The isolate mints it as a shareable file backing, so what crosses is memory: the
    /// VMM `mmap`s it, both processes see the same pages, and there is no `ioctl` surface
    /// anywhere in the transaction. This arm is why (b) is worth doing rather than merely
    /// safe.
    ///
    /// ⊘ It deliberately names **no** [`HostHandle`]: fabricated memory is not an RM
    /// object, and a request that could name one would invite exactly the confusion the
    /// other variant exists to refuse.
    Fabricated,
    /// ⊘ **The real device's own pages** — host framebuffer, a channel's ring/USERD, a
    /// BAR0 register window — named by the RM object that owns them.
    ///
    /// Always [`RmError::NotExportableAsMemory`]; see that variant for the three
    /// independent reasons, each cited.
    HostDeviceMemory {
        /// The RM object whose pages are wanted.
        memory: HostHandle,
    },
}

/// ★★★★★ **What joining ONE framebuffer leaf produced** — [`RmBackend::join_fb_leaf`]'s
/// answer, and the shape `fb_cpu_view.md` §4 measured on a real GA106 before it was built.
///
/// Three facts from one chain, reported separately because each has a different owner and
/// a different reclaim point:
///
/// - `backing` — the **memory**, minted in the isolate and handed up to the VMM. This is
///   the half that closes *"two memories"*: the VMM `mmap`s the same pages the isolate
///   described to RM, so a byte the guest writes through the emulated framebuffer and a
///   byte the engine reads through the GPU MMU are the **same byte**.
/// - `memory` — the `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` RM built over the isolate's mapping.
///   Owned by RM; freed through [`VerbPlan::Release`] like any other host object.
/// - `host_va` — where it landed. Equal to the plan's `at`, or the verb refused with
///   [`RmError::PlacementRefused`] and this value never existed.
///
/// ⚠ **The leaf is host SYSTEM memory, not card memory, and that is a named divergence**
/// from the C artifact and from [`VerbPlan::PublishVidmem`], not an oversight. The engine
/// reaches it over PCIe rather than out of local framebuffer. It is **not optional**: card
/// memory is exactly the memory that cannot carry a guest-reachable CPU view, which is the
/// whole of [`ExportSource::HostDeviceMemory`]'s refusal. Stated here so the cost is read
/// off the type rather than discovered in a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbLeafJoined {
    /// ★★★ The shareable backing, for the VMM to `mmap` as the guest's own view of this
    /// framebuffer range. Its token is the **adapter's**, minted when the adapter adopted
    /// the descriptor — never the child's index and never a value off the wire.
    pub backing: ExportedBacking,
    /// The RM object describing the isolate's mapping of those same pages.
    pub memory: HostHandle,
    /// The host GPU VA it was mapped at.
    pub host_va: u64,
}

/// One request to [`RmBackend::export_backing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportRequest {
    /// Which class of bytes.
    pub source: ExportSource,
    /// How many bytes the VMM intends to install.
    pub len: u64,
    /// What the **guest** is to be allowed to do with them. A request, not an outcome —
    /// see [`ExportedBacking::prot`].
    pub prot: Prot,
}

/// ★★ What the isolate handed back: **memory, named by a token, never a descriptor**.
///
/// ## Why there is no fd in this type, and why that is the whole design
///
/// This crate is pure. More importantly, a value type carrying an OS descriptor would put
/// the descriptor on the *core's* side of the port, where every rule about what may be
/// done with one would be advisory. The descriptor rides the reply frame's ancillary data
/// and is adopted by the transport ([`kayfabe_isolate_host::CrossedFd`]), which is the one
/// place that can check what it actually **is**; [`ExportedBacking::token`] is the adapter's
/// own index for it, minted by the **parent** and never by the wire — the same discipline
/// [`HostHandle`] uses, for the same reason.
///
/// Deliberately the mirror image of [`kayfabe_vmm::RamHandle`], which carries the VMM →
/// isolate direction (guest RAM, VMM-minted). This is isolate → VMM.
///
/// [`kayfabe_isolate_host::CrossedFd`]: https://docs.rs/kayfabe-isolate-host
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportedBacking {
    /// Adapter-scoped opaque index naming the backing the adapter adopted. Feeds a
    /// `kayfabe_vmm::HostRegion::id`; interpreted by nothing in the core.
    pub token: u64,
    /// Byte offset into that backing at which the requested range begins.
    pub offset: u64,
    /// How many bytes are actually there.
    pub len: u64,
    /// ★ What the isolate **actually granted**, which may be narrower than what
    /// [`ExportRequest::prot`] asked for — a read-only export is a real thing an isolate
    /// may decide to hand out, and a caller that installed the *requested* protection
    /// would hand the guest a write it does not have. Use this one.
    pub prot: Prot,
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

    /// ★★★ Intent verb: allocate `len` bytes of **device-local (vidmem)** memory —
    /// a blank host framebuffer object, for the SECOND crossing.
    ///
    /// # ⊘ Why this is a separate verb and not `alloc_sysmem` with a flag
    ///
    /// The two are not the same allocation with a preference attached. `alloc_sysmem`
    /// asks for `MAPPING_NO_MAP`, which makes its object deliberately **un-CPU-mappable**
    /// — correct for a describe-only range, and fatal for this one, because the whole
    /// point of the second crossing's mature form is a *double* mapping (one for the host
    /// GPU, one for the CPU view the guest's own framebuffer accesses land in;
    /// `C: docs/design/mode2_fb_crossing_question.md` §5, GEN-2). A caller that reached
    /// for sysmem here would get an object that maps into the host VAS, satisfies every
    /// check, and can never be the shared object.
    ///
    /// ⚠ And the aperture is not cosmetic: the guest's own PTE for these ranges declares
    /// `_TARGET_LOCAL_FB`. Backing a vidmem-declared range with host sysmem produces a
    /// host mapping that **works** and is in the wrong aperture — a silent disagreement
    /// between what the guest's page tables say and what the host engine walks, which is
    /// the class of wrongness this port refuses to make representable by a `bool`.
    ///
    /// ⊘ This verb allocates. It does **not** map, does not seed and does not copy: a
    /// freshly allocated vidmem object's contents are not specified, and nothing here
    /// pretends otherwise.
    fn alloc_vidmem(&mut self, len: u64) -> Result<HostHandle, RmError>;

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

    /// ★★★ Perform **one** sub-copy of a partitioned copy-engine request, on the engine
    /// the plan chose (`eight_blockers_resolved.md` §12.4).
    ///
    /// ONE verb rather than two, and the arm is a field of the instruction rather than a
    /// choice this method makes. The distinction §12 draws is *representability*, which
    /// is a property of the ADDRESS PLANE — the backend does not hold the address plane
    /// and must not appear to. An implementation matches on [`CeSubCopy::by`]:
    ///
    /// - [`CeExecutor::HostCe`] — submit a real copy-engine copy in `vas`. Both operands
    ///   are host-published at guest-identical addresses (`#102` stage A), so the guest's
    ///   own numbers are the ones hardware walks for.
    /// - [`CeExecutor::Ours`] — move the bytes here, against this isolate's mapping of
    ///   the fabricated aperture. This is the arm the C spends most of its time in and
    ///   the only arm that can serve a page-table write, whose payload is guest-physical
    ///   PTE values hardware would resolve as addresses.
    ///
    /// # Errors
    /// Whatever the host refuses with. ★ A refusal **may leave a prefix of this
    /// sub-copy applied** — that is what a real engine does when it faults mid-copy
    /// (the C breaks out of its span loop and the remainder is silently never written,
    /// `C: nvkvm_gpu_emul.c:6389` "#13 CE-DROP"). Nothing is allocated, so there are no
    /// orphans to unwind; the caller's evidence is the sub-copy index that failed.
    fn ce_copy(&mut self, vas: HostHandle, sub: CeSubCopy) -> Result<(), RmError>;

    /// ★★★ **Read `buf.len()` bytes of the FABRICATED APERTURE** at guest-framebuffer
    /// physical address `phys` — the page-table decoder's byte source
    /// (`eight_blockers_resolved.md` §12.2, `#102` stage C3).
    ///
    /// ## Why this verb exists, and why it is bounded to *fabricated* memory
    ///
    /// §12's ruling decomposes a copy-engine request by **representability**: what a real
    /// engine can be pointed at goes to real hardware, and what it cannot — an operand in
    /// space *we* invented — is performed by us, [`CeExecutor::Ours`], **in the isolate**.
    /// The consequence the ruling then draws is the whole reason for this method: every
    /// byte written into fabricated space was written *by us*, so the content of the
    /// guest's page tables is already in the isolate's own mapping of that aperture. This
    /// is the read half of the write [`RmBackend::ce_copy`]'s `Ours` arm performs.
    ///
    /// ★★ **The bound is the safety property.** We shadow the fabricated aperture *only*
    /// — memory we invented and therefore already own — never "every copy destination".
    /// Generalising past that is what made a core-owned content store collapse (§11.6
    /// Option 3, rejected). There is deliberately no verb here that reads *representable*
    /// memory: if a caller ever wants one, the boundary has moved and the design has to
    /// be re-argued, not the signature widened.
    ///
    /// ## The three answers, and why "not covered" is not an error
    ///
    /// - `Ok(true)` — served; `buf` holds the bytes.
    /// - `Ok(false)` — **this isolate's fabricated aperture does not cover the range.**
    ///   A guest's page table naming a physical page outside it is an ordinary guest
    ///   fact, not a malfunction, and the walker turns it into a loud
    ///   `kayfabe_mmu::walker::WalkFault::Unbacked`: MISS = FAULT, forwarded, never
    ///   guessed into a capture. It must **never** be answered as a page of zeros, which
    ///   decodes as a page that legitimately maps nothing.
    /// - `Err` — the transport or the host failed. A different fact from `Ok(false)` and
    ///   kept separate on purpose: one is about the guest, the other is about us.
    ///
    /// # Errors
    /// Whatever the host or the transport refuses with.
    fn fb_read(&mut self, phys: u64, buf: &mut [u8]) -> Result<bool, RmError>;

    /// Intent verb: export the host memory object `memory` (a render target in host
    /// VRAM) as a presentable [`SurfaceHandle`] — the **producer half of the display
    /// seam** (`execution_plane.md` §3.3, seam audit GR-2b). The C proved this runs
    /// in the ISOLATE (stub `PRIME_HANDLE_TO_FD` dma-buf export, session-owned —
    /// `present_path_b_done`); the flow is one-way guest→host. The consumer half is
    /// `Present::present`. Anti-bolt-on note: this is the ONE named display verb —
    /// the verb surface does not grow per engine.
    fn export_surface(&mut self, memory: HostHandle) -> Result<SurfaceHandle, RmError>;

    /// ★★★ **Perform the mapping HERE, and hand back MEMORY the VMM can install** —
    /// the owner's decision (b) for `#133`/`#128`, made into a verb
    /// (`isolate_vmm_fd_crossing.md` §12).
    ///
    /// ## The problem this exists to solve
    ///
    /// A KVM memslot names a userspace address **in the VMM's own address space**, so
    /// something has to cross the isolate boundary. The obvious something is the GPU
    /// descriptor: the isolate opens `/dev/nvidia*`, hands it up, the VMM `mmap`s it.
    /// That works and it is what the C does — and it puts an `ioctl`-capable RM escape
    /// descriptor in a process that is very often root, where RM re-derives privilege
    /// **from the caller on every escape**, not from the opener
    /// (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:304`; `guest_blast_radius.md`
    /// F14). The owner's ruling was to move the mapping work behind this verb instead:
    /// hygiene and contract, *"the descriptor simply should not be somewhere we do not
    /// control"*.
    ///
    /// ⚠ **The ruling explicitly does not rest on sandboxing the VMM.** A compromised VMM
    /// is the boundary, not a step inside it; confining the hypervisor is the deployment's
    /// job and not this project's. F14 is therefore not *closed* by this verb — it is the
    /// reason not to hand the descriptor up in the first place.
    ///
    /// ## What crosses instead
    ///
    /// A **shared-file backing** (a sealed `memfd`). The VMM maps *that*, so the pages are
    /// the same pages and nothing in the transaction has an `ioctl` handler for an RM
    /// escape. ★ Note the coincidence that is not a coincidence: the class this verb can
    /// export is exactly the class whose effective CPU memory type is **knowable** —
    /// `kayfabe_linux_raw::Backing::attainable_cache_policy` answers `Some(WriteBack)` for
    /// a shared file and `None` for a device file, because for a device file *"the driver
    /// already decided … and userspace cannot read it back"*. One boundary, two
    /// consequences.
    ///
    /// ## ⊘ What it does NOT cover — a READING of the driver, not an omission
    ///
    /// ★ Said at the epistemic level actually held (`claim_ledger.md`): the two citations
    /// below are **readings of `ogkm-580` at a named file:line**, which say what the
    /// driver *does*, and no hardware ran for either. That settles this particular
    /// question because both are unconditional refusals on the source path with no runtime
    /// input — but the citations are readings, and they are written as readings.
    ///
    /// [`ExportSource::HostDeviceMemory`] is always [`RmError::NotExportableAsMemory`].
    /// The three independent reasons are cited on that variant. This is the incomplete
    /// half of (b), and it is named here rather than faked: a real device BAR is not
    /// memfd-backed, and the two routes that could have carried it are a character device
    /// (an `ioctl` surface — the thing being avoided) and an NVIDIA dma-buf (whose CPU
    /// mapping is hard-gated to zero-framebuffer parts).
    ///
    /// # Errors
    /// [`RmError::NotExportableAsMemory`] for the device class; [`RmError::NoMemory`] if
    /// the host would not mint the backing; whatever the transport refuses with.
    fn export_backing(&mut self, want: ExportRequest) -> Result<ExportedBacking, RmError>;

    /// ★★★★★ **ONE memory for a framebuffer leaf** — mint a fabricated backing, map it
    /// here, describe it to RM, place it at `at`, and hand the **same pages** up to the VMM
    /// (`fb_cpu_view.md` §4).
    ///
    /// # ⊘ Why this is a verb of its own and not `export_backing` plus `map_gpu_va`
    ///
    /// Because the VMM cannot name the isolate's backing. [`ExportedBacking`] carries the
    /// **adapter's** token, minted when the adapter adopts the descriptor; the child's index
    /// into its own table deliberately does not travel (`kayfabe_isolate_host::export`'s
    /// module docs — *"a value the peer supplies must never name a slot in our registry"*).
    /// So a VMM holding an exported backing has no way to say *"describe **that** one to RM
    /// at **this** VA"*. The two halves have to be one verb, issued by the party that owns
    /// the memfd.
    ///
    /// ⊘ **It is not [`RmBackend::export_backing`] with extra steps in the other direction
    /// either.** `export_backing` acquires no RM object and therefore sits beside
    /// [`Worker::execute`]; this acquires two (a descriptor and a fixed GPU mapping) and
    /// belongs inside a chain that can unwind them.
    ///
    /// # ★★ What the implementation owes, in order
    ///
    /// 1. mint a shareable backing of `len` bytes — the [`ExportSource::Fabricated`] arm,
    ///    the one that is *designed* to succeed;
    /// 2. `mmap` it **in the isolate**, and keep that mapping alive for as long as the
    ///    descriptor: RM pins the pages behind an `OS_DESCRIPTOR` and a mapping torn out
    ///    from under it is a live GPU mapping of memory this process no longer describes;
    /// 3. `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over that mapping;
    /// 4. map it into `vas` at `at`, `DMA_OFFSET_FIXED`, and **refuse** rather than adopt a
    ///    placement that is not `at`;
    /// 5. `phys` is carried so the isolate can answer for this range **by framebuffer
    ///    address** afterwards. ⊘ It is the VMM's number and nothing here derives it.
    ///
    /// ★ **Per-isolate state, never per-worker.** The mapping table this builds is a
    /// property of the isolate: an isolate is a **pool**, and the worker that joins a leaf
    /// need not be the worker later asked to read it. A table on the backend is the bug that
    /// one-worker tests cannot see.
    ///
    /// # Errors
    /// [`RmError::NoMemory`] if the backing cannot be minted; [`RmError::PlacementRefused`]
    /// when the fixed map did not land at `at`; otherwise whatever RM or the transport
    /// refused with. ⊘ A backend with nowhere to keep the mapping must refuse **by name**
    /// rather than mint a per-worker table.
    fn join_fb_leaf(
        &mut self,
        vas: HostHandle,
        len: u64,
        at: GpuVa,
        phys: u64,
    ) -> Result<FbLeafJoined, RmError>;

    /// ★★★ **The both-directions instrument for a joined leaf** — read what the isolate's
    /// own mapping holds at `phys`, and (when `poke` is `Some`) leave a per-word pattern
    /// behind in it.
    ///
    /// # ⊘ This is an instrument, and it is written as one
    ///
    /// It is the only thing in this trait that *writes* fabricated content, and it exists
    /// because the property [`RmBackend::join_fb_leaf`] claims — *"the VMM's view and the
    /// isolate's view are one memory"* — is not observable from either side alone. A VMM
    /// that wrote through its own mapping and read back through its own mapping would be
    /// comparing a buffer with itself.
    ///
    /// ★ **One round trip carries both directions**: the read happens **before** the poke,
    /// so the reply is the guest→isolate answer and the poke is the isolate→guest stimulus
    /// the caller then reads through its own view. Two verbs would have made the ordering a
    /// convention; one makes it the type.
    ///
    /// `Ok(false)` means **no joined range covers `[phys, phys+len)`** — the same
    /// *"MISS = FAULT"* fact [`RmBackend::fb_read`] carries, and deliberately not zeros: an
    /// unjoined range answering with a page of zeros is indistinguishable from a joined one
    /// holding zeros, and those are opposite findings.
    ///
    /// # Errors
    /// Whatever the transport refused with. ⊘ `Ok(false)` is not an error.
    fn fb_join_peek(
        &mut self,
        phys: u64,
        buf: &mut [u8],
        poke: Option<u32>,
    ) -> Result<bool, RmError>;

    /// ★★★★★ **Map a slice of GUEST RAM into this isolate — because the VMM said to.**
    ///
    /// This is the one door. `mode2_isolate_memory_boundary.md` §5: *the isolate never
    /// `mmap`s guest RAM on its own; it is instructed.* Everything the isolate can reach of
    /// the guest's memory arrives through a [`GuestRamGrant`], whose numbers the VMM
    /// derived from its own region map — see that type for why the alternative (the isolate
    /// asking and the VMM checking) is a circular check rather than a weaker one.
    ///
    /// ## Why guest RAM has to be MAPPED and cannot be COPIED
    ///
    /// The refuted alternative, and it is refuted by the guest's own behaviour rather than
    /// by preference: the guest **polls** its completion semaphore directly out of its own
    /// RAM, and advances `GP_PUT` in its own ring expecting the engine to see it. A poll
    /// has **no trigger point** — no event, no ioctl, no exit — at which a copy-back could
    /// be scheduled. ★ The C agrees and did it this way: for sysmem it mapped
    /// (`pci_dma_map` → host VA → `OS_DESCRIPTOR` over the real pages) and copied only for
    /// emulated-framebuffer seeding, which is memory it owned.
    ///
    /// ## ⊘ What this verb is NOT
    ///
    /// It is **not** [`RmBackend::export_backing`]'s inverse and must not be confused with
    /// it. `export_backing` runs isolate → VMM and hands up memory the *isolate* minted;
    /// this runs VMM → isolate and hands down memory the *guest* owns. The two directions
    /// have opposite threat models, which is why [`ExportedBacking`] and
    /// [`kayfabe_vmm::RamHandle`] are separate types with separate token spaces.
    ///
    /// # Errors
    /// [`RmError::GuestRamUnavailable`] if the VM was not launched with a shared memory
    /// backing — a deployment fact, refused loudly rather than degraded into a copy;
    /// [`RmError::NoMemory`] if the host refused the mapping; whatever the transport
    /// refuses with.
    fn map_guest_ram(&mut self, grant: GuestRamGrant) -> Result<GuestRamMapped, RmError>;

    /// Give back what [`RmBackend::map_guest_ram`] mapped.
    ///
    /// ⚠ §3's freeing rule, stated here because it is the half a later reader drops: the
    /// kernel frees the pages on `munmap` regardless — what confirming this buys is
    /// **knowing the isolate has lost access before the range is reused**. So a VMM that
    /// reclaims a guest-RAM range must wait for this to return, and on the timeout path
    /// must free after the child is **reaped**, not after it is signalled: between the
    /// signal and the mm teardown the mappings still exist.
    ///
    /// # Errors
    /// [`RmError::GuestRamUnavailable`] if there is no guest RAM to unmap; whatever the
    /// transport refuses with.
    fn unmap_guest_ram(&mut self, mapped: GuestRamMapped) -> Result<(), RmError>;

    /// ★★★★★ **Describe a live guest-RAM mapping to the host driver as a memory object** —
    /// `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over the pages [`RmBackend::map_guest_ram`]
    /// already mapped (`guest_ram_crossing.md` §5.8).
    ///
    /// This is the verb that turns *"this process can read the guest's bytes"* into
    /// *"the host GPU can reach the guest's bytes"*, and it is the last one missing
    /// between the two. After it, nothing new is needed:
    /// [`RmBackend::map_gpu_va`] already places a memory object at a **dictated** address
    /// and already refuses a placement it did not get.
    ///
    /// ## ⊘ Why this is a SEPARATE verb and not a flag on `map_guest_ram`
    ///
    /// The two have different failure modes and different lifetimes. `map_guest_ram` is an
    /// `mmap` in this process and is undone by `munmap`; this is an RM allocation that
    /// **pins** those pages for the host GPU and is undone by `free`. Folding them would
    /// make the natural cleanup — drop the mapping — silently leave RM holding pinned
    /// pages, which is the class [`RmBackend::map_guest_ram`]'s own docs already refuse to
    /// hide. ⚠ `alloc_os_descriptor`'s note applies verbatim: **dropping the host mapping
    /// does not release RM's reference.**
    ///
    /// ## ⊘ What this verb may NOT grow
    ///
    /// An offset and a length. It describes **exactly** what the grant named, because the
    /// grant's numbers are the VMM's and a second pair of numbers here would be a second
    /// description of the same authorization — and the isolate would own one of them. That
    /// is `mode2_isolate_memory_boundary.md` §3's circularity arriving one verb later.
    ///
    /// # Errors
    /// [`RmError::GuestRamUnavailable`] if this isolate has no guest-RAM plane;
    /// [`RmError::NoMemory`] if `mapped` names no live mapping; whatever RM refuses the
    /// allocation with.
    fn describe_guest_ram(&mut self, mapped: GuestRamMapped) -> Result<HostHandle, RmError>;
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
    /// ★★★ **THE SECOND CROSSING** — a blank **host vidmem** object, mapped FIXED at the
    /// guest's own VA. `C: docs/design/mode2_fb_crossing_question.md` §5 (GEN-2), settled
    /// 2026-06-04 and built twice in the C artifact.
    ///
    /// (optionally) allocate the `Vas`'s host VAS → [`RmBackend::alloc_vidmem`] →
    /// [`RmBackend::map_gpu_va`] **at `at`**, with [`VerbPlan::Publish`]'s own placement
    /// check applied unchanged. It answers with [`VerbReply::Published`], because what it
    /// produces *is* a published backing — the difference is entirely in which store the
    /// bytes came out of.
    ///
    /// ## ⊘ Why it is a third variant and not an aperture field on [`VerbPlan::Publish`]
    ///
    /// The same argument [`VerbPlan::PinGuestRam`] makes one screen down, and it lands the
    /// same way: the two chains differ in what a **wrong** answer costs. `Publish` mints
    /// host sysmem with `MAPPING_NO_MAP`; a range published that way can never become the
    /// CPU-side half of GEN-2's double mapping, and it declares an aperture the guest's own
    /// leaf does not. Both failures are silent — the object allocates, the fixed map
    /// succeeds, every check passes — which is exactly the shape a `bool` erases.
    ///
    /// ## ⚠ What this variant does NOT carry, deliberately
    ///
    /// No seed, no copy, no content of any kind. GEN-2's `copy_content` is a **one-time
    /// establishment bridge** and is not part of the allocate-and-place chain; a plan that
    /// carried bytes would be a plan that could silently overwrite a buffer the guest had
    /// already written.
    PublishVidmem {
        /// The `Vas`'s already-materialized host VAS, or `None` to allocate one.
        host_vas: Option<HostHandle>,
        /// Bytes of host vidmem to allocate and map — the guest leaf's own length, so
        /// that the mapping covers exactly the range the guest's page tables bind.
        len: u64,
        /// ★★★ The guest VA this range must be addressable at. Address identity: the
        /// mapping is placed HERE, or the verb fails.
        at: GpuVa,
    },
    /// ★★★★★ **ONE MEMORY for a framebuffer leaf** — the chain that **replaces**
    /// [`VerbPlan::PublishVidmem`] at every leaf (`fb_cpu_view.md` §4).
    ///
    /// (optionally) allocate the `Vas`'s host VAS → [`RmBackend::join_fb_leaf`], with
    /// [`VerbPlan::Publish`]'s own placement check applied unchanged on the way back.
    ///
    /// ## ⊘ It is a REPLACEMENT, not an addition
    ///
    /// `PublishVidmem` gives the leaf **real card memory with no CPU view**, which is two
    /// memories: the engine reads the card object and the guest reads the emulator's
    /// fabricated one, silently, in both directions and with no fault anywhere. That is not
    /// a shortfall of the variant — it is what a vidmem object **is**, and the measurement
    /// that settles it is [`ExportSource::HostDeviceMemory`]'s refusal. ⇒ A leaf served by
    /// both chains would have two backings at one VA, so a shell arms exactly one.
    ///
    /// ## ⚠ The cost, named here rather than found later
    ///
    /// The leaf becomes host **sysmem**. See [`FbLeafJoined`].
    JoinFbLeaf {
        /// The `Vas`'s already-materialized host VAS, or `None` to allocate one.
        host_vas: Option<HostHandle>,
        /// The guest leaf's own length, so the mapping covers exactly the range the guest's
        /// page tables bind.
        len: u64,
        /// ★★★ The guest VA this range must be addressable at. Address identity: the
        /// mapping is placed HERE, or the verb fails.
        at: GpuVa,
        /// ★★ The **framebuffer-physical** address this leaf occupies in the emulated
        /// device, as the guest's own page-table walk produced it. Carried so the isolate
        /// can answer for the range by framebuffer address afterwards
        /// ([`RmBackend::fb_join_peek`]); ⊘ nothing below the VMM derives it.
        phys: u64,
    },
    /// ★★★★★ **THE GUEST'S OWN PAGES, published at the guest's own VA** — the chain
    /// [`VerbPlan::Publish`] is the *fabricated* counterpart of
    /// (`guest_ram_crossing.md` §5.8).
    ///
    /// (optionally) allocate the `Vas`'s host VAS → [`RmBackend::map_guest_ram`] →
    /// [`RmBackend::describe_guest_ram`] → [`RmBackend::map_gpu_va`] **at `at`**, with
    /// [`VerbPlan::Publish`]'s own placement check applied unchanged.
    ///
    /// ## ⊘ Why it is a second variant and not a field on `Publish`
    ///
    /// `Publish` allocates **host** sysmem and maps it at the guest's VA: the address is
    /// the guest's and the bytes are not. That is correct for a range the guest has never
    /// written and wrong for a ring the guest is *polling* — the guest advances `GP_PUT`
    /// in its own pages, and a host allocation at the same address is a different buffer
    /// that will never change. The two chains therefore differ in what a **wrong** answer
    /// costs, which is exactly the distinction a `bool` erases.
    ///
    /// ## ★★★ The grant's numbers are the VMM's, and the type says so
    ///
    /// [`GuestRamGrant`] can only be minted by
    /// [`GuestRamGrant::originated_by_the_vmm`], so a plan carrying one carries the
    /// VMM's own derivation from its own stated layout. ⊘ There is deliberately **no
    /// guest-physical address in this variant** — a GPA would be a number the core could
    /// be tempted to re-derive an offset from, and re-deriving it is the `-m 8G` bug
    /// `kayfabe_vmm_qemu::layout` exists to refuse.
    PinGuestRam {
        /// The `Vas`'s already-materialized host VAS, or `None` to allocate one.
        host_vas: Option<HostHandle>,
        /// ★★★ The VMM's instruction: which slice of the guest-RAM block, and what the
        /// isolate may do with it.
        grant: GuestRamGrant,
        /// ★★★ The guest VA these pages must be addressable at. Address identity, and
        /// here it is not a convention but the guest's *existing* binding — this range is
        /// already mapped at `at` in the guest's own page tables.
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
    /// ★★★ **A copy-engine request, already PARTITIONED by representability**
    /// (`eight_blockers_resolved.md` §12.3).
    ///
    /// One guest `LAUNCH_DMA` becomes one of these, carrying one sub-copy per maximal
    /// run over which the answer is constant. §12.4's ruling is that **the executor is
    /// the isolate in both cases** — real copy-engine submission and our own copy over
    /// VRAM-backed mappings alike, never the hypervisor process and never the pure core
    /// — and this variant is what makes that structural rather than aspirational: the
    /// core builds a plan and has no way to move a byte itself.
    ///
    /// The **plan** chooses the engine per sub-copy ([`CeSubCopy::by`]); the backend
    /// only obeys. A backend free to pick would be free to point a real engine at
    /// fabricated space, which is `Xid 31 FAULT_PDE` one layer down.
    CeSplit {
        /// The host VAS the sub-copies' addresses live in — the owning `Vas`'s own,
        /// per-`Vas` (the #14 fix), at guest-identical addresses (`#102` stage A).
        vas: HostHandle,
        /// The sub-copies, **in submission order**. A copy engine's ordering guarantee
        /// within one request is what the guest's own semaphore release depends on, so
        /// this is a sequence and never a set.
        subs: Vec<CeSubCopy>,
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

/// ★★★ **THE EXECUTE DECISION** — which engine moves the bytes of one sub-copy.
///
/// Deliberately *not* a `bool`: in the C this is a bare local (`bool host_ce`,
/// `C: nvkvm_gpu_emul.c:6310`) whose negation is spelled as three `else if` branches
/// further down, which is exactly how "everything else is forwarded" became an answer
/// nobody had made (`eight_blockers_resolved.md` §11.5).
///
/// It lives in the **port** crate rather than in `kayfabe-fwd` because both sides of the
/// seam must name it: the core decides it, the isolate obeys it, and one concept gets
/// one name. `kayfabe-fwd` re-exports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CeExecutor {
    /// Real hardware runs it — a real copy-engine submission, honouring its own
    /// semaphores. §12.2: *"that is normally faster than a CPU memcpy, not merely more
    /// faithful."* No byte passes through us.
    HostCe,
    /// **We** run it, because the operand is *unrepresentable*: it names fabricated
    /// space (our page-directory base, our guest-physical framebuffer) that no real
    /// engine can be pointed at. In the C this is the CPU byte-copy at `C: :6371-6425`;
    /// here it runs **in the isolate**, against the isolate's mapping of that aperture.
    Ours,
}

/// What a sub-copy READS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CeSource {
    /// A source address, in the same host VAS as the destination.
    Address(u64),
    /// A constant pattern — the scrub (`0`) and the `REMAP_ENABLE` fill (`C: :6320`,
    /// `:6349`). No source address exists, so there is no source operand to classify
    /// and no source range to partition: a fill's representability is a property of its
    /// **destination alone**.
    Constant(u32),
}

/// One sub-copy of a (possibly split) copy-engine request.
///
/// Addresses are absolute, not offsets: a sub-copy is a self-contained instruction, and
/// an offset-relative form would need the original request to be interpretable, which is
/// how a partition silently becomes non-total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CeSubCopy {
    /// Destination address.
    pub dst: u64,
    /// What to read.
    pub src: CeSource,
    /// Bytes. **Never zero** — a zero-length sub-copy is a partition bug, not a
    /// degenerate case to be handled downstream, and `kayfabe-fwd` never emits one.
    pub len: u64,
    /// ★ Which engine performs it — chosen by the PLAN, obeyed by the backend.
    pub by: CeExecutor,
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
            VerbPlan::Publish { host_vas, .. }
            | VerbPlan::PublishVidmem { host_vas, .. }
            | VerbPlan::JoinFbLeaf { host_vas, .. }
            | VerbPlan::PinGuestRam { host_vas, .. } => host_vas.iter().copied().collect(),
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
            VerbPlan::CeSplit { vas, .. } => vec![*vas],
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
    /// ★★★★★ [`VerbPlan::JoinFbLeaf`]'s reply — **and it carries the backing as well as
    /// the RM object**, because the whole point is that a second party can map it.
    ///
    /// ⊘ A separate variant rather than a field on [`VerbReply::Published`]: a commit that
    /// adopted a `Published` and silently found no backing would record a leaf as *joined*
    /// when it was merely *placed*, which is the exact two-memories state this chain
    /// exists to end.
    FbLeafJoined {
        /// Freshly allocated host VAS, if the plan asked for one.
        host_vas: Option<HostHandle>,
        /// The three facts the chain produced. See [`FbLeafJoined`].
        joined: FbLeafJoined,
    },
    /// ★★★ [`VerbPlan::PinGuestRam`]'s reply — **and it carries the guest-RAM mapping
    /// as well as the RM object**, because releasing one does not release the other.
    GuestRamPinned {
        /// Freshly allocated host VAS, if the plan asked for one.
        host_vas: Option<HostHandle>,
        /// ★ The isolate's `mmap` of the guest pages. ⊘ Reported separately from
        /// `memory`: the RM object pins the pages and the mapping is this process's view
        /// of them, and a caller that freed only the object would leave an isolate with a
        /// live window onto guest RAM it no longer has any reason to see.
        mapped: GuestRamMapped,
        /// The `OS_DESCRIPTOR` object RM built over those pages.
        memory: HostHandle,
        /// The host GPU VA it was mapped at — equal to the plan's `at`, or the verb
        /// failed.
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
    /// [`VerbPlan::CeSplit`]'s reply — how the request was actually divided between the
    /// two engines. Counted rather than merely "ok" so a test can assert that a split
    /// request really did reach BOTH engines, which is the whole claim §12.3 makes.
    CeSplit {
        /// Sub-copies performed on real hardware.
        host_ce: usize,
        /// Sub-copies performed by us.
        ours: usize,
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

    /// ★★★ **Read the fabricated aperture** through this worker's backend
    /// ([`RmBackend::fb_read`], `#102` stage C3) — the page-table decoder's byte source.
    ///
    /// ## Why this is beside [`Worker::execute`] and not a [`VerbPlan`] variant
    ///
    /// `execute` exists to run a **chain that allocates**: it gates the plan's handles,
    /// chains intermediate results and unwinds what a mid-chain failure already acquired.
    /// A read of the aperture has none of those properties — it names **no
    /// [`HostHandle`]**, so there is nothing for the foreign-handle gate to check, and it
    /// acquires nothing, so there is nothing to unwind. Expressing it as a plan would add
    /// a `VerbReply` variant carrying bytes and a plan variant with no handles, i.e. two
    /// shapes whose only content is that they are exceptions to what the type is for.
    ///
    /// ★★ What it does keep, and the reason it is a method on `Worker` rather than a
    /// direct call on the backend: **the R1 assertion**. A decode pass reads one page per
    /// round trip to the isolate; doing that under a ranked lock is precisely the
    /// "blocking call under a lock" R1 forbids, and it is the failure that would be
    /// found in production rather than in a test. Every route to this call therefore runs
    /// the same lock-freedom check every host verb does.
    ///
    /// # Errors
    /// Whatever the backend refuses with. `Ok(false)` is **not** an error — see
    /// [`RmBackend::fb_read`].
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1).
    pub fn fb_read(&mut self, phys: u64, buf: &mut [u8]) -> Result<bool, RmError> {
        kayfabe_util::lockwitness::assert_lock_free("reading the fabricated aperture");
        self.backend.fb_read(phys, buf)
    }

    /// ★★★ **The joined-leaf instrument** ([`RmBackend::fb_join_peek`]) — beside
    /// [`Worker::execute`] for [`Worker::fb_read`]'s reasons exactly: it names no
    /// [`HostHandle`] and acquires nothing, so there is neither a foreign-handle gate to run
    /// nor an unwind to build. What it keeps is R1, because it is a round trip to another
    /// process.
    ///
    /// # Errors
    /// Whatever the backend refuses with. `Ok(false)` is **not** an error — see
    /// [`RmBackend::fb_join_peek`].
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1).
    pub fn fb_join_peek(
        &mut self,
        phys: u64,
        buf: &mut [u8],
        poke: Option<u32>,
    ) -> Result<bool, RmError> {
        kayfabe_util::lockwitness::assert_lock_free("peeking a joined framebuffer leaf");
        self.backend.fb_join_peek(phys, buf, poke)
    }

    /// ★★★ **Ask the isolate to perform a mapping and hand back memory the VMM can
    /// install** ([`RmBackend::export_backing`], `isolate_vmm_fd_crossing.md` §12).
    ///
    /// Beside [`Worker::execute`] rather than inside it, for [`Worker::fb_read`]'s
    /// reasons exactly: this acquires no RM object, so there is nothing for the chain's
    /// unwind to release and no [`VerbReply`] shape it fits. What it keeps from `execute`
    /// is the pair of gates that are not optional —
    ///
    /// - **R1.** Minting a backing is a syscall in another process reached over a socket.
    ///   Doing that under a ranked lock is the blocking-call-under-a-lock R1 forbids.
    /// - ★★ **The foreign-handle gate.** [`ExportSource::HostDeviceMemory`] names a
    ///   [`HostHandle`], and a handle from another isolate's namespace is live-and-
    ///   different here (`l1_concurrency.md` §12.26). It is gated even though the verb
    ///   refuses every such request anyway, because *"the refusal happens to cover it"* is
    ///   a property of today's implementation and the gate is a property of the port. A
    ///   backend that ever learns to serve one must meet the gate already in place.
    ///
    /// # Errors
    /// [`RmError::ForeignHandle`] before anything runs; otherwise whatever
    /// [`RmBackend::export_backing`] refuses with.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1).
    pub fn export_backing(&mut self, want: ExportRequest) -> Result<ExportedBacking, RmError> {
        kayfabe_util::lockwitness::assert_lock_free("exporting a host backing to the VMM");
        if let ExportSource::HostDeviceMemory { memory } = want.source
            && !memory.belongs_to(self.isolate)
        {
            return Err(RmError::ForeignHandle {
                handle: memory,
                worker_isolate: self.isolate,
            });
        }
        self.backend.export_backing(want)
    }

    /// ★★★★★ Carry the VMM's guest-RAM grant to this worker's isolate.
    ///
    /// R1 first, for [`Worker::export_backing`]'s reason: this is a syscall in another
    /// process reached over a socket.
    ///
    /// ⊘ There is **no foreign-handle gate on the way in**, and that is a statement rather
    /// than an omission: a [`GuestRamGrant`] names no [`HostHandle`] at all. It carries a
    /// guest-physical offset the VMM derived, which belongs to no isolate's namespace and
    /// cannot be a value from a sibling's. The gate exists on the way *back* — the mapping
    /// this returns is named in **this** isolate's namespace, and
    /// [`Worker::unmap_guest_ram`] refuses a name from any other.
    ///
    /// # Errors
    /// [`RmError::GuestRamUnavailable`] if the VM was launched without a shared memory
    /// backing; otherwise whatever [`RmBackend::map_guest_ram`] refuses with.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1).
    pub fn map_guest_ram(&mut self, grant: GuestRamGrant) -> Result<GuestRamMapped, RmError> {
        kayfabe_util::lockwitness::assert_lock_free("mapping guest RAM into an isolate");
        self.backend.map_guest_ram(grant)
    }

    /// Give back a guest-RAM mapping.
    ///
    /// ★★ The foreign-handle gate **is** here, and it is load-bearing rather than
    /// symmetric-for-tidiness: guest RAM is the one resource whose cross-isolate reach is a
    /// real escalation (isolate A releasing — or, once the enforcement layer lands,
    /// re-authorizing — isolate B's view of guest process B's pages). The same raw value is
    /// live-and-different in every namespace, so an ungated release would act on a
    /// bystander mapping.
    ///
    /// # Errors
    /// [`RmError::ForeignHandle`] before anything runs; otherwise whatever
    /// [`RmBackend::unmap_guest_ram`] refuses with.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1).
    pub fn unmap_guest_ram(&mut self, mapped: GuestRamMapped) -> Result<(), RmError> {
        kayfabe_util::lockwitness::assert_lock_free("releasing a guest-RAM mapping");
        if !mapped.region.belongs_to(self.isolate) {
            return Err(RmError::ForeignHandle {
                handle: mapped.region,
                worker_isolate: self.isolate,
            });
        }
        self.backend.unmap_guest_ram(mapped)
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
            // ★★★ **THE SECOND CROSSING.** Structurally `Publish`'s twin, and the single
            // difference is `alloc_vidmem` where that arm calls `alloc_sysmem` — which is
            // the whole of GEN-2's allocate-and-place chain
            // (`C: mode2_fb_crossing_question.md` §5). ⊘ The unwind, the placement check
            // and the orphan sets are IDENTICAL on purpose: a second, subtly different
            // copy of an unwind is how an orphan gets dropped on the floor.
            VerbPlan::PublishVidmem { host_vas, len, at } => {
                let (vas, fresh_vas) = match *host_vas {
                    Some(h) => (h, None),
                    None => {
                        let h = rm.alloc_vaspace()?;
                        (h, Some(h))
                    }
                };
                let memory = match rm.alloc_vidmem(*len) {
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
                // ★★★ #102 — the same address-identity check, for the same reason. A
                // vidmem object placed anywhere other than `at` is published and
                // unaddressable: the guest's methods name `at` and the host MMU walks for
                // `at`.
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
            // ★★★★★ **ONE MEMORY.** Structurally `Publish`'s twin, and the difference is
            // that the chain hands back a descriptor as well as a handle — the pages the
            // engine will read are pages the VMM can map, which is the whole increment.
            //
            // ⊘ The unwind is deliberately IDENTICAL in shape to the two arms above. The
            // backing itself is NOT in the orphan set and that is not an omission: it is a
            // `memfd` the child owns, released when the child's own table drops it, and
            // `Orphans` frees RM objects and unmaps GPU VAs — a descriptor is neither. The
            // same asymmetry `PinGuestRam` records one arm down for its guest-RAM mapping.
            VerbPlan::JoinFbLeaf {
                host_vas,
                len,
                at,
                phys,
            } => {
                let (vas, fresh_vas) = match *host_vas {
                    Some(h) => (h, None),
                    None => {
                        let h = rm.alloc_vaspace()?;
                        (h, Some(h))
                    }
                };
                let joined = match rm.join_fb_leaf(vas, *len, *at, *phys) {
                    Ok(j) => j,
                    Err(e) => return Err(unwind(rm, fresh_vas.into_iter().collect(), e)),
                };
                // ★★★ #102 — the SAME address-identity check, at the seam that crosses
                // into the untrusted-by-construction backend. The backend is asked to
                // refuse a wrong placement itself and does; this is the second, independent
                // check, and it is here for the reason the `Publish` arm gives: this is the
                // one place a placement can be downgraded silently.
                if joined.host_va != at.0 {
                    let _ = rm.unmap_gpu_va(vas, joined.host_va);
                    let mut orphans = vec![joined.memory];
                    orphans.extend(fresh_vas);
                    return Err(unwind(
                        rm,
                        orphans,
                        RmError::PlacementRefused {
                            want: at.0,
                            got: joined.host_va,
                        },
                    ));
                }
                Ok(VerbReply::FbLeafJoined {
                    host_vas: fresh_vas,
                    joined,
                })
            }
            // ★★★★★ **THE FIRST GUEST BYTE.** Structurally `Publish`'s twin, and every
            // difference between them is a difference in *whose* pages are underneath.
            VerbPlan::PinGuestRam {
                host_vas,
                grant,
                at,
            } => {
                let (vas, fresh_vas) = match *host_vas {
                    Some(h) => (h, None),
                    None => {
                        let h = rm.alloc_vaspace()?;
                        (h, Some(h))
                    }
                };
                // ⊘ The grant is passed through untouched. Nothing here recomputes its
                // offset, clamps its length or checks it against anything — the numbers
                // are the VMM's and the only check available in this process would be a
                // check of a request against itself.
                let mapped = match rm.map_guest_ram(*grant) {
                    Ok(m) => m,
                    Err(e) => return Err(unwind(rm, fresh_vas.into_iter().collect(), e)),
                };
                // ⚠ The unwind sets below name `mapped` NOWHERE, and that is not an
                // omission: `Orphans` frees RM objects and unmaps GPU VAs, and a
                // guest-RAM mapping is neither. It is released on this same worker, in
                // line, before the error leaves — because after that the name is gone and
                // nobody can.
                let memory = match rm.describe_guest_ram(mapped) {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = rm.unmap_guest_ram(mapped);
                        return Err(unwind(rm, fresh_vas.into_iter().collect(), e));
                    }
                };
                let host_va = match rm.map_gpu_va(vas, memory, grant.len(), *at) {
                    Ok(va) => va,
                    Err(e) => {
                        let _ = rm.unmap_guest_ram(mapped);
                        let mut orphans = vec![memory];
                        orphans.extend(fresh_vas);
                        return Err(unwind(rm, orphans, e));
                    }
                };
                // ★★★ `placed_as_asked`. The SAME check `Publish` makes, and it matters
                // more here: a fabricated buffer relocated by RM is merely unreachable,
                // while the guest's own ring relocated by RM is a host channel pointed at
                // whatever else lives at that address. ⊘ Never adopted.
                if host_va != at.0 {
                    let _ = rm.unmap_gpu_va(vas, host_va);
                    let _ = rm.unmap_guest_ram(mapped);
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
                Ok(VerbReply::GuestRamPinned {
                    host_vas: fresh_vas,
                    mapped,
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
            VerbPlan::CeSplit { vas, subs } => {
                // ★★★ §12.4 — THE EXECUTOR IS THE ISOLATE, for both arms. In submission
                // ORDER, because a copy engine's within-request ordering is what the
                // guest's own semaphore release depends on; and one at a time, because a
                // sub-copy that fails must not be followed by later ones that assume it
                // landed.
                let mut host_ce = 0usize;
                let mut ours = 0usize;
                for sub in subs {
                    rm.ce_copy(*vas, *sub)?;
                    match sub.by {
                        CeExecutor::HostCe => host_ce += 1,
                        CeExecutor::Ours => ours += 1,
                    }
                }
                Ok(VerbReply::CeSplit { host_ce, ours })
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

    /// ★★★ **E1 — why this isolate can never serve a verb, at the SEAM.**
    /// `None` means it can (or could, until it was retired in the ordinary way).
    ///
    /// ## The gap this closes, stated as the bench measured it
    ///
    /// `bench_rebuild_notes.md` §5 row 7: *"a **failed** real isolate is
    /// indistinguishable from the stillborn one at the seam"*. Both answer
    /// `pool_size() == 0`-or-all-dead, `checkout() == None`, `is_retired() == true` — and
    /// so does a **saturated** pool for one of those. The reason each already existed as a
    /// fact ([`StillbornIsolate::why`], `HostIsolate::spawn_error`) and was reachable only
    /// through the **concrete** type, which the core never holds: it holds
    /// [`IsolateBox`], i.e. `dyn Isolate`. So a spawn that failed for a nameable reason —
    /// no NVIDIA driver loaded, `clone` refused, descriptor table exhausted — presented to
    /// every layer above as *"nothing happened"*.
    ///
    /// ## ⊘ Why it returns a KIND and not only a sentence
    ///
    /// Because a check keyed on a *word* is satisfied by writing the word. The two
    /// conditions are structurally different and a caller must be able to branch on that
    /// difference without parsing prose: [`RefusalKind::NoPlane`] is a **deployment**
    /// fact chosen by the composition root and true before the process started;
    /// [`RefusalKind::SpawnFailed`] is a **runtime failure** of a plane that was asked for
    /// and could not be had. Only the second one means something is wrong with the host.
    ///
    /// The sentence travels alongside because the kind cannot carry the cause, and the
    /// cause is what an operator acts on.
    fn refusal(&self) -> Option<IsolateRefusal<'_>>;
}

/// Which of the two structurally different reasons an [`Isolate`] refuses everything.
///
/// See [`Isolate::refusal`] for why this is an enum and not a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefusalKind {
    /// **There is no forwarding plane in this build/configuration** — the composition
    /// root installed [`StillbornIsolates`] deliberately. Nothing was attempted and
    /// nothing failed; a boot that shows only this is behaving exactly as configured.
    NoPlane,
    /// **A real plane was asked for and could not be built.** Something was attempted on
    /// the host and it failed: the image would not publish, `clone` was refused, a
    /// socketpair could not be made, or the child's RM bring-up handshake came back
    /// `Failed`. ⊘ This is the one that means *investigate the host*.
    SpawnFailed,
}

impl RefusalKind {
    /// A stable, machine-greppable name. ⊘ Not the sentence — see [`IsolateRefusal::why`].
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalKind::NoPlane => "no-plane",
            RefusalKind::SpawnFailed => "spawn-failed",
        }
    }

    /// Every kind, for gates that must quantify over the whole set rather than over a
    /// list someone can shorten.
    pub const ALL: [RefusalKind; 2] = [RefusalKind::NoPlane, RefusalKind::SpawnFailed];
}

/// One isolate's refusal: what kind, and the sentence that names the cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IsolateRefusal<'a> {
    /// The structural classification — branch on this.
    pub kind: RefusalKind,
    /// The cause, for a human. Borrowed from the isolate, so it costs nothing to ask.
    pub why: &'a str,
}

/// ★★★ **E1 — the isolate plane's health, as ONE value a teardown report can print.**
///
/// Counted over every isolate the device currently holds, plus the monotonic
/// materialization total that survives their death.
///
/// ⊘ **`materialized == 0` is a finding, not a blank.** Since E0b an isolate is spawned
/// by a *guest* RM event, so zero means the guest never got that far — a completely
/// different diagnosis from "it spawned and refuses", and before this counter the two
/// were the same silence on the host side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IsolateCensus {
    /// How many isolates this device has ever materialized (monotonic; survives reaps).
    pub materialized: u64,
    /// How many it holds right now, across every live proc and target.
    pub live: u64,
    /// Of those, how many answer [`RefusalKind::NoPlane`].
    pub no_plane: u64,
    /// Of those, how many answer [`RefusalKind::SpawnFailed`] — ★ the number that means
    /// the host could not do what it was asked.
    pub spawn_failed: u64,
    /// One refusal sentence, verbatim, and its kind.
    ///
    /// ★ **`SpawnFailed` outranks `NoPlane`** when both are present: a plane that was
    /// asked for and broke is strictly more informative than one that was never
    /// installed, and a report with room for one line must carry the actionable one.
    pub first: Option<(RefusalKind, String)>,
}

impl IsolateCensus {
    /// Fold one isolate in: counts it live, and folds its [`Isolate::refusal`].
    pub fn observe(&mut self, iso: &dyn Isolate) {
        self.live = self.live.saturating_add(1);
        self.observe_refusal(iso.refusal());
    }

    /// Fold one **answer** in, without a live count — the fold's own rule, reachable
    /// without an [`Isolate`] to build.
    ///
    /// ★ Split out from [`IsolateCensus::observe`] so the precedence rule below can be
    /// tested against both kinds in both orders. The alternative would have been a test
    /// double that reports `SpawnFailed`, i.e. a mock deciding the answer to the question
    /// under test — the shape this project has already had a planted mutation survive.
    pub fn observe_refusal(&mut self, r: Option<IsolateRefusal<'_>>) {
        let Some(r) = r else { return };
        match r.kind {
            RefusalKind::NoPlane => self.no_plane = self.no_plane.saturating_add(1),
            RefusalKind::SpawnFailed => self.spawn_failed = self.spawn_failed.saturating_add(1),
        }
        let better = match &self.first {
            None => true,
            Some((RefusalKind::NoPlane, _)) => r.kind == RefusalKind::SpawnFailed,
            Some((RefusalKind::SpawnFailed, _)) => false,
        };
        if better {
            self.first = Some((r.kind, r.why.to_string()));
        }
    }

    /// How many live isolates refuse, of either kind.
    #[must_use]
    pub fn refusing(&self) -> u64 {
        self.no_plane.saturating_add(self.spawn_failed)
    }
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
    ///
    /// ★★★ **And this is the door R1 is asserted at on the BIRTH side** — the exact
    /// counterpart of this type's `Drop`, added because the drop half was only ever
    /// half the rule. A real isolate's birth is `clone` into six namespaces,
    /// `execveat` of a sealed memfd and a blocking hello handshake; under
    /// `KAYFABE_ISOLATES=real` it then runs real `NV01_ROOT_CLIENT`/`NV01_DEVICE_0`
    /// ioctls. That is a blocking call in exactly the sense R1 forbids under a lock,
    /// and `kayfabe_linux_raw::ChildSpec::spawn` already asserts it — **for the host
    /// plane only**. The whole suite runs on mocks, so the rule had no mechanism on
    /// the path the suite exercises, and a regression could only ever be found by a
    /// live boot. (It was: `f0b7efa_run_basereal_qemu.log`.)
    ///
    /// Asserting *here* rather than inside each `IsolateFactory` implementation is
    /// what makes it one mechanism instead of three that can drift: this constructor
    /// is the only way an isolate enters core state, whichever plane made it, and
    /// nothing can acquire a lock between the factory's return and this call.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1) or any adapter leaf lock.
    #[must_use]
    pub fn new(isolate: Box<dyn Isolate>) -> Self {
        kayfabe_util::lockwitness::assert_lock_free(
            "materializing an isolate (sandbox spawn: clone into namespaces + execveat + \
             hello handshake)",
        );
        kayfabe_util::leafwitness::assert_leaf_free(
            "materializing an isolate (sandbox spawn: clone into namespaces + execveat + \
             hello handshake)",
        );
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
/// `Send + Sync`: owned by the shared `Gpu` (crate docs, #17).
///
/// ★★★ **`spawn` takes `&self`, and that is a concurrency requirement rather than a
/// style choice.** R1 forbids a blocking call under any lock, and a spawn is the most
/// blocking thing this port does — so the spawn must happen with **zero** locks held,
/// which means the factory has to be reachable *without* the device lock that owns the
/// core. A `&mut` door would have to be guarded by something, and guarding it with a
/// lock held across the spawn is R1's violation wearing a different lock (and one the
/// ranked witness cannot even see, `kayfabe_util::leafwitness`). `Spine` therefore holds
/// the factory behind an `Arc` and the L1 shell keeps its own clone.
///
/// The only state an implementation wanted `&mut` for was its birth witness, which is
/// microsecond bookkeeping and takes its own lock **around the push only** — never
/// across the spawn.
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
    fn spawn(&self, id: IsolateId) -> Box<dyn Isolate>;
}

/// ★★★ An [`IsolateFactory`] that spawns **nothing** — the isolate plane's
/// `kayfabe_device::RefusingRam`.
///
/// Every isolate it hands back is **retired at birth**: pool width zero, `checkout()`
/// always `None`, so the core's ordinary backpressure path refuses every verb before one
/// is composed. It is what a composition root installs when it has an object model to
/// declare facts into and **no forwarding plane to issue them against**.
///
/// ## ⊘ This is not a mock, and the distinction is the whole point
///
/// A mock *answers* a verb — with a made-up handle, a made-up address, a made-up
/// completion — and that is precisely the failure mode `only_live_boots_are_proof` and the
/// project's measured "mock wall" are about: a green result that a real driver never
/// produced. This type answers **no verb at all**. It is the same posture, and the same
/// argument, as [`kayfabe_isolate_host::HostIsolate::stillborn`]'s already-documented
/// behaviour — *"retired at birth, every slot dead, `checkout` returning `None`. That is
/// correct behaviour (the core's backpressure path handles it)"* — reached deliberately
/// rather than by a spawn failing.
///
/// ⚠ **And it is therefore indistinguishable, at the core, from a saturated pool** —
/// exactly the ambiguity `HostIsolate::spawn_error` exists to record. That is why
/// [`StillbornIsolates::why`] is a required constructor argument and not a default: a
/// composition root that installs this owes an operator a sentence, and §4.4.1's pattern
/// is that a deployment fact no type can carry gets checked at realize.
///
/// ⊘ **It is not a substitute for a forwarding plane and must never become one.** The day
/// this port issues a real host verb, the composition root installs a real factory; the
/// only thing this type is allowed to make true is *"the object model can accept protocol
/// facts before the data plane exists"*.
#[derive(Debug)]
pub struct StillbornIsolates {
    why: &'static str,
    /// Every id this factory was asked for, in order — the same witness
    /// `MockIsolateFactory::spawned` and `HostIsolateFactory::spawned` publish, so a test
    /// can assert the isolate-per-`(Proc, GpuId)` property against any of the three.
    ///
    /// ⊘ Behind a `Mutex` because [`IsolateFactory::spawn`] takes `&self` (see the trait
    /// docs). The lock is taken **around the push and nothing else** — never across a
    /// spawn — so it is the microsecond bookkeeping R1 permits, not a lock held over a
    /// blocking call. Read it with [`StillbornIsolates::spawned`].
    spawned: std::sync::Mutex<Vec<IsolateId>>,
}

impl StillbornIsolates {
    /// A factory whose isolates all refuse, naming `why`.
    ///
    /// `why` is `&'static str` rather than a `String` on purpose: it is a *deployment*
    /// fact about this build's composition root, not a runtime condition, so it cannot be
    /// assembled out of anything a guest supplies.
    #[must_use]
    pub fn new(why: &'static str) -> StillbornIsolates {
        StillbornIsolates {
            why,
            spawned: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Every id this factory was asked for, in order (the birth witness).
    ///
    /// # Panics
    /// If the witness mutex was poisoned by a panic inside a `spawn`.
    #[must_use]
    pub fn spawned(&self) -> Vec<IsolateId> {
        self.spawned.lock().expect("the spawn witness").clone()
    }

    /// Why every isolate from this factory refuses — the sentence a composition root
    /// checks at realize (§4.4.1).
    #[must_use]
    pub fn why(&self) -> &'static str {
        self.why
    }
}

impl IsolateFactory for StillbornIsolates {
    fn spawn(&self, id: IsolateId) -> Box<dyn Isolate> {
        self.spawned.lock().expect("the spawn witness").push(id);
        Box::new(StillbornIsolate { id, why: self.why })
    }
}

/// One isolate from [`StillbornIsolates`]: retired at birth, no workers, no verbs.
#[derive(Debug)]
pub struct StillbornIsolate {
    id: IsolateId,
    why: &'static str,
}

impl StillbornIsolate {
    /// Why this isolate refuses. The counterpart of
    /// `kayfabe_isolate_host::HostIsolate::spawn_error`, and `Some` for the same reason:
    /// *"the pool is saturated"* and *"there is no forwarding plane in this build"* are
    /// the same observation at the [`Isolate`] seam, and only one of them is transient.
    #[must_use]
    pub fn why(&self) -> &'static str {
        self.why
    }
}

impl Isolate for StillbornIsolate {
    fn id(&self) -> IsolateId {
        self.id
    }

    /// **Zero.** Not "one slot that is dead": a width the core could see idle-count
    /// against is a width something might one day wait for, and there is nothing here to
    /// wait for. `pool_size() == 0` says *this isolate can never issue a verb* in the one
    /// vocabulary the core already reads.
    fn pool_size(&self) -> usize {
        0
    }

    fn idle_workers(&self) -> usize {
        0
    }

    fn checkout(&mut self) -> Option<Worker> {
        None
    }

    /// ⊘ Unreachable by construction, and it drops rather than panics.
    ///
    /// [`Isolate::checkout`] never yields a [`Worker`], so nothing can ever be returned to
    /// this isolate — the only way here is a caller checking a worker in to the wrong
    /// isolate, which is a bug in the caller. Dropping is what
    /// `kayfabe_isolate_host::HostIsolate::checkin` already does for a dead slot, and a
    /// panic on a guest-reachable path would turn somebody else's bug into this device's
    /// abort.
    fn checkin(&mut self, worker: Worker) {
        drop(worker);
    }

    fn checked_out(&self) -> Vec<WorkerId> {
        Vec::new()
    }

    fn cancel_handle(&self, _worker: WorkerId) -> Option<CancelHandle> {
        None
    }

    /// `false` — there is no slot to mark dead. Distinct from `HostIsolate`'s `true` for
    /// an in-range index: a slot that never existed did not just die.
    fn worker_died(&mut self, _worker: WorkerId) -> bool {
        false
    }

    fn in_flight(&self) -> usize {
        0
    }

    /// A no-op: [`Self::is_retired`] is already `true` and nothing can change it.
    fn retire(&mut self) {}

    /// **Always `true`.** Retired at birth — which is what makes every `checkout`
    /// refusal permanent rather than transient, and is the fact `Spine::reap_retired`
    /// reads.
    fn is_retired(&self) -> bool {
        true
    }

    /// ★ **Always [`RefusalKind::NoPlane`], and never `SpawnFailed`** — nothing was
    /// attempted here. This type is what a composition root installs when it *chose* to
    /// have no forwarding plane, and reporting it as a failure would put a red line in
    /// front of an operator for a build behaving exactly as configured (E1).
    fn refusal(&self) -> Option<IsolateRefusal<'_>> {
        Some(IsolateRefusal {
            kind: RefusalKind::NoPlane,
            why: self.why,
        })
    }
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
    CeExecutor,
    CeSource,
    CeSubCopy,
    UngatedVa,
    VerbReply,
    Orphans,
    VerbFailure,
    dyn Isolate,
    dyn IsolateFactory,
    IsolateBox,
    // ★ Asserted for the concrete types too, not only through the trait objects above: a
    // composition root stores `StillbornIsolates` by value inside the `Sync` core, so
    // losing either bound would be a compile error HERE rather than a confusing one at
    // the far end of `Gpu::realize`.
    StillbornIsolates,
    StillbornIsolate,
    // E1: the census crosses from the core to the composition root through a shared
    // handle, so it must be `Sync` for the same reason the factory is.
    IsolateCensus,
    RefusalKind
);
// The backend and the `Worker` that owns one: `Send + Sync` because pool slots live
// inside the `Sync` core (crate docs), even though no call path ever shares one.
kayfabe_util::assert_send_sync!(dyn RmBackend, Worker);
