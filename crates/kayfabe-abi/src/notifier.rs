//! Axis A: the **error-notifier record** a GSP writes into the guest's own memory when it
//! has RC'd a channel — and where the guest told us to write it.
//!
//! `docs/design/simulated_gpu_fault.md` §5.3 / `docs/design/resume_from_fault.md` §S5(a).
//! This module owns two things and nothing else: the 16 bytes, and the decode of the
//! address out of `NV_CHANNEL_ALLOC_PARAMS`. Both are NVIDIA `#[repr(C)]` facts, so they
//! are quarantined here (decision #2) and no crate above may state an offset for them.
//!
//! # ★★★ Why the write exists at all
//!
//! With Confidential Compute off — our target — **CPU-RM does not write it.** The
//! receiver's conditional is explicit (`ogkm-580:
//! src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:657-668`): `if (bIsCcEnabled &&
//! pKernelChannel != NULL) { krcErrorSetNotifier(...); }`, with the comment *"With CC
//! enabled, CPU-RM needs to write error notifiers"*. Off, it calls
//! `krcErrorSendEventNotifications_HAL` instead, whose own doc says why — *"GSP writes to
//! notifiers to avoid a race condition between the time when an error is handled and when
//! the client receives notifications … This function actually sends those notifications"*
//! (`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:352-372`).
//!
//! So the split is: **the GSP writes the bytes, CPU-RM rings the doorbell about them.** We
//! are the GSP. Sending the event without the bytes leaves every consumer polling a zero
//! — and the one this port can read spins forever on exactly that
//! (`kayfabe_arch::fault::ErrorNotifier`'s docs carry the citations).
//!
//! # ★★ ONE record, two C typedefs — established, not assumed
//!
//! `hErrorContext` resolves to either a ContextDma or a `Memory`, and RM writes a
//! *different C struct* through each
//! (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2016-2090`,
//! `ErrorNotifierType` at `ogkm-580: src/nvidia/generated/g_kernel_channel_nvoc.h:96-119`):
//!
//! ```text
//!   NOTIFICATION      (ctxdma)                 NvNotification       (memory)
//!   +0   NvU32 TimeLo                          +0   NvU32 nanoseconds[0]
//!   +4   NvU32 TimeHi                          +4   NvU32 nanoseconds[1]
//!   +8   NvV32 OtherInfo32                     +8   NvV32 info32
//!   +12  NvV16 OtherInfo16                     +12  NvV16 info16
//!   +14  NvV16 Status                          +14  NvV16 status
//! ```
//!
//! (`ogkm-580: src/nvidia/inc/kernel/gpu/mem_mgr/virt_mem_allocator_common.h:51-69` and
//! `ogkm-580: src/common/sdk/nvidia/inc/nvgputypes.h:57-64`.) **The three fields any
//! consumer reads sit at the same three offsets in both**, so this port encodes one
//! record and is right for either kind. The only genuine disagreement is in the
//! timestamp's two halves, and it is NVIDIA's own: `notifyFillNOTIFICATION` writes
//! `TimeLo` to +0, while `notifyFillNvNotification` writes `TimeHi` to +0
//! (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/method_notification.c:138-150` vs
//! `:181-185`). Nothing in either consumer path reads it back, so this port writes the
//! timestamp as one little-endian `u64` and says so rather than picking a side silently.
//!
//! # ★ The publication ORDER is part of the contract
//!
//! RM writes `info16`, `info32` and the timestamp first and the **status last**
//! (`method_notification.c:181-185`), because `status` is the flag a poller spins on —
//! `uvm_channel_get_status` tests `error_notifier->status == 0` and nothing else
//! (`ogkm-580: kernel-open/nvidia-uvm/uvm_channel.c:2058-2082`). A record published
//! status-first is a window in which a waiter reads "there is an error" beside a stale
//! exception type. [`ErrorNotification::PUBLISH_SPLIT`] is that ordering, expressed as
//! data so the caller cannot get it wrong by writing sixteen bytes in one go.

use kayfabe_arch::fault::ErrorNotifier;

use crate::wire::{AbiError, u32_at, u64_at};

/// `sizeof(NvNotification)` == `sizeof(NOTIFICATION)` — the record this port writes.
pub const NOTIFICATION_SIZE: usize = 16;

