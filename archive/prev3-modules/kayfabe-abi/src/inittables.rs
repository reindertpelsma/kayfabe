//! Two `GSP_RM_CONTROL` **reply bodies** the guest's RM cannot start without: the FIFO
//! device-info table and the kernel interrupt table.
//!
//! ## ★★★ Why a reply body is a different kind of thing from everything else here
//!
//! The rest of this crate decodes what a guest *sent*. These two encode what a GSP
//! *answers*, and the difference matters because an answer has no fallback: an echoed
//! reply carries the guest's own zeroed `[OUT]` buffer, and RM reads it as a table of
//! length zero. Measured on a stock 580.159.04 guest at `3fb3fca` — with
//! [`kayfabe_gsp::EchoOk`] in force, `kfifoGetHostDeviceInfoTable_KERNEL` allocates
//! `sizeof(entry) * 0`, gets `NULL`, and RM bails:
//!
//! ```text
//! NVRM: Check failed: pEngineInfo->engineInfoList != NULL @ kernel_fifo.c:2095
//! NVRM: [NV_ERR_NO_MEMORY] returned from kfifoGetHostDeviceInfoTable_HAL @ kernel_fifo.c:2211
//! NVRM: [NV_ERR_INVALID_ARGUMENT] returned from vectReserve(&pIntr->intrTable, ...) @ intr.c:1067
//! NVRM: RmInitNvDevice: *** Cannot pre-initialize the device
//! ```
//!
//! Both refusals are the **same** defect seen twice. `vectReserve` asserts `n > 0`
//! (`ogkm-580: src/nvidia/src/libraries/containers/vector.c:173`), so a zero `tableLen`
//! is `NV_ERR_INVALID_ARGUMENT`; `portMemAllocNonPaged(0)` is `NULL`, so a zero
//! `numEntries` is `NV_ERR_NO_MEMORY`. Neither is a size check that a bigger buffer
//! fixes — they are RM saying *you told me this device has nothing*.
//!
//! ## ★★ These are ENCODERS over rows, not blobs
//!
//! The rows live on the chip profile (`kayfabe_device::ChipProfile::engines`), because an
//! engine table is a fact about silicon. What lives here is only the wire layout: field
//! order, strides, the `[OUT]` sizes RM allocates against, and the refusals for a row that
//! cannot be expressed. A second generation adds rows; it does not touch this file.
//!
//! ## ★ Both `paramsSize` values are FIXED, and that is load-bearing
//!
//! RM sends the whole `[OUT]` struct and copies back exactly `paramsSize` bytes
//! (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11083` — `portMemCopy(pParamStructPtr,
//! paramsSize, rpc_params->params, paramsSize)`), so a reply that encodes only the
//! populated prefix leaves the tail of RM's struct holding whatever the request had.
//! For the interrupt table that tail is `subtreeMap`, which is **not** optional:
//! `intrInitInterruptTable_KERNEL` copies it straight into `pIntr->subtreeMap`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/intr/intr.c:1084-1085`). Both encoders therefore
//! return the full struct, zero-filled, every time.

/// `NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE` — the engine enumeration
/// `kfifoGetHostDeviceInfoTable_KERNEL` issues on `hInternalClient`/`hInternalSubdevice`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_fifo.c:2058-2063`).
///
/// ogkm `src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080fifo.h:618`.
pub const NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE: u32 = 0x2080_1112;

/// `NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE` — the vector map
/// `intrInitInterruptTable_KERNEL` issues.
///
/// ogkm `src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:1460`.
pub const NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE: u32 = 0x2080_0a5c;

/// `NV2080_CTRL_FIFO_GET_DEVICE_INFO_TABLE_ENGINE_DATA_TYPES` — the width of
/// `engineData[]`. Indexed by `ENGINE_INFO_TYPE`
/// (`ogkm-580: src/nvidia/inc/kernel/gpu/fifo/engine_info.h:37-104`), whose 15 named
/// members are followed by one unused slot.
pub const ENGINE_DATA_TYPES: usize = 16;

/// `NV2080_CTRL_FIFO_GET_DEVICE_INFO_TABLE_ENGINE_MAX_PBDMA`.
pub const ENGINE_MAX_PBDMA: usize = 2;

/// `NV2080_CTRL_FIFO_GET_DEVICE_INFO_TABLE_ENGINE_MAX_NAME_LEN` — including the NUL.
pub const ENGINE_MAX_NAME_LEN: usize = 16;

/// `NV2080_CTRL_FIFO_GET_DEVICE_INFO_TABLE_MAX_ENTRIES` — how many entries fit in one
/// reply. RM pages with `baseIndex` in strides of this.
pub const DEVICE_INFO_MAX_ENTRIES: usize = 32;

