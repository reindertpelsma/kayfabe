//! ★★★★★ **`HostFacts` — the per-die facts the served controls answer from.**
//!
//! The old tree answered the ~40 boot `RM_CONTROL`s from a `&'static ChipProfile`: one
//! hand-maintained row per die (`kayfabe-device/src/ga10x.rs`, GA106 only). v3 has no per-die
//! rows (`V3_BUILD.md`, "Per-die tables → HOST-QUERY"). The ENCODING code is unchanged
//! ([`crate::inittables`] still calls the same `kf_abi` encoders); only the SOURCE of the
//! numbers moved, into this struct, which the composition root fills at realize from the host
//! GPU through **unprivileged** RM controls, from the family axis ([`kf_chip::Family`]), or by
//! authoring a value RM's own code accepts.
//!
//! ⊘ kf-rm does not talk to the host. kf-rm does not depend on `kf-host` (host calls go through
//! traits a later crate implements), so the work is split in three:
//!
//! - **this module** — the struct, its provenance table, and the pure **derivations** from host
//!   reply bytes (`derive_*`), each checkable against the old tree's captured GA106 rows (the
//!   port map's rule: *"on a GA106 host, derived must equal captured"* —
//!   `tests/host_facts_ga106.rs`, fed real-GA106 reply bytes from `traces/real_ga106/`);
//! - [`crate::hostquery`] — WHICH controls are issued, with which request bytes, over the
//!   [`crate::hostquery::HostControls`] seam, plus the family rules and authored values;
//! - the composition root (`kf-qemu`'s `rmfacts`) — implements the seam over the real host
//!   session. That file is the only one that touches a GPU.
//!
//! # Provenance, per field
//!
//! [`PROVENANCE`] states, for every field, where the value comes from. `tests/host_facts_ga106.rs`
//! destructures [`HostFacts`] exhaustively and checks every field has exactly one row, so a
//! field added without a stated source is a compile error or a red test — never a silent
//! default.
//!
//! ⚠ Several `kf_abi` row types hold `&'static` slices (`FifoDeviceEntry::name`,
//! `DeviceInfoRow::pri_bases`, `GrStaticProfile::gpcs`, …) because they were built for
//! compile-time rows. A filler that builds them at realize leaks each once (`Box::leak`), as the
//! old `ga106_profile(fb_size_mb)` did — bounded, one per device.

use kf_abi::bifstatic::BifStaticRow;
use kf_abi::businfo::PcieGen;
use kf_abi::chipinfo::ChipInfoRow;
use kf_abi::confcompute::ConfComputeRow;
use kf_abi::deviceinfo::DeviceInfoRow;
use kf_abi::falconinfo::FalconInventoryRow;
use kf_abi::fifochannels::FifoChannelsRow;
use kf_abi::gmmustatic::GmmuStaticRow;
use kf_abi::grinfo::GrInfoProfile;
use kf_abi::grstatic::{CONTEXT_BUFFER_ID_COUNT, ContextBuffer, GrStaticProfile};
use kf_abi::gspfeatures::GspFeatures;
use kf_abi::gspstaticinfo::GpuName;
use kf_abi::inittables::{FifoDeviceEntry, INTR_CATEGORY_COUNT, IntrTableEntry};
use kf_abi::memsysconfig::MemorySystemRow;
use kf_abi::regaccessmap::RegisterAccessMapRow;
use kf_abi::smcmode::SmcMode;
use kf_chip::Family;

/// ★ The per-die facts of the HOST GPU the guest's board is presented over.
///
/// Every field's source is in [`PROVENANCE`]. ⊘ No `Default`: a fact nobody filled is a
/// refusal at realize, never a zero the guest caches.
#[derive(Debug, Clone)]
pub struct HostFacts {
    /// The GPU family (`MC_GET_ARCH_INFO` → [`Family::from_arch`]).
    pub family: Family,
    /// Whether a chip-to-chip link exists (`BUS_GET_C2C_INFO`).
    pub has_c2c: bool,
    /// ★ The host die's own copy-engine caps (`CE_GET_ALL_CAPS` `0x20802a0a`, NON_PRIVILEGED):
    /// the per-LCE caps the three CE-caps controls serve and the GRCE set the engine table lays
    /// out. ⊘ Replaces `GA10X_LCE_BASE_CAPS` / `GA10X_GRCE_LCE_MASK` (GB20x has four GRCEs).
    pub ce_caps: kf_abi::cecaps::HostCeCaps,
    /// The FIFO engine list (`GET_ENGINES_V2` + `GET_HW_ENGINE_ID` + family device-info rules).
    /// Served by `DeviceInfo`, `InternalDeviceInfo` and the three CE-caps controls.
    pub engines: Vec<FifoDeviceEntry>,
    /// LCE → PCE masks, one per present LCE (`CE_GET_CE_PCE_MASK`), in LCE order.
    pub lce_pce_masks: Vec<u32>,
    /// The kernel interrupt table (`MC_GET_STATIC_INTR_TABLE`).
    pub intr_table: Vec<IntrTableEntry>,
    /// The interrupt category → subtree map (`MC_GET_INTR_CATEGORY_SUBTREE_MAP`).
    pub intr_subtree_map: [u64; INTR_CATEGORY_COUNT],
    /// Chip sub-revision, CMP SKU flag, register bases (family rule; sub-rev from arch info).
    pub chip_info: ChipInfoRow,
    /// The user register access map (authored: `accessmap`).
    pub user_register_access_map: RegisterAccessMapRow,
    /// The falcons RM is told to construct (authored).
    pub constructed_falcons: FalconInventoryRow,
    /// The memory-system static config (`FB_GET_INFO_V2` for L2/RAM type/LTCs; the rest
    /// fabricated so ogkm accepts it).
    pub memory_system: MemorySystemRow,
    /// Per-engine PRI bases (family device-info rule).
    pub device_info: DeviceInfoRow,
    /// Confidential-compute static info (fabricated: CC off).
    pub conf_compute: ConfComputeRow,
    /// BIF static info (host bus info; fabricated where ogkm only needs acceptance).
    pub bif_static: BifStaticRow,
    /// Channels per runlist (ours to set).
    pub fifo_channels: FifoChannelsRow,
    /// GMMU fault-buffer sizes — ours to state ([`Source::Advertised`], `crate::authored::GMMU_STATIC`).
    pub gmmu_static: GmmuStaticRow,
    /// GR geometry: GPCs, TPCs, SM order, caps (`GR_GET_INFO_V2` + GPC/TPC masks).
    pub gr_static: GrStaticProfile,
    /// The GR info table (`GR_GET_INFO_V2`).
    pub gr_info: GrInfoProfile,
    /// GR context buffer sizes (`GR_GET_ENGINE_CONTEXT_PROPERTIES`).
    pub gr_context_buffers: [ContextBuffer; CONTEXT_BUFFER_ID_COUNT],
    /// ★ v3-gfx: GR zcull geometry (`GR_GET_ZCULL_INFO`); `None` = the host die has none.
    pub gr_zcull_info: Option<[u32; kf_abi::grstatic::ZCULL_INFO_ROW_WORDS]>,
    /// ★ v3-gfx: the ZBC table index ranges `(start, end)` for color / depth / stencil
    /// (`GET_ZBC_CLEAR_TABLE_SIZE` on a host ZBC object); `None` = the die has no ZBC table.
    pub zbc_table_sizes: Option<[(u32, u32); 3]>,
    /// ★ v3-gfx: `FB_GET_INFO_V2` geometry indices the guest kernel forwards and `kf_abi::fbinfo`
    /// does not derive (`PARTITION_COUNT`/`_MASK`, `LTC_MASK`), each the host's own answer;
    /// an index the host refuses is absent (and the guest's request for it refused, as before).
    pub forwarded_fb_extra: Vec<(u32, u32)>,
    /// ★ v3-gfx: `FB_GET_GPU_CACHE_INFO` (`0x20801315`) — the host's L2 state words
    /// `{powerState, writeMode, bypassMode, rcmState}`; `None` = refused on the host.
    pub gpu_cache_info: Option<[u32; 4]>,
    /// `GPU_GET_INFO_V2` indices answered from the host's own reply.
    pub forwarded_gpu_info: Vec<(u32, u32)>,
    /// ★ `FB_GET_INFO_V2` indices answered from the host's own reply (bus width, RAM type, FBP
    /// count/mask, L2 size, LTC/LTS counts). ⊘ Replaces the GA10x ratio projections
    /// (`kf_abi::fbinfo::GA10X_*`): an AD106 has a 128-bit bus and a 32 MiB L2 that no Ampere
    /// ratio reproduces — the host states the die's own words, unprivileged.
    pub forwarded_fb_info: Vec<(u32, u32)>,
    /// SMC (MIG) mode (`GPU_GET_INFO_V2[GPU_SMC_MODE]`; ⚠ the INTERNAL `GET_SMC_MODE` is
    /// kernel-only).
    pub smc_mode: SmcMode,
    /// The die's PCIe generation (`BUS_GET_INFO_V2`).
    pub pcie_max_gen: PcieGen,
    /// CE fault-method buffer size (`CE_GET_FAULT_METHOD_BUFFER_SIZE` `0x20802a08`). ⊘ Never 0:
    /// RM DMAs CE fault records into a buffer of exactly this size.
    pub ce_fault_method_buffer_size: u32,
    /// GSP feature mask (`GSP_GET_FEATURES` `0x20803601`, word 0).
    pub gsp_features: GspFeatures,
    /// ★ The marketing name (`GPU_GET_NAME_STRING` `0x20800110`, ASCII), served in
    /// `GET_GSP_STATIC_INFO`. `None` serves the 64 zero bytes behind `nvidia-smi`'s `ERR!` —
    /// the old tree's long-standing defect; a composition root must fill it from the host.
    pub gpu_name: Option<GpuName>,
    /// The short name (`GPU_GET_SHORT_NAME_STRING` `0x20800111`), e.g. `GA106-A`.
    pub gpu_short_name: Option<GpuName>,
    /// ★ The host's VBIOS version `(REVISION, OEM_REVISION)` (`BIOS_GET_INFO_V2` `0x20800810`,
    /// NON_PRIVILEGED) — what the synthetic ROM declares (`kf_chip::bar0::vbios_profile`) and what
    /// the guest's own `BIOS_GET_INFO_V2` is answered with (`nvidia-smi`'s VBIOS column).
    /// ⊘ Replaces the GA106 row's `0x9418_0000` on every die.
    /// `None` = the host did not answer: **cosmetic, so never a realize failure** — the ROM carries
    /// `kf_abi::vbios::NEUTRAL_VBIOS_VERSION` and the guest's ask is refused, as before.
    pub vbios_version: Option<(u32, u8)>,
    /// ★ The host's reply to libcudart's `PERF_GET_LEVEL_INFO_V2` question
    /// (`kf_abi::cudartinit::perf_level_info_v2_request`, `0x2080200b`, NON_PRIVILEGED), asked
    /// once at realize. `None` = the host refused it, and the guest's identical ask is refused
    /// likewise (the guest sees what host userspace sees). ⊘ Replaces the GA106 clock words.
    pub perf_level_info_v2: Option<Vec<u8>>,
    /// ★ The host's answers to the GSS-legacy requests of `kf_abi::gssreplay::ROWS` (the clock
    /// listing / clock query `libnvidia-encode` gates a session on), asked at realize with requests
    /// we author. A row the host refused is absent (the guest's is then answered as before).
    pub gss_replay: Vec<kf_abi::gssreplay::Answer>,
    /// ★ The host's `MSENC_GET_CAPS_V2` / `BSP_GET_CAPS_V2` tables for the advertised video
    /// engines (`kf_abi::videocaps`).
    pub video_caps: Vec<kf_abi::videocaps::CapsAnswer>,
}