/// The offset the **status** half-word lives at, and the split point of
/// [`ErrorNotification::PUBLISH_SPLIT`].
pub const NOTIFICATION_STATUS_OFF: usize = 14;

/// `notifierStatus` — the literal RM passes for a robust-channel error.
///
/// ★ Not a `NV_STATUS`, despite the parameter's name: `krcErrorSetNotifier_IMPL` calls
/// `krcErrorWriteNotifier_HAL(…, 0xffff /* notifierStatus */, …)`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/rc/kernel_rc_notification.c:326-338`). The value
/// is a sentinel; the *reason* travels in `info32`. Any non-zero would unstick the
/// waiters, and this is the one the guest's own driver writes.
pub const NOTIFIER_STATUS_RC: u16 = 0xffff;

/// `NV_CHANNELGPFIFO_NOTIFICATION_TYPE_ERROR` — index 0 of the notifier array
/// (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:2858`). RM's own write passes
/// `0 /* Index */` (`kernel_rc_notification.c:70-97`), and `errorNotifierMem.base`
/// already has the channel's `errorContextOffset` folded in
/// (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:557-560`), so this port
/// adds nothing to the base.
pub const NOTIFICATION_TYPE_ERROR_INDEX: u64 = 0;

/// One channel error notification, in field terms.
///
/// No `Default`: a zeroed record has `status == 0`, which is precisely the value every
/// consumer reads as *"no error"* — a well-formed way to say nothing happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorNotification {
    /// `info32` — the `ROBUST_CHANNEL_*` exception type, the same number the RC event
    /// carries in `exceptType` and a kernel log prints as `Xid`
    /// (`kernel_rc_notification.c:70-97` passes `exceptType` here).
    pub except_type: u32,
    /// `info16` — `gpuGetNv2080EngineType(localRmEngineType)`, i.e. the same
    /// `nv2080EngineType` the RC event routes on (`kernel_rc_notification.c:74-97`).
    pub engine_type: u16,
    /// `status` — [`NOTIFIER_STATUS_RC`]. Kept a field rather than hard-wired so a future
    /// caller with a different sentinel edits data, not the encoder.
    pub status: u16,
    /// The timestamp halves, as one value. See the module docs for why it is one `u64`
    /// here and why nothing reads it.
    pub timestamp: u64,
}

impl ErrorNotification {
    /// The bytes a caller must publish **first**: everything except the status.
    ///
    /// See the module docs — status-last is RM's own order and it is what makes the
    /// record safe to poll. A caller writes `[..PUBLISH_SPLIT]`, then
    /// `[PUBLISH_SPLIT..]`.
    pub const PUBLISH_SPLIT: usize = NOTIFICATION_STATUS_OFF;

    /// The RC notification for `except_type` on `engine_type`, with the status sentinel
    /// RM writes.
    #[must_use]
    pub fn rc(except_type: u32, engine_type: u16, timestamp: u64) -> Self {
        Self {
            except_type,
            engine_type,
            status: NOTIFIER_STATUS_RC,
            timestamp,
        }
    }

    /// The 16-byte little-endian image.
    #[must_use]
    pub fn encode(&self) -> [u8; NOTIFICATION_SIZE] {
        let mut b = [0u8; NOTIFICATION_SIZE];
        b[0..8].copy_from_slice(&self.timestamp.to_le_bytes());
        b[8..12].copy_from_slice(&self.except_type.to_le_bytes());
        b[12..14].copy_from_slice(&self.engine_type.to_le_bytes());
        b[NOTIFICATION_STATUS_OFF..NOTIFICATION_SIZE].copy_from_slice(&self.status.to_le_bytes());
        b
    }
}

/// `ErrorNotifierType`, `internalFlags[3:2]` of `NV_CHANNEL_ALLOC_PARAMS`
/// (`ogkm-580: src/nvidia/generated/g_kernel_channel_nvoc.h:185-189`, values from the
/// `ErrorNotifierType` enum at `:96-119`).
const NOTIFIER_TYPE_SHIFT: u32 = 2;
const NOTIFIER_TYPE_MASK: u32 = 0b11;
const NOTIFIER_TYPE_UNKNOWN: u32 = 0;
const NOTIFIER_TYPE_NONE: u32 = 1;
const NOTIFIER_TYPE_CTXDMA: u32 = 2;
const NOTIFIER_TYPE_MEMORY: u32 = 3;

