//! # kayfabe-fwd — intent recovery → unprivileged host ops
//!
//! The forwarding plane (`mode2_rust_rewrite_architecture.md` §4.2, lesson L2):
//! translate what the guest *means* into **unprivileged** host userspace operations
//! through the owning `Proc`'s isolate — never replay privileged GSP-internal
//! controls. Correctness = observable end-states only.
//!
//! ## Implemented this milestone (the pure-logic slice the simulation drives)
//!
//! - [`publish_backing`] — the data-plane materialization path: back a guest VA range
//!   in a specific **`Vas`** (never "the process" — address ops key on PDB, decision
//!   #14) with a fresh GPA-arena allocation + a host mapping in *that Vas's own host
//!   VAS*. This function IS the #14 fix in code: two procs' identical guest VAs run
//!   through disjoint arenas and disjoint host VASes by construction.
//! - [`plan_doorbell`] — **the ONE ring gate**: `Arch::decode_doorbell` → vChid →
//!   `by_vchid` → `(Proc, Channel)` (in [`route_doorbell`]) → **the #14 ring-gate** (the
//!   channel's Vas working set must be host-published — structural, not caller
//!   discipline) → a `VerbPlan::Doorbell` that materializes/schedules that channel on
//!   **its proc's own** exec plane (nothing one-shot, nothing scalar — crack ⚠4) and
//!   rings its host token on **its proc's own** isolate.
//!
//!   ★ **corrected 2026-07-27** — this list used to read *"[`handle_doorbell`] — the ONE
//!   ring path (there is no other function that reaches `RmBackend::ring_doorbell`)"*.
//!   That cardinality is false and was found by the whitepaper's verification pass:
//!   `RmBackend::ring_doorbell` has exactly **one** call site and it is inside
//!   `kayfabe_isolate::Worker::execute`, which [`handle_doorbell`] reaches only
//!   indirectly — and the L1 path a real guest MMIO write takes,
//!   `kayfabe_rt::SharedDevice::doorbell`, **never enters [`handle_doorbell`] at all**.
//!   The *safety* property is unchanged, one level down: [`plan_doorbell`] is the sole
//!   constructor of `VerbPlan::Doorbell` in the production crates and it runs the gate
//!   before returning one, so `Worker::execute` has nothing un-gated it could be asked to
//!   ring. [`handle_doorbell`] and `SharedDevice::doorbell` are two **compositions** over
//!   that one gate (single-threaded and L1-sharded); neither is a second door.
//!
//!   ★★ **Closed 2026-07-27**: the residual noted here — *"`VerbPlan` is a public enum,
//!   so the guarantee is over the call graph, not enforced by the type system"* — is
//!   gone. `VerbPlan::Doorbell` is `#[non_exhaustive]` and its only constructor,
//!   [`kayfabe_isolate::VerbPlan::gated_doorbell`], runs the gate; hand-building the
//!   variant no longer compiles anywhere outside `kayfabe-isolate`.
//! - [`deliver_completions`] / [`poll_completions`] — glue from the core's completion
//!   policy to `Vmm::raise_irq` (the SWGEN0 edge; transport encoding is `kayfabe-gsp`'s
//!   job once it ports).
//!
//! ## Ports here later (documented skeleton)
//!
//! The ONE pushbuffer method parser (SEM_EXECUTE / MEM_OP / LAUNCH_DMA — address-table
//! §7), the Case-1 shadow-forwarding / Case-2 ack-only tables, CE PT-write capture
//! feed (#13), channel/TSG lifecycle. Each arrives with its regression tests
//! (testing strategy §2).
//!
//! ## Concurrency (decision #17) — the route/act split (L1 cardinal rule R4)
//!
//! This crate is stateless — free functions over the core's types, so the
//! concurrency contract is inherited verbatim from `kayfabe-core` (see its crate
//! docs). Every mixed entry point is factored into the shape the L1 sharding
//! design requires (`l1_concurrency.md` §3.4):
//!
//! - **route** — a pure read of the device-global [`Spine`] (`&Spine`: token
//!   decode, `by_vchid`/`by_pdb` lookup, arch tables). Runs under L1's device
//!   *read* lock; touches no proc.
//! - **act** — the mutation of exactly the routed target (`&mut Proc`, plus
//!   `&Spine` where the act needs routing tables). Runs under that proc's `Mutex`.
//!   ★ Since stage 3 the act phase itself splits into **plan / execute / commit**
//!   (R1): the locked phases only read/decide and re-validate; every `RmBackend`
//!   verb runs between them, lock-free, on a checked-out [`kayfabe_isolate::Worker`].
//! - The original `&mut Gpu` entry points remain as **split-borrow compositions**
//!   of route+act — the single-threaded / degenerate-one-lock shape the tests and
//!   L1-M1 drive.
//!
//! Functions taking `&Gpu`/`&Spine`/`&Proc` are concurrent-safe under shared
//! borrows; functions taking `&mut` require caller-provided exclusivity — and the
//! `&mut Proc` ones ([`publish_backing`], the act phases) parallelize per-proc
//! (disjoint borrows, no shared lock).

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, GpuId, GpuVa, Pdb, VChid};
use kayfabe_completion::{CompletionError, OsEventRef, PostBatch};
use kayfabe_core::gpu::{Channel, Gpu, Proc, Spine};
use kayfabe_core::{ChanId, ProcAnchor, ProcId};
use kayfabe_isolate::{
    CancelReason, ChannelHandles, HostHandle, IsolateId, RmError, VerbPlan, VerbReply, Worker,
    WorkerId,
};
#[doc(inline)]
pub use kayfabe_isolate::{CeExecutor, CeSource, CeSubCopy};
use kayfabe_mmu::AddressTable;
use kayfabe_mmu::{AddressFault, Binding};
use kayfabe_vmm::{FbMeta, IrqSpec, Present, PresentError, SurfaceHandle, Vmm, VmmError};

mod trace;

/// The MSI-X vector completions are raised on. Abstract placeholder until the
/// interrupt-tree model ports (`kayfabe-regs`-equivalent); the mocks assert on it.
pub const COMPLETION_VECTOR: IrqSpec = IrqSpec::Msix(0);

/// Forwarding-plane faults. Loud by design: a routing miss is never resolved by
/// guessing (no content-pick, no MRU scan — those do not exist in the rewrite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FwdFault {
    /// The doorbell token did not decode for this architecture (hostile bytes).
    MalformedToken {
        /// The raw token.
        token: u64,
    },
    /// The decoded vChid has no registered channel **on this target GPU** — forward-
    /// population never saw its channel-alloc. MISS=FAULT (the C's `bar1_wpg` MRU
    /// fallback is designed out). Carries its target: a `VChid` is a per-GPU namespace,
    /// so the miss is scoped to the GPU whose doorbell trapped (MG-3).
    UnknownVchid {
        /// The target GPU the doorbell addressed.
        gpu: GpuId,
        /// The decoded vChid.
        vchid: VChid,
    },
    /// The routed proc exists but is retired (cross-teardown consumption refused —
    /// lesson L10).
    RetiredProc(ProcId),
    /// ★ The op's routing key belongs to a **CONDEMNED component**
    /// (`l1_concurrency.md` §7.3 / §12.13): one of this guest process's isolate workers
    /// died **out of band**, so §7.3's "no resurrect" / `WorkerDied`'s "never a respawn"
    /// make the whole component permanently dead — it has no `Proc`, no isolate, no
    /// GPA arena and no route, and it will get none until the *guest itself* frees
    /// its client root.
    ///
    /// **Why this is a refusal and not a transparent re-materialization.** The isolate
    /// is a process, so the host kernel already reclaimed everything it held; rebuilding
    /// the component's host objects would be *almost* clean. It is wrong anyway, because
    /// **the guest's data died with the isolate** — a published backing is host memory
    /// (`RmBackend::alloc_sysmem`) owned by that isolate's RM client, so a rebuild hands
    /// the guest a **zeroed** backing for a VA it believes still holds its data. Silent
    /// corruption is strictly worse than this fault, which is the semantic real hardware
    /// already has: **sticky-fatal**, like an Xid. It is not a brick — a re-initialising
    /// application (fresh RM client ⇒ different component ⇒ not condemned) is served
    /// normally, and a dying one has its clients freed by the guest kernel.
    ///
    /// Distinct from [`FwdFault::RetiredProc`] because there is no `ProcId` left to
    /// name (the proc was removed and reaped; ids are never reused), and distinct
    /// from [`FwdFault::UnknownPdb`]/[`FwdFault::UnknownVchid`] because the key is
    /// not unknown — it is *forbidden*. The label comes out of the same forward
    /// projection that fills the live routing maps, so naming it costs no reverse
    /// resolution (the `RmGraph::gpu_of` / address-table doctrine).
    Condemned {
        /// The condemned component's deterministic label (its smallest client
        /// handle) — the guest's own identity for the process that lost its worker.
        anchor: ProcAnchor,
    },
    /// The channel is not bound to any declared VAS and system routing does not
    /// apply — refusing to guess an address space.
    NoVas(ChanId),
    /// ★ A copy-engine request partitioned into more sub-copies than [`MAX_CE_SPANS`]
    /// (`#102` stage C2). Guest-influenced on both axes — the request's length and the
    /// address table's fragmentation — so it is bounded, and the bound is a LOUD refusal
    /// rather than a truncation: a partition that stops early silently drops the tail of
    /// a copy.
    CeTooFragmented {
        /// The request's destination.
        dst: GpuVa,
        /// The request's declared length.
        len: u64,
    },
    /// The target proc has no `Vas` for this `(GpuId, PDB)` (data-plane routing miss).
    /// Carries its target: a `Pdb` is a per-GPU namespace (MG-3).
    UnknownPdb {
        /// The target GPU.
        gpu: GpuId,
        /// The PDB that missed on that target.
        pdb: Pdb,
    },
    /// A per-`(Proc, GpuId)` host isolate/arena was not materialized for an op's
    /// target GPU (an internal inconsistency — the derivation ensures one per touched
    /// target). Loud, never a silent cross-GPU reach.
    NoTarget {
        /// The proc.
        proc: ProcId,
        /// The target GPU with no materialized isolate/arena.
        gpu: GpuId,
    },
    /// The address table refused (miss/overlap).
    Address(AddressFault),
    /// The proc's GPA arena is exhausted.
    Arena,
    /// Reading guest memory failed while parsing a pushbuffer (`Vmm::gpa_read`
    /// refused a GPFIFO range). Distinct from [`FwdFault::Arena`] by design: this is
    /// a guest-side read failure, not a host arena-exhaustion condition.
    ///
    /// ★ **And distinct from [`FwdFault::NonRamGpa`]** — this one means *nothing is
    /// there*; that one means *a device is there*. They are near neighbours over the
    /// same call (`testing_doctrine.md` §2 rule 3), and the day they start reporting as
    /// each other a test must change.
    GpaRead {
        /// The guest-physical address the refused read started at.
        gpa: u64,
    },
    /// ★★★ **The guest aimed a descriptor at a device register window.** A GPFIFO
    /// entry (or any other guest-supplied GPA) named an address that resolves to MMIO
    /// rather than to host RAM, and the port refused to serve it.
    ///
    /// # Why this is its own variant and not a `GpaRead`
    ///
    /// It is not a read failure, it is a **refused lock inversion**. `Vmm::gpa_read` is
    /// in-lock legal (`l1_os_shell.md` §6.1) and this call site runs it under the
    /// device read lock, so an implementation that served a device-aimed GPA would take
    /// the VMM's global lock beneath one of our ranked locks — §6.3's ABBA, constructed
    /// on demand by the guest, and invisible to all four of §6.3's enforcement layers.
    /// `[src] v10.2.0 system/physmem.c:3250` (write) / `:3347` (read) →
    /// `prepare_mmio_access` `:3196-3209`.
    ///
    /// Folding it into [`FwdFault::GpaRead`] would make the one observable signal that
    /// the refusal happened indistinguishable from an ordinary unbacked-page miss — the
    /// §12.10 wrong-reason conflation, on the security-relevant arm.
    NonRamGpa {
        /// The first guest-physical address in the requested range that resolves to a
        /// device region.
        gpa: u64,
    },
    /// The isolate's RM backend refused the op.
    Rm(RmError),
    /// A class the guest tried to alloc as an engine object is not one this arch
    /// recognizes as an engine — MISS=FAULT (never guessed into a GR/CE object).
    NotAnEngine(ClassId),
    /// A completion-arm operation was issued on a channel whose [`EngineKind`]
    /// signals through a *different* arm (e.g. arming a mapped fence on a
    /// GR-compute channel, whose completion is the shared-sema arm). The channel's
    /// engine kind selects the arm — exact, never guessed (§2.4's tie-in).
    WrongArm {
        /// The channel the operation targeted.
        chan: ChanId,
        /// Its engine kind (which selects a different arm).
        engine: EngineKind,
    },
    /// The present/display sink refused a GR-graphics scanout.
    Present(PresentError),
    /// The owning proc's completion queue is full — a hostile guest triggered more
    /// completions than it drained. Loud-fault, never unbounded growth (boundary-1).
    Completion(CompletionError),
    /// Every worker in the `(proc, gpu)` isolate's **bounded pool** is in flight
    /// (`l1_concurrency.md` §7.2). This is **backpressure, not failure**: an L1
    /// caller that can wait releases ALL its locks, waits for a return, and re-enters
    /// from the top with full R5 re-validation. It surfaces as a fault only to
    /// callers that chose not to wait (the single-threaded composed entry points).
    PoolSaturated {
        /// The proc whose pool is saturated.
        proc: ProcId,
        /// The target GPU whose isolate pool is saturated.
        gpu: GpuId,
    },
    /// ★ **The op was CANCELLED** — its requester interrupted the in-flight verb
    /// (`l1_concurrency.md` §5.4, §12.16 gap G4). Not a host failure and not
    /// staleness: the host is fine and the proc is typically still very much alive
    /// (the ordinary case is one guest *thread* dying while its process runs on).
    ///
    /// It exists because without it a cancellation arrives as
    /// [`RmError::Other`] and the failure-path re-validation resolves it to
    /// `FwdFault::Rm(..)` whenever the proc is still live — which is the *normal*
    /// cancellation case. That is §12.10's wrong-reason conflation one layer over: a
    /// canary asserting "it refused" would pass while the fault said "the host
    /// failed" about a host that did nothing wrong.
    ///
    /// **Non-retryable.** Re-issuing work whose requester is gone is not a resolution.
    /// It is §12.9's third staleness shape: non-retryable and orphan-carrying (see
    /// [`kayfabe_isolate::VerbFailure`]).
    ///
    /// The mechanism that produces it — the §5.4 interrupt handshake — is L1-M2's;
    /// this is the vocabulary, landed first so it is not a retrofit.
    Cancelled {
        /// The proc whose op was cancelled. Named because the fault must not read as
        /// "this proc is gone" — it usually is not.
        proc: ProcId,
        /// ★ **WHY** it was cancelled (`l1_os_shell.md` §7.3): a proc exiting, a device
        /// reset, the verb watchdog, or the requesting guest thread taking a signal.
        ///
        /// Carried because *"a fault must name the truth, not the symptom"*, and because
        /// the four are operationally different answers for the guest: `ProcExit` means
        /// the work had no requester left, `Watchdog` means the host was too slow, and a
        /// canary that could not tell them apart would pass on whichever it got — which
        /// is the same shape as §14.8 F4's `VmmError` finding one plane over.
        reason: CancelReason,
    },
    /// ★★ **WEDGED** — the host verb never returned and the requester was released
    /// without a reply (`l1_os_shell.md` §7.5, the two-stage watchdog's second expiry).
    ///
    /// Structurally different from [`FwdFault::Cancelled`] in the way that matters: a
    /// cancellation is a fact about the *requester* and leaves a healthy worker behind;
    /// this is a fact about a **host thread in uninterruptible sleep**, which no
    /// user-space design can kill. What the escape converts is an *unbounded silent
    /// stall* into a *bounded loud failure plus a leak we can name, count and report*.
    ///
    /// It is always accompanied — in the same act, never as a reorderable second step —
    /// by the slot dying permanently and the component being condemned. That pairing is
    /// what makes abandoning the reply safe here and nowhere else (§7.2: the desync
    /// hazard is a *future* reader of that channel, and the escape guarantees there is
    /// none).
    Wedged {
        /// The proc whose verb was abandoned. Its component is condemned by the same
        /// act, so every later op of it faults [`FwdFault::Condemned`].
        proc: ProcId,
        /// The target GPU whose isolate wedged.
        gpu: GpuId,
        /// The pool slot that is now permanently dead.
        worker: WorkerId,
    },
    /// ★ **R5**: the world moved while a verb was in flight lock-free, so the commit
    /// phase's target is no longer what the plan named. MISS=FAULT extends to
    /// staleness — the op surfaces this refusal and does **not** "finish what it
    /// started" against a world that no longer contains its target
    /// (`l1_concurrency.md` §3.3 R5, §11 B5).
    Stale(Stale),
    /// ★ **The SYSTEM proc has no data plane** (`l1_concurrency.md` §12.26): something
    /// asked [`publish_backing`] to allocate host memory on `Gpu::system`.
    ///
    /// This is the rule that keeps a cross-`Proc` host reference *unrepresentable*
    /// rather than merely absent. Guest-kernel work that would need a backing —
    /// the CeUtils scrub, the GR golden capture — is **forged** to the system proc's
    /// completion queue, never forwarded, so the system proc never mints host memory
    /// and can therefore never hold a handle a *user* proc's isolate owns. Every real
    /// byte the guest kernel moves on a user process's behalf is forwarded through
    /// **that user proc's own** isolate, which is also the isolate whose death
    /// reclaims it.
    ///
    /// Loud rather than silent because the day someone needs the system proc to
    /// publish, the lifetime question this rule answers has to be re-opened
    /// deliberately — with a refcount or a global quiesce point — not discovered
    /// afterwards.
    SystemDataPlane,
    /// ★★ **A host backing whose object is not in this publication's own isolate
    /// namespace** (`gpga_address_space.md` §9.3, boundary 2).
    ///
    /// The commit phase is where a host object first enters core state, so it is where
    /// the object's owner scope has to be checked — and under **reservation arenas** the
    /// check stops being belt-and-braces. RM grants *objects, not ranges*: an isolate
    /// that holds an arena handle can map **any** offset in it. So adopting a backing
    /// whose object belongs to another isolate does not hand this proc one range, it
    /// hands it reach over that isolate's entire reservation, and it does so through a
    /// binding that reads as perfectly ordinary everywhere downstream.
    ///
    /// The scope tested is [`kayfabe_isolate::HostHandle`]'s own — `belongs_to`, the
    /// same predicate `Worker::execute`'s foreign-handle gate uses — deliberately, so
    /// there is one notion of "whose object is this" rather than two that can drift.
    ///
    /// Not guest-reachable today (the reply comes from this proc's own worker, which
    /// mints in its own namespace), which is exactly why it is a *loud refusal* and not
    /// an assertion: the day something else fills this reply in — a shared arena
    /// allocator, a replayed capture — the wrong answer must be a refusal rather than a
    /// silently adopted cross-isolate mapping.
    ForeignBacking {
        /// The isolate this publication is for.
        isolate: IsolateId,
        /// The handle that does not belong to it.
        memory: HostHandle,
    },
}

