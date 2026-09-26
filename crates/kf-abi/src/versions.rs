//! The driver-version table: nvproxy's inherit-then-mutate model, and the
//! **exact** version boundaries it turns on.
//!
//! # Why exact boundaries and not majors
//!
//! The C artifact keys its ABI profile on the **major version alone**
//! (`nvidia-gpu-passthrough/src/common/nvkvm_abi.h:112-121`,
//! `nvkvm_abi_id_for_major`). That is too coarse and demonstrably so: the
//! `NVOS46_PARAMETERS` growth lands at **580.65.06**, not at 580.0
//! (`gvisor/pkg/sentry/devices/nvproxy/version.go:1057-1059` switches to
//! `NVOS46_PARAMETERS_V580` at exactly that entry), and `NVOS47_PARAMETERS`
//! grew at **550.54.04** (`gvisor/pkg/abi/nvgpu/frontend.go:707-710`), mid-major
//! in both cases. A major-only key cannot express either, so it is right by
//! luck for the releases that happen to exist and wrong for the ones that do
//! not.
//!
//! It is also wrong in a *second* way: `nvkvm_abi_by_id` returns the 570 profile
//! for any unrecognised id (`nvkvm_abi.h:105-110`), so an unknown driver
//! silently gets 575's struct sizes. Here an unknown-and-too-old version is
//! [`AbiError::NoTableForVersion`]. MISS = FAULT, never a nearest-neighbour
//! guess.
//!
//! # The supported range, and why it starts where it does
//!
//! The oldest table is **550.54.04**. Below that, `NVOS47_PARAMETERS` is the
//! 40-byte pre-`size` shape, which this crate does not carry
//! (`crate::transcribed`'s module doc says why: no supported driver is that old,
//! and an untested transcription is worse than an absent one). So versions below
//! 550.54.04 are refused rather than decoded with a layout that is wrong by 8
//! bytes in the middle of the struct.

use crate::capability::{
    CAPS_550_54_04, CAPS_550_90_07, CAPS_555_42_02, CAPS_560_28_03, CAPS_570_86_15, CAPS_575_51_02,
    CAPS_580_65_06, CAPS_610_43_02, CapabilityTable,
};
use crate::generated::{classes, ctrl, nvos, rpc};
use crate::guestsysinfo::VgxVersion;
use kf_arch::UserdMem;

use crate::notifier::{
    ChannelEngineWire, ChannelNotifierWire, ChannelUserdMemWire, ChannelUserdWire,
};
use crate::transcribed::{Nv2080CtrlGpuPromoteCtxParamsHeader, Nvos46ParametersPre580};
use crate::vbios::VbiosWire;
use crate::view::{
    AllocReq, AllocWire, ChannelAllocFacts, ClientAllocFacts, ControlReq, CtxShareAllocFacts,
    DeviceAllocFacts, DupReq, FreeReq, MAX_PROMOTE_ENTRIES, MapMemoryDma, PdbAperture, PromoteCtx,
    PromoteEntry, RpcAllocReq, RpcControlReq, RpcEnvelope, SetPageDir, TsgAllocFacts,
    UnmapMemoryDma, classify_promote_entry, rpc_payload_len,
};
use crate::wire::{AbiError, u32_at, u64_at};
use crate::{DriverAbi, DriverVersion};
use kf_arch::fault::ErrorNotifier;
use kf_arch::ids::{ClassId, ControlCmd};

/// Which `NVOS46_PARAMETERS` shape a driver version uses.
///
/// The only versioned layout in this milestone's slice — recorded as an explicit
/// enum rather than a size, because the *offsets* move too and a size alone
/// would let a caller compute `status` at the wrong place (which is `#81`, the
/// C artifact's own `nvos46_status_off` table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapDmaWire {
    /// 56 bytes; `dmaOffset` @ +40, `status` @ +48.
    Pre580_65_06,
    /// 64 bytes; `flags2` @ +36, `kindOverride` @ +40, `dmaOffset` @ +48,
    /// `status` @ +56.
    From580_65_06,
}

/// Which `GspStaticConfigInfo` shape a driver version speaks — the `GET_GSP_STATIC_INFO`
/// (fn 65) reply body ([`crate::gspstaticinfo`]).
///
/// ★ The break is at **610.43.02** and it is structural, not a field move: 610 removes
/// `grCapsBits[]`, `fbio_mask`, `fb_bus_width`, `fb_ram_type`, `fbp_mask`,
/// `l2_cache_size` and `gpuNameString_Unicode[]`, and adds `bPdiValid`/`pdi` and
/// `vbiosRevision` (`ogkm-580: src/nvidia/inc/kernel/gpu/gsp/gsp_static_config.h:78-169`
/// vs `ogkm-610:` the same path). `grCapsBits[]` is the **first** member, so every offset
/// in the struct moves — there is no shared prefix to lean on.
///
/// ⊘ Only [`GspStaticInfoWire::Pre610`] is encoded. The 580 offsets are pinned against an
/// RTX 3060's own reply (`traces/mode2_c_reference/cap1b_coldboot_hermetic_d6` record
/// 141977); there is no such capture for 610, no 610 guest has been booted here, and
/// computing offsets for a struct this port has never seen on a wire is exactly the
/// guess this crate exists to avoid. The variant exists so that the day one is captured
/// is a table edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GspStaticInfoWire {
    /// `grCapsBits[23]` first; `fbRegionInfoParams` @344, `fb_length` @1352, 1792 bytes.
    Pre610,
    /// The 610 reshuffle. **Not encoded** — see the enum's doc.
    From610_43_02,
    /// ★ A measured layout that is neither of the above (565: 1640 bytes, 570/575: 1656,
    /// 590: 1808, 595: 1592 — `traces/driver_matrix/report.md`). **Not encoded**: fn 65 is
    /// refused by name at that version until the encoder is driven by the measured layout
    /// (`docs/design/V3_DRIVER_MATRIX.md` §6).
    Unencoded,
}

/// Which `GSP_MSG_QUEUE_ELEMENT` shape a driver version speaks.
///
/// ★ The break is at **610.43.02**, and it is the whole element header, not a field:
/// 48 bytes with an `elemCount` becomes 16 bytes with MCTP/NVDM transport words. Read at
/// both endpoints — `ogkm-580: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:43-51`
/// and `ogkm-610: .../message_queue_priv.h:52-67`. 575/580/590/595 are all on the 48-byte
/// side; only the 610 boundary itself was read here, and a `>= 610` key is safe under
/// either reading of the relayed tags because that is the directly-verified one
/// (`mode2_gsp_port_plan.md` §14.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GspElementWire {
    /// 48-byte header: `authTagBuffer[16]@0`, `aadBuffer[16]@16`, `checkSum@32`,
    /// `seqNum@36`, **`elemCount@40`**, `rpc@48`. No transport headers — bytes @0..@31 are
    /// the Confidential-Compute buffers, which a CC-off guest never reads.
    Pre610,
    /// 16-byte header: `mctpHeader@0`, `nvdmHeader@4`, `checkSum@8`, `seqNum@12`, payload
    /// at 16. **No `elemCount`** — the receiver derives the run length from `rpc.length`,
    /// and offset 40 is `rpc.sequence`.
    From610_43_02,
}

/// The MCTP/NVDM transport words a 610-era element carries, and the parts of them the
/// guest actually **validates**.
///
/// ★ The validated masks are load-bearing, not decoration. The receiver checks exactly two
/// bit fields — `REF_VAL(MCTP_HEADER_VERSION, mctpHeader) == 0x1` and
/// `REF_VAL(MCTP_MSG_HEADER_VENDOR_ID, nvdmHeader) == 0x10de`
/// (`ogkm-610: src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:735-762`). SOM, EOM, the
/// packet sequence and the NVDM **type** byte are written by the sender and never read, so
/// no test anywhere may assert that a guest rejects a wrong one — that would assert a
/// behaviour the driver does not have. Same rule the RPC `signature` already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GspTransportWords {
    /// Byte offset of `mctpHeader`.
    pub header_off: usize,
    /// The word a conforming sender writes there.
    pub header_word: u32,
    /// The bits of it the receiver reads (`MCTP_HEADER_VERSION`, `3:0`).
    pub header_validated_mask: u32,
    /// Byte offset of `nvdmHeader`.
    pub nvdm_off: usize,
    /// The word a conforming sender writes there.
    pub nvdm_word: u32,
    /// The bits of it the receiver reads (`MCTP_MSG_HEADER_VENDOR_ID`, `23:8`).
    pub nvdm_validated_mask: u32,
}

impl GspElementWire {
    /// `queueElementHdrSize` with Confidential Compute **off**.
    #[must_use]
    pub fn hdr_size(self) -> usize {
        match self {
            GspElementWire::Pre610 => 48,
            GspElementWire::From610_43_02 => 16,
        }
    }

    /// Byte offset of `checkSum`.
    #[must_use]
    pub fn checksum_off(self) -> usize {
        match self {
            GspElementWire::Pre610 => 32,
            GspElementWire::From610_43_02 => 8,
        }
    }

    /// Byte offset of `seqNum`.
    #[must_use]
    pub fn seqnum_off(self) -> usize {
        match self {
            GspElementWire::Pre610 => 36,
            GspElementWire::From610_43_02 => 12,
        }
    }

    /// Byte offset of `elemCount`, on the versions that have one.
    #[must_use]
    pub fn elem_count_off(self) -> Option<usize> {
        match self {
            GspElementWire::Pre610 => Some(40),
            GspElementWire::From610_43_02 => None,
        }
    }

    /// The transport words, on the versions that carry them.
    ///
    /// ★ Assembled here from the driver's own bit fields rather than transcribed:
    /// `mctpCreateTransportHeader(som=1, eom=1, seid=0, deid=0, seq=0)` is
    /// `REF_NUM(MCTP_HEADER_VERSION 3:0, 1) | REF_NUM(EOM 30:30, 1) | REF_NUM(SOM 31:31, 1)`
    /// = `0xC000_0001`, and `mctpCreateNvdmHeader(NVDM_TYPE_RM_RPC)` is
    /// `REF_DEF(TYPE 6:0, VENDOR_PCI=0x7e) | REF_DEF(VENDOR_ID 23:8, NV=0x10de) |
    /// REF_NUM(NVDM_TYPE 31:24, 0x25)` = `0x2510_DE7E`
    /// (`ogkm-610: src/nvidia/arch/nvalloc/common/inc/mctp_format.h:39-58, 79-95, 108-120`,
    /// `.../nvdm_format.h:61`, emitted at
    /// `ogkm-610: message_queue_cpu.c:505-512`).
    #[must_use]
    pub fn transport(self) -> Option<GspTransportWords> {
        match self {
            GspElementWire::Pre610 => None,
            GspElementWire::From610_43_02 => Some(GspTransportWords {
                header_off: 0,
                header_word: 0xC000_0001,
                header_validated_mask: 0x0000_000F,
                nvdm_off: 4,
                nvdm_word: 0x2510_DE7E,
                nvdm_validated_mask: 0x00FF_FF00,
            }),
        }
    }
}

/// Which `MESSAGE_QUEUE_INIT_ARGUMENTS` shape a driver version publishes.
///
/// ★ The plan presented *"the guest declares its own queue geometry"* as the high-leverage
/// design choice. **That is 610 only.** At 580 the struct has exactly four fields and the
/// geometry is compile-time (`ogkm-580: src/nvidia/inc/kernel/gpu/gsp/gsp_init_args.h:29-34`,
/// populated at `ogkm-580: kernel_gsp.c:4486-4489`; the constants are
/// `ogkm-580: message_queue_priv.h:91-104`). So on the bench there is nothing to read and
/// the table below is what supplies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GspInitArgsWire {
    /// Four fields: `sharedMemPhysAddr, pageTableEntryCount, cmdQueueOffset,
    /// statQueueOffset`. Identical to nouveau's r570 form. No geometry is negotiated.
    FourField,
    /// Nine: the four above plus `queueElementHdrSize, queueElementSizeMin,
    /// queueElementSizeMax, queueHeaderAlign, queueElementAlign`
    /// (`ogkm-610: gsp_init_args.h:32-45`).
    ///
    /// ⚠ Because `MESSAGE_QUEUE_INIT_ARGUMENTS` is the **first** member of
    /// `GSP_ARGUMENTS_CACHED` and grows here, **every subsequent offset in that struct
    /// differs between the two tags**. Nothing reads them today; the first person who
    /// needs one must not transcribe a 610 offset for a 580 guest.
    NineField,
}

impl GspInitArgsWire {
    /// Bytes of `MESSAGE_QUEUE_INIT_ARGUMENTS` that must be readable for the fields this
    /// port consumes. `NvLength` is `size_t`, so there are 4 pad bytes after the `u32`
    /// at +8 and the first four fields end at 32.
    #[must_use]
    pub fn min_size(self) -> usize {
        match self {
            GspInitArgsWire::FourField => 32,
            GspInitArgsWire::NineField => 40,
        }
    }

    /// Offset of `queueElementHdrSize`, on the versions that declare it — the **capability**
    /// that lets the element header size be derived rather than keyed, where it exists.
    #[must_use]
    pub fn element_hdr_size_off(self) -> Option<usize> {
        match self {
            GspInitArgsWire::FourField => None,
            GspInitArgsWire::NineField => Some(32),
        }
    }
}

