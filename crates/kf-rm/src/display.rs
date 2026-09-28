//! ★ v3-display — the physical-RM side of the display plane (`docs/design/V3_DISPLAY.md` §4).
//!
//! The guest's CPU-RM (KernelDisplay) and, above it, NVKMS drive a display engine that exists only
//! in kayfabe. This link answers the physical-RM display controls the guest routes to "GSP" (us),
//! from the chip's display row ([`kf_chip::display`]) and the virtual monitor ([`kf_disp::edid`]).
//! It forwards nothing to the host: the host's display engine is never touched.
//!
//! Seated ahead of the object seat and the ledger ([`crate::served_chain`]) only when the device
//! was realized with the display plane on (`display=on`); otherwise `GET_IP_VERSION` stays refused
//! and the guest's display engine is amputated (`sweep.rs`, the displayless posture of
//! `V3_HEADLESS_GRAPHICS.md` §3). ⊘ With the switch off (the default) this link is never built,
//! so nothing below can change a default-off answer.
//!
//! ## ★ Step (1), 2026-09-27: the link delegates to [`kf_disp::model::DisplayModel`]
//!
//! (`V3_DISPLAY.md`, the stop note's next step (1).) The link holds a [`SharedDisplayModel`] built
//! from the chip's row, one default 1080p DVI monitor ([`kf_disp::edid::Monitor::default_1080p`])
//! and the control layouts DERIVED for the guest's driver version
//! ([`kf_disp::layout::for_version`], `tools/derive_display_layouts.sh`), and:
//!
//! - **answers every control the model claims** ([`DisplayModel::claims`]): the M0 set below, plus
//!   `CHANNEL_PUSHBUFFER` and the ~30 NVKMS bring-up controls of §4.2 (A) (NV0073 / NV5070 / NVC370
//!   / NVC372) that the m0a run found refused at `0x730101`;
//! - **refuses by name** a claimed control whose params are FINN-serialized (a layout the model
//!   does not decode) or absent — the refusal set is exactly what the model claims;
//! - **observes** (never answers) `GSP_RM_ALLOC` of the display classes and `GSP_RM_FREE`: a channel
//!   alloc is recorded in the model's registry ([`DisplayModel::alloc`]), a free releases it — also
//!   when the freed object is the channel's display object, its device or its client, whose frees
//!   take the channel with them. The object seat below still records and answers the object.
//!
//! A guest driver version whose layouts this tree has not derived keeps the M0 answers below,
//! exactly as before step (1) (the link then observes nothing).
//!
//! ## M0: the controls KernelDisplay needs to come up [E]
//!
//! | control | where the guest asks | what we answer |
//! |---|---|---|
//! | `INTERNAL_DISPLAY_GET_IP_VERSION` `0x20800a4b` | `kdispStatePreInitLocked` (`kern_disp.c:324-350`) | the row's `DISPvXXYY` |
//! | `INTERNAL_DISPLAY_GET_STATIC_INFO` `0x20800a01` | `kdispStateInitLocked` (`:442-535`) — failure is fatal | our heads, windows and channel count |
//! | `INTERNAL_INIT_BRIGHTC_STATE_LOAD` `0x20800ac6` | `kdispInitBrightcStateLoad` (`:358-400`) — a non-OK status is fatal | `NV_OK`, the guest's own `status` field left as it sent it (no backlight) |
//! | `INTERNAL_SET_STATIC_EDID_DATA` `0x20800adf` | `kdispSetupAcpiEdid` (`:402-440`) — non-OK is fatal | `NV_OK`; the ACPI EDIDs of a laptop panel have no meaning here |
//! | `INTERNAL_DISPLAY_WRITE_INST_MEM` `0x20800a49` | `instmemStateInitLocked` | `NV_OK`; the instance memory's place is recorded (ctxdma lookup, M2) |

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use kf_disp::model::{DisplayModel, Statement};
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

const NV_OK: u32 = 0;
const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;
/// `NVOS54`'s status word inside the control envelope (as `inittables`, `zbc`).
const CONTROL_STATUS_OFF: usize = 12;

/// `NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_STATIC_INFO`.
pub const GET_STATIC_INFO: u32 = 0x2080_0a01;
/// `NV2080_CTRL_CMD_INTERNAL_DISPLAY_WRITE_INST_MEM`.
pub const WRITE_INST_MEM: u32 = 0x2080_0a49;
/// `NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_IP_VERSION`.
pub const GET_IP_VERSION: u32 = 0x2080_0a4b;
/// `NV2080_CTRL_CMD_INTERNAL_INIT_BRIGHTC_STATE_LOAD`.
pub const INIT_BRIGHTC_STATE_LOAD: u32 = 0x2080_0ac6;
/// `NV2080_CTRL_CMD_INTERNAL_SET_STATIC_EDID_DATA`.
pub const SET_STATIC_EDID_DATA: u32 = 0x2080_0adf;
/// The M0 set: what the link answers without a model (a guest driver whose display layouts this
/// tree has not derived).
pub const M0_CONTROLS: [u32; 5] = [GET_IP_VERSION, GET_STATIC_INFO, INIT_BRIGHTC_STATE_LOAD, SET_STATIC_EDID_DATA, WRITE_INST_MEM];