/// Which re-validation a commit phase failed (`FwdFault::Stale`). Each variant is a
/// distinct way the world can move across the lock-free verb gap; naming them apart
/// is what makes the §8.4 staleness canaries assert something specific instead of
/// "an error happened".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stale {
    /// The proc retired (or was reaped) while its verb was in flight.
    Proc(ProcId),
    /// The channel the plan targeted was torn down.
    Channel(ChanId),
    /// The `Vas` the plan targeted is gone.
    Vas {
        /// Its target GPU.
        gpu: GpuId,
        /// Its PDB.
        pdb: Pdb,
    },
    /// An `apply`/refresh rewrote routing: `(gpu, vchid)` no longer resolves to the
    /// `(proc, chan)` the plan was made against.
    Route {
        /// The target GPU.
        gpu: GpuId,
        /// The vChid whose route moved.
        vchid: VChid,
    },
    /// The commit's target adopted DIFFERENT host state while this verb was in
    /// flight (a sibling thread's commit won the race). Adopting ours on top would
    /// silently orphan theirs, so the loser refuses and releases what it allocated.
    Rebound,
    /// The `(proc, gpu)` isolate/arena the plan named is gone.
    Target {
        /// The proc.
        proc: ProcId,
        /// The target GPU.
        gpu: GpuId,
    },
}

impl From<CompletionError> for FwdFault {
    fn from(e: CompletionError) -> Self {
        FwdFault::Completion(e)
    }
}

impl From<AddressFault> for FwdFault {
    fn from(f: AddressFault) -> Self {
        FwdFault::Address(f)
    }
}
impl From<RmError> for FwdFault {
    fn from(e: RmError) -> Self {
        FwdFault::Rm(e)
    }
}

// =================================================================================
// ★ THE PLAN / EXECUTE / COMMIT SEAM (`l1_concurrency.md` §3.3, R1's "consequence
// for the core shape"; stage 3 closing the §12.6 gap).
//
// A verb-issuing act phase runs under the owning proc's lock, so it can no longer
// call a blocking `RmBackend` verb in line. Every such site is split in three:
//
//   plan    — under device-read + proc lock: read core state, decide, and EMIT a
//             typed `VerbPlan` plus the ID-shaped hints the commit will need.
//             Emits; does not call. Takes `&Proc` (a pure read) wherever it can.
//   execute — NO locks held: `Worker::execute` runs the chain on a checked-out
//             worker, chaining its own intermediate results (host VAS handle →
//             memory handle → mapped VA) with zero core access. That door asserts
//             R1, so this phase cannot be run under a lock even by accident.
//   commit  — locks re-acquired: RE-VALIDATE (R5) by re-resolving through IDs, then
//             apply the reply to core state — or refuse loudly and hand back the
//             host objects it could not adopt.
//
// Plan products are IDs, never held references (R5's enforcement note), so a commit
// physically cannot dereference something the gap freed. The composed `&mut Proc` /
// `&mut Gpu` entry points below remain, now as *compositions* of the three phases
// that run the round trip on a checked-out worker with no lock held — which is why
// calling one under a lock is an immediate R1 panic instead of a silent violation.
// =================================================================================

/// A locked plan phase's product: the ID-shaped hints its commit needs, plus the
/// verb chain to run lock-free. `verbs = None` means **no host work at all** — the
/// site resolved entirely from core state (an idempotent engine-object replay), so
/// no worker is checked out and the pool is never touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned<P> {
    /// The ID-shaped plan the commit re-validates against.
    pub plan: P,
    /// The verb chain, or `None` when the site needs no host work.
    pub verbs: Option<VerbPlan>,
}

/// Host objects a **refused commit** could not adopt.
///
/// Re-exported from the isolate port, where it moved when [`kayfabe_isolate::Worker`]
/// gained the ability to return one (§12.16, G4: a mid-chain verb failure has orphans
/// too, and the worker cannot depend on this crate).
///
/// R5's disposition rule made explicit: a commit that refuses must not silently leak
/// what its execute phase already allocated. The caller runs
/// [`Orphans::release_plan`] on the SAME worker, still lock-free, before checking it
/// back in.
///
/// ★ **Correction (§12.16, G4).** This doc used to end "*The one case with no such
/// caller is a proc that vanished entirely — then the whole isolate is retired and its
/// handle namespace dies with it … Both dispositions are decided, neither is a leak.*"
/// Both halves of that were wrong:
///
/// - **The namespace dies at the REAP, not at `retire()`.** `Proc::retire` stops the
///   isolate; the sandbox and its handles survive until `Spine::reap_retired` drops the
///   `Proc` — deferredly, at an adapter-declared quiesce point, and (since G3) only
///   once the isolate is quiesced. Between those two moments the objects are held, not
///   disposed of. That is a *deferred* disposition, which is a fine thing to have and
///   a different thing from what the sentence claimed.
/// - **There is a third disposition, and it was unnamed:** a worker that dies
///   mid-chain. Nothing unwinds, the reply never returns, and everything allocated
///   before the failure point is in no `Orphans`, in no core state, and enumerable from
///   nothing. Its only backstop is the same bulk one the C had — the session's fds
///   closing at reap (`C: src/qemu/virtio_nvgpu.c:100-118`, the #80 reaper).
///
/// See [`kayfabe_isolate::VerbFailure`] for the precise limits of what can be
/// enumerated, including the open question about interrupted allocs.
pub use kayfabe_isolate::Orphans;

/// A commit phase's loud refusal: why, what it could not adopt, and whether the op
/// should be re-planned from the top.
///
/// ★ **The two shapes of "the world moved" (a stage-3 finding, `l1_concurrency.md`
/// §12.9).** R5 says a commit whose target vanished must refuse. But not every
/// staleness is a vanishing: first-touch materialization (host VAS, host channel,
/// engine object) is a **compare-and-swap** across the lock-free gap, and two sibling
/// threads of ONE proc racing it is the ordinary case, not an error. The loser has
/// nothing wrong with its request — someone else simply did the work it wanted —
/// so it must **re-resolve**: release its duplicate and re-plan against the winner's
/// state. Refusing there would turn a legal concurrent submission into a spurious
/// guest-visible fault, which is a worse bug than the one R5 prevents.
///
/// ★ **This is the miss taxonomy at the commit seam** (`kayfabe_core` crate docs):
/// converging staleness is a **DEFER** (the world moved *toward* an answer, so re-plan
/// against it — bounded by `MAX_COMMIT_RETRIES` because a defer must terminate) and
/// divergent staleness is a **FAULT** (the plan's target is gone; nothing that can arrive
/// brings it back). The ledger of that defer — every host object a losing attempt
/// allocated, released exactly once — is pinned by `tests/retry_ledger.rs`.
///
/// So: `retry = true` ⇒ *converging* staleness (re-plan, bounded); `retry = false` ⇒
/// *divergent* staleness (the target is gone — MISS=FAULT, surface it).
///
/// ★ **`#[must_use]` (§12.16, G4):** a dropped `Refusal` silently leaks every host
/// object in its [`Orphans`] — the disposition rule this type exists to carry, undone
/// by a missing semicolon's worth of attention. The compiler is the enforcement.
#[must_use = "a dropped Refusal discards its `orphans` — every host object it names \
              leaks. Release them on the checked-out worker (`orphans.release_plan()`) \
              and surface `fault`, or hand the whole Refusal onward."]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The fault to surface to the guest (if this is not retried).
    pub fault: FwdFault,
    /// Host objects the caller must release (see [`Orphans`]).
    pub orphans: Orphans,
    /// True if re-planning from the top is the correct resolution (see above).
    pub retry: bool,
}

impl Refusal {
    /// A divergent refusal with nothing to dispose of: the target is gone.
    fn bare(fault: FwdFault) -> Self {
        Refusal {
            fault,
            orphans: Orphans::default(),
            retry: false,
        }
    }
}

/// The reply shape did not match the plan that produced it — an internal wiring
/// error (the adapter handed a commit someone else's reply), never guest-reachable.
fn wrong_reply<T>(what: &str) -> Result<T, Refusal> {
    panic!("commit phase received a {what} reply that does not match its plan")
}

/// ★ Check a worker OUT of `proc`'s isolate for `gpu` — pool bookkeeping, run under
/// the proc lock (`l1_concurrency.md` §7.3). Moves the worker's handle out to the
/// calling thread; the round trip then runs with no lock held.
///
/// `Ok(None)` is **backpressure**: every worker is in flight (or the isolate is
/// retiring and refuses new checkouts). The caller releases all locks, waits, and
/// re-enters from the top — never spins, never waits under a lock.
///
/// ★ **The miss taxonomy, with the CALLER choosing the category** (`kayfabe_core` crate
/// docs). "No worker available" is a fact that *will* change — a round trip ends and a
/// slot returns — so it is **DEFER** for any caller that can wait: `SharedDevice::verb_op`
/// parks on the pool gate and re-enters with full R5 re-validation, and the wait is
/// counted so saturation is distinguishable from a hang
/// (`kayfabe_rt::device::PoolWaits`). For a caller that *cannot* wait (the single-threaded
/// composed entry points) the same absence surfaces as `FwdFault::PoolSaturated` — a
/// FAULT. Both are correct because the category is a property of the site: the same fact
/// is deferrable exactly when the site can be re-run.
///
/// The missing-isolate arm below is unconditionally FAULT: a `(proc, gpu)` with no
/// isolate is an internal inconsistency, not a fact awaiting arrival.
pub fn checkout(proc: &mut Proc, gpu: GpuId) -> Result<Option<Worker>, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let pid = proc.id;
    Ok(proc
        .isolates
        .get_mut(&gpu)
        .ok_or(FwdFault::NoTarget { proc: pid, gpu })?
        .checkout())
}

/// ★★ [`checkout`] **plus T0's opportunistic drain** (`l1_os_shell.md` §7.6 T0, gap G2)
/// — the form every L1 verb-issuing site uses.
///
/// A checked-out worker is exactly the opportunity T0 names ("*opportunistically at the
/// next verb-issuing op for that proc — the worker is checked out anyway, near-zero
/// marginal cost*"), so the queue rides out of the locked phase with the worker rather
/// than needing a mechanism of its own. The returned [`Orphans`] is empty unless a
/// previous `refresh` dropped a `Vas` or a `Channel` of this `(proc, gpu)` while the proc
/// stayed alive **and** that isolate was otherwise idle — see
/// [`Proc::checkout_with_pending_release`] for why the idle test is load-bearing and why
/// the two must be one act.
///
/// The refusals are [`checkout`]'s, unchanged and checked first, so a retired proc or an
/// unmaterialized target never reaches the drain: a retired isolate refuses every verb
/// including the release, and its disposition of record is the session's death (§7.0).
///
/// # Errors
/// - [`FwdFault::RetiredProc`] — the proc is retired.
/// - [`FwdFault::NoTarget`] — no isolate for this `(proc, gpu)`.
pub fn checkout_and_drain(
    proc: &mut Proc,
    gpu: GpuId,
) -> Result<(Option<Worker>, Orphans), FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    if !proc.isolates.contains_key(&gpu) {
        return Err(FwdFault::NoTarget { proc: proc.id, gpu });
    }
    Ok(proc.checkout_with_pending_release(gpu))
}

/// Return a checked-out worker to its pool slot (proc lock; §7.3). If the target
/// isolate is gone the worker is dropped with it — a retired isolate's slots are not
/// resurrected.
pub fn checkin(proc: &mut Proc, gpu: GpuId, worker: Worker) {
    if let Some(iso) = proc.isolates.get_mut(&gpu) {
        iso.checkin(worker);
    }
}

/// The single-threaded composition of the three phases, used by the `&mut Proc`
/// entry points: check a worker out, run the chain **with no lock held**, commit,
/// dispose of any orphans, check the worker back in.
///
/// L1's `SharedDevice` deliberately does NOT call this — it interleaves the same
/// three phases with lock acquire/release and a pool-full wait. This form exists for
/// callers that already hold exclusive `&mut Proc` (tests, bring-up, the degenerate
/// single-threaded shape), and it inherits R1's teeth for free: it reaches the
/// backend through [`Worker::execute`], which panics if any lock is held.
fn round_trip<T>(
    proc: &mut Proc,
    gpu: GpuId,
    planned_verbs: Option<VerbPlan>,
    commit: impl FnOnce(&mut Proc, Option<VerbReply>) -> Result<T, Refusal>,
) -> Result<T, FwdFault> {
    let Some(verbs) = planned_verbs else {
        return commit(proc, None).map_err(|r| r.fault);
    };
    let pid = proc.id;
    let Some(mut worker) = checkout(proc, gpu)? else {
        return Err(FwdFault::PoolSaturated { proc: pid, gpu });
    };
    let executed = worker.execute(&verbs);
    let out = match executed {
        Ok(reply) => commit(proc, Some(reply)).map_err(|r| {
            if !r.orphans.is_empty() {
                // Residue of a failed release has no sink in the core yet — see
                // `dispose_on` and §12.16's "what remains".
                let _ = worker.execute(&r.orphans.release_plan());
            }
            r.fault
        }),
        // ★ G4 (§12.16): cancellation is named apart from host failure here too, and
        // the failure's own orphans get a disposal attempt on the same worker before
        // it is checked back in.
        Err(f) => {
            let reason = worker.cancel_observed();
            // ★★ §7.5 — a WEDGED worker cannot dispose of anything: it is still inside
            // the ioctl that wedged it. Asking it to would produce a second wedge, so
            // the chain's intermediates go straight onto the proc's `pending_release`
            // queue, where §12.35's audit can NAME them. Every other failure still gets
            // its disposal attempt on the same live worker first — and what that could
            // not dispose of is staged as well, closing the `let _ =` §12.16 left here.
            let residue = if f.err == RmError::Wedged {
                f.orphans
            } else {
                dispose_on(&mut worker, f.orphans)
            };
            proc.stage_release(gpu, residue);
            Err(match f.err {
                RmError::Wedged => FwdFault::Wedged {
                    proc: pid,
                    gpu,
                    worker: worker.id(),
                },
                e => verb_fault(pid, e, reason),
            })
        }
    };
    checkin(proc, gpu, worker);
    out
}

/// Best-effort disposal of a verb failure's `orphans` on the SAME worker, still
/// lock-free — and it hands back **what it still could not dispose of**
/// (`l1_concurrency.md` §12.16, gap G4).
///
/// The residue has no core-side sink yet: recording undisposed host objects across a
/// proc's lifetime is the reclamation ledger, which is L1-M2's to design. Until it
/// exists the disposition of record is the one the C also relied on — the isolate's
/// whole handle namespace dying when its session is reaped
/// (`C: src/qemu/virtio_nvgpu.c:100-118`). This function exists so that the residue is
/// a **named, returned value** at every call site rather than a swallowed `let _ =`,
/// which is what makes the ledger an addition later instead of a retrofit.
#[must_use = "the returned residue is the set of host objects that STILL exist and \
              could not be disposed of — bind it and say what happens to it."]
pub fn dispose_on(worker: &mut Worker, orphans: Orphans) -> Orphans {
    if orphans.is_empty() {
        return orphans;
    }
    match worker.execute(&orphans.release_plan()) {
        Ok(_) => Orphans::default(),
        Err(f) => f.orphans,
    }
}

/// ★ Surface a lock-free verb failure as a forwarding fault, keeping **cancellation
/// distinct from host failure** (`l1_concurrency.md` §12.16, gap G4; §12.10 one layer
/// over).
///
/// [`RmError::Interrupted`] is a fact about the *requester* — a guest thread died or
/// took a signal and its in-flight verb was interrupted (§5.4). Reporting it as
/// `FwdFault::Rm` would say "the host refused" about a host that did exactly what it
/// was asked. Every other `RmError` is a genuine host refusal and stays one.
///
/// ★ `reason` is what the worker's own cancel seam **observed**
/// ([`kayfabe_isolate::Worker::cancel_observed`]), read lock-free by the executing
/// thread. `None` with an `Interrupted` error means the break signal landed but nobody
/// recorded why — a backend bug, not a guest condition — so it is surfaced as
/// [`CancelReason::GuestSignal`], the §5.4 founding case, rather than guessed at or
/// silently re-typed as a host failure.
#[must_use]
pub fn verb_fault(proc: ProcId, err: RmError, reason: Option<CancelReason>) -> FwdFault {
    match err {
        RmError::Interrupted => FwdFault::Cancelled {
            proc,
            reason: reason.unwrap_or(CancelReason::GuestSignal),
        },
        e => FwdFault::Rm(e),
    }
}

