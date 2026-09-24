//! ★★★★★ **What WE state as the GSP of the device we present** — the physical-RM answers a
//! usermode host session cannot reach, authored per family.
//!
//! Ruling (coordinator, w827): *"WE ARE THE GSP, so physical-RM answers a usermode control
//! cannot reach are ours to AUTHOR, per FAMILY, from ogkm's own physical-RM source, with
//! `Ogkm(file:line)` or `Advertised(why)` provenance — never a GA106 capture in production code;
//! the capture stays the test oracle."*
//!
//! Every value here carries its [`Why`]. Two kinds, and the difference is the point:
//!
//! - [`Why::Ogkm`] — a header constant or a code path in `ogkm-580` (or nouveau, where the
//!   open driver's header omits the family's value) that STATES the number.
//! - [`Why::Advertised`] — a number the open tree does NOT state (the physical-RM body is GSP
//!   firmware, or the value is a silicon reset default), chosen by us because the device we
//!   present is ours and the stated reason says why the choice is safe. ⚠ Where an Advertised
//!   value coincides with what a real GA106 answered, the doc says so plainly: that is a
//!   choice made so the GA106 differential stays byte-identical, not a measurement of any
//!   other die.
//!
//! ⊘ A family for which a rule needs a number no source states is refused BY NAME, for that
//! family only ([`LayoutRefusal`]).

use kf_abi::gmmustatic::GmmuStaticRow;
use kf_abi::inittables::{
    ENGINE_DATA_TYPES, ENGINE_MAX_PBDMA, FifoDeviceEntry, INTR_VECTOR_INVALID, IntrTableEntry,
};
use kf_chip::Family;

/// Why a value is what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Stated by ogkm-580 (or nouveau) at `file:line`.
    Ogkm(&'static str),
    /// Not stated anywhere in the open tree; ours to advertise, for the stated reason.
    Advertised(&'static str),
}

/// A family whose rule needs a number no source states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutRefusal {
    /// The family.
    pub family: Family,
    /// What is missing, by name.
    pub missing: &'static str,
}

// =====================================================================================
// CE fault method buffer
// =====================================================================================

/// ★ `CE_GET_FAULT_METHOD_BUFFER_SIZE` — **Advertised**, 20 KiB, every family.
pub const CE_FAULT_METHOD_BUFFER_SIZE: u32 = 0x5000;

/// Why [`CE_FAULT_METHOD_BUFFER_SIZE`] is ours to state and why it is safe.
pub const CE_FAULT_METHOD_BUFFER_SIZE_WHY: Why = Why::Advertised(
    "no formula exists in the open tree: subdeviceCtrlCmdCeGetFaultMethodBufferSize_IMPL is GSP firmware \
     (declared g_subdevice_nvoc.h:5094, defined nowhere; kceGetFaultMethodBufferSize_IMPL kernel_ce.c:827-848 only \
     forwards). The guest allocates one buffer of this size per runqueue per channel group \
     (kernel_channel_group_gv100.c:77-110) and promotes it to us; in this design a guest channel runs on a HOST \
     twin whose host-RM-owned method buffer is the one hardware writes, so the guest's buffer is never a DMA \
     target and only needs to be a non-zero page multiple RM can allocate. 20 KiB = 5 pages is chosen equal to \
     what a stock GA106 GSP states, so the reserved-FB arithmetic (kfifoCalcTotalSizeOfFaultMethodBuffers_GV100) \
     matches the measured driver.",
);

// =====================================================================================
// GMMU fault buffers
// =====================================================================================

/// ★ The GMMU fault-buffer sizes this device advertises — **Advertised**, every family.
pub const GMMU_STATIC: GmmuStaticRow = GmmuStaticRow {
    replayable_size: 0x0003_1000,
    replayable_shadow_metadata_size: 0,
    non_replayable_size: 0x0012_0c20,
    non_replayable_shadow_metadata_size: 0,
};