/// `sizeof(NV2080_CTRL_FIFO_DEVICE_ENTRY)`.
pub const DEVICE_ENTRY_SIZE: usize = ENGINE_DATA_TYPES * 4 + ENGINE_MAX_PBDMA * 4 * 2 + 4 + 16;

/// `sizeof(NV2080_CTRL_FIFO_GET_DEVICE_INFO_TABLE_PARAMS)` — `baseIndex`, `numEntries`,
/// `bMore` (an `NvBool` in a 4-byte slot), then `entries[32]`.
pub const DEVICE_INFO_PARAMS_SIZE: usize = 12 + DEVICE_INFO_MAX_ENTRIES * DEVICE_ENTRY_SIZE;

/// `NV2080_CTRL_INTERNAL_INTR_MAX_TABLE_SIZE`.
pub const INTR_MAX_TABLE_SIZE: usize = 128;

/// `sizeof(NV2080_CTRL_INTERNAL_INTR_GET_KERNEL_TABLE_ENTRY)` — an `NvU16` followed by
/// three `NvU32`, so 2 bytes of tail padding after `engineIdx`.
pub const INTR_ENTRY_SIZE: usize = 16;

/// `NV2080_CTRL_INTR_CATEGORY_ENUM_COUNT` — `ogkm-580:
/// src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080mc.h:337`, pinned by that header's own
/// `ct_assert` against the vGPU copy.
pub const INTR_CATEGORY_COUNT: usize = 7;

/// Byte offset of `subtreeMap[]` inside the params.
///
/// ★ `4 + 128*16 = 2052` is not 8-aligned and the member is `NV_DECLARE_ALIGNED(..., 8)`,
/// so the compiler inserts **four bytes of padding** that no field name mentions. Writing
/// the map at 2052 puts every mask one slot low, and the symptom would be an assert deep
/// inside `intrCacheIntrFields` rather than anything pointing here.
pub const INTR_SUBTREE_MAP_OFF: usize = 2056;

/// `sizeof(NV2080_CTRL_INTERNAL_INTR_GET_KERNEL_TABLE_PARAMS)`.
pub const INTR_PARAMS_SIZE: usize = INTR_SUBTREE_MAP_OFF + INTR_CATEGORY_COUNT * 8;

/// One row of the FIFO device-info table: one engine, as the guest's RM will file it.
///
/// Deliberately **not** `#[repr(C)]`. The wire form is produced by
/// [`encode_device_info_table`] and nothing else, so there is exactly one description of
/// the layout and a row is free to be ergonomic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FifoDeviceEntry {
    /// `engineName[16]` — RM copies it out for diagnostics and never branches on it.
    /// Must be shorter than [`ENGINE_MAX_NAME_LEN`] so the NUL fits.
    pub name: &'static str,
    /// `engineData[]`, indexed by `ENGINE_INFO_TYPE`.
    pub engine_data: [u32; ENGINE_DATA_TYPES],
    /// `pbdmaIds[]`; only the first [`FifoDeviceEntry::num_pbdmas`] are read.
    pub pbdma_ids: [u32; ENGINE_MAX_PBDMA],
    /// `pbdmaFaultIds[]`; likewise.
    pub pbdma_fault_ids: [u32; ENGINE_MAX_PBDMA],
    /// `numPbdmas`. RM asserts this against its own array bounds
    /// (`ogkm-580: kernel_fifo.c:2117-2124`), so a row over [`ENGINE_MAX_PBDMA`] is
    /// refused here rather than encoded into an `NV_ERR_INVALID_STATE` in the guest.
    pub num_pbdmas: u32,
}

/// The `ENGINE_INFO_TYPE` slots of [`FifoDeviceEntry::engine_data`] that this port reads
/// or reasons about. Named so a chip row is readable and a gate can be written against
/// one, rather than every table site spelling a bare index.
///
/// ogkm `src/nvidia/inc/kernel/gpu/fifo/engine_info.h:37-104`.
pub mod engine_info_type {
    /// `ENGINE_INFO_TYPE_ENG_DESC` — `MKENGDESC(classId, instance)`.
    pub const ENG_DESC: usize = 0;
    /// `ENGINE_INFO_TYPE_RM_ENGINE_TYPE`.
    pub const RM_ENGINE_TYPE: usize = 2;
    /// `ENGINE_INFO_TYPE_RUNLIST`.
    pub const RUNLIST: usize = 3;
    /// `ENGINE_INFO_TYPE_INSTANCE_ID`.
    pub const INSTANCE_ID: usize = 10;
    /// `ENGINE_INFO_TYPE_IS_HOST_DRIVEN_ENGINE` — non-zero makes RM count a runlist.
    pub const IS_HOST_DRIVEN_ENGINE: usize = 12;
}

