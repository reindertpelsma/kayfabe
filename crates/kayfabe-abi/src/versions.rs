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

use crate::generated::{classes, ctrl, nvos, rpc};
use crate::transcribed::Nvos46ParametersPre580;
use crate::view::{
    AllocReq, AllocWire, ClientAllocFacts, ControlReq, DeviceAllocFacts, DupReq, FreeReq,
    MapMemoryDma, PdbAperture, RpcAllocReq, RpcEnvelope, SetPageDir, UnmapMemoryDma,
    rpc_payload_len,
};
use crate::wire::{AbiError, u32_at};
use crate::{DriverAbi, DriverVersion};
use kayfabe_arch::ids::ClassId;

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
        note: "oldest supported: NVOS47 gained `size` here \
               (gvisor/pkg/abi/nvgpu/frontend.go:707-710, NVOS47_PARAMETERS_V550)",
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
        note: "NVOS46 gained flags2+kindOverride \
               (gvisor/pkg/sentry/devices/nvproxy/version.go:1057-1059)",
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

    /// Decode a `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` payload.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`]. The struct's tail (`chId`, `subDeviceId`,
    /// `pasid`) is confirmed only by ogkm 610.43.02, so a shorter payload from an
    /// older driver is refused rather than read past — see the generated module's
    /// version caveat.
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

    /// Decode the fixed header of a `GSP_RM_ALLOC` **RPC body** (everything after
    /// the 32-byte `rpc_message_header`), i.e. `rpc_gsp_rm_alloc_v03_00`.
    ///
    /// Not versioned within the supported range: the struct lives in NVIDIA's
    /// OS-independent RM core and has carried these seven fields since `_v03_00`
    /// (`ogkm: g_rpc-structures.h:1408-1419`). It takes the version table anyway,
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
    /// kind to RM (`ogkm: src/common/sdk/nvidia/inc/class/cl0000.h:42`,
    /// `ogkm: src/nvidia/generated/g_allclasses.h:289`); the generated module's
    /// own doc on [`classes::NV01_ROOT_CLIENT`] says the same.
    ///
    /// Lives here rather than in the bridge for the quarantine reason
    /// (decision #2): the NVIDIA class *numbers* are this crate's, and the crates
    /// above it speak a predicate. Same shape as
    /// [`crate::client_kind_from_process_id`].
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
