//! ★★★ **The DEVICE_INFO2 table** — `NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE`
//! (`0x20800a40`), and the first control this port answers by **deriving** rather than by
//! stating.
//!
//! ## ★★★ This is the SECOND statement of a fact the port already makes
//!
//! `kayfabe_device::ChipProfile::engines` — the `&'static [FifoDeviceEntry]` served through
//! [`crate::inittables::NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE`] (`0x20801112`) — is
//! this device's engine inventory. This control asks for **the same silicon in a different
//! wire format**: twelve `NvU32` per device instead of an `engineData[16]` array, a subset
//! of the rows, and one field the FIFO table does not carry.
//!
//! A second, independently-written table is the drift hazard this codebase refuses
//! everywhere else (`ChipProfile::fb_length`'s doc calls its own case *"the third statement
//! of one fact"*). So there is **no** device-info table here. There is a *derivation*:
//! [`encode_internal_device_info_table`] takes the engine slice and projects it. The row
//! type this module defines carries exactly one thing the projection cannot compute, and
//! nothing else — see [`DeviceInfoRow`].
//!
//! ⊘ And there is deliberately **no public per-entry struct**. If this module exported a
//! twelve-field `DeviceInfo` with public fields, a chip row could be written that names its
//! engines a second time — which is the thing the derivation exists to prevent. The only
//! way to produce these bytes is to hand the encoder the FIFO table.
//!
//! ## The projection, field by field
//!
//! `[measured]` every mapping below was checked equal to the oracle's captured bytes for
//! all five rows this device serves — the C's `ctl_20800a40`
//! (`C: src/qemu/mode2_initctrl_ga106.h:4028`), itself a replay of a real RTX 3060's GSP
//! reply. `tests/internal_device_info_table.rs` re-checks it against those bytes.
//!
//! | wire field | source |
//! |---|---|
//! | `faultId` | `engineData[`[`MMU_FAULT_ID`](engine_info_type::MMU_FAULT_ID)`]` |
//! | `instanceId` | `engineData[`[`INSTANCE_ID`](engine_info_type::INSTANCE_ID)`]` |
//! | `typeEnum` | `engineData[`[`DEV_TYPE_ENUM`](engine_info_type::DEV_TYPE_ENUM)`]` |
//! | `resetId` | `engineData[`[`RESET`](engine_info_type::RESET)`]` |
//! | `devicePriBase` | ⚠ [`DeviceInfoRow`] — the one field with no FIFO source |
//! | `isEngine` | `engineData[`[`IS_HOST_DRIVEN_ENGINE`](engine_info_type::IS_HOST_DRIVEN_ENGINE)`]` |
//! | `rlEngId` | `engineData[`[`RUNLIST_ENGINE_ID`](engine_info_type::RUNLIST_ENGINE_ID)`]` |
//! | `runlistPriBase` | `engineData[`[`RUNLIST_PRI_BASE`](engine_info_type::RUNLIST_PRI_BASE)`]` |
//! | `groupLocalInstanceId` | the same slot as `instanceId` |
//! | `groupId`, `ginTargetId`, `deviceBroadcastPriBase` | zero |
//!
//! `[measured]` the last two rows are not guesses about what those fields *mean*: in **all
//! twelve** of the oracle's rows `groupLocalInstanceId == instanceId`, and `groupId`,
//! `ginTargetId` and `deviceBroadcastPriBase` are zero. ⊘ That is a statement about a GA106
//! with one GPC group and no GIN, not about the fields in general — a chip that partitions
//! (GA100 with MIG, or any Blackwell) would need a real source for them, and this module
//! would have to grow one rather than keep zeroing.
//!
//! ## ★★ `devicePriBase` is the one fact the FIFO table does not carry
//!
//! `engineData[]` has sixteen slots and none of them is the engine's own PRI block base:
//! `RUNLIST_PRI_BASE` is the *runlist's* base and `CHRAM_PRI_BASE` is the channel-RAM's
//! (`ogkm-580: src/nvidia/inc/kernel/gpu/fifo/engine_info.h:37-104`). So the derivation is
//! total except here, and [`DeviceInfoRow`] is exactly that hole and no larger.
//!
//! ## ★★ Why RM's pseudo-engine must not appear
//!
//! `kayfabe_device::ga10x::GA106_ENGINES` carries a row named `SOFTWARE`. It is not
//! silicon: it is RM's own pseudo-engine, the one row with `IS_HOST_DRIVEN_ENGINE == 0`,
//! and the row `kfifoGetHostDeviceInfoTable_KERNEL` special-cases when it reserves PBDMA
//! fault ids (`ogkm-580: kernel_fifo.c:2160-2166`). It has no PRI block, and the oracle's
//! DEVICE_INFO2 table has no row for it.
//!
//! ★ It is excluded by a **marking on the row type**, [`DevicePriBase::NotADevice`], not by
//! a name check. A name check answers *"is this row called SOFTWARE"*; the marking answers
//! *"does this row name a device with a PRI block"*, which is the question the reply is
//! actually about — and it generalises to whatever a second chip's FIFO table carries.
//!
//! ⚠ **And the excluded row is where the noise lives.** `[measured]` `GA106_ENGINES`'
//! `SOFTWARE` row is capture noise from `RESET` onward — `faultId = 0xffffffff`,
//! `RESET = 0x82300810`, `RUNLIST_PRI_BASE = 0x77f2058f`. A derivation that emitted it
//! would put `0x77f2058f` into a field RM files as a register base.
//!
//! Three separate things stop it, and they were designed as three because the exclusion is
//! the only one that depends on a chip row being right:
//!
//! 1. [`DevicePriBase::NotADevice`] drops the row. The chip states it.
//! 2. [`DeviceInfoError::PriBaseAbsentForHostDrivenEngine`] stops that marking being put on
//!    real silicon, so the exclusion cannot quietly grow.
//! 3. Even if the row were marked a device, `RUNLIST_PRI_BASE` and `RUNLIST_ENGINE_ID` are
//!    *"Valid only for Esched-driven engines"* (`ogkm-580: engine_info.h:70-90`), and this
//!    row is not one — the projection emits **zero** for both. And for a row that *is*
//!    host-driven, `0x77f2058f` is refused outright: it is neither
//!    [`PRI_REGISTER_ALIGN`]-aligned ([`DeviceInfoError::PriBaseNotRegisterAligned`]) nor
//!    inside a 16 MiB BAR0 ([`DeviceInfoError::PriBaseOutsideAperture`]).
//!
//! ## ★★★ Which rows this port serves, and the six it overrules the oracle on
//!
//! The oracle answers **twelve** entries. This port serves **five** — `GR0` and `CE0..CE3`,
//! exactly what `GA106_ENGINES` already advertises — and it is the engine list, not this
//! module, that decides:
//!
//! | oracle row | what it is | why it is absent here |
//! |---|---|---|
//! | 1, 2, 3, 9 | NVDEC0, NVENC0, OFA, SEC2-class | `GA106_ENGINES` omits them: no plane in this port drives one, and an engine we advertise is an engine RM goes on to **use** |
//! | 8 | `CE4` | the C's own wire table drops it — the host's `GET_ENGINES_V2` exposes only `CE0..CE3` |
//! | 10, 11 | `isEngine = 0` devices (`typeEnum` `0x14`, `0x17`) | ⊘ **not expressible here at all**: they are not in the FIFO engine table, and this module can only project rows that table already carries |
//!
//! ⊘ The last line is a real limit of the derivation and is recorded as one. The trade is
//! deliberate: a device this port could state *only* here would be a fact with no second
//! witness, which is the shape the derivation exists to forbid. If a guest is ever measured
//! to need one, the honest fix is a source both replies read — not a hand-written row.
//!
//! ## ★★★ The guest's only reader, and what it does with the table
//!
//! `gpuConstructDeviceInfoTable_FWCLIENT` copies the reply into `pGpu->pDeviceInfoTable`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_gspclient.c:205-265`). In the whole vendored
//! open tree exactly one consumer then reads it:
//!
//! ```c
//! for (i = 0; i < pGpu->numDeviceInfoEntries; i++)
//!     if (pGpu->pDeviceInfoTable[i].typeEnum == NV_PTOP_ZB_DEVICE_INFO_DEV_TYPE_ENUM_LCE)
//!     {
//!         minMmuFaultId = NV_MIN(minMmuFaultId, pGpu->pDeviceInfoTable[i].faultId);
//!         maxMmuFaultId = NV_MAX(maxMmuFaultId, pGpu->pDeviceInfoTable[i].faultId);
//!     }
//! ```
//!
//! (`kgmmuInitCeMmuFaultIdRange_GA100`,
//! `ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/ampere/kern_gmmu_ga100.c:255-296`; it is
//! the HAL GA106 binds — `ogkm-580: src/nvidia/generated/g_kern_gmmu_nvoc.c:1272-1276`
//! lists `GA106` explicitly — and it is reached under `NV_ASSERT_OK_OR_RETURN` from
//! `kgmmuStateInitLocked_IMPL`, `ogkm-580: kern_gmmu.c:339`.)
//!
//! Two consequences, and both are refusals below:
//!
//! - **A table with no `LCE` row ends the boot.** `min` stays `(NvU32)-1`, the function
//!   logs *"Failed to find any MMU Fault ID"*, fires `NV_ASSERT(0)` and returns
//!   `NV_ERR_OBJECT_NOT_FOUND` (`:281-287`). That covers the empty table too — which
//!   `gpuConstructDeviceInfoTable_FWCLIENT` itself accepts with `NV_OK` at `:231-232`, so
//!   the failure surfaces two engines later with nothing pointing here.
//!   [`DeviceInfoError::NoCopyEngineAdvertised`].
//! - **The set is reduced to a RANGE, and membership then means identity.**
//!   `bEngineCE = (engineId >= minCeMmuFaultId && engineId <= maxCeMmuFaultId)`
//!   (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:739-740`),
//!   which decides whether a fault is RM-serviceable or UVM's. A gap in the advertised
//!   fault ids is an id the guest will attribute to a copy engine we did not advertise.
//!   [`DeviceInfoError::CopyEngineFaultIdsNotContiguous`].
//!
//! ## ★★★ The fail-open the ORACLE ITSELF is an instance of
//!
//! `[measured]` the C registers this reply as
//! `{0x20800a40u, 0x0u, 24580u, 16384u, ctl_20800a40}` — a declared `paramsSize` of
//! **24580** against **16384** bytes of data, 8196 bytes short. RM copies back exactly
//! `paramsSize` bytes (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c` — `portMemCopy(…,
//! paramsSize, …, paramsSize)`), and unlike the falcon inventory's caller **this one does
//! not zero its buffer first**: `portMemAllocNonPaged(sizeof *pParams)` at
//! `ogkm-580: gpu_gspclient.c:219`, no `portMemSet`. So the 8196 bytes past the C's data
//! are whatever the guest kernel heap held.
//!
//! `4 + 341 * 48 = 16372`, so of the 512-entry array the C's reply covers entries `0..=340`
//! in full, delivers 12 bytes of entry 341, and does not reach entries `342..=511` at all.
//! It is harmless **only** because `numEntries = 12` — the count happens to land far inside
//! the delivered prefix. Had the C ever answered a count above 341, RM would have filed
//! entries out of uninitialised heap, each one a `faultId`, a `typeEnum` and a
//! `devicePriBase`, and `NV_ASSERT_OR_RETURN(numEntries <= 512)`
//! (`ogkm-580: gpu_gspclient.c:234`) would have passed every one of them.
//!
//! ★★ That is *"a count bound that outruns its buffer"*: the bound RM checks is against the
//! **array's extent**, and the thing that actually limits what arrived is the **reply's
//! length**, which nothing compares them to. It is the same shape as
//! [`crate::falconinfo::FalconInfoError::TooManyFalcons`] with the two roles swapped —
//! there the count outran the source table inside a correctly sized buffer, here it would
//! outrun the buffer itself.
//!
//! [`encode_internal_device_info_table`] makes it unencodable by construction, in two
//! halves that must both hold:
//!
//! 1. It **always** returns all [`INTERNAL_DEVICE_INFO_PARAMS_SIZE`] bytes, zero-filled
//!    past the last entry. There is no short reply to declare a long count against.
//! 2. The count is **derived from the projection**, never carried — [`DeviceInfoRow`] has
//!    no count field, for the reason [`crate::falconinfo::FalconInventoryRow`] has none.
//!
//! `[inferred]` the full 24580 bytes fit the transport: the reply is
//! `32 + 40 + 24580 = 24652` bytes against the bench's `queueElementSizeMax` of
//! `GSP_MSG_QUEUE_ELEMENT_SIZE_MIN * 16 = 65536` (`kayfabe_gsp::element`). ⊘ No boot has
//! carried this reply yet, so that is arithmetic, not a measurement.
//!
//! ## ⊘ Not `kayfabe_device::inert`-eligible, and here the bytes DO tell you
//!
//! Inert's rule is *"the guest reads nothing but the status"*. RM reads `numEntries`,
//! branches on it twice (`:231`, `:234`), allocates against it and loops to it. And unlike
//! [`crate::falconinfo`]'s caller — where an inert reply and a served all-zero reply are
//! literally the same bytes — this caller never zeroes its params, so an inert reply here
//! is not "an empty table", it is *whatever was on the heap*, with `numEntries` taken from
//! it.