/// Why [`GMMU_STATIC`] is ours to state.
pub const GMMU_STATIC_WHY: Why = Why::Advertised(
    "the physical computation IS in the open tree and it is not a formula: \
     kgmmuSetAndGetDefaultFaultBufferSize_TU102 (kern_gmmu_tu102.c:548-566, the only non-stub HAL) writes \
     NV_VIRTUAL_FUNCTION_PRIV_MMU_FAULT_BUFFER_SIZE_SET_DEFAULT_YES and reads the silicon's reset entry count \
     back (* NVC369_BUF_SIZE = 32); nouveau does the same (fault/gv100.c:112-114). No header or source states the \
     default. We are the MMU the guest sees: these are the sizes of OUR buffers, both multiples of 32, and \
     overflow is RM's own overflow path, never a corrupt write. Chosen equal to what a stock GA106 GSP states \
     (6272 and 36961 packets); ⚠ not a measurement of any other die.",
);

// =====================================================================================
// GR
// =====================================================================================

/// ★ `fecsRecordSize` — **Advertised**, 128 bytes, every family.
pub const FECS_RECORD_SIZE: u32 = 128;

/// Why [`FECS_RECORD_SIZE`].
pub const FECS_RECORD_SIZE_WHY: Why = Why::Advertised(
    "the FECS trace defines come from physical RM (kernel_graphics.c:1477 only copies them). The record the \
     kernel reads is FECS_EVENT_RECORD, 120 bytes (fecs_event_list.c:58-69), and RM strides the buffer by \
     fecsRecordSize (:791), so the size must be >= 120; 128 is the smallest power of two that holds it. In this \
     design GR contexts run on host twins, so no FECS writes the guest's trace buffer.",
);

/// ★ `tpcToPesMap` — **Ogkm-rule over host litters**: physical TPC `i` hangs off PES
/// `i / LITTER_NUM_TPCS_PER_PES`, for `i < LITTER_NUM_TPC_PER_GPC`; zero beyond. Both litters
/// come from the host's own `GR_GET_INFO_V2`.
///
/// ⚠ The contiguous distribution is the rule; ogkm's physical RM does not state it (the map
/// arrives from GSP, `rpcstructurecopy.c:1020`). It reproduces the real GA106's map exactly
/// (`kf_abi::grstatic::GA106_TPC_TO_PES_MAP`), which is the test.
#[must_use]
pub fn tpc_to_pes_map(litter_tpc_per_gpc: u32, tpcs_per_pes: u32) -> Option<[u32; kf_abi::grstatic::MAX_TPC_PER_GPC]> {
    let mut map = [0u32; kf_abi::grstatic::MAX_TPC_PER_GPC];
    if tpcs_per_pes == 0 || litter_tpc_per_gpc as usize > map.len() {
        return None;
    }
    for (i, m) in map.iter_mut().enumerate().take(litter_tpc_per_gpc as usize) {
        *m = i as u32 / tpcs_per_pes;
    }
    Some(map)
}

/// `bPerSubCtxheaderSupported` — every family served is Volta or later, where the
/// per-subcontext header exists (`kernel_graphics.c:1518` consumes it; GA106 answers 1).
pub const PER_SUBCTX_HEADER_SUPPORTED: bool = true;

// =====================================================================================
// Interrupt rows of the device we present
// =====================================================================================

/// `MC_ENGINE_IDX_GSP` (`ogkm-580: engine_idx.h:90`).
pub const MC_ENGINE_IDX_GSP: u16 = 50;
/// `MC_ENGINE_IDX_DISP` (`ogkm-580: engine_idx.h:41`).
pub const MC_ENGINE_IDX_DISP: u16 = 2;
/// ★ The interrupt-tree vector OUR GSP raises its message-queue interrupt on.
pub const GSP_STALL_VECTOR: u32 = 0x9b;
/// ★ The interrupt-tree vector OUR display stall interrupt is published at.
pub const DISP_STALL_VECTOR: u32 = 0x9a;

