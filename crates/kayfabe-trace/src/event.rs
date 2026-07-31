//! The trace vocabulary — one variant per thing a **differential** can compare.
//!
//! Two families live in ONE enum, on purpose (`mode2_gsp_port_plan.md` §6.1,
//! *"Interleaving is the payload, not metadata"*):
//!
//! - the **wire plane** — the device stream a recorder captures: MMIO reads *with the
//!   value served*, MMIO writes, guest-RAM reads *with the bytes returned*, guest-RAM
//!   writes, IRQ raises, clock. These are the *input* a replay feeds back in, and the
//!   *assertion targets* on the output side.
//! - the **core planes** — the decisions the logic crates make: rmgraph apply, routing,
//!   address bind/unbind/resolve, doorbell dispatch, completion post/drain/poll, isolate
//!   verb. These are what a *decoded projection* differential asserts.
//!
//! Splitting them into two streams is the classic replay failure the port plan names:
//! you lose *"the C wrote the status tx header **before** it posted INIT_DONE"*. One
//! stream, one counter.
//!
//! ## ★ Decoded fields, never raw bytes — and never pre-rendered strings
//!
//! §6.3: the differential is over a decoded projection. Byte-diffing would enshrine the
//! C's zero padding, its uninitialised element tails, and `rpc.length = 36` for a
//! 32-byte header. Every variant here therefore carries *fields*, so a divergence can be
//! expressed as a field-level exception with a reason. The one place raw bytes survive
//! is [`TraceEvent::GuestRead`]/[`TraceEvent::GuestWrite`], where the bytes **are** the
//! observable (a replay that cannot answer a DMA read is not hermetic) — and even there
//! the harness is expected to decode before it compares.
//!
//! ## ★ Where a payload type lives above this crate
//!
//! `kayfabe-trace` sits **below** `kayfabe-core` in the crate lattice, so that
//! `kayfabe-core`, `kayfabe-fwd`, `kayfabe-rt` and `kayfabe-gsp` can all emit into it
//! without a cycle. It therefore cannot *name* `RmEvent`, `ProcId`, `FwdFault` or
//! `RmGraphError`. Two mechanisms cover that, and neither is a prose rule:
//!
//! - for **verbs**, a trace-local mirror plus a trait ([`AsRmVerb`]) the owning crate
//!   implements with an exhaustive `match` — a new `RmEvent` variant fails to compile
//!   until it is named here;
//! - for **refusals**, [`FaultTag`] plus [`Faulted`], implemented the same exhaustive
//!   way. This crate implements it for the two fault types below it in the lattice
//!   ([`AddressFault`], [`RmError`]); `kayfabe-fwd` implements it for `FwdFault`.
//!
//! Where a payload type IS below this crate it is used directly ([`AddressFault`],
//! [`RmError`], [`OsEventRef`], [`BatchId`], [`HostBacking`]) — carrying the real type
//! is what stops the vocabulary drifting away from the code it describes
//! (`testing_doctrine.md` §6.1, *"prefer a mechanism to a sentence"*).

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{ClassId, ControlCmd, EngineKind, Gpa, GpuId, GpuVa, HClient, HObject};
use kayfabe_arch::ids::{Pdb, VChid};
use kayfabe_completion::{BatchId, OsEventRef};
use kayfabe_isolate::{IsolateId, RmError, Txn, VerbPlan, WorkerId};
use kayfabe_mmu::{AddressFault, HostBacking};
// ★ The port plan's §6.1 `IrqSpec` already exists, portably, in `kayfabe-vmm` — the
// interrupt-delivery port the whole rewrite is written against. Re-exported rather than
// mirrored: a trace type that spelled interrupts as one hypervisor's raise API would be
// exactly the coupling the VMM-vocabulary gate exists to prevent.
pub use kayfabe_vmm::IrqSpec;

