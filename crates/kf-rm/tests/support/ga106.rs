//! ★ **TEST FIXTURE ONLY — the old tree's captured GA106 rows, as a [`HostFacts`] value.**
//!
//! These rows are what `kayfabe-device/src/ga10x.rs` (`GA106: ChipProfile`, frozen reference)
//! served, copied verbatim. In v3 they are the ORACLE, never a source: a real device's
//! [`HostFacts`] is filled from the host at realize, and on a GA106 host the derived value
//! must equal these (`tests/host_facts_ga106.rs`). ⊘ Nothing outside `tests/` may include
//! this file — a per-die row in `src/` is exactly the shape v3 exists to end.
//!
//! Shared by `#[path]` with `kf-crec`'s reply-plane differential.

#![allow(dead_code)]

use std::sync::Arc;

use kf_abi::bifstatic::BifStaticRow;
use kf_abi::chipinfo::{ChipInfoRow, RegBaseRow, reg_base};
use kf_abi::confcompute::ConfComputeRow;
use kf_abi::deviceinfo::{DeviceInfoRow, DevicePriBase, EnginePriBase};
use kf_abi::falconinfo::FalconInventoryRow;
use kf_abi::fifochannels::FifoChannelsRow;
use kf_abi::gmmustatic::GmmuStaticRow;
use kf_abi::gspstaticinfo::FbRegion;
use kf_abi::inittables::{FifoDeviceEntry, INTR_CATEGORY_COUNT, IntrTableEntry};
use kf_abi::memsysconfig::{ComptagAllocationPolicy, MemorySystemRow, RAM_TYPE_GDDR6};
use kf_abi::pcibars::PciBarRow;
use kf_abi::regaccessmap::RegisterAccessMapRow;
use kf_rm::{BoardFacts, HostFacts};

/// The GA106's compiled framebuffer size (old `ga10x::FB_SIZE_MB`).
pub const FB_SIZE_MB: u64 = 12288;
/// Old `ga10x::FW_CARVE_OUT_BYTES`.
pub const FW_CARVE_OUT_BYTES: u64 = 0x1042_0000;
/// Old `ga10x::REGS_APERTURE_LEN`.
pub const REGS_APERTURE_LEN: u64 = 16 << 20;
/// Old `ga10x::VIRTUAL_FUNCTION_BASE`.
const VIRTUAL_FUNCTION_BASE: u32 = 0x00BB_0000;

/// Old `ga10x::bar1_pde_base_for`.
pub const fn bar1_pde_base_for(fb_size_mb: u64) -> u64 {
    const ABOVE_CARVE_OUT_BASE: u64 = 0x20C_C000;
    (kf_chip::falcon_gsp::fb_length_for(fb_size_mb) - FW_CARVE_OUT_BYTES) + ABOVE_CARVE_OUT_BASE
}

/// The BAR2 root the same GSP declared (`bar2PdeBase`, cap1b record 141977 byte 1672):
/// `0x2_F339_2000` at 12 GiB, i.e. `0x37B_2000` above the carve-out base.
pub const fn bar2_pde_base_for(fb_size_mb: u64) -> u64 {
    const ABOVE_CARVE_OUT_BASE: u64 = 0x37B_2000;
    (kf_chip::falcon_gsp::fb_length_for(fb_size_mb) - FW_CARVE_OUT_BYTES) + ABOVE_CARVE_OUT_BASE
}

/// Old `ga10x::GA106_FB_REGIONS`, for any size (old `ga106_profile(fb_size_mb)`).
pub fn fb_regions(fb_size_mb: u64) -> Vec<FbRegion> {
    let fb_length = kf_chip::falcon_gsp::fb_length_for(fb_size_mb);
    let carve = fb_length - FW_CARVE_OUT_BYTES;
    vec![
        FbRegion {
            base: 0,
            limit: carve - 1,
            reserved: 0,
            performance: 6,
            support_compressed: true,
            support_iso: true,
            protected: false,
        },
        FbRegion {
            base: carve,
            limit: fb_length - 1,
            reserved: FW_CARVE_OUT_BYTES,
            performance: 0,
            support_compressed: false,
            support_iso: false,
            protected: false,
        },
    ]
}