/// One driver version's ABI table.
///
/// nvproxy's `driverABI` is four handler maps
/// (`version.go:100-107`); this is the same idea at the size this slice needs —
/// one entry per versioned layout, and nothing for the layouts that do not vary.
/// Adding a versioned struct means adding a field here and a line to each table,
/// which is the ~14-51-line delta `mode2_abi_agnostic_layer.md` §2.1 measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverAbiTable {
    /// ★ The EXACT measured driver version this table was assembled for (since 2026-09-26 —
    /// before, the boundary row a version inherited from).
    version: DriverVersion,
    map_dma: MapDmaWire,
    gsp_element: GspElementWire,
    gsp_init_args: GspInitArgsWire,
    /// Which `GspStaticConfigInfo` shape this driver version reads back out of a
    /// `GET_GSP_STATIC_INFO` reply ([`crate::gspstaticinfo`]).
    ///
    /// Here for the reason the whole module exists: the struct is reshuffled at
    /// 610.43.02, and a port that answered every version with one layout would be
    /// handing a 610 guest a region table read at the wrong offsets — the failure that
    /// looks like corrupt memory rather than like a version mismatch.
    gsp_static_info: GspStaticInfoWire,
    /// ★ The **default-deny RM capability surface** for this boundary
    /// ([`crate::capability`]): which control commands and which allocation
    /// classes a guest at this driver version may name at all.
    ///
    /// It is a field here, and not a free function, for the reason the whole
    /// module exists: *adding a driver version must not edit a logic crate*. A
    /// new version is a new `TABLES` row pointing at a new [`CapabilityTable`].
    ///
    /// ★★★ That type is **shared-base + per-boundary blocks** since task #122, not
    /// inherit-then-add. So the row a version points at is that version's **whole**
    /// surface, and a version whose vendor *removed* a command points at a table that
    /// does not name the block carrying it — which is the case 575.51.02 is, and which
    /// the previous shape could not express (see [`crate::capability`]'s module doc).
    caps: &'static CapabilityTable,
    /// Which **synthetic-VBIOS** parse path this driver version speaks
    /// ([`crate::vbios`]).
    ///
    /// Here for the same reason `caps` is: adding a driver version must not edit
    /// a logic crate. Today every row carries [`VbiosWire::Tu102Bit`], because
    /// the four files defining that path are byte-identical at both vendored
    /// ogkm tags — see [`VbiosWire`]'s doc for the measurement. The field exists
    /// so the day a version diverges is a table edit, not a redesign.
    vbios: VbiosWire,
    /// ★★ The **vGPU RPC version this driver speaks** — the pair the `SET_GUEST_SYSTEM_INFO`
    /// handshake exchanges ([`crate::guestsysinfo`]).
    ///
    /// `None` where this port has no `VGX_*_VERSION_NUMBER` citation for the row, and
    /// answering the handshake then **refuses by name**. That is the point: the guest
    /// reads the version back out of the *reply* and selects its whole RPC function table
    /// from it, so a device that echoed would agree with anything and the disagreement
    /// would surface hundreds of messages later at the wrong struct offsets.
    vgx: Option<VgxVersion>,
    /// ★★★ Where this boundary's `NV_CHANNEL_ALLOC_PARAMS` puts `internalFlags` and
    /// `errorNotifierMem` — **the channel's error notifier**, which the guest's CPU-RM
    /// resolves and RPCs to the GSP, and which the GSP is the one contracted to write
    /// (`crate::notifier`).
    ///
    /// `None` where this port has **not read that version's tree**, and that is the
    /// point rather than an omission. `crate::view::ChannelAllocFacts` stops decoding at
    /// +32 because the struct's tail moves inside the supported range, so reading these
    /// two fields is a right a *read* tree buys; only 580.159.04 and 610.43.02 are
    /// vendored (`ogkm_is_versioned`). A boundary with `None` never learns a notifier,
    /// so `kayfabe_core::fault::verdict` refuses to emit an RC there — which is the safe
    /// direction, because an RC with no notifier write is the hang task #111 exists to
    /// remove (`docs/design/resume_from_fault.md` §S5(b)).
    channel_notifier: Option<ChannelNotifierWire>,
    /// ★★★★ §16.16 — where this boundary's `NV_CHANNEL_ALLOC_PARAMS` puts
    /// `hUserdMemory[0]` and `userdOffset[0]`. Same seam and same rule as
    /// [`Self::channel_notifier`]: both fields sit past [`CHANNEL_ALLOC_PREFIX`] in the
    /// region 610 shifts by eight bytes, so reading them is a right a *read* tree buys and
    /// an unopened boundary carries `None` rather than a guess. See
    /// [`ChannelUserdWire`] for why USERD is the canary the ring cannot be.
    channel_userd: Option<ChannelUserdWire>,
    /// ★★★★★ Where this boundary's `NV_CHANNEL_ALLOC_PARAMS` puts **`userdMem`** — the
    /// descriptor the guest's own CPU-RM fills with the **resolved physical address** of
    /// this channel's USERD before it RPCs the GSP.
    ///
    /// Same seam and same rule as the three above. See [`ChannelUserdMemWire`] for the
    /// driver source, and for the three documents whose *"the guest's USERD address is
    /// unobtainable"* this field refutes.
    channel_userd_mem: Option<ChannelUserdMemWire>,
    /// ★★★★★ Where this boundary's `NV_CHANNEL_ALLOC_PARAMS` puts **`engineType`** — the
    /// only wire field that separates a GR channel from a CE channel, since both are
    /// `AMPERE_CHANNEL_GPFIFO_A`.
    ///
    /// Same seam and same rule as [`Self::channel_notifier`] and [`Self::channel_userd`]:
    /// the field sits past [`CHANNEL_ALLOC_PREFIX`] in the region 610 shifts by eight
    /// bytes, so reading it is a right a *read* tree buys and an unopened boundary carries
    /// `None` rather than a guess. See [`ChannelEngineWire`] for what a `None` costs: the
    /// core falls back to the engine-object refinement it used before this field existed,
    /// which is exactly the pre-2026-08-11 behaviour and not a new failure.
    channel_engine: Option<ChannelEngineWire>,
    /// ★★★ Where this version's `rpc_gsp_rm_control_v` puts its fields — MEASURED
    /// ([`RmControlWire`]). The header is 24 bytes through 570.x and 40 from 575.51.02, so a
    /// fixed offset mis-slices every control of every guest on one side of that boundary.
    rm_control: RmControlWire,
    /// The capability row's justification (nvproxy boundary) — kept in the data so a reader
    /// sees why this version gets that allowlist without leaving the file.
    pub note: &'static str,
}

/// ★★★ `rpc_gsp_rm_control_v` — the body of every `GSP_RM_CONTROL` (fn 76), as MEASURED at
/// one driver tag (`crate::generated::matrix::RPC_GSP_RM_CONTROL_V`).
///
/// `[measured, tools/drivermatrix, 2026-09-26]` 535.309.01 … 570.148.08: 24 bytes,
/// `flags`@20, `params`@24. 575.51.03 … 615.71.09: 40 bytes, `rmapiRpcFlags`@20,
/// `rmctrlFlags`@24, `rmctrlAccessRight`@28, `reserved0`@32, `params`@40. `hClient`@0,
/// `hObject`@4, `cmd`@8, `status`@12 and `paramsSize`@16 hold at every tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RmControlWire {
    /// Offset of `params[]` — the fixed header's size.
    pub params_off: usize,
    /// Offset of the RPC-flags word (`rmapiRpcFlags`, spelled `flags` before 575) — the word
    /// [`crate::rpc_params_are_serialized`] tests.
    pub rpc_flags_off: usize,
    /// Offset of `rmctrlFlags`, where the version has it.
    pub rmctrl_flags_off: Option<usize>,
    /// Offset of `rmctrlAccessRight`, where the version has it.
    pub access_right_off: Option<usize>,
    /// Offset of `paramsSize`.
    pub params_size_off: usize,
    /// Offset of `status`.
    pub status_off: usize,
}

/// ★★★ The **capability allowlist's** version boundaries — the one hand-maintained row set
/// left in this module, and why it is legitimately hand-maintained.
///
/// Until 2026-09-26 this was `TABLES`, one hand row per boundary carrying EVERY versioned
/// fact (wire shapes, the VGX pair, the channel-alloc offsets) — eight rows, 550.54.04 up,
/// with `vgx`/`channel_*` `None` on six of them and every guest below the floor refused.
/// `docs/design/V3_DRIVER_MATRIX.md` §3 replaced the layout half with MEASURED data
/// ([`crate::matrix`]): those facts are now read per exact driver tag out of
/// [`crate::generated::matrix`], and [`table_for`] assembles a table for any measured tag.
///
/// ⊘ What stays here is **policy, not layout**: which controls and classes a guest at a
/// given version may name at all. Its source is gVisor nvproxy's per-version registry
/// (`gvisor/pkg/sentry/devices/nvproxy/version.go`), a security allowlist a human reviews,
/// and it is keyed on nvproxy's own boundaries ("newest row ≤ version" — an allowlist row
/// is valid until the next one changes it, which is what nvproxy's inherit-then-mutate
/// means). ⚠ Below the oldest row (535.x, 545.x) there is no reviewed allowlist in this
/// port; [`table_for`] refuses those versions by name ([`AbiError::NoCapabilityRow`]) rather
/// than handing them the 550 surface. nvproxy has 535.104.05 and 545.23.06 blocks
/// (`version.go:159`, `:837`) — porting them is the named follow-on (§6).
#[derive(Debug, Clone, Copy)]
struct CapsRow {
    from: DriverVersion,
    caps: &'static CapabilityTable,
    note: &'static str,
}

const fn dv(major: u16, minor: u16, patch: u16) -> DriverVersion {
    DriverVersion {
        major,
        minor,
        patch,
    }
}

/// Ascending. See [`CapsRow`].
const CAPS_ROWS: &[CapsRow] = &[
    CapsRow {
        from: dv(550, 54, 4),
        caps: &CAPS_550_54_04,
        note: "oldest reviewed allowlist (NVOS47 gained `size` here — \
               gvisor/pkg/abi/nvgpu/frontend.go:707-710)",
    },
    CapsRow {
        from: dv(550, 90, 7),
        caps: &CAPS_550_90_07,
        note: "+NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE (version.go:906)",
    },
    CapsRow {
        from: dv(555, 42, 2),
        caps: &CAPS_555_42_02,
        note: "purely SUBTRACTIVE: nvproxy deletes NVC36F_CTRL_GET_CLASS_ENGINEID (version.go:933)",
    },
    CapsRow {
        from: dv(560, 28, 3),
        caps: &CAPS_560_28_03,
        note: "+8 allocation classes, +NV_SEMAPHORE_SURFACE_CTRL_CMD_UNBIND_CHANNEL \
               (version.go:945-977)",
    },
    CapsRow {
        from: dv(570, 86, 15),
        caps: &CAPS_570_86_15,
        note: "+6 allocation classes, the DRAM-encryption controls at their PRE-575 numbers \
               (version.go:990-1027)",
    },
    CapsRow {
        from: dv(575, 51, 2),
        caps: &CAPS_575_51_02,
        note: "the REPLACING boundary: two DRAM-encryption controls renumbered, \
               THERMAL_SYSTEM_EXECUTE_V2 added (version.go:1036-1053)",
    },
    CapsRow {
        from: dv(580, 65, 6),
        caps: &CAPS_580_65_06,
        note: "+2 allocation classes (version.go:1057-1078)",
    },
    CapsRow {
        from: dv(610, 43, 2),
        caps: &CAPS_610_43_02,
        note: "the 610 control set (version.go:1182-1243)",
    },
];

/// The capability allowlist for `version` — the newest nvproxy row at or below it, or `None`
/// below the oldest row. Policy lookup only; [`table_for`] adds the measured-tag rule.
#[must_use]
pub fn capabilities_for(version: DriverVersion) -> Option<&'static CapabilityTable> {
    CAPS_ROWS.iter().rev().find(|r| r.from <= version).map(|r| r.caps)
}

/// Every capability allowlist a version can be admitted against, in boundary order — the
/// universe `crate::capability`'s structural tests quantify over.
pub fn capability_tables() -> impl Iterator<Item = &'static CapabilityTable> {
    CAPS_ROWS.iter().map(|r| r.caps)
}

/// ★★ The facts a guest has CONSUMED by the time it sends fn 1 (`SET_GUEST_SYSTEM_INFO`), as
/// two driver versions state them — the precondition for re-selecting a device's table at fn 1
/// (`docs/design/V3_DRIVER_MATRIX.md` §4.2, owner ruling 6, 2026-09-26).
///
/// Before fn 1 is answered the guest has: booted the GSP through the queue framing
/// (`GSP_MSG_QUEUE_ELEMENT`, `MESSAGE_QUEUE_INIT_ARGUMENTS`, the element size maximum), read
/// the synthetic VBIOS, and sent `GSP_SET_SYSTEM_INFO` / `SET_REGISTRY` and fn 1 itself by
/// NUMBER, fn 1's body in the `rpc_set_guest_system_info_v` layout, and it waits for
/// `GSP_INIT_DONE` by number. Nothing else in the table has been used yet. Two versions that
/// agree on all of it can swap tables at fn 1 without the guest having seen the difference.
///
/// Returns `None` when they agree, or the first fact that differs, by name.
#[must_use]
pub fn pre_fn1_surface_differs(a: &DriverAbiTable, b: &DriverAbiTable) -> Option<&'static str> {
    if a.gsp_element_wire() != b.gsp_element_wire() {
        return Some("GSP_MSG_QUEUE_ELEMENT");
    }
    if a.gsp_init_args_wire() != b.gsp_init_args_wire() {
        return Some("MESSAGE_QUEUE_INIT_ARGUMENTS");
    }
    if a.gsp_element_size_max() != b.gsp_element_size_max() {
        return Some("GSP_MSG_QUEUE_ELEMENT_SIZE_MAX");
    }
    if a.vbios_wire() != b.vbios_wire() {
        return Some("the synthetic VBIOS parse path");
    }
    use crate::generated::matrix::{ALL_STRUCTS, ALL_VALUES};
    for name in [
        "rpc_functions:NV_VGPU_MSG_FUNCTION_SET_GUEST_SYSTEM_INFO",
        "rpc_functions:NV_VGPU_MSG_FUNCTION_SET_GUEST_SYSTEM_INFO_EXT",
        "rpc_functions:NV_VGPU_MSG_FUNCTION_GSP_SET_SYSTEM_INFO",
        "rpc_functions:NV_VGPU_MSG_FUNCTION_SET_REGISTRY",
        "rpc_events:NV_VGPU_MSG_EVENT_GSP_INIT_DONE",
    ] {
        let Some(runs) = ALL_VALUES.iter().find(|r| r.name == name) else {
            return Some(name);
        };
        match (runs.at(a.version), runs.at(b.version)) {
            (Ok(x), Ok(y)) if x == y && x.is_some() => {}
            _ => return Some(runs.name),
        }
    }
    for name in ["rpc_set_guest_system_info_v", "rpc_message_header_v"] {
        let Some(runs) = ALL_STRUCTS.iter().find(|r| r.name == name) else {
            return Some(name);
        };
        match (runs.at(a.version), runs.at(b.version)) {
            (Ok(Some(x)), Ok(Some(y))) if x == y => {}
            _ => return Some(runs.name),
        }
    }
    None
}

/// The driver version this project's bench actually runs
/// (`docs/reference/rm_semantics_measured.md` §0), on both axes by default.
pub const BENCH_DRIVER: DriverVersion = DriverVersion {
    major: 580,
    minor: 159,
    patch: 4,
};