/// Result of one backing publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Published {
    /// The GPA carved from the proc's private arena.
    pub gpa: u64,
    /// The host GPU VA this range is addressable at — **equal to the guest VA**
    /// (`#102`, address identity). It is reported rather than dropped because a caller
    /// that just published wants the fact confirmed, not re-derived.
    pub host_va: u64,
    /// ★ #102 — the host memory object backing the range, in the minting isolate's
    /// namespace.
    ///
    /// Added because address identity took a fact away: the host VA used to encode which
    /// isolate produced it (the mock minted it out of `(proc, GPU)` bit lanes, and half a
    /// dozen tests read provenance straight off those bits). It cannot any more — the
    /// address is now the *guest's* number and says nothing about who mapped it. A
    /// [`HostHandle`] does say, exactly and by type: it is `(Proc, GpuId)`-scoped, so
    /// `memory.isolate()` is the provenance those tests were approximating.
    pub memory: HostHandle,
}

/// Back `[va, va+len)` in the `Vas` identified by `(gpu, pdb)` inside `proc`:
/// carve GPA from the proc's **per-target** arena, allocate host memory + map it into
/// the Vas's own host VAS via the proc's **per-target** isolate, and forward-populate
/// the address table.
///
/// Keying discipline (decision #14 + MG-3/MG-5): the caller routes here via
/// `Gpu::by_pdb[(gpu, pdb)]`; a `Pdb` is a per-GPU namespace, so the target GPU is
/// part of the address op's identity. The `Proc` owns one arena + isolate PER target
/// (a bug on GPU0 cannot reach GPU1's host handles).
pub fn publish_backing(
    proc: &mut Proc,
    gpu: GpuId,
    pdb: Pdb,
    va: GpuVa,
    len: u64,
) -> Result<Published, FwdFault> {
    let planned = plan_publish(proc, gpu, pdb, va, len)?;
    round_trip(proc, gpu, planned.verbs, |proc, reply| {
        commit_publish(proc, &planned.plan, reply)
    })
}

/// The ID-shaped hints [`commit_publish`] re-validates against. Identities only —
/// never a held reference into core state (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishPlan {
    /// The owning proc.
    pub proc: ProcId,
    /// The target GPU (isolate + arena key).
    pub gpu: GpuId,
    /// The `Vas`'s PDB.
    pub pdb: Pdb,
    /// The guest VA being backed.
    pub va: GpuVa,
    /// Length.
    pub len: u64,
    /// The `Vas`'s host VAS **as observed at plan time** — `None` means the chain
    /// allocates one, and the commit must refuse if someone else materialized one in
    /// the gap (Stale::Rebound) rather than orphaning theirs.
    pub host_vas: Option<HostHandle>,
}

/// PLAN (R1): decide `publish_backing`'s host work from core state and emit it.
/// A pure `&Proc` read — nothing is mutated until the commit.
pub fn plan_publish(
    proc: &Proc,
    gpu: GpuId,
    pdb: Pdb,
    va: GpuVa,
    len: u64,
) -> Result<Planned<PublishPlan>, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    // ★ §12.26 — the system-plane rule, enforced at the ONE site that mints host
    // memory, and BEFORE any host verb exists (so there is nothing to orphan).
    // ★ §12.26 — the system-plane rule, enforced at the ONE site that mints host
    // memory, and BEFORE any host verb exists (so there is nothing to orphan).
    if proc.id == Gpu::SYSTEM_PROC {
        return Err(FwdFault::SystemDataPlane);
    }
    let pid = proc.id;
    let vas = proc
        .vases
        .get(&(gpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu, pdb })?;
    // The arena and the isolate must both exist BEFORE any host verb runs: a target
    // miss is an internal inconsistency, and finding it after the allocs would mean
    // allocating host state for a target we then refuse.
    if !proc.arenas.contains_key(&gpu) || !proc.isolates.contains_key(&gpu) {
        return Err(FwdFault::NoTarget { proc: pid, gpu });
    }
    // ★★★ #102 — refuse an already-bound VA HERE, before a single host verb exists.
    //
    // Address identity made this necessary *and* possible. Necessary: the host VAS is
    // occupied at exactly this address too, so the map verb would now fail inside the
    // driver — and the caller would learn about the collision as `Rm(NoMemory)` after
    // allocating a VAS and a memory object, with the core's own `Overlap` vocabulary
    // shadowed by the host's. Possible: before identity the core could not know, because
    // the host chose the address and every map got a fresh one.
    //
    // This is the plan-side half only. `commit_publish`'s bind still refuses (R5) — a
    // sibling thread can bind this range in the gap between the read and the commit, and
    // *that* is the case the commit check exists for. Checking twice is not redundancy:
    // the cheap check avoids host work, the late check is the correctness one.
    if vas.table.resolve(pdb, va).is_ok() {
        return Err(FwdFault::Address(AddressFault::Overlap { pdb, va }));
    }
    let host_vas = vas.host_vas;
    Ok(Planned {
        plan: PublishPlan {
            proc: pid,
            gpu,
            pdb,
            va,
            len,
            host_vas,
        },
        // ★★★ #102 — the guest VA travels INTO the host verb. The plan no longer says
        // "map this somewhere and tell me where"; it says "map this at the address the
        // guest named", which is the only request whose answer a forwarded pushbuffer
        // can use.
        verbs: Some(VerbPlan::Publish {
            host_vas,
            len,
            at: va,
        }),
    })
}

/// COMMIT (R5): re-resolve everything through IDs and apply the reply — carve the
/// GPA from the proc's own arena and forward-populate the address table — or refuse
/// loudly and hand back what could not be adopted.
///
/// # Panics
/// If `reply` is not the [`VerbReply::Published`] its plan asked for (an adapter
/// wiring error, never guest-reachable).
pub fn commit_publish(
    proc: &mut Proc,
    plan: &PublishPlan,
    reply: Option<VerbReply>,
) -> Result<Published, Refusal> {
    let Some(VerbReply::Published {
        host_vas: fresh_vas,
        memory,
        host_va,
    }) = reply
    else {
        return wrong_reply("publish");
    };
    // Everything this commit could fail to adopt, in release order.
    let orphans = |vas_used: HostHandle, with_vas: Option<HostHandle>| Orphans {
        unmap: vec![(vas_used, host_va)],
        free: with_vas.into_iter().chain([memory]).collect(),
    };
    let vas_used = fresh_vas
        .or(plan.host_vas)
        .expect("chain produced a host VAS");

    if proc.is_retired() || proc.id != plan.proc {
        return Err(Refusal {
            fault: FwdFault::Stale(Stale::Proc(plan.proc)),
            orphans: orphans(vas_used, fresh_vas),
            retry: false,
        });
    }
    let pid = proc.id;

    // ★★ §9.3 — OWNER SCOPE, checked where a host object ENTERS core state.
    //
    // ★ Ordered AFTER the R5 identity guard, and that ordering was corrected rather than
    // chosen: placed before it, this refusal fired on a commit applied to the *wrong
    // proc* and reported "foreign handle" about a plan/proc mismatch — §12.10's
    // wrong-reason conflation, masking the root cause with a symptom of it
    // (`l1_verb_seam.rs::commit_publish_and_doorbell_proc_guards_refuse_on_either_term_alone`
    // caught it). Here the proc's identity is already established, so "is this object
    // ours" is a well-posed question rather than a consequence of a different failure.
    // See [`FwdFault::ForeignBacking`].
    let isolate = IsolateId::new(pid.0, plan.gpu);
    if !memory.belongs_to(isolate) {
        return Err(Refusal {
            fault: FwdFault::ForeignBacking { isolate, memory },
            // Only what is OURS goes on the release list. `memory` is another isolate's
            // object: we have no standing to free it, and queueing it would ask this
            // proc's worker to free across the very boundary this refusal names.
            orphans: Orphans {
                unmap: vec![(vas_used, host_va)],
                free: fresh_vas.into_iter().collect(),
            },
            retry: false,
        });
    }

    let Proc { vases, arenas, .. } = proc;
    let Some(vas) = vases.get_mut(&(plan.gpu, plan.pdb)) else {
        return Err(Refusal {
            fault: FwdFault::Stale(Stale::Vas {
                gpu: plan.gpu,
                pdb: plan.pdb,
            }),
            orphans: orphans(vas_used, fresh_vas),
            retry: false,
        });
    };
    // R5 on the host VAS itself: the plan decided whether to allocate one by reading
    // `vas.host_vas`; if that answer changed in the gap, a sibling thread won and our
    // fresh VAS (plus everything mapped into it) is an orphan.
    match (plan.host_vas, fresh_vas) {
        (None, Some(fresh)) => {
            if vas.host_vas.is_some() {
                // Converging: a sibling materialized this Vas's host VAS first. Free
                // ours and re-plan — the retry maps into the winner's VAS.
                return Err(Refusal {
                    fault: FwdFault::Stale(Stale::Rebound),
                    orphans: orphans(fresh, Some(fresh)),
                    retry: true,
                });
            }
            vas.host_vas = Some(fresh);
        }
        (Some(known), None) => {
            if vas.host_vas != Some(known) {
                return Err(Refusal {
                    fault: FwdFault::Stale(Stale::Rebound),
                    orphans: orphans(known, None),
                    retry: true,
                });
            }
        }
        _ => unreachable!("the publish chain allocates a host VAS iff the plan had none"),
    }
    let Some(arena) = arenas.get_mut(&plan.gpu) else {
        return Err(Refusal {
            fault: FwdFault::Stale(Stale::Target {
                proc: pid,
                gpu: plan.gpu,
            }),
            orphans: orphans(vas_used, None),
            retry: false,
        });
    };
    let block = arena.alloc(plan.len, 0x1000).map_err(|_| Refusal {
        fault: FwdFault::Arena,
        orphans: orphans(vas_used, None),
        retry: false,
    })?;
    let gpa = block.gpa;
    if let Err(e) = vas.table.bind(
        plan.pdb,
        plan.va,
        plan.len,
        Binding {
            phys: gpa.0,
            aperture: Aperture::SysmemCoherent,
            // ★ G1 (§12.16): the ALLOCATION travels with the PLACEMENT. Storing
            // only `host_va` here is what made the host memory object
            // unreachable from core state — a bound range no reclaim path could
            // ever free. `HostBacking` makes that omission untypeable.
            //
            // ★ `whole` and not `slice` (`gpga_address_space.md` §8.2): this chain
            // allocates a fresh host object per publication, so the binding IS the
            // object and its release frees it. Arena sub-allocation is the OTHER
            // constructor, and nothing mints it yet — `VerbReply::Published` has no
            // offset to carry one, and that reply lives on the isolate seam.
            host: Some(kayfabe_mmu::HostBacking::whole(memory, host_va)),
        },
    ) {
        // ★ G6: the bind refused, so the GPA is owed straight back. Before the arena
        // had a `free` this range simply leaked for the life of the proc.
        let returned = arena.free(block).is_ok();
        debug_assert!(returned, "a block returns to the arena that cut it");
        return Err(Refusal {
            fault: FwdFault::Address(e),
            orphans: orphans(vas_used, None),
            retry: false,
        });
    }
    // ★ G6: keep the token beside the binding, so the range is reclaimable by name.
    vas.blocks.insert(plan.va.0, block);
    Ok(Published {
        gpa: gpa.0,
        host_va,
        memory,
    })
}

/// ★ G6 — reclaim ONE published backing (`l1_concurrency.md` §12.20): unbind the range,
/// return its GPA to **this proc's own** arena, and hand back the host objects the caller
/// must release.
///
/// This is the intra-proc counterpart of `Spine::reap_retired`, and it exists because
/// `GpaArena` used to have no `free` at all: reclamation was whole-arena-at-proc-death
/// only, so a long-lived process that maps and unmaps repeatedly walked its cursor to the
/// end and took a permanent [`FwdFault::Arena`]. That is the C's #80 leak
/// (`teardown_hardening_done`) reproduced one level down after being fixed one level up.
///
/// Like G1's reclaim, this is the *mechanism*; **when** to call it is the caller's,
/// driven by declared graph facts (the `RmGraph` refcounts DUP_OBJECT from the protocol,
/// so liveness is known rather than inferred — there is deliberately no collector here).
/// The host half travels with it in the returned [`Orphans`] for the same reason the two
/// must not drift apart: a GPA recycled while its host memory is still mapped is the
/// `ALREADY-MAPPED` class, so the pair is one call.
///
/// # Errors
/// - [`FwdFault::NoTarget`] — the proc has no arena for this GPU.
/// - [`FwdFault::UnknownPdb`] — the `Vas` is gone (its VASpace was freed by the guest).
/// - [`FwdFault::Address`] with [`AddressFault::Miss`] — **nothing is owed at this VA**:
///   it was never host-published here, or it was already reclaimed. The arena must never
///   accept a range it does not owe, so this is refused before anything is mutated.
pub fn unpublish_backing(
    proc: &mut Proc,
    gpu: GpuId,
    pdb: Pdb,
    va: GpuVa,
) -> Result<Orphans, FwdFault> {
    let pid = proc.id;
    if !proc.arenas.contains_key(&gpu) {
        return Err(FwdFault::NoTarget { proc: pid, gpu });
    }
    let Proc { vases, arenas, .. } = proc;
    let vas = vases
        .get_mut(&(gpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu, pdb })?;
    // The token FIRST, and only *read* first: a VA this Vas owes nothing at is refused
    // with the table still untouched, so a double free changes nothing at all.
    if !vas.blocks.contains_key(&va.0) {
        return Err(FwdFault::Address(AddressFault::Miss { pdb, va }));
    }
    let block = vas.blocks.remove(&va.0).expect("checked above");
    let host_vas = vas.host_vas;
    let backing = vas.table.unbind(va).and_then(|(_len, b)| b.host);
    let arena = arenas.get_mut(&gpu).expect("checked above");
    if arena.free(block).is_err() {
        // Unreachable while a live proc keeps its arena: the block names the arena that
        // cut it. Loud rather than a panic, and the range stays out of circulation.
        return Err(FwdFault::Stale(Stale::Target { proc: pid, gpu }));
    }
    let mut out = Orphans::default();
    if let (Some(host_vas), Some(h)) = (host_vas, backing) {
        // The UNMAP is unconditional — the GPU mapping is per-binding either way.
        out.unmap.push((host_vas, h.host_va()));
        // ★★ The FREE is not (`gpga_address_space.md` §8.2/§9.3). `frees_object()` is
        // false for an arena slice: the object serves sibling bindings at other offsets,
        // so freeing it here would unmap the arena out from under them — the first
        // release destroying what the last one owns. The arena is freed by its own
        // owner, and until that owner exists the isolate process boundary is the
        // backstop (§7.0), never this call.
        if h.frees_object() {
            out.free.push(h.memory());
        }
    }
    Ok(out)
}

/// ROUTE: which proc owns `(target, pdb)`? A pure spine read (`by_pdb`) — the
/// data-plane routing half of the route/act split.
///
/// **MISS ⇒ FAULT** (`kayfabe_core` crate docs, the miss taxonomy). This is a *use* site:
/// the guest has addressed a VAS, so "the PDB has not been declared yet" is not a fact
/// that can still arrive **for this operation** — the operation is now. The derivation
/// layer defers (`Gpu::sync_proc_to_boundary`); routing refuses. That pairing is what
/// makes the refusal exact instead of merely early.
///
/// ★ §12.13: a miss is checked against the condemned map before it is reported, so a
/// key whose component lost a worker out of band gets [`FwdFault::Condemned`] — the
/// *specific* refusal — instead of an anonymous `UnknownPdb`. Both are misses; only
/// one of them is a security-relevant fact.
pub fn route_pdb(spine: &Spine, target: GpuId, pdb: Pdb) -> Result<ProcId, FwdFault> {
    if let Some(&pid) = spine.by_pdb.get(&(target, pdb)) {
        return Ok(pid);
    }
    if let Some(anchor) = spine.condemned_pdb(target, pdb) {
        return Err(FwdFault::Condemned { anchor });
    }
    Err(FwdFault::UnknownPdb { gpu: target, pdb })
}

/// Resolve `va` in `proc`'s `Vas` identified by `(target, pdb)` — the per-proc
/// read half of [`resolve`] (L1: device read lock + that proc's lock). Pure lookup.
///
/// **MISS ⇒ FAULT**, both terms: an unknown `(target, pdb)` is `FwdFault::UnknownPdb`,
/// and an unbound VA is `AddressFault::Miss`. Nothing defers here — the address table IS
/// the guest's TLB, and a TLB has no "later" (`kayfabe_mmu` crate docs).
pub fn resolve_in(
    proc: &Proc,
    target: GpuId,
    pdb: Pdb,
    va: GpuVa,
) -> Result<(Binding, u64), FwdFault> {
    let vas = proc
        .vases
        .get(&(target, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu: target, pdb })?;
    Ok(vas.table.resolve(pdb, va)?)
}