/// One row of the kernel interrupt table: where one MC engine's interrupt would arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntrTableEntry {
    /// `engineIdx` — an `MC_ENGINE_IDX_*`, RM's own index space, not an engine type.
    pub engine_idx: u16,
    /// `pmcIntrMask`. Zero on every post-Volta part: the vector table replaced PMC.
    pub pmc_intr_mask: u32,
    /// `vectorStall`, or [`INTR_VECTOR_INVALID`].
    pub vector_stall: u32,
    /// `vectorNonStall`, or [`INTR_VECTOR_INVALID`].
    pub vector_non_stall: u32,
}

/// `NV2080_INTR_VECTOR_INVALID` — the "this engine has no vector of that kind" marker.
pub const INTR_VECTOR_INVALID: u32 = 0xffff_ffff;

/// `MC_ENGINE_IDX_CE0` — `ogkm-580: src/nvidia/inc/kernel/gpu/intr/engine_idx.h:54`.
pub const MC_ENGINE_IDX_CE0: u16 = 15;

/// `MC_ENGINE_IDX_CE_MAX` is `MC_ENGINE_IDX_CE19` = 34, so twenty rows
/// (`ogkm-580: engine_idx.h:73-74`).
pub const MC_ENGINE_IDX_CE_COUNT: u16 = 20;

/// `MC_ENGINE_IDX_CE(x)` (`ogkm-580: engine_idx.h:173`), bounded by `MC_ENGINE_IDX_IS_CE`
/// (`:187-188`).
///
/// ⊘ `None` past `CE19` rather than a computed number: `MC_ENGINE_IDX_VIC` is 35, i.e. the
/// row **immediately** after `CE19`, so an unbounded `CE0 + x` does not run off into unused
/// space — it names a different engine, and the interrupt table would answer for it.
#[must_use]
pub const fn mc_engine_idx_ce(index: u32) -> Option<u16> {
    if index < MC_ENGINE_IDX_CE_COUNT as u32 {
        Some(MC_ENGINE_IDX_CE0 + index as u16)
    } else {
        None
    }
}

/// Why a table could not be encoded.
///
/// Every variant is a row this port refuses to put on the wire, and each names both the
/// row and the bound — because the alternative is a guest-side assert whose message
/// mentions neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitTableError {
    /// An engine name has no room for its NUL terminator.
    EngineNameTooLong {
        /// The offending name.
        name: &'static str,
        /// Its length in bytes.
        len: usize,
        /// The most that fits, terminator included.
        max: usize,
    },
    /// A row declares more PBDMAs than the wire entry can carry.
    TooManyPbdmas {
        /// The engine that declared it.
        name: &'static str,
        /// What it declared.
        num_pbdmas: u32,
        /// The array bound.
        max: usize,
    },
    /// More interrupt rows than `table[]` holds.
    IntrTableTooLong {
        /// How many rows were offered.
        len: usize,
        /// The array bound.
        max: usize,
    },
    /// A `baseIndex` RM could not have sent: it pages in strides of
    /// [`DEVICE_INFO_MAX_ENTRIES`], so anything else means the request was not the one
    /// this encoder models.
    UnalignedBaseIndex {
        /// The index asked for.
        base_index: u32,
        /// The stride it must be a multiple of.
        stride: usize,
    },
}

impl core::fmt::Display for InitTableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EngineNameTooLong { name, len, max } => write!(
                f,
                "engine name {name:?} is {len} bytes; engineName[] holds {max} including \
                 the terminator"
            ),
            Self::TooManyPbdmas {
                name,
                num_pbdmas,
                max,
            } => write!(
                f,
                "engine {name:?} declares {num_pbdmas} PBDMAs; the wire entry carries {max}"
            ),
            Self::IntrTableTooLong { len, max } => {
                write!(f, "{len} interrupt rows; table[] holds {max}")
            }
            Self::UnalignedBaseIndex { base_index, stride } => write!(
                f,
                "baseIndex {base_index} is not a multiple of {stride}; RM pages in that \
                 stride, so this is not a request this encoder models"
            ),
        }
    }
}

impl core::error::Error for InitTableError {}

/// What one page of the device-info table came out as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfoPage {
    /// The full [`DEVICE_INFO_PARAMS_SIZE`]-byte `[OUT]` struct.
    pub params: Vec<u8>,
    /// How many entries this page carries — `numEntries` as encoded.
    pub num_entries: u32,
    /// Whether a further page follows — `bMore` as encoded.
    pub more: bool,
}