/// ★★★ The ABI table for a **measured** driver version, assembled from the measured matrix.
///
/// Exact membership, never "newest ≤": a version that is not one of
/// [`crate::generated::matrix::MEASURED`] is [`AbiError::Unmeasured`] (see [`crate::matrix`]
/// for why a release between two measured tags must not borrow either neighbour's layouts),
/// and a measured version whose layouts no decoder here speaks is refused by name with the
/// fact that differs ([`AbiError::NoEncoding`]). The tables are built once per process.
///
/// # Errors
///
/// [`AbiError::Unmeasured`], [`AbiError::NoCapabilityRow`], [`AbiError::NoEncoding`].
pub fn table_for(version: DriverVersion) -> Result<&'static DriverAbiTable, AbiError> {
    use crate::generated::matrix::MEASURED;
    static CACHE: std::sync::OnceLock<Vec<Result<DriverAbiTable, AbiError>>> =
        std::sync::OnceLock::new();
    let all = CACHE.get_or_init(|| MEASURED.iter().map(|v| derive_table(*v)).collect());
    let i = MEASURED
        .binary_search(&version)
        .map_err(|_| AbiError::Unmeasured {
            major: version.major,
            minor: version.minor,
            patch: version.patch,
        })?;
    all[i].as_ref().map_err(|e| *e)
}

fn no_encoding(what: &'static str, v: DriverVersion) -> AbiError {
    AbiError::NoEncoding {
        what,
        major: v.major,
        minor: v.minor,
        patch: v.patch,
    }
}

/// The measured layout of `runs` at `v`, which a measured `v` always has unless the struct
/// is absent at that tag (then `NoEncoding` naming it).
fn measured(
    runs: &'static crate::matrix::StructRuns,
    v: DriverVersion,
) -> Result<crate::matrix::Resolved, AbiError> {
    crate::matrix::Resolved::of(runs, v).map_err(|_| no_encoding(runs.name, v))
}

fn off(
    r: &crate::matrix::Resolved,
    path: &'static str,
    what: &'static str,
) -> Result<usize, AbiError> {
    r.maybe(path)
        .map(|f| f.off())
        .ok_or(no_encoding(what, r.version))
}

/// Assemble one measured version's table. Every field below is either READ from the
/// matrix or DERIVED from what was read by a rule stated at the field; nothing is typed.
fn derive_table(v: DriverVersion) -> Result<DriverAbiTable, AbiError> {
    use crate::generated::matrix as m;

    let caps_row =
        CAPS_ROWS
            .iter()
            .rev()
            .find(|r| r.from <= v)
            .ok_or(AbiError::NoCapabilityRow {
                major: v.major,
                minor: v.minor,
                patch: v.patch,
            })?;

    // NVOS46: the two shapes this crate decodes, told apart by the measured `status` offset
    // (the field whose misplacement was the C artifact's bug #81).
    let nvos46 = measured(&m::NVOS46_PARAMETERS, v)?;
    let map_dma = match (
        nvos46.size(),
        off(&nvos46, "status", "NVOS46_PARAMETERS.status")?,
    ) {
        (56, 48) => MapDmaWire::Pre580_65_06,
        (64, 56) => MapDmaWire::From580_65_06,
        _ => return Err(no_encoding("NVOS46_PARAMETERS", v)),
    };

    // The GSP queue element: 48-byte (elemCount) or 16-byte MCTP/NVDM, each checked field by
    // field against the measured layout; a third shape (615.71.09's encryption union) is
    // refused by name.
    let el = measured(&m::GSP_MSG_QUEUE_ELEMENT, v)?;
    let gsp_element = if el.maybe("elemCount").is_some() {
        let pre = GspElementWire::Pre610;
        let ok = off(&el, "checkSum", "GSP_MSG_QUEUE_ELEMENT.checkSum")? == pre.checksum_off()
            && off(&el, "seqNum", "GSP_MSG_QUEUE_ELEMENT.seqNum")? == pre.seqnum_off()
            && Some(off(&el, "elemCount", "GSP_MSG_QUEUE_ELEMENT.elemCount")?)
                == pre.elem_count_off()
            && off(&el, "rpc", "GSP_MSG_QUEUE_ELEMENT.rpc")? == pre.hdr_size();
        if !ok {
            return Err(no_encoding("GSP_MSG_QUEUE_ELEMENT", v));
        }
        pre
    } else {
        let new = GspElementWire::From610_43_02;
        let t = new
            .transport()
            .ok_or(no_encoding("GSP_MSG_QUEUE_ELEMENT", v))?;
        let ok = el.maybe("mctpHeader").map(|f| f.off()) == Some(t.header_off)
            && el.maybe("nvdmHeader").map(|f| f.off()) == Some(t.nvdm_off)
            && el.maybe("checkSum").map(|f| f.off()) == Some(new.checksum_off())
            && el.maybe("seqNum").map(|f| f.off()) == Some(new.seqnum_off())
            && el.maybe("payload").map(|f| f.off()) == Some(new.hdr_size());
        if !ok {
            return Err(no_encoding("GSP_MSG_QUEUE_ELEMENT", v));
        }
        new
    };

    // MESSAGE_QUEUE_INIT_ARGUMENTS: the four fields every version shares sit at 0/8/16/24
    // (checked); the geometry fields exist only where the struct declares them.
    let qa = measured(&m::MESSAGE_QUEUE_INIT_ARGUMENTS, v)?;
    for (path, want) in [
        ("sharedMemPhysAddr", 0usize),
        ("pageTableEntryCount", 8),
        ("cmdQueueOffset", 16),
        ("statQueueOffset", 24),
    ] {
        if off(&qa, path, "MESSAGE_QUEUE_INIT_ARGUMENTS")? != want {
            return Err(no_encoding("MESSAGE_QUEUE_INIT_ARGUMENTS", v));
        }
    }
    let gsp_init_args = match qa.maybe("queueElementHdrSize") {
        None => GspInitArgsWire::FourField,
        Some(f) if f.off() == 32 => GspInitArgsWire::NineField,
        Some(_) => {
            return Err(no_encoding(
                "MESSAGE_QUEUE_INIT_ARGUMENTS.queueElementHdrSize",
                v,
            ));
        }
    };

    // GspStaticConfigInfo: the hand encoder (`crate::gspstaticinfo`) was pinned against
    // 580.159.04's reply, so it is right exactly where the measured layout equals that one.
    let sci = measured(&m::GSPSTATICCONFIGINFO, v)?;
    let sci_bench = measured(&m::GSPSTATICCONFIGINFO, BENCH_DRIVER)?;
    let gsp_static_info = if sci.layout == sci_bench.layout {
        GspStaticInfoWire::Pre610
    } else if v.major >= 610 {
        GspStaticInfoWire::From610_43_02
    } else {
        GspStaticInfoWire::Unencoded
    };

    // The vGPU handshake pair — measured per tag (it moves INSIDE a branch: 570.124.06 says
    // 0x29/0x0B, 570.148.08 says 0x29/0x0C).
    let vgx = match (
        m::VGX_VERSION_VGX_MAJOR_VERSION_NUMBER
            .at_u32(v)
            .ok()
            .flatten(),
        m::VGX_VERSION_VGX_MINOR_VERSION_NUMBER
            .at_u32(v)
            .ok()
            .flatten(),
    ) {
        (Some(major), Some(minor)) => Some(VgxVersion { major, minor }),
        _ => None,
    };

    // NV_CHANNEL_ALLOC_PARAMS tail offsets — read, not recognised: whatever the tag measures
    // IS the wire (V580's constants hold at every tag 535.309.01 … 595.84, V610's at 610.x).
    let ch = measured(&m::NV_CHANNEL_ALLOC_PARAMS, v)?;
    let channel_notifier = match (ch.maybe("internalFlags"), ch.maybe("errorNotifierMem")) {
        (Some(a), Some(b)) => Some(ChannelNotifierWire {
            internal_flags: a.off(),
            error_notifier_mem: b.off(),
        }),
        _ => None,
    };
    let channel_userd = match (ch.maybe("hUserdMemory"), ch.maybe("userdOffset")) {
        (Some(a), Some(b)) => Some(ChannelUserdWire {
            h_userd_memory: a.off(),
            userd_offset: b.off(),
        }),
        _ => None,
    };
    let channel_userd_mem = ch
        .maybe("userdMem")
        .map(|f| ChannelUserdMemWire { userd_mem: f.off() });
    let channel_engine = ch.maybe("engineType").map(|f| ChannelEngineWire {
        engine_type: f.off(),
    });

    // `rpc_gsp_rm_control_v`: 24-byte header with `flags`@20 through 570.x, 40-byte with
    // `rmapiRpcFlags`@20 / `rmctrlFlags`@24 / `rmctrlAccessRight`@28 from 575.51.02.
    let rc = measured(&m::RPC_GSP_RM_CONTROL_V, v)?;
    let rm_control = RmControlWire {
        params_off: off(&rc, "params", "rpc_gsp_rm_control_v.params")?,
        rpc_flags_off: rc
            .maybe("rmapiRpcFlags")
            .or_else(|| rc.maybe("flags"))
            .map(|f| f.off())
            .ok_or(no_encoding("rpc_gsp_rm_control_v.flags", v))?,
        rmctrl_flags_off: rc.maybe("rmctrlFlags").map(|f| f.off()),
        access_right_off: rc.maybe("rmctrlAccessRight").map(|f| f.off()),
        params_size_off: off(&rc, "paramsSize", "rpc_gsp_rm_control_v.paramsSize")?,
        status_off: off(&rc, "status", "rpc_gsp_rm_control_v.status")?,
    };

    Ok(DriverAbiTable {
        version: v,
        map_dma,
        gsp_element,
        gsp_init_args,
        gsp_static_info,
        caps: caps_row.caps,
        vbios: VbiosWire::Tu102Bit,
        vgx,
        channel_notifier,
        channel_userd,
        channel_userd_mem,
        channel_engine,
        rm_control,
        note: caps_row.note,
    })
}