/// Resolve `va` in the `Vas` identified by `(gpu, pdb)`. Pure lookup; MISS=FAULT.
/// Composition of [`route_pdb`] + [`resolve_in`].
pub fn resolve(gpu: &Gpu, target: GpuId, pdb: Pdb, va: GpuVa) -> Result<(Binding, u64), FwdFault> {
    let pid = route_pdb(&gpu.spine, target, pdb)?;
    let proc = gpu.procs.get(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    resolve_in(proc, target, pdb, va)
}

/// Outcome of a doorbell dispatch, for assertions and tracing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorbellOutcome {
    /// The proc the token routed to.
    pub proc: ProcId,
    /// The channel it routed to.
    pub chan: ChanId,
    /// The host token that was rung.
    pub host_token: u64,
    /// True if this dispatch had to schedule the channel first (first submission).
    pub scheduled_now: bool,
}

/// Check every VA in `working_set` resolves **host-published** in `table` — the
/// #14 gate condition. Bound-but-unpublished (`Binding::host = None`, the exact #14
/// EXECUTION fault: the shadow had it, the host VAS did not) and unbound are both
/// loud faults, never a guess (`execution_plane.md` §2.4).
///
/// ★ This is the **query** form, used by [`gate_working_set`] and the address-probe
/// sites. The **enforcing** form is [`VasGate`] below, which the same predicate drives
/// from inside `VerbPlan::gated_doorbell` — one predicate, two callers, no second
/// definition to drift.
fn gate_vas(
    table: &AddressTable,
    pdb: Pdb,
    working_set: impl IntoIterator<Item = GpuVa>,
) -> Result<(), FwdFault> {
    for va in working_set {
        if !host_published(table, pdb, va) {
            return Err(FwdFault::Address(AddressFault::Miss { pdb, va }));
        }
    }
    Ok(())
}

/// THE gate predicate, in one place: `va` resolves in `table` under `pdb` **and** its
/// binding carries a host publication.
///
/// Both misses collapse to one answer deliberately — `AddressTable::resolve` already
/// reports an unresolved VA as `AddressFault::Miss { pdb, va }`, which is the same fault
/// a resolved-but-unpublished VA gets, because they are the same thing to a ring: an
/// address the host GR VAS cannot translate. (That equality is what lets
/// [`kayfabe_isolate::RingWorkingSet`] be a bare predicate without the two crates
/// growing two classifications of one miss.)
fn host_published(table: &AddressTable, pdb: Pdb, va: GpuVa) -> bool {
    matches!(table.resolve(pdb, va), Ok((binding, _off)) if binding.host.is_some())
}

/// ★★ The **enforcing** #14 ring-gate: one channel's `Vas`, handed to
/// [`kayfabe_isolate::VerbPlan::gated_doorbell`] — the only constructor of a
/// `VerbPlan::Doorbell` — which runs [`host_published`] over the submission's working
/// set before a plan exists at all.
///
/// Keyed by PDB, which is the whole of #14: two procs' *identical* guest VAs resolve in
/// their OWN `Vas`, so the gate passes for both only because each published into its own
/// host VAS (distinct `HostHandle`s).
struct VasGate<'a>(&'a AddressTable, Pdb);

impl kayfabe_isolate::RingWorkingSet for VasGate<'_> {
    fn is_host_published(&self, va: GpuVa) -> bool {
        host_published(self.0, self.1, va)
    }
}

/// The `Vas`-less view: a GSP-managed, system-routed channel (`Channel::vas_pdb =
/// None`) has no address space to have published anything *into*, so nothing is
/// published and only an **empty** working set is gateable — exactly the pre-existing
/// `None if working_set.is_empty()` arm, now expressed as the address plane it is
/// rather than as a special case beside the gate.
struct NoVasGate;

impl kayfabe_isolate::RingWorkingSet for NoVasGate {
    fn is_host_published(&self, _va: GpuVa) -> bool {
        false
    }
}

/// A routed doorbell: everything the act phase needs, resolved by a pure spine
/// read (the ROUTE half of L1 cardinal rule R4). Carries the routing identities so
/// act-phase faults name the same `(GpuId, VChid)` the trap addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorbellRoute {
    /// The owning proc the token routed to.
    pub proc: ProcId,
    /// The channel it routed to.
    pub chan: ChanId,
    /// The target GPU the doorbell addressed (the BAR that trapped).
    pub gpu: GpuId,
    /// The decoded vChid (per-GPU runlist index).
    pub vchid: VChid,
    /// The raw token (recorded per-proc for poll-kick replay).
    pub token: u64,
}

/// Which exec-plane miss is this? ★ §12.13: a `(gpu, vchid)` that misses `by_vchid`
/// is either genuinely unknown or the exec plane of a **condemned** component; the
/// condemned map answers that forward, out of the same projection.
fn vchid_miss(spine: &Spine, gpu: GpuId, vchid: VChid) -> FwdFault {
    match spine.condemned_vchid(gpu, vchid) {
        Some(anchor) => FwdFault::Condemned { anchor },
        None => FwdFault::UnknownVchid { gpu, vchid },
    }
}

/// ROUTE (R4): decode a doorbell token and demux it to its owning `(Proc, Channel)`
/// — a **pure read of the spine** (`Arch::decode_doorbell` + `by_vchid`), no proc
/// touched, no `&mut` anywhere. In L1 this runs under the device *read* lock only.
///
/// ★ MG-3: the vChid demux is keyed on `(target GPU, vChid)` — the doorbell's
/// target names WHICH GPU (the BAR that trapped); a vChid is a per-GPU runlist
/// index, so identical vChids on two GPUs route to their own channels.
pub fn route_doorbell(
    spine: &Spine,
    target_gpu: GpuId,
    token: u64,
) -> Result<DoorbellRoute, FwdFault> {
    let target = spine
        .arch()
        .decode_doorbell(token)
        .ok_or(FwdFault::MalformedToken { token })?;
    let (pid, cid) = *spine
        .by_vchid
        .get(&(target_gpu, target.vchid))
        .ok_or_else(|| vchid_miss(spine, target_gpu, target.vchid))?;
    Ok(DoorbellRoute {
        proc: pid,
        chan: cid,
        gpu: target_gpu,
        vchid: target.vchid,
        token,
    })
}

/// ACT (R4): run the routed doorbell against **its owning proc only** —
/// `&mut Proc`, never `&mut Gpu`. Ring-gate → lazy materialization/schedule → ring.
///
/// ★ **The single-threaded composition of [`plan_doorbell`] / `Worker::execute` /
/// [`commit_doorbell`]** (R1). It reaches the backend through the worker door, which
/// asserts R1 — so calling THIS under a proc lock is an immediate named panic, not a
/// silent convoy. L1's `SharedDevice` drives the three phases itself, interleaved
/// with its lock acquire/release and its pool-full wait.
///
/// `working_set` is the set of VAs this submission's work touches, as recovered by
/// the caller (launch descriptors / submit parse). A declared VA that is unbound or
/// bound-but-unpublished (`host_va = None` — the emulator's shadow had it, the
/// channel's OWN host VAS did not) is a loud fault BEFORE the channel is even
/// materialized, never a cross-proc content-pick. (An empty `working_set` is an
/// honest "this submission touches no tracked VA" — there is nothing to fault on,
/// and no host state is at risk.)
///
/// Materialization is lazy and **per-proc**: the first doorbell on a channel
/// allocates + schedules its host channel in its Vas's host VAS through its own
/// isolate (no warm-up assumption — testing strategy `wo_channel_alloc_then_
/// immediate_doorbell`), and the "already scheduled" state lives on the proc's
/// [`kayfabe_core::gpu::ExecPlane`] — there is no global one-shot to leave a second
/// proc's channel off-runlist (#12's CTX2 bug, crack ⚠4).
pub fn exec_doorbell(
    spine: &Spine,
    proc: &mut Proc,
    route: &DoorbellRoute,
    working_set: &[GpuVa],
) -> Result<DoorbellOutcome, FwdFault> {
    let planned = plan_doorbell(proc, route, working_set)?;
    let gpu = planned.plan.cgpu;
    round_trip(proc, gpu, planned.verbs, |proc, reply| {
        commit_doorbell(spine, proc, &planned.plan, reply)
    })
}

/// The ID-shaped hints [`commit_doorbell`] re-validates against (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorbellPlan {
    /// Owning proc.
    pub proc: ProcId,
    /// The channel.
    pub chan: ChanId,
    /// The GPU the doorbell trapped on (routing key with `vchid`).
    pub gpu: GpuId,
    /// The channel's OWN target GPU (its isolate/arena key).
    pub cgpu: GpuId,
    /// The decoded vChid.
    pub vchid: VChid,
    /// The raw token (recorded per-proc for poll-kick replay).
    pub token: u64,
    /// The channel's declared VAS, if any.
    pub vas_pdb: Option<Pdb>,
    /// The channel's host handles **as observed at plan time** (`None` = the chain
    /// materializes them).
    pub channel: Option<ChannelHandles>,
    /// Whether this submission must schedule the channel first.
    pub schedule: bool,
}

/// PLAN (R1) for the ONE ring path. Runs the #14 ring-gate **before any host op**
/// exactly as before — the gate now lives in the phase that holds the lock, which is
/// strictly stronger: it is checked against the same consistent snapshot the plan is
/// derived from.
///
/// A pure `&Proc` read; nothing is mutated until the commit.
pub fn plan_doorbell(
    proc: &Proc,
    route: &DoorbellRoute,
    working_set: &[GpuVa],
) -> Result<Planned<DoorbellPlan>, FwdFault> {
    let pid = route.proc;
    let cid = route.chan;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
    }
    let chan: &Channel = proc.channels.get(&cid).ok_or(FwdFault::UnknownVchid {
        gpu: route.gpu,
        vchid: route.vchid,
    })?;
    let cgpu = chan.gpu;
    if !proc.isolates.contains_key(&cgpu) {
        return Err(FwdFault::NoTarget {
            proc: pid,
            gpu: cgpu,
        });
    }

    // ---- The channel's own `Vas`, resolved BEFORE any host op. A declared PDB whose
    //      `Vas` is absent is a loud refusal here, exactly as before.
    let vas = match chan.vas_pdb {
        Some(pdb) => Some(
            proc.vases
                .get(&(cgpu, pdb))
                .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?,
        ),
        None => None,
    };

    let channel = chan.host_channel.zip(chan.host_token);
    // Lazy per-proc materialization: the channel's graph-derived `EngineKind` rides
    // the alloc so the adapter lands it on the RIGHT runlist (GR-1: the C's
    // `dma_copy_class_alloc_params` engineType=0 → 401 class, designed out).
    let host_vas = if channel.is_none() {
        match vas {
            Some(v) => v.host_vas,
            None => return Err(FwdFault::NoVas(cid)),
        }
    } else {
        None
    };
    let schedule = !proc.exec.scheduled.contains(&cid);

    // ---- ★★ The #14 ring-gate, BEFORE any host op — and it now runs **inside the
    //      constructor**. `VerbPlan::Doorbell` is `#[non_exhaustive]`, so
    //      `VerbPlan::gated_doorbell` is the only thing in the workspace (or outside it)
    //      that can produce one, and it refuses before returning a plan. There is
    //      therefore no plan-shaped object in existence for an ungated working set —
    //      the invariant is on the type, not on this function remembering to call a
    //      gate first (`ARCHITECTURE.md` invariant 5, closed 2026-07-27).
    let vas_gate = vas.zip(chan.vas_pdb).map(|(v, pdb)| VasGate(&v.table, pdb));
    let no_vas = NoVasGate;
    let gate: &dyn kayfabe_isolate::RingWorkingSet = match &vas_gate {
        Some(g) => g,
        None => &no_vas,
    };
    let verbs =
        VerbPlan::gated_doorbell(gate, working_set, host_vas, channel, chan.engine, schedule)
            .map_err(|kayfabe_isolate::UngatedVa(va)| match chan.vas_pdb {
                Some(pdb) => FwdFault::Address(AddressFault::Miss { pdb, va }),
                None => FwdFault::NoVas(cid),
            })?;

    Ok(Planned {
        plan: DoorbellPlan {
            proc: pid,
            chan: cid,
            gpu: route.gpu,
            cgpu,
            vchid: route.vchid,
            token: route.token,
            vas_pdb: chan.vas_pdb,
            channel,
            schedule,
        },
        verbs: Some(verbs),
    })
}

/// COMMIT (R5) for the ring path: re-resolve the route through the spine and the
/// channel through its `ChanId`, then adopt the materialized host handles and record
/// the submission. Refuses — releasing whatever it allocated — if the route moved,
/// the channel was torn down, or a sibling commit rebound the same channel/VAS.
///
/// # Panics
/// If `reply` is not the [`VerbReply::Doorbell`] its plan asked for.
pub fn commit_doorbell(
    spine: &Spine,
    proc: &mut Proc,
    plan: &DoorbellPlan,
    reply: Option<VerbReply>,
) -> Result<DoorbellOutcome, Refusal> {
    let Some(VerbReply::Doorbell {
        host_vas: fresh_vas,
        channel: fresh_chan,
        scheduled,
    }) = reply
    else {
        return wrong_reply("doorbell");
    };
    let orphans = || Orphans {
        unmap: Vec::new(),
        free: fresh_chan
            .map(|(h, _)| h)
            .into_iter()
            .chain(fresh_vas)
            .collect(),
    };
    // Converging staleness (someone else materialized what we were materializing)
    // re-plans; divergent staleness (the target is gone) is a loud refusal.
    let refuse = |what: Stale| {
        Err(Refusal {
            fault: FwdFault::Stale(what),
            orphans: orphans(),
            retry: matches!(what, Stale::Rebound),
        })
    };
    if proc.is_retired() || proc.id != plan.proc {
        return refuse(Stale::Proc(plan.proc));
    }
    // R5 on the ROUTE: an `apply`/refresh may have rewritten `by_vchid` in the gap.
    // The plan was made for `(gpu, vchid) → (proc, chan)`; if that is no longer the
    // routing truth, this submission belongs to a world that no longer exists.
    if spine.by_vchid.get(&(plan.gpu, plan.vchid)) != Some(&(plan.proc, plan.chan)) {
        return refuse(Stale::Route {
            gpu: plan.gpu,
            vchid: plan.vchid,
        });
    }
    let Proc {
        vases,
        channels,
        exec,
        poll,
        ..
    } = proc;
    let Some(chan) = channels.get_mut(&plan.chan) else {
        return refuse(Stale::Channel(plan.chan));
    };
    if let Some(fresh) = fresh_vas {
        let pdb = plan
            .vas_pdb
            .expect("materialization requires a declared VAS");
        let Some(vas) = vases.get_mut(&(plan.cgpu, pdb)) else {
            return refuse(Stale::Vas {
                gpu: plan.cgpu,
                pdb,
            });
        };
        if vas.host_vas.is_some() {
            return refuse(Stale::Rebound);
        }
        vas.host_vas = Some(fresh);
    }
    match fresh_chan {
        // We materialized: nobody else may have, or one of the two host channels is
        // instantly orphaned (and the guest's vChid would ring the wrong one).
        Some((hchan, htok)) => {
            if chan.host_channel.is_some() {
                return refuse(Stale::Rebound);
            }
            chan.host_channel = Some(hchan);
            chan.host_token = Some(htok);
        }
        // We reused what the plan read: it must still be what the channel holds.
        None => {
            if chan.host_channel.zip(chan.host_token) != plan.channel {
                return refuse(Stale::Rebound);
            }
        }
    }
    if scheduled {
        exec.scheduled.insert(plan.chan);
    }
    poll.last_token = Some(plan.token);
    let host_token = chan.host_token.expect("materialized above");
    Ok(DoorbellOutcome {
        proc: plan.proc,
        chan: plan.chan,
        host_token,
        scheduled_now: scheduled,
    })
}

/// ★ The **single-threaded composition** of the one gated ring path — the exec-plane
/// demux, **structurally gated** (#14, `execution_plane.md` §2.4; the C's "one exec path"
/// refactor-debt lesson): one guest doorbell write → gate → the owning proc's channel rung
/// on the owning proc's isolate.
///
/// The **split-borrow composition** of [`route_doorbell`] (pure spine read) +
/// [`exec_doorbell`] (owning-proc act) — L1 cardinal rule R4 factored in the core.
///
/// ★ **corrected 2026-07-27** (found by the whitepaper's verification pass). This doc used
/// to claim *"No ungated sibling exists; nothing else in the workspace calls
/// `RmBackend::ring_doorbell`"* and *"[`exec_doorbell`] is the ONLY function that reaches
/// `RmBackend::ring_doorbell`"*. Both are false as stated: the sole `ring_doorbell` call
/// site is inside `kayfabe_isolate::Worker::execute`, and **this function is not on the L1
/// path at all** — a real guest MMIO write goes through `kayfabe_rt::SharedDevice::doorbell`,
/// which drives plan/execute/commit itself and never calls `handle_doorbell`.
///
/// The gate is still **structural, not caller discipline**, and the argument simply moves
/// down one level: [`plan_doorbell`] is the sole constructor of `VerbPlan::Doorbell`
/// *within the production crates*, and it runs the #14 ring-gate before any host op
/// exists — so neither composition can hand `Worker::execute` an un-gated ring, and the
/// removed `ring_gated` sibling stays removed.
///
/// ★★ **And since 2026-07-27 that is a fact about the TYPES, not only the call graph.**
/// The ⚠ that used to stand here said "structural" described the call graph while
/// `VerbPlan` was a public enum with public variant fields and `Worker::execute` was
/// public — so a `VerbPlan::Doorbell` could be hand-built outside this crate and rung
/// with the gate never having run, which `tests/tests/cross_proc_lifetime.rs` did.
/// `VerbPlan::Doorbell` is now `#[non_exhaustive]` (no struct expression exists outside
/// `kayfabe-isolate` — E0639, pinned by that crate's `tests/ui/ungated_doorbell.rs`) and
/// its only constructor, [`kayfabe_isolate::VerbPlan::gated_doorbell`], **is** the gate:
/// it checks every working-set VA host-published in the ringing channel's own `Vas`
/// before a plan exists. The residual is stated at that constructor and is a different,
/// smaller one: the address plane it gates over is caller-supplied, because Rust cannot
/// express "only this crate may call this function".
pub fn handle_doorbell(
    gpu: &mut Gpu,
    target_gpu: GpuId,
    token: u64,
    working_set: &[GpuVa],
) -> Result<DoorbellOutcome, FwdFault> {
    let Gpu { spine, procs, .. } = gpu;
    let route = route_doorbell(spine, target_gpu, token)?;
    let proc = procs
        .get_mut(&route.proc)
        .ok_or(FwdFault::RetiredProc(route.proc))?;
    exec_doorbell(spine, proc, &route, working_set)
}

