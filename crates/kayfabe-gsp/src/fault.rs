//! Every way this crate refuses, named.
//!
//! `testing_doctrine.md` §2: a test asserts the **variant, with its payload** — never
//! `is_err()`. So every refusal carries the guest-supplied number that caused it, and no
//! two refusals share a variant "because they are both bad input".
//!
//! ## Refuse the message, never the device (G8)
//!
//! `mode2_gsp_port_plan.md` §7-G8: two guest polls hang the *guest* forever if we stop
//! answering, and both are on the teardown path — the fn-47 reply
//! (`ogkm: src/nvidia/src/kernel/vgpu/rpc.c:9146-9170`, synchronous) and the
//! suspend-sentinel poll (`ogkm: kernel_gsp_tu102.c:352-357`). So a `GspFault` is
//! **per-message**: it aborts one service step, and the register surface keeps answering.
//! Nothing in this crate can stop the observable surface, which is why
//! [`GspFault`] is a return type and never a state.

/// A guest-RAM access the host refused. The port ([`crate::GuestRam`]) reports it
/// without naming the hypervisor's own error type, which this crate may not see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RamRefused {
    /// The guest-physical address of the refused access.
    pub gpa: u64,
    /// Its length in bytes.
    pub len: usize,
}

/// The `msgqRxLink` rejection codes, as the **guest's own** predicate spells them.
///
/// Reproduced code-for-code from `ogkm: src/nvidia/src/libraries/msgq/msgq.c:329-461`
/// so that "would the guest link the header we just published?" is answerable here,
/// offline, without a bench slot. The numeric values are the guest's return values.
///
/// ★ `-7` is the one that mattered: it has exactly one cause, `rx.size != size`
/// (`msgq.c:386-389`), which is the signature of *a status queue that never received a
/// tx header* — the measured failure of the C artifact's second driver life
/// (`docs/reference/mode2_bench_lifecycle.md` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RxLinkCode {
    /// `-1` — handle null, or already linked (`msgq.c:335-338`).
    AlreadyLinked,
    /// `-2` — `msgSize < MSGQ_MSG_SIZE_MIN` (`msgq.c:340-343`).
    MsgSizeBelowMin,
    /// `-3` — `msgSize > size` (`msgq.c:345-348`).
    MsgSizeAboveQueue,
    /// `-5` — null backing store (`msgq.c:350-353`). There is no `-4`.
    NullBackingStore,
    /// `-6` — `size < rx.entryOff + msgSize` (`msgq.c:380-383`).
    EntryOffTooLarge,
    /// `-7` — `rx.size != size` (`msgq.c:386-389`).
    SizeMismatch,
    /// `-8` — `rx.msgSize != msgSize` (`msgq.c:390-393`).
    MsgSizeMismatch,
    /// `-9` — `rx.version != MSGQ_VERSION` (`msgq.c:394-397`).
    VersionMismatch,
    /// `-10` — the derived-field triple (`msgq.c:400-405`).
    DerivedFieldsMismatch,
    /// `-11` — the backend read of the peer header failed (`msgq.c:370-373`). Reachable
    /// here as "we could not read the guest's header at all".
    BackendReadFailed,
    /// `-12` — the backend write of the zeroed read pointer failed (`msgq.c:438-441`).
    BackendWriteFailed,
}

impl RxLinkCode {
    /// The guest's own numeric return value. Kept because a bench log prints the number,
    /// and a test that asserts `-7` should be reading the same number the driver did.
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            RxLinkCode::AlreadyLinked => -1,
            RxLinkCode::MsgSizeBelowMin => -2,
            RxLinkCode::MsgSizeAboveQueue => -3,
            RxLinkCode::NullBackingStore => -5,
            RxLinkCode::EntryOffTooLarge => -6,
            RxLinkCode::SizeMismatch => -7,
            RxLinkCode::MsgSizeMismatch => -8,
            RxLinkCode::VersionMismatch => -9,
            RxLinkCode::DerivedFieldsMismatch => -10,
            RxLinkCode::BackendReadFailed => -11,
            RxLinkCode::BackendWriteFailed => -12,
        }
    }
}

/// An [`crate::ElementLayout`] that could not describe a real element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayoutError {
    /// A named field does not fit inside the declared header.
    FieldOutsideHeader {
        /// Byte offset of the offending field.
        offset: usize,
        /// The declared header size it must fit in.
        hdr_size: usize,
    },
    /// Two named fields overlap.
    FieldsOverlap {
        /// The first field's offset.
        a: usize,
        /// The second field's offset.
        b: usize,
    },
    /// The header is too small to hold a checksum and a sequence number.
    HeaderTooSmall {
        /// The declared header size.
        hdr_size: usize,
    },
}