impl DriverAbiTable {
    /// ★ The **default-deny capability surface** for this driver version — which
    /// control commands and which allocation classes the guest may name at all
    /// ([`crate::capability`]).
    ///
    /// The one gate is at the guest ingress (`kayfabe_rmrpc::translate`), and this
    /// is where it reads its answer from.
    #[must_use]
    pub fn capabilities(&self) -> &'static CapabilityTable {
        self.caps
    }

    /// Which synthetic-VBIOS parse path this version speaks
    /// ([`crate::vbios::build`]'s `wire` argument).
    #[must_use]
    pub fn vbios_wire(&self) -> VbiosWire {
        self.vbios
    }

    /// The vGPU RPC version this driver speaks, or `None` where this port has no citation
    /// for it — see the field's own doc for why that is a refusal and not a default.
    #[must_use]
    pub fn vgx_version(&self) -> Option<VgxVersion> {
        self.vgx
    }

    /// Which `GSP_MSG_QUEUE_ELEMENT` shape this version speaks.
    #[must_use]
    pub fn gsp_element_wire(&self) -> GspElementWire {
        self.gsp_element
    }

    /// Which `MESSAGE_QUEUE_INIT_ARGUMENTS` shape this version publishes.
    #[must_use]
    pub fn gsp_init_args_wire(&self) -> GspInitArgsWire {
        self.gsp_init_args
    }

    /// Which `GspStaticConfigInfo` shape this version reads a fn-65 reply as.
    #[must_use]
    pub fn gsp_static_info_wire(&self) -> GspStaticInfoWire {
        self.gsp_static_info
    }

    /// `GSP_MSG_QUEUE_ELEMENT_SIZE_MIN` — `RM_PAGE_SIZE`, the granularity a run is
    /// counted and copied in (`ogkm-580: message_queue_priv.h:91`,
    /// `ogkm-610: message_queue_priv.h:112`). A *driver* page size, not the host's.
    ///
    /// Identical at both vendored tags; carried here rather than in a logic crate because
    /// it is a driver constant, and declared per-version so the day it moves it is a data
    /// edit with a version behind it.
    #[must_use]
    pub fn gsp_element_size_min(&self) -> u32 {
        4096
    }

    /// `GSP_MSG_QUEUE_ELEMENT_SIZE_MAX` = `SIZE_MIN * 16`, and — at 580 — also the exact
    /// size of the receive **staging buffer** the guest copies a run into
    /// (`ogkm-580: message_queue_priv.h:92`, carve at
    /// `ogkm-580: message_queue_cpu.c:132-134, 143-145`).
    #[must_use]
    pub fn gsp_element_size_max(&self) -> u32 {
        self.gsp_element_size_min() * 16
    }

    /// Which `NVOS46_PARAMETERS` shape this version speaks.
    #[must_use]
    pub fn map_dma_wire(&self) -> MapDmaWire {
        self.map_dma
    }

    /// `sizeof(NVOS46_PARAMETERS)` for this version — the value an ioctl's size
    /// field must match.
    #[must_use]
    pub fn map_dma_size(&self) -> usize {
        match self.map_dma {
            MapDmaWire::Pre580_65_06 => Nvos46ParametersPre580::SIZE,
            MapDmaWire::From580_65_06 => nvos::Nvos46Parameters::SIZE,
        }
    }

    /// Offset of `NVOS46_PARAMETERS::status` for this version.
    ///
    /// The C artifact carries the same two numbers by hand
    /// (`nvkvm_abi.h:66,76,86`: 48, 48, 56) because writing the status to the
    /// wrong offset was bug `#81`.
    #[must_use]
    pub fn map_dma_status_offset(&self) -> usize {
        match self.map_dma {
            MapDmaWire::Pre580_65_06 => 48,
            MapDmaWire::From580_65_06 => 56,
        }
    }

    /// Decode `NV_ESC_RM_FREE` parameters.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_free(&self, bytes: &[u8]) -> Result<FreeReq, AbiError> {
        let p = nvos::Nvos00Parameters::decode(bytes)?;
        Ok(FreeReq {
            client: p.h_root,
            parent: p.h_object_parent,
            handle: p.h_object_old,
        })
    }

    /// Decode `NV_ESC_RM_ALLOC` parameters in the v1 (`NVOS21`) shape.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_alloc_v1(&self, bytes: &[u8]) -> Result<AllocReq, AbiError> {
        let p = nvos::Nvos21Parameters::decode(bytes)?;
        Ok(AllocReq {
            client: p.h_root,
            parent: p.h_object_parent,
            handle: p.h_object_new,
            class: p.h_class,
            params_ptr: p.p_alloc_parms,
            // Declared absent, exactly as nvproxy declares it
            // (`frontend.go:322-324` returns 0 rather than panicking).
            rights_requested: 0,
            params_size: p.params_size,
            wire: AllocWire::V1,
        })
    }

    /// Decode `NV_ESC_RM_ALLOC` parameters in the v2 (`NVOS64`) shape.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_alloc_v2(&self, bytes: &[u8]) -> Result<AllocReq, AbiError> {
        let p = nvos::Nvos64Parameters::decode(bytes)?;
        Ok(AllocReq {
            client: p.h_root,
            parent: p.h_object_parent,
            handle: p.h_object_new,
            class: p.h_class,
            params_ptr: p.p_alloc_parms,
            rights_requested: p.p_rights_requested,
            params_size: p.params_size,
            wire: AllocWire::V2,
        })
    }

    /// Decode `NV_ESC_RM_ALLOC` by the size the **ioctl** declared, which is how
    /// RM itself and nvproxy discriminate the two shapes
    /// (`gvisor/pkg/abi/nvgpu/frontend.go:290-295`, `GetRmAllocParamObj(isNVOS64)`).
    ///
    /// The discriminator is the ioctl's own size word, **not** `bytes.len()`:
    /// the buffer may legitimately be longer, and choosing the shape from a
    /// length the guest controls indirectly is how you get a 32-byte struct
    /// parsed as a 48-byte one.
    ///
    /// # Errors
    ///
    /// [`AbiError::UnknownAllocWire`] if `ioctl_size` is neither shape's size;
    /// [`AbiError::Truncated`] if the buffer is short.
    pub fn decode_alloc(&self, bytes: &[u8], ioctl_size: usize) -> Result<AllocReq, AbiError> {
        if ioctl_size == nvos::Nvos21Parameters::SIZE {
            self.decode_alloc_v1(bytes)
        } else if ioctl_size == nvos::Nvos64Parameters::SIZE {
            self.decode_alloc_v2(bytes)
        } else {
            Err(AbiError::UnknownAllocWire { ioctl_size })
        }
    }

    /// Decode `NV_ESC_RM_CONTROL` parameters.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_control(&self, bytes: &[u8]) -> Result<ControlReq, AbiError> {
        let p = nvos::Nvos54Parameters::decode(bytes)?;
        Ok(ControlReq {
            client: p.h_client,
            object: p.h_object,
            cmd: p.cmd,
            flags: p.flags,
            params_ptr: p.params,
            params_size: p.params_size,
        })
    }

    /// Decode `NV_ESC_RM_DUP_OBJECT` parameters.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_dup(&self, bytes: &[u8]) -> Result<DupReq, AbiError> {
        let p = nvos::Nvos55Parameters::decode(bytes)?;
        Ok(DupReq {
            dst_client: p.h_client,
            dst_parent: p.h_parent,
            dst_handle: p.h_object,
            src_client: p.h_client_src,
            src_handle: p.h_object_src,
            flags: p.flags,
        })
    }

    /// Decode `NV_ESC_RM_MAP_MEMORY_DMA` parameters **in this version's shape**.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`], with `need` reporting this version's size — so a
    /// 56-byte buffer against a 580.65.06+ table says `need 64, got 56` rather
    /// than succeeding with `dmaOffset` read out of `kindOverride`.
    pub fn decode_map_memory_dma(&self, bytes: &[u8]) -> Result<MapMemoryDma, AbiError> {
        match self.map_dma {
            MapDmaWire::Pre580_65_06 => {
                let p = Nvos46ParametersPre580::decode(bytes)?;
                Ok(MapMemoryDma {
                    client: p.h_client,
                    device: p.h_device,
                    dma: p.h_dma,
                    memory: p.h_memory,
                    offset: p.offset,
                    length: p.length,
                    flags: p.flags,
                    dma_offset: p.dma_offset,
                })
            }
            MapDmaWire::From580_65_06 => {
                let p = nvos::Nvos46Parameters::decode(bytes)?;
                Ok(MapMemoryDma {
                    client: p.h_client,
                    device: p.h_device,
                    dma: p.h_dma,
                    memory: p.h_memory,
                    offset: p.offset,
                    length: p.length,
                    flags: p.flags,
                    dma_offset: p.dma_offset,
                })
            }
        }
    }

    /// Write the two `[OUT]` fields of `NV_ESC_RM_MAP_MEMORY_DMA` back into the
    /// guest's buffer, at **this version's** offsets.
    ///
    /// Only `dmaOffset` and `status` are written. Every other byte — including
    /// the `[IN]` fields, the padding, and (on 580.65.06+) `flags2` and
    /// `kindOverride` — is left exactly as the guest wrote it. That is the
    /// `writeback_bug_pattern` rule: a writer that rewrites the whole struct
    /// hands the caller back whatever the emulator happened to have in the
    /// field, and CUDA reads it as its own.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if the buffer is shorter than this version's
    /// struct.
    pub fn write_map_memory_dma_result(
        &self,
        bytes: &mut [u8],
        dma_offset: u64,
        status: u32,
    ) -> Result<(), AbiError> {
        let (dma_off_at, status_at, size) = match self.map_dma {
            MapDmaWire::Pre580_65_06 => (40usize, 48usize, Nvos46ParametersPre580::SIZE),
            MapDmaWire::From580_65_06 => (48usize, 56usize, nvos::Nvos46Parameters::SIZE),
        };
        let len = bytes.len();
        if len < size {
            return Err(AbiError::Truncated {
                c_name: "NVOS46_PARAMETERS",
                need: size,
                got: len,
            });
        }
        let d = dma_offset.to_le_bytes();
        let s = status.to_le_bytes();
        bytes
            .get_mut(dma_off_at..dma_off_at + 8)
            .ok_or(AbiError::Truncated {
                c_name: "NVOS46_PARAMETERS",
                need: size,
                got: len,
            })?
            .copy_from_slice(&d);
        bytes
            .get_mut(status_at..status_at + 4)
            .ok_or(AbiError::Truncated {
                c_name: "NVOS46_PARAMETERS",
                need: size,
                got: len,
            })?
            .copy_from_slice(&s);
        Ok(())
    }

    /// Decode `NV_ESC_RM_UNMAP_MEMORY_DMA` parameters.
    ///
    /// Not versioned within the supported range: `NVOS47_PARAMETERS` took its
    /// current 48-byte shape at 550.54.04, which is this crate's floor.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_unmap_memory_dma(&self, bytes: &[u8]) -> Result<UnmapMemoryDma, AbiError> {
        let p = nvos::Nvos47Parameters::decode(bytes)?;
        Ok(UnmapMemoryDma {
            client: p.h_client,
            device: p.h_device,
            dma: p.h_dma,
            memory: p.h_memory,
            flags: p.flags,
            dma_offset: p.dma_offset,
            size: p.size,
        })
    }

    /// Decode the client-root alloc params under the **prefix contract** — see
    /// [`ClientAllocFacts`] for why it is 8 bytes and not 120.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if fewer than 8 bytes are available.
    pub fn decode_client_alloc_facts(&self, bytes: &[u8]) -> Result<ClientAllocFacts, AbiError> {
        if bytes.len() < CLIENT_ALLOC_PREFIX {
            return Err(AbiError::Truncated {
                c_name: classes::Nv0000AllocParameters::C_NAME,
                need: CLIENT_ALLOC_PREFIX,
                got: bytes.len(),
            });
        }
        Ok(ClientAllocFacts {
            h_client: u32_at(bytes, 0)?,
            process_id: u32_at(bytes, 4)?,
        })
    }

    /// Decode the Device alloc params.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_device_alloc_facts(&self, bytes: &[u8]) -> Result<DeviceAllocFacts, AbiError> {
        let p = classes::Nv0080AllocParameters::decode(bytes)?;
        Ok(DeviceAllocFacts {
            device_id: p.device_id,
            h_client_share: p.h_client_share,
            h_target_client: p.h_target_client,
            h_target_device: p.h_target_device,
            flags: p.flags,
            va_space_size: p.va_space_size,
            va_mode: p.va_mode,
        })
    }

    /// Decode the TSG alloc params.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_tsg_alloc_facts(&self, bytes: &[u8]) -> Result<TsgAllocFacts, AbiError> {
        let p = classes::NvChannelGroupAllocationParameters::decode(bytes)?;
        Ok(TsgAllocFacts {
            h_vaspace: p.h_va_space,
            engine_type: p.engine_type,
        })
    }

    /// Decode the CtxShare (subcontext) alloc params.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_ctxshare_alloc_facts(
        &self,
        bytes: &[u8],
    ) -> Result<CtxShareAllocFacts, AbiError> {
        let p = classes::NvCtxshareAllocationParameters::decode(bytes)?;
        Ok(CtxShareAllocFacts {
            h_vaspace: p.h_va_space,
        })
    }

    /// ★★★★ **§16.28 — the `index` field of `NV_VASPACE_ALLOCATION_PARAMETERS`**, the one
    /// wire fact that separates *"create a new address space"* from *"acquire a reference
    /// to the Device's existing one"*. See
    /// [`crate::bringup::NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE`] for the RM chain that
    /// emits the second kind, publishes its page-directory root, and then frees the handle.
    ///
    /// # ⊘ It returns an `Option` and NOT a `Result`, deliberately
    ///
    /// `None` means **this port could not read the field** — params shorter than four
    /// bytes — and never *"the guest declared index 0"*. The distinction matters in one
    /// direction only, and it is the direction that has bitten this project twice
    /// (`accuracy_is_fatal_when_a_fallback_was_keyed_on_ignorance`): `FERMI_VASPACE_A` is a
    /// class the bridge **accepts today** with no decoder at all, so a fallible decoder
    /// here would turn a short or absent params block into a refused VA-space alloc — a
    /// class going from working to refused because somebody wrote a reader for it.
    ///
    /// ⊘ Nothing validates the value: an `index` this port does not recognise is carried
    /// through as itself, and only an exact equality with
    /// [`crate::bringup::NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE`] means anything downstream.
    #[must_use]
    pub fn decode_vaspace_index(&self, bytes: &[u8]) -> Option<u32> {
        u32_at(bytes, 0).ok()
    }

    /// Decode the channel alloc params under the **prefix contract** — see
    /// [`ChannelAllocFacts`] for the 580-vs-610 divergence that makes it one, and
    /// [`CHANNEL_ALLOC_PREFIX`] for the bound.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if fewer than [`CHANNEL_ALLOC_PREFIX`] bytes are
    /// available. Never a zero-extended partial decode: a channel whose params
    /// stop short of `hVASpace` has not declared one, and reading absence as
    /// `hVASpace = 0` would silently turn a malformed message into a legal
    /// "GSP-managed VAS" declaration.
    pub fn decode_channel_alloc_facts(&self, bytes: &[u8]) -> Result<ChannelAllocFacts, AbiError> {
        if bytes.len() < CHANNEL_ALLOC_PREFIX {
            return Err(AbiError::Truncated {
                c_name: CHANNEL_ALLOC_C_NAME,
                need: CHANNEL_ALLOC_PREFIX,
                got: bytes.len(),
            });
        }
        Ok(ChannelAllocFacts {
            // +8 and +16 are INSIDE the agreeing prefix (`hObjectError` @0,
            // `hObjectBuffer` @4, `gpFifoOffset` @8, `gpFifoEntries` @16), so reading
            // them costs the version contract nothing. See `ChannelAllocFacts`.
            gp_fifo_offset: u64_at(bytes, 8)?,
            gp_fifo_entries: u32_at(bytes, 16)?,
            flags: u32_at(bytes, 20)?,
            h_ctx_share: u32_at(bytes, 24)?,
            h_vaspace: u32_at(bytes, 28)?,
        })
    }

    /// ★★★ Decode the channel's declared **error notifier** — where the GSP is
    /// contracted to write when it RCs this channel (`crate::notifier`).
    ///
    /// Separate from [`Self::decode_channel_alloc_facts`] rather than folded into it, and
    /// that separation is the version seam: the facts decoder reads only the +0..+32
    /// region both vendored trees spell identically, while these two fields sit in the
    /// region that **moves**. Keeping them apart is what lets a boundary answer
    /// `Ok(None)` for the notifier without weakening the prefix contract for everything
    /// else.
    ///
    /// `Ok(None)` means *this port cannot learn a notifier for this channel* and covers
    /// two different situations, deliberately merged here and split one level up by
    /// [`ErrorNotifier`]'s own variants: the boundary has no pinned layout (the tree was
    /// never read), or the channel declared no notifier at all.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if a pinned boundary's params stop short of the fields.
    pub fn decode_channel_error_notifier(
        &self,
        bytes: &[u8],
    ) -> Result<Option<ErrorNotifier>, AbiError> {
        match self.channel_notifier {
            Some(wire) => wire.decode(bytes),
            None => Ok(None),
        }
    }

    /// ★★★★★ Decode the privilege level the guest's CPU-RM stamped on a channel alloc
    /// (`internalFlags`, [`crate::notifier::ChannelPrivilege`]) — the `V3_P5_PORT_MAP.md` Q7
    /// identity fact. `Ok(None)`: no pinned layout for this boundary, or the params stop short.
    ///
    /// # Errors
    /// [`AbiError`] from the primitive reader (unreachable past the length check).
    pub fn decode_channel_privilege(
        &self,
        bytes: &[u8],
    ) -> Result<Option<crate::notifier::ChannelPrivilege>, AbiError> {
        match self.channel_notifier {
            Some(wire) => wire.decode_privilege(bytes),
            None => Ok(None),
        }
    }

    /// ★★★★ §16.16 — decode the channel's declared **USERD** handle and offset.
    ///
    /// Separate from [`Self::decode_channel_alloc_facts`] for
    /// [`Self::decode_channel_error_notifier`]'s reason, which is the same reason: these
    /// two fields live past the +32 prefix both vendored trees agree on, in the region 610
    /// moves. `Ok(None)` means *this port cannot learn a USERD for this channel* — the
    /// boundary has no pinned layout, or the params stopped short — and is never a claim
    /// that the channel declared none. See [`ChannelUserdWire`].
    ///
    /// # Errors
    /// [`AbiError::Truncated`] from the primitive readers, which the wire's own length
    /// check makes unreachable.
    pub fn decode_channel_userd(&self, bytes: &[u8]) -> Result<Option<(u32, u64)>, AbiError> {
        match self.channel_userd {
            Some(wire) => wire.decode(bytes),
            None => Ok(None),
        }
    }

    /// ★★★★★ Decode the channel's **resolved** USERD descriptor — `userdMem`, the physical
    /// address the guest's own CPU-RM put on the wire for the GSP.
    ///
    /// Separate from [`Self::decode_channel_userd`] because the two answer different
    /// questions about different parties' numbers: that one reports what the guest's
    /// *client* declared (a handle in a namespace we cannot resolve), this one reports what
    /// the guest's *kernel* resolved it to. ⊘ They are not two projections of one fact and
    /// must not be folded: a channel can declare `hUserdMemory[0] = 0` and still carry a
    /// filled `userdMem`, and a channel whose params stop short carries neither.
    ///
    /// `Ok(None)` = *this port cannot learn a resolved USERD for this channel*. See
    /// [`ChannelUserdMemWire`].
    ///
    /// # Errors
    /// [`AbiError::Truncated`] from the primitive readers, which the wire's own length
    /// check makes unreachable.
    pub fn decode_channel_userd_mem(&self, bytes: &[u8]) -> Result<Option<UserdMem>, AbiError> {
        match self.channel_userd_mem {
            Some(wire) => wire.decode(bytes),
            None => Ok(None),
        }
    }

    /// ★★★★★ Decode the channel's declared **engine** — `NV_CHANNEL_ALLOC_PARAMS.engineType`
    /// narrowed to the vocabulary the core speaks.
    ///
    /// Separate from [`Self::decode_channel_alloc_facts`] for
    /// [`Self::decode_channel_error_notifier`]'s reason and by the same mechanism: the
    /// field lives past the +32 prefix, in the region 610 moves.
    ///
    /// `Ok(None)` merges three situations, and the merge is safe **only** because the
    /// consumer's fallback is the behaviour that shipped before this decoder existed: the
    /// boundary has no pinned layout, the params stopped short, or the declared code is one
    /// this port does not recognise. ⊘ It is never *"the guest declared GR"* — that is the
    /// `dlen=0` lesson, and it is the reading that would make this decoder worse than its
    /// absence, because the guess it replaced is at least labelled a guess.
    ///
    /// # Errors
    /// [`AbiError`] from the primitive readers, which the wire's own length check makes
    /// unreachable.
    pub fn decode_channel_engine(
        &self,
        bytes: &[u8],
    ) -> Result<Option<kf_arch::ids::EngineKind>, AbiError> {
        match self.channel_engine {
            Some(wire) => wire.decode_kind(bytes),
            None => Ok(None),
        }
    }

    /// ★★★★★ **w393 — the declared `NV2080_ENGINE_TYPE_*` code, RAW**, beside
    /// [`Self::decode_channel_engine`]'s narrowed reading of the same four bytes.
    ///
    /// # ⊘ Why the raw code crosses this seam at all, when decision #2 says numbers stay here
    ///
    /// [`Self::decode_channel_engine`] narrows `COPY2` to [`kf_arch::ids::EngineKind::Ce`]
    /// and the copy-engine **instance** is lost. That was fine while every host channel was
    /// born at the engine-object latch, where `declared_copy_engine_type` recovers the
    /// instance from the CE *object's* params (§16.106). A channel born **at the guest's own
    /// channel alloc** has no object yet — and a birth that fell back to `COPY0` there would
    /// re-create §16.106's 14 measured `kfifoRunlistSetId_GM107` refusals, one rung earlier.
    /// The guest stated the instance in this very message; carrying it verbatim is the only
    /// answer that is not a guess.
    ///
    /// ⊘ It is carried as an **opaque** `u32` and interpreted by nobody above this crate:
    /// the core files it on the channel's graph node and the isolate adapter hands it back
    /// to RM as the same `engineType` the guest wrote. Nothing branches on its value.
    ///
    /// `Ok(None)` for exactly [`Self::decode_channel_engine`]'s three reasons.
    ///
    /// # Errors
    /// [`AbiError`] from the primitive readers, which the wire's own length check makes
    /// unreachable.
    pub fn decode_channel_engine_type(&self, bytes: &[u8]) -> Result<Option<u32>, AbiError> {
        match self.channel_engine {
            Some(wire) => wire.decode(bytes),
            None => Ok(None),
        }
    }

    /// Which alloc-params shape a class carries — the **class table**, and the
    /// only thing that decides which decoder above an alloc goes through.
    ///
    /// `None` means *this port has not mapped that class*, which is a different
    /// statement from "it declares nothing": [`AllocParams::NoDeclaredFacts`] is
    /// the second one, and it is a decision with a citation behind it rather
    /// than an absence.
    ///
    /// Lives here, not in the bridge above, for decision #2's quarantine reason:
    /// the NVIDIA class *numbers* are this crate's and the crates above speak a
    /// vocabulary. Same shape as [`Self::is_client_root_class`] and
    /// [`crate::GuestOs::client_kind_from_process_id`].
    #[must_use]
    pub fn alloc_params(&self, class: ClassId) -> Option<AllocParams> {
        if self.is_client_root_class(class) {
            return Some(AllocParams::ClientRoot);
        }
        match class.0 {
            classes::NV01_DEVICE_0 => Some(AllocParams::Device),
            classes::KEPLER_CHANNEL_GROUP_A => Some(AllocParams::Tsg),
            classes::FERMI_CONTEXT_SHARE_A => Some(AllocParams::CtxShare),
            classes::AMPERE_CHANNEL_GPFIFO_A => Some(AllocParams::Channel),
            // ★★★★ §16.28 — **`FERMI_VASPACE_A` MOVED OFF `NoDeclaredFacts`, and the
            // comment it used to sit under was WRONG in the load-bearing direction.**
            //
            // That comment read: *"A VASpace's params are geometry (`index`, `vaSize`,
            // `vaBase`, `pasid`) … the protocol content is the EDGE"*. `index` is **not**
            // geometry. `NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE` (`= 0x3`) is documented
            // in NVIDIA's own header as *"Acquire reference to device vaspace"*
            // (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:3187`), i.e. it is the field
            // that says **this alloc creates no address space at all** — it is a
            // transient NAME for one the Device already owns.
            //
            // ⊘ `a_wrong_comment_is_why_nobody_looked`: four rungs of the §16 campaign
            // searched for the walling channel's VA space while the one wire field that
            // identifies it sat behind a comment asserting it was geometry.
            //
            // ⚠ **Acceptance is unchanged.** [`DriverAbiTable::decode_vaspace_index`]
            // cannot fail: params shorter than four bytes yield `None` (*"this port could
            // not read the field"*), never a refusal. A class this port accepts today
            // must not become one it rejects because a decoder was written for it —
            // `accuracy_is_fatal_when_a_fallback_was_keyed_on_ignorance`.
            classes::FERMI_VASPACE_A => Some(AllocParams::VaSpace),
            // ★ Mapped, and declaring nothing the object model reads. An engine
            // object's params are engine-private; the protocol content is the EDGE —
            // parent, handle, class — which the RPC header already carries.
            classes::AMPERE_COMPUTE_B
            | classes::AMPERE_DMA_COPY_B
            // ★★★ `AMPERE_B` (`0xc797`), the GA10x **3D** object, joins its compute
            // sibling — and it is here because a boot asked for it, not because the
            // table was being completed. `[measured 2026-08-08, boot pro1_423bf08]`:
            // once `GPU_PROMOTE_CTX` started succeeding, `kgrobjConstruct` stopped
            // failing locally and the golden-image channel's 3D object reached the
            // wire for the FIRST time in any capture —
            // `hClass=0x0000c797 paramsSize=0x00000000 status=0x00000056`
            // (`run_pro1_423bf08_dmesg.log:11`), refused as `UnmappedAllocClass`.
            //
            // ⚠ `NoDeclaredFacts` is the STRONG reading here, not a shrug, and RM's own
            // resource table says so: `AMPERE_B` registers its params as
            // **`RS_OPTIONAL(NV_GR_ALLOCATION_PARAMETERS)`**
            // (`ogkm-580: src/nvidia/src/kernel/rmapi/resource_list.h:2010`), which
            // expands to `{ sizeof(x), bParamRequired = NV_FALSE }`
            // (`resource_desc.c:76`) — a NULL is legal by declaration, not by accident.
            // The struct itself is `{version, flags, size, caps}`, 4 x NvU32, **no handle
            // and no pointer** (`ogkm-580: nvos.h:2716-2721`), `caps` is an *output* the
            // caller reads back rather than a fact it states, and ★
            // `grep -rn NV_GR_ALLOCATION_PARAMETERS src/nvidia/src/kernel/gpu/gr/` finds
            // **nothing**: no GR code reads it on the alloc path at all. The one
            // allocator on this boot path supplies none of it —
            // `AllocWithHandle(…, hObj3D, classNum, NULL, 0)`
            // (`ogkm-580: kernel_graphics.c:2519-2521`) — which is why the wire says
            // `paramsSize=0`.
            //
            // ⊘ **Admitting the class is not serving what the class does.** The alloc's
            // real effect is GSP-side: the physical-RM GR object constructor is where a
            // golden context image gets built, and this port builds none. That is
            // affordable *here* and only here — the guest frees the whole tree three
            // lines later (`kernel_graphics.c:2533`) and never reads an image back
            // through this port, and the C artifact's standing answer for the golden
            // context proper is "silicon boundary, forward GR execution to the host"
            // (`c_cuda_ladder.md` §3). A guest that later runs its OWN GR engine against
            // a forged golden context is the case this row does NOT cover.
            | classes::AMPERE_B
            // ★★★ `GP100_UVM_SW` (`0xc076`) — UVM's per-channel fault-cancel SW object,
            // and the row that decides whether UVM has ANY channel at all.
            // `[measured 2026-08-09, boot s22_f4f3865]`: four refusals in the `cuInit`
            // window, one per UVM channel, each one the LAST call of `channelAllocate`
            // (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:6110-6122`) and each
            // one taking its channel down with it via `goto cleanup_free_controlpage`.
            //
            // ⚠ `NoDeclaredFacts` is the STRONGEST reading on this table, stronger than
            // `AMPERE_B`'s or `NV2081_BINAPI`'s: both of those are `RS_OPTIONAL`, i.e. a
            // struct exists and a NULL is merely legal. This class is registered
            // **`RS_NONE`** (`ogkm-580: resource_list.h:1539`) — no alloc-params struct is
            // declared for it anywhere — and its one allocator passes `NULL, 0`, measured
            // on the wire as `paramsSize=0x00000000`. There is no struct to decode, so
            // "its params are never read" is a property of the ABI here, not a choice.
            //
            // ⊘ Admitting the class is not serving what the class does — see its
            // `capability.rs` row for the scope, which is narrower than `AMPERE_B`'s: the
            // object exists to hold a subchannel for `FAULT_CANCEL_A`, and this port
            // raises no fault for UVM to cancel.
            | classes::GP100_UVM_SW
            // ★★★ `UVM_CHANNEL_RETAINER` (`0xc574`) — and this row's justification is the
            // OPPOSITE shape to every other member of this arm. The others are here because
            // no params struct exists, or because one exists and nothing reads it. This
            // class HAS a declared struct and it is fully read — `NV_UVM_CHANNEL_RETAINER_
            // ALLOC_PARAMS { NvHandle hClient; NvHandle hChannel; }`, 8 bytes, matching the
            // `paramsSize=0x00000008` on the wire
            // (`run_w210_8574466_ctl_probe.log:923-927`).
            //
            // ★★ It is `NoDeclaredFacts` because both members are `[IN]` and the constructor
            // writes **nothing** back, so there is no reply field this port would have to
            // invent — which is the only question this enum decides. ⊘ It is NOT a claim
            // that the params are unread, and the distinction matters: the day something
            // above needs the retained channel as an EDGE, this row becomes a real
            // `AllocParams` shape rather than staying here by inertia.
            //
            // ⊘ `an_echo_is_unverifiable_by_its_reply` is the standing hazard for any
            // echoed body, and it does not bite here for a reason that must be checked per
            // class rather than assumed: an echo hides a decision only when the reply
            // carries a field the caller acts on. This one carries none. ⊘ Never forwarded
            // to the host — the C artifact measured `0x33`.
            | classes::UVM_CHANNEL_RETAINER => {
                Some(AllocParams::NoDeclaredFacts)
            }
            // ★★ The two classes the 2026-08-01 boot measured this table missing, and
            // they join the arm above rather than getting decoders, for two *different*
            // reasons that both end at `NoDeclaredFacts`:
            //
            // - `NV20_SUBDEVICE_0`'s `NV2080_ALLOC_PARAMETERS` has one member,
            //   `subDeviceId`, and the core routes a subdevice by its **Device
            //   ancestor's** `deviceId` (`RmGraph::gpu_of` walks the parent edge). A
            //   field nothing reads is not a fact.
            // - `NV01_EVENT_KERNEL_CALLBACK_EX`'s `NV0005_ALLOC_PARAMETERS` carries an
            //   `NvP64 data` that is a **guest-kernel callback pointer**
            //   (`ogkm-580: cl0005.h:40-47`). ⊘ This port must never decode it: nothing
            //   in the tree dereferences a guest pointer, and the way that stays true is
            //   that no decoder exists to hand one up. `NoDeclaredFacts`'s contract —
            //   *"its params are never read, so a hostile one is bytes we do not look
            //   at"* (`kayfabe_rmrpc::translate_alloc`) — is exactly the property wanted
            //   here, and it is the strong reading of this arm rather than the weak one.
            classes::NV20_SUBDEVICE_0 | classes::NV01_EVENT_KERNEL_CALLBACK_EX => {
                Some(AllocParams::NoDeclaredFacts)
            }
            // ★★★★★ §16.76 — `NV01_EVENT_OS_EVENT` (`0x79`), and this row is a **liveness**
            // row rather than a completeness one.
            //
            // `[measured 2026-08-10, boot w209_ffc80f8_ctl, rev ffc80f8]` seven of these
            // arrive from libcuda's own client and every one is refused
            // `status=0x00000056` — `UnmappedAllocClass`, because this table had no arm for
            // it while `capability.rs` has admitted the class since the beginning. ⚠ Note
            // which gate that is: the class was **permitted** and **undecodable**, so the
            // refusal came from the params table and not from the boundary. `w210`
            // (`8574466`) then removed the guest's give-up and the same process stopped
            // returning from `cuCtxCreate` at all — a bounded failure became an unbounded
            // hang, because nothing supplies the wakeup those seven registrations exist to
            // receive.
            //
            // ⚠ `NoDeclaredFacts` is the STRONG reading, and it is the same reading its
            // sibling `NV01_EVENT_KERNEL_CALLBACK_EX` takes one arm up: the two classes
            // share `NV0005_ALLOC_PARAMETERS`, whose `data` member is an `NvP64`
            // **guest-kernel callback pointer** (`ogkm-580: cl0005.h:40-47`). ⊘ Nothing in
            // this tree dereferences a guest pointer and the way that stays true is that no
            // decoder exists to hand one up — so this class reaches the object model as an
            // EDGE, exactly like its sibling, and the object model learns nothing else.
            //
            // ⊘ **The event registry is NOT this arm's output, and that separation is
            // deliberate.** `kayfabe_device::osevent::OsEventRecorder` reads
            // `(hClient, hEvent, notifyIndex)` off the wire as a `CommandObserver` — a seat
            // whose type makes it unable to answer or to change a reply byte — precisely so
            // that *"the port may post an event to this pair"* and *"the object model
            // decoded these params"* stay two different facts. Folding `notifyIndex` into
            // `AllocFacts` would put a wakeup-plane field on the core's object vocabulary
            // and give this arm a decoder that the pointer argument above says it must not
            // have.
            classes::NV01_EVENT_OS_EVENT => Some(AllocParams::NoDeclaredFacts),
            // ★ P5b — `NV01_MEMORY_VIRTUAL` (`0x70`), the VA RANGE every user `MAP_MEMORY_DMA`
            // names as `hDma`. A GSP client RPCs its alloc even with guest-managed VA *"because
            // virtual ContextDma and the memory destructor depend on it"*
            // (`ogkm-580: virt_mem_range.c:120-133`). `[measured kf3m2]` it was
            // `UnmappedAllocClass` on every raw-client VA space — and the guest read the refusal
            // as success (the params `status` was the request's `0`), so each later free reached
            // the graph as `FreeUnknown`.
            // ⊘ `NoDeclaredFacts`: `NV_MEMORY_VIRTUAL_ALLOCATION_PARAMS {offset, limit,
            // hVASpace}` are all `[IN]` on this path — the guest computes the returned limit
            // itself AFTER the RPC (`virt_mem_range.c:136`), so the echoed params change nothing
            // it reads, and nothing GSP-side is ours to build (the page tables are the guest's).
            crate::bringup::NV01_MEMORY_VIRTUAL => Some(AllocParams::NoDeclaredFacts),
            // ★★★ `NV2081_BINAPI` (`0x2081`) — the class **libcuda** needs, and the first
            // row in this table admitted because an *injection experiment* named it rather
            // than because a boot logged it. `[measured 2026-08-08, real GA106, real
            // libcuda, one status forced at a time]` (`execution_plane_increments.md`
            // §14.27):
            //
            //     refuse alloc 0x2081  -> cuInit(0) = 100
            //     refuse ctrl  0x20810108 (the only control ON it) -> cuInit(0) = 0
            //
            // ⊘ So the class is load-bearing and its opaque control is not — the exact
            // opposite of what §14.26 predicted, and a reminder that *"has no oracle"* is a
            // statement about our instruments while *"is required"* is one about the driver.
            //
            // ⚠ `NoDeclaredFacts` is again the STRONG reading. `NV2081_ALLOC_PARAMETERS` is
            // `{ NvU32 reserved }` — one word, **no handle and no pointer**
            // (`ogkm-580: src/common/sdk/nvidia/inc/class/cl2081.h:36-38`) — registered
            // `RS_OPTIONAL` (`resource_list.h:444`), so a NULL is legal by declaration; and
            // measured on the ioctl boundary, real libcuda sends exactly that: `paramsSize=0`
            // with a NULL pointer. There is nothing here to decode.
            //
            // ⊘ **Admitting the class is not serving what the class does**, and here that
            // gap is unusually wide: `binapiControl_IMPL` forwards every control on this
            // handle to GSP-RM *whole*, without the kernel interpreting it
            // (`ogkm-580: src/nvidia/src/kernel/rmapi/binary_api.c:61-127`), so the object's
            // entire purpose is a tunnel this port does not dig. That is affordable because
            // the one control measured on it is measured NOT to matter; a second control
            // appearing on this handle is a new fact and not covered by this row.
            //
            // ★ The alloc itself is inert on the device: `RS_FLAGS_ALLOC_NON_PRIVILEGED`,
            // `Parents = RS_LIST(classId(Subdevice))` (`resource_list.h:439-448`) — an
            // unprivileged leaf under a Subdevice that allocates no engine, owns no channel
            // and schedules nothing.
            classes::NV2081_BINAPI => Some(AllocParams::NoDeclaredFacts),
            _ => None,
        }
    }

    /// Which params shape a **control command** carries — the control table, the
    /// exact counterpart of [`Self::alloc_params`] and here for the same
    /// decision-#2 reason: the NVIDIA *cmd numbers* are this crate's, and the
    /// bridge above speaks a vocabulary.
    ///
    /// `None` means *this port does not recognise the command at all*, which is
    /// deliberately a different statement from
    /// [`ControlParams::PageDirNotModelled`] — the latter is a command we know
    /// moves a VASpace's page-directory binding and cannot yet express.
    #[must_use]
    pub fn control_params(&self, cmd: ControlCmd) -> Option<ControlParams> {
        match cmd.0 {
            ctrl::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY => Some(ControlParams::SetPageDir),
            ctrl::NV2080_CTRL_CMD_GPU_PROMOTE_CTX => Some(ControlParams::PromoteCtx),
            NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES
            | NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER => {
                Some(ControlParams::VaspacePublishedPdes)
            }
            NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY => Some(ControlParams::PageDirNotModelled),
            _ => None,
        }
    }

    /// Decode the fixed header of a `GSP_RM_CONTROL` **RPC body** (everything
    /// after the 32-byte `rpc_message_header`), i.e. `rpc_gsp_rm_control_v03_00`.
    ///
    /// Not versioned within the supported range —
    /// `ogkm-580: src/nvidia/generated/g_rpc-structures.h:1506-1518` and
    /// `ogkm-610: src/nvidia/generated/g_rpc-structures.h:1423-1435` are the same
    /// list, field for field. It takes the version table anyway, like every other
    /// decoder here.
    ///
    /// ★ Header **only**. `paramsSize` is guest-declared, so slicing `params[]`
    /// with it is a validation the caller owes, and the caller is the one that can
    /// name the refusal with both numbers.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if fewer than [`RpcControlReq::HEADER`] bytes are
    /// available — never a zero-extended partial decode.
    pub fn decode_rpc_control(&self, payload: &[u8]) -> Result<RpcControlReq, AbiError> {
        let w = self.rm_control;
        if payload.len() < w.params_off {
            return Err(AbiError::Truncated {
                c_name: RpcControlReq::C_NAME,
                need: w.params_off,
                got: payload.len(),
            });
        }
        Ok(RpcControlReq {
            client: u32_at(payload, 0)?,
            object: u32_at(payload, 4)?,
            cmd: u32_at(payload, 8)?,
            // `status` is an [OUT] field the guest sends as zero —
            // `rpcWriteCommonHeader` zeroes the whole message buffer before the
            // sender fills it (`ogkm-580: src/nvidia/src/kernel/rmapi/rpc_common.c:149-152`
            // / `ogkm-610: src/nvidia/src/kernel/rmapi/rpc_common.c:149-152` — same lines
            // at both).
            params_size: u32_at(payload, w.params_size_off)?,
            rmapi_rpc_flags: u32_at(payload, w.rpc_flags_off)?,
            // From 575: +24 `rmctrlFlags`, +28 `rmctrlAccessRight` (both sent as 0 by
            // `rpcRmApiControl_GSP`, `ogkm-580: rpc.c:10994-10995`), +32 `reserved0`.
            // Before 575 the header ends at +24 and those bytes are `params[0..16]`.
            params_at: w.params_off,
        })
    }

    /// ★ The caps-control params layout of `cmd` (`NV0080_CTRL_CMD_MSENC_GET_CAPS_V2` /
    /// `_BSP_GET_CAPS_V2`) at THIS version — measured, or `None` where the version has no
    /// such struct (MSENC V2 does not exist at 535/545).
    #[must_use]
    pub fn video_caps_layout(&self, cmd: u32) -> Option<crate::videocaps::CapsLayout> {
        use crate::generated::matrix as m;
        // ⊘ `[matrix]` renamed at 610.43.02: the MSENC/BSP names survive only as `#define`
        // aliases of NVENC/NVDEC (`ctrl0080nvenc.h:96`, `ctrl0080nvdec.h`), which DWARF cannot
        // see — the layout is the old name's where it exists, the new name's after.
        let (runs, renamed) = match cmd {
            crate::videocaps::MSENC_GET_CAPS_V2 => {
                (&m::NV0080_CTRL_MSENC_GET_CAPS_V2_PARAMS, &m::NV0080_CTRL_NVENC_GET_CAPS_V2_PARAMS)
            }
            crate::videocaps::BSP_GET_CAPS_V2 => {
                (&m::NV0080_CTRL_BSP_GET_CAPS_PARAMS_V2, &m::NV0080_CTRL_NVDEC_GET_CAPS_PARAMS_V2)
            }
            _ => return None,
        };
        let l = crate::matrix::Resolved::of(runs, self.version)
            .or_else(|_| crate::matrix::Resolved::of(renamed, self.version))
            .ok()?;
        Some(crate::videocaps::CapsLayout {
            params_size: l.size(),
            caps_len: l.maybe("capsTbl")?.bytes()?,
            instance_off: l.maybe("instanceId")?.off(),
        })
    }

    /// Where this version's `rpc_gsp_rm_control_v` fields sit (MEASURED).
    #[must_use]
    pub fn rm_control_wire(&self) -> RmControlWire {
        self.rm_control
    }

    /// The exact measured driver version this table describes.
    #[must_use]
    pub fn driver_version(&self) -> DriverVersion {
        self.version
    }

    /// Decode a `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` payload.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] below [`ctrl::Nv0080CtrlDmaSetPageDirectoryParams::SIZE`].
    ///
    /// ★ **Correction to the generated module's version caveat**, which says the
    /// tail (`chId`, `subDeviceId`, `pasid`) has ogkm 610.43.02 as its only
    /// oracle. It does not: `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl0080/ctrl0080dma.h:832-840`
    /// declares the identical seven members in the identical order, so **32 is
    /// the agreed size across the whole supported range**, not one tree's
    /// opinion. The caveat cannot be edited where it is written (that file is
    /// generated); it is superseded here.
    pub fn decode_set_page_dir(&self, bytes: &[u8]) -> Result<SetPageDir, AbiError> {
        let p = ctrl::Nv0080CtrlDmaSetPageDirectoryParams::decode(bytes)?;
        Ok(SetPageDir {
            phys_address: p.phys_address,
            num_entries: p.num_entries,
            aperture: PdbAperture::from_flags(p.flags),
            flags: p.flags,
            h_vaspace: p.h_va_space,
        })
    }

    /// ★★ Decode `NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS` — the 48-byte transcribed prefix
    /// plus `entryCount` entries of the generated 32-byte record, classified into
    /// [`PromoteEntry`]'s three protocol states.
    ///
    /// # The four refusals, each by name
    ///
    /// 1. **`bytes.len() < 560`** → [`AbiError::Truncated`]. The caller has already
    ///    checked the guest's declared `paramsSize` against
    ///    [`ControlParams::params_size`] *exactly*; this is the second, independent
    ///    check against the bytes that actually arrived.
    /// 2. **`entryCount > 16`** → [`AbiError::PromoteEntryCount`]. Never clamped — see
    ///    that variant for the 1536-byte over-read a clamp produced in the C artifact.
    ///    ★ The bound is checked **before a single entry is touched**.
    /// 3. **`physAttr[1:0] == 3`** → [`AbiError::PromoteAperture`]. Undefined, so it is
    ///    refused rather than folded into sysmem.
    /// 4. **the legacy `hVirtMemory`/`(virtAddress, size)` shape** →
    ///    [`AbiError::PromoteLegacyShape`].
    ///
    /// # ★ What it does NOT do
    ///
    /// It does not drop an unbindable entry. A promote-only entry (`phys == 0`,
    /// `size == 0`, VA set) is a legitimate, expected message the two-preparer protocol
    /// produces — 4 of the 9 entries in the repo's own captured blob — and it is
    /// **classified**, so a consumer that cannot bind it can still say so. The C's
    /// `!sz` arm discarded it with no name and no count.
    ///
    /// # Errors
    ///
    /// The four above.
    pub fn decode_promote_ctx(&self, bytes: &[u8]) -> Result<PromoteCtx, AbiError> {
        self.decode_promote_ctx_inner(bytes)
    }

    /// ★ v3-video — the FALCON promote: `_kflcnPromoteContext` (`ogkm-580: kernel_falcon.c:
    /// 184-276`) sends `{engineType, ChID, hObject = the channel, size = ctxBufferSize,
    /// virtAddress = the context buffer's VA, entryCount = 0}` for a non-externally-owned VAS —
    /// the one real producer of the `(virtAddress, size)` shape [`Self::decode_promote_ctx`]
    /// refuses. Accepted ONLY in exactly that form and ONLY for a video engine (NVENC / NVDEC,
    /// NV2080 space); returns `(engine_type, hChanClient, hObject, virtAddress, size)`.
    ///
    /// # Errors
    /// [`AbiError::Truncated`], or [`AbiError::PromoteLegacyShape`] for anything else.
    pub fn decode_falcon_promote(
        &self,
        bytes: &[u8],
    ) -> Result<(u32, u32, u32, u64, u64), AbiError> {
        let need = Nv2080CtrlGpuPromoteCtxParamsHeader::PARAMS_SIZE;
        if bytes.len() < need {
            return Err(AbiError::Truncated {
                c_name: Nv2080CtrlGpuPromoteCtxParamsHeader::C_NAME,
                need,
                got: bytes.len(),
            });
        }
        let h = Nv2080CtrlGpuPromoteCtxParamsHeader::decode(bytes)?;
        if h.h_virt_memory == 0
            && h.virt_address != 0
            && h.size != 0
            && h.entry_count == 0
            && crate::submit::is_video_engine_type(h.engine_type)
        {
            return Ok((
                h.engine_type,
                h.h_chan_client,
                h.h_object,
                h.virt_address,
                h.size,
            ));
        }
        Err(AbiError::PromoteLegacyShape {
            h_virt_memory: h.h_virt_memory,
            virt_address: h.virt_address,
            size: h.size,
        })
    }

    fn decode_promote_ctx_inner(&self, bytes: &[u8]) -> Result<PromoteCtx, AbiError> {
        let need = Nv2080CtrlGpuPromoteCtxParamsHeader::PARAMS_SIZE;
        if bytes.len() < need {
            return Err(AbiError::Truncated {
                c_name: Nv2080CtrlGpuPromoteCtxParamsHeader::C_NAME,
                need,
                got: bytes.len(),
            });
        }
        let h = Nv2080CtrlGpuPromoteCtxParamsHeader::decode(bytes)?;

        // ★ The legacy path is refused, not guessed. Both real producers zero all three.
        if h.h_virt_memory != 0 || h.virt_address != 0 || h.size != 0 {
            return Err(AbiError::PromoteLegacyShape {
                h_virt_memory: h.h_virt_memory,
                virt_address: h.virt_address,
                size: h.size,
            });
        }

        // ★★★ D1. The bound, against the header's own constant, BEFORE any entry read.
        let declared = h.entry_count;
        if declared as usize > MAX_PROMOTE_ENTRIES {
            return Err(AbiError::PromoteEntryCount {
                declared,
                max: MAX_PROMOTE_ENTRIES,
            });
        }

        let mut entries: [Option<PromoteEntry>; MAX_PROMOTE_ENTRIES] = [None; MAX_PROMOTE_ENTRIES];
        for (i, slot) in entries.iter_mut().enumerate().take(declared as usize) {
            let at = Nv2080CtrlGpuPromoteCtxParamsHeader::SIZE
                + i * ctrl::Nv2080CtrlGpuPromoteCtxBufferEntry::SIZE;
            let e = ctrl::Nv2080CtrlGpuPromoteCtxBufferEntry::decode(bytes.get(at..).ok_or(
                AbiError::Truncated {
                    c_name: Nv2080CtrlGpuPromoteCtxParamsHeader::C_NAME,
                    need,
                    got: bytes.len(),
                },
            )?)?;
            *slot = Some(classify_promote_entry(i, &e)?);
        }
        Ok(PromoteCtx::new(
            h.engine_type,
            h.h_chan_client,
            h.h_object,
            entries,
        ))
    }

    /// Decode the fixed header of a `GSP_RM_ALLOC` **RPC body** (everything after
    /// the 32-byte `rpc_message_header`), i.e. `rpc_gsp_rm_alloc_v03_00`.
    ///
    /// Not versioned within the supported range: the struct lives in NVIDIA's
    /// OS-independent RM core and has carried these seven fields since `_v03_00`
    /// (`ogkm-580: src/nvidia/generated/g_rpc-structures.h:1491-1502` /
    /// `ogkm-610: src/nvidia/generated/g_rpc-structures.h:1408-1419` — the same
    /// list at both). It takes the version table anyway,
    /// like every other decoder here, so the day it *does* move the call sites do
    /// not change.
    ///
    /// ★ It decodes the header **only**. `paramsSize` is guest-declared, so
    /// slicing `params[]` with it is a validation the caller owes, and the caller
    /// is the one that can name the refusal with both numbers.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if fewer than [`RpcAllocReq::HEADER`] bytes are
    /// available — never a zero-extended partial decode.
    pub fn decode_rpc_alloc(&self, payload: &[u8]) -> Result<RpcAllocReq, AbiError> {
        if payload.len() < RpcAllocReq::HEADER {
            return Err(AbiError::Truncated {
                c_name: RpcAllocReq::C_NAME,
                need: RpcAllocReq::HEADER,
                got: payload.len(),
            });
        }
        Ok(RpcAllocReq {
            client: u32_at(payload, 0)?,
            parent: u32_at(payload, 4)?,
            handle: u32_at(payload, 8)?,
            class: u32_at(payload, 12)?,
            // +16 is `status`, an [OUT] field the guest sends as zero.
            params_size: u32_at(payload, 20)?,
            params_flags: u32_at(payload, 24)?,
            // +28 is `reserved[4]`.
            params_at: RpcAllocReq::HEADER,
        })
    }

    /// Is this class an **RM client root** — the class whose alloc creates a
    /// namespace, and whose `hClient` *is* its object handle?
    ///
    /// `[src]` `NV01_ROOT` (0x0) and `NV01_ROOT_CLIENT` (0x41) are one resource
    /// kind to RM (`ogkm-580: src/common/sdk/nvidia/inc/class/cl0000.h:42` /
    /// `ogkm-610: src/common/sdk/nvidia/inc/class/cl0000.h:42` — same line at both;
    /// `ogkm-580: src/nvidia/generated/g_allclasses.h:276` /
    /// `ogkm-610: src/nvidia/generated/g_allclasses.h:289`); the generated module's
    /// own doc on [`classes::NV01_ROOT_CLIENT`] says the same.
    ///
    /// Lives here rather than in the bridge for the quarantine reason
    /// (decision #2): the NVIDIA class *numbers* are this crate's, and the crates
    /// above it speak a predicate. Same shape as
    /// [`crate::GuestOs::client_kind_from_process_id`].
    #[must_use]
    pub fn is_client_root_class(&self, class: ClassId) -> bool {
        class.0 == classes::NV01_ROOT || class.0 == classes::NV01_ROOT_CLIENT
    }

    /// Decode a GSP-RPC envelope, validating its guest-written `length`.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if the buffer cannot hold the 32-byte envelope;
    /// [`AbiError::RpcSignature`] if the signature word is wrong;
    /// [`AbiError::RpcLength`] if `length` is below the envelope size or beyond
    /// the buffer.
    pub fn decode_rpc_envelope(&self, bytes: &[u8]) -> Result<RpcEnvelope, AbiError> {
        let h = rpc::RpcMessageHeaderV0300::decode(bytes)?;
        if h.signature != RpcEnvelope::SIGNATURE_VALID {
            return Err(AbiError::RpcSignature {
                found: h.signature,
                expected: RpcEnvelope::SIGNATURE_VALID,
            });
        }
        let payload_len = rpc_payload_len(h.length, bytes.len())?;
        Ok(RpcEnvelope {
            header_version: h.header_version,
            signature: h.signature,
            length: h.length,
            function: h.function,
            rpc_result: h.rpc_result,
            rpc_result_private: h.rpc_result_private,
            sequence: h.sequence,
            payload_len,
        })
    }

    /// The payload bytes of an RPC message, i.e. the flexible-array tail.
    ///
    /// # Errors
    ///
    /// Whatever [`Self::decode_rpc_envelope`] returns; the slice itself cannot
    /// fail once the envelope validated, because the envelope's validation is
    /// exactly the bound this uses.
    pub fn rpc_payload<'a>(&self, bytes: &'a [u8]) -> Result<&'a [u8], AbiError> {
        let env = self.decode_rpc_envelope(bytes)?;
        bytes
            .get(RpcEnvelope::SIZE..RpcEnvelope::SIZE + env.payload_len)
            .ok_or(AbiError::RpcLength {
                declared: env.length,
                available: bytes.len(),
            })
    }
}

