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

use kayfabe_arch::UserdMem;
use kayfabe_arch::fault::ErrorNotifier;
use kayfabe_arch::ids::EngineKind;

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

/// ★★★★ **What a channel declared about its USERD** — §16.16, the canary object.
///
/// # ⊘ Why USERD and not the ring, when the ring is what hangs
///
/// `[measured 2026-08-09, boot `res1_fc21926`]` the framebuffer page the guest's page
/// tables name for its GPFIFO ring is `resN-NEVER-WRITTEN`, and all 64 scanned entries read
/// zero. ★ **An all-zero ring is an AMBIGUOUS null**: empty, unwritten, or read at the
/// wrong address all produce it, and they need opposite fixes.
///
/// USERD converts that into a **discriminating** null. It comes out of the *same* alloc
/// params as the ring, carries its *own* declared aperture, is small, and — decisively —
/// has **recognisable content**: a live channel's `GP_GET`/`GP_PUT` are small integers
/// bounded by the ring's entry count. A live channel whose USERD reads all-zero forever is
/// almost certainly a **wrong address**, because there is no state in which a running
/// channel's cursors are both zero and stay that way.
///
/// ⇒ USERD sane + ring wrong ⇒ we mis-locate **this one object**. USERD also wrong ⇒ we
/// mis-locate **channel structures systematically**, which is a larger and better finding.
///
/// # ⚠ `userd_offset` is a documented SILENT-STALL mechanism
///
/// `crate::submit::ChannelAllocParams::userd_offset_0` records it: a non-zero
/// `userdOffset[0]` that the consumer ignores makes hardware see `GP_PUT == GP_GET`
/// **forever, with no error reported anywhere**. So the offset is carried and printed
/// beside the handle rather than assumed zero.
///
/// # The version seam, and it is the SAME one
///
/// Both fields sit past [`crate::versions::CHANNEL_ALLOC_PREFIX`], in the region 610 moves
/// by eight bytes. This type is how a *read* tree buys the right to read them, exactly as
/// [`ChannelNotifierWire`] does; a boundary whose tree nobody opened carries `None` rather
/// than a guess. ⊘ Blindly reading `+32` would decode 610's `hHandleVASpace` as a USERD
/// handle — a plausible non-zero number that is not a USERD at all, which is the worst
/// possible failure for a canary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelUserdWire {
    /// Offset of `NvHandle hUserdMemory[NV_MAX_SUBDEVICES]`; subdevice 0 is at this offset.
    pub h_userd_memory: usize,
    /// Offset of `NvU64 userdOffset[NV_MAX_SUBDEVICES]`; subdevice 0 is at this offset.
    pub userd_offset: usize,
}

impl ChannelUserdWire {
    /// 580.159.04 — the bench driver. `hUserdMemory[0]` @ +32, `userdOffset[0]` @ +64, from
    /// `ogkm-580: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-342` with
    /// `NV_MAX_SUBDEVICES = 8` (`ogkm-580: src/common/sdk/nvidia/inc/nvlimits.h:42`):
    /// eight `NvHandle` fill +32..+64, eight 8-aligned `NvU64` fill +64..+128, and
    /// `engineType` follows at +128 — the offset `crate::view::ChannelAllocFacts`' own map
    /// records for 580 and which the C artifact independently measured
    /// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:6760`). ★ Two independent
    /// derivations landing on +128 is what pins the two offsets above it.
    pub const V580: Self = Self {
        h_userd_memory: 32,
        userd_offset: 64,
    };

    /// 610.43.02 — eight bytes later, for [`ChannelNotifierWire::V610`]'s reason:
    /// `hHandleVASpace` @ +32 is four bytes and forces four more of re-alignment onto the
    /// 8-aligned `userdOffset[]`. `engineType` lands at +136, which is the arithmetic's
    /// own check.
    pub const V610: Self = Self {
        h_userd_memory: 36,
        userd_offset: 72,
    };