/// Post any composable completion batch for target `gpu_target` and raise the SWGEN0
/// edge (MG-6: per-target GSP queue). Returns the posted batch, if any. (Queue
/// *encoding* is `kayfabe-gsp`'s job once it ports.)
pub fn deliver_completions(
    gpu: &mut Gpu,
    vmm: &mut dyn Vmm,
    gpu_target: GpuId,
) -> Option<PostBatch> {
    let batch = gpu.pump_completions(gpu_target)?;
    vmm.raise_irq(COMPLETION_VECTOR).ok()?;
    Some(batch)
}

/// A proc's completion-poll RPC arrived (`MC_SERVICE_INTERRUPTS`-shaped) on target
/// `gpu_target`: re-post its un-acked completions off its OWN poll and raise the edge
/// — the #14 round-8 starvation is impossible by construction (§4.3.2), per target.
pub fn poll_completions(
    gpu: &mut Gpu,
    vmm: &mut dyn Vmm,
    gpu_target: GpuId,
    pid: ProcId,
) -> Option<PostBatch> {
    let now = vmm.now();
    let batch = gpu.completion_poll(gpu_target, pid, now)?;
    vmm.raise_irq(COMPLETION_VECTOR).ok()?;
    Some(batch)
}

// =================================================================================
// GR/CE context lifecycle — the Case-1 forward / Case-2 ack-only split
// (`execution_plane.md` §2.2 / §2.5). The core is routing-only: it forwards the
// Case-1 allocs so the HOST kernel-RM builds + self-promotes its OWN context (golden
// ctx on real silicon), and ACKs the Case-2 GSP-internal controls the guest still
// issues (their effect is already achieved host-side). Zero new identity — the GR/CE
// context IS the `(Channel, Vas)` pair the graph already derives.
// =================================================================================

/// How a control routed through the Case-1/Case-2 split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlRoute {
    /// **Case 1** — forwarded ~1:1 to the host through the owning proc's isolate
    /// (the RPC *is* the userspace op).
    Forwarded,
    /// **Case 2** — GSP-internal / ROUTE_TO_PHYSICAL with no unprivileged equivalent
    /// (`PROMOTE_CTX`, `GET_CTX_BUFFER_INFO`, …). ACKed to the guest, nothing done on
    /// the host — its effect is already achieved by the Case-1 forwarding. Replaying
    /// it on an unprivileged isolate would be a "wrong layer" error, never done.
    AckOnly,
}

/// The outcome of a Case-1 engine-object forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineObjectForwarded {
    /// The engine kind the object made this channel's context (routing tag).
    pub engine: EngineKind,
    /// The host engine-object handle the forward returned.
    pub host_object: HostHandle,
    /// True if this forward materialized the host channel first (idempotent re-sends
    /// do not re-materialize).
    pub materialized_channel: bool,
    /// True if this forward was a **replay** resolved from the channel's
    /// idempotency table ([`Channel::host_engine_objects`]) — no host alloc was
    /// issued; `host_object` is the ORIGINAL host object (§2.2: re-sends are
    /// idempotent, the same discipline as the graph's alloc/DUP replay).
    pub reused: bool,
}

/// A routed Case-1 engine-object alloc (the ROUTE half — same split as
/// [`DoorbellRoute`]): the arch resolved the class to an [`EngineKind`] and
/// `by_vchid` resolved the owning `(Proc, Channel)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineObjectRoute {
    /// The owning proc.
    pub proc: ProcId,
    /// The channel the object allocs on.
    pub chan: ChanId,
    /// The target GPU the alloc addressed.
    pub gpu: GpuId,
    /// The channel's vChid (per-GPU), for act-phase fault naming.
    pub vchid: VChid,
    /// The engine kind the arch mapped `class` to.
    pub engine: EngineKind,
}

/// ROUTE: resolve a Case-1 engine-object alloc to its owning `(Proc, Channel)` —
/// a pure spine read (`Arch::engine_of_object` + `by_vchid`). A class the arch
/// does not recognize as an engine object is a loud `NotAnEngine` (MISS=FAULT).
pub fn route_engine_object(
    spine: &Spine,
    target_gpu: GpuId,
    vchid: VChid,
    class: ClassId,
) -> Result<EngineObjectRoute, FwdFault> {
    let engine = spine
        .arch()
        .engine_of_object(class)
        .ok_or(FwdFault::NotAnEngine(class))?;
    let (pid, cid) = *spine
        .by_vchid
        .get(&(target_gpu, vchid))
        .ok_or_else(|| vchid_miss(spine, target_gpu, vchid))?;
    Ok(EngineObjectRoute {
        proc: pid,
        chan: cid,
        gpu: target_gpu,
        vchid,
        engine,
    })
}

/// ACT: forward the routed engine-object alloc on **its owning proc only**
/// (`&mut Proc`): lazily materialize the host channel (same per-proc discipline as
/// the doorbell act), then alloc the engine object via the proc's own isolate.
///
/// The single-threaded composition of [`plan_engine_object`] / `Worker::execute` /
/// [`commit_engine_object`] — same R1 shape as [`exec_doorbell`].
///
/// **Idempotent under replay** (§2.2; the protocol is order-/repeat-independent): a
/// re-sent alloc for a class already forwarded on this channel resolves from
/// [`Channel::host_engine_objects`] and returns the ORIGINAL host object — the host
/// never sees a duplicate engine-object alloc (the guest-retry hazard the graph
/// already covers for alloc/DUP, extended to the host-forward plane).
pub fn exec_engine_object(
    spine: &Spine,
    proc: &mut Proc,
    route: &EngineObjectRoute,
    class: ClassId,
    params: &[u8],
) -> Result<EngineObjectForwarded, FwdFault> {
    let planned = plan_engine_object(proc, route, class, params)?;
    let gpu = planned.plan.cgpu;
    round_trip(proc, gpu, planned.verbs, |proc, reply| {
        commit_engine_object(spine, proc, &planned.plan, reply)
    })
}

/// The ID-shaped hints [`commit_engine_object`] re-validates against (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineObjectPlan {
    /// Owning proc.
    pub proc: ProcId,
    /// The channel the object allocs on.
    pub chan: ChanId,
    /// The GPU the alloc addressed (routing key with `vchid`).
    pub gpu: GpuId,
    /// The channel's own target GPU (isolate key).
    pub cgpu: GpuId,
    /// The channel's vChid.
    pub vchid: VChid,
    /// The engine kind the arch mapped `class` to.
    pub engine: EngineKind,
    /// The engine-object class.
    pub class: ClassId,
    /// The channel's declared VAS, if any.
    pub vas_pdb: Option<Pdb>,
    /// The channel's host handles as observed at plan time.
    pub channel: Option<ChannelHandles>,
    /// Set when the alloc resolved from the channel's idempotency table — no host
    /// work at all, so no worker is checked out (`verbs = None`).
    pub replay: Option<HostHandle>,
}

/// PLAN (R1) for the Case-1 engine-object forward.
///
/// **Idempotent under replay** (§2.2): a re-sent alloc for a class already forwarded
/// on this channel resolves here, from core state, and emits **no verbs at all** —
/// the host never sees a duplicate, and the replay never touches the worker pool.
pub fn plan_engine_object(
    proc: &Proc,
    route: &EngineObjectRoute,
    class: ClassId,
    params: &[u8],
) -> Result<Planned<EngineObjectPlan>, FwdFault> {
    let pid = route.proc;
    let cid = route.chan;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
    }
    let chan = proc.channels.get(&cid).ok_or(FwdFault::UnknownVchid {
        gpu: route.gpu,
        vchid: route.vchid,
    })?;
    let cgpu = chan.gpu;
    if !proc.isolates.contains_key(&cgpu) {
        return Err(FwdFault::NoTarget {
            proc: pid,
            gpu: cgpu,
        });
    }
    let channel = chan.host_channel.zip(chan.host_token);
    // A replay is only representable once the channel materialized (the idempotency
    // table is populated by a forward, which requires a host channel).
    let replay = channel
        .is_some()
        .then(|| chan.host_engine_objects.get(&class).copied())
        .flatten();
    let host_vas = if channel.is_none() {
        let pdb = chan.vas_pdb.ok_or(FwdFault::NoVas(cid))?;
        proc.vases
            .get(&(cgpu, pdb))
            .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?
            .host_vas
    } else {
        None
    };
    let plan = EngineObjectPlan {
        proc: pid,
        chan: cid,
        gpu: route.gpu,
        cgpu,
        vchid: route.vchid,
        engine: route.engine,
        class,
        vas_pdb: chan.vas_pdb,
        channel,
        replay,
    };
    let verbs = if replay.is_some() {
        None
    } else {
        Some(VerbPlan::EngineObject {
            host_vas,
            channel,
            engine: chan.engine,
            class,
            params: params.to_vec(),
        })
    };
    Ok(Planned { plan, verbs })
}

/// COMMIT (R5) for the Case-1 forward: same route/channel re-resolution as the
/// doorbell, then adopt the host engine object into the channel's idempotency table.
///
/// # Panics
/// If `reply` is not the [`VerbReply::EngineObject`] its plan asked for.
pub fn commit_engine_object(
    spine: &Spine,
    proc: &mut Proc,
    plan: &EngineObjectPlan,
    reply: Option<VerbReply>,
) -> Result<EngineObjectForwarded, Refusal> {
    let (fresh_vas, fresh_chan, object) = match (plan.replay, reply) {
        // Replay: nothing ran, nothing to adopt — but the target must still exist.
        (Some(original), None) => (None, None, Some(original)),
        (
            None,
            Some(VerbReply::EngineObject {
                host_vas,
                channel,
                object,
            }),
        ) => (host_vas, channel, Some(object)),
        _ => return wrong_reply("engine-object"),
    };
    let object = object.expect("both arms produce a host object");
    let orphans = || Orphans {
        unmap: Vec::new(),
        free: (plan.replay.is_none())
            .then_some(object)
            .into_iter()
            .chain(fresh_chan.map(|(h, _)| h))
            .chain(fresh_vas)
            .collect(),
    };
    // Converging staleness (someone else materialized what we were materializing)
    // re-plans; divergent staleness (the target is gone) is a loud refusal.
    let refuse = |what: Stale| {
        Err(Refusal {
            fault: FwdFault::Stale(what),
            orphans: orphans(),
            retry: matches!(what, Stale::Rebound),
        })
    };
    if proc.is_retired() || proc.id != plan.proc {
        return refuse(Stale::Proc(plan.proc));
    }
    if spine.by_vchid.get(&(plan.gpu, plan.vchid)) != Some(&(plan.proc, plan.chan)) {
        return refuse(Stale::Route {
            gpu: plan.gpu,
            vchid: plan.vchid,
        });
    }
    let Proc {
        vases, channels, ..
    } = proc;
    let Some(chan) = channels.get_mut(&plan.chan) else {
        return refuse(Stale::Channel(plan.chan));
    };
    if let Some(fresh) = fresh_vas {
        let pdb = plan
            .vas_pdb
            .expect("materialization requires a declared VAS");
        let Some(vas) = vases.get_mut(&(plan.cgpu, pdb)) else {
            return refuse(Stale::Vas {
                gpu: plan.cgpu,
                pdb,
            });
        };
        if vas.host_vas.is_some() {
            return refuse(Stale::Rebound);
        }
        vas.host_vas = Some(fresh);
    }
    match fresh_chan {
        Some((hchan, htok)) => {
            if chan.host_channel.is_some() {
                return refuse(Stale::Rebound);
            }
            chan.host_channel = Some(hchan);
            chan.host_token = Some(htok);
        }
        None => {
            if chan.host_channel.zip(chan.host_token) != plan.channel {
                return refuse(Stale::Rebound);
            }
        }
    }
    if plan.replay.is_none() {
        // A sibling thread may have forwarded the SAME class in the gap; the table is
        // the idempotency authority, so the loser refuses and frees its duplicate
        // rather than overwriting (which would orphan the winner's object silently).
        if chan.host_engine_objects.contains_key(&plan.class) {
            return refuse(Stale::Rebound);
        }
        chan.host_engine_objects.insert(plan.class, object);
    }
    Ok(EngineObjectForwarded {
        engine: plan.engine,
        host_object: object,
        materialized_channel: fresh_chan.is_some(),
        reused: plan.replay.is_some(),
    })
}

/// **Case 1**: forward an engine-object alloc (compute / graphics / CE / NVENC) on the
/// channel identified by `vchid`, so the host kernel-RM builds and self-promotes its
/// OWN context. The **split-borrow composition** of [`route_engine_object`] +
/// [`exec_engine_object`] (same route/act discipline as the doorbell).
///
/// `class` is the guest's engine-object class; `params` is the ABI-lowered alloc
/// blob (Axis A). MISS=FAULT throughout.
pub fn forward_engine_object(
    gpu: &mut Gpu,
    target_gpu: GpuId,
    vchid: VChid,
    class: ClassId,
    params: &[u8],
) -> Result<EngineObjectForwarded, FwdFault> {
    let Gpu { spine, procs, .. } = gpu;
    let route = route_engine_object(spine, target_gpu, vchid, class)?;
    let proc = procs
        .get_mut(&route.proc)
        .ok_or(FwdFault::RetiredProc(route.proc))?;
    exec_engine_object(spine, proc, &route, class, params)
}

/// ROUTE: classify a `GSP_RM_CONTROL` through the Case-1/Case-2 split — a pure
/// spine read (`Arch::is_case2_control`), no proc touched.
#[must_use]
pub fn classify_control(spine: &Spine, cmd: ControlCmd) -> ControlRoute {
    if spine.arch().is_case2_control(cmd) {
        // Case 2: ack-only. The host already did it (Case-1). Do NOT replay — an
        // unprivileged replay returns InsufficientPermissions ("wrong layer").
        ControlRoute::AckOnly
    } else {
        ControlRoute::Forwarded
    }
}

/// ACT: forward a Case-1 control on **its owning proc only** (`&mut Proc`), on the
/// op's TARGET GPU's isolate (MG-5): the control object `obj` is a handle in that
/// isolate's namespace; routing it elsewhere is unrepresentable.
///
/// The single-threaded composition of [`plan_control`] / `Worker::execute` /
/// [`commit_control`] — same R1 shape as [`exec_doorbell`].
pub fn forward_control(
    proc: &mut Proc,
    target_gpu: GpuId,
    obj: HostHandle,
    cmd: ControlCmd,
    payload: &mut [u8],
) -> Result<(), FwdFault> {
    let planned = plan_control(proc, target_gpu, obj, cmd, payload)?;
    round_trip(proc, target_gpu, planned.verbs, |proc, reply| {
        commit_control(proc, &planned.plan, reply, payload)
    })
}

/// The ID-shaped hints [`commit_control`] re-validates against (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPlan {
    /// Owning proc.
    pub proc: ProcId,
    /// The op's TARGET GPU (MG-5: `obj` is a handle in THAT isolate's namespace).
    pub gpu: GpuId,
    /// The control object.
    pub obj: HostHandle,
    /// The command.
    pub cmd: ControlCmd,
}

/// PLAN (R1) for a Case-1 control forward. The payload is copied into the plan by
/// value: a plan outlives the lock scope that made it, so it may not borrow.
pub fn plan_control(
    proc: &Proc,
    target_gpu: GpuId,
    obj: HostHandle,
    cmd: ControlCmd,
    payload: &[u8],
) -> Result<Planned<ControlPlan>, FwdFault> {
    let pid = proc.id;
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(pid));
    }
    if !proc.isolates.contains_key(&target_gpu) {
        return Err(FwdFault::NoTarget {
            proc: pid,
            gpu: target_gpu,
        });
    }
    Ok(Planned {
        plan: ControlPlan {
            proc: pid,
            gpu: target_gpu,
            obj,
            cmd,
        },
        verbs: Some(VerbPlan::Control {
            obj,
            cmd,
            payload: payload.to_vec(),
        }),
    })
}

/// COMMIT (R5) for a control forward: re-validate that the proc and its target
/// isolate still exist, then write the host's reply back into the guest's buffer.
///
/// **Honest note on this site's staleness shape.** The control's host effect has
/// already happened by the time the commit runs; the only thing the commit *owns* is
/// the write-back. So a refusal here means "the answer has nowhere to go", not "the
/// op was undone" — and there is no orphan to release, because the object the
/// control ran on was the guest's, not something this op allocated. That is a real
/// asymmetry with the alloc-shaped sites, stated rather than papered over.
///
/// # Panics
/// If `reply` is not the [`VerbReply::Control`] its plan asked for.
pub fn commit_control(
    proc: &mut Proc,
    plan: &ControlPlan,
    reply: Option<VerbReply>,
    payload: &mut [u8],
) -> Result<(), Refusal> {
    let Some(VerbReply::Control { payload: out }) = reply else {
        return wrong_reply("control");
    };
    if proc.is_retired() || proc.id != plan.proc {
        return Err(Refusal::bare(FwdFault::Stale(Stale::Proc(plan.proc))));
    }
    if !proc.isolates.contains_key(&plan.gpu) {
        return Err(Refusal::bare(FwdFault::Stale(Stale::Target {
            proc: plan.proc,
            gpu: plan.gpu,
        })));
    }
    let n = payload.len().min(out.len());
    payload[..n].copy_from_slice(&out[..n]);
    Ok(())
}

