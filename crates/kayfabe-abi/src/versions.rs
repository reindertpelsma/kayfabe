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
    CAPS_560_28_03, CAPS_570_86_15, CAPS_580_65_06, CAPS_BASE, CapabilityTable,
};
use crate::generated::{classes, ctrl, nvos, rpc};
use crate::transcribed::{Nv2080CtrlGpuPromoteCtxParamsHeader, Nvos46ParametersPre580};
use crate::vbios::VbiosWire;
use crate::view::{
    AllocReq, AllocWire, ChannelAllocFacts, ClientAllocFacts, ControlReq, CtxShareAllocFacts,
    DeviceAllocFacts, DupReq, FreeReq, MAX_PROMOTE_ENTRIES, MapMemoryDma, PdbAperture, PromoteCtx,
    PromoteEntry, RpcAllocReq, RpcControlReq, RpcEnvelope, SetPageDir, TsgAllocFacts,
    UnmapMemoryDma, classify_promote_entry, rpc_payload_len,
};
use crate::wire::{AbiError, u32_at};
use crate::{DriverAbi, DriverVersion};
use kayfabe_arch::ids::{ClassId, ControlCmd};

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
    version: DriverVersion,
    map_dma: MapDmaWire,
    gsp_element: GspElementWire,
    gsp_init_args: GspInitArgsWire,
    /// ★ The **default-deny RM capability surface** for this boundary
    /// ([`crate::capability`]): which control commands and which allocation
    /// classes a guest at this driver version may name at all.
    ///
    /// It is a field here, and not a free function, for the reason the whole
    /// module exists: *adding a driver version must not edit a logic crate*. A
    /// new version is a new `TABLES` row pointing at a new
    /// [`CapabilityTable`] — and because that type is inherit-then-add, the new
    /// row is only the delta.
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
    /// Why this entry exists — kept in the data so a reader of the table sees
    /// the boundary's justification without leaving the file.
    pub note: &'static str,
}