use crate::inittables::FifoDeviceEntry;

/// `NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:766`).
pub const NV2080_CTRL_CMD_INTERNAL_GET_DEVICE_INFO_TABLE: u32 = 0x2080_0a40;

/// `NV2080_CTRL_CMD_INTERNAL_DEVICE_INFO_MAX_ENTRIES = 512`
/// (`ogkm-580: ctrl2080internal.h:762`) — the extent of `deviceInfoTable[]`, and the bound
/// RM checks the count against (`ogkm-580: gpu_gspclient.c:234`).
///
/// ⚠ Not to be confused with [`crate::inittables::DEVICE_INFO_MAX_ENTRIES`] (32), which is
/// the *other* device-info control's paging stride. The two replies describe the same
/// silicon and share nothing else.
pub const INTERNAL_DEVICE_INFO_MAX_ENTRIES: usize = 512;

/// `sizeof(NV2080_CTRL_INTERNAL_DEVICE_INFO)` — twelve `NvU32`, no padding
/// (`ogkm-580: ctrl2080internal.h:747-760`).
pub const INTERNAL_DEVICE_INFO_STRIDE: usize = 48;

/// Byte offset of `numEntries`.
pub const NUM_ENTRIES_OFF: usize = 0;

/// Byte offset of `deviceInfoTable[0]`.
pub const DEVICE_INFO_TABLE_OFF: usize = 4;