/// Route a `GSP_RM_CONTROL` through the Case-1/Case-2 split. A **Case-2** control is
/// ACKed and NOT forwarded (its host effect is already achieved); a **Case-1** control
/// is forwarded to the host on `obj` through the owning proc's isolate. The
/// **split-borrow composition** of [`classify_control`] + [`forward_control`].
///
/// This is the anti-bolt-on payoff in code: adding an engine adds *rows* to the arch's
/// Case-2 set and its class table — never a new host verb, never a new routing path.
pub fn route_control(
    gpu: &mut Gpu,
    target_gpu: GpuId,
    pid: ProcId,
    obj: HostHandle,
    cmd: ControlCmd,
    payload: &mut [u8],
) -> Result<ControlRoute, FwdFault> {
    if let ControlRoute::AckOnly = classify_control(&gpu.spine, cmd) {
        return Ok(ControlRoute::AckOnly);
    }
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    forward_control(proc, target_gpu, obj, cmd, payload)?;
    Ok(ControlRoute::Forwarded)
}

/// The guest is waiting on the GR golden-capture completion (a GSP-event the host's
/// in-kernel FECS capture satisfies). Route it to the **system** proc — it is
/// kernel-internal and content-irrelevant (the guest only needs the *completion* its
/// 4-second poll waits on). Returns the observed os-event ref for assertions.
///
/// Typed to the system proc by construction (lesson L5 / the #12 finishPayload rule):
/// forging a completion for a userspace proc is unrepresentable here.
///
/// **Deliberately NOT route/act-split** (the one `&mut Gpu` per-proc-shaped entry
/// left): a `&mut Proc` form would let a caller hand it a *user* proc, dissolving
/// the structural L5 guarantee. It targets `Gpu::system` by name, is a rare
/// bring-up event (once per device), and L1 runs it under the device write lock.
pub fn signal_golden_capture(gpu: &mut Gpu, event: OsEventRef) -> Result<OsEventRef, FwdFault> {
    gpu.system.completion.observe(event)?;
    Ok(event)
}

// =================================================================================
// The ONE pushbuffer parser (`execution_plane.md` §2.3) — the address-table
// populator + sema/fence extractor. It decodes JUST the four fact kinds; everything
// else is opaque and passes through (the anti-emulation boundary, trap-min #6). The
// decode LOGIC is core; the method ENCODINGS come from `Arch::pushbuffer()`.
//
// Two co-equal address-table populate sources meet here (address_table.md, L3): the
// bind-time RPC bindings (batch 1, `Gpu::sync_rpc_mappings`) and the observed CE
// PT-writes captured below. Both land in the same per-`Vas` table.
// =================================================================================

/// Upper bound on a single GPFIFO range's method bytes the parser will read. A
/// hostile GPFIFO entry can declare any length; this caps it to a bounded read so an
/// attacker-controlled length is never an arbitrary allocation (boundary-1). Real
/// pushbuffer segments are far smaller; a range hitting this cap is simply truncated
/// (the surplus decodes to nothing actionable, MISS=FAULT at use).
const MAX_PUSH_RANGE_BYTES: usize = 1 << 20;

/// Upper bound on the TOTAL method bytes one `parse_pushbuffer` call will read across
/// ALL of a ring's GPFIFO ranges. `MAX_PUSH_RANGE_BYTES` bounds any single range, but a
/// hostile ring can declare *many* maxed-out ranges; this caps their sum so the decoded
/// method vector cannot grow without bound either (boundary-1). Ranges past the budget
/// are skipped (their content decodes to nothing actionable — MISS=FAULT at use).
const MAX_PUSH_TOTAL_BYTES: usize = 8 << 20;

// ---------------------------------------------------------------------------------
// ★★★ TWO DECISIONS, NOT ONE (`eight_blockers_resolved.md` §11.5 / §12).
//
// The C makes two different decisions about one `LAUNCH_DMA`, on two different
// predicates, and stage B (`379f712`) folded them together — it got the CAPTURE
// predicate right and answered EXECUTE by accident, routing everything non-phys to
// "forward it, let hardware run it".
//
// | decision | the C's predicate | site |
// |---|---|---|
// | EXECUTE — host CE vs our own copy | `m2cexec && !mscrub && !remap && !src_phys && !dst_phys && is_user_ce(chan_client)` | `C: nvkvm_gpu_emul.c:6310` |
// | CAPTURE — is this a page-table write? | the fb-write hook, on the **resolved physical** destination | `C: :6353`, `:6437` |
//
// They are separate here because they read different inputs and can disagree on the
// same command. Read the C's execute row carefully: `is_user_ce` means **every
// guest-kernel CE copy is CPU-emulated there**, including the framebuffer-alias
// page-table write — which is *virtual*-destination and would pass any purely
// operand-carried test — and so are all scrubs and fills.
//
// [`ce_executor_c`] is that predicate, ported. [`classify_ce`] is the capture one.
// ---------------------------------------------------------------------------------

/// Who submitted a copy-engine command — the C's `is_user_ce(chan_client)`
/// (`C: nvkvm_gpu_emul.c:2493` — paraphrased rather than quoted, because the C's own
/// wording names a host user-mode library and the hexagonal gate rightly refuses that
/// vocabulary here: *is this client one of the user-mode driver's CE-copy clients, the
/// user-observable data path? UVM/init clients are not*).
///
/// The port keys it on the **proc**, not on a client list. `kayfabe_core::Gpu::system`
/// already *is* the guest-kernel component — "kernel RM / scrubber / CeUtils traffic",
/// every declared kernel client joined by rule (§12.27) — so the fact the C had to
/// accumulate into `m2_user_ce_clients[]` at runtime is one the projection declares.
/// That is a strengthening, not a departure: the C's list was populated by observation
/// and a client it had not yet seen read as *not* user-CE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelOrigin {
    /// A user process's channel — the user-observable CE data path.
    User,
    /// The guest kernel's own channel — CeUtils, the scrubber, UVM.
    GuestKernel,
}

impl ChannelOrigin {
    /// The origin of a channel owned by `pid`.
    #[must_use]
    pub fn of(pid: ProcId) -> Self {
        if pid == Gpu::SYSTEM_PROC {
            ChannelOrigin::GuestKernel
        } else {
            ChannelOrigin::User
        }
    }
}

/// ★ **The C's execute predicate, ported literally** (`C: nvkvm_gpu_emul.c:6310`):
///
/// ```text
/// bool host_ce = s->m2cexec && !mscrub && !remap && !src_phys && !dst_phys &&
///                nvkvm_m2_is_user_ce(s, s->chan_client);
/// ```
///
/// Every conjunct survives except `m2cexec`, which is a **bench debug switch** (the
/// C's "run the copy on the CPU so I can read it" flag), not a design axis — this port
/// has no mode in which execution forwarding is off, so a constant `true` conjunct is
/// not modelled. Named here rather than silently dropped.
///
/// ★★ This is the **baseline**, not the shipped policy. §12's ruling replaces the
/// `is_user_ce` conjunct — a fact about *who submitted the work* — with representability,
/// a fact about the **address**. `ce_c_vs_representability` (integration tests) pins
/// every row where the two answers differ, so each departure is a value a test reads
/// rather than a paragraph.
#[must_use]
pub fn ce_executor_c(
    work: kayfabe_arch::CeWork,
    origin: ChannelOrigin,
    src_is_virtual: bool,
    dst_is_virtual: bool,
) -> CeExecutor {
    let plain_copy = matches!(work, kayfabe_arch::CeWork::Copy);
    if plain_copy && src_is_virtual && dst_is_virtual && origin == ChannelOrigin::User {
        CeExecutor::HostCe
    } else {
        CeExecutor::Ours
    }
}

/// ★★★ **What a copy-engine command's operands CARRY** — the discriminator the whole
/// data plane turns on (`mode2_dataplane_architecture.md`, "The architecture to build").
///
/// Not *who submitted the work* and not *what form the destination took*, but what the
/// operands mean:
///
/// - **VA-operand** — copies and kernels. The operands are GPU virtual addresses the host
///   MMU resolves once the address space is resident. Nothing for the **address plane** to
///   extract: no PTE values are in flight, so there is no capture. There is no software
///   *shadow* of these, and there must not be: the C's shadow-plus-forged-completion was
///   byte-exact and never touched a GPU, which is precisely why nothing noticed the buffer
///   had no host mapping until hardware was finally asked to resolve it.
/// - **Phys-operand** — page-table writes and scrubs. The payload is guest-physical PTE
///   values, which **cannot be handed to hardware**. Capture, so the address plane can
///   decode what the page now describes at the guest's own commit point.
///
/// ★★ **This enum answers CAPTURE ONLY.** It used to say "forward it, let hardware
/// execute it" on the `VaOperand` arm — which is the *execute* decision, a different
/// predicate over different inputs, answered here by accident
/// (`eight_blockers_resolved.md` §11.5). Who runs the copy is [`CeExecutor`]'s question.
/// A command can perfectly well be "not a page-table write" **and** "not hardware's to
/// run": every guest-kernel data copy in the C is exactly that.
///
/// ★★ The classification runs on the **resolved physical destination**, never on
/// `dst_is_virtual`. This port used to gate on `!dst_is_virtual`, which excludes exactly
/// the case #13 is about: the guest kernel's copy-engine utility identity-maps the whole
/// framebuffer into its own address space at 512 MiB pages and issues its page-table
/// writes as **VIRTUAL-destination** copies (`C: nvkvm_gpu_emul.c:4936-4952`). The C hooks
/// on the resolved physical regardless of form (`C: :6353`, `:6437`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeOperands {
    /// Operands the host MMU resolves. Forward; do not intercept.
    VaOperand {
        /// The resolved physical destination (diagnostics only — nothing acts on it).
        phys: u64,
    },
    /// Operands carrying guest-physical PTE values. Intercept.
    PhysOperand {
        /// The 4 KiB page-table page this write landed on.
        page: u64,
        /// The aperture the destination resolved through.
        aperture: Aperture,
        /// ★ The proc whose `Vas` OWNS that page — **not necessarily the proc whose
        /// channel issued the write.** The guest kernel writes user processes' page
        /// tables; that asymmetry is the reason the ownership index is device-global.
        owner: ProcId,
        /// The owning `Vas`'s PDB.
        owner_pdb: Pdb,
    },
}

/// One latched page-table write (`#13`, the C's `m2_cpt` dirty entry).
///
/// **Latched, not decoded.** The hot path records which page was touched and nothing else:
/// a big scrub can hit one page-directory page thousands of times, and decoding its
/// subtree per span **livelocked on the bench** — *"the first per-write attempt hung with
/// State=R busy-poll and no CTX OK"* (`C: :8686-8690`). The decode happens at the guest's
/// own commit point, the semaphore release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtWrite {
    /// The target GPU (a physical page address is per-GPU).
    pub gpu: GpuId,
    /// The 4 KiB page-table page base.
    pub page: u64,
    /// The aperture the write resolved through.
    pub aperture: Aperture,
    /// The proc whose `Vas` owns the page (see [`CeOperands::PhysOperand`]).
    pub owner: ProcId,
    /// The owning `Vas`'s PDB.
    pub owner_pdb: Pdb,
    /// Bytes the copy declared — how much of the page it may have changed.
    pub bytes: u64,
}

/// What one pushbuffer parse observed (for assertions + the caller's next steps).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushbufferOutcome {
    /// ★ Phys-operand commands: page-table pages this pushbuffer's copies wrote, each
    /// carrying the `Vas` that owns it. Latched here; applied by the caller.
    pub pt_writes: Vec<PtWrite>,
    /// ★ VA-operand commands seen and **not captured** — the address plane extracted
    /// nothing from them. Counted so "we did not intercept this" is an assertable fact
    /// rather than an absence, and so a test can tell "classified as data" apart from
    /// "never decoded at all".
    ///
    /// ★★ This is the CAPTURE tally and says **nothing** about who executes the copy —
    /// see [`PushbufferOutcome::host_ce`] / [`PushbufferOutcome::ours`], which partition
    /// the same commands on the other predicate. `data_copies + pt_writes.len()` and
    /// `host_ce + ours` count the same `LAUNCH_DMA`s two different ways.
    pub data_copies: usize,
    /// ★★★ EXECUTE, the **C BASELINE**, arm one: commands [`ce_executor_c`] leaves to
    /// real hardware.
    ///
    /// This is what the C would have done, kept beside what we actually do
    /// ([`PushbufferOutcome::ce_spans`]) so every departure §12's ruling introduces is a
    /// value a test reads rather than a paragraph. Nothing acts on it.
    pub c_execute_host_ce: usize,
    /// ★★★ EXECUTE, the **C BASELINE**, arm two: commands the C runs itself. There,
    /// every guest-kernel CE copy, every scrub and every fill lands here
    /// (`C: nvkvm_gpu_emul.c:6310`).
    pub c_execute_ours: usize,
    /// ★★★ **The partition we act on** (§12): every copy-engine request in this
    /// pushbuffer, split into sub-copies by the representability of its operands, in
    /// submission order. The caller turns these into [`VerbPlan::CeSplit`] and runs them
    /// on a worker — with no lock held (R1).
    pub ce_spans: Vec<CeSpan>,
    /// Semaphore releases observed → each `observe`d on the owning proc's queue.
    pub sem_releases: Vec<(GpuVa, u64)>,
    /// TLB invalidates seen (pdb, membar). A membar is honored as a hard barrier
    /// (the parser records it; a real transport blocks advance until refresh).
    pub invalidates: Vec<(Pdb, bool)>,
    /// Count of opaque methods passed through (acted on by no core state).
    pub opaque: usize,
}

/// ★★★ **THE ONE PLACE the core touches guest-physical memory.**
///
/// Every `Vmm::gpa_read` / `Vmm::gpa_write` in the pure crates goes through here, and a
/// CI gate keeps it that way (`.github/workflows/ci.yml`, the GPA-accessor gate). The
/// point is not tidiness — it is that the *classification* of a refusal must exist in
/// exactly one place, for the same reason `RmGraph::undeclared_namespace` does:
///
/// - [`VmmError::NonRamGpa`] ⇒ [`FwdFault::NonRamGpa`] — the guest aimed a descriptor
///   at a device register window. **Named, never folded**, because this call runs under
///   the device read lock and a backend that served it would take the VMM's global lock
///   beneath one of our ranked locks (`l1_os_shell.md` §6.3 / §10.1 item 6).
/// - [`VmmError::BadGpa`] ⇒ [`FwdFault::GpaRead`] carrying **the port's address, not
///   ours**. ★ **Fixed 2026-07-27 (`l1_os_shell.md` §14.8 F6).** This arm used to fall
///   into the catch-all below, which substitutes the *requested* address — so a
///   straddling descriptor that ran off the end of a window reported where it *started*
///   while its near neighbour ([`VmmError::NonRamGpa`]) reported the **boundary byte**
///   [`kayfabe_vmm::GuestRamMap::resolve`] actually named. Two refusals whose payloads
///   mean different things is the shape `testing_doctrine.md` §2 rule 3 forbids, and it
///   was invisible because §12.43's straddle test uses a RAM→DEVICE range — i.e. the arm
///   that already kept it.
/// - anything else ⇒ [`FwdFault::GpaRead`] naming the address the *request* started at.
///   Nothing reaches this arm today ([`VmmError`]'s other variants are raised by the
///   mapping plane, not by `gpa_read`); it exists so a future variant degrades to a loud
///   refusal rather than to a compile error someone silences.
///
/// A `map_err(|_| …)` at the call site — which is what this replaced — discards the
/// variant, and with it the only evidence the refusal was the security-relevant one.
fn guest_read(vmm: &mut dyn Vmm, gpa: u64, buf: &mut [u8]) -> Result<(), FwdFault> {
    vmm.gpa_read(gpa, buf).map_err(|e| match e {
        VmmError::NonRamGpa { gpa } => FwdFault::NonRamGpa { gpa },
        VmmError::BadGpa { gpa } => FwdFault::GpaRead { gpa },
        _ => FwdFault::GpaRead { gpa },
    })
}

/// Decode a byte range of method words into `(header, args)` pairs, arch-driven.
/// Total on any input (a hostile/truncated range yields fewer methods, never a
/// panic or an unbounded read).
fn decode_methods(arch: &dyn kayfabe_arch::Arch, bytes: &[u8]) -> Vec<(u32, Vec<u32>)> {
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|w| u32::from_le_bytes(w.try_into().expect("4 bytes")))
        .collect();
    let pb = arch.pushbuffer();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let header = words[i];
        let nargs = pb.method_len(header);
        let start = i + 1;
        let end = start.saturating_add(nargs).min(words.len());
        out.push((header, words[start..end].to_vec()));
        // Always advance past at least the header, so a bogus count cannot stall.
        i = end.max(i + 1);
    }
    out
}

/// ROUTE/read phase of the pushbuffer parse: walk `ring`'s GPFIFO entries (arch
/// format, a pure spine read) and read each range's method words from guest memory
/// via `vmm`, bounded (boundary-1: per-range and total caps). Touches no proc — in
/// L1 this runs under the device *read* lock, before the owning proc's lock.
pub fn read_pushbuffer(
    spine: &Spine,
    vmm: &mut dyn Vmm,
    ring: &[u8],
) -> Result<Vec<(u32, Vec<u32>)>, FwdFault> {
    // Walk the GPFIFO entries (arch format), reading each range's method bytes. A
    // hostile GPFIFO entry can name any length; cap the per-range read so a bogus
    // length is a bounded read, never an arbitrary allocation (boundary-1 posture).
    let ranges = spine.arch().pushbuffer().gpfifo_entries(ring);
    let mut methods = Vec::new();
    let mut total = 0usize;
    for r in ranges {
        if total >= MAX_PUSH_TOTAL_BYTES {
            break; // Total-work budget spent — a hostile many-range ring stops here.
        }
        let len = (r.len as usize)
            .min(MAX_PUSH_RANGE_BYTES)
            .min(MAX_PUSH_TOTAL_BYTES - total);
        let mut buf = vec![0u8; len];
        guest_read(vmm, r.gpa, &mut buf)?;
        methods.extend(decode_methods(spine.arch(), &buf));
        total += len;
    }
    Ok(methods)
}