/// Encode one page of `NV2080_CTRL_FIFO_GET_DEVICE_INFO_TABLE_PARAMS`.
///
/// `base_index` is echoed back and selects the window of `entries`, exactly as RM's own
/// paging loop expects (`ogkm-580: kernel_fifo.c:2050-2087`). A `base_index` past the end
/// is not an error — it yields `numEntries = 0, bMore = false`, which is how RM's loop
/// would terminate if it ever ran one page long.
///
/// # Errors
///
/// [`InitTableError::UnalignedBaseIndex`], [`InitTableError::EngineNameTooLong`] or
/// [`InitTableError::TooManyPbdmas`].
pub fn encode_device_info_table(
    entries: &[FifoDeviceEntry],
    base_index: u32,
) -> Result<DeviceInfoPage, InitTableError> {
    let stride = DEVICE_INFO_MAX_ENTRIES;
    if !(base_index as usize).is_multiple_of(stride) {
        return Err(InitTableError::UnalignedBaseIndex { base_index, stride });
    }
    for e in entries {
        if e.name.len() >= ENGINE_MAX_NAME_LEN {
            return Err(InitTableError::EngineNameTooLong {
                name: e.name,
                len: e.name.len(),
                max: ENGINE_MAX_NAME_LEN,
            });
        }
        if e.num_pbdmas as usize > ENGINE_MAX_PBDMA {
            return Err(InitTableError::TooManyPbdmas {
                name: e.name,
                num_pbdmas: e.num_pbdmas,
                max: ENGINE_MAX_PBDMA,
            });
        }
    }

    let start = (base_index as usize).min(entries.len());
    let page = &entries[start..entries.len().min(start + stride)];
    let more = start + page.len() < entries.len();

    let mut params = vec![0u8; DEVICE_INFO_PARAMS_SIZE];
    params[0..4].copy_from_slice(&base_index.to_le_bytes());
    params[4..8].copy_from_slice(&u32::try_from(page.len()).unwrap_or(u32::MAX).to_le_bytes());
    // `bMore` is an `NvBool` — one byte, in a 4-byte slot the struct's alignment creates.
    params[8] = u8::from(more);

    for (i, e) in page.iter().enumerate() {
        let o = 12 + i * DEVICE_ENTRY_SIZE;
        for (j, w) in e.engine_data.iter().enumerate() {
            params[o + j * 4..o + j * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let pb = o + ENGINE_DATA_TYPES * 4;
        for (j, w) in e.pbdma_ids.iter().enumerate() {
            params[pb + j * 4..pb + j * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let pf = pb + ENGINE_MAX_PBDMA * 4;
        for (j, w) in e.pbdma_fault_ids.iter().enumerate() {
            params[pf + j * 4..pf + j * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let np = pf + ENGINE_MAX_PBDMA * 4;
        params[np..np + 4].copy_from_slice(&e.num_pbdmas.to_le_bytes());
        let nm = np + 4;
        params[nm..nm + e.name.len()].copy_from_slice(e.name.as_bytes());
        // The remaining bytes are already zero, which is the terminator.
    }

    Ok(DeviceInfoPage {
        params,
        num_entries: u32::try_from(page.len()).unwrap_or(u32::MAX),
        more,
    })
}

/// Encode `NV2080_CTRL_INTERNAL_INTR_GET_KERNEL_TABLE_PARAMS`.
///
/// # Errors
///
/// [`InitTableError::IntrTableTooLong`].
pub fn encode_intr_kernel_table(
    entries: &[IntrTableEntry],
    subtree_map: &[u64; INTR_CATEGORY_COUNT],
) -> Result<Vec<u8>, InitTableError> {
    if entries.len() > INTR_MAX_TABLE_SIZE {
        return Err(InitTableError::IntrTableTooLong {
            len: entries.len(),
            max: INTR_MAX_TABLE_SIZE,
        });
    }
    let mut params = vec![0u8; INTR_PARAMS_SIZE];
    params[0..4].copy_from_slice(
        &u32::try_from(entries.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    for (i, e) in entries.iter().enumerate() {
        let o = 4 + i * INTR_ENTRY_SIZE;
        params[o..o + 2].copy_from_slice(&e.engine_idx.to_le_bytes());
        // +2 is the padding the `NvU16` leaves before the first `NvU32`.
        params[o + 4..o + 8].copy_from_slice(&e.pmc_intr_mask.to_le_bytes());
        params[o + 8..o + 12].copy_from_slice(&e.vector_stall.to_le_bytes());
        params[o + 12..o + 16].copy_from_slice(&e.vector_non_stall.to_le_bytes());
    }
    for (i, m) in subtree_map.iter().enumerate() {
        let o = INTR_SUBTREE_MAP_OFF + i * 8;
        params[o..o + 8].copy_from_slice(&m.to_le_bytes());
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    const E: FifoDeviceEntry = FifoDeviceEntry {
        name: "GR0",
        engine_data: [0; ENGINE_DATA_TYPES],
        pbdma_ids: [0, 1],
        pbdma_fault_ids: [0x20, 0x21],
        num_pbdmas: 2,
    };

    #[test]
    fn params_sizes_are_the_sizes_rm_allocates() {
        // The two numbers RM's `sizeof` produces. If either drifts, the guest copies back
        // a different number of bytes than we wrote and the tail is the request's.
        assert_eq!(DEVICE_ENTRY_SIZE, 100);
        assert_eq!(DEVICE_INFO_PARAMS_SIZE, 3212);
        assert_eq!(INTR_PARAMS_SIZE, 2112);
        // The alignment hole before `subtreeMap`, stated as its own width so the constant
        // cannot be "simplified" back onto the packed offset. Induced 2026-07-31 on this branch
        // (`cargo test -p kayfabe-device --test init_tables`): with the constant moved to
        // 2052 the whole map reads one slot low, and the golden test caught it only after
        // it was changed to spell 2056 as a literal rather than importing this constant.
        let packed_end = 4 + INTR_MAX_TABLE_SIZE * INTR_ENTRY_SIZE;
        assert_eq!(packed_end, 2052);
        assert_eq!(INTR_SUBTREE_MAP_OFF - packed_end, 4);
    }

    #[test]
    fn a_name_with_no_room_for_its_terminator_is_refused() {
        let mut e = E;
        e.name = "0123456789abcdef"; // exactly 16 — the NUL would not fit
        let err = encode_device_info_table(&[e], 0).unwrap_err();
        assert_eq!(
            err,
            InitTableError::EngineNameTooLong {
                name: "0123456789abcdef",
                len: 16,
                max: 16
            }
        );
    }

    #[test]
    fn a_row_over_the_pbdma_bound_is_refused() {
        let mut e = E;
        e.num_pbdmas = 3;
        assert_eq!(
            encode_device_info_table(&[e], 0).unwrap_err(),
            InitTableError::TooManyPbdmas {
                name: "GR0",
                num_pbdmas: 3,
                max: 2
            }
        );
    }

    #[test]
    fn a_base_index_off_the_paging_stride_is_refused() {
        assert_eq!(
            encode_device_info_table(&[E], 1).unwrap_err(),
            InitTableError::UnalignedBaseIndex {
                base_index: 1,
                stride: 32
            }
        );
    }

    #[test]
    fn paging_reports_more_and_then_stops() {
        let many = [E; 33];
        let p0 = encode_device_info_table(&many, 0).unwrap();
        assert_eq!((p0.num_entries, p0.more), (32, true));
        let p1 = encode_device_info_table(&many, 32).unwrap();
        assert_eq!((p1.num_entries, p1.more), (1, false));
        assert_eq!(u32::from_le_bytes(p1.params[0..4].try_into().unwrap()), 32);
        // A page past the end terminates rather than erroring — RM's loop breaks on
        // `bMore`, so this is only reachable if it ever ran one page long.
        let p2 = encode_device_info_table(&many, 64).unwrap();
        assert_eq!((p2.num_entries, p2.more), (0, false));
    }

    #[test]
    fn an_intr_table_over_the_bound_is_refused() {
        let e = IntrTableEntry {
            engine_idx: 1,
            pmc_intr_mask: 0,
            vector_stall: INTR_VECTOR_INVALID,
            vector_non_stall: INTR_VECTOR_INVALID,
        };
        let many = [e; INTR_MAX_TABLE_SIZE + 1];
        assert_eq!(
            encode_intr_kernel_table(&many, &[0; INTR_CATEGORY_COUNT]).unwrap_err(),
            InitTableError::IntrTableTooLong { len: 129, max: 128 }
        );
    }

    #[test]
    fn the_subtree_map_lands_past_the_alignment_hole() {
        let map = [1, 2, 3, 4, 5, 6, 7];
        let p = encode_intr_kernel_table(&[], &map).unwrap();
        // The four bytes the padding creates stay zero...
        assert_eq!(&p[2052..2056], &[0, 0, 0, 0]);
        // ...and the first mask is at 2056, not 2052.
        assert_eq!(u64::from_le_bytes(p[2056..2064].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(p[2104..2112].try_into().unwrap()), 7);
    }
}