/// Byte offset of `faultId` within one entry.
pub const FAULT_ID_OFF: usize = 0;
/// Byte offset of `instanceId` within one entry.
pub const INSTANCE_ID_OFF: usize = 4;
/// Byte offset of `typeEnum` within one entry.
pub const TYPE_ENUM_OFF: usize = 8;
/// Byte offset of `resetId` within one entry.
pub const RESET_ID_OFF: usize = 12;
/// Byte offset of `devicePriBase` within one entry.
pub const DEVICE_PRI_BASE_OFF: usize = 16;
/// Byte offset of `isEngine` within one entry.
pub const IS_ENGINE_OFF: usize = 20;
/// Byte offset of `rlEngId` within one entry.
pub const RL_ENG_ID_OFF: usize = 24;
/// Byte offset of `runlistPriBase` within one entry.
pub const RUNLIST_PRI_BASE_OFF: usize = 28;
/// Byte offset of `groupId` within one entry.
pub const GROUP_ID_OFF: usize = 32;
/// Byte offset of `ginTargetId` within one entry.
pub const GIN_TARGET_ID_OFF: usize = 36;
/// Byte offset of `deviceBroadcastPriBase` within one entry.
pub const DEVICE_BROADCAST_PRI_BASE_OFF: usize = 40;
/// Byte offset of `groupLocalInstanceId` within one entry.
pub const GROUP_LOCAL_INSTANCE_ID_OFF: usize = 44;