/// Old `ga10x::GA106_PCI_BARS`.
pub fn pci_bars() -> Vec<PciBarRow> {
    vec![
        PciBarRow { name: "registers", size_bytes: REGS_APERTURE_LEN },
        PciBarRow { name: "framebuffer-window", size_bytes: 256 << 20 },
        PciBarRow { name: "instance-window", size_bytes: 32 << 20 },
        PciBarRow { name: "io", size_bytes: 0 },
    ]
}

/// The GA106 board (RTX 3060 `10de:2504`, MSI subsystem), at `fb_size_mb`.
pub fn board_at(fb_size_mb: u64) -> BoardFacts {
    BoardFacts {
        fb_regions: fb_regions(fb_size_mb),
        fb_length: kf_chip::falcon_gsp::fb_length_for(fb_size_mb),
        bar1_pde_base: bar1_pde_base_for(fb_size_mb),
        bar2_pde_base: bar2_pde_base_for(fb_size_mb),
        pci_vendor_id: 0x10de,
        pci_device_id: 0x2504,
        pci_revision: 0xA1,
        pci_subsystem_vendor_id: 0x1462,
        pci_subsystem_id: 0x397D,
        pci_bars: pci_bars(),
    }
}

/// The GA106 board at its compiled size.
pub fn board() -> Arc<BoardFacts> {
    Arc::new(board_at(FB_SIZE_MB))
}