/// A record's position in the totally-ordered stream.
///
/// See [`crate::Recorder`] for exactly what that order does and does not guarantee —
/// it is a property of the recorder, not of this number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Seq(pub u64);

impl core::fmt::Display for Seq {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// One sequenced observation: the unit a sink stores and a differential compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Position in this recorder's total order.
    pub seq: Seq,
    /// What happened.
    pub ev: TraceEvent,
}

/// The value of a `kayfabe_core::ProcId`.
///
/// `ProcId` is core-owned semantics (*"the label of one dup-connected component of
/// declared user clients"*) and `kayfabe-core` is above this crate, so the type cannot
/// be named here. The conversion belongs in `kayfabe-core` as an `impl From<ProcId> for
/// ProcRef` the day it emits — a mechanism, not a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcRef(pub u32);

impl core::fmt::Display for ProcRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "proc{}", self.0)
    }
}

/// A PCI base address register index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bar(pub u8);

/// The width of one register access.
///
/// A closed enum rather than the port plan's `size: u8`, so that a recorder replaying a
/// captured stream turns a corrupt width into a **decode refusal** instead of carrying a
/// nonsense number into the comparison. Same information, one fewer way to be wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Width {
    /// One byte.
    B1,
    /// Two bytes.
    B2,
    /// Four bytes — the overwhelmingly common register access.
    B4,
    /// Eight bytes.
    B8,
}

impl Width {
    /// Decode a captured byte count. `None` for anything a register access cannot be.
    #[must_use]
    pub fn from_bytes(n: u8) -> Option<Width> {
        match n {
            1 => Some(Width::B1),
            2 => Some(Width::B2),
            4 => Some(Width::B4),
            8 => Some(Width::B8),
            _ => None,
        }
    }

    /// The byte count.
    #[must_use]
    pub fn bytes(self) -> u8 {
        match self {
            Width::B1 => 1,
            Width::B2 => 2,
            Width::B4 => 4,
            Width::B8 => 8,
        }
    }
}

/// The **exact** refusal variant, spelled by the type that produced it.
///
/// Not a rendered message: it is a discriminant name, and it is compiler-checked,
/// because every [`Faulted`] impl is an exhaustive `match` — adding a fault variant
/// without naming it here fails the build. That is what makes an assertion on a tag
/// equivalent to `testing_doctrine.md` §2's "name the variant".
///
/// Payloads are NOT folded in here: where the payload is the claim, the event carries it
/// in its own typed fields (an [`TraceEvent::AddressResolve`] already carries the `pdb`
/// and `va` a `AddressFault::Miss` would repeat), or the real error type is carried
/// whole ([`VerbOutcome::Failed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaultTag(pub &'static str);