/// `NV_ADDR_SYSMEM` — `errorNotifierMem.addressSpace`
/// (`ogkm-580: src/nvidia/inc/kernel/vgpu/rm_plugin_shared_code.h:65-70`).
const ADDR_SYSMEM: u32 = 1;

/// Where the two fields sit in `NV_CHANNEL_ALLOC_PARAMS` at one driver boundary.
///
/// ★★ A **per-boundary** fact and not a constant, because this struct's tail is the one
/// in the slice that is *known to have moved*: 610.43.02 inserts `hHandleVASpace` after
/// `hVASpace` and every field from +32 on shifts by 8
/// (`ogkm-610: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-352` against
/// `ogkm-580: :296-342`). `crate::view::ChannelAllocFacts` refuses to read past +32 for
/// exactly that reason; this type is how a *read* tree buys the right to read further,
/// and a boundary whose tree nobody has opened carries `None` rather than a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelNotifierWire {
    /// Offset of `NvU32 internalFlags`.
    pub internal_flags: usize,
    /// Offset of `NV_MEMORY_DESC_PARAMS errorNotifierMem`.
    pub error_notifier_mem: usize,
}

impl ChannelNotifierWire {
    /// 580.159.04 — the bench driver. `hPhysChannelGroup` @ +240, `internalFlags` @ +244,
    /// `errorNotifierMem` @ +248, from `ogkm-580:
    /// src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-342` with
    /// `NV_MAX_SUBDEVICES = 8` (`ogkm-580: src/common/sdk/nvidia/inc/nvlimits.h:42`) and
    /// `NV_MEMORY_DESC_PARAMS` = 24 bytes (`alloc_channel.h:37-42`). The same arithmetic
    /// independently produces `crate::submit::ChannelAllocParams`'s documented map.
    pub const V580: Self = Self {
        internal_flags: 244,
        error_notifier_mem: 248,
    };

    /// 610.43.02 — the same fields, eight bytes later, because `hHandleVASpace` @ +32 is
    /// four bytes and forces four more of re-alignment onto the 8-aligned
    /// `userdOffset[]` (`ogkm-610: alloc_channel.h:296-352`).
    pub const V610: Self = Self {
        internal_flags: 252,
        error_notifier_mem: 256,
    };