/// Why [`GSP_STALL_VECTOR`] / [`DISP_STALL_VECTOR`].
pub const GSP_DISP_VECTORS_WHY: Why = Why::Advertised(
    "the GSP and DISP stall rows of INTERNAL_INTR_GET_KERNEL_TABLE are physical-RM facts no usermode control \
     reports (MC_GET_STATIC_INTR_TABLE carries only NV2080_INTR_TYPE rows). The GSP's vector is where the \
     guest's ISR looks for our message-queue interrupt (kernel_gsp.c:5472-5496; intr_tu102.c:284 for DISP), so \
     it is a fact about OUR device: the interrupt plane reads it back out of this table and never restates it. \
     0x9b / 0x9a are chosen equal to a stock GA106's; both lie in the leaf range every family's tree has \
     (NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF_TRIGGER_VECTOR is 11:0, dev_vm.h). Refused if the host's own \
     rows already use either vector.",
);

/// ★ Append the GSP and DISP stall rows to a host-derived table.
///
/// # Errors
/// The colliding vector, when a host row already publishes it — two engines on one vector
/// would make the guest's ISR demultiplex wrongly.
pub fn with_gsp_and_disp_rows(mut table: Vec<IntrTableEntry>) -> Result<Vec<IntrTableEntry>, u32> {
    for v in [GSP_STALL_VECTOR, DISP_STALL_VECTOR] {
        if table.iter().any(|e| e.vector_stall == v || e.vector_non_stall == v) {
            return Err(v);
        }
    }
    for (engine_idx, v) in [(MC_ENGINE_IDX_GSP, GSP_STALL_VECTOR), (MC_ENGINE_IDX_DISP, DISP_STALL_VECTOR)] {
        table.push(IntrTableEntry { engine_idx, pmc_intr_mask: 0, vector_stall: v, vector_non_stall: INTR_VECTOR_INVALID });
    }
    Ok(table)
}

/// ★ The engine NON-STALL rows of the device we present — one per runlist that owns a notifier.
///
/// ⊘ Not the host's: `MC_GET_ENGINE_NOTIFICATION_INTR_VECTORS` (`0x2080170d`) answers
/// `NV_ERR_NOT_SUPPORTED` to a usermode client (measured `f4b78ed9`, GA106, 580.159.04), and its
/// vectors describe the host die's runlists, which the guest never sees — it sees
/// [`engine_table`]'s. On Ampere+ a non-stall vector is the engine's interrupt ID in the
/// runlist/device-info order (the GA106 capture: GR=0, then one per further unit), and every
/// guest interrupt is injected BY US from a host NSI event, so the vector is ours to choose.
///
/// Rule: GR0 → vector 0 (its runlist-0 graphics CEs notify through it and get no row, as in the
/// capture); each async CE → its runlist number; the SW pseudo-engine → none. Distinct by
/// construction and below the 12-bit leaf-vector limit.
#[must_use]
pub fn engine_notification_rows(engines: &[EngineKind]) -> Vec<IntrTableEntry> {
    let grce = |i: u32| i < 64 && kf_abi::cecaps::GA10X_GRCE_LCE_MASK & (1u64 << i) != 0;
    let mut next_runlist = 1u32;
    let mut out = Vec::new();
    for &kind in engines {
        let row = |engine_idx: u32, v: u32| IntrTableEntry {
            engine_idx: engine_idx as u16,
            pmc_intr_mask: 0,
            vector_stall: INTR_VECTOR_INVALID,
            vector_non_stall: v,
        };
        match kind {
            EngineKind::Graphics(i) => out.push(row(MC_GR0 + i, 0)),
            EngineKind::Copy(i) if grce(i) => {}
            EngineKind::Copy(i) => {
                out.push(row(MC_CE0 + i, next_runlist));
                next_runlist += 1;
            }
            EngineKind::Software => {}
        }
    }
    out
}