impl core::fmt::Display for FaultTag {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

/// Names the exact refusal a fault type represents.
///
/// Implemented here for the fault types below this crate in the lattice, and by the
/// owning crate for the ones above it (`kayfabe-fwd` for `FwdFault`, `kayfabe-core` for
/// `RmGraphError`). Every impl must be an exhaustive `match` — that, not a comment, is
/// what keeps the vocabulary from drifting.
pub trait Faulted {
    /// The tag for this value's variant.
    fn fault_tag(&self) -> FaultTag;
}

impl Faulted for AddressFault {
    fn fault_tag(&self) -> FaultTag {
        match self {
            AddressFault::Miss { .. } => FaultTag("AddressFault::Miss"),
            AddressFault::Overlap { .. } => FaultTag("AddressFault::Overlap"),
            AddressFault::Malformed { .. } => FaultTag("AddressFault::Malformed"),
            AddressFault::HostVaMismatch { .. } => FaultTag("AddressFault::HostVaMismatch"),
            AddressFault::SliceLenMismatch { .. } => FaultTag("AddressFault::SliceLenMismatch"),
        }
    }
}

impl Faulted for RmError {
    fn fault_tag(&self) -> FaultTag {
        match self {
            RmError::InsufficientPermissions => FaultTag("RmError::InsufficientPermissions"),
            RmError::BadHandle(_) => FaultTag("RmError::BadHandle"),
            RmError::NoMemory => FaultTag("RmError::NoMemory"),
            RmError::Interrupted => FaultTag("RmError::Interrupted"),
            RmError::Wedged => FaultTag("RmError::Wedged"),
            RmError::ForeignHandle { .. } => FaultTag("RmError::ForeignHandle"),
            RmError::PlacementRefused { .. } => FaultTag("RmError::PlacementRefused"),
            RmError::NotExportableAsMemory { .. } => FaultTag("RmError::NotExportableAsMemory"),
            RmError::Other(_) => FaultTag("RmError::Other"),
        }
    }
}

/// The RM protocol verb an apply carried.
///
/// A trace-local mirror of `kayfabe_core::rmgraph::RmEvent`, which lives above this
/// crate. `kayfabe-core` bridges the two with [`AsRmVerb`]; the bridge is an exhaustive
/// `match`, so a new `RmEvent` variant cannot reach the graph without reaching the
/// trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RmVerb {
    /// `RM_ALLOC`.
    Alloc {
        /// The allocated class.
        class: ClassId,
        /// The parent handle in the same namespace.
        parent: HObject,
    },
    /// `DUP_OBJECT` (NVOS55) — the only cross-client transfer edge.
    Dup {
        /// The source client namespace.
        src_client: HClient,
        /// The source handle.
        src: HObject,
    },
    /// A `SET_PAGE_DIRECTORY`-shaped declaration.
    SetPageDir {
        /// The declared page-directory base.
        pdb: Pdb,
    },
    /// `RM_MAP_MEMORY_DMA` (NVOS46).
    MapMemoryDma {
        /// The memory object being mapped.
        memory: HObject,
        /// The guest VA the mapping starts at.
        va: GpuVa,
        /// The mapping length in bytes.
        len: u64,
    },
    /// `RM_UNMAP_MEMORY_DMA`.
    Unmap {
        /// The guest VA the mapping started at.
        va: GpuVa,
    },
    /// `RM_FREE`.
    Free,
}

/// Names the [`RmVerb`] an RM event represents.
///
/// Implemented by `kayfabe-core` for `RmEvent` (an exhaustive `match`), because that
/// type lives above this crate.
pub trait AsRmVerb {
    /// This event's verb.
    fn as_rm_verb(&self) -> RmVerb;
}

/// The identity a routing decision was keyed on.
///
/// The two data-plane identities the design keys every table by (rule L7): PDB for the
/// address plane, vChid for the exec plane. Both are **per-GPU namespaces**, which is
/// why the event carries the `GpuId` beside the key rather than folding them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RouteKey {
    /// An address-plane lookup, keyed by page-directory base.
    Pdb(Pdb),
    /// An exec-plane lookup, keyed by virtual channel id.
    VChid(VChid),
}

/// What a routing decision answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routed {
    /// The key routed to a live proc.
    To(ProcRef),
    /// The key did not route. The tag names the exact refusal — `UnknownPdb` and
    /// `Condemned` are near neighbours on this path, and `testing_doctrine.md` §2.3
    /// requires a test to say which.
    Refused(FaultTag),
}

/// What an address-plane resolve answered. **MISS = FAULT** — there is no third arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    /// A binding covers the VA.
    Hit {
        /// Offset of the VA within its binding.
        offset: u64,
        /// The host materialization, if this range is published at all. `None` is the
        /// #14 EXECUTION fault's precondition (bound in the shadow, absent from the
        /// channel's own host VAS) and is deliberately distinguishable from a miss.
        host: Option<HostBacking>,
    },
    /// The lookup faulted, with its exact variant and payload.
    Fault(AddressFault),
}