    /// The bytes a decode needs: through `addressSpace`, the last field it reads.
    ///
    /// `cacheAttrib` follows and is not read — the notifier is written by us and read by
    /// the guest's CPU, never by an engine, so its cache attribute is the guest's own
    /// problem and RM's own comment says the writes need no flush
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/method_notification.c:176-180`).
    #[must_use]
    pub const fn needs(&self) -> usize {
        self.error_notifier_mem + 20
    }

    /// Decode the channel's declared error notifier out of `NV_CHANNEL_ALLOC_PARAMS`.
    ///
    /// `Ok(None)` is **not** an error and covers two situations that share one outcome:
    /// the channel declared no notifier (`ERROR_NOTIFIER_TYPE_NONE`, or `_UNKNOWN`, which
    /// is what a kernel CPU-RM client leaves in the field before RM resolves it —
    /// `ogkm-580: g_kernel_channel_nvoc.h:96-104`), and the params stop before the fields
    /// exist at all. A declared-but-unreachable notifier is
    /// `Ok(Some(ErrorNotifier::Unreachable))`, which is a different finding and stays
    /// one.
    ///
    /// # ★★★ Why a SHORT buffer is `Ok(None)` here and a refusal one field up
    ///
    /// `crate::versions::DriverAbiTable::decode_channel_alloc_facts` refuses a short
    /// message by name, and must: absence there would be read as `hVASpace = 0`, which is
    /// a **legal declaration** ("GSP-managed VAS"), so zero-extending would forge a
    /// meaningful statement the guest never made.
    ///
    /// This field has the opposite shape. *"There is no notifier"* and *"we could not
    /// read whether there is one"* already collapse into the **same conservative
    /// outcome** — `kayfabe_core::fault::verdict` refuses to emit either way — so nothing
    /// is forged by merging them. And the alternative was **measured** to be worse
    /// (`cargo test --workspace`, 2026-08-01): making the decode mandatory turned a legal
    /// short channel alloc into `AbiError::Truncated { need: 268 }` and failed nine tests
    /// in `tests/tests/rmrpc_bridge.rs` that allocate channels having nothing to do with
    /// fault reporting. A field added past a version-agreement prefix must be additive, or
    /// it is not an addition.
    ///
    /// ⊘ **Nothing here trusts a base address.** The value is the guest's, and this
    /// decode does not check it against any region — the write port
    /// (`kayfabe_gsp::GuestRam`) is the thing that refuses an address outside guest RAM,
    /// by name, and a second bounds opinion here would be a second source of truth for
    /// where guest RAM is.
    ///
    /// # Errors
    ///
    /// [`AbiError`] only from the primitive readers, which the length check above makes
    /// unreachable; kept as a `Result` so they stay total functions rather than panicking
    /// ones.
    pub fn decode(&self, bytes: &[u8]) -> Result<Option<ErrorNotifier>, AbiError> {
        if bytes.len() < self.needs() {
            return Ok(None);
        }
        let internal = u32_at(bytes, self.internal_flags)?;
        let kind = (internal >> NOTIFIER_TYPE_SHIFT) & NOTIFIER_TYPE_MASK;
        match kind {
            NOTIFIER_TYPE_NONE | NOTIFIER_TYPE_UNKNOWN => return Ok(None),
            NOTIFIER_TYPE_CTXDMA | NOTIFIER_TYPE_MEMORY => {}
            // The field is two bits and all four values are named above; the arm exists
            // so the match is total without a wildcard that could absorb a future value.
            _ => return Ok(None),
        }
        let base = u64_at(bytes, self.error_notifier_mem)?;
        let size = u64_at(bytes, self.error_notifier_mem + 8)?;
        let aperture = u32_at(bytes, self.error_notifier_mem + 16)?;
        // A notifier smaller than one record is not one. RM sizes it as
        // `pErrContextMemDesc->Size - errorContextOffset` (`kernel_channel.c:561-563`),
        // so a short value means the guest's own resolution disagrees with itself.
        if aperture != ADDR_SYSMEM || size < NOTIFICATION_SIZE as u64 {
            return Ok(Some(ErrorNotifier::Unreachable));
        }
        Ok(Some(ErrorNotifier::Sysmem { gpa: base }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Params with a sysmem notifier of `kind`, for a decode at `wire`.
    fn params(wire: ChannelNotifierWire, kind: u32, base: u64, size: u64, addr: u32) -> Vec<u8> {
        let mut b = vec![0u8; wire.needs()];
        b[wire.internal_flags..wire.internal_flags + 4]
            .copy_from_slice(&(kind << NOTIFIER_TYPE_SHIFT).to_le_bytes());
        let m = wire.error_notifier_mem;
        b[m..m + 8].copy_from_slice(&base.to_le_bytes());
        b[m + 8..m + 16].copy_from_slice(&size.to_le_bytes());
        b[m + 16..m + 20].copy_from_slice(&addr.to_le_bytes());
        b
    }

    /// The three fields land where BOTH vendored typedefs put them, and the status is
    /// last. Offsets are literals on purpose: this test is the transcription check, so
    /// reading them back out of the encoder would assert nothing.
    #[test]
    fn the_record_matches_nvnotification_and_notification_alike() {
        let n = ErrorNotification::rc(31, 1, 0x1122_3344_5566_7788);
        let b = n.encode();
        assert_eq!(b.len(), 16, "sizeof(NvNotification)");
        assert_eq!(
            u64::from_le_bytes(b[0..8].try_into().expect("8")),
            0x1122_3344_5566_7788
        );
        assert_eq!(
            u32::from_le_bytes(b[8..12].try_into().expect("4")),
            31,
            "info32 == exceptType == ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT"
        );
        assert_eq!(u16::from_le_bytes(b[12..14].try_into().expect("2")), 1);
        assert_eq!(
            u16::from_le_bytes(b[14..16].try_into().expect("2")),
            0xffff,
            "the sentinel krcErrorSetNotifier_IMPL passes"
        );
        assert_eq!(
            ErrorNotification::PUBLISH_SPLIT,
            14,
            "status is published LAST"
        );
    }

    /// ★ The status is the ONLY thing a `uvm_channel_get_status`-shaped reader tests, so a
    /// record whose status stayed zero is a record that says nothing happened.
    #[test]
    fn a_zero_status_reads_as_no_error() {
        let mut n = ErrorNotification::rc(31, 1, 0);
        n.status = 0;
        let b = n.encode();
        assert_eq!(u16::from_le_bytes(b[14..16].try_into().expect("2")), 0);
    }

    /// Both vendored boundaries decode the same declaration, eight bytes apart.
    #[test]
    fn both_pinned_boundaries_find_the_same_notifier() {
        for wire in [ChannelNotifierWire::V580, ChannelNotifierWire::V610] {
            let p = params(wire, NOTIFIER_TYPE_CTXDMA, 0xdead_0000, 64, ADDR_SYSMEM);
            assert_eq!(
                wire.decode(&p),
                Ok(Some(ErrorNotifier::Sysmem { gpa: 0xdead_0000 }))
            );
        }
        assert_eq!(
            ChannelNotifierWire::V610.internal_flags - ChannelNotifierWire::V580.internal_flags,
            8,
            "hHandleVASpace plus its re-alignment"
        );
    }

    /// The four `ErrorNotifierType` values, each to its own outcome.
    #[test]
    fn the_notifier_type_decides_before_the_address_does() {
        let w = ChannelNotifierWire::V580;
        for kind in [NOTIFIER_TYPE_NONE, NOTIFIER_TYPE_UNKNOWN] {
            let p = params(w, kind, 0x4000, 64, ADDR_SYSMEM);
            assert_eq!(w.decode(&p), Ok(None), "kind {kind} declares no notifier");
        }
        for kind in [NOTIFIER_TYPE_CTXDMA, NOTIFIER_TYPE_MEMORY] {
            let p = params(w, kind, 0x4000, 64, ADDR_SYSMEM);
            assert_eq!(
                w.decode(&p),
                Ok(Some(ErrorNotifier::Sysmem { gpa: 0x4000 })),
                "both notifier kinds share one record layout"
            );
        }
    }

    /// A notifier we have no write port for is DECLARED unreachable, never dropped.
    #[test]
    fn a_non_sysmem_or_short_notifier_is_unreachable_not_absent() {
        let w = ChannelNotifierWire::V580;
        // NV_ADDR_FBMEM
        let fb = params(w, NOTIFIER_TYPE_MEMORY, 0x4000, 64, 2);
        assert_eq!(w.decode(&fb), Ok(Some(ErrorNotifier::Unreachable)));
        // sysmem, but smaller than one record
        let short = params(w, NOTIFIER_TYPE_MEMORY, 0x4000, 8, ADDR_SYSMEM);
        assert_eq!(w.decode(&short), Ok(Some(ErrorNotifier::Unreachable)));
    }

    /// ★★ Short params decode to "no notifier" and **do not refuse the allocation**.
    ///
    /// The regression this pins was **measured** (`cargo test --workspace`, 2026-08-01):
    /// making the decode mandatory turned every short channel alloc into
    /// `AbiError::Truncated { c_name: "NV_CHANNEL_ALLOC_PARAMS", need: 268 }` and failed
    /// nine tests in `tests/tests/rmrpc_bridge.rs` that allocate perfectly legal channels.
    /// A notifier read past the version-agreement prefix must be additive; see the
    /// method's docs for why the same argument does **not** license zero-extending
    /// `hVASpace`.
    #[test]
    fn short_params_decline_the_notifier_without_refusing_the_channel() {
        let w = ChannelNotifierWire::V580;
        for len in [0, 32, w.needs() - 1] {
            assert_eq!(
                w.decode(&vec![0u8; len]),
                Ok(None),
                "{len} bytes: no notifier learned, and no refusal manufactured"
            );
        }
        // …and one more byte is all it takes to learn one, so the boundary is exact.
        let p = params(w, NOTIFIER_TYPE_MEMORY, 0x9000, 64, ADDR_SYSMEM);
        assert_eq!(p.len(), w.needs());
        assert_eq!(
            w.decode(&p),
            Ok(Some(ErrorNotifier::Sysmem { gpa: 0x9000 }))
        );
    }
}