/// ★★★ **The operand split**, for one decoded copy-engine command.
///
/// Two steps, in this order and only this order:
///
/// 1. **RESOLVE** the destination to a physical address. A virtual destination is walked
///    through the *issuing channel's own* `Vas` — which is what makes the framebuffer-alias
///    case work: the kernel's copy-engine utility maps FB into its own address space, so
///    its 512 MiB alias resolves in ITS table to the physical page it is writing. MISS =
///    FAULT, no fallback walk, no cross-VAS guess (that fallback is the C's #12 collision
///    class, `eight_blockers_resolved.md` §2).
/// 2. **CLASSIFY** on that resolved physical, via the device-global ownership index.
///
/// Doing it in the other order — classify on the operand *form*, then resolve — is the
/// inverted gate this replaces.
fn classify_ce(
    spine: &Spine,
    proc: &Proc,
    cid: ChanId,
    chan_pdb: Option<Pdb>,
    cgpu: GpuId,
    dst: GpuVa,
    dst_is_virtual: bool,
) -> Result<CeOperands, FwdFault> {
    let (phys, aperture) = if dst_is_virtual {
        // The destination is an address in the issuing channel's VAS. Walk it there.
        let pdb = chan_pdb.ok_or(FwdFault::NoVas(cid))?;
        let vas = proc
            .vases
            .get(&(cgpu, pdb))
            .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?;
        match vas.table.resolve(pdb, dst) {
            Ok((b, off)) => (b.phys.wrapping_add(off), b.aperture),
            // ★ A virtual destination we cannot resolve is NOT a fault here, and that is
            // deliberate. The overwhelming majority of virtual-destination copies are
            // ordinary data writes into a user address space we never had to model — the
            // forward path, whose whole premise is that we do not track those addresses.
            // Faulting would turn "we are not intercepting this" into "the guest did
            // something wrong". What must never happen is *guessing* it into a page-table
            // write, and an unresolved destination cannot be classified as one: it has no
            // physical address to look up.
            Err(_) => return Ok(CeOperands::VaOperand { phys: 0 }),
        }
    } else {
        // A physical destination names the framebuffer directly.
        (dst.0, Aperture::Vidmem)
    };
    match spine.pt_page_owner(cgpu, phys) {
        Some((owner, owner_pdb)) => Ok(CeOperands::PhysOperand {
            page: phys & !0xfffu64,
            aperture,
            owner,
            owner_pdb,
        }),
        None => Ok(CeOperands::VaOperand { phys }),
    }
}

// =================================================================================
// ★★★ THE REPRESENTABILITY SPLIT (`eight_blockers_resolved.md` §12) — #102 stage C2.
//
// The owner's ruling, restated as the four things it decides:
//
//   1. We perform a copy ONLY where it is UNREPRESENTABLE by a real copy engine — an
//      operand landing in *fabricated* space that no real engine can be pointed at.
//   2. Everything representable goes to real hardware. That is normally FASTER than a
//      CPU memcpy, not merely more faithful.
//   3. A single request may SPLIT. Its representable sub-ranges are issued to real CE;
//      only the unrepresentable remainder is ours. ⇒ the operand ranges must be
//      PARTITIONED, not classified whole.
//   4. The executor is the ISOLATE in both cases (`VerbPlan::CeSplit`).
//
// ★★★ THE CRITERION IS A PROPERTY OF THE ADDRESS, NOT OF OUR KNOWLEDGE ABOUT ITS ROLE.
// That is what dissolves the orphan-leaf problem (§12.1(i)): a fresh page-table leaf in
// fabricated space is performed-by-us — and therefore its content is in our hands —
// BEFORE any PDE points at it, so it does not need to be recognised as a page table
// first. There is deliberately NO "is this a page table?" test here; re-introducing one
// is precisely the bug the ruling removes.
// =================================================================================

/// ★★★ **Where a copy-engine operand's address LIVES** — the §12 criterion, and it is a
/// property of the address alone.
///
/// The C's own version of this is `nvkvm_dp_classify_fb` (`C: nvkvm_gpu_emul.c:1013`),
/// which answers `1 = fbback`, `2 = gpga with a real object`, `0 = still a fake
/// fb_page`. It is the same question — *does real device memory exist behind this
/// address?* — asked over guest-framebuffer-physical addresses instead of over the
/// address table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Representability {
    /// ★ **Representable.** The range is host-published in the owning `Vas` — real host
    /// memory, mapped into that `Vas`'s own host VAS **at the identical address**
    /// (address identity, `#102` stage A). A real engine can be pointed at the guest's
    /// own number, because that is the number the host MMU walks for.
    HostBacked,
    /// ★★★ **Fabricated.** The range is *declared* in the address table and nothing
    /// host-side exists behind it: it lives in the emulated framebuffer, which is memory
    /// we invented. A real engine pointed here resolves nothing — `Xid 31 FAULT_PDE`.
    /// This is where the guest's page tables live, and it is where a
    /// declared-but-unpublished range lives, and the split does not distinguish them
    /// *because it must not* (§12.1(i)).
    Fabricated,
    /// **A physical operand** — the command named a guest-framebuffer-physical address,
    /// so no GPU VA denotes it at all. Unrepresentable by construction rather than by
    /// lookup: *"a CE physical copy bypasses the MMU, so the page-table walk can NEVER
    /// discover this dst (no PTE)"* (`C: :6244`). The C agrees on the answer for a
    /// different-looking reason — `dst_phys` is a negated conjunct of its execute
    /// predicate (`C: :6310`), so a physical operand is never the host engine's there
    /// either.
    PhysicalOperand,
    /// **Not tracked.** No binding covers the range. Forwarded — never guessed into a
    /// capture and never claimed as ours (MISS = FAULT is about *resolving*, and we are
    /// not resolving it; the overwhelming majority of these are ordinary data in a user
    /// address space we never had to model).
    ///
    /// ★ The safety net for this arm is **not** here: it is the #14 ring gate
    /// ([`VerbPlan::gated_doorbell`]), which refuses to ring a channel whose working set
    /// is not host-published. So "forward it" cannot degrade into "hardware dereferences
    /// something that was never mapped" — that submission does not reach a doorbell.
    Untracked,
}

impl Representability {
    /// Which engine may be pointed at an address of this kind.
    ///
    /// ★ Note what is NOT consulted: the work kind, and who submitted it. Those are
    /// [`ce_executor_c`]'s inputs — the C's predicate, kept as the baseline this is
    /// measured against — and replacing them with the address is the whole content of
    /// §12's ruling.
    #[must_use]
    pub fn executor(self) -> CeExecutor {
        match self {
            Representability::HostBacked | Representability::Untracked => CeExecutor::HostCe,
            Representability::Fabricated | Representability::PhysicalOperand => CeExecutor::Ours,
        }
    }
}

/// One sub-range of a partitioned copy-engine request: the instruction, plus the
/// evidence that chose its engine.
///
/// The evidence rides along rather than being recomputed, because "why did this range go
/// to that engine" is exactly what a test must be able to assert without re-implementing
/// the classifier and thereby asserting nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeSpan {
    /// The instruction handed to the isolate.
    pub sub: CeSubCopy,
    /// Where the destination address lives.
    pub dst_kind: Representability,
    /// Where the source address lives — `None` for a scrub or a fill, which have no
    /// source operand (`C: :6320` "No src is set.").
    pub src_kind: Option<Representability>,
}

/// Upper bound on the sub-copies ONE copy-engine request may partition into.
///
/// A guest controls both the request's length and its address table's fragmentation, so
/// the span count is guest-influenced and needs a bound (boundary-1). Exceeding it is a
/// **loud refusal** ([`FwdFault::CeTooFragmented`]) and never a truncation: a partition
/// that stops early is a partition that silently drops the tail of a copy, which is the
/// C's own `#13 CE-DROP` failure mode (`C: :6389`) reintroduced on purpose.
pub const MAX_CE_SPANS: usize = 4096;

/// Upper bound on the sub-copies ONE `parse_pushbuffer` call may produce across ALL of a
/// ring's copy-engine requests.
///
/// [`MAX_CE_SPANS`] bounds any single request; a hostile ring can declare *many*
/// maximally fragmented ones, and the same reasoning that gave the parser
/// `MAX_PUSH_TOTAL_BYTES` beside `MAX_PUSH_RANGE_BYTES` applies here. Loud, never a
/// truncation, for the same reason.
pub const MAX_CE_SPANS_PER_PARSE: usize = 1 << 16;

/// Classify ONE already-resolved sub-range of an operand.
fn representability_of(binding: Option<&Binding>) -> Representability {
    match binding {
        Some(b) if b.host.is_some() => Representability::HostBacked,
        Some(_) => Representability::Fabricated,
        None => Representability::Untracked,
    }
}

/// The partition of one operand's range, as `(start, len, kind)` runs.
///
/// A physical operand is ONE run of [`Representability::PhysicalOperand`]: there is
/// nothing to look up, and no sub-range of it could be anything else.
fn operand_runs(
    table: Option<&AddressTable>,
    addr: GpuVa,
    is_virtual: bool,
    len: u64,
) -> Vec<(u64, u64, Representability)> {
    if len == 0 {
        return Vec::new();
    }
    if !is_virtual {
        return vec![(addr.0, len, Representability::PhysicalOperand)];
    }
    match table {
        Some(t) => t
            .spans(addr, len)
            .into_iter()
            .map(|(s, l, b)| (s, l, representability_of(b.as_ref())))
            .collect(),
        // No table for this channel's VAS at all: nothing is tracked, so nothing is
        // ours. The same clipping rule the table's own range query uses.
        None => {
            let end = (u128::from(addr.0) + u128::from(len)).min(1u128 << 64);
            let eff = (end - u128::from(addr.0)) as u64;
            if eff == 0 {
                Vec::new()
            } else {
                vec![(addr.0, eff, Representability::Untracked)]
            }
        }
    }
}

/// ★★★ **THE RANGE ALGEBRA** (§12.3): partition one copy-engine request into the
/// maximal sub-copies over which BOTH operands' representability is constant.
///
/// A copy has two ends, and a sub-copy may go to real hardware only if **both** of them
/// can be expressed to it. So the destination's partition and the source's partition are
/// **intersected** — at their common offsets, not at their addresses, because the two
/// operands sit at different addresses and advance together.
///
/// Guarantees, all pinned by the property test:
/// - the sub-copies are contiguous, ordered and non-overlapping;
/// - they cover the effective range **exactly** — a partition that is not total is a
///   silently dropped copy;
/// - none is zero-length;
/// - `dst`/`src` of the `i`-th sub-copy are the original operands advanced by the same
///   offset, which is what makes partition-then-execute byte-identical to
///   execute-whole.
///
/// A wrapping `addr + len` is CLIPPED, never wrapped (see [`AddressTable::spans`]); the
/// destination governs the effective length, and the source is clipped to match, so a
/// copy is never issued reading past where its destination stops.
///
/// # Errors
/// [`FwdFault::CeTooFragmented`] if the partition exceeds [`MAX_CE_SPANS`].
pub fn partition_ce(
    dst_table: Option<&AddressTable>,
    dst: GpuVa,
    dst_is_virtual: bool,
    src: GpuVa,
    src_is_virtual: bool,
    len: u64,
    work: kayfabe_arch::CeWork,
) -> Result<Vec<CeSpan>, FwdFault> {
    let dst_runs = operand_runs(dst_table, dst, dst_is_virtual, len);
    // The destination decides how much of the request exists at all (a clipped
    // destination clips the whole copy).
    let eff: u64 = dst_runs.iter().map(|(_, l, _)| *l).sum();
    if eff == 0 {
        return Ok(Vec::new());
    }
    // A scrub or a fill has NO source operand, so there is no second partition to
    // intersect — its representability is a property of its destination alone.
    let has_src = matches!(work, kayfabe_arch::CeWork::Copy);
    let src_runs = if has_src {
        operand_runs(dst_table, src, src_is_virtual, eff)
    } else {
        Vec::new()
    };

    // Walk both partitions by OFFSET into the request, cutting at every boundary either
    // one introduces. `src_runs` may be shorter than `eff` only if the source range was
    // clipped at the top of the address space; the remainder then has no source, which
    // is a source that reads nothing — modelled as untracked (forwardable), never as a
    // silent shortening of the destination.
    let mut out: Vec<CeSpan> = Vec::new();
    let mut off: u64 = 0;
    let mut di = 0usize;
    let mut d_consumed: u64 = 0;
    let mut si = 0usize;
    let mut s_consumed: u64 = 0;
    while off < eff {
        let (_, d_len, d_kind) = dst_runs[di];
        let d_left = d_len - d_consumed;
        let (s_left, s_kind) = if !has_src {
            (u64::MAX, None)
        } else if si < src_runs.len() {
            let (_, s_len, s_kind) = src_runs[si];
            (s_len - s_consumed, Some(s_kind))
        } else {
            (eff - off, Some(Representability::Untracked))
        };
        let take = d_left.min(s_left).min(eff - off);
        debug_assert!(take > 0, "a partition step must consume bytes");
        if out.len() == MAX_CE_SPANS {
            return Err(FwdFault::CeTooFragmented { dst, len });
        }
        let by = match s_kind {
            // Both ends must be expressible for hardware to run it. Combining by "the
            // stricter answer wins" rather than by a rule per operand: an unrepresentable
            // SOURCE is just as fatal to a real engine as an unrepresentable destination,
            // and the C says the same thing with `!src_phys && !dst_phys` (`C: :6310`).
            Some(s) => match (d_kind.executor(), s.executor()) {
                (CeExecutor::HostCe, CeExecutor::HostCe) => CeExecutor::HostCe,
                _ => CeExecutor::Ours,
            },
            None => d_kind.executor(),
        };
        out.push(CeSpan {
            sub: CeSubCopy {
                dst: dst.0.wrapping_add(off),
                src: match work {
                    kayfabe_arch::CeWork::Copy => CeSource::Address(src.0.wrapping_add(off)),
                    // A scrub zeroes; a fill writes its pattern. The C's scrub arm is a
                    // no-op only because ITS backing is sparse-zero — stating it as an
                    // explicit zero fill keeps the meaning where the backing cannot
                    // supply it.
                    kayfabe_arch::CeWork::Scrub => CeSource::Constant(0),
                    kayfabe_arch::CeWork::Fill { pattern } => CeSource::Constant(pattern),
                },
                len: take,
                by,
            },
            dst_kind: d_kind,
            src_kind: s_kind,
        });
        off += take;
        d_consumed += take;
        if d_consumed == d_len {
            di += 1;
            d_consumed = 0;
        }
        if has_src && si < src_runs.len() {
            s_consumed += take;
            if s_consumed == src_runs[si].1 {
                si += 1;
                s_consumed = 0;
            }
        }
    }
    // Adjacent sub-copies that ended up on the SAME engine are merged, so a boundary
    // that both partitions happen to agree across does not become two instructions. The
    // evidence is kept from the first of the run.
    let mut merged: Vec<CeSpan> = Vec::with_capacity(out.len());
    for s in out {
        match merged.last_mut() {
            Some(prev)
                if prev.sub.by == s.sub.by
                    && prev.dst_kind == s.dst_kind
                    && prev.src_kind == s.src_kind
                    && prev.sub.dst.wrapping_add(prev.sub.len) == s.sub.dst =>
            {
                prev.sub.len += s.sub.len;
            }
            _ => merged.push(s),
        }
    }
    Ok(merged)
}

/// ★★★ Build the ISOLATE's instruction for a partitioned request (§12.4 — *"the
/// executor is the isolate in both cases"*).
///
/// The core decides *what*; the isolate holds bytes and does *it*. There is deliberately
/// no path by which the pure core moves a byte: this returns a plan, and a plan is
/// executed on a checked-out worker with **no lock held** (R1).
///
/// An empty partition yields no plan: a request that covers nothing is not a verb.
#[must_use]
pub fn plan_ce_split(host_vas: HostHandle, spans: &[CeSpan]) -> Option<VerbPlan> {
    if spans.is_empty() {
        return None;
    }
    Some(VerbPlan::CeSplit {
        vas: host_vas,
        subs: spans.iter().map(|s| s.sub).collect(),
    })
}