/// What a doorbell dispatch did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatched {
    /// Work was forwarded to the host.
    Rung {
        /// The proc that owns the channel.
        proc: ProcRef,
        /// The host doorbell token that was rung.
        host_token: u64,
        /// True if this dispatch had to make the channel runnable first (the first
        /// submission on that channel; the per-proc `ExecPlane` state, never a global
        /// one-shot — #12's CTX2 bug).
        scheduled_now: bool,
    },
    /// The doorbell was refused before any host work.
    Refused(FaultTag),
}

/// A completion-plane operation.
///
/// The four points `testing_doctrine.md` cares about on this plane: an event was
/// observed, a batch was posted, a batch was drained, a poll ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionOp {
    /// A host completion source signalled and the queue took it.
    Observed {
        /// The completion the host reported.
        event: OsEventRef,
    },
    /// A batch was composed and posted to the guest.
    Posted {
        /// The batch identity.
        batch: BatchId,
        /// How many completions it carried.
        len: usize,
    },
    /// The guest acknowledged the batch and the queues released it.
    Drained {
        /// The batch identity.
        batch: BatchId,
    },
    /// A poll ran. `posted` is `None` when the poll found nothing to deliver — which is
    /// an *observation*, not an absence, and is why the poll is recorded at all.
    Polled {
        /// The batch this poll posted, if any.
        posted: Option<BatchId>,
        /// Completions still outstanding after the poll.
        outstanding: usize,
    },
}

/// Which host-verb chain an isolate ran.
///
/// A trace-local mirror of [`VerbPlan`]'s discriminant. `VerbPlan` is *below* this crate,
/// so the bridge is a plain exhaustive `match` here rather than a trait upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VerbTag {
    /// Allocate + map guest-published backing.
    Publish,
    /// Materialize/schedule a channel and ring it.
    Doorbell,
    /// The Case-1 engine-object chain.
    EngineObject,
    /// One Case-1 control.
    Control,
    /// A copy-engine request partitioned by representability (`#102` stage C2).
    CeSplit,
    /// Dispose of host objects a refused commit could not adopt.
    Release,
}

impl VerbTag {
    /// The tag of a plan. Exhaustive: a new [`VerbPlan`] variant fails the build here.
    #[must_use]
    pub fn of(plan: &VerbPlan) -> VerbTag {
        match plan {
            VerbPlan::Publish { .. } => VerbTag::Publish,
            VerbPlan::Doorbell { .. } => VerbTag::Doorbell,
            VerbPlan::EngineObject { .. } => VerbTag::EngineObject,
            VerbPlan::Control { .. } => VerbTag::Control,
            VerbPlan::CeSplit { .. } => VerbTag::CeSplit,
            VerbPlan::Release { .. } => VerbTag::Release,
        }
    }
}

/// How an isolate verb chain ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerbOutcome {
    /// The chain completed.
    Ok,
    /// The chain failed, cancelled, or was abandoned. The real [`RmError`] is carried
    /// whole — `RmError::Interrupted` ("cancelled, did not fail") and `RmError::Wedged`
    /// ("no reply ever came") are different facts, and a tag alone would lose the
    /// `BadHandle`/`ForeignHandle` payloads.
    Failed(RmError),
}

/// The universal served-or-refused answer, for planes whose success carries no payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The operation was accepted.
    Ok,
    /// The operation was refused, with its exact variant named.
    Refused(FaultTag),
}