/// The bytes [`ClientAllocFacts`] is decoded from — `hClient` and `processID`.
pub const CLIENT_ALLOC_PREFIX: usize = 8;

/// The bytes [`ChannelAllocFacts`] is decoded from — through `hVASpace` @ +28.
///
/// ★ This is a **version-agreement** bound, not a struct size: it is exactly the
/// region `ogkm-610` 610.43.02 and `ogkm-580` 580.159.04 spell identically. See
/// [`ChannelAllocFacts`] for the divergence at +32.
pub const CHANNEL_ALLOC_PREFIX: usize = 32;

/// The C typedef [`CHANNEL_ALLOC_PREFIX`] is a prefix of. Named here rather than
/// taken from a generated `C_NAME` because the struct is deliberately **not**
/// mirrored — see [`ChannelAllocFacts`].
const CHANNEL_ALLOC_C_NAME: &str = "NV_CHANNEL_ALLOC_PARAMS";

/// Which alloc-params shape a class carries. See [`DriverAbiTable::alloc_params`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocParams {
    /// `NV01_ROOT` / `NV01_ROOT_CLIENT` — [`DriverAbiTable::decode_client_alloc_facts`].
    ClientRoot,
    /// `NV01_DEVICE_0` — [`DriverAbiTable::decode_device_alloc_facts`].
    Device,
    /// `KEPLER_CHANNEL_GROUP_A` — [`DriverAbiTable::decode_tsg_alloc_facts`].
    Tsg,
    /// `FERMI_CONTEXT_SHARE_A` — [`DriverAbiTable::decode_ctxshare_alloc_facts`].
    CtxShare,
    /// `AMPERE_CHANNEL_GPFIFO_A` — [`DriverAbiTable::decode_channel_alloc_facts`].
    Channel,
    /// ★★★★ §16.28 — `FERMI_VASPACE_A` — [`DriverAbiTable::decode_vaspace_index`].
    ///
    /// One field, `index`, and it is read for one reason: `index == 3`
    /// ([`NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE`]) means the alloc **acquires a
    /// reference to the Device's existing default VA space** rather than creating one.
    /// See that constant for the RM call chain that makes such an alloc appear, publish a
    /// page-directory root, and then be freed again.
    VaSpace,
    /// A mapped class whose params declare nothing the object model reads.
    NoDeclaredFacts,
}