/// Every refusal the faked GSP can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GspFault {
    /// The architecture has no [`kayfabe_arch::GspModel`]. MISS = FAULT: there is no
    /// default register model, so a device configured without one refuses loudly rather
    /// than serving zeros.
    NoGspModel,
    /// A register this model decodes, but cannot serve for the current state.
    RegisterUnserviceable {
        /// The register that had no value.
        reg: kayfabe_arch::GspReg,
    },
    /// A guest-RAM access was refused by the VMM.
    GuestRam(RamRefused),
    /// The queue is not bound: there is no geometry, so there is nothing to parse.
    ///
    /// ★ This is the type-level fix for the C's `q_ready`-guarded loose fields
    /// (`C: src/qemu/nvkvm_gpu_emul.c:188-196`): the GPA only exists inside
    /// [`crate::QueueState::Bound`], so the "parse arbitrary guest RAM as RPC" defect is
    /// unrepresentable rather than merely avoided.
    QueueNotBound,
    /// The guest's published geometry failed its own acceptance predicate, so publishing
    /// a matching status header could not have linked.
    GeometryRejected(RxLinkCode),
    /// `msgCount` was zero — the C artifact's unguarded `% s->q_msgcount` SIGFPE
    /// (`C:1615`), refused here at decode instead.
    MsgCountZero,
    /// The guest's queue flags do not carry `MSGQ_FLAGS_SWAP_RX`.
    ///
    /// `rxSwapped` is the **AND** of both sides' flags (`ogkm: msgq.c:411-412`), so a
    /// guest that did not set it flips the read-pointer polarity and both sides deadlock
    /// **silently, with no error**. Every known client sets it — the driver
    /// (`ogkm: message_queue_cpu.c:174-180`) and nouveau (`nv: r535/gsp.c:1171`) — so
    /// this refusal only ever fires on input a conforming guest does not produce, and it
    /// exists to convert the one failure mode with no diagnostic into a named one.
    SwapRxNotAgreed {
        /// The flags word the guest published.
        flags: u32,
    },
    /// The guest's read pointer into our status queue is out of range. Hostile input:
    /// the guest writes it (`ogkm: msgq.c:485-488` returns "no space" for the same case).
    PeerReadPtrOutOfRange {
        /// The value read.
        value: u32,
        /// The ring's element count.
        count: u32,
    },
    /// The guest's write pointer into its command queue is out of range
    /// (`ogkm: msgq.c:655-658`).
    PeerWritePtrOutOfRange {
        /// The value read.
        value: u32,
        /// The ring's element count.
        count: u32,
    },
    /// The status ring cannot take this message without overwriting elements the guest
    /// has not consumed. **Retryable**, not fatal — the completion plane's back-pressure.
    QueueFull {
        /// How many elements the message needs.
        needed: u32,
        /// How many are free (`readPtr + msgCount - writePtr - 1`, `ogkm: msgq.c:490`).
        free: u32,
    },
    /// A message length outside the bounds the transport allows.
    MsgLenOutOfRange {
        /// The declared `rpc.length`.
        declared: u32,
        /// The lower bound (the RPC envelope's own size).
        min: u32,
        /// The upper bound (`queueElementSizeMax - queueElementHdrSize`).
        max: u32,
    },
    /// ★★ **GSP-S1.** An element count — ours on the way out, or the guest's on the way in
    /// — that exceeds what the receive **staging buffer** can hold.
    ///
    /// This is a memory-safety bound aimed at the *guest's* kernel, not a lint. The
    /// staging buffer is `queueElementSizeMax` bytes with the live `msgq` metadata carved
    /// immediately after it (`ogkm-580: message_queue_cpu.c:132-134, 143-145`), and the
    /// copy loop that fills it is bounded only by ring availability
    /// (`ogkm-580: :628, 648-650`). At the reachable maximum of 62 elements on a 63-slot
    /// ring that is `(62 - 16) * 4096 = 188 416` bytes written past a
    /// `portMemAllocNonPaged` allocation. Same defect class as the C artifact's unguarded
    /// `% s->q_msgcount` SIGFPE (`C:1615`), pointed at the guest instead of at QEMU.
    ///
    /// See [`crate::max_elements`] for the derivation; the bound is never a literal.
    ElementCountOutOfRange {
        /// The count that was declared (or would have been emitted).
        count: u32,
        /// `queueElementSizeMax / element_size` — what the staging buffer holds.
        max: u32,
    },
    /// The guest's `elemCount` field disagrees with the count its own `rpc.length`
    /// derives.
    ///
    /// For a conforming guest the two are equal by construction: `elemCount` is assigned
    /// `GSP_MSG_QUEUE_BYTES_TO_ELEMENTS(hdrSize + rpc.length)` unconditionally, outside
    /// the Confidential-Compute branch (`ogkm-580: message_queue_cpu.c:482`). For a
    /// non-conforming one they are not, and since the producer advanced its `writePtr` by
    /// the **field** (`:578`) while a length-derived consumer would advance by the
    /// derivation, accepting the message desynchronises the ring for the rest of the
    /// device's life. Refused by name instead, with the cursor left where it was.
    ///
    /// Unreachable on layouts with no such field — see [`crate::peek_elem_count`].
    ElementCountMismatch {
        /// What the element's `elemCount` field said.
        declared: u32,
        /// What `ceil((hdrSize + rpc.length) / elementSize)` derives.
        derived: u32,
    },
    /// The element's checksum did not fold to zero.
    ChecksumMismatch {
        /// What the fold produced (zero is the only accepted value).
        folded: u32,
    },
    /// The element's sequence number is not the one expected next.
    SeqNumGap {
        /// The sequence number we were expecting.
        expected: u32,
        /// The one the element carried.
        got: u32,
    },
    /// The transport (MCTP/NVDM) header did not carry the expected word.
    TransportHeaderInvalid {
        /// Byte offset of the offending word within the element.
        offset: usize,
        /// The word the guest wrote.
        found: u32,
        /// The word the driver's own encoder produces.
        expected: u32,
    },
    /// A buffer was too short for the structure being read out of it.
    Truncated {
        /// Bytes needed.
        need: usize,
        /// Bytes available.
        have: usize,
    },
    /// The region's self-describing page table could not be used.
    RegionMalformed(RegionError),
    /// An offset/length pair fell outside the region the guest declared.
    RegionOutOfRange {
        /// The byte offset within the region.
        offset: u64,
        /// The access length.
        len: u64,
        /// The region's declared size.
        region_len: u64,
    },
    /// The RPC envelope failed `kayfabe-abi`'s validation. Carried as the ABI's own
    /// error so the exact reason survives (bad signature vs impossible length).
    Envelope(kayfabe_abi::wire::AbiError),
    /// An event may not be posted before the boot handshake has completed
    /// (`mode2_gsp_port_plan.md` §7-G7; the guest `NV_ASSERT(0)`s on a non-allowlisted
    /// event during its bootup window, `ogkm: kernel_gsp.c:1419-1440`).
    NotRunning,
    /// The element layout supplied by the ABI seam is self-inconsistent.
    Layout(LayoutError),
    /// Two entries of the function-id table carry the same wire id, so classification
    /// would answer with whichever arm matched first.
    DuplicateFunctionCode {
        /// The id that appears twice.
        code: u32,
    },
    /// The LibOS region array carried no `RMARGS` region, so there is nothing to bind to.
    ///
    /// The C stops at the first all-zero entry (`C:3399-3401`); that the array is
    /// zero-terminated is **[inferred] I8** — the header declares no sentinel — so a zero
    /// entry is treated here as *skip*, and this is the refusal after the whole declared
    /// array has been scanned.
    RmargsRegionAbsent {
        /// How many entries were scanned.
        scanned: usize,
    },
    /// The guest declared a `queueElementHdrSize` that disagrees with the layout the
    /// version key selected.
    ///
    /// Reachable only on drivers whose `MESSAGE_QUEUE_INIT_ARGUMENTS` carries the field
    /// (610-shaped). It is a **cross-check of two independent sources** — the guest's own
    /// declaration against the ABI table — and a disagreement means one of them is wrong
    /// about the driver we are talking to, which is precisely the silent-wrong-profile
    /// failure the version key exists to prevent.
    DeclaredHeaderSizeMismatch {
        /// What the guest declared.
        declared: u64,
        /// What the selected layout says.
        expected: u64,
    },
}