/// `sizeof(NV2080_CTRL_INTERNAL_DISPLAY_GET_STATIC_INFO_PARAMS)` — `feHwSysCap windowPresentMask
/// bFbRemapperEnabled(+pad) numHeads i2cPort internalDispActiveMask embeddedDisplayPortMask
/// bExternalMuxSupported bInternalMuxSupported(+pad) numDispChannels` (`ctrl2080internal.h:71-82`).
pub const STATIC_INFO_SIZE: usize = 36;
/// `sizeof(NV2080_CTRL_INTERNAL_DISPLAY_WRITE_INST_MEM_PARAMS)` — `instMemPhysAddr instMemSize
/// instMemAddrSpace instMemCpuCacheAttr` (`ctrl2080internal.h:891-896`).
pub const WRITE_INST_MEM_SIZE: usize = 24;
/// `NV402C_CTRL_NUM_I2C_PORTS` — "no external daughterboard" (`kern_disp.c:504-511`).
pub const NO_I2C_PORT: u32 = 16;
/// Display channel numbers run core `0`, windows `1..=32`, window-immediates `33..=64`, cursors
/// `73..=80` (`published/disp/v03_00/dev_disp.h`: `NV_PDISP_CHN_NUM_*`); `clientChannelTable` is
/// indexed by it (`kern_disp.c:476-486`), so the count is one past the last cursor.
pub const NUM_DISP_CHANNELS: u32 = 81;

/// ★ Hostile guest: the most display objects whose parent edge the link remembers (for the frees
/// that take a channel with its parent). One NVKMS device allocates a display object, a disp-SW
/// object and at most `1 + 2·windows + heads` = 21 channels; past the bound an object is not
/// remembered (logged) — its own free still releases it, only an ancestor's free no longer does.
pub const MAX_DISPLAY_OBJECTS: usize = 256;

/// ★ Step (1): the model the display link delegates to, shareable with the display plane.
///
/// ⊘ **Lock discipline** (`THE_CONSTRAINTS.md`: nothing blocks on a vCPU or under a lock a vCPU
/// takes). No vCPU path takes this lock — a PUT or cursor write is posted to the display worker's
/// queue and the vCPU returns (`V3_DISPLAY.md` §4.3) — and every holder does bounded, in-memory work
/// under it: one control's answer, one alloc or free, a drain of the statement queue. Nothing slow
/// (a guest-memory read of a pushbuffer, a CUDA launch, a timer wait, a log write) happens while it
/// is held; this link drops it before it logs.
pub type SharedDisplayModel = Arc<Mutex<DisplayModel>>;

/// The monitors behind the virtual connectors: one DVI-D monitor with a 1920×1080@60 EDID we author
/// (`V3_DISPLAY.md` §4.7; a configurable size is later work).
fn monitors() -> Vec<kf_disp::edid::Monitor> {
    vec![kf_disp::edid::Monitor::default_1080p()]
}

/// ★ The model for a chip's display row and a guest driver, or `None` when this tree has not
/// derived that driver's display layouts (never a guessed layout: `kf_disp::layout`).
#[must_use]
pub fn model_for(driver: &kf_abi::versions::DriverAbiTable, row: &kf_chip::display::DisplayRow) -> Option<DisplayModel> {
    let layouts = kf_disp::layout::for_version(&driver.driver_version().to_string())?;
    Some(DisplayModel::new(row, monitors(), layouts))
}

fn lock(m: &SharedDisplayModel) -> MutexGuard<'_, DisplayModel> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// ★ Step (1): the model's statements are drained by the link and written to the log, AFTER the
/// lock is dropped. There is no display worker yet to hand them to (step (3)); the registry itself
/// (`channels`, `inst_mem`) is what that worker will read.
fn log_statements(st: &[Statement]) {
    for s in st {
        eprintln!("kf-rm: display: {s:?}");
    }
}

/// ★ Is `class` one of the display objects the guest's kernel allocates and RPCs to us — on any
/// family (the ids are unique across families; the capability table refuses a family's classes
/// to another family's guest before this is asked).
#[must_use]
pub fn is_display_class(class: u32) -> bool {
    class == kf_chip::display::ALL[0].classes.common
        || kf_chip::display::ALL.iter().any(|r| {
            let c = &r.classes;
            [c.display, c.core, c.window, c.window_imm, c.cursor, c.disp_sw].contains(&class)
        })
}

/// Where the guest put display instance memory (`WRITE_INST_MEM`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstMem {
    /// Physical address (FB offset or guest physical address, per `addr_space`).
    pub phys: u64,
    /// Size in bytes.
    pub size: u64,
    /// `ADDR_FBMEM` (2) or `ADDR_SYSMEM` (1).
    pub addr_space: u32,
}