pub static ENGINES: &[FifoDeviceEntry] = &[
    FifoDeviceEntry {
        name: "GR0",
        engine_data: [
            0xd334df00, // [ 0] ENG_DESC
            0x00000000, // [ 1] FIFO_TAG
            0x00000001, // [ 2] RM_ENGINE_TYPE
            0x00000000, // [ 3] RUNLIST
            0x00000040, // [ 4] MMU_FAULT_ID
            0x00000033, // [ 5] RC_MASK
            0x0000000c, // [ 6] RESET
            0x00000000, // [ 7] INTR
            0x00000054, // [ 8] MC
            0x00000000, // [ 9] DEV_TYPE_ENUM
            0x00000000, // [10] INSTANCE_ID
            0x00c00000, // [11] RUNLIST_PRI_BASE
            0x00000001, // [12] IS_HOST_DRIVEN_ENGINE
            0x00000000, // [13] RUNLIST_ENGINE_ID
            0x00c20000, // [14] CHRAM_PRI_BASE
            0x00000000, // [15] KERNEL_RM_MAX
        ],
        pbdma_ids: [0x00000000, 0x00000001],
        pbdma_fault_ids: [0x00000020, 0x00000021],
        num_pbdmas: 2,
    },
    FifoDeviceEntry {
        name: "CE0",
        engine_data: [
            0x793ceb00, // [ 0] ENG_DESC
            0x00000001, // [ 1] FIFO_TAG
            0x00000009, // [ 2] RM_ENGINE_TYPE
            0x00000000, // [ 3] RUNLIST
            0x0000000f, // [ 4] MMU_FAULT_ID
            0x00000017, // [ 5] RC_MASK
            0x00000002, // [ 6] RESET
            0x00000000, // [ 7] INTR
            0x0000000f, // [ 8] MC
            0x00000013, // [ 9] DEV_TYPE_ENUM
            0x00000000, // [10] INSTANCE_ID
            0x00c00000, // [11] RUNLIST_PRI_BASE
            0x00000001, // [12] IS_HOST_DRIVEN_ENGINE
            0x00000001, // [13] RUNLIST_ENGINE_ID
            0x00c20000, // [14] CHRAM_PRI_BASE
            0x00000000, // [15] KERNEL_RM_MAX
        ],
        pbdma_ids: [0x00000000, 0x00000001],
        pbdma_fault_ids: [0x00000020, 0x00000021],
        num_pbdmas: 1,
    },
    FifoDeviceEntry {
        name: "CE1",
        engine_data: [
            0x793ceb01, // [ 0] ENG_DESC
            0x00000002, // [ 1] FIFO_TAG
            0x0000000a, // [ 2] RM_ENGINE_TYPE
            0x00000000, // [ 3] RUNLIST
            0x00000010, // [ 4] MMU_FAULT_ID
            0x00000018, // [ 5] RC_MASK
            0x00000003, // [ 6] RESET
            0x82300100, // [ 7] INTR
            0x00000010, // [ 8] MC
            0x00000013, // [ 9] DEV_TYPE_ENUM
            0x00000001, // [10] INSTANCE_ID
            0x00c00000, // [11] RUNLIST_PRI_BASE
            0x00000001, // [12] IS_HOST_DRIVEN_ENGINE
            0x00000002, // [13] RUNLIST_ENGINE_ID
            0x00c20000, // [14] CHRAM_PRI_BASE
            0x00000000, // [15] KERNEL_RM_MAX
        ],
        pbdma_ids: [0x00000001, 0x00000001],
        pbdma_fault_ids: [0x00000020, 0x00000021],
        num_pbdmas: 1,
    },
    FifoDeviceEntry {
        name: "CE2",
        engine_data: [
            0x793ceb02, // [ 0] ENG_DESC
            0x00000003, // [ 1] FIFO_TAG
            0x0000000b, // [ 2] RM_ENGINE_TYPE
            0x00000001, // [ 3] RUNLIST
            0x00000011, // [ 4] MMU_FAULT_ID
            0x00000019, // [ 5] RC_MASK
            0x00000004, // [ 6] RESET
            0x77f2058f, // [ 7] INTR
            0x00000011, // [ 8] MC
            0x00000013, // [ 9] DEV_TYPE_ENUM
            0x00000002, // [10] INSTANCE_ID
            0x00c00400, // [11] RUNLIST_PRI_BASE
            0x00000001, // [12] IS_HOST_DRIVEN_ENGINE
            0x00000000, // [13] RUNLIST_ENGINE_ID
            0x00c22000, // [14] CHRAM_PRI_BASE
            0x00000000, // [15] KERNEL_RM_MAX
        ],
        pbdma_ids: [0x00000005, 0x00000001],
        pbdma_fault_ids: [0x00000022, 0x00000021],
        num_pbdmas: 1,
    },
    FifoDeviceEntry {
        name: "CE3",
        engine_data: [
            0x793ceb03, // [ 0] ENG_DESC
            0x00000004, // [ 1] FIFO_TAG
            0x0000000c, // [ 2] RM_ENGINE_TYPE
            0x00000002, // [ 3] RUNLIST
            0x00000012, // [ 4] MMU_FAULT_ID
            0x0000001a, // [ 5] RC_MASK
            0x00000005, // [ 6] RESET
            0x018e0102, // [ 7] INTR
            0x00000012, // [ 8] MC
            0x00000013, // [ 9] DEV_TYPE_ENUM
            0x00000003, // [10] INSTANCE_ID
            0x00c00800, // [11] RUNLIST_PRI_BASE
            0x00000001, // [12] IS_HOST_DRIVEN_ENGINE
            0x00000000, // [13] RUNLIST_ENGINE_ID
            0x00c24000, // [14] CHRAM_PRI_BASE
            0x00000000, // [15] KERNEL_RM_MAX
        ],
        pbdma_ids: [0x00000006, 0x00000001],
        pbdma_fault_ids: [0x00000023, 0x00000021],
        num_pbdmas: 1,
    },
    FifoDeviceEntry {
        name: "SOFTWARE",
        engine_data: [
            0x95a6f500, // [ 0] ENG_DESC
            0xffffffff, // [ 1] FIFO_TAG
            0x0000002d, // [ 2] RM_ENGINE_TYPE
            0x00000007, // [ 3] RUNLIST
            0xffffffff, // [ 4] MMU_FAULT_ID
            0x00000000, // [ 5] RC_MASK
            0x82300810, // [ 6] RESET
            0x77f2058f, // [ 7] INTR
            0x018e000f, // [ 8] MC
            0x00000000, // [ 9] DEV_TYPE_ENUM
            0x82300100, // [10] INSTANCE_ID
            0x77f2058f, // [11] RUNLIST_PRI_BASE
            0x00000000, // [12] IS_HOST_DRIVEN_ENGINE
            0x00000000, // [13] RUNLIST_ENGINE_ID
            0x82300710, // [14] CHRAM_PRI_BASE
            0x00000000, // [15] KERNEL_RM_MAX
        ],
        pbdma_ids: [0x00000008, 0x00000001],
        pbdma_fault_ids: [0x82300100, 0x77f2058f],
        num_pbdmas: 1,
    },
];