/// ★ One observation in the replay stream.
///
/// See the module docs for the two families and why they share one enum. Every variant
/// is something a *differential* can compare field by field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent {
    // ───────────────────────────── the wire plane (§6.1) ─────────────────────────────
    /// A register read, carrying **the value that was served** — not the value the
    /// register nominally holds. That distinction is the whole point of recording reads.
    MmioRead {
        /// Which BAR trapped.
        bar: Bar,
        /// Offset within the BAR.
        off: u64,
        /// Access width.
        size: Width,
        /// The value served to the guest.
        val: u64,
    },
    /// A register write.
    MmioWrite {
        /// Which BAR trapped.
        bar: Bar,
        /// Offset within the BAR.
        off: u64,
        /// Access width.
        size: Width,
        /// The value the guest wrote.
        val: u64,
    },
    /// A guest-RAM read, carrying **the bytes it returned**. A replay that cannot answer
    /// a DMA read is not hermetic (§6.1), so the bytes are part of the record.
    GuestRead {
        /// Guest-physical address read.
        gpa: Gpa,
        /// The bytes returned.
        bytes: Vec<u8>,
    },
    /// A guest-RAM write — an assertion target: this is what the guest would observe.
    GuestWrite {
        /// Guest-physical address written.
        gpa: Gpa,
        /// The bytes written.
        bytes: Vec<u8>,
    },
    /// An interrupt raise — an assertion target.
    IrqRaise {
        /// How it was delivered.
        spec: IrqSpec,
    },
    /// A clock observation. Recorded so a replay is deterministic without reading a real
    /// clock: the stream *is* the timebase.
    Clock {
        /// Nanoseconds on the recorded timebase.
        ns: u64,
    },

    // ──────────────────────────────── the core planes ────────────────────────────────
    /// One RM graph apply — the source-of-truth plane.
    RmApply {
        /// The GPU the client's device names, where the event declares one.
        gpu: Option<GpuId>,
        /// Owning client namespace.
        client: HClient,
        /// The handle the verb names.
        handle: HObject,
        /// The verb.
        verb: RmVerb,
        /// Accepted, or refused with its exact `RmGraphError` variant named.
        outcome: Outcome,
    },
    /// One routing decision — the projection plane.
    Route {
        /// The target GPU (routing keys are per-GPU namespaces).
        gpu: GpuId,
        /// The identity looked up.
        key: RouteKey,
        /// What it answered.
        outcome: Routed,
    },
    /// A VA range was forward-populated into a VAS's address table.
    AddressBind {
        /// The GPU the VAS belongs to.
        gpu: GpuId,
        /// The VAS, by its page-directory base.
        pdb: Pdb,
        /// Start of the range.
        va: GpuVa,
        /// Length in bytes.
        len: u64,
        /// The backing physical address the range points at.
        phys: u64,
        /// Which aperture that address lives in — the same number means different
        /// memory in each, so a projection that dropped it would compare unequal
        /// bindings as equal.
        aperture: Aperture,
        /// Accepted, or refused (overlap / malformed).
        outcome: Outcome,
    },
    /// A VA range was eagerly dropped.
    AddressUnbind {
        /// The GPU the VAS belongs to.
        gpu: GpuId,
        /// The VAS, by its page-directory base.
        pdb: Pdb,
        /// Start of the range.
        va: GpuVa,
        /// Whether a live binding was actually removed. `false` is an observation, not
        /// an error: an unbind of nothing is legal and must be distinguishable.
        removed: bool,
    },
    /// A VA was resolved — the address table IS the guest's TLB, so this is the TLB
    /// lookup, and its miss is a fault.
    AddressResolve {
        /// The GPU the VAS belongs to.
        gpu: GpuId,
        /// The VAS, by its page-directory base.
        pdb: Pdb,
        /// The VA looked up.
        va: GpuVa,
        /// What it answered.
        outcome: Resolved,
    },
    /// A doorbell was dispatched — the exec plane's hot path.
    Doorbell {
        /// The GPU whose BAR trapped.
        gpu: GpuId,
        /// The decoded virtual channel.
        vchid: VChid,
        /// The raw token the guest wrote.
        token: u64,
        /// What happened.
        outcome: Dispatched,
    },
    /// A completion-plane operation.
    Completion {
        /// The GPU the completion belongs to.
        gpu: GpuId,
        /// The owning proc.
        proc: ProcRef,
        /// Which operation.
        op: CompletionOp,
    },
    /// One host-verb chain ran on one isolate worker.
    IsolateVerb {
        /// The isolate whose handle namespace the verb ran in.
        isolate: IsolateId,
        /// The pool worker that ran it.
        worker: WorkerId,
        /// The transaction the cancel seam keys on.
        txn: Txn,
        /// Which chain.
        verb: VerbTag,
        /// How it ended.
        outcome: VerbOutcome,
    },
    /// A Case-1 control was classified and forwarded, or replayed internally.
    Control {
        /// The GPU the control targets.
        gpu: GpuId,
        /// The command.
        cmd: ControlCmd,
        /// The engine the target object belongs to, where one is known.
        engine: Option<EngineKind>,
        /// Accepted, or refused with its exact variant named.
        outcome: Outcome,
    },
}