    /// The bytes a decode needs: through `userdOffset[0]`, the last field it reads.
    #[must_use]
    pub const fn needs(&self) -> usize {
        self.userd_offset + 8
    }

    /// Decode subdevice 0's declared USERD handle and offset.
    ///
    /// `Ok(None)` when the params stop before the fields exist — **additive**, for
    /// [`ChannelNotifierWire::decode`]'s measured reason: making a past-prefix decode
    /// mandatory turned legal short channel allocs into `AbiError::Truncated` and failed
    /// nine unrelated tests. A field added past a version-agreement prefix must be
    /// additive or it is not an addition.
    ///
    /// ⊘ A declared handle of **zero is carried as `Some(0)`**, not folded to `None`: RM
    /// treats zero as *"no USERD object, RM allocates it for me"*, which is a **statement
    /// the guest made**, while `None` here means *"we could not read whether it made
    /// one"*. Collapsing them would turn our ignorance into the guest's declaration — the
    /// same error class as decoding an empty capture to zeros.
    ///
    /// # Errors
    /// [`AbiError`] only from the primitive readers, which the length check makes
    /// unreachable; kept as a `Result` so those stay total functions.
    pub fn decode(&self, bytes: &[u8]) -> Result<Option<(u32, u64)>, AbiError> {
        if bytes.len() < self.needs() {
            return Ok(None);
        }
        Ok(Some((
            u32_at(bytes, self.h_userd_memory)?,
            u64_at(bytes, self.userd_offset)?,
        )))
    }
}

/// ★★★★★ **WHERE THE GUEST'S OWN CPU-RM SAID ITS USERD PHYSICALLY IS** —
/// `NV_CHANNEL_ALLOC_PARAMS.userdMem`, an `NV_MEMORY_DESC_PARAMS` the guest's kernel fills
/// in **for the GSP** and which therefore arrives at this port on every channel alloc.
///
/// # ⊘⊘ THE THREE-TIMES-REPEATED CLAIM THIS TYPE REFUTES
///
/// Three separate documents concluded, independently, that the guest's USERD address is
/// **unobtainable**, and all three named the same reason — *"USERD is named by
/// handle+offset (and, in the runlist entry, by raw physical address), never by a VA, so
/// the page-table walk that gave the ring its join source cannot be pointed at USERD"*
/// (`traces/boots/w262/RESULT.md` §5; `docs/design/leg_b_userd_adoption_blocker.md` §1;
/// `nvidia-gpu-passthrough/docs/design/userd_is_not_the_ring.md` §3).
///
/// **All three are looking at the wrong message.** The premise they share is that the only
/// USERD facts on the wire are [`ChannelUserdWire`]'s two — a handle in the *guest's* client
/// namespace, and an offset into it. That is true of the **client's** `NV_CHANNEL_ALLOC_PARAMS`
/// and false of the **RPC's**, and it is the RPC that reaches us:
///
/// > `pRpcParams->userdMem.base = memdescGetPhysAddr(pKernelChannel->pUserdSubDeviceMemDesc[subdevInst], AT_GPU, 0);`
/// > — `ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2747-2757`
///
/// `_kchannelSendChannelAllocRpc` builds a **fresh** `NV_CHANNEL_ALLOC_PARAMS` and resolves
/// the client's `hUserdMemory[0]`/`userdOffset[0]` **locally, CPU-side, before the RPC**,
/// because GSP has no access to a client handle namespace. So the guest hands us the answer
/// in the same buffer we already decode twice.
///
/// ★★★ **And `userdOffset` is ALREADY APPLIED.** `pUserdSubDeviceMemDesc` is a *sub*-memdesc
/// created at the offset — `memdescCreateSubMem(&pUserdSubMemDesc, pUserdMemDescForSubDev,
/// pGpu, userdOffset, userdSize)`, `ogkm-580: kernel_channel_gv100.c:234-237` — and
/// [`Self::decode`] reads `memdescGetPhysAddr(..., 0)` of *that*. ⇒ [`UserdMem::base`] is the
/// address of **this channel's own 512-byte USERD slot**, not of the object containing it.
/// Nothing has to be added to it and nothing may be.
///
/// # The offsets, and they are pinned by two fields this tree already reads
///
/// `NV_MEMORY_DESC_PARAMS` is 24 bytes (`base`, `size`, `addressSpace`, `cacheAttrib` —
/// `ogkm-580: alloc_channel.h:37-42`) and the four descriptors run
/// `instanceMem`, `userdMem`, `ramfcMem`, `mthdbufMem` from +144 (580).
/// ⊘ That is **not** an independent derivation to be trusted on its own: it is checked
/// against [`ChannelNotifierWire::V580`]'s already-pinned `internal_flags: 244`, which sits
/// exactly `24 * 3 + 4` past `userdMem`. `userd_mem_is_where_the_notifier_wire_says_it_is`
/// refuses to let the two drift.
///
/// # ⚠ What this type does NOT establish
///
/// The address is the **guest's** and this decode validates nothing about it. It is a
/// framebuffer offset in an *emulated* framebuffer, so it means nothing to host RM; the only
/// legitimate consumer is one that already holds a host object over that framebuffer range
/// and can express the address as an offset **into that object**. A consumer that hands the
/// number to hardware unqualified has handed hardware a guest-controlled physical address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelUserdMemWire {
    /// Offset of the `NV_MEMORY_DESC_PARAMS userdMem` sub-struct. `base` is at +0 within
    /// it, `size` at +8, `addressSpace` at +16, `cacheAttrib` at +20.
    pub userd_mem: usize,
}