/// The registry, in **ascending** version order.
///
/// `table_for` picks the newest entry `<= requested`, which is inherit-then-
/// mutate expressed as data: an entry only exists where something changed.
pub const TABLES: &[DriverAbiTable] = &[
    DriverAbiTable {
        version: DriverVersion {
            major: 550,
            minor: 54,
            patch: 4,
        },
        map_dma: MapDmaWire::Pre580_65_06,
        gsp_element: GspElementWire::Pre610,
        gsp_init_args: GspInitArgsWire::FourField,
        caps: &CAPS_BASE,
        vbios: VbiosWire::Tu102Bit,
        note: "oldest supported: NVOS47 gained `size` here \
               (gvisor/pkg/abi/nvgpu/frontend.go:707-710, NVOS47_PARAMETERS_V550)",
    },
    // ★ The next two rows exist ONLY for the capability surface: every wire
    // layout in them is its predecessor's, and nvproxy adds no frontend ioctl
    // and no control command at either boundary — just allocation classes
    // (`gvisor/pkg/sentry/devices/nvproxy/version.go:945-977` and `:990-1027`).
    // They are here because the alternative is giving a 550 guest the class set
    // of a 570 one, which is a quietly WIDER gate at the oldest supported
    // version — the direction a security table must never drift in.
    DriverAbiTable {
        version: DriverVersion {
            major: 560,
            minor: 28,
            patch: 3,
        },
        map_dma: MapDmaWire::Pre580_65_06,
        gsp_element: GspElementWire::Pre610,
        gsp_init_args: GspInitArgsWire::FourField,
        caps: &CAPS_560_28_03,
        vbios: VbiosWire::Tu102Bit,
        note: "capability-only boundary: +8 allocation classes, no layout change \
               (gvisor/pkg/sentry/devices/nvproxy/version.go:945-977)",
    },
    DriverAbiTable {
        version: DriverVersion {
            major: 570,
            minor: 86,
            patch: 15,
        },
        map_dma: MapDmaWire::Pre580_65_06,
        gsp_element: GspElementWire::Pre610,
        gsp_init_args: GspInitArgsWire::FourField,
        caps: &CAPS_570_86_15,
        vbios: VbiosWire::Tu102Bit,
        note: "capability-only boundary: +6 allocation classes, no layout change \
               (gvisor/pkg/sentry/devices/nvproxy/version.go:990-1027)",
    },
    // ★★★ 575.51.02 — added 2026-07-30 as the SECOND-DRIVER-VERSION MEASUREMENT, and it
    // is the row that shows what this shape can and cannot carry.
    //
    // The additive half really is one row: no wire layout moves here, so every field
    // below is its predecessor's and this entry costs exactly these lines.
    //
    // ⚠ **The subtractive half cannot be expressed, and the boundary is MOSTLY
    // subtractive.** nvproxy's `v575_51_02` is three deletes-and-replaces on the control
    // map, not an addition (`gvisor/pkg/sentry/devices/nvproxy/version.go:1036-1053`):
    //   - `NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT` 0x20801358 -> 0x20801357
    //   - `NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS`          0x20801359 -> 0x20801358
    //   - `NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2` 0x20800513 added
    // `CapabilityTable` is inherit-then-**add** and has no removal field, which its own
    // module doc already states. So this row carries the *fact* of the boundary and none
    // of its *content*, and the consequence is live and measured — see
    // `the_575_boundary_is_subtractive_and_this_shape_cannot_carry_it` in
    // `crate::capability`'s tests, which pins that 0x20801359 is refused for a 550/570
    // guest that legitimately issues it, and that 0x20801358 is permitted under the
    // 575-era NAME while a 550/570 guest means the pre-575 command by that number.
    //
    // Recorded, not fixed: a removal field is a type change with its own blast radius and
    // belongs with its first real consumer, exactly as the module doc says.
    DriverAbiTable {
        version: DriverVersion {
            major: 575,
            minor: 51,
            patch: 2,
        },
        map_dma: MapDmaWire::Pre580_65_06,
        gsp_element: GspElementWire::Pre610,
        gsp_init_args: GspInitArgsWire::FourField,
        caps: &CAPS_570_86_15,
        vbios: VbiosWire::Tu102Bit,
        note: "★ the first SUBTRACTIVE boundary: nvproxy replaces two DRAM-encryption \
               commands here rather than adding any (version.go:1036-1053). This shape \
               can only add, so the row records the boundary and carries none of its \
               content — the additive third (THERMAL_SYSTEM_EXECUTE_V2) is already in \
               CAPS_BASE because that table is the 575 set the C shipped",
    },
    DriverAbiTable {
        version: DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        },
        map_dma: MapDmaWire::From580_65_06,
        gsp_element: GspElementWire::Pre610,
        gsp_init_args: GspInitArgsWire::FourField,
        caps: &CAPS_580_65_06,
        vbios: VbiosWire::Tu102Bit,
        note: "NVOS46 gained flags2+kindOverride, and +2 allocation classes \
               (gvisor/pkg/sentry/devices/nvproxy/version.go:1057-1078)",
    },
    DriverAbiTable {
        version: DriverVersion {
            major: 610,
            minor: 43,
            patch: 2,
        },
        map_dma: MapDmaWire::From580_65_06,
        gsp_element: GspElementWire::From610_43_02,
        gsp_init_args: GspInitArgsWire::NineField,
        caps: &CAPS_580_65_06,
        vbios: VbiosWire::Tu102Bit,
        note: "★ the GSP element header changes shape here: 48 bytes with an \
               elemCount become 16 with MCTP/NVDM transport words, and \
               MESSAGE_QUEUE_INIT_ARGUMENTS grows from 4 fields to 9 \
               (ogkm-610: message_queue_priv.h:52-67, gsp_init_args.h:32-45 vs \
               ogkm-580: message_queue_priv.h:43-51, gsp_init_args.h:29-34). Also \
               the ogkm tag every generated layout in this crate came from",
    },
];