/// The kind discriminant of a [`TraceEvent`], for counters and filtering.
///
/// Deliberately a separate, payload-free enum: the perf budget must be able to count a
/// stream without retaining it, and a filter must be expressible without constructing a
/// dummy event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventKind {
    /// [`TraceEvent::MmioRead`].
    MmioRead,
    /// [`TraceEvent::MmioWrite`].
    MmioWrite,
    /// [`TraceEvent::GuestRead`].
    GuestRead,
    /// [`TraceEvent::GuestWrite`].
    GuestWrite,
    /// [`TraceEvent::IrqRaise`].
    IrqRaise,
    /// [`TraceEvent::Clock`].
    Clock,
    /// [`TraceEvent::RmApply`].
    RmApply,
    /// [`TraceEvent::Route`].
    Route,
    /// [`TraceEvent::AddressBind`].
    AddressBind,
    /// [`TraceEvent::AddressUnbind`].
    AddressUnbind,
    /// [`TraceEvent::AddressResolve`].
    AddressResolve,
    /// [`TraceEvent::Doorbell`].
    Doorbell,
    /// [`TraceEvent::Completion`].
    Completion,
    /// [`TraceEvent::IsolateVerb`].
    IsolateVerb,
    /// [`TraceEvent::Control`].
    Control,
}

impl EventKind {
    /// Every kind, in declaration order. The counters array is indexed by
    /// [`EventKind::index`], and this is what makes "which kinds were never seen"
    /// — the non-vacuity question — answerable at all.
    pub const ALL: [EventKind; 15] = [
        EventKind::MmioRead,
        EventKind::MmioWrite,
        EventKind::GuestRead,
        EventKind::GuestWrite,
        EventKind::IrqRaise,
        EventKind::Clock,
        EventKind::RmApply,
        EventKind::Route,
        EventKind::AddressBind,
        EventKind::AddressUnbind,
        EventKind::AddressResolve,
        EventKind::Doorbell,
        EventKind::Completion,
        EventKind::IsolateVerb,
        EventKind::Control,
    ];

    /// How many kinds there are.
    pub const COUNT: usize = EventKind::ALL.len();

    /// Dense index into a per-kind array.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            EventKind::MmioRead => 0,
            EventKind::MmioWrite => 1,
            EventKind::GuestRead => 2,
            EventKind::GuestWrite => 3,
            EventKind::IrqRaise => 4,
            EventKind::Clock => 5,
            EventKind::RmApply => 6,
            EventKind::Route => 7,
            EventKind::AddressBind => 8,
            EventKind::AddressUnbind => 9,
            EventKind::AddressResolve => 10,
            EventKind::Doorbell => 11,
            EventKind::Completion => 12,
            EventKind::IsolateVerb => 13,
            EventKind::Control => 14,
        }
    }

    /// True if this kind belongs to the device wire plane (the recorded stream) rather
    /// than to a core decision plane.
    #[must_use]
    pub fn is_wire(self) -> bool {
        matches!(
            self,
            EventKind::MmioRead
                | EventKind::MmioWrite
                | EventKind::GuestRead
                | EventKind::GuestWrite
                | EventKind::IrqRaise
                | EventKind::Clock
        )
    }
}