impl ChannelUserdMemWire {
    /// 580.159.04 — the bench driver. `userdMem` @ **+168**.
    ///
    /// From `ogkm-580: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-342`:
    /// `engineType` @ +128 (which [`ChannelUserdWire::V580`] derives and the C artifact
    /// measured), then `cid` +132, `subDeviceId` +136, `hObjectEccError` +140,
    /// `instanceMem` +144, **`userdMem` +168**, `ramfcMem` +192, `mthdbufMem` +216,
    /// `hPhysChannelGroup` +240, `internalFlags` +244.
    ///
    /// ★ The last of those is [`ChannelNotifierWire::V580::internal_flags`], already pinned
    /// in this file at **244**, so this offset is not a fresh derivation — it is `244 - 76`.
    pub const V580: Self = Self { userd_mem: 168 };

    /// 610.43.02 — eight bytes later, for [`ChannelNotifierWire::V610`]'s reason, and
    /// checked the same way against its `internal_flags: 252`.
    pub const V610: Self = Self { userd_mem: 176 };

    /// The bytes a decode needs: through `addressSpace`, the last field it reads.
    ///
    /// ⊘ `cacheAttrib` follows and is deliberately not read. It describes how the guest's
    /// **CPU** should map its own USERD; the only consumer here hands the range to host RM
    /// as part of an object whose cache attribute the host driver decides.
    #[must_use]
    pub const fn needs(&self) -> usize {
        self.userd_mem + 20
    }

    /// Decode the channel's resolved USERD descriptor.
    ///
    /// `Ok(None)` when the params stop before the fields exist — **additive**, for
    /// [`ChannelNotifierWire::decode`]'s measured reason. ⊘ It is never
    /// [`UserdMem::Undeclared`]: *"the params were too short to say"* and *"the params said
    /// zero"* are the `dlen=0` distinction, and this tree has paid for collapsing it.
    ///
    /// # Errors
    /// [`AbiError`] only from the primitive readers, which the length check makes
    /// unreachable.
    pub fn decode(&self, bytes: &[u8]) -> Result<Option<UserdMem>, AbiError> {
        if bytes.len() < self.needs() {
            return Ok(None);
        }
        let base = u64_at(bytes, self.userd_mem)?;
        let size = u64_at(bytes, self.userd_mem + 8)?;
        let address_space = u32_at(bytes, self.userd_mem + 16)?;
        Ok(Some(match address_space {
            crate::fmbpromote::ADDR_FBMEM => UserdMem::Framebuffer { base, size },
            crate::fmbpromote::ADDR_SYSMEM => UserdMem::Sysmem { base, size },
            _ => UserdMem::Undeclared { address_space },
        }))
    }
}