/// `sizeof(NV2080_CTRL_INTERNAL_GET_DEVICE_INFO_TABLE_PARAMS)` — the `[OUT]` struct RM
/// allocates (`ogkm-580: gpu_gspclient.c:219`) and therefore the `paramsSize` it declares.
///
/// ★ `[measured]`, and the run is the **C artifact's, not this port's**: its reply table
/// registers `{0x20800a40u, 0x0u, 24580u, 16384u, ctl_20800a40}` — exactly this
/// `paramsSize` against 16384 bytes of data — at
/// `C: src/qemu/mode2_initctrl_ga106.h:6252`, `nvidia-gpu-passthrough` rev `018e492`, the
/// commit that last wrote those bytes. An inherited measurement, named as one; nothing
/// here was read off a machine by us. See this module's docs for why that gap is the
/// fail-open this encoder closes.
pub const INTERNAL_DEVICE_INFO_PARAMS_SIZE: usize =
    DEVICE_INFO_TABLE_OFF + INTERNAL_DEVICE_INFO_MAX_ENTRIES * INTERNAL_DEVICE_INFO_STRIDE;

/// `NV_PTOP_DEVICE_INFO2_DEV_TYPE_ENUM_LCE = 19`
/// (`ogkm-580: src/common/inc/swref/published/ampere/ga100/dev_top.h:34`, reached through
/// the `NV_PTOP_ZB_…` alias at `dev_top_addendum.h:33`).
///
/// The only `typeEnum` any consumer in the vendored open tree compares against — see this
/// module's docs.
pub const DEV_TYPE_ENUM_LCE: u32 = 19;

/// The granularity a PRI base is checked at: a `NvU32` register address.
///
/// ⚠ **Deliberately not 4 KiB.** [`crate::falconinfo::FALCON_REGISTER_BLOCK_LEN`] can check
/// a 4 KiB boundary because `[measured]` every falcon base the oracle reports is
/// `0x1000`-aligned — the C artifact's captured GA106 replies at
/// `C: src/qemu/mode2_initctrl_ga106.h`, `nvidia-gpu-passthrough` rev `018e492`, which is
/// an inherited measurement and not one this port took. Here it would refuse the oracle's
/// own bytes: `runlistPriBase` is
/// `0xc00400`, `0xc00800`, `0xc00c00` on `CE2`, `CE3`, `CE4`. So the only granularity this
/// module can honestly claim is the one that makes the value name a register at all.
pub const PRI_REGISTER_ALIGN: u32 = 4;

/// The `ENGINE_INFO_TYPE` slots **this reply's projection** reads.
///
/// `ogkm-580: src/nvidia/inc/kernel/gpu/fifo/engine_info.h:37-104`.
///
/// ★ Two of the seven already have a home in
/// [`crate::inittables::engine_info_type`] and are re-exported rather than respelled — a
/// second spelling of an index is the same drift this module's whole design is about. The
/// other five are new here because no other reply needed them.
pub mod engine_info_type {
    pub use crate::inittables::engine_info_type::{INSTANCE_ID, IS_HOST_DRIVEN_ENGINE};

    /// `ENGINE_INFO_TYPE_MMU_FAULT_ID` — `NV_PFIFO_INTR_MMU_FAULT_ENG_ID_*`. Becomes
    /// `faultId`, which is the field the guest reduces to a range.
    pub const MMU_FAULT_ID: usize = 4;
    /// `ENGINE_INFO_TYPE_RESET` — *"Reset Bit Position. On Ampere, only valid if not
    /// _INVALID"*. Becomes `resetId`.
    pub const RESET: usize = 6;
    /// `ENGINE_INFO_TYPE_DEV_TYPE_ENUM` — becomes `typeEnum`, compared against
    /// [`DEV_TYPE_ENUM_LCE`](super::DEV_TYPE_ENUM_LCE).
    pub const DEV_TYPE_ENUM: usize = 9;
    /// `ENGINE_INFO_TYPE_RUNLIST_PRI_BASE` — *"Valid only for Esched-driven engines"*,
    /// which is why the projection zeroes it on a row that is not one.
    pub const RUNLIST_PRI_BASE: usize = 11;
    /// `ENGINE_INFO_TYPE_RUNLIST_ENGINE_ID` — likewise *"Valid only for Esched-driven
    /// engines"*.
    pub const RUNLIST_ENGINE_ID: usize = 13;
}

