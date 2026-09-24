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
//! ⊘ kf-rm does not fill it. kf-rm does not depend on `kf-host` (host calls go through traits a
//! later crate implements), so this module holds only the struct, its provenance table, and the
//! small pure **derivations** from host reply bytes that can be checked here against the old
//! tree's captured GA106 rows (the port map's rule: *"on a GA106 host, derived must equal
//! captured"* — `tests/host_facts_ga106.rs`, fed real-GA106 reply bytes from
//! `traces/real_ga106/`).
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
    /// GMMU fault-buffer sizes (family row).
    pub gmmu_static: GmmuStaticRow,
    /// GR geometry: GPCs, TPCs, SM order, caps (`GR_GET_INFO_V2` + GPC/TPC masks).
    pub gr_static: GrStaticProfile,
    /// The GR info table (`GR_GET_INFO_V2`).
    pub gr_info: GrInfoProfile,
    /// GR context buffer sizes (`GR_GET_ENGINE_CONTEXT_PROPERTIES`).
    pub gr_context_buffers: [ContextBuffer; CONTEXT_BUFFER_ID_COUNT],
    /// `GPU_GET_INFO_V2` indices answered from the host's own reply.
    pub forwarded_gpu_info: Vec<(u32, u32)>,
    /// SMC (MIG) mode (`GPU_GET_PARTITIONS`; ⚠ the INTERNAL `GET_SMC_MODE` is kernel-only).
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
}

/// ★ Every [`HostFacts`] field and its source. Checked exhaustive by
/// `tests/host_facts_ga106.rs`.
pub const PROVENANCE: &[(&str, Source)] = &[
    ("family", Source::HostControl { cmd: 0x2080_1701, name: "MC_GET_ARCH_INFO" }),
    ("has_c2c", Source::HostControl { cmd: 0x2080_182b, name: "BUS_GET_C2C_INFO" }),
    ("engines", Source::HostControl { cmd: 0x2080_0170, name: "GPU_GET_ENGINES_V2 (+ GET_HW_ENGINE_ID, family device-info rules)" }),
    ("lce_pce_masks", Source::HostControl { cmd: 0x2080_2a02, name: "CE_GET_CE_PCE_MASK" }),
    ("intr_table", Source::HostControl { cmd: 0x2080_170e, name: "MC_GET_STATIC_INTR_TABLE" }),
    ("intr_subtree_map", Source::HostControl { cmd: 0x2080_170f, name: "MC_GET_INTR_CATEGORY_SUBTREE_MAP" }),
    ("chip_info", Source::FamilyRule("register bases per family (ogkm dev_*.h); sub-rev from MC_GET_ARCH_INFO")),
    ("user_register_access_map", Source::Authored("accessmap.rs")),
    ("constructed_falcons", Source::Authored("none constructed")),
    ("memory_system", Source::HostControl { cmd: 0x2080_1303, name: "FB_GET_INFO_V2 (L2 size, RAM type, LTC count; rest fabricated)" }),
    ("device_info", Source::FamilyRule("engine PRI bases (nova/nouveau device-info decoding)")),
    ("conf_compute", Source::Authored("CC off, fabricated so ogkm accepts it")),
    ("bif_static", Source::Authored("fabricated so ogkm accepts it (no C2C, single function)")),
    ("fifo_channels", Source::Authored("the channel count is ours to set")),
    ("gmmu_static", Source::FamilyRule("GMMU fault-buffer sizes, family row")),
    ("gr_static", Source::HostControl { cmd: 0x2080_1228, name: "GR_GET_INFO_V2 + GR_GET_GPC_MASK 0x2080122a / GR_GET_TPC_MASK 0x2080122b / GR_GET_GLOBAL_SM_ORDER 0x2080121b / GR_GET_CAPS_V2 0x20801227" }),
    ("gr_info", Source::HostControl { cmd: 0x2080_1228, name: "GR_GET_INFO_V2" }),
    ("gr_context_buffers", Source::HostControl { cmd: 0x2080_122d, name: "GR_GET_ENGINE_CONTEXT_PROPERTIES" }),
    ("forwarded_gpu_info", Source::HostControl { cmd: 0x2080_0102, name: "GPU_GET_INFO_V2" }),
    ("smc_mode", Source::HostControl { cmd: 0x2080_0175, name: "GPU_GET_PARTITIONS (no partitions => SMC unsupported)" }),
    ("pcie_max_gen", Source::HostControl { cmd: 0x2080_1823, name: "BUS_GET_INFO_V2 (PCIE_GEN_INFO)" }),
    ("ce_fault_method_buffer_size", Source::HostControl { cmd: 0x2080_2a08, name: "CE_GET_FAULT_METHOD_BUFFER_SIZE" }),
    ("gsp_features", Source::HostControl { cmd: 0x2080_3601, name: "GSP_GET_FEATURES" }),
    ("gpu_name", Source::HostControl { cmd: 0x2080_0110, name: "GPU_GET_NAME_STRING (ASCII)" }),
    ("gpu_short_name", Source::HostControl { cmd: 0x2080_0111, name: "GPU_GET_SHORT_NAME_STRING" }),
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