/// ★ P5b §2.7 — **the guest vector a HOST engine's non-stall completion is announced on**, read
/// back out of the table the guest was served (`intr_table`), never restated: `GR<i>` → the
/// `MC_ENGINE_IDX_GR<i>` row, async `CE<i>` → the `MC_ENGINE_IDX_CE<i>` row, and a GRCE — which
/// [`engine_notification_rows`] gives no row because it notifies through its GR — → `GR0`'s.
/// `None` when the table carries no non-stall vector for it (then nothing is raised, by name).
#[must_use]
pub fn non_stall_vector_for(table: &[IntrTableEntry], engine: EngineKind) -> Option<u32> {
    let row = |idx: u32| {
        table
            .iter()
            .find(|e| u32::from(e.engine_idx) == idx && e.vector_non_stall != INTR_VECTOR_INVALID)
            .map(|e| e.vector_non_stall)
    };
    match engine {
        EngineKind::Graphics(i) => row(MC_GR0 + i),
        EngineKind::Copy(i) => row(MC_CE0 + i).or_else(|| row(MC_GR0)),
        EngineKind::Software => None,
    }
}

// =====================================================================================
// The FIFO engine table of the device we present
// =====================================================================================

/// An engine the device presents, from the host's `GET_ENGINES_V2` list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    /// `GRn`.
    Graphics(u32),
    /// `CEn` (an LCE).
    Copy(u32),
    /// RM's software pseudo-engine.
    Software,
}

impl EngineKind {
    /// `FifoDeviceEntry::name` — `GR0`, `CE3`, `SOFTWARE`.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            EngineKind::Graphics(i) => format!("GR{i}"),
            EngineKind::Copy(i) => format!("CE{i}"),
            EngineKind::Software => "SOFTWARE".to_string(),
        }
    }
}

/// `ENGINE_INFO_TYPE_*` slot indices (`ogkm-580: inc/kernel/gpu/fifo/engine_info.h:37-104`).
pub mod slot {
    /// `ENG_DESC`.
    pub const ENG_DESC: usize = 0;
    /// `FIFO_TAG`.
    pub const FIFO_TAG: usize = 1;
    /// `RM_ENGINE_TYPE`.
    pub const RM_ENGINE_TYPE: usize = 2;
    /// `RUNLIST`.
    pub const RUNLIST: usize = 3;
    /// `MMU_FAULT_ID`.
    pub const MMU_FAULT_ID: usize = 4;
    /// `RC_MASK`.
    pub const RC_MASK: usize = 5;
    /// `RESET`.
    pub const RESET: usize = 6;
    /// `INTR`.
    pub const INTR: usize = 7;
    /// `MC`.
    pub const MC: usize = 8;
    /// `DEV_TYPE_ENUM`.
    pub const DEV_TYPE_ENUM: usize = 9;
    /// `INSTANCE_ID`.
    pub const INSTANCE_ID: usize = 10;
    /// `RUNLIST_PRI_BASE`.
    pub const RUNLIST_PRI_BASE: usize = 11;
    /// `IS_HOST_DRIVEN_ENGINE`.
    pub const IS_HOST_DRIVEN_ENGINE: usize = 12;
    /// `RUNLIST_ENGINE_ID`.
    pub const RUNLIST_ENGINE_ID: usize = 13;
    /// `CHRAM_PRI_BASE`.
    pub const CHRAM_PRI_BASE: usize = 14;
}

/// NVOC class ids `MKENGDESC` packs (`ENG_DESC = classId << 8 | instance`) —
/// `ogkm-580: generated/g_eng_desc_nvoc.h:287` (`Graphics 0xd334df`), `:1247` (`OBJCE
/// 0x793ceb`), `:92` (`OBJSWENG 0x95a6f5`). Driver-version constants, family-invariant.
const CLASS_GRAPHICS: u32 = 0x00d3_34df;
const CLASS_OBJCE: u32 = 0x0079_3ceb;
const CLASS_OBJSWENG: u32 = 0x0095_a6f5;
/// `NV_PTOP_DEVICE_INFO2_DEV_TYPE_ENUM_GRAPHICS` / `_LCE` (`ampere/ga100/dev_top.h:34`,
/// `blackwell/gb100/dev_top.h:42`).
const DEV_TYPE_GRAPHICS: u32 = 0;
const DEV_TYPE_LCE: u32 = 0x13;
/// `MC_ENGINE_IDX_GR0` / `_CE0` (`engine_idx.h:54,128`).
const MC_GR0: u32 = 84;
const MC_CE0: u32 = 15;