/// `NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY` — the symmetric teardown of
/// [`ctrl::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY`], and RPC'd to GSP on the same
/// `IS_GSP_CLIENT` branch (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/dma.c:606-608`).
/// Its params are `{hVASpace, subDeviceId}` and carry no address
/// (`ogkm-580: ctrl0080dma.h:882-885`), so it *revokes* a page-directory binding
/// rather than declaring one.
///
/// Hand-written rather than generated: the generator's slice emits exactly the
/// one control struct the port decodes, and these three ids exist to be
/// **refused by name**, not decoded.
pub(crate) const NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY: u32 = 0x0080_1814;

/// `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl90f1.h:268` /
/// `ogkm-610: src/common/sdk/nvidia/inc/ctrl/ctrl90f1.h:268` — same line at both,
/// and the id's value `0x90f10106` is byte-identical there).
///
/// ★ The line was `:272` in both halves until 2026-07-28; that is the *params*
/// `typedef` (`NV90F1_CTRL_VASPACE_COPY_SERVER_RESERVED_PDES_PARAMS`), not the
/// command id. Carrying a version tag is not evidence the tree was read — this
/// citation was `ogkm-580:`-tagged and still pointed four lines past the claim.
///
/// ★★ **The one that matters.** It is issued at VASpace *construct* time for
/// every split-VAS-eligible VAS on a GSP client — `gvaspaceConstruct__IMPL`
/// → `gvaspaceReserveSplitVaSpace_IMPL` → `_gvaspaceReserveVaForClientRm`
/// → `gvaspaceCopyServerRmReservedPdesToServerRm_IMPL`, which issues
/// `NV_RM_RPC_CONTROL` and so reaches the wire as `GSP_RM_CONTROL`
/// (`ogkm-580: src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:598-611, 395, 313, 378, 4039, 5161-5189`).
/// Split-VAS management is **on by default** for any GSP client
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_registry.c:171-186`).
///
/// Its `levels[0].physAddress` **is** the VAS's root page directory. So for an
/// ordinary RM-managed VASpace this — not `SET_PAGE_DIRECTORY` — is the only
/// message that carries a PDB, and a port that models only `0x00801813` has no
/// PDB for it at all. That is `gsp_core_bridge.md` §7 item 1, now settled and
/// settled *against* the design's assumption.
pub(crate) const NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES: u32 = 0x90f1_0106;

/// `NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:1902`,
/// `ogkm-610: :1905` — the id's value `0x20800a9f` is identical at both; only the
/// line moved, so this is a **moved citation, not a version seam**).
///
/// ★ The 580 half read `:1903-1908` until 2026-07-28. Line 1903 is blank at 580;
/// the `#define` is `:1902` and the params `typedef` does not start until `:1906`.
///
/// A `ROUTE_TO_PHYSICAL` wrapper whose params are a single-member struct around
/// the same `NV90F1_CTRL_VASPACE_COPY_SERVER_RESERVED_PDES_PARAMS` at offset 0
/// (`ogkm-580: g_subdevice_nvoc.c:3655-3663` gives `flags = 0xc0` = ROUTE_TO_PHYSICAL
/// | INTERNAL). It is emitted for the GPU-group global VASpace on the
/// `!IS_VIRTUAL` arm — i.e. on bare metal, which is our target
/// (`ogkm-580: gpu_vaspace.c:4140-4154`), on `pGpu->hInternalClient`.
pub(crate) const NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER: u32 =
    0x2080_0a9f;

