//! ★★★★★ **Filling [`HostFacts`] from the host — which controls are issued, with which
//! request bytes, and what each reply becomes.**
//!
//! [`crate::hostfacts`] holds the struct, its [`PROVENANCE`](crate::hostfacts::PROVENANCE) table and the pure `derive_*`
//! functions (reply bytes → fact). This module is the other half: the request each field is
//! asked with, over the [`HostControls`] seam, plus the family rules and authored values the
//! provenance table names. The composition root implements [`HostControls`] over the real host
//! session (`kf-qemu`'s `rmfacts`); a test implements it over the real GA106's captured replies
//! (`tests/host_facts_query_ga106.rs`) — the SAME request code runs in both.
//!
//! # The rules this module keeps
//!
//! - ★ **Every field from exactly the source [`PROVENANCE`](crate::hostfacts::PROVENANCE)
//!   names.** A host-control field is asked of the host; a family rule or an authored /
//!   advertised value — what WE state as the GSP of the device we present (w827 ruling) — comes
//!   from [`crate::authored`], each with its ogkm citation or its stated reason; an `Unsourced`
//!   row would be refused with the table's own text.
//! - ⊘ **A refusal is named, never defaulted.** A control the host refuses, a reply that does
//!   not decode, a slot with no source: each becomes a [`FieldRefusal`] naming the field and the
//!   cause. [`query_host_facts`] collects EVERY refusal before returning, so one realize reports
//!   the whole gap list, not its first line.
//! - ⊘ **No per-die constant.** Nothing here names a GA106 value; the numbers are ogkm header
//!   constants, family rules, and [`crate::authored`]'s advertised values, each with its reason.

use crate::authored::{self, EngineKind};
use crate::hostfacts::{self, FactRefusal, HostFacts};
use kf_abi::bifstatic::BifStaticRow;
use kf_abi::chipinfo::{ChipInfoRow, RegBaseRow, reg_base};
use kf_abi::confcompute::ConfComputeRow;
use kf_abi::deviceinfo::{DeviceInfoRow, DevicePriBase, EnginePriBase};
use kf_abi::falconinfo::{ConstructedFalcon, FalconInventoryRow};
use kf_abi::fifochannels::FifoChannelsRow;
use kf_abi::grinfo::GrInfoProfile;
use kf_abi::grstatic::{CONTEXT_BUFFER_ID_COUNT, ContextBuffer, TpcRow};
use kf_abi::memsysconfig::{ComptagAllocationPolicy, MemorySystemRow};
use kf_abi::regaccessmap::RegisterAccessMapRow;
use kf_chip::Family;

/// `NV_ERR_NOT_SUPPORTED`.
pub const NV_ERR_NOT_SUPPORTED: u32 = 0x56;

/// A control the host refused. `status` is RM's `NV_STATUS` when the host stated one (`None`
/// for a failure below RM — an errno, a malformed ioctl); `detail` is the session's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRefusal {
    /// RM's status, when there is one.
    pub status: Option<u32>,
    /// What the host session reported.
    pub detail: String,
}

/// ★ **The seam**: issue one `NV2080` control on the host subdevice, `params` in and out.
///
/// ⊘ Implemented by the composition root over the real session, and by tests over captured
/// replies — never by anything that invents a reply.
pub trait HostControls {
    /// Issue `cmd` with `params`; on `Ok`, `params` holds the host's reply.
    ///
    /// # Errors
    /// [`HostRefusal`] — the host's own refusal.
    fn control(&mut self, cmd: u32, params: &mut [u8]) -> Result<(), HostRefusal>;

    /// Issue an `NV0080` control on the host DEVICE object. Default: refused (a test double that
    /// answers only subdevice controls).
    ///
    /// # Errors
    /// [`HostRefusal`].
    fn device_control(&mut self, cmd: u32, _params: &mut [u8]) -> Result<(), HostRefusal> {
        Err(HostRefusal { status: None, detail: format!("{cmd:#x}: no device-level controls on this session") })
    }
}

/// Why one field could not be filled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldCause {
    /// The host refused a control the field is asked through.
    Host {
        /// The control.
        cmd: u32,
        /// The host's refusal.
        refused: HostRefusal,
    },
    /// The host answered, and the answer could not become the fact.
    Reply(FactRefusal),
    /// The field — or the named part of it — has no source ([`Source::Unsourced`](crate::hostfacts::Source::Unsourced)).
    Unsourced(&'static str),
    /// A per-family authored rule needs a number no source states for this family
    /// ([`authored::LayoutRefusal`]).
    FamilyLayout(authored::LayoutRefusal),
    /// The host's family is not the family realize chose.
    FamilyMismatch {
        /// The family realize passed in.
        asked: Family,
        /// The family the host's `MC_GET_ARCH_INFO` names.
        host: Family,
    },
    /// A field this one is derived from was itself refused.
    DependsOn(&'static str),
}

impl From<FactRefusal> for FieldCause {
    fn from(r: FactRefusal) -> Self {
        FieldCause::Reply(r)
    }
}

/// One refused field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRefusal {
    /// The [`HostFacts`] field.
    pub field: &'static str,
    /// Why.
    pub cause: FieldCause,
}

/// ★ Every field that could not be filled — the whole list, in [`PROVENANCE`](crate::hostfacts::PROVENANCE) order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFactsRefused {
    /// The refusals.
    pub refusals: Vec<FieldRefusal>,
}

impl HostFactsRefused {
    /// The refused field names, in order — what a test or a log asserts against.
    #[must_use]
    pub fn fields(&self) -> Vec<&'static str> {
        self.refusals.iter().map(|r| r.field).collect()
    }
}

impl core::fmt::Display for HostFactsRefused {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} host fact(s) refused by name:", self.refusals.len())?;
        for r in &self.refusals {
            write!(f, "\n  {}: ", r.field)?;
            match &r.cause {
                FieldCause::Host { cmd, refused } => {
                    write!(f, "host refused {cmd:#010x}")?;
                    if let Some(s) = refused.status {
                        write!(f, " (status {s:#x})")?;
                    }
                    write!(f, ": {}", refused.detail)?;
                }
                FieldCause::Reply(fr) => write!(f, "reply not servable: {fr:?}")?,
                FieldCause::Unsourced(why) => write!(f, "UNSOURCED: {why}")?,
                FieldCause::FamilyMismatch { asked, host } => {
                    write!(f, "realize chose {asked:?}, the host is {host:?}")?;
                }
                FieldCause::FamilyLayout(l) => write!(f, "no {:?} rule: {}", l.family, l.missing)?,
                FieldCause::DependsOn(other) => write!(f, "depends on `{other}`, which was refused")?,
            }
        }
        Ok(())
    }
}

impl std::error::Error for HostFactsRefused {}

/// Issue `cmd` with `params` (already carrying its input), returning the reply.
fn ask(host: &mut dyn HostControls, cmd: u32, mut params: Vec<u8>) -> Result<Vec<u8>, FieldCause> {
    host.control(cmd, &mut params).map_err(|refused| FieldCause::Host { cmd, refused })?;
    Ok(params)
}

/// A zeroed request of `size` bytes.
fn zeroed(size: usize) -> Vec<u8> {
    vec![0u8; size]
}