/// Where a fact comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// An unprivileged host RM control (`0x2080xxxx`), issued once at realize.
    HostControl {
        /// The control id.
        cmd: u32,
        /// Its ogkm name.
        name: &'static str,
    },
    /// A rule of the family axis (`kf_chip::Family`), or ogkm per-family headers.
    FamilyRule(&'static str),
    /// A value we author — the board we present, or one fabricated so RM's own code accepts it.
    Authored(&'static str),
    /// ★ A value WE state as the GSP of the device we present, where the open tree states no
    /// number (w827 ruling): the text names the constant in [`crate::authored`] and its reason.
    Advertised(&'static str),
    /// ⊘ **No source exists**: no unprivileged host control reports it and no family rule is
    /// established. [`crate::hostquery::query_host_facts`] refuses the field BY NAME with this
    /// text and never fills it — a row here is a stated gap, not a default.
    Unsourced(&'static str),
}

/// ★ Every [`HostFacts`] field and its source. Checked exhaustive by
/// `tests/host_facts_ga106.rs`.
pub const PROVENANCE: &[(&str, Source)] = &[
    ("family", Source::HostControl { cmd: 0x2080_1701, name: "MC_GET_ARCH_INFO" }),
    ("has_c2c", Source::HostControl { cmd: 0x2080_182b, name: "BUS_GET_C2C_INFO" }),
    ("ce_caps", Source::HostControl { cmd: 0x2080_2a0a, name: "CE_GET_ALL_CAPS (per-LCE caps + GRCE bits; the three kernel-OR-able bits stripped, kf_abi::cecaps::KERNEL_OR_CAPS)" }),
    // ★ w827 ruling: "WE ARE THE GSP". The host supplies the engine TYPES and COUNTS; every
    // slot of each row describes OUR device (guest channels are re-born on host twins), authored
    // per family in `crate::authored::engine_table` (ogkm constants + a stated layout). The
    // host's own runlist/PBDMA/reset slots are unreachable anyway (0x20800179 PRIVILEGED,
    // 0x20801112 KERNEL). ⊘ Hopper refused by name: no NV_PFAULT_MMU_ENG_ID_HOST0 in the tree.
    ("engines", Source::HostControl { cmd: 0x2080_0170, name: "GPU_GET_ENGINES_V2 (types, counts); every slot authored per family: authored::engine_table / ENGINE_LAYOUT_WHY" }),
    ("lce_pce_masks", Source::HostControl { cmd: 0x2080_2a02, name: "CE_GET_CE_PCE_MASK" }),
    ("intr_table", Source::HostControl { cmd: 0x2080_170e, name: "MC_GET_STATIC_INTR_TABLE (static rows) (static rows, keyed to MC_ENGINE_IDX by rule) + authored::engine_notification_rows (non-stall rows of OUR runlists; 0x2080170d is NOT_SUPPORTED to usermode) + the GSP and DISP stall rows of the device we present (authored::GSP_DISP_VECTORS_WHY)" }),
    ("intr_subtree_map", Source::HostControl { cmd: 0x2080_170f, name: "MC_GET_INTR_CATEGORY_SUBTREE_MAP" }),
    ("chip_info", Source::FamilyRule("USERMODE base = DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET) + NV_VIRTUAL_FUNCTION (ogkm dev_vm.h); sub-rev from MC_GET_ARCH_INFO; isCmpSku from GPU_GET_INFO_V2[CMP_SKU 0x3c]")),
    ("user_register_access_map", Source::Authored("accessmap.rs")),
    ("constructed_falcons", Source::HostControl { cmd: 0x2080_01b0, name: "GPU_GET_CONSTRUCTED_FALCON_INFO, kept to the engDescs of the advertised VIDEO engines (NVENC/NVDEC); empty when the host lists none (hostquery::query_video_falcons)" }),
    ("memory_system", Source::HostControl { cmd: 0x2080_1303, name: "FB_GET_INFO_V2 (L2 size, RAM type, LTC count) + GR_GET_INFO_V2[LITTER_NUM_SLICES_PER_LTC]; comptag policy, compression page and flags authored" }),
    ("device_info", Source::FamilyRule("PRI bases: GR = NV_PGRAPH 0x400000, LCE = the NV_CE block 0x104000 (ogkm dev_ce.h), NVENC/NVDEC = the host falcon table's registerBase for the same engDesc, SW = not a device; over the GPU_GET_ENGINES_V2 list")),
    ("conf_compute", Source::Authored("CC off, fabricated so ogkm accepts it")),
    ("bif_static", Source::Authored("fabricated so ogkm accepts it (no C2C, single function)")),
    ("fifo_channels", Source::Authored("the channel count is ours to set")),
    // ★ w827 ruling: ours to author. The physical computation IS in the open tree and is a
    // silicon reset-default read-back, not a formula (kern_gmmu_tu102.c:548-566).
    ("gmmu_static", Source::Advertised("authored::GMMU_STATIC / GMMU_STATIC_WHY")),
    ("gr_static", Source::HostControl { cmd: 0x2080_1228, name: "GR_GET_INFO_V2 (SMs per TPC) + GR_GET_GPC_MASK 0x2080122a / GR_GET_TPC_MASK 0x2080122b / GR_GET_GLOBAL_SM_ORDER 0x2080121b / GR_GET_CAPS_V2 0x20801227; + GR_GET_ZCULL_MASK 0x20801237; mmu-per-GPC / PES per GPC from GR info litters; TPC-to-PES map, FECS record size, per-subctx header authored (authored.rs)" }),
    ("gr_info", Source::HostControl { cmd: 0x2080_1228, name: "GR_GET_INFO_V2" }),
    ("gr_context_buffers", Source::HostControl { cmd: 0x2080_122d, name: "GR_GET_ENGINE_CONTEXT_PROPERTIES" }),
    ("gr_zcull_info", Source::HostControl { cmd: 0x2080_1206, name: "GR_GET_ZCULL_INFO (host NOT_SUPPORTED = no zcull on the die)" }),
    ("zbc_table_sizes", Source::HostControl { cmd: 0x9096_0106, name: "GET_ZBC_CLEAR_TABLE_SIZE on a host GF100_ZBC_CLEAR object (ranges only; the table itself is per-VM, kf_rm::zbc)" }),
    ("forwarded_fb_extra", Source::HostControl { cmd: 0x2080_1303, name: "FB_GET_INFO_V2 [PARTITION_COUNT 0x04, PARTITION_MASK 0x14/0x37, LTC_MASK 0x2b/0x38], each index asked alone" }),
    ("gpu_cache_info", Source::HostControl { cmd: 0x2080_1315, name: "FB_GET_GPU_CACHE_INFO" }),
    ("forwarded_gpu_info", Source::HostControl { cmd: 0x2080_0102, name: "GPU_GET_INFO_V2" }),
    ("forwarded_fb_info", Source::HostControl { cmd: 0x2080_1303, name: "FB_GET_INFO_V2 (the seven indices libcuda forwards; host words verbatim)" }),
    // ⊘ w827 CORRECTED from `GPU_GET_PARTITIONS 0x20800175` "(no partitions => SMC
    // unsupported)": an inference, and wrong for a MIG-capable part with MIG off (A100 is
    // DISABLED, not UNSUPPORTED). `GPU_GET_INFO_V2[GPU_SMC_MODE]` returns the mode word itself
    // (`kf_abi::smcmode`; real GA106 R21 sweep: `0x2a NV_OK data=0`).
    ("smc_mode", Source::HostControl { cmd: 0x2080_0102, name: "GPU_GET_INFO_V2[GPU_SMC_MODE 0x2a]" }),
    ("pcie_max_gen", Source::HostControl { cmd: 0x2080_1823, name: "BUS_GET_INFO_V2 (PCIE_GEN_INFO)" }),
    // ★ w827 ruling: ours to author. 0x20802a08 is KERNEL_PRIVILEGED (flags 0x1c040) and its
    // physical body is GSP firmware, so it is not asked; see authored::CE_FAULT_METHOD_BUFFER_SIZE_WHY.
    ("ce_fault_method_buffer_size", Source::Advertised("authored::CE_FAULT_METHOD_BUFFER_SIZE / CE_FAULT_METHOD_BUFFER_SIZE_WHY")),
    ("gsp_features", Source::HostControl { cmd: 0x2080_3601, name: "GSP_GET_FEATURES" }),
    ("gpu_name", Source::HostControl { cmd: 0x2080_0110, name: "GPU_GET_NAME_STRING (ASCII)" }),
    ("gpu_short_name", Source::HostControl { cmd: 0x2080_0111, name: "GPU_GET_SHORT_NAME_STRING" }),
    ("vbios_version", Source::HostControl { cmd: 0x2080_0810, name: "BIOS_GET_INFO_V2 [REVISION 0x0, OEM_REVISION 0x1] (a host that does not answer = None: cosmetic, never a realize failure)" }),
    ("perf_level_info_v2", Source::HostControl { cmd: 0x2080_200b, name: "PERF_GET_LEVEL_INFO_V2 (libcudart's question, asked once; a host refusal is kept and relayed)" }),
    ("video_caps", Source::HostControl { cmd: 0x0080_1c02, name: "MSENC_GET_CAPS_V2 0x801b02 / BSP_GET_CAPS_V2 0x801c02 on the host DEVICE, per advertised instance (kf_abi::videocaps)" }),
    ("gss_replay", Source::HostControl { cmd: 0x2080_a028, name: "GSS-legacy 0x20809064 / 0x2080a028 (layouts measured, kf_abi::gssreplay::ROWS), asked with requests we author; the bytes the host wrote" }),
];

/// Why a host reply could not become a fact — by name, never a zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactRefusal {
    /// The reply body is shorter than the control's struct.
    ShortReply {
        /// The control.
        cmd: u32,
        /// Bytes present.
        len: usize,
    },
    /// The host answered a value that cannot be served (e.g. a zero buffer size).
    Unservable {
        /// The control.
        cmd: u32,
        /// Why.
        why: &'static str,
    },
    /// A required index was absent from a list-shaped reply.
    Missing {
        /// The control.
        cmd: u32,
        /// The index.
        index: u32,
    },
    /// No LCE answered at all.
    NoCopyEngine,
}

/// ★ `lce_pce_masks` from the host's `CE_GET_CE_PCE_MASK` replies, asked LCE0, LCE1, … in
/// order. `replies[i]` is `None` where the host REFUSED LCE i; the list ends at the first
/// refusal (an absent LCE is absent, never a zero mask — `cepce.rs`'s "four entries, not five").
///
/// # Errors
/// [`FactRefusal`] on a short reply or when LCE0 is refused.
pub fn derive_lce_pce_masks(replies: &[Option<&[u8]>]) -> Result<Vec<u32>, FactRefusal> {
    let mut out = Vec::new();
    for r in replies {
        let Some(body) = r else { break };
        let mask = kf_abi::cepce::decode_ce_pce_mask(body).map_err(|_| FactRefusal::ShortReply {
            cmd: kf_abi::cepce::NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK,
            len: body.len(),
        })?;
        out.push(mask);
    }
    if out.is_empty() {
        return Err(FactRefusal::NoCopyEngine);
    }
    Ok(out)
}

/// ★ `ce_fault_method_buffer_size` from the host's `0x20802a08` reply. Refuses zero by name.
///
/// # Errors
/// [`FactRefusal::Unservable`] on a zero or short reply.
pub fn derive_ce_fault_method_buffer_size(reply: &[u8]) -> Result<u32, FactRefusal> {
    kf_abi::fmbsize::decode_fault_method_buffer_size(reply).map_err(|_| FactRefusal::Unservable {
        cmd: 0x2080_2a08,
        why: "zero or short: RM DMAs CE fault records into a buffer of exactly this size",
    })
}

/// ★ The present-LCE mask from the host's `CE_GET_ALL_CAPS` (`0x20802a0a`, NON_PRIVILEGED)
/// reply — the cross-check on [`HostFacts::engines`]'s CE rows: the CE geometry the served
/// chain derives from `engines` must name exactly the LCEs the host says are present.
///
/// # Errors
/// [`FactRefusal::ShortReply`].
pub fn derive_ce_present_mask(reply: &[u8]) -> Result<u64, FactRefusal> {
    let at = kf_abi::cecaps::PRESENT_OFF;
    let w = reply.get(at..at + 8).ok_or(FactRefusal::ShortReply {
        cmd: kf_abi::cecaps::NV2080_CTRL_CMD_CE_GET_ALL_CAPS,
        len: reply.len(),
    })?;
    let mut b = [0u8; 8];
    b.copy_from_slice(w);
    Ok(u64::from_le_bytes(b))
}

/// ★ `ce_caps` from the host's `CE_GET_ALL_CAPS` (`0x20802a0a`) reply.
///
/// # Errors
/// [`FactRefusal::ShortReply`]; [`FactRefusal::NoCopyEngine`] when the host marks none present.
pub fn derive_ce_caps(reply: &[u8]) -> Result<kf_abi::cecaps::HostCeCaps, FactRefusal> {
    let c = kf_abi::cecaps::HostCeCaps::decode(reply).map_err(|_| FactRefusal::ShortReply {
        cmd: kf_abi::cecaps::NV2080_CTRL_CMD_CE_GET_ALL_CAPS,
        len: reply.len(),
    })?;
    if c.present == 0 {
        return Err(FactRefusal::NoCopyEngine);
    }
    Ok(c)
}

pub use kf_abi::vbios::{
    BIOS_GET_INFO_V2_PARAMS_SIZE, BIOS_INFO_INDEX_OEM_REVISION, BIOS_INFO_INDEX_REVISION,
    NV2080_CTRL_CMD_BIOS_GET_INFO_V2,
};

/// ★ `vbios_version` from a `BIOS_GET_INFO_V2` reply asked `[REVISION, OEM_REVISION]`.
///
/// # Errors
/// [`FactRefusal::ShortReply`], [`FactRefusal::Missing`] naming an absent index, or
/// [`FactRefusal::Unservable`] for an OEM revision wider than the ROM's byte.
pub fn derive_vbios_version(reply: &[u8]) -> Result<(u32, u8), FactRefusal> {
    let cmd = NV2080_CTRL_CMD_BIOS_GET_INFO_V2;
    if reply.len() < BIOS_GET_INFO_V2_PARAMS_SIZE {
        return Err(FactRefusal::ShortReply { cmd, len: reply.len() });
    }
    let w = |o: usize| u32::from_le_bytes([reply[o], reply[o + 1], reply[o + 2], reply[o + 3]]);
    let n = (w(0) as usize).min(15);
    let find = |index: u32| (0..n).find(|&i| w(4 + 8 * i) == index).map(|i| w(8 + 8 * i));
    let rev = find(BIOS_INFO_INDEX_REVISION).ok_or(FactRefusal::Missing { cmd, index: BIOS_INFO_INDEX_REVISION })?;
    let oem = find(BIOS_INFO_INDEX_OEM_REVISION).ok_or(FactRefusal::Missing { cmd, index: BIOS_INFO_INDEX_OEM_REVISION })?;
    let oem = u8::try_from(oem).map_err(|_| FactRefusal::Unservable { cmd, why: "OEM revision wider than the ROM's byte" })?;
    Ok((rev, oem))
}

/// ★ The GSP feature mask from the host's `GSP_GET_FEATURES` reply (word 0).
///
/// # Errors
/// [`FactRefusal`] on a short reply or a bit this port does not model.
pub fn derive_gsp_features(reply: &[u8]) -> Result<GspFeatures, FactRefusal> {
    let cmd = kf_abi::gspfeatures::NV2080_CTRL_CMD_GSP_GET_FEATURES;
    let w = reply.get(..4).ok_or(FactRefusal::ShortReply { cmd, len: reply.len() })?;
    GspFeatures::from_u32(u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
        .ok_or(FactRefusal::Unservable { cmd, why: "a feature bit this port does not model" })
}

/// ★ The three memory-geometry facts `memory_system` takes from the host: `(l2_cache_size,
/// ram_type, ltc_count)`, out of `FB_GET_INFO_V2` `(index, data)` pairs.
///
/// # Errors
/// [`FactRefusal::Missing`] naming the absent index.
pub fn derive_fb_geometry(pairs: &[(u32, u32)]) -> Result<kf_abi::fbinfo::FbGeometry, FactRefusal> {
    use kf_abi::fbinfo as fb;
    let get = |index: u32| {
        pairs.iter().find(|(i, _)| *i == index).map(|(_, d)| *d).ok_or(FactRefusal::Missing {
            cmd: fb::NV2080_CTRL_CMD_FB_GET_INFO_V2,
            index,
        })
    };
    Ok(fb::FbGeometry {
        l2_cache_size: u64::from(get(fb::FB_INFO_INDEX_L2CACHE_SIZE)?),
        ram_type: get(fb::FB_INFO_INDEX_RAM_TYPE)?,
        ltc_count: get(fb::FB_INFO_INDEX_LTC_COUNT)?,
    })
}

/// ★ The family from the host's `MC_GET_ARCH_INFO` reply.
///
/// # Errors
/// [`FactRefusal`] on a short reply or an architecture with no family.
pub fn derive_family(reply: &[u8]) -> Result<Family, FactRefusal> {
    let cmd = kf_chip::NV2080_CTRL_CMD_MC_GET_ARCH_INFO;
    let b: &[u8; kf_chip::MC_GET_ARCH_INFO_SIZE] = reply
        .get(..kf_chip::MC_GET_ARCH_INFO_SIZE)
        .and_then(|s| s.try_into().ok())
        .ok_or(FactRefusal::ShortReply { cmd, len: reply.len() })?;
    let (arch, imp) = kf_chip::decode_arch_info(b);
    Family::from_arch(arch, imp).map_err(|_| FactRefusal::Unservable { cmd, why: "no family for this architecture" })
}

/// ★ A model name from a host name-string reply: the ASCII run starting at `at` (4 for
/// `GPU_GET_NAME_STRING`, after `gpuNameStringFlags`; 0 for the short name), up to the NUL.
///
/// ⚠ Leaks the string once (`GpuName` holds `&'static str`, built for compile-time rows): call
/// it at realize, once per device — bounded.
///
/// # Errors
/// [`FactRefusal::Unservable`] when the run is empty, unterminated, or not printable ASCII.
pub fn derive_gpu_name(cmd: u32, reply: &[u8], at: usize) -> Result<GpuName, FactRefusal> {
    let bad = FactRefusal::Unservable { cmd, why: "not a NUL-terminated printable ASCII name" };
    let tail = reply.get(at..).ok_or(FactRefusal::ShortReply { cmd, len: reply.len() })?;
    let end = tail.iter().position(|&b| b == 0).ok_or(bad)?;
    let s = core::str::from_utf8(&tail[..end]).map_err(|_| bad)?;
    let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
    GpuName::new(leaked).ok_or(bad)
}

// =====================================================================================
// ★ w827 — the remaining derivations, one per host reply shape.
//
// Every function below is PURE: reply bytes in, a fact or a named refusal out. The request
// side (which control, which input bytes) is `crate::hostquery`, so a test can feed the real
// GA106's captured replies through exactly the code a device runs. Layouts are ogkm-580's
// `ctrl2080*.h`, cited per constant; every size below is also the size a real GA106's
// libcuda asked with (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt`), where it did.
// =====================================================================================

fn le32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|w| u16::from_le_bytes([w[0], w[1]]))
}

fn need(cmd: u32, reply: &[u8], size: usize) -> Result<(), FactRefusal> {
    if reply.len() < size {
        return Err(FactRefusal::ShortReply { cmd, len: reply.len() });
    }
    Ok(())
}

/// `NV2080_CTRL_MC_GET_ARCH_INFO_PARAMS.subRevision` — the `NvU8` at byte 12
/// (`ogkm-580: ctrl2080mc.h:65-70`). `chip_info.chip_sub_rev` is exactly this.
///
/// # Errors
/// [`FactRefusal::ShortReply`].
pub fn derive_sub_revision(reply: &[u8]) -> Result<u8, FactRefusal> {
    let cmd = kf_chip::NV2080_CTRL_CMD_MC_GET_ARCH_INFO;
    need(cmd, reply, kf_chip::MC_GET_ARCH_INFO_SIZE)?;
    Ok(reply[12])
}

/// ★ `has_c2c` from `BUS_GET_C2C_INFO`: `bIsLinkUp`, the field every other one is conditioned
/// on (`kf_abi::c2cinfo`).
///
/// # Errors
/// [`FactRefusal::ShortReply`]; [`FactRefusal::Unservable`] for a byte that is not an `NvBool`.
pub fn derive_has_c2c(reply: &[u8]) -> Result<bool, FactRefusal> {
    let cmd = kf_abi::c2cinfo::NV2080_CTRL_CMD_BUS_GET_C2C_INFO;
    need(cmd, reply, kf_abi::c2cinfo::C2C_INFO_PARAMS_SIZE)?;
    match reply[kf_abi::c2cinfo::B_IS_LINK_UP_OFF] {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(FactRefusal::Unservable { cmd, why: "bIsLinkUp is not an NvBool" }),
    }
}

/// `NV2080_CTRL_CMD_GPU_GET_ENGINES_V2` (`ogkm-580: ctrl2080gpu.h:773`).
pub const NV2080_CTRL_CMD_GPU_GET_ENGINES_V2: u32 = 0x2080_0170;
/// `NV2080_GPU_MAX_ENGINES_LIST_SIZE` (`ogkm-580: ctrl2080gpu.h:776`).
pub const GPU_MAX_ENGINES_LIST_SIZE: usize = 0x54;
/// `sizeof(NV2080_CTRL_GPU_GET_ENGINES_V2_PARAMS)` — `engineCount` + the list. `[measured]`
/// libcuda asks with 340 on a real GA106.
pub const GET_ENGINES_V2_PARAMS_SIZE: usize = 4 + 4 * GPU_MAX_ENGINES_LIST_SIZE;

/// The host's engine list (`NV2080_ENGINE_TYPE_*`, host order) from `GET_ENGINES_V2`.
///
/// # Errors
/// [`FactRefusal::ShortReply`]; [`FactRefusal::Unservable`] for a count over the array.
pub fn derive_engine_list(reply: &[u8]) -> Result<Vec<u32>, FactRefusal> {
    let cmd = NV2080_CTRL_CMD_GPU_GET_ENGINES_V2;
    need(cmd, reply, GET_ENGINES_V2_PARAMS_SIZE)?;
    let n = le32(reply, 0).unwrap_or(0) as usize;
    if n > GPU_MAX_ENGINES_LIST_SIZE {
        return Err(FactRefusal::Unservable { cmd, why: "engineCount exceeds NV2080_GPU_MAX_ENGINES_LIST_SIZE" });
    }
    Ok((0..n).filter_map(|i| le32(reply, 4 + 4 * i)).collect())
}

/// `NV2080_CTRL_CMD_MC_GET_ENGINE_NOTIFICATION_INTR_VECTORS` (`ogkm-580: ctrl2080mc.h:250`).
pub const NV2080_CTRL_CMD_MC_GET_ENGINE_NOTIFICATION_INTR_VECTORS: u32 = 0x2080_170d;
/// `NV2080_CTRL_CMD_MC_GET_STATIC_INTR_TABLE` (`ogkm-580: ctrl2080mc.h:280`).
pub const NV2080_CTRL_CMD_MC_GET_STATIC_INTR_TABLE: u32 = 0x2080_170e;
/// `NV2080_CTRL_CMD_MC_GET_INTR_CATEGORY_SUBTREE_MAP`.
pub const NV2080_CTRL_CMD_MC_GET_INTR_CATEGORY_SUBTREE_MAP: u32 = 0x2080_170f;
/// `NV2080_CTRL_MC_GET_STATIC_INTR_TABLE_MAX`.
pub const MC_STATIC_INTR_TABLE_MAX: usize = 32;
/// `sizeof(NV2080_CTRL_MC_GET_STATIC_INTR_TABLE_PARAMS)` — `numEntries` + 32 × 16.
pub const MC_STATIC_INTR_TABLE_PARAMS_SIZE: usize = 4 + 16 * MC_STATIC_INTR_TABLE_MAX;
/// `NV2080_CTRL_MC_GET_ENGINE_NOTIFICATION_INTR_VECTORS_MAX_ENGINES`.
pub const MC_ENGINE_NOTIFICATION_MAX: usize = 256;
/// `sizeof(NV2080_CTRL_MC_GET_ENGINE_NOTIFICATION_INTR_VECTORS_PARAMS)` — `numEntries` + 256 × 8.
pub const MC_ENGINE_NOTIFICATION_PARAMS_SIZE: usize = 4 + 8 * MC_ENGINE_NOTIFICATION_MAX;
/// `sizeof(NV2080_CTRL_MC_GET_INTR_CATEGORY_SUBTREE_MAP_PARAMS)` — seven aligned `NvU64`.
pub const MC_SUBTREE_MAP_PARAMS_SIZE: usize = 8 * INTR_CATEGORY_COUNT;

/// ★ `MC_ENGINE_IDX_*` for an `NV2080_INTR_TYPE_*` — the key the vGPU-facing static table uses
/// and the key the kernel table (`INTERNAL_INTR_GET_KERNEL_TABLE`) uses are different enums
/// (`ogkm-580: ctrl2080mc.h:288-304` vs `inc/kernel/gpu/intr/engine_idx.h:39-163`). `None` is
/// a type with no index here — refused by name, never dropped.
#[must_use]
pub const fn mc_engine_idx_of_intr_type(intr_type: u32) -> Option<u16> {
    match intr_type {
        0x1 => Some(61),               // NON_REPLAYABLE_FAULT
        0x2 => Some(63),               // NON_REPLAYABLE_FAULT_ERROR
        0x3 => Some(64),               // INFO_FAULT
        0x4 => Some(59),               // REPLAYABLE_FAULT
        0x5 => Some(62),               // REPLAYABLE_FAULT_ERROR
        0x6 => Some(60),               // ACCESS_CNTR
        0x7 => Some(1),                // TMR
        0x8 => Some(73),               // CPU_DOORBELL
        0x9..=0x10 => Some(156 + (intr_type as u16 - 0x9)), // GR0..GR7_FECS_LOG
        _ => None,
    }
}

/// ★ `MC_ENGINE_IDX_*` for an `NV2080_ENGINE_TYPE_*` — the engine rows of the kernel table
/// (`ogkm-580: engine_idx.h` vs `class/cl2080_notification.h:281-351`). Both copy-engine decades
/// are covered (`kf_abi::submit::copy_index_of_engine_type`); `None` is refused by name.
#[must_use]
pub fn mc_engine_idx_of_engine_type(engine_type: u32) -> Option<u16> {
    if let Some(ce) = kf_abi::submit::copy_index_of_engine_type(engine_type) {
        return (ce < u32::from(kf_abi::inittables::MC_ENGINE_IDX_CE_COUNT))
            .then(|| kf_abi::inittables::MC_ENGINE_IDX_CE0 + ce as u16);
    }
    match engine_type {
        0x01..=0x08 => Some(84 + (engine_type as u16 - 0x01)), // GR0..GR7
        0x13..=0x1a => Some(65 + (engine_type as u16 - 0x13)), // NVDEC0..NVDEC7
        0x1b..=0x1d => Some(38 + (engine_type as u16 - 0x1b)), // NVENC0..NVENC2
        0x3f => Some(41),                                      // NVENC3
        0x26 => Some(47),                                      // SEC2
        0x2b..=0x32 => Some(51 + (engine_type as u16 - 0x2b)), // NVJPEG0..NVJPEG7
        0x33 => Some(81),                                      // OFA0
        0x3e => Some(82),                                      // OFA1
        _ => None,
    }
}

/// ★ The kernel interrupt table, from the host's static table (`0x2080170e`, keyed by
/// `NV2080_INTR_TYPE`) and its engine notification vectors (`0x2080170d`, keyed by engine
/// type), both re-keyed to `MC_ENGINE_IDX`. Static rows first, in host order; then one
/// non-stall-only row per engine, in host order.
///
/// ⊘ It is what the host SAYS, not the old captured table: that table also carried
/// `MC_ENGINE_IDX_GSP` and `_DISP` stall rows, and neither control reports them. A consumer
/// that needs them must author them as facts about the board we present, not read them here.
///
/// # Errors
/// [`FactRefusal::ShortReply`]; [`FactRefusal::Unservable`] for a count over its array, a key
/// with no `MC_ENGINE_IDX`, or a table over `INTR_MAX_TABLE_SIZE`.
pub fn derive_intr_table(static_reply: &[u8], notification_reply: &[u8]) -> Result<Vec<IntrTableEntry>, FactRefusal> {
    let mut out = derive_static_intr_table(static_reply)?;
    let cmd = NV2080_CTRL_CMD_MC_GET_ENGINE_NOTIFICATION_INTR_VECTORS;
    need(cmd, notification_reply, MC_ENGINE_NOTIFICATION_PARAMS_SIZE)?;
    let n = le32(notification_reply, 0).unwrap_or(0) as usize;
    if n > MC_ENGINE_NOTIFICATION_MAX {
        return Err(FactRefusal::Unservable { cmd, why: "numEntries exceeds MAX_ENGINES" });
    }
    for i in 0..n {
        let at = 4 + 8 * i;
        let engine_type = le32(notification_reply, at).unwrap_or(0);
        let engine_idx = mc_engine_idx_of_engine_type(engine_type)
            .ok_or(FactRefusal::Unservable { cmd, why: "an NV2080_ENGINE_TYPE with no MC_ENGINE_IDX in this port" })?;
        out.push(IntrTableEntry {
            engine_idx,
            pmc_intr_mask: 0,
            vector_stall: kf_abi::inittables::INTR_VECTOR_INVALID,
            vector_non_stall: le32(notification_reply, at + 4).unwrap_or(0),
        });
    }
    if out.len() > kf_abi::inittables::INTR_MAX_TABLE_SIZE {
        return Err(FactRefusal::Unservable { cmd, why: "more rows than INTR_MAX_TABLE_SIZE" });
    }
    Ok(out)
}

/// The static rows alone (`0x2080170e`), re-keyed to `MC_ENGINE_IDX` — what a usermode host can
/// report; the engine non-stall rows are authored ([`crate::authored::engine_notification_rows`]).
///
/// # Errors
/// As [`derive_intr_table`], for the static reply.
pub fn derive_static_intr_table(static_reply: &[u8]) -> Result<Vec<IntrTableEntry>, FactRefusal> {
    let cmd = NV2080_CTRL_CMD_MC_GET_STATIC_INTR_TABLE;
    need(cmd, static_reply, MC_STATIC_INTR_TABLE_PARAMS_SIZE)?;
    let n = le32(static_reply, 0).unwrap_or(0) as usize;
    if n > MC_STATIC_INTR_TABLE_MAX {
        return Err(FactRefusal::Unservable { cmd, why: "numEntries exceeds NV2080_CTRL_MC_GET_STATIC_INTR_TABLE_MAX" });
    }
    let mut out = Vec::new();
    for i in 0..n {
        let at = 4 + 16 * i;
        let word = |k: usize| le32(static_reply, at + 4 * k).unwrap_or(0);
        let engine_idx = mc_engine_idx_of_intr_type(word(0))
            .ok_or(FactRefusal::Unservable { cmd, why: "an NV2080_INTR_TYPE with no MC_ENGINE_IDX in this port" })?;
        out.push(IntrTableEntry {
            engine_idx,
            pmc_intr_mask: word(1),
            vector_stall: word(2),
            vector_non_stall: word(3),
        });
    }
    Ok(out)
}

/// The category → subtree map, verbatim (`0x2080170f`).
///
/// # Errors
/// [`FactRefusal::ShortReply`].
pub fn derive_intr_subtree_map(reply: &[u8]) -> Result<[u64; INTR_CATEGORY_COUNT], FactRefusal> {
    let cmd = NV2080_CTRL_CMD_MC_GET_INTR_CATEGORY_SUBTREE_MAP;
    need(cmd, reply, MC_SUBTREE_MAP_PARAMS_SIZE)?;
    let mut out = [0u64; INTR_CATEGORY_COUNT];
    for (i, o) in out.iter_mut().enumerate() {
        let mut b = [0u8; 8];
        b.copy_from_slice(&reply[8 * i..8 * i + 8]);
        *o = u64::from_le_bytes(b);
    }
    Ok(out)
}

/// `NV2080_CTRL_CMD_GR_GET_INFO_V2` (`ogkm-580: ctrl2080gr.h:1457`).
pub const NV2080_CTRL_CMD_GR_GET_INFO_V2: u32 = 0x2080_1228;
/// `NV2080_CTRL_CMD_GR_GET_GPC_MASK`.
pub const NV2080_CTRL_CMD_GR_GET_GPC_MASK: u32 = 0x2080_122a;
/// `NV2080_CTRL_CMD_GR_GET_TPC_MASK`.
pub const NV2080_CTRL_CMD_GR_GET_TPC_MASK: u32 = 0x2080_122b;
/// `NV2080_CTRL_CMD_GR_GET_GLOBAL_SM_ORDER`.
pub const NV2080_CTRL_CMD_GR_GET_GLOBAL_SM_ORDER: u32 = 0x2080_121b;
/// `NV2080_CTRL_CMD_GR_GET_CAPS_V2`.
pub const NV2080_CTRL_CMD_GR_GET_CAPS_V2: u32 = 0x2080_1227;
/// `NV2080_CTRL_CMD_GR_GET_ENGINE_CONTEXT_PROPERTIES`.
pub const NV2080_CTRL_CMD_GR_GET_ENGINE_CONTEXT_PROPERTIES: u32 = 0x2080_122d;
/// `sizeof(NV0080_CTRL_GR_ROUTE_INFO)` — `NvU32 flags` then an 8-aligned `NvU64 route`. All
/// zero is `TYPE_NONE`: GR0 on a part without MIG.
pub const GR_ROUTE_INFO_SIZE: usize = 16;
/// Where `grRouteInfo` sits in `GR_GET_INFO_V2`: after `4 + 0x3a × 8 = 468`, aligned to 8.
pub const GR_GET_INFO_V2_ROUTE_OFF: usize = 472;
/// `sizeof(NV2080_CTRL_GR_GET_INFO_V2_PARAMS)`.
pub const GR_GET_INFO_V2_PARAMS_SIZE: usize = GR_GET_INFO_V2_ROUTE_OFF + GR_ROUTE_INFO_SIZE;
/// `sizeof(NV2080_CTRL_GR_GET_GPC_MASK_PARAMS)` / `..._TPC_MASK_PARAMS` — route, then two
/// words. `[measured]` 24 on a real GA106.
pub const GR_MASK_PARAMS_SIZE: usize = 24;
/// `NV2080_CTRL_CMD_GR_GET_GLOBAL_SM_ORDER_MAX_SM_COUNT`.
pub const GR_GLOBAL_SM_ORDER_MAX_SM: usize = 512;
/// One `globalSmId[]` entry: nine `NvU16`.
pub const GR_GLOBAL_SM_ENTRY_SIZE: usize = 18;
/// `sizeof(NV2080_CTRL_GR_GET_GLOBAL_SM_ORDER_PARAMS)` — entries, `numSm`, `numTpc`, then the
/// 8-aligned route. `[measured]` 9240 on a real GA106.
pub const GR_GLOBAL_SM_ORDER_PARAMS_SIZE: usize = 9224 + GR_ROUTE_INFO_SIZE;
/// `sizeof(NV2080_CTRL_GR_GET_CAPS_V2_PARAMS)` — 23 caps bytes, the 8-aligned route at 24,
/// `bCapsPopulated` at 40. `[measured]` 48 on a real GA106.
pub const GR_CAPS_V2_PARAMS_SIZE: usize = 48;
/// `sizeof(NV2080_CTRL_GR_GET_ENGINE_CONTEXT_PROPERTIES_PARAMS)` — route, `engineId`,
/// `alignment`, `size`, `bInfoPopulated`.
pub const GR_CONTEXT_PROPERTIES_PARAMS_SIZE: usize = 32;

/// The whole `GR_GET_INFO_V2` table, one row per index in index order (`data[i]` for
/// `index == i`).
///
/// # Errors
/// [`FactRefusal::ShortReply`]; [`FactRefusal::Unservable`] when the reply is not the full
/// table in index order, or fails [`GrInfoProfile::validate`].
pub fn derive_gr_info(reply: &[u8]) -> Result<GrInfoProfile, FactRefusal> {
    use kf_abi::grinfo::GR_INFO_MAX_SIZE;
    let cmd = NV2080_CTRL_CMD_GR_GET_INFO_V2;
    need(cmd, reply, GR_GET_INFO_V2_PARAMS_SIZE)?;
    if le32(reply, 0) != Some(GR_INFO_MAX_SIZE as u32) {
        return Err(FactRefusal::Unservable { cmd, why: "grInfoListSize is not the whole table" });
    }
    let mut data = [0u32; GR_INFO_MAX_SIZE];
    for (i, d) in data.iter_mut().enumerate() {
        if le32(reply, 4 + 8 * i) != Some(i as u32) {
            return Err(FactRefusal::Missing { cmd, index: i as u32 });
        }
        *d = le32(reply, 8 + 8 * i).unwrap_or(0);
    }
    let p = GrInfoProfile { data };
    p.validate().map_err(|_| FactRefusal::Unservable { cmd, why: "a GR info entry RM's own readers require non-zero is zero" })?;
    Ok(p)
}

/// `gpcMask` from `GR_GET_GPC_MASK` (the word after the route).
///
/// # Errors
/// [`FactRefusal::ShortReply`]; [`FactRefusal::Unservable`] for a zero mask (the rejected
/// shortcut `kf_abi::grstatic` names).
pub fn derive_gpc_mask(reply: &[u8]) -> Result<u32, FactRefusal> {
    let cmd = NV2080_CTRL_CMD_GR_GET_GPC_MASK;
    need(cmd, reply, GR_MASK_PARAMS_SIZE)?;
    match le32(reply, GR_ROUTE_INFO_SIZE) {
        Some(0) | None => Err(FactRefusal::Unservable { cmd, why: "gpcMask is zero" }),
        Some(m) => Ok(m),
    }
}

/// `tpcMask` for `gpc` from `GR_GET_TPC_MASK`, checking the reply names the GPC asked.
///
/// # Errors
/// [`FactRefusal::ShortReply`]; [`FactRefusal::Unservable`] when the echoed `gpcId` differs.
pub fn derive_tpc_mask(reply: &[u8], gpc: u32) -> Result<u32, FactRefusal> {
    let cmd = NV2080_CTRL_CMD_GR_GET_TPC_MASK;
    need(cmd, reply, GR_MASK_PARAMS_SIZE)?;
    if le32(reply, GR_ROUTE_INFO_SIZE) != Some(gpc) {
        return Err(FactRefusal::Unservable { cmd, why: "the reply names a different gpcId" });
    }
    Ok(le32(reply, GR_ROUTE_INFO_SIZE + 4).unwrap_or(0))
}

/// The 23 caps bytes from `GR_GET_CAPS_V2`, refused unless `bCapsPopulated`.
///
/// # Errors
/// [`FactRefusal::ShortReply`]; [`FactRefusal::Unservable`] when not populated.
pub fn derive_gr_caps(reply: &[u8]) -> Result<[u8; kf_abi::grstatic::GR_CAPS_TBL_SIZE], FactRefusal> {
    let cmd = NV2080_CTRL_CMD_GR_GET_CAPS_V2;
    need(cmd, reply, GR_CAPS_V2_PARAMS_SIZE)?;
    if reply[40] != 1 {
        return Err(FactRefusal::Unservable { cmd, why: "bCapsPopulated is false" });
    }
    let mut caps = [0u8; kf_abi::grstatic::GR_CAPS_TBL_SIZE];
    caps.copy_from_slice(&reply[..kf_abi::grstatic::GR_CAPS_TBL_SIZE]);
    Ok(caps)
}

/// ★ The TPC rows in `globalTpcId` order, and the SMs per TPC, from `GR_GET_GLOBAL_SM_ORDER`.
///
/// Each TPC's row is its `localSmId == 0` entry's `{gpcId, localTpcId, virtualTpcId}`; every TPC
/// must own exactly `numSm / numTpc` entries — the pairing `kf_abi::grstatic::TpcRow` states,
/// checked here rather than assumed.
///
/// # Errors
/// [`FactRefusal::ShortReply`]; [`FactRefusal::Unservable`] for zero or over-range counts, an
/// uneven SM split, or a TPC with no `localSmId == 0` entry.
pub fn derive_sm_order(reply: &[u8]) -> Result<(Vec<kf_abi::grstatic::TpcRow>, u16), FactRefusal> {
    let cmd = NV2080_CTRL_CMD_GR_GET_GLOBAL_SM_ORDER;
    need(cmd, reply, GR_GLOBAL_SM_ORDER_PARAMS_SIZE)?;
    let at = GR_GLOBAL_SM_ORDER_MAX_SM * GR_GLOBAL_SM_ENTRY_SIZE;
    let num_sm = le16(reply, at).unwrap_or(0);
    let num_tpc = le16(reply, at + 2).unwrap_or(0);
    let bad = |why| FactRefusal::Unservable { cmd, why };
    if num_tpc == 0 || usize::from(num_sm) > GR_GLOBAL_SM_ORDER_MAX_SM || num_sm % num_tpc != 0 {
        return Err(bad("numSm/numTpc zero, over range, or not an even split"));
    }
    let sms_per_tpc = num_sm / num_tpc;
    let field = |sm: usize, k: usize| le16(reply, sm * GR_GLOBAL_SM_ENTRY_SIZE + 2 * k).unwrap_or(u16::MAX);
    let mut tpcs = Vec::with_capacity(usize::from(num_tpc));
    for t in 0..num_tpc {
        let owned: Vec<usize> = (0..usize::from(num_sm)).filter(|&sm| field(sm, 3) == t).collect();
        if owned.len() != usize::from(sms_per_tpc) {
            return Err(bad("a TPC does not own numSm/numTpc SMs"));
        }
        let first = owned.iter().copied().find(|&sm| field(sm, 2) == 0).ok_or(bad("a TPC has no localSmId 0"))?;
        tpcs.push(kf_abi::grstatic::TpcRow {
            gpc_id: field(first, 0),
            local_tpc_id: field(first, 1),
            virtual_tpc_id: field(first, 8),
        });
    }
    Ok((tpcs, sms_per_tpc))
}

/// One context buffer from `GR_GET_ENGINE_CONTEXT_PROPERTIES`. ★ `host_not_supported` is the
/// host's `NV_ERR_NOT_SUPPORTED` for this id, which RM returns exactly when the buffer's size
/// is `NV_U32_MAX` (`ogkm-580: kernel_graphics.c:4065-4068`) — so it is a positive statement
/// of [`kf_abi::grstatic::CONTEXT_BUFFER_ABSENT`], not a gap.
///
/// # Errors
/// [`FactRefusal::ShortReply`]; [`FactRefusal::Unservable`] for a present buffer with zero
/// alignment or an unpopulated reply.
pub fn derive_context_buffer(reply: Option<&[u8]>) -> Result<ContextBuffer, FactRefusal> {
    use kf_abi::grstatic::CONTEXT_BUFFER_ABSENT;
    let cmd = NV2080_CTRL_CMD_GR_GET_ENGINE_CONTEXT_PROPERTIES;
    let Some(reply) = reply else {
        return Ok(ContextBuffer { size: CONTEXT_BUFFER_ABSENT, alignment: CONTEXT_BUFFER_ABSENT });
    };
    need(cmd, reply, GR_CONTEXT_PROPERTIES_PARAMS_SIZE)?;
    if reply[28] != 1 {
        return Err(FactRefusal::Unservable { cmd, why: "bInfoPopulated is false" });
    }
    let alignment = le32(reply, 20).unwrap_or(0);
    let size = le32(reply, 24).unwrap_or(0);
    if alignment == 0 {
        return Err(FactRefusal::Unservable { cmd, why: "a present context buffer with zero alignment" });
    }
    Ok(ContextBuffer { size, alignment })
}

/// The value of `index` in a `GPU_GET_INFO_V2` reply.
///
/// # Errors
/// [`FactRefusal::ShortReply`] on a malformed reply; [`FactRefusal::Missing`] when absent.
pub fn derive_gpu_info_value(reply: &[u8], index: u32) -> Result<u32, FactRefusal> {
    let cmd = kf_abi::gpuinfo::NV2080_CTRL_CMD_GPU_GET_INFO_V2;
    let pairs = kf_abi::gpuinfo::decode_gpu_info_pairs(reply).map_err(|_| FactRefusal::ShortReply { cmd, len: reply.len() })?;
    pairs.iter().find(|(i, _)| *i == index).map(|(_, d)| *d).ok_or(FactRefusal::Missing { cmd, index })
}

/// `NV2080_CTRL_GPU_INFO_INDEX_GPU_SMC_MODE` (`ogkm-580: ctrl2080gpu.h:87`).
pub const GPU_INFO_INDEX_GPU_SMC_MODE: u32 = 0x2a;
/// `NV2080_CTRL_GPU_INFO_INDEX_CMP_SKU` (`ogkm-580: ctrl2080gpu.h:107`).
pub const GPU_INFO_INDEX_CMP_SKU: u32 = 0x3c;

/// `smc_mode` from `GPU_GET_INFO_V2[GPU_SMC_MODE]` — the same word the INTERNAL control carries
/// (`kf_abi::smcmode`: `getGpuInfos` does `data = params.smcMode`).
///
/// # Errors
/// As [`derive_gpu_info_value`]; [`FactRefusal::Unservable`] for a word naming no mode.
pub fn derive_smc_mode(reply: &[u8]) -> Result<SmcMode, FactRefusal> {
    let cmd = kf_abi::gpuinfo::NV2080_CTRL_CMD_GPU_GET_INFO_V2;
    let word = derive_gpu_info_value(reply, GPU_INFO_INDEX_GPU_SMC_MODE)?;
    kf_abi::smcmode::decode_smc_mode(&word.to_le_bytes())
        .map_err(|_| FactRefusal::Unservable { cmd, why: "GPU_SMC_MODE names no NV2080_CTRL_GPU_INFO_GPU_SMC_MODE_*" })
}

/// `isCmpSku` from `GPU_GET_INFO_V2[CMP_SKU]` (`_NO` = 0, `_YES` = 1).
///
/// # Errors
/// As [`derive_gpu_info_value`]; [`FactRefusal::Unservable`] for any other word.
pub fn derive_cmp_sku(reply: &[u8]) -> Result<bool, FactRefusal> {
    let cmd = kf_abi::gpuinfo::NV2080_CTRL_CMD_GPU_GET_INFO_V2;
    match derive_gpu_info_value(reply, GPU_INFO_INDEX_CMP_SKU)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(FactRefusal::Unservable { cmd, why: "CMP_SKU is neither _NO nor _YES" }),
    }
}