/// The MMU fault-engine ids of a family: `(GRAPHICS, CE0, HOST0)`.
///
/// - Turing / Ampere / Ada: `GRAPHICS = 64` (`volta/gv100/dev_fault.h:26`, returned by
///   `kern_gmmu_gv100.c:522`), `CE0 = 15` (`turing/tu102`, `ampere/ga100`, `ada/ad102`
///   `dev_fault.h`), `HOST0 = 0x20` (nouveau `engine/fifo/tu102.c:106`, `gv100.c:413`; ga100/
///   ga102 use the tu102 table).
/// - Blackwell: `384 / 65 / 85` (`blackwell/gb100` and `gb202` `dev_fault.h:28,67/74,95/102`).
/// - Hopper: `GRAPHICS 384`, `CE0 43` (`hopper/gh100/dev_fault.h:27,39`) — ⊘ and NO `HOST0`
///   anywhere in the tree (`gh100/dev_fault.h` lists none; nouveau has no Hopper FIFO), so the
///   PBDMA fault ids cannot be stated: refused for Hopper, by name.
fn fault_ids(family: Family) -> Result<(u32, u32, u32), LayoutRefusal> {
    match family {
        Family::Turing | Family::Ampere | Family::Ada => Ok((64, 15, 0x20)),
        Family::Blackwell => Ok((384, 65, 85)),
        Family::Hopper => Err(LayoutRefusal {
            family,
            missing: "NV_PFAULT_MMU_ENG_ID_HOST0 (PBDMA fault ids): hopper/gh100/dev_fault.h states GRAPHICS 384 and \
                      CE0 43 but no HOST0, and nouveau has no Hopper FIFO",
        }),
    }
}

/// Why the engine table's layout is what it is.
pub const ENGINE_LAYOUT_WHY: Why = Why::Advertised(
    "runlist, PBDMA, reset, RC and CHRAM slots describe OUR device's topology, not the host's: guest channels \
     are re-born on host twins and guest tokens are translated. Layout: GR0 and the GRCEs (kf_abi::cecaps::\
     GA10X_GRCE_LCE_MASK, the set the served CE caps advertise) share runlist 0 with PBDMAs 0 and 1; every other \
     LCE owns runlist 1, 2, ... with PBDMA runlist+1; RUNLIST_PRI_BASE = 0xC00000 + 0x400*runlist and \
     CHRAM_PRI_BASE = 0xC20000 + 0x2000*runlist on Ampere+ (engine_info.h:66-90: 'valid only on Ampere+', so 0 \
     on Turing; no kernel-RM reader outside kfifoEngineInfoXlate); RESET bits 12 (GR) and 2+i / 3+i (CEi, i<10 / \
     i>=10) in our NV_PMC_DEVICE_ENABLE (sole reader kbifGetValidDeviceEnginesToReset_GA100, \
     kernel_bif_ga100.c:572-605); INTR 0 (kernel_fifo_ga100.c:52 'no longer stored on Ampere+', no reader on \
     Turing either); RC_MASK 0 (no kernel-RM reader at all). ENG_DESC, DEV_TYPE_ENUM, MC and the MMU fault ids \
     are ogkm constants (cited in the code).",
);