impl core::fmt::Display for EventKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            EventKind::MmioRead => "MmioRead",
            EventKind::MmioWrite => "MmioWrite",
            EventKind::GuestRead => "GuestRead",
            EventKind::GuestWrite => "GuestWrite",
            EventKind::IrqRaise => "IrqRaise",
            EventKind::Clock => "Clock",
            EventKind::RmApply => "RmApply",
            EventKind::Route => "Route",
            EventKind::AddressBind => "AddressBind",
            EventKind::AddressUnbind => "AddressUnbind",
            EventKind::AddressResolve => "AddressResolve",
            EventKind::Doorbell => "Doorbell",
            EventKind::Completion => "Completion",
            EventKind::IsolateVerb => "IsolateVerb",
            EventKind::Control => "Control",
        };
        f.write_str(s)
    }
}

impl TraceEvent {
    /// This event's kind.
    #[must_use]
    pub fn kind(&self) -> EventKind {
        match self {
            TraceEvent::MmioRead { .. } => EventKind::MmioRead,
            TraceEvent::MmioWrite { .. } => EventKind::MmioWrite,
            TraceEvent::GuestRead { .. } => EventKind::GuestRead,
            TraceEvent::GuestWrite { .. } => EventKind::GuestWrite,
            TraceEvent::IrqRaise { .. } => EventKind::IrqRaise,
            TraceEvent::Clock { .. } => EventKind::Clock,
            TraceEvent::RmApply { .. } => EventKind::RmApply,
            TraceEvent::Route { .. } => EventKind::Route,
            TraceEvent::AddressBind { .. } => EventKind::AddressBind,
            TraceEvent::AddressUnbind { .. } => EventKind::AddressUnbind,
            TraceEvent::AddressResolve { .. } => EventKind::AddressResolve,
            TraceEvent::Doorbell { .. } => EventKind::Doorbell,
            TraceEvent::Completion { .. } => EventKind::Completion,
            TraceEvent::IsolateVerb { .. } => EventKind::IsolateVerb,
            TraceEvent::Control { .. } => EventKind::Control,
        }
    }

    /// The refusal this event records, if it records one.
    ///
    /// One accessor over every plane's outcome shape, so a harness can ask *"what was
    /// refused in this run, and exactly which variant"* without matching fifteen arms —
    /// and so the negative-trace assertion (`mode2_gsp_port_plan.md` §6.3) is one line.
    #[must_use]
    pub fn refusal(&self) -> Option<FaultTag> {
        match self {
            TraceEvent::RmApply { outcome, .. }
            | TraceEvent::AddressBind { outcome, .. }
            | TraceEvent::Control { outcome, .. } => match outcome {
                Outcome::Ok => None,
                Outcome::Refused(t) => Some(*t),
            },
            TraceEvent::Route { outcome, .. } => match outcome {
                Routed::To(_) => None,
                Routed::Refused(t) => Some(*t),
            },
            TraceEvent::AddressResolve { outcome, .. } => match outcome {
                Resolved::Hit { .. } => None,
                Resolved::Fault(f) => Some(f.fault_tag()),
            },
            TraceEvent::Doorbell { outcome, .. } => match outcome {
                Dispatched::Rung { .. } => None,
                Dispatched::Refused(t) => Some(*t),
            },
            TraceEvent::IsolateVerb { outcome, .. } => match outcome {
                VerbOutcome::Ok => None,
                VerbOutcome::Failed(e) => Some(e.fault_tag()),
            },
            TraceEvent::MmioRead { .. }
            | TraceEvent::MmioWrite { .. }
            | TraceEvent::GuestRead { .. }
            | TraceEvent::GuestWrite { .. }
            | TraceEvent::IrqRaise { .. }
            | TraceEvent::Clock { .. }
            | TraceEvent::AddressUnbind { .. }
            | TraceEvent::Completion { .. } => None,
        }
    }
}