pub static INTR_TABLE: &[IntrTableEntry] = &[
    IntrTableEntry {
        engine_idx: 59,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0x00000040,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 62,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0x00000083,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 60,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0x00000048,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 73,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0x00000081,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 156,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 157,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 158,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 159,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 160,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 161,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 162,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 163,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 50,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0x0000009b,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 2,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0x0000009a,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 84,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000000,
    },
    IntrTableEntry {
        engine_idx: 47,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000002,
    },
    IntrTableEntry {
        engine_idx: 38,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000001,
    },
    IntrTableEntry {
        engine_idx: 65,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000003,
    },
    IntrTableEntry {
        engine_idx: 15,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 16,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0xffffffff,
    },
    IntrTableEntry {
        engine_idx: 17,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000007,
    },
    IntrTableEntry {
        engine_idx: 18,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000008,
    },
    IntrTableEntry {
        engine_idx: 19,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x0000000a,
    },
    IntrTableEntry {
        engine_idx: 81,
        pmc_intr_mask: 0x00000000,
        vector_stall: 0xffffffff,
        vector_non_stall: 0x00000009,
    },
];

/// Old `ga10x::GA106_INTR_SUBTREE_MAP`.
pub static INTR_SUBTREE_MAP: [u64; INTR_CATEGORY_COUNT] = [0x0, 0x8, 0x1, 0x0, 0x0, 0x2, 0x4];

static REG_BASES: &[RegBaseRow] = &[RegBaseRow {
    index: reg_base::USERMODE,
    offset: VIRTUAL_FUNCTION_BASE,
    name: "NV_VIRTUAL_FUNCTION (usermode work submission)",
}];

/// Old `ga10x::GA106_MEMORY_SYSTEM`.
pub const MEMORY_SYSTEM: MemorySystemRow = MemorySystemRow {
    comptag_policy: ComptagAllocationPolicy::Raw,
    disable_compbit_backing: false,
    disable_post_l2_compression: false,
    ecc_fbpa_enabled: false,
    l2_prefill: false,
    l2_cache_size: 0x0024_0000,
    fbpa_present: true,
    compr_page_size: 0x0001_0000,
    ram_type: RAM_TYPE_GDDR6,
    ltc_count: 6,
    lts_per_ltc_count: 4,
};