/// ★★ **Where one engine's own PRI register block starts — the single fact the FIFO
/// device-info table does not carry.**
///
/// A two-armed enum rather than an `Option<u32>` because the absent arm is not *"unknown"*,
/// it is a **claim**: this row does not name a device with a PRI block, and must therefore
/// not appear in this reply at all. `Option::None` would read as an omission a future
/// author could reasonably fill in; [`DevicePriBase::NotADevice`] reads as the statement it
/// is, and [`DeviceInfoError::NoPriBaseForEngine`] keeps a genuine omission from being
/// spelled the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePriBase {
    /// This engine is a device, and its PRI block begins at this BAR0 offset.
    At(u32),
    /// ★ This row is **not a hardware device** and gets no entry in this reply.
    ///
    /// `GA106_ENGINES`' `SOFTWARE` row is the only one: RM's own pseudo-engine, the one
    /// entry with `IS_HOST_DRIVEN_ENGINE == 0`, special-cased by
    /// `kfifoGetHostDeviceInfoTable_KERNEL` (`ogkm-580: kernel_fifo.c:2160-2166`) and
    /// absent from the oracle's DEVICE_INFO2 table. See
    /// [`DeviceInfoError::PriBaseAbsentForHostDrivenEngine`] for what stops this marking
    /// being put on real silicon.
    NotADevice,
}

/// One engine's PRI base, keyed by the engine's name in the FIFO device-info table.
///
/// ★ Keyed by name, not by position. A position-parallel slice would let a reordering of
/// `ChipProfile::engines` silently re-attribute every base — the class of drift that has no
/// red test until a guest faults at an address that names nothing. A name that does not
/// match is [`DeviceInfoError::PriBaseForUnknownEngine`]; an engine with no match is
/// [`DeviceInfoError::NoPriBaseForEngine`]. Neither is silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnginePriBase {
    /// `FifoDeviceEntry::name`, exactly.
    pub engine: &'static str,
    /// Where that engine's PRI block is, or that it has none.
    pub pri_base: DevicePriBase,
}

/// ★★★ **The chip row for this reply — and it is the PRI bases and nothing else.**
///
/// Everything else in the twelve-field entry comes from `ChipProfile::engines`, because it
/// is already there. This row exists to hold the one field that is not, and to mark the
/// rows that are not devices.
///
/// ★ There is deliberately **no count field** and **no entry list**. `numEntries` is
/// derived from the projection at encode time, so a reply whose declared count disagrees
/// with its table is not rejected — it cannot be *expressed*. That is
/// [`crate::falconinfo::FalconInventoryRow`]'s design, applied where the fail-open it
/// closes has actually been observed on the wire (see this module's docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceInfoRow {
    /// One statement per engine in `ChipProfile::engines` — no more, no fewer.
    pub pri_bases: &'static [EnginePriBase],
}

impl DeviceInfoRow {
    /// What this row says about `engine`, or `None` if it says nothing.
    #[must_use]
    pub fn pri_base_for(&self, engine: &str) -> Option<DevicePriBase> {
        self.pri_bases
            .iter()
            .find(|b| b.engine == engine)
            .map(|b| b.pri_base)
    }
}

/// Which of an entry's two PRI-base fields a refusal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriBaseField {
    /// `devicePriBase` — supplied by [`DeviceInfoRow`].
    Device,
    /// `runlistPriBase` — projected from
    /// `engineData[`[`RUNLIST_PRI_BASE`](engine_info_type::RUNLIST_PRI_BASE)`]`.
    Runlist,
}

impl core::fmt::Display for PriBaseField {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Device => "devicePriBase",
            Self::Runlist => "runlistPriBase",
        })
    }
}