/// ★ The link.
pub struct DisplayPolicy {
    driver: kf_abi::versions::DriverAbiTable,
    row: &'static kf_chip::display::DisplayRow,
    /// ★ Step (1): the model this link delegates to; `None` only for a guest driver whose display
    /// layouts are not derived (the M0 answers then stand, and nothing is observed).
    model: Option<SharedDisplayModel>,
    /// The guest's instance memory, once stated — the M0 path's record (with a model, the model
    /// holds it: [`Self::stated_inst_mem`]).
    pub inst_mem: Option<InstMem>,
    /// Every display control the M0 path answered, in order (bounded: the first 256).
    pub seen: Vec<u32>,
    /// `(hClient, hObject)` → `hParent` of every display object observed (bounded:
    /// [`MAX_DISPLAY_OBJECTS`]) — the edges an ancestor's free follows.
    objects: BTreeMap<(u32, u32), u32>,
    /// ★ The controls this link claims — the model's [`DisplayModel::claimed`] (or the M0 set),
    /// fixed at construction, so asking needs no lock.
    claimed: BTreeSet<u32>,
}

impl DisplayPolicy {
    /// The link for a chip's display row: delegates to a model built for the guest driver's derived
    /// layouts ([`model_for`]), or answers the M0 set when they are not derived.
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable, row: &'static kf_chip::display::DisplayRow) -> DisplayPolicy {
        let model = model_for(&driver, row).map(|m| Arc::new(Mutex::new(m)));
        if model.is_none() {
            eprintln!(
                "kf-rm: display: no derived display layouts for guest driver {} — answering the M0 set only \
                 (tools/derive_display_layouts.sh derives them)",
                driver.driver_version()
            );
        }
        DisplayPolicy::with(driver, row, model)
    }

    /// ★ The link over a model the caller shares (the display plane's handle on the same registry).
    #[must_use]
    pub fn over(driver: kf_abi::versions::DriverAbiTable, row: &'static kf_chip::display::DisplayRow, model: SharedDisplayModel) -> DisplayPolicy {
        DisplayPolicy::with(driver, row, Some(model))
    }

    /// The M0 link with no model — what a guest driver without derived display layouts gets.
    #[must_use]
    pub fn without_model(driver: kf_abi::versions::DriverAbiTable, row: &'static kf_chip::display::DisplayRow) -> DisplayPolicy {
        DisplayPolicy::with(driver, row, None)
    }

    fn with(
        driver: kf_abi::versions::DriverAbiTable,
        row: &'static kf_chip::display::DisplayRow,
        model: Option<SharedDisplayModel>,
    ) -> DisplayPolicy {
        let claimed = match &model {
            Some(m) => lock(m).claimed().into_iter().collect(),
            None => M0_CONTROLS.into_iter().collect(),
        };
        DisplayPolicy { driver, row, model, inst_mem: None, seen: Vec::new(), objects: BTreeMap::new(), claimed }
    }

    /// The model this link delegates to (a handle on the same registry), if any.
    #[must_use]
    pub fn model(&self) -> Option<SharedDisplayModel> {
        self.model.clone()
    }

    /// ★ Is `cmd` this link's — the model's claims ([`DisplayModel::claimed`]), or the M0 set when
    /// there is no model. The FINN and missing-params refusals fire on exactly this set.
    #[must_use]
    pub fn claims(&self, cmd: u32) -> bool {
        self.claimed.contains(&cmd)
    }

    /// The whole claim set (see [`Self::claims`]).
    #[must_use]
    pub fn claimed(&self) -> &BTreeSet<u32> {
        &self.claimed
    }

    /// The instance memory the guest stated (`WRITE_INST_MEM`), from the model or the M0 record.
    #[must_use]
    pub fn stated_inst_mem(&self) -> Option<InstMem> {
        match &self.model {
            Some(m) => lock(m).inst_mem.map(|i| InstMem { phys: i.phys, size: i.size, addr_space: i.addr_space }),
            None => self.inst_mem,
        }
    }

    /// ★ `GET_STATIC_INFO`'s reply for our virtual display (the M0 path's).
    #[must_use]
    pub fn static_info(&self) -> [u8; STATIC_INFO_SIZE] {
        let mut p = [0u8; STATIC_INFO_SIZE];
        let heads = self.row.heads.min(8);
        let fe_hw_sys_cap = (1u32 << heads) - 1; // HEAD_EXISTS(i) = bit i (`dev_disp.h` v03_00)
        let windows = self.row.windows.min(32);
        let window_mask = if windows == 32 { u32::MAX } else { (1u32 << windows) - 1 };
        put(&mut p, 0, fe_hw_sys_cap);
        put(&mut p, 4, window_mask);
        // bFbRemapperEnabled @8 = 0
        put(&mut p, 12, heads);
        put(&mut p, 16, NO_I2C_PORT);
        // internalDispActiveMask @20 = 0, embeddedDisplayPortMask @24 = 0, muxes @28/@29 = 0
        put(&mut p, 32, NUM_DISP_CHANNELS);
        p
    }

    /// ★ The answer to one display control's params: `Ok(reply params)` or `Err(NV status)`;
    /// `None` when the control is not this link's.
    pub fn answer(&mut self, cmd: u32, params: &[u8]) -> Option<Result<Vec<u8>, u32>> {
        let Some(m) = self.model.clone() else {
            return self.answer_m0(cmd, params);
        };
        let (r, st) = {
            let mut g = lock(&m);
            let r = g.control(cmd, params);
            (r, g.take_statements())
        };
        log_statements(&st);
        r
    }

    /// The M0 answers (no model).
    fn answer_m0(&mut self, cmd: u32, params: &[u8]) -> Option<Result<Vec<u8>, u32>> {
        let r = match cmd {
            GET_IP_VERSION => {
                if params.len() != 4 {
                    return Some(Err(NV_ERR_INVALID_ARGUMENT));
                }
                Ok(self.row.ip_version.to_le_bytes().to_vec())
            }
            GET_STATIC_INFO => {
                if params.len() != STATIC_INFO_SIZE {
                    return Some(Err(NV_ERR_INVALID_ARGUMENT));
                }
                Ok(self.static_info().to_vec())
            }
            // [IN] from the guest (a laptop's ACPI backlight / panel EDIDs); nothing to do
            INIT_BRIGHTC_STATE_LOAD | SET_STATIC_EDID_DATA => Ok(params.to_vec()),
            WRITE_INST_MEM => {
                if params.len() != WRITE_INST_MEM_SIZE {
                    return Some(Err(NV_ERR_INVALID_ARGUMENT));
                }
                let q = |o: usize| u64::from_le_bytes(params[o..o + 8].try_into().unwrap_or([0; 8]));
                let d = |o: usize| u32::from_le_bytes(params[o..o + 4].try_into().unwrap_or([0; 4]));
                self.inst_mem = Some(InstMem { phys: q(0), size: q(8), addr_space: d(16) });
                Ok(params.to_vec())
            }
            _ => return None,
        };
        if self.seen.len() < 256 {
            self.seen.push(cmd);
        }
        Some(r)
    }

    fn on_control(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if !self.claims(req.cmd) {
            return None;
        }
        let refuse = |status: u32| Some(Reply { rpc_result: status, body: Vec::new() });
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            // ⊘ No control this link claims is FINN-serializable in the guest's RM: the 580 FINN
            // interface list has no NV0073 / NV5070 / NVC370 / NVC372 / NV2080-internal-display
            // entry (`ogkm-580: src/nvidia/interface/rmapi/src/g_finn_rm_api.c:803-850`,
            // `FinnRmApiGetUnserializedSize`), so a serialized one is a layout the model has not
            // measured — refused by name rather than decoded blind.
            eprintln!("kf-rm: display: control {:#010x} arrived FINN-serialized — refused", req.cmd);
            return refuse(NV_ERR_NOT_SUPPORTED);
        }
        let Some(params) = req.params_at.checked_add(req.params_size as usize).and_then(|e| cmd.payload.get(req.params_at..e)) else {
            return refuse(NV_ERR_INVALID_ARGUMENT);
        };
        match self.answer(req.cmd, params)? {
            Ok(p) if p.len() == params.len() => {
                let mut body = cmd.payload.clone();
                body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4].copy_from_slice(&NV_OK.to_le_bytes());
                body[req.params_at..req.params_at + p.len()].copy_from_slice(&p);
                Some(Reply { rpc_result: NV_OK, body })
            }
            // ⊘ An answer that is not the request's own size cannot be written back into it.
            Ok(_) => refuse(NV_ERR_INVALID_ARGUMENT),
            Err(st) => refuse(st),
        }
    }

    /// ★ Step (1): a display object's `GSP_RM_ALLOC`, OBSERVED — the object seat below records it
    /// and answers; this records a channel in the model's registry and remembers the object's
    /// parent edge.
    fn on_alloc(&mut self, cmd: &RpcCommand) {
        let Some(m) = self.model.clone() else { return };
        let body = cmd.wire_body();
        let Ok(h) = self.driver.decode_rpc_alloc(body) else { return };
        if !is_display_class(h.class) {
            return;
        }
        // ⊘ A class the boundary refuses is refused by the object seat next: nothing comes to exist.
        if !self.driver.capabilities().alloc_class(kf_arch::ids::ClassId(h.class)).is_permitted() {
            return;
        }
        // ⊘ Serialized or short params: the object seat refuses the alloc (`rmrpc`'s
        // `SerializedParams` / the declared window), so there is nothing to record either.
        let Some(params) = crate::rmrpc::alloc_params_window(&self.driver, body) else {
            eprintln!("kf-rm: display: alloc {:#x}:{:#x} class {:#06x}: params not readable — not recorded", h.client, h.handle, h.class);
            return;
        };
        let key = (h.client, h.handle);
        if self.objects.len() < MAX_DISPLAY_OBJECTS || self.objects.contains_key(&key) {
            self.objects.insert(key, h.parent);
        } else {
            eprintln!(
                "kf-rm: display: {MAX_DISPLAY_OBJECTS} display objects remembered — {:#x}:{:#x}'s parent edge is not",
                h.client, h.handle
            );
        }
        let (channel, recorded, st) = {
            let mut g = lock(&m);
            let channel = g.classes.channel_kind(h.class).is_some();
            let recorded = channel && g.alloc(h.client, h.handle, h.class, params);
            (channel, recorded, g.take_statements())
        };
        if channel && !recorded {
            eprintln!(
                "kf-rm: display: channel alloc {:#x}:{:#x} class {:#06x} NOT recorded ({} params bytes: not the derived \
                 allocation struct, or an instance this display does not have)",
                h.client,
                h.handle,
                h.class,
                params.len()
            );
        }
        log_statements(&st);
    }

    /// The object `root` of `client` and every remembered display object below it.
    fn subtree(&self, client: u32, root: u32) -> BTreeSet<u32> {
        let mut dead = BTreeSet::from([root]);
        // Bounded: each pass adds at least one of at most `MAX_DISPLAY_OBJECTS` objects, or stops.
        loop {
            let before = dead.len();
            for (&(c, h), p) in &self.objects {
                if c == client && dead.contains(p) {
                    dead.insert(h);
                }
            }
            if dead.len() == before {
                return dead;
            }
        }
    }

    /// ★ Step (1): a `GSP_RM_FREE`, OBSERVED. The guest's RM sends one per object
    /// (`ogkm-580: rs_client.c:785-843` → `alloc_free.c:959-990`), children before their parent
    /// (`rs_client.c:1085-1092`), so a channel's own free is the usual case; a free of its display object, device or client takes it too (a hostile guest may
    /// free a parent alone, and the object seat drops the subtree).
    fn on_free(&mut self, cmd: &RpcCommand) {
        let Some(m) = self.model.clone() else { return };
        let Ok(f) = self.driver.decode_free(&cmd.payload) else { return };
        let (client, object) = (f.client, f.handle);
        let gone: BTreeSet<u32> = if object == client {
            self.objects.keys().filter(|k| k.0 == client).map(|k| k.1).collect()
        } else {
            self.subtree(client, object)
        };
        for h in &gone {
            self.objects.remove(&(client, *h));
        }
        let st = {
            let mut g = lock(&m);
            if object == client {
                g.free_client(client);
            } else {
                for h in &gone {
                    g.free(client, *h);
                }
            }
            g.take_statements()
        };
        log_statements(&st);
    }
}