fn put32(p: &mut [u8], at: usize, v: u32) {
    p[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// A `*_GET_INFO_V2` request (`{count, {index, data}[N]}` — the GPU, BUS and FB shapes are the
/// same) asking `indices`, in `size` bytes.
fn info_list_request(size: usize, indices: &[u32]) -> Vec<u8> {
    let mut p = zeroed(size);
    put32(&mut p, 0, u32::try_from(indices.len()).unwrap_or(u32::MAX));
    for (i, &index) in indices.iter().enumerate() {
        put32(&mut p, 4 + 8 * i, index);
    }
    p
}

// =====================================================================================
// Architecture
// =====================================================================================

/// `MC_GET_ARCH_INFO` → `(family, subRevision)`.
///
/// # Errors
/// [`FieldCause`].
pub fn query_arch(host: &mut dyn HostControls) -> Result<(Family, u8), FieldCause> {
    let r = ask(host, kf_chip::NV2080_CTRL_CMD_MC_GET_ARCH_INFO, zeroed(kf_chip::MC_GET_ARCH_INFO_SIZE))?;
    Ok((hostfacts::derive_family(&r)?, hostfacts::derive_sub_revision(&r)?))
}

// =====================================================================================
// Engines
// =====================================================================================

/// Classify an `NV2080_ENGINE_TYPE`, and give its `RM_ENGINE_TYPE`. `None` = not advertised.
///
/// ★ **Authored**: the served engine list is GR, the copy engines, the VIDEO engines (NVENC /
/// NVDEC, w-video 2026-09-26 — the passthrough plane now drives them on host twins) and RM's `SW`
/// pseudo-engine. *"An engine we advertise is an engine RM goes on to USE"* (`kf_abi::deviceinfo`)
/// still holds for the rest: NVJPG, OFA and SEC2 are read and deliberately not advertised. ⊘ The
/// video set is the HOST's — an H100 lists no NVENC, so its guest gets none. A video engine is
/// advertised only when the host's falcon table also names it ([`query_video_falcons`]). The RM space is contiguous where the NV2080 space is
/// not (`kf_abi::submit`'s two inverses): `RM_ENGINE_TYPE_COPY(i) = 0x09 + i` for all twenty,
/// `GR(i) = 0x01 + i`, `SW = 0x2d` (`ogkm-580: gpu_engine_type.h:34-139`).
#[must_use]
pub fn classify_engine(nv2080_engine_type: u32) -> Option<(EngineKind, u32)> {
    if let Some(ce) = kf_abi::submit::copy_index_of_engine_type(nv2080_engine_type) {
        return Some((EngineKind::Copy(ce), kf_abi::submit::RM_ENGINE_TYPE_COPY0 + ce));
    }
    if let Some(i) = kf_abi::submit::nvenc_index_of_engine_type(nv2080_engine_type) {
        return Some((EngineKind::VideoEncode(i), kf_abi::submit::RM_ENGINE_TYPE_NVENC0 + i));
    }
    if let Some(i) = kf_abi::submit::nvdec_index_of_engine_type(nv2080_engine_type) {
        return Some((EngineKind::VideoDecode(i), kf_abi::submit::RM_ENGINE_TYPE_NVDEC0 + i));
    }
    match nv2080_engine_type {
        0x01..=0x08 => Some((EngineKind::Graphics(nv2080_engine_type - 1), nv2080_engine_type)),
        kf_abi::submit::NV2080_ENGINE_TYPE_SW => Some((EngineKind::Software, kf_abi::submit::RM_ENGINE_TYPE_SW)),
        _ => None,
    }
}

/// ★ The engines the device presents, in host order: the host's `GET_ENGINES_V2` list, kept to
/// the kinds [`classify_engine`] advertises. The host supplies the TYPES and COUNTS; the rest
/// of each row is the device's own layout ([`authored::engine_table`]).
///
/// # Errors
/// [`FieldCause`] — including an empty GR or CE set, which no GSP-client RM boots with.
pub fn query_engine_list(host: &mut dyn HostControls) -> Result<Vec<EngineKind>, FieldCause> {
    let cmd = hostfacts::NV2080_CTRL_CMD_GPU_GET_ENGINES_V2;
    let list = ask(host, cmd, zeroed(hostfacts::GET_ENGINES_V2_PARAMS_SIZE))?;
    let kinds: Vec<EngineKind> =
        hostfacts::derive_engine_list(&list)?.into_iter().filter_map(|t| classify_engine(t).map(|(k, _)| k)).collect();
    if !kinds.iter().any(|k| matches!(k, EngineKind::Graphics(_))) || !kinds.iter().any(|k| matches!(k, EngineKind::Copy(_))) {
        return Err(FieldCause::Reply(FactRefusal::Unservable { cmd, why: "the host lists no GR or no copy engine" }));
    }
    Ok(kinds)
}

/// ★ `device_info` — the family rule over the engine list.
///
/// `GR` → `NV_PGRAPH` at `0x400000` (every family since Fermi); an LCE → the `NV_CE` register
/// block at `0x104000` (`ogkm-580` `dev_ce.h`: `NV_CE_PCE_MAP 0x00104028` on GA100/GA102,
/// `NV_CE_GRCE_MASK 0x001040d8` on GB202 — one block all LCEs share); `SW` → not a device.
/// ⚠ Turing/Ada/Hopper `dev_ce.h` in the tree carry no register to witness the block base; the
/// rule is stated for them on the Ampere and Blackwell witnesses.
///
/// Leaks the row once (`DeviceInfoRow` holds `&'static`) — call at realize, once per device.
#[must_use]
pub fn device_info_rule(engines: &[EngineKind], falcons: &[ConstructedFalcon]) -> DeviceInfoRow {
    const NV_PGRAPH: u32 = 0x0040_0000;
    const NV_CE_BLOCK: u32 = 0x0010_4000;
    let rows: Vec<EnginePriBase> = engines
        .iter()
        .map(|&k| EnginePriBase {
            engine: Box::leak(k.name().into_boxed_str()),
            pri_base: match k {
                EngineKind::Graphics(_) => DevicePriBase::At(NV_PGRAPH),
                EngineKind::Copy(_) => DevicePriBase::At(NV_CE_BLOCK),
                // ★ A video engine's PRI block is its falcon's: the host's own `registerBase` for
                // the same `engDesc` ([`query_video_falcons`]; `[measured]` GA106 NVENC0 `0x1c8000`,
                // NVDEC0 `0x848000`). A kind with no falcon never reaches here — filtered at query.
                EngineKind::VideoEncode(_) | EngineKind::VideoDecode(_) => falcons
                    .iter()
                    .find(|f| Some(f.eng_desc) == video_eng_desc(k))
                    .map_or(DevicePriBase::NotADevice, |f| DevicePriBase::At(f.register_base)),
                EngineKind::Software => DevicePriBase::NotADevice,
            },
        })
        .collect();
    DeviceInfoRow { pri_bases: Box::leak(rows.into_boxed_slice()) }
}

/// `ENG_NVENC(i)` / `ENG_NVDEC(i)` — `(NVOC classId << 8) | i` (`OBJMSENC 0xe97b6c`, `OBJBSP
/// 0x8f99e1`, `ogkm-580: g_eng_desc_nvoc.h:1273,833`); `None` for a non-video kind.
#[must_use]
pub fn video_eng_desc(k: EngineKind) -> Option<u32> {
    match k {
        EngineKind::VideoEncode(i) => Some(0x00e9_7b6c << 8 | i),
        EngineKind::VideoDecode(i) => Some(0x008f_99e1 << 8 | i),
        _ => None,
    }
}

/// ★★ **The video falcons, from the HOST's own falcon table** —
/// `NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO` (`0x208001b0`, `NON_PRIVILEGED`,
/// `ogkm-580: g_subdevice_nvoc.c`), kept to the `engDesc`s of the video kinds in `kinds`.
///
/// Why only those: an entry is an instruction to construct a `GenericKernelFalcon`, and for a
/// video `engDesc` RM then registers its NON-STALL service keyed by the FIFO table's MC index
/// (`kernel_falcon.c:362-397`, asserting the row exists) — nothing else. ⊘ No BAR0 register of the
/// falcon is ever read by a GSP-client guest: `gkflcnResetHw` refuses (`:356-360`), the register
/// HALs are reached only from KernelGsp (`kernel_gsp.c:2280`), and a generic falcon has no
/// engstate hooks. So `registerBase` is served as the host states it (and becomes the device-info
/// PRI base), and no register model is needed. FECS/GPCCS/PMU/SEC2/OFA rows are dropped: SEC2
/// would register an interrupt service keyed by an engine this device does not list.
///
/// `ctxBufferSize`/`ctxAttr`/`addrSpaceList` are the host's: the guest sizes the (never-executed)
/// context buffer it promotes with them — the twin's real one is host RM's.
///
/// # Errors
/// [`FieldCause`] — the host's refusal, or a reply whose count overruns the table.
pub fn query_video_falcons(host: &mut dyn HostControls, kinds: &[EngineKind]) -> Result<Vec<ConstructedFalcon>, FieldCause> {
    use kf_abi::falconinfo as fi;
    let cmd = fi::NV2080_CTRL_CMD_GPU_GET_CONSTRUCTED_FALCON_INFO;
    let r = ask(host, cmd, zeroed(fi::FALCON_INFO_PARAMS_SIZE))?;
    let rows = fi::decode_constructed_falcon_info(&r)
        .map_err(|_| FieldCause::Reply(FactRefusal::Unservable { cmd, why: "falcon count overruns constructedFalconsTable[]" }))?;
    let wanted: Vec<u32> = kinds.iter().filter_map(|&k| video_eng_desc(k)).collect();
    Ok(rows.into_iter().filter(|f| wanted.contains(&f.eng_desc)).collect())
}

/// ★ `gss_replay` — ask the host every `kf_abi::gssreplay::ROWS` request (authored: zero but the
/// named input words) and keep what it wrote. A refused row is left out: never fatal.
pub fn query_gss_replay(host: &mut dyn HostControls) -> Vec<kf_abi::gssreplay::Answer> {
    let mut out = Vec::new();
    for row in kf_abi::gssreplay::ROWS {
        let mut p = row.request();
        match host.control(row.cmd, &mut p) {
            Ok(()) => out.push(kf_abi::gssreplay::Answer::from_host(*row, &p)),
            Err(e) => eprintln!("kf3: host facts: GSS {:#010x} {:x?} refused by the host ({e:?}) — not served", row.cmd, row.inputs),
        }
    }
    out
}

/// ★ `video_caps` — the host's `MSENC_GET_CAPS_V2` (instance 0; the id is documented ignored) and
/// `BSP_GET_CAPS_V2` for every advertised decoder instance, asked on the host DEVICE with requests
/// we author (`kf_abi::videocaps`). A refused one is left out (the guest's is then refused).
pub fn query_video_caps(host: &mut dyn HostControls, kinds: &[EngineKind]) -> Vec<kf_abi::videocaps::CapsAnswer> {
    use kf_abi::videocaps as vc;
    let mut asks: Vec<(u32, u32)> = Vec::new();
    if kinds.iter().any(|k| matches!(k, EngineKind::VideoEncode(_))) {
        asks.push((vc::MSENC_GET_CAPS_V2, 0));
    }
    for k in kinds {
        if let EngineKind::VideoDecode(i) = k {
            asks.push((vc::BSP_GET_CAPS_V2, *i));
        }
    }
    let mut out = Vec::new();
    for (cmd, instance) in asks {
        let mut p = vc::host_request(instance);
        match host.device_control(cmd, &mut p) {
            Ok(()) => {
                let n = vc::caps_len(cmd).unwrap_or(0);
                out.push(vc::CapsAnswer { cmd, instance, caps: p[..n].to_vec() });
            }
            Err(e) => eprintln!("kf3: host facts: {cmd:#x} instance {instance} refused by the host ({e:?}) — not served"),
        }
    }
    out
}

/// `lce_pce_masks` — `CE_GET_CE_PCE_MASK` for LCE 0, 1, … until the host refuses one (an absent
/// LCE is refused, `kf_abi::cepce`), over both NV2080 copy decades.
///
/// # Errors
/// [`FieldCause`].
pub fn query_lce_pce_masks(host: &mut dyn HostControls) -> Result<Vec<u32>, FieldCause> {
    let cmd = kf_abi::cepce::NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK;
    let mut replies: Vec<Option<Vec<u8>>> = Vec::new();
    for i in 0..kf_abi::submit::RM_ENGINE_TYPE_COPY_SIZE {
        let engine_type = if i < 10 {
            kf_abi::submit::ENGINE_TYPE_COPY0 + i
        } else {
            kf_abi::submit::ENGINE_TYPE_COPY10 + i - 10
        };
        let mut req = zeroed(kf_abi::cepce::CE_GET_CE_PCE_MASK_PARAMS_SIZE);
        put32(&mut req, 0, engine_type);
        match host.control(cmd, &mut req) {
            Ok(()) => replies.push(Some(req)),
            // ⊘ LCE0 refused is not "no copy engines": it is the host saying no, and its words
            // are the finding.
            Err(refused) if i == 0 => return Err(FieldCause::Host { cmd, refused }),
            Err(_) => {
                replies.push(None);
                break;
            }
        }
    }
    let view: Vec<Option<&[u8]>> = replies.iter().map(|r| r.as_deref()).collect();
    Ok(hostfacts::derive_lce_pce_masks(&view)?)
}

// =====================================================================================
// Interrupts, chip info, memory
// =====================================================================================

/// `intr_table` — the host's static table and engine notification vectors, plus the GSP and
/// DISP stall rows of the device we present ([`authored::with_gsp_and_disp_rows`]).
///
/// # Errors
/// [`FieldCause`] — including a host row already on the vector our GSP raises.
pub fn query_intr_table(
    host: &mut dyn HostControls,
    kinds: Result<&[authored::EngineKind], &FieldCause>,
    grce_mask: u64,
) -> Result<Vec<kf_abi::inittables::IntrTableEntry>, FieldCause> {
    let s = ask(host, hostfacts::NV2080_CTRL_CMD_MC_GET_STATIC_INTR_TABLE, zeroed(hostfacts::MC_STATIC_INTR_TABLE_PARAMS_SIZE))?;
    let kinds = kinds.map_err(|_| FieldCause::DependsOn("engines"))?;
    let mut table = hostfacts::derive_static_intr_table(&s)?;
    table.extend(authored::engine_notification_rows(kinds, grce_mask));
    authored::with_gsp_and_disp_rows(table).map_err(|_| {
        FieldCause::Reply(FactRefusal::Unservable {
            cmd: hostfacts::NV2080_CTRL_CMD_MC_GET_STATIC_INTR_TABLE,
            why: "a host interrupt row already uses the GSP or DISP vector this device presents",
        })
    })
}

/// `intr_subtree_map`.
///
/// # Errors
/// [`FieldCause`].
pub fn query_intr_subtree_map(
    host: &mut dyn HostControls,
) -> Result<[u64; kf_abi::inittables::INTR_CATEGORY_COUNT], FieldCause> {
    let r = ask(
        host,
        hostfacts::NV2080_CTRL_CMD_MC_GET_INTR_CATEGORY_SUBTREE_MAP,
        zeroed(hostfacts::MC_SUBTREE_MAP_PARAMS_SIZE),
    )?;
    Ok(hostfacts::derive_intr_subtree_map(&r)?)
}

/// One `GPU_GET_INFO_V2` index, asked alone (one refused index fails the whole call —
/// `kf_abi::smcmode` — so each field asks its own).
fn ask_gpu_info(host: &mut dyn HostControls, index: u32) -> Result<Vec<u8>, FieldCause> {
    use kf_abi::gpuinfo as g;
    ask(host, g::NV2080_CTRL_CMD_GPU_GET_INFO_V2, info_list_request(g::GPU_GET_INFO_V2_PARAMS_SIZE, &[index]))
}

/// ★ `USERMODE`'s register base: `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` = `0xB80000`
/// plus `NV_VIRTUAL_FUNCTION` = `0x30000` — the same on every family this port serves
/// (`ogkm-580` `dev_vm.h`: `turing/tu102:27-28`, `hopper/gh100:27` (`NV_VIRTUAL_FUNCTION`),
/// `blackwell/gb100:31,39`), and the page `kf_trap::memmap::VF_USERMODE_PAGE` presents.
pub const USERMODE_REG_BASE: u32 = 0x00B8_0000 + 0x0003_0000;

static REG_BASES: &[RegBaseRow] = &[RegBaseRow {
    index: reg_base::USERMODE,
    offset: USERMODE_REG_BASE,
    name: "NV_VIRTUAL_FUNCTION (usermode work submission)",
}];

/// `chip_info` — the family rule for the register bases, the sub-revision the host's arch info
/// names, and `isCmpSku` from `GPU_GET_INFO_V2[CMP_SKU]`.
///
/// # Errors
/// [`FieldCause`].
pub fn query_chip_info(host: &mut dyn HostControls, sub_revision: u8) -> Result<ChipInfoRow, FieldCause> {
    let cmp = hostfacts::derive_cmp_sku(&ask_gpu_info(host, hostfacts::GPU_INFO_INDEX_CMP_SKU)?)?;
    Ok(ChipInfoRow { chip_sub_rev: sub_revision, is_cmp_sku: cmp, reg_bases: REG_BASES })
}

/// `NV2080_CTRL_GR_INFO_INDEX_LITTER_NUM_SLICES_PER_LTC` (`ogkm-580: ctrl0080gr.h`, `0x32`).
pub const GR_INFO_IDX_LITTER_NUM_SLICES_PER_LTC: usize = 0x32;

/// `gr_info` — the whole `GR_GET_INFO_V2` table, asked in index order on GR0.
///
/// # Errors
/// [`FieldCause`].
pub fn query_gr_info(host: &mut dyn HostControls) -> Result<GrInfoProfile, FieldCause> {
    use kf_abi::grinfo::GR_INFO_MAX_SIZE;
    let indices: Vec<u32> = (0..GR_INFO_MAX_SIZE as u32).collect();
    // The route at GR_GET_INFO_V2_ROUTE_OFF stays zero: TYPE_NONE, GR0.
    let req = info_list_request(hostfacts::GR_GET_INFO_V2_PARAMS_SIZE, &indices);
    Ok(hostfacts::derive_gr_info(&ask(host, hostfacts::NV2080_CTRL_CMD_GR_GET_INFO_V2, req)?)?)
}

/// `memory_system` — L2 size, RAM type and LTC count from `FB_GET_INFO_V2`; slices per LTC
/// from `gr_info`'s `LITTER_NUM_SLICES_PER_LTC`; the rest AUTHORED: raw-mode comptags (the
/// mode in which RM expects no comptag allocator behind the reply — `kf_abi::memsysconfig`),
/// a 64 KiB compression page (the big-page size raw mode is stated over), FBPA present, and
/// ECC / L2-prefill / compbit-backing / post-L2-compression flags off.
///
/// # Errors
/// [`FieldCause`].
pub fn query_memory_system(host: &mut dyn HostControls, gr_info: Option<&GrInfoProfile>) -> Result<MemorySystemRow, FieldCause> {
    use kf_abi::fbinfo as fb;
    let req = info_list_request(
        fb::FB_GET_INFO_V2_PARAMS_SIZE,
        &[fb::FB_INFO_INDEX_L2CACHE_SIZE, fb::FB_INFO_INDEX_RAM_TYPE, fb::FB_INFO_INDEX_LTC_COUNT],
    );
    let r = ask(host, fb::NV2080_CTRL_CMD_FB_GET_INFO_V2, req)?;
    let pairs = fb::decode_fb_info_pairs(&r)
        .map_err(|_| FactRefusal::ShortReply { cmd: fb::NV2080_CTRL_CMD_FB_GET_INFO_V2, len: r.len() })?;
    let g = hostfacts::derive_fb_geometry(&pairs)?;
    let lts = gr_info.ok_or(FieldCause::DependsOn("gr_info"))?.data[GR_INFO_IDX_LITTER_NUM_SLICES_PER_LTC];
    Ok(MemorySystemRow {
        comptag_policy: ComptagAllocationPolicy::Raw,
        disable_compbit_backing: false,
        disable_post_l2_compression: false,
        ecc_fbpa_enabled: false,
        l2_prefill: false,
        l2_cache_size: g.l2_cache_size,
        fbpa_present: true,
        compr_page_size: 0x0001_0000,
        ram_type: g.ram_type,
        ltc_count: g.ltc_count,
        lts_per_ltc_count: lts,
    })
}

// =====================================================================================
// GR
// =====================================================================================

/// What the host says about GR's geometry — every host-sourced input of
/// `kf_abi::grstatic::GrStaticProfile`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrGeometry {
    /// `GR_GET_GPC_MASK`.
    pub gpc_mask: u32,
    /// `(gpcId, tpcMask)` for every set bit of `gpc_mask`, in id order (`GR_GET_TPC_MASK`).
    pub tpc_masks: Vec<(u32, u32)>,
    /// `zcullMask` per GPC, in the same order (`GR_GET_ZCULL_MASK 0x20801237`, NON_PRIVILEGED; it
    /// reads the same `floorsweepingMasks.zcullMask[]` the internal control carries,
    /// `kernel_graphics.c:3808-3822`). The host's `NV_ERR_NOT_SUPPORTED` — RM's answer for a
    /// `NV_U32_MAX` mask (`:3817-3819`) — is carried as `u32::MAX`.
    pub zcull_masks: Vec<u32>,
    /// The TPC rows in `globalTpcId` order (`GR_GET_GLOBAL_SM_ORDER`).
    pub tpcs: Vec<TpcRow>,
    /// SMs per TPC (`GR_GET_GLOBAL_SM_ORDER`, cross-checked against `gr_info`).
    pub sms_per_tpc: u16,
    /// The caps table (`GR_GET_CAPS_V2`).
    pub caps: [u8; kf_abi::grstatic::GR_CAPS_TBL_SIZE],
}

/// `NV2080_CTRL_CMD_GR_GET_ZCULL_MASK`.
pub const NV2080_CTRL_CMD_GR_GET_ZCULL_MASK: u32 = 0x2080_1237;
/// `NV2080_CTRL_GR_INFO_INDEX_LITTER_NUM_TPC_PER_GPC`.
pub const GR_INFO_IDX_LITTER_NUM_TPC_PER_GPC: usize = 0x17;
/// `NV2080_CTRL_GR_INFO_INDEX_LITTER_NUM_PES_PER_GPC`.
pub const GR_INFO_IDX_LITTER_NUM_PES_PER_GPC: usize = 0x1c;
/// `NV2080_CTRL_GR_INFO_INDEX_LITTER_NUM_TPCS_PER_PES`.
pub const GR_INFO_IDX_LITTER_NUM_TPCS_PER_PES: usize = 0x1e;
/// `NV2080_CTRL_GR_INFO_INDEX_LITTER_NUM_GPCMMU_PER_GPC`.
pub const GR_INFO_IDX_LITTER_NUM_GPCMMU_PER_GPC: usize = 0x27;

/// ★ The host-sourced inputs of `gr_static`.
///
/// # Errors
/// [`FieldCause`]; the SMs-per-TPC cross-check against `gr_info` is a refusal, not a pick.
pub fn query_gr_geometry(host: &mut dyn HostControls, gr_info: Option<&GrInfoProfile>) -> Result<GrGeometry, FieldCause> {
    let gpc_mask = hostfacts::derive_gpc_mask(&ask(
        host,
        hostfacts::NV2080_CTRL_CMD_GR_GET_GPC_MASK,
        zeroed(hostfacts::GR_MASK_PARAMS_SIZE),
    )?)?;
    let mut tpc_masks = Vec::new();
    let mut zcull_masks = Vec::new();
    for gpc in (0..32).filter(|b| gpc_mask & (1 << b) != 0) {
        let mut req = zeroed(hostfacts::GR_MASK_PARAMS_SIZE);
        put32(&mut req, hostfacts::GR_ROUTE_INFO_SIZE, gpc);
        let r = ask(host, hostfacts::NV2080_CTRL_CMD_GR_GET_TPC_MASK, req)?;
        tpc_masks.push((gpc, hostfacts::derive_tpc_mask(&r, gpc)?));
        let mut z = zeroed(8);
        put32(&mut z, 0, gpc);
        zcull_masks.push(match host.control(NV2080_CTRL_CMD_GR_GET_ZCULL_MASK, &mut z) {
            Ok(()) => u32::from_le_bytes([z[4], z[5], z[6], z[7]]),
            Err(HostRefusal { status: Some(NV_ERR_NOT_SUPPORTED), .. }) => u32::MAX,
            Err(refused) => return Err(FieldCause::Host { cmd: NV2080_CTRL_CMD_GR_GET_ZCULL_MASK, refused }),
        });
    }
    let (tpcs, sms_per_tpc) = hostfacts::derive_sm_order(&ask(
        host,
        hostfacts::NV2080_CTRL_CMD_GR_GET_GLOBAL_SM_ORDER,
        zeroed(hostfacts::GR_GLOBAL_SM_ORDER_PARAMS_SIZE),
    )?)?;
    let caps = hostfacts::derive_gr_caps(&ask(
        host,
        hostfacts::NV2080_CTRL_CMD_GR_GET_CAPS_V2,
        zeroed(hostfacts::GR_CAPS_V2_PARAMS_SIZE),
    )?)?;
    let info = gr_info.ok_or(FieldCause::DependsOn("gr_info"))?;
    if info.data[kf_abi::grinfo::IDX_LITTER_NUM_SM_PER_TPC] != u32::from(sms_per_tpc) {
        return Err(FieldCause::Reply(FactRefusal::Unservable {
            cmd: hostfacts::NV2080_CTRL_CMD_GR_GET_GLOBAL_SM_ORDER,
            why: "SMs per TPC disagrees with GR_GET_INFO_V2's LITTER_NUM_SM_PER_TPC",
        }));
    }
    let tpc_total: u32 = tpc_masks.iter().map(|(_, m)| m.count_ones()).sum();
    if tpc_total as usize != tpcs.len() {
        return Err(FieldCause::Reply(FactRefusal::Unservable {
            cmd: hostfacts::NV2080_CTRL_CMD_GR_GET_TPC_MASK,
            why: "the TPC masks' population disagrees with the SM order's TPC count",
        }));
    }
    Ok(GrGeometry { gpc_mask, tpc_masks, zcull_masks, tpcs, sms_per_tpc, caps })
}

/// ★ v3-gfx: `zbc_table_sizes` — `GET_ZBC_CLEAR_TABLE_SIZE` for each table type. The composition
/// root routes `0x9096xxxx` to a host `GF100_ZBC_CLEAR` object (`kf-qemu` `rmfacts.rs`); the host's
/// `NV_ERR_NOT_SUPPORTED` means the die has none (`None`).
///
/// # Errors
/// [`FieldCause`] — any other refusal, or an empty/inverted range.
pub fn query_zbc_table_sizes(host: &mut dyn HostControls) -> Result<Option<[(u32, u32); 3]>, FieldCause> {
    use kf_abi::zbc as z;
    let mut out = [(0, 0); 3];
    for t in z::TableType::ALL {
        let mut p = zeroed(z::GET_SIZE_PARAMS_SIZE);
        z::put(&mut p, 2, t.wire());
        match host.control(z::GET_ZBC_CLEAR_TABLE_SIZE, &mut p) {
            Ok(()) => {}
            Err(HostRefusal { status: Some(NV_ERR_NOT_SUPPORTED), .. }) => return Ok(None),
            Err(refused) => return Err(FieldCause::Host { cmd: z::GET_ZBC_CLEAR_TABLE_SIZE, refused }),
        }
        let (start, end) = (z::word(&p, 0).unwrap_or(0), z::word(&p, 1).unwrap_or(0));
        if start == 0 || end < start {
            return Err(FieldCause::Reply(FactRefusal::Unservable {
                cmd: z::GET_ZBC_CLEAR_TABLE_SIZE,
                why: "an empty or inverted ZBC index range (index 0 is reserved by RM)",
            }));
        }
        out[t.wire() as usize - 1] = (start, end);
    }
    Ok(Some(out))
}

/// ★ v3-gfx: the extra `FB_GET_INFO_V2` indices [`HostFacts::forwarded_fb_extra`] carries.
pub const FORWARDED_FB_EXTRA_INDICES: [u32; 5] = [0x04, 0x14, 0x37, 0x2b, 0x38];

/// ★ v3-gfx: `forwarded_fb_extra` — each of [`FORWARDED_FB_EXTRA_INDICES`] asked ALONE (one refused index
/// must not take the others with it); a refused index is simply absent.
///
/// # Errors
/// Never today — kept fallible so a decode failure can become a named refusal.
pub fn query_forwarded_fb_extra(host: &mut dyn HostControls) -> Result<Vec<(u32, u32)>, FieldCause> {
    use kf_abi::fbinfo as fb;
    let mut out = Vec::new();
    for idx in FORWARDED_FB_EXTRA_INDICES {
        let mut req = info_list_request(fb::FB_GET_INFO_V2_PARAMS_SIZE, &[idx]);
        if host.control(fb::NV2080_CTRL_CMD_FB_GET_INFO_V2, &mut req).is_ok()
            && let Ok(pairs) = fb::decode_fb_info_pairs(&req)
            && let Some(&(i, d)) = pairs.first()
            && i == idx
        {
            out.push((i, d));
        }
    }
    Ok(out)
}

/// `NV2080_CTRL_CMD_FB_GET_GPU_CACHE_INFO` (flags `0x40148`: NON_PRIVILEGED, ROUTE_TO_PHYSICAL).
pub const NV2080_CTRL_CMD_FB_GET_GPU_CACHE_INFO: u32 = 0x2080_1315;

/// ★ v3-gfx: `gpu_cache_info` — the host's four L2 state words; any refusal is `None`.
///
/// # Errors
/// Never today.
pub fn query_gpu_cache_info(host: &mut dyn HostControls) -> Result<Option<[u32; 4]>, FieldCause> {
    let mut p = zeroed(16);
    Ok(host.control(NV2080_CTRL_CMD_FB_GET_GPU_CACHE_INFO, &mut p).ok().map(|()| {
        let w = |i: usize| u32::from_le_bytes([p[4 * i], p[4 * i + 1], p[4 * i + 2], p[4 * i + 3]]);
        [w(0), w(1), w(2), w(3)]
    }))
}

/// `NV2080_CTRL_CMD_GR_GET_ZCULL_INFO` — the unprivileged client control (flags `0x10109`,
/// `g_subdevice_nvoc.c`), 40 bytes, all `[OUT]`.
pub const NV2080_CTRL_CMD_GR_GET_ZCULL_INFO: u32 = 0x2080_1206;

/// ★ v3-gfx: `gr_zcull_info` — the host's own zcull geometry (`GR_GET_ZCULL_INFO`). The host's
/// `NV_ERR_NOT_SUPPORTED` — RM's answer when its cache is empty (`kernel_graphics.c:3862-3863`),
/// i.e. a die with no zcull — is carried as `None`, and the guest's internal control is then
/// refused exactly as that die's own GSP would.
///
/// # Errors
/// [`FieldCause`] — any other refusal.
pub fn query_gr_zcull_info(
    host: &mut dyn HostControls,
) -> Result<Option<[u32; kf_abi::grstatic::ZCULL_INFO_ROW_WORDS]>, FieldCause> {
    let mut z = zeroed(4 * kf_abi::grstatic::ZCULL_INFO_ROW_WORDS);
    match host.control(NV2080_CTRL_CMD_GR_GET_ZCULL_INFO, &mut z) {
        Ok(()) => {
            let mut row = [0u32; kf_abi::grstatic::ZCULL_INFO_ROW_WORDS];
            for (i, w) in row.iter_mut().enumerate() {
                *w = u32::from_le_bytes([z[4 * i], z[4 * i + 1], z[4 * i + 2], z[4 * i + 3]]);
            }
            Ok(Some(row))
        }
        Err(HostRefusal { status: Some(NV_ERR_NOT_SUPPORTED), .. }) => Ok(None),
        Err(refused) => Err(FieldCause::Host { cmd: NV2080_CTRL_CMD_GR_GET_ZCULL_INFO, refused }),
    }
}

/// ★★ `gr_static` from the host geometry, the host's GR litters, and the authored members:
///
/// | member | source |
/// |---|---|
/// | GPC rows' `tpc_mask` / `tpc_count` | host TPC masks (count = population) |
/// | `mmu_per_gpc` | host `LITTER_NUM_GPCMMU_PER_GPC` |
/// | `num_pes_per_gpc` | host `LITTER_NUM_PES_PER_GPC` |
/// | `zcull_mask` | host `GR_GET_ZCULL_MASK` |
/// | `tpcs`, `sms_per_tpc` | host SM order |
/// | `tpc_to_pes_map` | [`authored::tpc_to_pes_map`] over host litters |
/// | `caps` | host caps |
/// | `fecs_record_size` | [`authored::FECS_RECORD_SIZE`] |
/// | `per_subctx_header_supported` | [`authored::PER_SUBCTX_HEADER_SUPPORTED`] |
///
/// Leaks the GPC and TPC rows once (`&'static`) — realize, once per device.
///
/// # Errors
/// [`FieldCause`] — a non-contiguous GPC mask (the profile states GPCs by count), a litter
/// the TPC-to-PES rule cannot use, or a profile `kf_abi` refuses to encode.
pub fn gr_static_from(g: &GrGeometry, info: &GrInfoProfile) -> Result<kf_abi::grstatic::GrStaticProfile, FieldCause> {
    use kf_abi::grstatic::{GpcRow, GrStaticProfile};
    let cmd = hostfacts::NV2080_CTRL_CMD_GR_GET_GPC_MASK;
    if g.gpc_mask & g.gpc_mask.wrapping_add(1) != 0 {
        return Err(FieldCause::Reply(FactRefusal::Unservable { cmd, why: "a non-contiguous GPC mask: GrStaticProfile states GPCs 0..n" }));
    }
    let gpcs: Vec<GpcRow> = g
        .tpc_masks
        .iter()
        .zip(&g.zcull_masks)
        .map(|(&(_, tpc_mask), &zcull_mask)| GpcRow {
            tpc_mask,
            tpc_count: tpc_mask.count_ones(),
            mmu_per_gpc: info.data[GR_INFO_IDX_LITTER_NUM_GPCMMU_PER_GPC],
            num_pes_per_gpc: info.data[GR_INFO_IDX_LITTER_NUM_PES_PER_GPC],
            zcull_mask,
        })
        .collect();
    let tpc_to_pes_map = authored::tpc_to_pes_map(
        info.data[GR_INFO_IDX_LITTER_NUM_TPC_PER_GPC],
        info.data[GR_INFO_IDX_LITTER_NUM_TPCS_PER_PES],
    )
    .ok_or(FieldCause::Reply(FactRefusal::Unservable {
        cmd: hostfacts::NV2080_CTRL_CMD_GR_GET_INFO_V2,
        why: "LITTER_NUM_TPCS_PER_PES is zero or LITTER_NUM_TPC_PER_GPC exceeds MAX_TPC_PER_GPC",
    }))?;
    let p = GrStaticProfile {
        gpcs: Box::leak(gpcs.into_boxed_slice()),
        tpcs: Box::leak(g.tpcs.clone().into_boxed_slice()),
        sms_per_tpc: g.sms_per_tpc,
        tpc_to_pes_map,
        caps: g.caps,
        fecs_record_size: authored::FECS_RECORD_SIZE,
        per_subctx_header_supported: authored::PER_SUBCTX_HEADER_SUPPORTED,
    };
    p.validate()
        .map_err(|_| FieldCause::Reply(FactRefusal::Unservable { cmd, why: "the GR profile fails kf_abi's own validation" }))?;
    Ok(p)
}

/// `gr_context_buffers` — `GR_GET_ENGINE_CONTEXT_PROPERTIES` for every id. The host's
/// `NV_ERR_NOT_SUPPORTED` for an id is RM's own "absent" (`derive_context_buffer`).
///
/// # Errors
/// [`FieldCause`] — any other refusal.
pub fn query_gr_context_buffers(
    host: &mut dyn HostControls,
) -> Result<[ContextBuffer; CONTEXT_BUFFER_ID_COUNT], FieldCause> {
    let cmd = hostfacts::NV2080_CTRL_CMD_GR_GET_ENGINE_CONTEXT_PROPERTIES;
    let mut out = [ContextBuffer { size: 0, alignment: 0 }; CONTEXT_BUFFER_ID_COUNT];
    for (id, slot) in out.iter_mut().enumerate() {
        let mut req = zeroed(hostfacts::GR_CONTEXT_PROPERTIES_PARAMS_SIZE);
        put32(&mut req, hostfacts::GR_ROUTE_INFO_SIZE, id as u32);
        *slot = match host.control(cmd, &mut req) {
            Ok(()) => hostfacts::derive_context_buffer(Some(&req))?,
            Err(HostRefusal { status: Some(NV_ERR_NOT_SUPPORTED), .. }) => hostfacts::derive_context_buffer(None)?,
            Err(refused) => return Err(FieldCause::Host { cmd, refused }),
        };
    }
    Ok(out)
}

// =====================================================================================
// The single-control fields
// =====================================================================================

/// ★ The `GPU_GET_INFO_V2` indices the guest kernel FORWARDS to GSP-RM, so ours to answer
/// (`kf_abi::gpuinfo`): `0x11` is the one established. Driver-version axis, not per-die: the
/// VALUE is the host's.
pub const FORWARDED_GPU_INFO_INDICES: &[u32] = &[0x11];

/// `forwarded_gpu_info` — the host's own answers for [`FORWARDED_GPU_INFO_INDICES`].
///
/// # Errors
/// [`FieldCause`].
pub fn query_forwarded_gpu_info(host: &mut dyn HostControls) -> Result<Vec<(u32, u32)>, FieldCause> {
    let mut out = Vec::new();
    for &index in FORWARDED_GPU_INFO_INDICES {
        out.push((index, hostfacts::derive_gpu_info_value(&ask_gpu_info(host, index)?, index)?));
    }
    Ok(out)
}

/// ★ The `FB_GET_INFO_V2` indices a guest's `FB_GET_INFO_V2` RPC may carry
/// (`kf_abi::fbinfo::answer_fb_get_info_v2`'s set), asked of the host in one call.
pub const FORWARDED_FB_INFO_INDICES: &[u32] = &[
    kf_abi::fbinfo::FB_INFO_INDEX_BUS_WIDTH,
    kf_abi::fbinfo::FB_INFO_INDEX_RAM_TYPE,
    kf_abi::fbinfo::FB_INFO_INDEX_FBP_COUNT,
    kf_abi::fbinfo::FB_INFO_INDEX_FBP_MASK,
    kf_abi::fbinfo::FB_INFO_INDEX_L2CACHE_SIZE,
    kf_abi::fbinfo::FB_INFO_INDEX_LTC_COUNT,
    kf_abi::fbinfo::FB_INFO_INDEX_LTS_COUNT,
];

/// `forwarded_fb_info` — the host's own words for [`FORWARDED_FB_INFO_INDICES`]. ⊘ Every index
/// must be answered: a missing one is refused by name, never a zero.
///
/// # Errors
/// [`FieldCause`].
pub fn query_forwarded_fb_info(host: &mut dyn HostControls) -> Result<Vec<(u32, u32)>, FieldCause> {
    use kf_abi::fbinfo as fb;
    let req = info_list_request(fb::FB_GET_INFO_V2_PARAMS_SIZE, FORWARDED_FB_INFO_INDICES);
    let r = ask(host, fb::NV2080_CTRL_CMD_FB_GET_INFO_V2, req)?;
    let pairs = fb::decode_fb_info_pairs(&r)
        .map_err(|_| FactRefusal::ShortReply { cmd: fb::NV2080_CTRL_CMD_FB_GET_INFO_V2, len: r.len() })?;
    let mut out = Vec::new();
    for &index in FORWARDED_FB_INFO_INDICES {
        let d = pairs
            .iter()
            .find(|(i, _)| *i == index)
            .map(|(_, d)| *d)
            .ok_or(FactRefusal::Missing { cmd: fb::NV2080_CTRL_CMD_FB_GET_INFO_V2, index })?;
        out.push((index, d));
    }
    Ok(out)
}

/// `smc_mode`.
///
/// # Errors
/// [`FieldCause`].
pub fn query_smc_mode(host: &mut dyn HostControls) -> Result<kf_abi::smcmode::SmcMode, FieldCause> {
    Ok(hostfacts::derive_smc_mode(&ask_gpu_info(host, hostfacts::GPU_INFO_INDEX_GPU_SMC_MODE)?)?)
}

/// `pcie_max_gen`.
///
/// # Errors
/// [`FieldCause`].
pub fn query_pcie_max_gen(host: &mut dyn HostControls) -> Result<kf_abi::businfo::PcieGen, FieldCause> {
    use kf_abi::businfo as bus;
    let req = info_list_request(bus::BUS_GET_INFO_V2_PARAMS_SIZE, &[bus::BUS_INFO_INDEX_PCIE_GEN_INFO]);
    Ok(hostfacts::derive_pcie_max_gen(&ask(host, bus::NV2080_CTRL_CMD_BUS_GET_INFO_V2, req)?)?)
}

/// `gsp_features`.
///
/// # Errors
/// [`FieldCause`].
pub fn query_gsp_features(host: &mut dyn HostControls) -> Result<kf_abi::gspfeatures::GspFeatures, FieldCause> {
    use kf_abi::gspfeatures as g;
    let r = ask(host, g::NV2080_CTRL_CMD_GSP_GET_FEATURES, zeroed(g::GSP_GET_FEATURES_PARAMS_SIZE))?;
    Ok(hostfacts::derive_gsp_features(&r)?)
}

/// `NV2080_CTRL_CMD_GPU_GET_NAME_STRING` — `gpuNameStringFlags` (0 = ASCII) + a 64-`NvU16` union.
pub const NV2080_CTRL_CMD_GPU_GET_NAME_STRING: u32 = 0x2080_0110;
/// `sizeof(NV2080_CTRL_GPU_GET_NAME_STRING_PARAMS)`. `[measured]` 132 on a real GA106.
pub const GPU_GET_NAME_STRING_PARAMS_SIZE: usize = 4 + 2 * 64;
/// `NV2080_CTRL_CMD_GPU_GET_SHORT_NAME_STRING` — 64 bytes.
pub const NV2080_CTRL_CMD_GPU_GET_SHORT_NAME_STRING: u32 = 0x2080_0111;
/// `sizeof(NV2080_CTRL_GPU_GET_SHORT_NAME_STRING_PARAMS)`. `[measured]` 64.
pub const GPU_GET_SHORT_NAME_STRING_PARAMS_SIZE: usize = 64;

/// `gpu_name` (ASCII, flags 0).
///
/// # Errors
/// [`FieldCause`].
pub fn query_gpu_name(host: &mut dyn HostControls) -> Result<kf_abi::gspstaticinfo::GpuName, FieldCause> {
    let cmd = NV2080_CTRL_CMD_GPU_GET_NAME_STRING;
    let r = ask(host, cmd, zeroed(GPU_GET_NAME_STRING_PARAMS_SIZE))?;
    Ok(hostfacts::derive_gpu_name(cmd, &r, 4)?)
}

/// `gpu_short_name`.
///
/// # Errors
/// [`FieldCause`].
pub fn query_gpu_short_name(host: &mut dyn HostControls) -> Result<kf_abi::gspstaticinfo::GpuName, FieldCause> {
    let cmd = NV2080_CTRL_CMD_GPU_GET_SHORT_NAME_STRING;
    let r = ask(host, cmd, zeroed(GPU_GET_SHORT_NAME_STRING_PARAMS_SIZE))?;
    Ok(hostfacts::derive_gpu_name(cmd, &r, 0)?)
}

/// `has_c2c`.
///
/// # Errors
/// [`FieldCause`].
pub fn query_has_c2c(host: &mut dyn HostControls) -> Result<bool, FieldCause> {
    use kf_abi::c2cinfo as c;
    let r = ask(host, c::NV2080_CTRL_CMD_BUS_GET_C2C_INFO, zeroed(c::C2C_INFO_PARAMS_SIZE))?;
    Ok(hostfacts::derive_has_c2c(&r)?)
}

/// ★ `ce_caps` — the host die's own `CE_GET_ALL_CAPS` (`0x20802a0a`, NON_PRIVILEGED; `[OUT]`
/// only, so a zeroed request).
///
/// # Errors
/// [`FieldCause`].
pub fn query_ce_caps(host: &mut dyn HostControls) -> Result<kf_abi::cecaps::HostCeCaps, FieldCause> {
    use kf_abi::cecaps as c;
    let r = ask(host, c::NV2080_CTRL_CMD_CE_GET_ALL_CAPS, zeroed(c::CE_GET_ALL_CAPS_PARAMS_SIZE))?;
    Ok(hostfacts::derive_ce_caps(&r)?)
}

/// ★ `vbios_version` — `BIOS_GET_INFO_V2 [REVISION, OEM_REVISION]` (`0x20800810`, NON_PRIVILEGED).
///
/// ⊘ **Cosmetic, so it never refuses realize** (coordinator, 2026-09-26): a host refusal OR a reply
/// that does not decode is `None` — the ROM then declares `kf_abi::vbios::NEUTRAL_VBIOS_VERSION` and
/// the guest's own ask is refused. The composition root logs the `None` by name.
///
/// # Errors
/// None today; the `Result` keeps the query's shape.
pub fn query_vbios_version(host: &mut dyn HostControls) -> Result<Option<(u32, u8)>, FieldCause> {
    let req = info_list_request(
        hostfacts::BIOS_GET_INFO_V2_PARAMS_SIZE,
        &[hostfacts::BIOS_INFO_INDEX_REVISION, hostfacts::BIOS_INFO_INDEX_OEM_REVISION],
    );
    Ok(ask(host, hostfacts::NV2080_CTRL_CMD_BIOS_GET_INFO_V2, req)
        .ok()
        .and_then(|r| hostfacts::derive_vbios_version(&r).ok()))
}

/// ★ `perf_level_info_v2` — libcudart's `PERF_GET_LEVEL_INFO_V2` question, asked of the host
/// once (`0x2080200b`, NON_PRIVILEGED). ⊘ A host REFUSAL is a value here (`None`), not a realize
/// failure: it is exactly what host userspace would be told, and the guest's identical ask is
/// then refused the same way. Only a reply that does not decode is a refusal of the field.
///
/// # Errors
/// [`FieldCause::Reply`] for a reply of the wrong length.
pub fn query_perf_level_info_v2(host: &mut dyn HostControls) -> Result<Option<Vec<u8>>, FieldCause> {
    use kf_abi::cudartinit as c;
    match ask(host, c::PERF_GET_LEVEL_INFO_V2, c::perf_level_info_v2_request()) {
        Ok(r) if r.len() == c::PERF_GET_LEVEL_INFO_V2_PARAMS_SIZE => Ok(Some(r)),
        Ok(r) => Err(FieldCause::Reply(FactRefusal::ShortReply { cmd: c::PERF_GET_LEVEL_INFO_V2, len: r.len() })),
        Err(FieldCause::Host { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

// =====================================================================================
// Authored
// =====================================================================================

/// `conf_compute` — AUTHORED: confidential compute off (neither BAR1 nor PCIe trusted).
pub const AUTHORED_CONF_COMPUTE: ConfComputeRow = ConfComputeRow { bar1_trusted: false, pcie_trusted: false };
/// `bif_static` — AUTHORED: no C2C, one PCI function, no Gen4 claim, no GCx restore
/// (`kf_abi::bifstatic`: every flag the encoder refuses to claim without a plane behind it).
pub const AUTHORED_BIF_STATIC: BifStaticRow = BifStaticRow {
    pcie_gen4_capable: false,
    c2c_link_up: false,
    device_multi_function: false,
    gcx_pmu_cfg_space_restore: false,
};
/// `fifo_channels` — AUTHORED: the channel count per runlist is ours to set.
pub const AUTHORED_FIFO_CHANNELS: FifoChannelsRow = FifoChannelsRow { channels_per_runlist: 0x0800 };

// =====================================================================================
// The whole struct
// =====================================================================================

/// ★★★ **Fill every [`HostFacts`] field from its [`PROVENANCE`](crate::hostfacts::PROVENANCE) source**, or refuse naming
/// every field that could not be filled.
///
/// `family` is the family realize already chose from the same host; the host's own
/// `MC_GET_ARCH_INFO` must name it too.
///
/// # Errors
/// [`HostFactsRefused`] — every refused field, in [`PROVENANCE`](crate::hostfacts::PROVENANCE) order.
pub fn query_host_facts(host: &mut dyn HostControls, family: Family) -> Result<HostFacts, HostFactsRefused> {
    let arch = query_arch(host);
    let asked = family;
    let family = match &arch {
        Ok((f, _)) if *f == asked => Ok(*f),
        Ok((f, _)) => Err(FieldCause::FamilyMismatch { asked, host: *f }),
        Err(e) => Err(e.clone()),
    };
    let has_c2c = query_has_c2c(host);
    let ce_caps = query_ce_caps(host);
    let grce = ce_caps.as_ref().map(kf_abi::cecaps::HostCeCaps::grce_mask);
    let kinds = query_engine_list(host);
    // ★ The video falcons, then the list kept to the video engines the host's falcon table names.
    // A refused falcon query is not fatal to the device: it advertises no video engine, loudly.
    let video_falcons: Vec<ConstructedFalcon> = match &kinds {
        Ok(k) if k.iter().any(|k| video_eng_desc(*k).is_some()) => match query_video_falcons(host, k) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("kf3: host facts: video engines NOT advertised — the host falcon table was refused: {e:?}");
                Vec::new()
            }
        },
        _ => Vec::new(),
    };
    let kinds = kinds.map(|k| {
        k.into_iter()
            .filter(|&k| video_eng_desc(k).is_none_or(|d| video_falcons.iter().any(|f| f.eng_desc == d)))
            .collect::<Vec<_>>()
    });
    let engines = match (&kinds, &grce) {
        (Ok(k), Ok(g)) => authored::engine_table(asked, k, *g).map_err(FieldCause::FamilyLayout),
        (Err(e), _) => Err(e.clone()),
        (Ok(_), Err(_)) => Err(FieldCause::DependsOn("ce_caps")),
    };
    let lce_pce_masks = query_lce_pce_masks(host);
    let intr_table = match &grce {
        Ok(g) => query_intr_table(host, kinds.as_deref().map_err(|e| e), *g),
        Err(_) => Err(FieldCause::DependsOn("ce_caps")),
    };
    let intr_subtree_map = query_intr_subtree_map(host);
    let chip_info = match &arch {
        Ok((_, sub)) => query_chip_info(host, *sub),
        Err(_) => Err(FieldCause::DependsOn("family")),
    };
    let gr_info = query_gr_info(host);
    let memory_system = query_memory_system(host, gr_info.as_ref().ok());
    let device_info = match &kinds {
        Ok(k) => Ok(device_info_rule(k, &video_falcons)),
        Err(_) => Err(FieldCause::DependsOn("engines")),
    };
    let gmmu_static: Result<kf_abi::gmmustatic::GmmuStaticRow, FieldCause> = Ok(authored::GMMU_STATIC);
    let gr_static = match (query_gr_geometry(host, gr_info.as_ref().ok()), &gr_info) {
        (Ok(g), Ok(info)) => gr_static_from(&g, info),
        (Err(e), _) => Err(e),
        (Ok(_), Err(_)) => Err(FieldCause::DependsOn("gr_info")),
    };
    let gr_context_buffers = query_gr_context_buffers(host);
    let gr_zcull_info = query_gr_zcull_info(host);
    let zbc_table_sizes = query_zbc_table_sizes(host);
    let forwarded_fb_extra = query_forwarded_fb_extra(host);
    let gpu_cache_info = query_gpu_cache_info(host);
    let forwarded_gpu_info = query_forwarded_gpu_info(host);
    let forwarded_fb_info = query_forwarded_fb_info(host);
    let smc_mode = query_smc_mode(host);
    let pcie_max_gen = query_pcie_max_gen(host);
    let ce_fault_method_buffer_size: Result<u32, FieldCause> = Ok(authored::CE_FAULT_METHOD_BUFFER_SIZE);
    let gsp_features = query_gsp_features(host);
    let gpu_name = query_gpu_name(host);
    let gpu_short_name = query_gpu_short_name(host);
    let vbios_version = query_vbios_version(host);
    let perf_level_info_v2 = query_perf_level_info_v2(host);
    let gss_replay = query_gss_replay(host);
    let video_caps = kinds.as_deref().map(|k| query_video_caps(host, k)).unwrap_or_default();

    let mut refusals = Vec::new();
    macro_rules! take {
        ($field:ident) => {
            match $field {
                Ok(v) => Some(v),
                Err(cause) => {
                    refusals.push(FieldRefusal { field: stringify!($field), cause });
                    None
                }
            }
        };
    }
    // ★ PROVENANCE order, so the refusal list reads like the table.
    let family = take!(family);
    let has_c2c = take!(has_c2c);
    let ce_caps = take!(ce_caps);
    let engines = take!(engines);
    let lce_pce_masks = take!(lce_pce_masks);
    let intr_table = take!(intr_table);
    let intr_subtree_map = take!(intr_subtree_map);
    let chip_info = take!(chip_info);
    let memory_system = take!(memory_system);
    let device_info = take!(device_info);
    let gmmu_static = take!(gmmu_static);
    let gr_static = take!(gr_static);
    let gr_info = take!(gr_info);
    let gr_context_buffers = take!(gr_context_buffers);
    let gr_zcull_info = take!(gr_zcull_info);
    let zbc_table_sizes = take!(zbc_table_sizes);
    let forwarded_fb_extra = take!(forwarded_fb_extra);
    let gpu_cache_info = take!(gpu_cache_info);
    let forwarded_gpu_info = take!(forwarded_gpu_info);
    let forwarded_fb_info = take!(forwarded_fb_info);
    let smc_mode = take!(smc_mode);
    let pcie_max_gen = take!(pcie_max_gen);
    let ce_fault_method_buffer_size = take!(ce_fault_method_buffer_size);
    let gsp_features = take!(gsp_features);
    let gpu_name = take!(gpu_name);
    let gpu_short_name = take!(gpu_short_name);
    let vbios_version = take!(vbios_version);
    let perf_level_info_v2 = take!(perf_level_info_v2);

    match (
        family,
        has_c2c,
        ce_caps,
        engines,
        lce_pce_masks,
        intr_table,
        intr_subtree_map,
        chip_info,
        memory_system,
        device_info,
        gmmu_static,
        gr_static,
        gr_info,
        gr_context_buffers,
        gr_zcull_info,
        zbc_table_sizes,
        forwarded_fb_extra,
        gpu_cache_info,
        forwarded_gpu_info,
        forwarded_fb_info,
        smc_mode,
        pcie_max_gen,
        ce_fault_method_buffer_size,
        gsp_features,
        gpu_name,
        gpu_short_name,
        vbios_version,
        perf_level_info_v2,
    ) {
        (
            Some(family),
            Some(has_c2c),
            Some(ce_caps),
            Some(engines),
            Some(lce_pce_masks),
            Some(intr_table),
            Some(intr_subtree_map),
            Some(chip_info),
            Some(memory_system),
            Some(device_info),
            Some(gmmu_static),
            Some(gr_static),
            Some(gr_info),
            Some(gr_context_buffers),
            Some(gr_zcull_info),
            Some(zbc_table_sizes),
            Some(forwarded_fb_extra),
            Some(gpu_cache_info),
            Some(forwarded_gpu_info),
            Some(forwarded_fb_info),
            Some(smc_mode),
            Some(pcie_max_gen),
            Some(ce_fault_method_buffer_size),
            Some(gsp_features),
            Some(gpu_name),
            Some(gpu_short_name),
            Some(vbios_version),
            Some(perf_level_info_v2),
        ) if refusals.is_empty() => Ok(HostFacts {
            family,
            has_c2c,
            ce_caps,
            engines,
            lce_pce_masks,
            intr_table,
            intr_subtree_map,
            chip_info,
            user_register_access_map: RegisterAccessMapRow::NOT_PUBLISHED,
            constructed_falcons: if video_falcons.is_empty() {
                FalconInventoryRow::NONE
            } else {
                FalconInventoryRow { falcons: Box::leak(video_falcons.into_boxed_slice()) }
            },
            memory_system,
            device_info,
            conf_compute: AUTHORED_CONF_COMPUTE,
            bif_static: AUTHORED_BIF_STATIC,
            fifo_channels: AUTHORED_FIFO_CHANNELS,
            gmmu_static,
            gr_static,
            gr_info,
            gr_context_buffers,
            gr_zcull_info,
            zbc_table_sizes,
            forwarded_fb_extra,
            gpu_cache_info,
            forwarded_gpu_info,
            forwarded_fb_info,
            smc_mode,
            pcie_max_gen,
            ce_fault_method_buffer_size,
            gsp_features,
            gpu_name: Some(gpu_name),
            gpu_short_name: Some(gpu_short_name),
            vbios_version,
            perf_level_info_v2,
            gss_replay,
            video_caps,
        }),
        _ => Err(HostFactsRefused { refusals }),
    }
}