/// Why the reply could not be encoded.
///
/// Every variant is a projection this port refuses to put on the wire, and each names the
/// guest-side consequence it stands in front of — because the alternative is an assert
/// two engine-init steps later whose message mentions neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceInfoError {
    /// More devices than `deviceInfoTable[]` holds.
    ///
    /// RM's own check is `NV_ASSERT_OR_RETURN(pParams->numEntries <=
    /// NV2080_CTRL_CMD_INTERNAL_DEVICE_INFO_MAX_ENTRIES, NV_ERR_INVALID_STATE)`
    /// (`ogkm-580: gpu_gspclient.c:234`), so this is the same bound stated on our side —
    /// and, unlike [`crate::falconinfo::FalconInfoError::TooManyFalcons`], RM's version of
    /// it is *correct*. The fail-open on this control is the reply **length**, not the
    /// count, and that one is closed structurally: see this module's docs.
    TooManyEntries {
        /// How many devices the projection produced.
        count: usize,
        /// [`INTERNAL_DEVICE_INFO_MAX_ENTRIES`].
        max: usize,
    },
    /// Two engines in the FIFO table share a name, so the name is not a key and
    /// [`DeviceInfoRow`]'s lookup would silently take the first.
    DuplicateEngineName {
        /// The name that appears twice.
        engine: &'static str,
    },
    /// Two rows of [`DeviceInfoRow`] name the same engine — a second, contradicting
    /// statement of one fact, resolved silently by whichever comes first.
    DuplicatePriBase {
        /// The engine named twice.
        engine: &'static str,
    },
    /// ★★ An engine in the FIFO table that [`DeviceInfoRow`] says nothing about.
    ///
    /// **The anti-drift refusal, and the reason this is a derivation rather than two
    /// tables.** Adding an engine to `ChipProfile::engines` without saying where its PRI
    /// block is would otherwise drop it from this reply — the two descriptions of one
    /// silicon disagreeing, with no red test anywhere. Silence is not an answer here; it is
    /// [`DevicePriBase::NotADevice`] or an address.
    NoPriBaseForEngine {
        /// The engine with no statement.
        engine: &'static str,
    },
    /// A [`DeviceInfoRow`] entry naming an engine the FIFO table does not carry — a fact
    /// about silicon this device does not advertise, and the other direction of the same
    /// drift.
    PriBaseForUnknownEngine {
        /// The name that matched nothing.
        engine: &'static str,
    },
    /// ★★ [`DevicePriBase::NotADevice`] on a row the FIFO table calls a host-driven engine.
    ///
    /// The two statements would then disagree about the same row: one says *"RM may put a
    /// runlist on this"*, the other says *"this is not a device"*. `IS_HOST_DRIVEN_ENGINE`
    /// is what makes RM count a runlist for the entry, so the marking is not available for
    /// anything real — which is what keeps the `SOFTWARE` exclusion a property of the
    /// derivation instead of a name it happens to know.
    ///
    /// ⊘ The converse is **not** refused: a device with `isEngine = 0` is a normal thing
    /// (`[measured]` — the oracle's rows 10 and 11 are exactly that, in
    /// `C: src/qemu/mode2_initctrl_ga106.h:4028`'s `ctl_20800a40` at
    /// `nvidia-gpu-passthrough` rev `018e492`; the C's run, inherited, not ours), so a
    /// non-host-driven row may still be marked [`DevicePriBase::At`].
    PriBaseAbsentForHostDrivenEngine {
        /// The engine the FIFO table calls host-driven.
        engine: &'static str,
    },
    /// A PRI base that does not lie inside this device's BAR0.
    ///
    /// ⚠ The bound is on the base **naming a register at all** — one `NvU32` inside the
    /// aperture — because the extent of a device's PRI block is not stated anywhere this
    /// port can read. It is deliberately weaker than
    /// [`crate::falconinfo::FalconInfoError::RegisterBaseOutsideAperture`], which knows its
    /// block length.
    ///
    /// ★ `[inferred]`, and stated as such: no consumer in the vendored open tree reads
    /// `devicePriBase` after the copy — `gpu_vgpu.c:313` only prints it — so this guards no
    /// *measured* MMIO. What it does guard is measured, and by whom: `GA106_ENGINES`'
    /// `SOFTWARE` row carries the capture noise `0x77f2058f` in `RUNLIST_PRI_BASE` — read
    /// out of the committed
    /// `cap1b_coldboot_hermetic_d6.rec.zst` (the `cmd=0x20801112` reply at record 142049),
    /// `nvidia-gpu-passthrough` rev `018e492`, which is the C artifact's capture and not a
    /// run of ours — and no such value can
    /// be filed by RM as a register base. (On `SOFTWARE` itself the Esched-validity rule
    /// already zeroes the field, and `0x77f2058f` in particular is caught one check earlier
    /// by [`DeviceInfoError::PriBaseNotRegisterAligned`] — see this module's docs for all
    /// three guards.)
    PriBaseOutsideAperture {
        /// The engine the entry came from.
        engine: &'static str,
        /// Which field.
        field: PriBaseField,
        /// The offending value.
        pri_base: u32,
        /// This device's register aperture length.
        aperture_len: u64,
    },
    /// A PRI base that is not [`PRI_REGISTER_ALIGN`]-aligned, and therefore does not name a
    /// register.
    PriBaseNotRegisterAligned {
        /// The engine the entry came from.
        engine: &'static str,
        /// Which field.
        field: PriBaseField,
        /// The offending value.
        pri_base: u32,
    },
    /// ★★★ **A table with no copy engine, which includes the empty table.**
    ///
    /// `kgmmuInitCeMmuFaultIdRange_GA100` scans for `typeEnum == `[`DEV_TYPE_ENUM_LCE`],
    /// and with no match logs *"Failed to find any MMU Fault ID"*, fires `NV_ASSERT(0)` and
    /// returns `NV_ERR_OBJECT_NOT_FOUND`
    /// (`ogkm-580: kern_gmmu_ga100.c:281-287`), which propagates out of
    /// `kgmmuStateInitLocked_IMPL` (`ogkm-580: kern_gmmu.c:339`).
    ///
    /// ⊘ So *"answer honestly with nothing"* — [`crate::falconinfo::FalconInventoryRow::NONE`]'s
    /// move — is **not available on this control**. `gpuConstructDeviceInfoTable_FWCLIENT`
    /// accepts `numEntries == 0` with `NV_OK` (`ogkm-580: gpu_gspclient.c:231-232`), which
    /// is exactly what makes it worth refusing here: the boot would then die one engine
    /// later, in the MMU, attributed to nothing.
    NoCopyEngineAdvertised,
    /// ★★ Copy-engine fault ids that are not one contiguous run of distinct values.
    ///
    /// ⚠ **A policy refusal**, in the sense [`crate::falconinfo::MAX_CTX_BUFFER_SIZE`] is:
    /// nothing in RM rejects a gap. RM instead *reduces the set to its extremes* —
    /// `minCeMmuFaultId`/`maxCeMmuFaultId` — and then tests membership by range
    /// (`ogkm-580: kern_gmmu_gv100.c:739-740`) to decide whether a fault is RM-serviceable
    /// or UVM's. Every id in a gap is therefore an id the guest attributes to a copy engine
    /// this device never advertised, which is the *"present and hollow"* shape
    /// `GA106_ENGINES` refuses one layer up.
    ///
    /// ⊘ Scope, honestly: in Mode 2 this device authors the fault reports, so a gap is only
    /// reachable through our own future code. The refusal keeps the advertised set and the
    /// classified set **identical** rather than merely usually-equal. `[measured]` GA106's
    /// four are `0xf..=0x12`, so nothing is straining against this.
    CopyEngineFaultIdsNotContiguous {
        /// The smallest advertised copy-engine fault id.
        first: u32,
        /// The largest.
        last: u32,
        /// How many copy engines were advertised. Contiguity is `last - first + 1 == count`.
        count: usize,
    },
}