/// ★★★★★ **WHICH ENGINE THE GUEST SAID THIS CHANNEL IS FOR** —
/// `NV_CHANNEL_ALLOC_PARAMS.engineType`, the one wire field that separates a GR channel
/// from a CE channel.
///
/// # ⊘ The framing this type corrects (2026-08-11)
///
/// `kayfabe_chips::ga10x`'s `classify` calls the engine *"a **params** fact
/// `RmEvent::Alloc` has nowhere to carry"*. That sentence is true and **incomplete in the
/// direction that decides the work**: the value does not merely lack a home in the core —
/// it is **never decoded off the wire at all**. `DriverAbiTable::decode_channel_alloc_facts`
/// stops at [`crate::versions::CHANNEL_ALLOC_PREFIX`] = 32, and `engineType` sits at **+128
/// (580)** / **+136 (610)**, i.e. deep in the region the two vendored trees disagree about
/// and which `crate::view::ChannelAllocFacts` has a standing ruling against reading blind.
///
/// ⇒ Giving `AllocFacts` a field would have carried **nothing**. The missing capability was
/// a *version-keyed decode*, which is what this type is — and the shape is not invented
/// here: it is exactly [`ChannelNotifierWire`]'s and [`ChannelUserdWire`]'s, the two fields
/// that already bought the right to read past the prefix. A boundary whose tree nobody
/// opened carries `None` rather than a guess.
///
/// # ★ The offsets are already twice-derived in this tree, and this type is the third
///
/// [`ChannelUserdWire::V580`]'s own doc derives `engineType` @ **+128** from
/// `hUserdMemory[0]` @ +32 and `userdOffset[0]` @ +64 with `NV_MAX_SUBDEVICES = 8`, and
/// notes the C artifact measured the same offset independently
/// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:6760`). [`ChannelUserdWire::V610`]
/// derives **+136** the same way. `crate::submit::ChannelAllocParams::encode_into` writes
/// the field at 128 on the 580 encoder. `engine_offsets_agree_with_the_userd_wire` below
/// keeps this type from drifting away from that arithmetic.
///
/// ⚠ **The INSTANCE is on the wire here and is dropped one layer up, deliberately.**
/// `engineType` is an `NV2080_ENGINE_TYPE_*` code, so a CE channel declares *which* copy
/// engine (`COPY0`..`COPY9`), not merely "a copy engine". [`Self::decode`] returns the raw
/// code so that fact survives this seam; `decode_declared_engine` narrows it to an
/// [`EngineKind`] for the core, which speaks a vocabulary rather than NVIDIA's numbers
/// (decision #2). See `kayfabe_isolate_host::rm::declared_copy_engine_type` for the one
/// consumer that needs the instance and gets it from the CE *object* instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelEngineWire {
    /// Offset of `NvU32 engineType`.
    pub engine_type: usize,
}

impl ChannelEngineWire {
    /// 580.159.04 — the bench driver. `engineType` @ +128, immediately after
    /// `userdOffset[NV_MAX_SUBDEVICES]` ends at +128
    /// (`ogkm-580: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-342`).
    pub const V580: Self = Self { engine_type: 128 };

    /// 610.43.02 — eight bytes later, for [`ChannelUserdWire::V610`]'s reason:
    /// `hHandleVASpace` @ +32 is four bytes and forces four more of re-alignment onto the
    /// 8-aligned `userdOffset[]`
    /// (`ogkm-610: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-347`).
    pub const V610: Self = Self { engine_type: 136 };