/// Every old `GA106` chip-row fact, as the v3 [`HostFacts`].
pub fn host_facts() -> HostFacts {
    HostFacts {
        family: kf_chip::Family::Ampere,
        has_c2c: false,
        ce_caps: ce_caps(),
        engines: ENGINES.to_vec(),
        lce_pce_masks: kf_abi::cepce::GA106_LCE_PCE_MASKS.to_vec(),
        intr_table: INTR_TABLE.to_vec(),
        intr_subtree_map: INTR_SUBTREE_MAP,
        chip_info: ChipInfoRow { chip_sub_rev: 0, is_cmp_sku: false, reg_bases: REG_BASES },
        user_register_access_map: RegisterAccessMapRow::NOT_PUBLISHED,
        constructed_falcons: FalconInventoryRow::NONE,
        memory_system: MEMORY_SYSTEM,
        device_info: DeviceInfoRow {
            pri_bases: &[
                EnginePriBase { engine: "GR0", pri_base: DevicePriBase::At(0x0040_0000) },
                EnginePriBase { engine: "CE0", pri_base: DevicePriBase::At(0x0010_4000) },
                EnginePriBase { engine: "CE1", pri_base: DevicePriBase::At(0x0010_4000) },
                EnginePriBase { engine: "CE2", pri_base: DevicePriBase::At(0x0010_4000) },
                EnginePriBase { engine: "CE3", pri_base: DevicePriBase::At(0x0010_4000) },
                EnginePriBase { engine: "SOFTWARE", pri_base: DevicePriBase::NotADevice },
            ],
        },
        conf_compute: ConfComputeRow { bar1_trusted: false, pcie_trusted: false },
        bif_static: BifStaticRow {
            pcie_gen4_capable: false,
            c2c_link_up: false,
            device_multi_function: false,
            gcx_pmu_cfg_space_restore: false,
        },
        fifo_channels: FifoChannelsRow { channels_per_runlist: 0x0800 },
        gmmu_static: GmmuStaticRow {
            replayable_size: 0x0003_1000,
            replayable_shadow_metadata_size: 0,
            non_replayable_size: 0x0012_0c20,
            non_replayable_shadow_metadata_size: 0,
        },
        gr_static: kf_abi::grstatic::GA106_GR_STATIC,
        gr_info: kf_abi::grinfo::GA106_GR_INFO,
        gr_context_buffers: kf_abi::grstatic::GA106_CONTEXT_BUFFERS,
        // ⊘ unmeasured on GA106 so far: None (refused, as before v3-gfx) until a host reply is captured.
        gr_zcull_info: None,
        // ⊘ v3-gfx fields: unmeasured on GA106 — the refusing values (as before v3-gfx).
        zbc_table_sizes: None,
        forwarded_fb_extra: vec![],
        gpu_cache_info: None,
        forwarded_gpu_info: kf_abi::gpuinfo::GA106_FORWARDED_GPU_INFO.to_vec(),
        // The GA106's measured words (`kf_abi::fbinfo` tests: bus 0xc0, FBPs 3, LTS 18), which
        // the GA10x projections of its row reproduce.
        forwarded_fb_info: kf_abi::fbinfo::FbGeometry {
            l2_cache_size: MEMORY_SYSTEM.l2_cache_size,
            ram_type: MEMORY_SYSTEM.ram_type,
            ltc_count: MEMORY_SYSTEM.ltc_count,
        }
        .forwarded_answers()
        .expect("the GA106 row projects")
        .to_vec(),
        smc_mode: kf_abi::smcmode::GA106_SMC_MODE,
        pcie_max_gen: kf_abi::businfo::PcieGen::Gen4,
        ce_fault_method_buffer_size: kf_abi::fmbsize::GA106_CE_FAULT_METHOD_BUFFER_SIZE,
        gsp_features: kf_abi::gspfeatures::GspFeatures::GA106,
        // ⊘ The old row served NO name (the `nvidia-smi` `ERR!` defect); the fixture stays
        // faithful to what the old device answered. `derive_gpu_name` is checked separately.
        gpu_name: None,
        gpu_short_name: None,
        // The old row's ROM version (`kf_abi::vbios::VBIOS_PROFILES[0]`) — ⊘ not a measured
        // board version; the v3 device asks the host (`BIOS_GET_INFO_V2`).
        vbios_version: (0x9418_0000, 0x00),
        perf_level_info_v2: Some(perf_level_info_v2()),
    }
}

/// `[measured 2026-08-09, real GA106]` `CE_GET_ALL_CAPS` (`0x20802a0a`), R18 and `cuInit` line 62:
/// `e303e303e203e203`, 120 zero bytes, `present = 0x0f`.
pub fn ce_caps_reply() -> Vec<u8> {
    let mut v = vec![0xe3, 0x03, 0xe3, 0x03, 0xe2, 0x03, 0xe2, 0x03];
    v.resize(kf_abi::cecaps::PRESENT_OFF, 0);
    v.extend_from_slice(&0x0f_u64.to_le_bytes());
    v
}

/// [`ce_caps_reply`], decoded.
pub fn ce_caps() -> kf_abi::cecaps::HostCeCaps {
    kf_abi::cecaps::HostCeCaps::decode(&ce_caps_reply()).expect("136 bytes")
}

/// `[measured 2026-08-20, real GA106]` the host's `PERF_GET_LEVEL_INFO_V2` reply to libcudart's
/// question: the request with the nine `kf_abi::cudartinit::SPLICED` words written.
pub fn perf_level_info_v2() -> Vec<u8> {
    let mut r = kf_abi::cudartinit::perf_level_info_v2_request();
    assert!(kf_abi::cudartinit::splice_cudart_init(kf_abi::cudartinit::PERF_GET_LEVEL_INFO_V2, &mut r));
    r
}

/// [`host_facts`], shared.
pub fn host() -> Arc<HostFacts> {
    Arc::new(host_facts())
}