impl core::fmt::Display for DeviceInfoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooManyEntries { count, max } => write!(
                f,
                "{count} devices; deviceInfoTable[] holds {max} and RM asserts the count \
                 against it"
            ),
            Self::DuplicateEngineName { engine } => write!(
                f,
                "two engines are named {engine:?}, so a PRI base cannot be attributed to \
                 either"
            ),
            Self::DuplicatePriBase { engine } => write!(
                f,
                "engine {engine:?} has two PRI-base statements; one fact, said twice, \
                 resolved by order"
            ),
            Self::NoPriBaseForEngine { engine } => write!(
                f,
                "engine {engine:?} is in the FIFO device-info table but this chip states no \
                 devicePriBase for it; say DevicePriBase::NotADevice or say where it is"
            ),
            Self::PriBaseForUnknownEngine { engine } => write!(
                f,
                "a devicePriBase is stated for {engine:?}, which is not in this chip's FIFO \
                 device-info table"
            ),
            Self::PriBaseAbsentForHostDrivenEngine { engine } => write!(
                f,
                "engine {engine:?} is marked NotADevice but the FIFO table sets its \
                 IS_HOST_DRIVEN_ENGINE, so RM will count a runlist for it"
            ),
            Self::PriBaseOutsideAperture {
                engine,
                field,
                pri_base,
                aperture_len,
            } => write!(
                f,
                "{engine:?} {field} {pri_base:#x} does not name a register inside this \
                 device's {aperture_len:#x}-byte register aperture"
            ),
            Self::PriBaseNotRegisterAligned {
                engine,
                field,
                pri_base,
            } => write!(
                f,
                "{engine:?} {field} {pri_base:#x} is not {PRI_REGISTER_ALIGN}-byte aligned \
                 and so names no register"
            ),
            Self::NoCopyEngineAdvertised => write!(
                f,
                "no entry has typeEnum {DEV_TYPE_ENUM_LCE:#x} (LCE); \
                 kgmmuInitCeMmuFaultIdRange would find no MMU fault id and end the boot"
            ),
            Self::CopyEngineFaultIdsNotContiguous { first, last, count } => write!(
                f,
                "{count} copy engines with fault ids spanning {first:#x}..={last:#x}; RM \
                 reduces them to that range and treats every id in it as a copy engine's"
            ),
        }
    }
}

impl core::error::Error for DeviceInfoError {}