/// Which params shape a control command carries. See
/// [`DriverAbiTable::control_params`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlParams {
    /// `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` —
    /// [`DriverAbiTable::decode_set_page_dir`]. The one control this port turns
    /// into a fact.
    SetPageDir,
    /// ★★★ **The guest PUBLISHING where a VA space's page directories live** —
    /// [`crate::gvaspacepdes::decode_server_reserved_pdes`], whose
    /// [`root()`](crate::gvaspacepdes::ServerReservedPdes::root) is the VA space's
    /// page-directory base.
    ///
    /// Two command ids, one 184-byte params struct, and the second is a
    /// `ROUTE_TO_PHYSICAL` wrapper around the first at offset 0:
    ///
    /// - `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` (`0x90f10106`) — issued at
    ///   construct time for **every ordinary RM-managed VAS** on a GSP client;
    /// - `NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER`
    ///   (`0x20800a9f`) — the same payload for the GPU-group global VAS.
    ///
    /// ★★★ **This is where a page-directory base comes from on the boot path, and
    /// [`Self::SetPageDir`] is not.** §14.9's census of a real GA106 boot measured
    /// `SET_PAGE_DIRECTORY` at **zero** occurrences and this pair at **five**;
    /// `SET_PAGE_DIRECTORY` reaches the wire only for a `SHARED_MANAGEMENT` /
    /// `IS_EXTERNALLY_OWNED` VASpace, because the handler asserts on exactly that
    /// (`ogkm-580: src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:3109`).
    ///
    /// ⚠ **The VA space is named by the RPC HEADER's `hObject`, not by a params field**
    /// (`rmCtrlParams.hObject = hVASpace`, `ogkm-580: gpu_vaspace.c:5174-5177`) — the one
    /// field a port drops first, and without it a root belongs to no address space.
    VaspacePublishedPdes,
    /// ★★ **Known to move a VASpace's page-directory binding, and not modelled.**
    ///
    /// `NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY` (`0x00801814`) — the *revocation*.
    /// [`crate::view::SetPageDir`]'s event has no verb for it, and inventing one is a core
    /// change rather than a bridge change. Answered with a *named* refusal rather than
    /// silence, because the state of a PDB is invisible downstream: a channel whose
    /// address space is wrong simply defers at its first doorbell, forever.
    ///
    /// ⊘ The two *publication* ids left this variant on 2026-08-08 for
    /// [`Self::VaspacePublishedPdes`]. The reason they were here — *"its params are a
    /// 184-byte struct … so it needs the generator and a `RUSTC_OFFSETS` pin rather than a
    /// hand-transcription"* — was answered by [`crate::gvaspacepdes`], which decodes,
    /// validates against `ctrl90f1.h`'s own rules and pins its size with a `const` assert.
    PageDirNotModelled,
    /// ★★ `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` —
    /// [`DriverAbiTable::decode_promote_ctx`]. The **address-plane** control: it
    /// declares where a graphics/compute context's buffers live.
    ///
    /// ★ Not versioned, and the absence of a fork is the finding. The params struct, the
    /// entry struct, `NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES` and **both** producer
    /// functions are byte-identical at 580.159.04 and 610.43.02. `MapDmaWire` exists
    /// because `NVOS46_PARAMETERS` genuinely moved; adding a seam here would be inventing
    /// one that the trees say does not exist.
    PromoteCtx,
}