/// ACT phase of the pushbuffer parse: apply the decoded `methods` of channel `cid`
/// to **its owning proc only** (`&mut Proc` + the read-only spine for the arch's
/// method decoder). Feeds: the operand split ([`classify_ce`]) → latched [`PtWrite`]s
/// for the caller to route to their owners; `SemRelease` → the proc's
/// `CompletionQueue`; honors `TlbInvalidate` membars; passes opaque methods through.
///
/// ★ It **observes** page-table writes and does not apply them: the owner of a written
/// page is routinely a different proc, and this phase holds only the issuing one.
/// [`latch_pt_writes`] is the applying half.
pub fn apply_pushbuffer(
    spine: &Spine,
    proc: &mut Proc,
    cid: ChanId,
    methods: Vec<(u32, Vec<u32>)>,
) -> Result<PushbufferOutcome, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let chan = proc.channels.get(&cid).ok_or(FwdFault::NoVas(cid))?;
    let chan_pdb = chan.vas_pdb;
    let cgpu = chan.gpu;
    // The C's `is_user_ce(s->chan_client)` conjunct — a property of the SUBMITTER, read
    // once per parse because a channel belongs to exactly one proc.
    let origin = ChannelOrigin::of(proc.id);

    let mut out = PushbufferOutcome::default();
    for (header, args) in methods {
        match spine.arch().pushbuffer().decode_method(header, &args) {
            kayfabe_arch::PushMethod::SetObject { .. } => {
                // Routing confirmation only — no address/completion state changes.
            }
            kayfabe_arch::PushMethod::CeLaunchDma {
                dst,
                src,
                len,
                dst_is_virtual,
                src_is_virtual,
                work,
            } => {
                // ★★★ DECISION 1 of 2 — EXECUTE.
                //
                // (a) The C's predicate (§11.5): work kind, BOTH operand forms, and the
                //     submitting channel's origin. No address. Recorded as the baseline.
                match ce_executor_c(work, origin, src_is_virtual, dst_is_virtual) {
                    CeExecutor::HostCe => out.c_execute_host_ce += 1,
                    CeExecutor::Ours => out.c_execute_ours += 1,
                }
                // (b) §12's ruling, which is what we ACT on: the ADDRESSES, partitioned.
                //     Resolved in the ISSUING channel's own Vas — the same table, and
                //     the same reason, as the capture decision below.
                let dst_table = chan_pdb
                    .and_then(|pdb| proc.vases.get(&(cgpu, pdb)))
                    .map(|v| &v.table);
                let spans = partition_ce(
                    dst_table,
                    dst,
                    dst_is_virtual,
                    src,
                    src_is_virtual,
                    len,
                    work,
                )?;
                if out.ce_spans.len() + spans.len() > MAX_CE_SPANS_PER_PARSE {
                    return Err(FwdFault::CeTooFragmented { dst, len });
                }
                out.ce_spans.extend(spans);
                // ★★★ DECISION 2 of 2 — CAPTURE. Reads the RESOLVED PHYSICAL destination
                // and nothing else. Independent of the above by construction: it is not
                // in scope of that match and cannot see its answer.
                match classify_ce(spine, proc, cid, chan_pdb, cgpu, dst, dst_is_virtual)? {
                    // ★ VA-OPERAND — not a page-table write. The operands are addresses
                    // the host MMU resolves for itself once the address space is
                    // resident, so the address plane has nothing to extract. Counted, not
                    // acted on. Whether hardware or we execute it is DECISION 1's answer.
                    CeOperands::VaOperand { .. } => out.data_copies += 1,
                    // ★★★ PHYS-OPERAND — a page-table write. The payload is guest-physical
                    // PTE values, which cannot be handed to hardware. LATCH the page here
                    // (O(1), index only — decoding per write livelocked on the bench,
                    // `C: :8686-8690`) and let the caller route it to its OWNER.
                    CeOperands::PhysOperand {
                        page,
                        aperture,
                        owner,
                        owner_pdb,
                    } => out.pt_writes.push(PtWrite {
                        gpu: cgpu,
                        page,
                        aperture,
                        owner,
                        owner_pdb,
                        bytes: len,
                    }),
                }
            }
            kayfabe_arch::PushMethod::SemRelease { addr, payload } => {
                // Completion observe on the OWNING proc's queue (per-`Proc`, §2.4).
                // A hostile guest flooding sem-releases is loud-capped, not OOM.
                proc.completion.observe(OsEventRef(addr.0 ^ payload))?;
                out.sem_releases.push((addr, payload));
            }
            kayfabe_arch::PushMethod::TlbInvalidate { pdb, membar } => {
                out.invalidates.push((pdb, membar));
                // A membar is a hard barrier: the interpreter honors it before
                // advancing (recorded here; the real transport blocks on refresh).
            }
            kayfabe_arch::PushMethod::Opaque => out.opaque += 1,
        }
    }
    Ok(out)
}

/// Parse the pushbuffer `ring` submitted on channel `cid` of proc `pid`, reading its
/// method words from guest memory via `vmm`. The **split-borrow composition** of
/// [`read_pushbuffer`] (spine read + guest-memory read) + [`apply_pushbuffer`]
/// (owning-proc act).
///
/// **Only runs where the core is already the mediator** (kernel/CeUtils/scrubber
/// channels + the CE-PT-write point). A userspace ring never carries a fact the core
/// must extract (verified safe, address_table.md §opaque-fast-path) — callers pass it
/// through as shared pages, no per-submit parse.
pub fn parse_pushbuffer(
    gpu: &mut Gpu,
    vmm: &mut dyn Vmm,
    pid: ProcId,
    cid: ChanId,
    ring: &[u8],
) -> Result<PushbufferOutcome, FwdFault> {
    let methods = read_pushbuffer(&gpu.spine, vmm, ring)?;
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    let out = apply_pushbuffer(&gpu.spine, proc, cid, methods)?;
    latch_pt_writes(gpu, &out.pt_writes);
    Ok(out)
}

/// ★★★ Route each latched page-table write to the `Vas` that **owns** the page.
///
/// This is a separate step from [`apply_pushbuffer`] for a structural reason, not a
/// stylistic one: the act phase holds the **issuing** proc, and the owner of a written
/// page-table page is routinely a **different** proc — the guest kernel is what writes a
/// user process's page tables. So the act phase can only *observe* the write; applying it
/// needs the device, which is exactly the plan/commit shape the rest of the plane uses.
///
/// A write whose owner has retired between the parse and here is **dropped, silently and
/// correctly**: its page tables are gone, and re-attaching a dirty page to a survivor is
/// how the C's never-pruned table aliased two processes (`eight_blockers_resolved.md` §2).
pub fn latch_pt_writes(gpu: &mut Gpu, writes: &[PtWrite]) {
    for w in writes {
        let Some(owner) = gpu.procs.get_mut(&w.owner) else {
            continue; // owner retired in the gap — the pages died with it
        };
        if owner.is_retired() {
            continue;
        }
        if let Some(vas) = owner.vases.get_mut(&(w.gpu, w.owner_pdb)) {
            vas.pt_pages.insert(w.page);
        }
    }
}

// =================================================================================
// Per-`Proc` working-set publication + ring-gate — THE #14 fix in code
// (`execution_plane.md` §2.4, decision #7, C: 6de85e7). The proven #14 root cause was
// an EXECUTION fault: the loser's GR channel took a host FAULT_PDE because its
// (identical) guest VAs were never published into its OWN host GR VAS. So before a
// channel's doorbell rings, its working set MUST be forward-populated into that
// channel's Vas's own host VAS; an unpublished VA at ring time is a LOUD fault, never
// a cross-proc content-pick (the exact confused-deputy designed out).
//
// The gate is STRUCTURAL: [`plan_doorbell`] is the ONE ring gate (★ corrected 2026-07-27:
// this said "[`handle_doorbell`] is the ONE ring path (nothing else in the workspace
// reaches `RmBackend::ring_doorbell`)" — false as stated; the sole `ring_doorbell` call
// site is in `kayfabe_isolate::Worker::execute`, and the L1 path goes through
// `kayfabe_rt::SharedDevice::doorbell`, not through `handle_doorbell`. Found by the
// whitepaper's verification pass). `plan_doorbell` is the sole constructor of
// `VerbPlan::Doorbell` in the production crates and it gates the caller-recovered
// working set against the channel's `Vas` table before returning one — so there is no
// ungated sibling to bypass and no un-gated plan any production path can hand a worker
// (the C's "one exec path" refactor-debt lesson, closed by construction).
//
// ★★ 2026-07-27: this used to carry a residual — "`VerbPlan` is a public enum, so this
// is a call-graph property, not a type-system one". It is now BOTH.
// `kayfabe_isolate::VerbPlan::Doorbell` is `#[non_exhaustive]` (no struct expression
// outside that crate: E0639, pinned by a trybuild row) and its only constructor,
// `VerbPlan::gated_doorbell`, RUNS this gate through the abstract `RingWorkingSet` view
// `VasGate` below implements. `gate_working_set` further down is the read-only QUERY
// form of the same predicate; it cannot ring anything.
// =================================================================================

/// Read-only query: would `working_set` pass channel `cid`'s ring-gate right now
/// (every VA published into that channel's Vas's own host VAS)? A VA with no host
/// publication (`Binding::host = None`) is a loud [`FwdFault`], never guessed.
///
/// This is the load-bearing per-`Vas` publication check: two procs' identical guest
/// VAs each resolve in their OWN Vas (keyed by PDB), so the gate passes for both only
/// because each published into its OWN host VAS (distinct `HostHandle`s). The
/// ENFORCING form lives inside [`plan_doorbell`] — this query cannot ring.
pub fn gate_working_set(
    gpu: &Gpu,
    pid: ProcId,
    cid: ChanId,
    working_set: &[GpuVa],
) -> Result<(), FwdFault> {
    let proc = gpu.procs.get(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    gate_working_set_in(proc, cid, working_set)
}

/// The per-proc form of [`gate_working_set`] (`&Proc` only — in L1: under that
/// proc's lock, no device-wide access needed). Same predicate, same loud faults.
///
/// **Every miss here ⇒ FAULT**, and one of them is the taxonomy's clearest illustration:
/// `chan.vas_pdb == None` is the *same absence* that `Gpu::sync_proc_to_boundary`
/// deliberately DEFERS on. At ring time it is never knowable — this submission is being
/// gated now — so it is `FwdFault::NoVas`, by name. The category belongs to the site, not
/// to the absence (`kayfabe_core` crate docs).
pub fn gate_working_set_in(
    proc: &Proc,
    cid: ChanId,
    working_set: &[GpuVa],
) -> Result<(), FwdFault> {
    let chan = proc.channels.get(&cid).ok_or(FwdFault::NoVas(cid))?;
    let pdb = chan.vas_pdb.ok_or(FwdFault::NoVas(cid))?;
    let vas = proc
        .vases
        .get(&(chan.gpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu: chan.gpu, pdb })?;
    gate_vas(&vas.table, pdb, working_set.iter().copied())
}

// =================================================================================
// Completion pattern (e) — the mapped-fence arm (`execution_plane.md` §1.2/§2.4;
// NVENC's fence-not-event shape, bench-proven in `nvenc_101`: the worker reads a
// GPU-written mapped fence with NO syscall). The channel's EngineKind selects the
// arm — exact at the Channel, never guessed from a parse. Distinct from the
// event-delivery path by construction: a fired fence never enters a
// CompletionQueue, never rides a DeliveryPlane batch, never raises SWGEN0.
// =================================================================================

/// Which completion arm a channel's [`EngineKind`] signals through (§2.4's
/// per-engine tie-in — the ONE place engine variety touches the completion plane).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionArm {
    /// Patterns (a)/(c): a semaphore write on a shared/published page (GR-compute,
    /// GR-graphics, CE) — passthrough-polled or parser-observed.
    SharedSema,
    /// Pattern (e): a mapped coherent fence the GPU writes and the guest worker
    /// reads with no syscall (NVENC).
    MappedFence,
}

/// The arm selection, keyed on the channel's [`EngineKind`] (which the `Channel`
/// carries — NVENC vs GR-compute is distinguishable at the channel, not just at
/// parse). NVDEC's completion shape is unproven (the declared honest gap): it stays
/// on the default shared-sema arm until bench-proven, never guessed onto the fence.
#[must_use]
pub fn completion_arm(engine: EngineKind) -> CompletionArm {
    match engine {
        EngineKind::NvEnc => CompletionArm::MappedFence,
        _ => CompletionArm::SharedSema,
    }
}

/// Arm a mapped-fence completion (pattern **e**) on channel `cid`: fire once the
/// fence at `addr` (in the channel's Vas) is observed at/after `target`, starting
/// from `current`. Returns `Ok(Some(event))` if the target is already reached at
/// arm time.
///
/// Discipline, all loud (MISS=FAULT):
/// - the channel's engine must select the fence arm ([`completion_arm`]) — arming
///   a fence on a sema-signalling channel is a [`FwdFault::WrongArm`];
/// - `addr` must be **mapped and host-published** in the channel's OWN Vas (the
///   host GPU writes it; an unpublished fence could never advance) — the same
///   per-`Vas` publication rule as the ring-gate;
/// - re-arms follow the retried-RPC discipline (identical = idempotent,
///   conflicting = loud) and the armed table is capacity-bounded (boundary-1);
/// - firing respects the #12 jump guard (`MAX_FENCE_JUMP`).
pub fn arm_fence(
    gpu: &mut Gpu,
    pid: ProcId,
    cid: ChanId,
    addr: GpuVa,
    current: u32,
    target: u32,
    event: OsEventRef,
) -> Result<Option<OsEventRef>, FwdFault> {
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    arm_fence_in(proc, cid, addr, current, target, event)
}

/// The per-proc ACT form of [`arm_fence`] (`&mut Proc` only — a fence arm touches
/// nothing device-global; in L1 it runs under that proc's lock).
pub fn arm_fence_in(
    proc: &mut Proc,
    cid: ChanId,
    addr: GpuVa,
    current: u32,
    target: u32,
    event: OsEventRef,
) -> Result<Option<OsEventRef>, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let chan = proc.channels.get(&cid).ok_or(FwdFault::NoVas(cid))?;
    if completion_arm(chan.engine) != CompletionArm::MappedFence {
        return Err(FwdFault::WrongArm {
            chan: cid,
            engine: chan.engine,
        });
    }
    let cgpu = chan.gpu;
    let pdb = chan.vas_pdb.ok_or(FwdFault::NoVas(cid))?;
    let vas = proc
        .vases
        .get(&(cgpu, pdb))
        .ok_or(FwdFault::UnknownPdb { gpu: cgpu, pdb })?;
    // The fence must be a mapped, host-published address in this channel's OWN Vas.
    gate_vas(&vas.table, pdb, [addr])?;
    Ok(proc.fences.arm((pdb.0, addr.0), current, target, event)?)
}

/// A host write to the fence at `(pdb, addr)` was observed carrying `value` (the
/// adapter feeds this from its fence-page observation point). Routes by PDB — the
/// data-plane identity — to the owning proc's fence arms; fires at/after target
/// under the #12 jump guard. A value on an un-armed fence is inert (`Ok(None)`).
pub fn fence_observed(
    gpu: &mut Gpu,
    target_gpu: GpuId,
    pdb: Pdb,
    addr: GpuVa,
    value: u32,
) -> Result<Option<OsEventRef>, FwdFault> {
    let pid = route_pdb(&gpu.spine, target_gpu, pdb)?;
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    fence_observed_in(proc, pdb, addr, value)
}

/// The per-proc ACT form of [`fence_observed`] (`&mut Proc` only; the caller
/// routed by PDB via [`route_pdb`] — in L1: device read lock + that proc's lock).
pub fn fence_observed_in(
    proc: &mut Proc,
    pdb: Pdb,
    addr: GpuVa,
    value: u32,
) -> Result<Option<OsEventRef>, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    Ok(proc.fences.observe((pdb.0, addr.0), value)?)
}

// =================================================================================
// The abstract present/display seam — GR-graphics's home (`execution_plane.md` §2.6).
// GR-graphics is the SAME engine as GR-compute (EngineKind::GrGraphics); the ONLY
// added surface is routing its scanout buffer to a `Present` sink, host-agnostic
// (QEMU/PRIME later; MockPresent now). The present-complete is fed back as a synthetic
// vblank via the OWNING proc's completion queue — never NVKMS.
// =================================================================================

/// Route proc `pid`'s GR-graphics scanout `buffer` — a [`SurfaceHandle`] minted by
/// that proc's own isolate (`RmBackend::export_surface`, the host-VRAM PRIME export;
/// guest-RAM handles do not typecheck here, GR-2a) — to the abstract [`Present`]
/// sink, then feed the present-complete back as a synthetic vblank on that proc's
/// completion queue (§2.4's graphics arm). Keeps display hypervisor/host-agnostic:
/// the core names only the [`Present`] seam; the concrete adapter (QEMU/PRIME) is a
/// later fill.
pub fn present_scanout(
    gpu: &mut Gpu,
    pid: ProcId,
    present: &mut dyn Present,
    buffer: SurfaceHandle,
    meta: FbMeta,
) -> Result<u64, FwdFault> {
    let proc = gpu.procs.get_mut(&pid).ok_or(FwdFault::RetiredProc(pid))?;
    present_scanout_in(proc, present, buffer, meta)
}

/// The per-proc ACT form of [`present_scanout`] (`&mut Proc` + the caller-owned
/// [`Present`] sink — nothing device-global; in L1: that proc's lock).
pub fn present_scanout_in(
    proc: &mut Proc,
    present: &mut dyn Present,
    buffer: SurfaceHandle,
    meta: FbMeta,
) -> Result<u64, FwdFault> {
    if proc.is_retired() {
        return Err(FwdFault::RetiredProc(proc.id));
    }
    let vblank = present.present(buffer, meta).map_err(FwdFault::Present)?;
    // Synthetic vblank → the proc's completion queue (the graphics completion arm).
    proc.completion.observe(OsEventRef(vblank.seq))?;
    Ok(vblank.seq)
}

// The concurrency contract, compile-time-asserted (decision #17).
kayfabe_util::assert_send_sync!(
    FwdFault,
    Published,
    DoorbellOutcome,
    DoorbellRoute,
    ControlRoute,
    CompletionArm,
    EngineObjectForwarded,
    EngineObjectRoute,
    PushbufferOutcome,
    Stale,
    Orphans,
    Refusal,
    PublishPlan,
    DoorbellPlan,
    EngineObjectPlan,
    ControlPlan,
    Planned<PublishPlan>,
);