/// The driver version this project's bench actually runs
/// (`docs/reference/rm_semantics_measured.md` §0).
///
/// It sits **above** 580.65.06, so the bench is on the 64-byte `NVOS46` — which
/// is what the C artifact's *runtime* profile also selects
/// (`nvkvm_abi.h:79-87`), and what its *parity test* does not
/// (`abi_parity_test.go:68` asserts 56 unconditionally). The two disagree; the
/// runtime is right.
pub const BENCH_DRIVER: DriverVersion = DriverVersion {
    major: 580,
    minor: 159,
    patch: 4,
};

/// Select the ABI table for a driver version: the newest entry `<= version`.
///
/// # Errors
///
/// [`AbiError::NoTableForVersion`] if `version` predates every entry. There is no
/// fallback entry by design — see the module doc.
pub fn table_for(version: DriverVersion) -> Result<&'static DriverAbiTable, AbiError> {
    TABLES
        .iter()
        .rev()
        .find(|t| t.version <= version)
        .ok_or(AbiError::NoTableForVersion {
            major: version.major,
            minor: version.minor,
            patch: version.patch,
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
            flags: u32_at(bytes, 20)?,
            h_ctx_share: u32_at(bytes, 24)?,
            h_vaspace: u32_at(bytes, 28)?,
        })
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
            // ★ Mapped, and declaring nothing the object model reads. A VASpace's
            // params are geometry (`index`, `vaSize`, `vaBase`, `pasid`) and an
            // engine object's are engine-private; the protocol content of all
            // three is the EDGE — parent, handle, class — which the RPC header
            // already carries.
            classes::FERMI_VASPACE_A | classes::AMPERE_COMPUTE_B | classes::AMPERE_DMA_COPY_B => {
                Some(AllocParams::NoDeclaredFacts)
            }
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
            NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY
            | NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES
            | NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER => {
                Some(ControlParams::PageDirNotModelled)
            }
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
        if payload.len() < RpcControlReq::HEADER {
            return Err(AbiError::Truncated {
                c_name: RpcControlReq::C_NAME,
                need: RpcControlReq::HEADER,
                got: payload.len(),
            });
        }
        Ok(RpcControlReq {
            client: u32_at(payload, 0)?,
            object: u32_at(payload, 4)?,
            cmd: u32_at(payload, 8)?,
            // +12 is `status`, an [OUT] field the guest sends as zero —
            // `rpcWriteCommonHeader` zeroes the whole message buffer before the
            // sender fills it (`ogkm-580: src/nvidia/src/kernel/rmapi/rpc_common.c:149-152`
            // / `ogkm-610: src/nvidia/src/kernel/rmapi/rpc_common.c:149-152` — same lines
            // at both).
            params_size: u32_at(payload, 16)?,
            rmapi_rpc_flags: u32_at(payload, 20)?,
            // +24 `rmctrlFlags`, +28 `rmctrlAccessRight` (both sent as 0 by
            // `rpcRmApiControl_GSP`, `ogkm-580: rpc.c:10994-10995` /
            // `ogkm-610: rpc.c:10799-10800`), +32 `reserved0`.
            params_at: RpcControlReq::HEADER,
        })
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
    /// ★★ **Known to move a VASpace's page-directory binding, and not modelled.**
    ///
    /// Three commands, three different reasons, one answer — and the answer is a
    /// *named* refusal rather than silence, because the absence of a PDB is
    /// invisible downstream (a channel simply defers at its first doorbell,
    /// forever):
    ///
    /// - `NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES` (`0x90f10106`) —
    ///   carries the root PD as `levels[0].physAddress` for **every ordinary
    ///   RM-managed VAS**, at construct time. Not decoded here: its params are a
    ///   184-byte struct ending in a six-element array of 24-byte level records
    ///   whose size has been *computed*, not read from any layout assertion, so
    ///   it needs the generator and a `RUSTC_OFFSETS` pin rather than a
    ///   hand-transcription.
    /// - `NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER`
    ///   (`0x20800a9f`) — the same payload for the GPU-group global VAS.
    /// - `NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY` (`0x00801814`) — the
    ///   *revocation*. `RmEvent` has no verb for it, and inventing one is a core
    ///   change rather than a bridge change.
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

    /// The table is ascending and has no duplicate versions — `table_for`'s
    /// reverse scan is only correct if it is.
    #[test]
    fn the_table_is_strictly_ascending() {
        for w in TABLES.windows(2) {
            assert!(
                w[0].version < w[1].version,
                "{:?} !< {:?}",
                w[0].version,
                w[1].version
            );
        }
    }

    /// Selection is "newest entry <= version", asserted at every boundary
    /// including one patch below each one. This is the assertion the C's
    /// major-only key cannot make.
    #[test]
    fn selection_lands_on_the_exact_boundary_not_the_major() {
        let at = |maj, min, pat| {
            table_for(DriverVersion {
                major: maj,
                minor: min,
                patch: pat,
            })
            .expect("in range")
            .map_dma_wire()
        };
        // Exactly the floor.
        assert_eq!(at(550, 54, 4), MapDmaWire::Pre580_65_06);
        // Well inside the pre-580 range.
        assert_eq!(at(575, 51, 2), MapDmaWire::Pre580_65_06);
        // ★ 580.65.05 is a 580 but PRE-boundary. A major-only key gets this
        // wrong; this is the whole reason the table is keyed on all three.
        assert_eq!(at(580, 65, 5), MapDmaWire::Pre580_65_06);
        assert_eq!(at(580, 64, 255), MapDmaWire::Pre580_65_06);
        // Exactly the boundary.
        assert_eq!(at(580, 65, 6), MapDmaWire::From580_65_06);
        // The bench.
        assert_eq!(at(580, 159, 4), MapDmaWire::From580_65_06);
        // Newer than every entry inherits the newest.
        assert_eq!(at(999, 0, 0), MapDmaWire::From580_65_06);
    }

    /// Below the floor is a refusal naming the version — never the nearest
    /// table. (`nvkvm_abi.h:105-110` returns the 570 profile here.)
    #[test]
    fn below_the_floor_is_a_loud_refusal_not_the_nearest_table() {
        for (maj, min, pat) in [
            (550u16, 54u16, 3u16),
            (550, 53, 255),
            (535, 104, 5),
            (0, 0, 0),
        ] {
            assert_eq!(
                table_for(DriverVersion {
                    major: maj,
                    minor: min,
                    patch: pat
                })
                .map(|t| t.version),
                Err(AbiError::NoTableForVersion {
                    major: maj,
                    minor: min,
                    patch: pat
                }),
                "{maj}.{min}.{pat} must not resolve to a table",
            );
        }
    }

    /// The bench driver constant resolves, and resolves to the 64-byte NVOS46.
    /// Non-vacuity for the whole version story: if this ever flips, the crate is
    /// decoding the bench's own traffic wrong.
    #[test]
    fn the_bench_driver_resolves_to_the_64_byte_nvos46() {
        let t = table_for(BENCH_DRIVER).expect("the bench driver is supported");
        assert_eq!(t.map_dma_wire(), MapDmaWire::From580_65_06);
        assert_eq!(t.map_dma_size(), 64);
        assert_eq!(t.map_dma_status_offset(), 56);
    }

    /// The two sizes and the two status offsets, pinned against the C artifact's
    /// own hand-maintained table (`nvkvm_abi.h:66,76,86`).
    #[test]
    fn sizes_and_status_offsets_match_the_c_artifacts_profile_table() {
        let old = table_for(DriverVersion {
            major: 575,
            minor: 51,
            patch: 2,
        })
        .expect("in range");
        assert_eq!(old.map_dma_size(), 56, "nvkvm_abi.h:76 .nvos46_size = 56");
        assert_eq!(
            old.map_dma_status_offset(),
            48,
            "nvkvm_abi.h:76 .nvos46_status_off = 48"
        );
        let new = table_for(DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        })
        .expect("in range");
        assert_eq!(new.map_dma_size(), 64, "nvkvm_abi.h:86 .nvos46_size = 64");
        assert_eq!(
            new.map_dma_status_offset(),
            56,
            "nvkvm_abi.h:86 .nvos46_status_off = 56"
        );
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