    /// The bytes a decode needs: through `engineType`, the only field it reads.
    #[must_use]
    pub const fn needs(&self) -> usize {
        self.engine_type + 4
    }

    /// Decode the declared `NV2080_ENGINE_TYPE_*` code, raw.
    ///
    /// `Ok(None)` when the params stop before the field exists — **additive**, for
    /// [`ChannelUserdWire::decode`]'s measured reason: making a past-prefix decode
    /// mandatory turns legal short channel allocs into `AbiError::Truncated`. A field added
    /// past a version-agreement prefix must be additive or it is not an addition.
    ///
    /// ⊘ Nothing is validated and nothing is folded. A code this port does not recognise
    /// arrives as itself; [`Self::decode_kind`] is where "we do not know what that is"
    /// becomes an answer, and it is `None` there rather than a guess.
    ///
    /// # Errors
    /// [`AbiError`] only from the primitive reader, which the length check makes
    /// unreachable; kept as a `Result` so that stays a total function.
    pub fn decode(&self, bytes: &[u8]) -> Result<Option<u32>, AbiError> {
        if bytes.len() < self.needs() {
            return Ok(None);
        }
        Ok(Some(u32_at(bytes, self.engine_type)?))
    }

    /// The declared code narrowed to the vocabulary the core speaks.
    ///
    /// ⚠ **`ENGINE_TYPE_GRAPHICS` answers [`EngineKind::GrCompute`], and that is a
    /// DELIBERATE under-determination rather than a reading of the wire.** GR runs both
    /// compute and 3D contexts on one runlist and `engineType` cannot tell them apart —
    /// only the engine *object* can (`AMPERE_COMPUTE_B` vs `AMPERE_B`). The consumer in
    /// `kayfabe_core::project` therefore lets the engine-object refinement pick **within**
    /// GR while the declaration decides **which engine**, which is the only split under
    /// which the declaration cannot lose information it does not have.
    ///
    /// ⊘ `None` = *this port does not recognise the code*, never a default. The two engines
    /// this port has never allocated a channel on (`NVENC`/`NVDEC`) land here with
    /// everything else, because answering [`EngineKind::Other`] would be a claim about the
    /// guest's declaration rather than about our ignorance of it.
    ///
    /// # Errors
    /// [`AbiError`] from [`Self::decode`].
    pub fn decode_kind(&self, bytes: &[u8]) -> Result<Option<EngineKind>, AbiError> {
        Ok(self.decode(bytes)?.and_then(engine_kind_of_nv2080))
    }
}