/// The die's own PCIe generation — `GPU_GEN` of `BUS_GET_INFO_V2[PCIE_GEN_INFO]`. ⊘ Never the
/// negotiated or current field: those are the slot's (`kf_abi::businfo::PcieGenInfo`).
///
/// # Errors
/// [`FactRefusal::ShortReply`] on a malformed reply; [`FactRefusal::Missing`] when absent;
/// [`FactRefusal::Unservable`] for a word naming no generation.
pub fn derive_pcie_max_gen(reply: &[u8]) -> Result<PcieGen, FactRefusal> {
    use kf_abi::businfo as bus;
    let cmd = bus::NV2080_CTRL_CMD_BUS_GET_INFO_V2;
    let pairs = bus::decode_bus_info_pairs(reply).map_err(|_| FactRefusal::ShortReply { cmd, len: reply.len() })?;
    let word = pairs
        .iter()
        .find(|(i, _)| *i == bus::BUS_INFO_INDEX_PCIE_GEN_INFO)
        .map(|(_, d)| *d)
        .ok_or(FactRefusal::Missing { cmd, index: bus::BUS_INFO_INDEX_PCIE_GEN_INFO })?;
    bus::PcieGenInfo::decode(word)
        .map(|g| g.gpu_gen)
        .ok_or(FactRefusal::Unservable { cmd, why: "PCIE_GEN_INFO names a generation the header does not define" })
}