impl ControlParams {
    /// `sizeof` this control's params struct, where the port decodes one.
    ///
    /// ★ A control's `paramsSize` is checked against this **exactly**, not as a
    /// lower bound: `deviceCtrlCmdDmaSetPageDirectory`'s caller passes
    /// `sizeof(NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS)` verbatim
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/dma.c:508-518`), so a
    /// different declared size is a guest that means a different struct.
    /// `gsp_core_bridge.md` §4.3: *"validate against the payload length **and**
    /// against the class's own size where the ABI knows it, and refuse the
    /// mismatch rather than taking the smaller."*
    ///
    /// `None` for [`Self::PageDirNotModelled`] — there is no decoder, so there is
    /// no size to check against and claiming one would be a number with no
    /// oracle.
    #[must_use]
    pub const fn params_size(self) -> Option<usize> {
        match self {
            ControlParams::SetPageDir => Some(ctrl::Nv0080CtrlDmaSetPageDirectoryParams::SIZE),
            // 184, and it is checked EXACTLY like every other row here: the two senders
            // pass `sizeof(NV90F1_CTRL_VASPACE_COPY_SERVER_RESERVED_PDES_PARAMS)` verbatim
            // (`ogkm-580: gpu_vaspace.c:4151, 5185`), so a different declared size is a
            // guest that means a different struct. ⊘ Not a lower bound — the decoder
            // refuses anything but this length (`ServerReservedPdesError::WrongSize`), and
            // this makes the refusal name the *command* as well as the buffer.
            ControlParams::VaspacePublishedPdes => {
                Some(crate::gvaspacepdes::COPY_SERVER_RESERVED_PDES_PARAMS_SIZE)
            }
            ControlParams::PageDirNotModelled => None,
            // 560 — and it is a PRODUCT of two machine-checked numbers plus the
            // transcribed prefix, never a literal. `subdeviceCtrlCmdGpuPromoteCtx`'s
            // caller passes `sizeof(NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS)` verbatim, so a
            // different declared size is a guest that means a different struct.
            ControlParams::PromoteCtx => Some(Nv2080CtrlGpuPromoteCtxParamsHeader::PARAMS_SIZE),
        }
    }
}

impl DriverAbi for DriverAbiTable {
    fn version(&self) -> DriverVersion {
        self.version
    }

    /// The alloc-param size for a class, or `None` when this build cannot state
    /// one.
    ///
    /// ★ `NV01_ROOT` / `NV01_ROOT_CLIENT` are deliberately **absent**, and the
    /// absence is the honest answer rather than a gap: `NV0000_ALLOC_PARAMETERS`
    /// is 120 bytes in ogkm 610.43.02 and has **no second oracle** — neither
    /// nvproxy nor the C artifact models it — so its size at 575/580 is
    /// unverified. Reporting 120 would be exactly the guessed size this table
    /// exists to prevent. The client-kind path does not need it: it reads the
    /// 8-byte prefix contract (`decode_client_alloc_facts`).
    ///
    /// `NV01_DEVICE_0` is present because its 56 bytes are confirmed three ways
    /// (ogkm 610.43.02, `gvisor/pkg/abi/nvgpu/classes.go:198-211`, and the C
    /// artifact's `abi_parity_test.go:120`).
    fn alloc_param_size(&self, class: ClassId) -> Option<usize> {
        if class.0 == classes::NV01_DEVICE_0 {
            Some(classes::Nv0080AllocParameters::SIZE)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(major: u16, minor: u16, patch: u16) -> Result<&'static DriverAbiTable, AbiError> {
        table_for(DriverVersion {
            major,
            minor,
            patch,
        })
    }

    /// The capability rows are ascending — "newest row ≤ version" is only right if they are.
    #[test]
    fn the_caps_rows_are_strictly_ascending() {
        for w in CAPS_ROWS.windows(2) {
            assert!(w[0].from < w[1].from, "{:?} !< {:?}", w[0].from, w[1].from);
        }
    }

    /// ★★★ Exact membership: a version that was never measured is refused by name — one
    /// patch below a real boundary, a made-up boundary the old hand table used (550.54.04 is
    /// not an ogkm tag), and a far-future version alike. No nearest neighbour.
    #[test]
    fn an_unmeasured_version_is_refused_by_name_never_the_nearest_table() {
        for (maj, min, pat) in [(580u16, 65u16, 5u16), (550, 54, 4), (999, 0, 0), (0, 0, 0)] {
            assert_eq!(
                at(maj, min, pat).map(|t| t.version),
                Err(AbiError::Unmeasured {
                    major: maj,
                    minor: min,
                    patch: pat
                }),
                "{maj}.{min}.{pat} must not resolve to a table",
            );
        }
        let msg = at(580, 159, 3).unwrap_err().to_string();
        assert!(
            msg.contains("580.159.03") && msg.contains("regen.sh"),
            "{msg}"
        );
    }

    /// Below the oldest reviewed allowlist is its own named refusal — a measured version with
    /// no capability surface, never admitted against the 550 one.
    #[test]
    fn below_the_oldest_capability_row_is_refused_by_name() {
        assert_eq!(
            at(535, 309, 1).map(|t| t.version),
            Err(AbiError::NoCapabilityRow {
                major: 535,
                minor: 309,
                patch: 1
            })
        );
        assert!(
            at(550, 54, 14).is_ok(),
            "the oldest row's first measured tag resolves"
        );
    }

    /// The NVOS46 shape follows the MEASURED layout, on both sides of 580.65.06.
    #[test]
    fn the_nvos46_shape_is_read_from_the_measured_layout() {
        let old = at(575, 57, 8).expect("measured");
        assert_eq!(old.map_dma_wire(), MapDmaWire::Pre580_65_06);
        assert_eq!(old.map_dma_size(), 56, "nvkvm_abi.h:76 .nvos46_size = 56");
        assert_eq!(
            old.map_dma_status_offset(),
            48,
            "nvkvm_abi.h:76 .nvos46_status_off = 48"
        );
        let new = at(580, 65, 6).expect("measured");
        assert_eq!(new.map_dma_wire(), MapDmaWire::From580_65_06);
        assert_eq!(new.map_dma_size(), 64, "nvkvm_abi.h:86 .nvos46_size = 64");
        assert_eq!(
            new.map_dma_status_offset(),
            56,
            "nvkvm_abi.h:86 .nvos46_status_off = 56"
        );
    }

    /// The bench driver constant resolves, and resolves to the 64-byte NVOS46.
    #[test]
    fn the_bench_driver_resolves_to_the_64_byte_nvos46() {
        let t = table_for(BENCH_DRIVER).expect("the bench driver is measured");
        assert_eq!(t.map_dma_wire(), MapDmaWire::From580_65_06);
        assert_eq!(
            t.driver_version(),
            BENCH_DRIVER,
            "the table is the EXACT version's"
        );
    }

    /// ★★★ The `GSP_RM_CONTROL` header is 24 bytes through 570.x and 40 from 575 — measured,
    /// and the decoder slices `params[]` where the version puts it.
    #[test]
    fn the_rm_control_header_follows_the_measured_boundary_at_575() {
        let w570 = at(570, 148, 8).expect("measured").rm_control_wire();
        assert_eq!((w570.params_off, w570.rpc_flags_off), (24, 20));
        assert_eq!((w570.rmctrl_flags_off, w570.access_right_off), (None, None));
        let w575 = at(575, 51, 3).expect("measured").rm_control_wire();
        assert_eq!((w575.params_off, w575.rpc_flags_off), (40, 20));
        assert_eq!(
            (w575.rmctrl_flags_off, w575.access_right_off),
            (Some(24), Some(28))
        );
        // The decoder follows the wire: a 570 control's params start at +24.
        let mut body = vec![0u8; 24 + 4];
        body[8..12].copy_from_slice(&0x2080_0102u32.to_le_bytes());
        body[16..20].copy_from_slice(&4u32.to_le_bytes());
        let r = at(570, 148, 8)
            .expect("measured")
            .decode_rpc_control(&body)
            .expect("decodes");
        assert_eq!((r.cmd, r.params_size, r.params_at), (0x2080_0102, 4, 24));
        assert!(
            at(575, 51, 3)
                .expect("measured")
                .decode_rpc_control(&body)
                .is_err(),
            "a 28-byte body is short of the 575 header"
        );
    }

    /// ★★ The vGPU handshake pair is measured per tag, and it moves INSIDE a branch.
    #[test]
    fn the_vgx_pair_is_measured_per_tag() {
        let pair = |t: &DriverAbiTable| t.vgx_version().map(|v| (v.major, v.minor));
        assert_eq!(pair(at(570, 124, 6).expect("measured")), Some((0x29, 0x0B)));
        assert_eq!(pair(at(570, 148, 8).expect("measured")), Some((0x29, 0x0C)));
        assert_eq!(pair(at(580, 159, 4).expect("measured")), Some((0x2B, 0x13)));
        assert_eq!(pair(at(610, 43, 2).expect("measured")), Some((0x2E, 0x0D)));
    }

    /// The channel-alloc tail offsets are READ: V580's constants at every measured tag up to
    /// 595.84, V610's at 610 — the old table had `None` on every 550–575 row.
    #[test]
    fn the_channel_alloc_wires_are_measured_not_boundary_rows() {
        for (maj, min, pat) in [
            (550u16, 54u16, 14u16),
            (570, 124, 6),
            (580, 159, 4),
            (595, 84, 0),
        ] {
            let t = at(maj, min, pat).expect("measured");
            assert_eq!(
                t.channel_notifier,
                Some(ChannelNotifierWire::V580),
                "{maj}.{min}.{pat}"
            );
            assert_eq!(
                t.channel_userd,
                Some(ChannelUserdWire::V580),
                "{maj}.{min}.{pat}"
            );
            assert_eq!(
                t.channel_userd_mem,
                Some(ChannelUserdMemWire::V580),
                "{maj}.{min}.{pat}"
            );
            assert_eq!(
                t.channel_engine,
                Some(ChannelEngineWire::V580),
                "{maj}.{min}.{pat}"
            );
        }
        let t = at(610, 43, 2).expect("measured");
        assert_eq!(t.channel_engine, Some(ChannelEngineWire::V610));
        assert_eq!(t.channel_userd, Some(ChannelUserdWire::V610));
    }

    /// The static-info encoder is claimed only where the measured layout IS the bench's.
    #[test]
    fn static_info_is_encoded_only_where_the_layout_equals_the_benchs() {
        for (maj, min, pat) in [(580u16, 65u16, 6u16), (580, 159, 4), (580, 178, 4)] {
            assert_eq!(
                at(maj, min, pat).expect("measured").gsp_static_info_wire(),
                GspStaticInfoWire::Pre610
            );
        }
        for (maj, min, pat) in [
            (570u16, 124u16, 6u16),
            (575, 57, 8),
            (590, 48, 1),
            (595, 84, 0),
        ] {
            assert_eq!(
                at(maj, min, pat).expect("measured").gsp_static_info_wire(),
                GspStaticInfoWire::Unencoded,
                "{maj}.{min}.{pat}"
            );
        }
        assert_eq!(
            at(610, 43, 2).expect("measured").gsp_static_info_wire(),
            GspStaticInfoWire::From610_43_02
        );
    }

    /// A measured layout nobody encodes is refused by name: 615.71.09's GSP queue element
    /// (an encryption union after `mctpMagic`/`mctpPayloadSize`) is neither shape.
    #[test]
    fn a_measured_layout_with_no_encoding_is_refused_by_name() {
        match at(615, 71, 9) {
            Err(AbiError::NoEncoding { what, .. }) => {
                assert!(what.starts_with("GSP_MSG_QUEUE_ELEMENT"), "{what}")
            }
            other => panic!(
                "615.71.09 must be refused by its element shape, got {:?}",
                other.map(|t| t.version)
            ),
        }
    }

    /// `alloc_param_size` states what it knows and refuses what it does not, and
    /// the refusal is asserted for the specific class it is about — so the day
    /// someone populates `NV01_ROOT` from a second oracle, this test changes.
    #[test]
    fn alloc_param_size_reports_only_the_triple_confirmed_class() {
        let t = table_for(BENCH_DRIVER).expect("supported");
        assert_eq!(
            t.alloc_param_size(ClassId(classes::NV01_DEVICE_0)),
            Some(56)
        );
        assert_eq!(
            t.alloc_param_size(ClassId(classes::NV01_ROOT)),
            None,
            "NV0000_ALLOC_PARAMETERS has no second oracle — see the method doc"
        );
        assert_eq!(t.alloc_param_size(ClassId(classes::NV01_ROOT_CLIENT)), None);
        assert_eq!(t.alloc_param_size(ClassId(0xDEAD_BEEF)), None);
    }
}