/// `NV2080_ENGINE_TYPE_*` → the core's [`EngineKind`], or `None` for a code this port does
/// not recognise. The inverse direction is [`crate::rc::EngineRoute::for_engine`].
///
/// ⊘ Total on purpose, with no wildcard-to-`Other` arm: see
/// [`ChannelEngineWire::decode_kind`] for why an unrecognised code must not become a kind.
#[must_use]
pub fn engine_kind_of_nv2080(code: u32) -> Option<EngineKind> {
    if code == crate::submit::ENGINE_TYPE_GRAPHICS {
        return Some(EngineKind::GrCompute);
    }
    if crate::submit::copy_index_of_engine_type(code).is_some() {
        return Some(EngineKind::Ce);
    }
    None
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

#[cfg(test)]
mod engine_wire_tests {
    use super::*;
    use crate::submit::{ENGINE_TYPE_GRAPHICS, engine_type_copy};

    /// ★★★ **The transcription check, and it is a SECOND derivation rather than a
    /// restatement.**
    ///
    /// [`ChannelUserdWire`]'s doc derives `engineType` from the USERD layout:
    /// `userdOffset[0]` starts the `NvU64[NV_MAX_SUBDEVICES]` array, eight of them fill
    /// 64 bytes, and `engineType` follows. So `engine_type == userd_offset + 64` must hold
    /// on every boundary that pins both — and if a future version reorders the tail, this
    /// is the assertion that refuses to let the two wires drift apart silently.
    ///
    /// ⊘ It does not prove either offset is right. It proves they cannot disagree, which
    /// is the failure a hand-written second wire actually has.
    #[test]
    fn engine_offsets_agree_with_the_userd_wire() {
        assert_eq!(
            ChannelEngineWire::V580.engine_type,
            ChannelUserdWire::V580.userd_offset + 64,
            "580: engineType follows userdOffset[NV_MAX_SUBDEVICES=8]"
        );
        assert_eq!(
            ChannelEngineWire::V610.engine_type,
            ChannelUserdWire::V610.userd_offset + 64,
            "610: the same arithmetic, eight bytes later"
        );
        assert_eq!(
            ChannelEngineWire::V610.engine_type - ChannelEngineWire::V580.engine_type,
            8,
            "the 610 skew is `hHandleVASpace`'s 4 bytes plus 4 of re-alignment"
        );
    }

    /// ★★★ **`userdMem` is pinned by a field this tree ALREADY reads, not by a fresh count.**
    ///
    /// The tail of `NV_CHANNEL_ALLOC_PARAMS` runs `instanceMem`, `userdMem`, `ramfcMem`,
    /// `mthdbufMem` (24 bytes each), then `hPhysChannelGroup` (4) and `internalFlags` (4).
    /// So `internal_flags == userd_mem + 24*3 + 4` must hold, and
    /// [`ChannelNotifierWire`]'s `internal_flags` was pinned long before this wire existed
    /// and is exercised on the bench by the error-notifier decode.
    ///
    /// ⊘ It does not prove `userd_mem` is right — it proves it cannot disagree with a
    /// number that has already been booted, which is the failure a third hand-written
    /// offset actually has.
    #[test]
    fn userd_mem_is_where_the_notifier_wire_says_it_is() {
        assert_eq!(
            ChannelNotifierWire::V580.internal_flags,
            ChannelUserdMemWire::V580.userd_mem + 24 * 3 + 4,
            "580: userdMem, ramfcMem, mthdbufMem, hPhysChannelGroup, internalFlags"
        );
        assert_eq!(
            ChannelNotifierWire::V610.internal_flags,
            ChannelUserdMemWire::V610.userd_mem + 24 * 3 + 4,
            "610: the same tail, eight bytes later"
        );
        // And the head: `instanceMem` is the descriptor before `userdMem`, and it follows
        // `engineType`, `cid`, `subDeviceId`, `hObjectEccError` — four `NvU32`.
        assert_eq!(
            ChannelUserdMemWire::V580.userd_mem,
            ChannelEngineWire::V580.engine_type + 16 + 24,
            "580: engineType, cid, subDeviceId, hObjectEccError, instanceMem"
        );
        assert_eq!(
            ChannelUserdMemWire::V610.userd_mem,
            ChannelEngineWire::V610.engine_type + 16 + 24,
            "610: the same head"
        );
    }

    /// ⊘ The three readings a `userdMem` descriptor can have, and that a zero descriptor is
    /// [`UserdMem::Undeclared`] rather than a framebuffer address of zero — which is a real
    /// address in an emulated framebuffer and would resolve.
    #[test]
    fn a_userd_descriptor_carries_its_aperture_on_the_value() {
        let mut p = vec![0u8; ChannelUserdMemWire::V580.needs()];
        assert_eq!(
            ChannelUserdMemWire::V580.decode(&p),
            Ok(Some(UserdMem::Undeclared { address_space: 0 })),
            "an all-zero descriptor is UNDECLARED, never a framebuffer address of zero"
        );
        let at = ChannelUserdMemWire::V580.userd_mem;
        p[at..at + 8].copy_from_slice(&0x0102_3000u64.to_le_bytes());
        p[at + 8..at + 16].copy_from_slice(&512u64.to_le_bytes());
        p[at + 16..at + 20].copy_from_slice(&crate::fmbpromote::ADDR_FBMEM.to_le_bytes());
        assert_eq!(
            ChannelUserdMemWire::V580.decode(&p),
            Ok(Some(UserdMem::Framebuffer {
                base: 0x0102_3000,
                size: 512
            }))
        );
        p[at + 16..at + 20].copy_from_slice(&crate::fmbpromote::ADDR_SYSMEM.to_le_bytes());
        assert_eq!(
            ChannelUserdMemWire::V580.decode(&p),
            Ok(Some(UserdMem::Sysmem {
                base: 0x0102_3000,
                size: 512
            })),
            "guest RAM is a legal USERD location and must be refusable BY NAME"
        );
        // ⊘ Short params are UNKNOWN, and that is a different fact from Undeclared.
        assert_eq!(
            ChannelUserdMemWire::V580.decode(&p[..at + 4]),
            Ok(None),
            "too short to say is never `the guest said zero`"
        );
    }

    /// ⊘ **A short params block is `Ok(None)`, never `Truncated`** — the additive rule a
    /// past-prefix field must obey. `CHANNEL_ALLOC_PREFIX`-sized params are legal and must
    /// stay legal after a reader was written for a field past them.
    #[test]
    fn a_short_params_block_reads_as_unknown_and_never_as_an_error() {
        let short = vec![0u8; crate::versions::CHANNEL_ALLOC_PREFIX];
        assert_eq!(ChannelEngineWire::V580.decode(&short), Ok(None));
        assert_eq!(ChannelEngineWire::V580.decode_kind(&short), Ok(None));
    }

    /// The kinds this port recognises, and — the half that matters — the ones it refuses
    /// to invent. A code we do not know is `None`, never `EngineKind::Other`: `Other` is a
    /// claim about the guest's declaration, `None` is a statement about our reading of it.
    #[test]
    fn an_unrecognised_engine_code_is_unknown_and_never_other() {
        assert_eq!(
            engine_kind_of_nv2080(ENGINE_TYPE_GRAPHICS),
            Some(EngineKind::GrCompute)
        );
        for i in 0..10 {
            assert_eq!(
                engine_kind_of_nv2080(engine_type_copy(i).expect("i < 10")),
                Some(EngineKind::Ce),
                "COPY{i} is a copy engine"
            );
        }
        // `0x13` is NVDEC0 — inside the numeric gap between the two COPY blocks, which is
        // exactly where a range check written as `>= COPY0` would wrongly say "copy".
        assert_eq!(engine_kind_of_nv2080(0x13), None, "NVDEC0 is not a CE");
        assert_eq!(engine_kind_of_nv2080(0), None, "and zero is not GRAPHICS");
    }

    /// Round-trip through a params block the ENCODER built, so the decoder is checked
    /// against this crate's own statement of the layout rather than against itself.
    #[test]
    fn the_decoder_reads_back_what_the_580_encoder_wrote() {
        let mut params = vec![0u8; ChannelEngineWire::V580.needs()];
        let declared = engine_type_copy(2).expect("COPY2");
        let n = params.len();
        params[128..132].copy_from_slice(&declared.to_le_bytes());
        assert_eq!(n, 132, "the 580 field ends the block this test builds");
        assert_eq!(ChannelEngineWire::V580.decode(&params), Ok(Some(declared)));
        assert_eq!(
            ChannelEngineWire::V580.decode_kind(&params),
            Ok(Some(EngineKind::Ce))
        );
        // ⊘ The 610 wire reads FOUR BYTES PAST this 580 block and must say so rather than
        // decode the tail it can reach.
        assert_eq!(ChannelEngineWire::V610.decode(&params), Ok(None));
    }
}