fn put(p: &mut [u8], off: usize, v: u32) {
    p[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

impl CommandPolicy for DisplayPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        match cmd.function {
            RpcFunction::RmControl => self.on_control(cmd),
            // ★ Observed, never answered: the channel link and the object seat below see them as
            // before, and the object seat answers.
            RpcFunction::RmAlloc => {
                self.on_alloc(cmd);
                None
            }
            RpcFunction::Free => {
                self.on_free(cmd);
                None
            }
            _ => None,
        }
    }
}

kf_util::assert_send_sync!(DisplayPolicy);

#[cfg(test)]
mod tests {
    use super::*;
    use kf_disp::model::ChannelKind;

    fn abi() -> kf_abi::versions::DriverAbiTable {
        *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench")
    }

    fn policy() -> DisplayPolicy {
        DisplayPolicy::new(abi(), &kf_chip::display::AMPERE)
    }

    fn m0_policy() -> DisplayPolicy {
        DisplayPolicy::without_model(abi(), &kf_chip::display::AMPERE)
    }

    fn layouts() -> &'static kf_disp::layout::Layouts {
        kf_disp::layout::for_version("580.159.04").expect("derived")
    }

    fn k(n: &str) -> u32 {
        layouts().k32(n).expect(n)
    }

    fn zeroed(s: &str) -> Vec<u8> {
        vec![0; layouts().size(s).expect(s)]
    }

    fn rpc(function: RpcFunction, payload: Vec<u8>) -> RpcCommand {
        RpcCommand { function, code: 0, sequence: 1, payload, elements: 1, delivered: Vec::new() }
    }

    /// A `GSP_RM_CONTROL` envelope (`rpc_gsp_rm_control_v03_00`, 40-byte header).
    fn control(cmd: u32, flags: u32, params: &[u8]) -> RpcCommand {
        let mut b = vec![0u8; 40];
        b[0..4].copy_from_slice(&0xc1d0_0001u32.to_le_bytes());
        b[4..8].copy_from_slice(&0xc1d0_0073u32.to_le_bytes());
        b[8..12].copy_from_slice(&cmd.to_le_bytes());
        b[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
        b[20..24].copy_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(params);
        rpc(RpcFunction::RmControl, b)
    }

    /// A `GSP_RM_ALLOC` envelope (`rpc_gsp_rm_alloc_v03_00`, 32-byte header).
    fn alloc(client: u32, parent: u32, handle: u32, class: u32, params: &[u8]) -> RpcCommand {
        let mut b = vec![0u8; 32];
        for (i, v) in [client, parent, handle, class, 0, params.len() as u32].iter().enumerate() {
            b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(params);
        rpc(RpcFunction::RmAlloc, b)
    }

    /// A `GSP_RM_FREE` (`NVOS00`: hRoot hObjectParent hObjectOld status).
    fn free(client: u32, parent: u32, object: u32) -> RpcCommand {
        rpc(RpcFunction::Free, [client, parent, object, 0].iter().flat_map(|v| v.to_le_bytes()).collect())
    }

    /// `NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS` (or the PIO one) with `channelInstance`.
    fn chan_params(pio: bool, inst: u32) -> Vec<u8> {
        let mut p = zeroed(if pio { "NV50VAIO_CHANNELPIO_ALLOCATION_PARAMETERS" } else { "NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS" });
        p[0..4].copy_from_slice(&inst.to_le_bytes());
        p
    }

    /// ★ GA10x: the IP version a real GA106 reports, and a static info with four heads, eight
    /// windows and a channel table large enough for every channel number the guest computes.
    #[test]
    fn ga10x_reports_the_real_ip_version_and_a_four_head_static_info() {
        for mut p in [m0_policy(), policy()] {
            assert_eq!(p.answer(GET_IP_VERSION, &[0; 4]), Some(Ok(vec![0x00, 0x00, 0x01, 0x04])));
            let si = p.answer(GET_STATIC_INFO, &[0; STATIC_INFO_SIZE]).expect("ours").expect("ok");
            let w = |o: usize| u32::from_le_bytes(si[o..o + 4].try_into().unwrap());
            assert_eq!(w(0), 0xf, "HEAD_EXISTS 0..3");
            assert_eq!(w(4), 0xff, "windows 0..7");
            assert_eq!(w(12), 4);
            assert_eq!(w(16), NO_I2C_PORT);
            assert_eq!(w(32), NUM_DISP_CHANNELS);
        }
        const { assert!(NUM_DISP_CHANNELS > 73 + 7, "the last cursor channel number fits") };
    }

    /// Instance memory is recorded; the fatal-if-refused [IN] controls answer OK; malformed sizes
    /// are refused; foreign controls are not ours — on the M0 path and through the model alike.
    #[test]
    fn init_controls_answer_and_inst_mem_is_recorded() {
        for mut p in [m0_policy(), policy()] {
            let mut im = [0u8; WRITE_INST_MEM_SIZE];
            im[0..8].copy_from_slice(&0x1234_5000u64.to_le_bytes());
            im[8..16].copy_from_slice(&0x1_0000u64.to_le_bytes());
            im[16..20].copy_from_slice(&2u32.to_le_bytes());
            assert!(matches!(p.answer(WRITE_INST_MEM, &im), Some(Ok(_))));
            assert_eq!(p.stated_inst_mem(), Some(InstMem { phys: 0x1234_5000, size: 0x1_0000, addr_space: 2 }));
            assert!(matches!(p.answer(INIT_BRIGHTC_STATE_LOAD, &[0u8; 4104]), Some(Ok(_))));
            assert!(matches!(p.answer(SET_STATIC_EDID_DATA, &[0u8; 8388]), Some(Ok(_))));
            assert_eq!(p.answer(GET_STATIC_INFO, &[0; 8]), Some(Err(NV_ERR_INVALID_ARGUMENT)));
            assert_eq!(p.answer(0x2080_0101, &[0; 4]), None);
        }
    }

    /// ★ Step (1) changes no M0 answer the m0a run measured: through the model, every M0 control's
    /// reply is byte-for-byte the hand-written M0 reply, for the same request bytes.
    #[test]
    fn the_model_answers_the_m0_set_byte_for_byte_as_m0_did() {
        let (mut m0, mut m) = (m0_policy(), policy());
        let mut im = vec![0u8; WRITE_INST_MEM_SIZE];
        im[..8].copy_from_slice(&0xdead_0000u64.to_le_bytes());
        let reqs: [(u32, Vec<u8>); 5] = [
            (GET_IP_VERSION, vec![0xaa; 4]),
            (GET_STATIC_INFO, vec![0x5a; STATIC_INFO_SIZE]),
            (INIT_BRIGHTC_STATE_LOAD, (0..4104).map(|i| i as u8).collect()),
            (SET_STATIC_EDID_DATA, (0..8388).map(|i| (i * 7) as u8).collect()),
            (WRITE_INST_MEM, im),
        ];
        for (cmd, req) in reqs {
            assert_eq!(m.answer(cmd, &req), m0.answer(cmd, &req), "{cmd:#010x}");
            let c = control(cmd, 0, &req);
            assert_eq!(m.respond(&c), m0.respond(&c), "{cmd:#010x} through respond()");
        }
    }

    /// ★ Step (1): the link's claims are the model's — the M0 set, `CHANNEL_PUSHBUFFER`, and the
    /// NVKMS bring-up controls the m0a run found refused (`0x730101`, `0x730107`, `0x730102`,
    /// `0x730151`) — and a claimed control is answered, not left for the ledger.
    #[test]
    fn the_link_claims_what_the_model_claims() {
        let mut p = policy();
        let m = p.model().expect("the bench driver's layouts are derived");
        for cmd in M0_CONTROLS.into_iter().chain([kf_disp::model::CHANNEL_PUSHBUFFER, 0x0073_0101, 0x0073_0107, 0x0073_0102, 0x0073_0151]) {
            assert!(p.claims(cmd), "{cmd:#010x}");
            assert!(m.lock().unwrap().claims(cmd), "{cmd:#010x}");
        }
        for cmd in [0x2080_0101, 0x2080_0a70, 0xa06f_0103, 0x0073_ffff] {
            assert_eq!(p.claims(cmd), m.lock().unwrap().claims(cmd), "{cmd:#010x}");
            assert!(!p.claims(cmd), "{cmd:#010x}");
        }
        let s = "NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS";
        let r = p.respond(&control(k("NV0073_CTRL_CMD_SYSTEM_GET_NUM_HEADS"), 0, &zeroed(s))).expect("answered");
        assert_eq!(r.rpc_result, NV_OK);
        let (off, _) = layouts().field(s, "numHeads").unwrap();
        assert_eq!(u32::from_le_bytes(r.body[40 + off..44 + off].try_into().unwrap()), 4);
        // the M0 link claims the M0 set only
        let m0 = m0_policy();
        assert!(M0_CONTROLS.iter().all(|c| m0.claims(*c)));
        assert!(!m0.claims(0x0073_0101) && !m0.claims(kf_disp::model::CHANNEL_PUSHBUFFER));
    }

    /// ★ Step (1): the FINN refusal set IS the claim set. Every control the model claims, arriving
    /// FINN-serialized, is refused `NOT_SUPPORTED` by name (never decoded); with its params missing,
    /// `INVALID_ARGUMENT`; a serialized control the link does not claim is not its business.
    #[test]
    fn the_finn_refusal_set_is_what_the_model_claims() {
        let mut p = policy();
        const SERIALIZED: u32 = 1 << 1;
        // categories 0x00..0x1f of each display interface, and the subdevice's internal-display page
        let claimed: Vec<u32> = [0x0073_0000u32, 0x5070_0000, 0xc370_0000, 0xc372_0000]
            .iter()
            .flat_map(|b| *b..*b + 0x2000)
            .chain(0x2080_0a00..0x2080_0b00)
            .filter(|c| p.claims(*c))
            .collect();
        assert_eq!(claimed.len(), 30 + 6, "the NVKMS bring-up set and the six internal controls");
        assert_eq!(claimed.iter().copied().collect::<BTreeSet<u32>>(), *p.claimed(), "nothing claimed outside the set");
        for cmd in &claimed {
            let r = p.respond(&control(*cmd, SERIALIZED, &[0; 16])).expect("refused by name");
            assert_eq!(r.rpc_result, NV_ERR_NOT_SUPPORTED, "{cmd:#010x}");
            let mut short = control(*cmd, 0, &[]);
            short.payload[16..20].copy_from_slice(&64u32.to_le_bytes()); // declares 64, sends 0
            assert_eq!(p.respond(&short).map(|r| r.rpc_result), Some(NV_ERR_INVALID_ARGUMENT), "{cmd:#010x}");
        }
        assert!(p.respond(&control(0x2080_0101, SERIALIZED, &[0; 16])).is_none(), "not claimed: not ours");
    }

    /// ★ Step (1), on: display channel allocs are recorded in the SHARED model (with their stated
    /// pushbuffer and GET = PUT = offset), frees release them — the channel's own free, its display
    /// object's, and its client's — and nothing is answered: the object seat still answers.
    #[test]
    fn display_allocs_are_tracked_and_frees_release_them() {
        let shared: SharedDisplayModel = Arc::new(Mutex::new(model_for(&abi(), &kf_chip::display::AMPERE).expect("derived")));
        let mut p = DisplayPolicy::over(abi(), &kf_chip::display::AMPERE, shared.clone());
        let (c, dev, disp) = (0xc1d0_0001, 0xcafe_0001, 0xcafe_0070);
        // the core channel's pushbuffer is stated first (internal subdevice), then the objects
        let s = "NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS";
        let mut q = kf_disp::layout::Params::new(layouts(), s, &zeroed(s)).unwrap();
        q.set("hclass", 0xC67D);
        q.set("addressSpace", 1);
        q.set("physicalAddr", 0x1234_5000);
        q.set("limit", 0xfff);
        q.set("valid", 1);
        assert_eq!(p.respond(&control(kf_disp::model::CHANNEL_PUSHBUFFER, 0, &q.buf)).map(|r| r.rpc_result), Some(NV_OK));
        assert!(p.respond(&alloc(c, dev, disp, 0xC670, &[])).is_none(), "observed, never answered");
        let mut core = chan_params(false, 0);
        core[12..16].copy_from_slice(&0x40u32.to_le_bytes()); // offset
        assert!(p.respond(&alloc(c, disp, 0xcafe_0d00, 0xC67D, &core)).is_none());
        for w in 0..8 {
            assert!(p.respond(&alloc(c, disp, 0xcafe_0e00 + w, 0xC67E, &chan_params(false, w))).is_none());
            assert!(p.respond(&alloc(c, disp, 0xcafe_0b00 + w, 0xC67B, &chan_params(false, w))).is_none());
        }
        for h in 0..4 {
            assert!(p.respond(&alloc(c, disp, 0xcafe_0a00 + h, 0xC67A, &chan_params(true, h))).is_none());
        }
        {
            let g = shared.lock().unwrap();
            assert_eq!(g.channels.len(), 1 + 8 + 8 + 4);
            let core = &g.channels[&(ChannelKind::Core, 0)];
            assert_eq!((core.handle, core.get, core.put), (0xcafe_0d00, 0x40, 0x40));
            assert_eq!(core.pb.map(|b| b.phys), Some(0x1234_5000));
            assert!(g.statements.is_empty(), "the link drains the statements into the log");
        }
        // the channel's own free
        assert!(p.respond(&free(c, disp, 0xcafe_0e03)).is_none());
        assert!(!shared.lock().unwrap().channels.contains_key(&(ChannelKind::Window, 3)));
        // a free of the display object takes every channel under it
        assert!(p.respond(&free(c, dev, disp)).is_none());
        assert!(shared.lock().unwrap().channels.is_empty());
        // and a client's free takes whatever it still held
        assert!(p.respond(&alloc(c, dev, disp, 0xC670, &[])).is_none());
        assert!(p.respond(&alloc(c, disp, 0xcafe_0d00, 0xC67D, &chan_params(false, 0))).is_none());
        assert_eq!(shared.lock().unwrap().channels.len(), 1);
        assert!(p.respond(&free(c, 0, c)).is_none());
        assert!(shared.lock().unwrap().channels.is_empty());
        assert!(p.objects.is_empty(), "no parent edge outlives its client");
    }

    /// ★ Hostile guest: an alloc that is not a display class, a channel instance the display does
    /// not have, short or FINN-serialized params, and another family's class are never recorded;
    /// another client's free of the same handle releases nothing; the remembered edges are bounded.
    #[test]
    fn hostile_allocs_and_frees_are_bounded() {
        let mut p = policy();
        let m = p.model().unwrap();
        let (c, disp) = (0xc1d0_0001, 0xcafe_0070);
        p.respond(&alloc(c, 0xcafe_0001, 0xcafe_0e09, 0xC67E, &chan_params(false, 9)));
        p.respond(&alloc(c, disp, 0xcafe_0a04, 0xC67A, &chan_params(true, 4)));
        p.respond(&alloc(c, disp, 0xcafe_0e00, 0xC67E, &chan_params(false, 0)[..8]));
        p.respond(&alloc(c, disp, 0xcafe_0e01, 0xC57E, &chan_params(false, 1)));
        let mut ser = alloc(c, disp, 0xcafe_0e02, 0xC67E, &chan_params(false, 2));
        ser.payload[24..28].copy_from_slice(&(1u32 << 1).to_le_bytes());
        p.respond(&ser);
        p.respond(&alloc(c, disp, 0xcafe_00c0, 0xC0B5, &[0; 8]));
        assert!(m.lock().unwrap().channels.is_empty(), "{:?}", m.lock().unwrap().channels);
        assert!(p.respond(&alloc(c, disp, 0xcafe_0e07, 0xC67E, &chan_params(false, 7))).is_none());
        p.respond(&free(0xc1d0_0002, disp, 0xcafe_0e07));
        p.respond(&free(0xc1d0_0002, 0, 0xc1d0_0002));
        assert_eq!(m.lock().unwrap().channels.len(), 1, "another client's free names another object");
        for i in 0..(MAX_DISPLAY_OBJECTS as u32 + 10) {
            p.respond(&alloc(c, disp, 0xd000_0000 + i, 0xC372, &[]));
        }
        assert_eq!(p.objects.len(), MAX_DISPLAY_OBJECTS);
        p.respond(&free(c, 0xcafe_0001, disp));
        assert!(m.lock().unwrap().channels.is_empty());
    }

    /// ★ The M0 link (a guest driver with no derived display layouts) observes nothing and claims
    /// only the M0 set — the behaviour before step (1).
    #[test]
    fn without_a_model_allocs_are_not_observed() {
        let mut p = m0_policy();
        assert!(p.model().is_none());
        assert!(p.respond(&alloc(0xc1d0_0001, 0xcafe_0070, 0xcafe_0d00, 0xC67D, &chan_params(false, 0))).is_none());
        assert!(p.objects.is_empty());
        assert!(p.respond(&control(0x0073_0101, 0, &[0; 16])).is_none(), "not claimed without the model");
    }
}