/// ★★ The FIFO device-info table of the device we present, over the host's engine list.
///
/// `engines` is the advertised list in host order (GR, CEs, SW). Leaks each name once
/// (`FifoDeviceEntry::name` is `&'static str`) — call at realize, once per device.
///
/// # Errors
/// [`LayoutRefusal`] for Hopper (no `HOST0`), or a list with a second GR (MIG, whose GR
/// runlists this layout does not state).
pub fn engine_table(family: Family, engines: &[EngineKind]) -> Result<Vec<FifoDeviceEntry>, LayoutRefusal> {
    let (gr_fault, ce0_fault, host0) = fault_ids(family)?;
    if engines.iter().any(|k| matches!(k, EngineKind::Graphics(i) if *i > 0)) {
        return Err(LayoutRefusal { family, missing: "runlists for GR1..GR7 (MIG): this layout states GR0 only" });
    }
    let esched = !matches!(family, Family::Turing);
    let grce = |i: u32| i < 64 && kf_abi::cecaps::GA10X_GRCE_LCE_MASK & (1u64 << i) != 0;
    let mut next_runlist = 1u32;
    let mut next_tag = 0u32;
    let mut grce_seen = 0u32;
    let mut out = Vec::with_capacity(engines.len());
    for &kind in engines {
        let mut d = [0u32; ENGINE_DATA_TYPES];
        let mut pbdma_ids = [0u32; ENGINE_MAX_PBDMA];
        let mut pbdma_fault_ids = [0u32; ENGINE_MAX_PBDMA];
        let mut num_pbdmas = 0u32;
        let (runlist, rl_engine) = match kind {
            EngineKind::Graphics(_) => (Some(0), 0),
            EngineKind::Copy(i) if grce(i) => {
                grce_seen += 1;
                (Some(0), grce_seen)
            }
            EngineKind::Copy(_) => {
                let r = next_runlist;
                next_runlist += 1;
                (Some(r), 0)
            }
            EngineKind::Software => (None, 0),
        };
        match kind {
            EngineKind::Graphics(i) => {
                d[slot::ENG_DESC] = CLASS_GRAPHICS << 8 | i;
                d[slot::MMU_FAULT_ID] = gr_fault;
                d[slot::RESET] = 12;
                d[slot::MC] = MC_GR0 + i;
                d[slot::DEV_TYPE_ENUM] = DEV_TYPE_GRAPHICS;
                d[slot::INSTANCE_ID] = i;
                pbdma_ids = [0, 1];
                num_pbdmas = 2;
            }
            EngineKind::Copy(i) => {
                d[slot::ENG_DESC] = CLASS_OBJCE << 8 | i;
                d[slot::MMU_FAULT_ID] = ce0_fault + i;
                d[slot::RESET] = if i < 10 { 2 + i } else { 3 + i };
                d[slot::MC] = MC_CE0 + i;
                d[slot::DEV_TYPE_ENUM] = DEV_TYPE_LCE;
                d[slot::INSTANCE_ID] = i;
                pbdma_ids[0] = if runlist == Some(0) { rl_engine - 1 } else { runlist.unwrap_or(0) + 1 };
                num_pbdmas = 1;
            }
            EngineKind::Software => {
                d[slot::ENG_DESC] = CLASS_OBJSWENG << 8;
                d[slot::FIFO_TAG] = u32::MAX;
                d[slot::MMU_FAULT_ID] = u32::MAX;
            }
        }
        if kind != EngineKind::Software {
            d[slot::FIFO_TAG] = next_tag;
            next_tag += 1;
            d[slot::IS_HOST_DRIVEN_ENGINE] = 1;
            let r = runlist.unwrap_or(0);
            d[slot::RUNLIST] = r;
            if esched {
                d[slot::RUNLIST_PRI_BASE] = 0x00C0_0000 + 0x400 * r;
                d[slot::CHRAM_PRI_BASE] = 0x00C2_0000 + 0x2000 * r;
                d[slot::RUNLIST_ENGINE_ID] = rl_engine;
            }
            for k in 0..num_pbdmas as usize {
                pbdma_fault_ids[k] = host0 + pbdma_ids[k];
            }
        }
        d[slot::RM_ENGINE_TYPE] = match kind {
            EngineKind::Graphics(i) => 1 + i,
            EngineKind::Copy(i) => kf_abi::submit::RM_ENGINE_TYPE_COPY0 + i,
            EngineKind::Software => kf_abi::submit::RM_ENGINE_TYPE_SW,
        };
        out.push(FifoDeviceEntry {
            name: Box::leak(kind.name().into_boxed_str()),
            engine_data: d,
            pbdma_ids,
            pbdma_fault_ids,
            num_pbdmas,
        });
    }
    Ok(out)
}