/// Why a region's page table could not be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RegionError {
    /// The page size is not a non-zero power of two.
    BadPageSize {
        /// The value supplied.
        page_size: u64,
    },
    /// The guest declared zero page-table entries, so the region has no pages.
    NoEntries,
    /// The declared entry count exceeds the bound the caller allows.
    TooManyEntries {
        /// The declared count.
        declared: u32,
        /// The cap.
        max: u32,
    },
    /// A page-table entry is not page-aligned.
    UnalignedEntry {
        /// Which entry.
        index: usize,
        /// Its value.
        value: u64,
    },
}

impl From<RamRefused> for GspFault {
    fn from(e: RamRefused) -> Self {
        GspFault::GuestRam(e)
    }
}

impl From<RegionError> for GspFault {
    fn from(e: RegionError) -> Self {
        GspFault::RegionMalformed(e)
    }
}

impl From<LayoutError> for GspFault {
    fn from(e: LayoutError) -> Self {
        GspFault::Layout(e)
    }
}

impl From<kayfabe_abi::wire::AbiError> for GspFault {
    fn from(e: kayfabe_abi::wire::AbiError) -> Self {
        GspFault::Envelope(e)
    }
}

kayfabe_util::assert_send_sync!(GspFault, RxLinkCode, RegionError, LayoutError, RamRefused);