/// ★★★ Encode `NV2080_CTRL_INTERNAL_GET_DEVICE_INFO_TABLE_PARAMS` **by projecting the
/// chip's FIFO engine table**.
///
/// `engines` is `kayfabe_device::ChipProfile::engines` — the same slice
/// [`crate::inittables::encode_device_info_table`] serves — and `row` supplies only the one
/// field it does not carry. There is no other way to produce these bytes, which is what
/// stops the two replies drifting; see this module's docs.
///
/// The whole [`INTERNAL_DEVICE_INFO_PARAMS_SIZE`]-byte struct is returned, zero past the
/// derived count. `numEntries` is the number of entries actually written and is never
/// carried alongside them.
///
/// `regs_aperture_len` is this device's BAR0 extent, so a PRI base is checked against the
/// aperture actually being served rather than against a constant in this crate.
///
/// # Errors
///
/// Every variant of [`DeviceInfoError`]; see each for the guest-side consequence it stands
/// in front of.
pub fn encode_internal_device_info_table(
    engines: &[FifoDeviceEntry],
    row: &DeviceInfoRow,
    regs_aperture_len: u64,
) -> Result<Vec<u8>, DeviceInfoError> {
    // ── The key has to be a key, on both sides, before any lookup means anything. ──
    for (i, e) in engines.iter().enumerate() {
        if engines[..i].iter().any(|p| p.name == e.name) {
            return Err(DeviceInfoError::DuplicateEngineName { engine: e.name });
        }
    }
    for (i, b) in row.pri_bases.iter().enumerate() {
        if row.pri_bases[..i].iter().any(|p| p.engine == b.engine) {
            return Err(DeviceInfoError::DuplicatePriBase { engine: b.engine });
        }
        if !engines.iter().any(|e| e.name == b.engine) {
            return Err(DeviceInfoError::PriBaseForUnknownEngine { engine: b.engine });
        }
    }

    // How many entries the projection will produce. Computed before anything is written so
    // the write loop cannot run off `deviceInfoTable[]`.
    let projected = engines
        .iter()
        .filter(|e| matches!(row.pri_base_for(e.name), Some(DevicePriBase::At(_))))
        .count();
    if projected > INTERNAL_DEVICE_INFO_MAX_ENTRIES {
        return Err(DeviceInfoError::TooManyEntries {
            count: projected,
            max: INTERNAL_DEVICE_INFO_MAX_ENTRIES,
        });
    }

    let mut params = vec![0u8; INTERNAL_DEVICE_INFO_PARAMS_SIZE];
    let mut written = 0usize;
    let mut lce_fault_ids: Vec<u32> = Vec::new();

    for e in engines {
        let host_driven = e.engine_data[engine_info_type::IS_HOST_DRIVEN_ENGINE] != 0;
        let Some(stated) = row.pri_base_for(e.name) else {
            return Err(DeviceInfoError::NoPriBaseForEngine { engine: e.name });
        };
        let device_pri_base = match stated {
            DevicePriBase::NotADevice => {
                if host_driven {
                    return Err(DeviceInfoError::PriBaseAbsentForHostDrivenEngine {
                        engine: e.name,
                    });
                }
                // ★ The exclusion. Not a name check: a row that claims no PRI block gets no
                // entry, whatever it is called.
                continue;
            }
            DevicePriBase::At(base) => base,
        };

        // ★ `RUNLIST_PRI_BASE` and `RUNLIST_ENGINE_ID` are documented *"Valid only for
        // Esched-driven engines"* (`ogkm-580: engine_info.h:70-90`), so on a row the FIFO
        // table does not call host-driven they are not data. The oracle's own two
        // non-engine rows carry zero in both.
        let (rl_eng_id, runlist_pri_base) = if host_driven {
            (
                e.engine_data[engine_info_type::RUNLIST_ENGINE_ID],
                e.engine_data[engine_info_type::RUNLIST_PRI_BASE],
            )
        } else {
            (0, 0)
        };

        for (field, base) in [
            (PriBaseField::Device, device_pri_base),
            (PriBaseField::Runlist, runlist_pri_base),
        ] {
            if !base.is_multiple_of(PRI_REGISTER_ALIGN) {
                return Err(DeviceInfoError::PriBaseNotRegisterAligned {
                    engine: e.name,
                    field,
                    pri_base: base,
                });
            }
            if u64::from(base) + 4 > regs_aperture_len {
                return Err(DeviceInfoError::PriBaseOutsideAperture {
                    engine: e.name,
                    field,
                    pri_base: base,
                    aperture_len: regs_aperture_len,
                });
            }
        }

        let fault_id = e.engine_data[engine_info_type::MMU_FAULT_ID];
        let instance_id = e.engine_data[engine_info_type::INSTANCE_ID];
        let type_enum = e.engine_data[engine_info_type::DEV_TYPE_ENUM];
        if type_enum == DEV_TYPE_ENUM_LCE {
            lce_fault_ids.push(fault_id);
        }

        let at = DEVICE_INFO_TABLE_OFF + written * INTERNAL_DEVICE_INFO_STRIDE;
        let mut put = |off: usize, v: u32| {
            params[at + off..at + off + 4].copy_from_slice(&v.to_le_bytes());
        };
        put(FAULT_ID_OFF, fault_id);
        put(INSTANCE_ID_OFF, instance_id);
        put(TYPE_ENUM_OFF, type_enum);
        put(RESET_ID_OFF, e.engine_data[engine_info_type::RESET]);
        put(DEVICE_PRI_BASE_OFF, device_pri_base);
        // ⚠ The slot's own value, not `u32::from(host_driven)`. Normalising a non-zero to 1
        // would be a redesign of bytes the oracle chose, and this port's rule is to
        // reproduce and subtract only *named* bugs.
        put(
            IS_ENGINE_OFF,
            e.engine_data[engine_info_type::IS_HOST_DRIVEN_ENGINE],
        );
        put(RL_ENG_ID_OFF, rl_eng_id);
        put(RUNLIST_PRI_BASE_OFF, runlist_pri_base);
        // `groupId`, `ginTargetId` and `deviceBroadcastPriBase` stay zero — `[measured]` in
        // all twelve of the oracle's rows: `ctl_20800a40` at
        // `C: src/qemu/mode2_initctrl_ga106.h:4028`, `nvidia-gpu-passthrough` rev `018e492`.
        // The C's capture, inherited; no run of ours says this. See this module's docs for
        // the scope of it.
        put(GROUP_ID_OFF, 0);
        put(GIN_TARGET_ID_OFF, 0);
        put(DEVICE_BROADCAST_PRI_BASE_OFF, 0);
        // `[measured]` equal to `instanceId` in all twelve of the oracle's rows —
        // `ctl_20800a40` at `C: src/qemu/mode2_initctrl_ga106.h:4028`,
        // `nvidia-gpu-passthrough` rev `018e492`. The C's capture, inherited, not ours.
        put(GROUP_LOCAL_INSTANCE_ID_OFF, instance_id);

        written += 1;
    }

    // ── What the guest's only reader of this table requires. ──
    if lce_fault_ids.is_empty() {
        return Err(DeviceInfoError::NoCopyEngineAdvertised);
    }
    lce_fault_ids.sort_unstable();
    let first = lce_fault_ids[0];
    let last = lce_fault_ids[lce_fault_ids.len() - 1];
    let span = u64::from(last) - u64::from(first) + 1;
    if span != lce_fault_ids.len() as u64 {
        return Err(DeviceInfoError::CopyEngineFaultIdsNotContiguous {
            first,
            last,
            count: lce_fault_ids.len(),
        });
    }

    // ★ Derived from the projection, never carried beside it: see [`DeviceInfoRow`].
    let count = u32::try_from(written).unwrap_or(u32::MAX);
    params[NUM_ENTRIES_OFF..NUM_ENTRIES_OFF + 4].copy_from_slice(&count.to_le_bytes());
    Ok(params)
}
