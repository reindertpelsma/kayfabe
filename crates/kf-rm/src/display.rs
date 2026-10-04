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
//! - **observes accepted** (never answers) `GSP_RM_ALLOC` of the display classes and `GSP_RM_FREE`: a channel
//!   alloc is recorded in the model's registry ([`DisplayModel::alloc`]), a free releases it — also
//!   when the freed object is the channel's display object, its device or its client, whose frees
//!   take the channel with them. The object seat records and accepts the complete event BEFORE
//!   notifying the registry; rejected events and held fragments cannot mutate it.
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
pub const M0_CONTROLS: [u32; 5] = [
    GET_IP_VERSION,
    GET_STATIC_INFO,
    INIT_BRIGHTC_STATE_LOAD,
    SET_STATIC_EDID_DATA,
    WRITE_INST_MEM,
];

/// ★ The claimed controls the **guest's own** export table marks cacheable
/// (`PERSISTENT_CACHEABLE`, `0x800000`): `NV0073_CTRL_CMD_SYSTEM_GET_SUPPORTED` and
/// `_SYSTEM_GET_INTERNAL_DISPLAYS` (flags `0x82004a`), `_SPECIFIC_GET_TYPE` (`0x820046`), accessRight 0
/// (`ogkm-580: g_disp_objs_nvoc.c`, the export table). The guest keeps our FIRST answer across
/// StateLoad/Unload, so each must be fixed for the device's life — the decision
/// [`crate::sticky::BRANCH_A_CACHEABLE`] forces for the init-table rows, made here for the display
/// link (2026-09-28). It holds: the connector set is the model's fixed monitor list ([`monitors`]) and
/// the display type a constant, pinned by a test. ⊘ Revisit when monitors become configurable or
/// hotplug exists — those answers then stop being constant and the guest would serve stale ones.
pub const GUEST_CACHEABLE: [u32; 3] = [0x0073_0107, 0x0073_0116, 0x0073_0240];

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

/// The monitors behind the virtual connectors: one DVI-D monitor with an EDID we author
/// (`V3_DISPLAY.md` §4.7) — 1920×1080@60 unless `display-max-fps` is set (`max_fps`, 0 unset: the
/// EDID is then byte-identical to the one before the property, D5), when the preferred mode and
/// the range limit follow the cap ([`kf_disp::edid::Monitor::configured`], §8.16).
///
/// ★ Public because the boot display reads the SAME first monitor (`V3_DISPLAY.md` §4.11): kf3's
/// option ROM carries its preferred mode and EDID, so the firmware's mode is the native one and the
/// two statements of "what the monitor is" cannot disagree — both callers pass the same property.
#[must_use]
pub fn monitors(max_fps: u32) -> Vec<kf_disp::edid::Monitor> {
    vec![kf_disp::edid::Monitor::configured(max_fps)]
}

/// ★ The model for a chip's display row and a guest driver, or `None` when this tree has not
/// derived that driver's display layouts (never a guessed layout: `kf_disp::layout`). `max_fps` is
/// the `display-max-fps` property ([`monitors`]).
#[must_use]
pub fn model_for(
    driver: &kf_abi::versions::DriverAbiTable,
    row: &kf_chip::display::DisplayRow,
    max_fps: u32,
) -> Option<DisplayModel> {
    let layouts = kf_disp::layout::for_version(&driver.driver_version().to_string())?;
    Some(DisplayModel::new(row, monitors(max_fps), layouts))
}

fn lock(m: &SharedDisplayModel) -> MutexGuard<'_, DisplayModel> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// ★ What the link does with the model's statements once it has dropped the lock.
enum Settle {
    /// No plane: the link drained them — written to the log (the GPU-free configuration).
    Log(Vec<Statement>),
    /// A display plane is attached: they stay queued for it, and it is woken.
    Wake(kf_disp::model::Waker),
}

/// ★ Under the lock: leave the statements for an attached plane (step (3): "the statement drain
/// moves to the display worker"), or drain them for the log when no plane exists.
fn settle(g: &mut DisplayModel) -> Settle {
    match g.waker() {
        Some(w) => Settle::Wake(w),
        None => Settle::Log(g.take_statements()),
    }
}

/// After the lock is dropped: log, or wake the plane (one eventfd write, never a wait).
fn finish(s: Settle) {
    match s {
        Settle::Log(st) => {
            for s in st {
                eprintln!("kf-rm: display: {s:?}");
            }
        }
        Settle::Wake(w) => w.wake(),
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
            [
                c.display,
                c.core,
                c.window,
                c.window_imm,
                c.cursor,
                c.disp_sw,
            ]
            .contains(&class)
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
    /// ★ The controls this link claims — the model's [`DisplayModel::claimed`] (or the M0 set),
    /// fixed at construction, so asking needs no lock.
    claimed: BTreeSet<u32>,
    /// ★ EXPERIMENT `x11-dispsw`: each display-SW constructor's query paired with its alloc
    /// ([`DispSwPairing`]); `None` with the switch off (nothing is looked at).
    dispsw_pairing: Option<DispSwPairing>,
}

/// `NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_ACTIVE_DISPLAY_DEVICES` (`ogkm-580:
/// ctrl2080internal.h:1537`) — sent by exactly one caller in the guest's CPU-RM, the
/// `GF100_DISP_SW` constructor (`disp_sw.c:74`).
pub const GET_ACTIVE_DISPLAY_DEVICES: u32 = 0x2080_0a5d;
/// `GF100_DISP_SW` (`ogkm-580: class/cl9072.h:35`).
const GF100_DISP_SW: u32 = 0x9072;

/// ★★ EXPERIMENT `x11-dispsw` (review 2026-10-03, MEDIUM) — **the one numbering slip NO physical RM
/// can see, counted.** The guest's CPU-RM gives a display-SW object its channel's next software
/// classID (`chandesConstruct` → `kchannelRegisterChild`, `ogkm-580: channel_descendant.c:254`)
/// BEFORE `dispswConstruct` asks this query (`disp_sw.c:74`) and checks `logicalHeadId` /
/// `displayMask` against it (`:83-98`); only a constructor that passes sends the alloc
/// (`alloc_free.c:860-916`), and the alloc carries no classID (`rpc.c:11140-11230`). So a
/// constructor that fails AFTER the query leaves that channel's guest numbering one past every
/// physical RM's — ours included — and names no channel we could repair or refuse.
///
/// What IS visible: both the query and the alloc RPC are sent under the same GPU lock
/// (`RS_FLAGS_ACQUIRE_GPU_GROUP_LOCK`, `resource_list.h:1509-1510`; `rpc.c:11167-11174`), so a
/// stock guest sends them as a pair, query then alloc. A query that arrives while the previous one
/// is still unpaired is such a failed constructor, and is counted and named.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DispSwPairing {
    /// A query was answered and its alloc has not arrived yet.
    open: bool,
    /// Queries seen.
    pub queries: u64,
    /// Queries whose alloc never came (seen at the next query).
    pub unpaired: u64,
}

impl DispSwPairing {
    /// A constructor's query: `true` when the PREVIOUS query's alloc never came.
    pub fn query(&mut self) -> bool {
        let lost = self.open;
        self.unpaired += u64::from(lost);
        self.queries += 1;
        self.open = true;
        lost
    }

    /// A `GF100_DISP_SW` alloc: it pairs the open query, if any.
    pub fn alloc(&mut self) {
        self.open = false;
    }
}

impl DisplayPolicy {
    /// The link for a chip's display row: delegates to a model built for the guest driver's derived
    /// layouts ([`model_for`]), or answers the M0 set when they are not derived.
    #[must_use]
    pub fn new(
        driver: kf_abi::versions::DriverAbiTable,
        row: &'static kf_chip::display::DisplayRow,
    ) -> DisplayPolicy {
        // no display plane here (the GPU-free configuration): the property needs one, so unset
        let model = model_for(&driver, row, 0).map(|m| Arc::new(Mutex::new(m)));
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
    pub fn over(
        driver: kf_abi::versions::DriverAbiTable,
        row: &'static kf_chip::display::DisplayRow,
        model: SharedDisplayModel,
    ) -> DisplayPolicy {
        DisplayPolicy::with(driver, row, Some(model))
    }

    /// ★ Step (3): the link over the display PLANE's model, shared across `ReselectAtFn1` rebuilds.
    /// The model is re-targeted to this driver's derived layouts (a rebuild for the guest's own
    /// version); a driver with no derived layouts gets the M0 link and the plane sees nothing.
    #[must_use]
    pub fn over_shared(
        driver: kf_abi::versions::DriverAbiTable,
        row: &'static kf_chip::display::DisplayRow,
        shared: &SharedDisplayModel,
    ) -> DisplayPolicy {
        let Some(layouts) = kf_disp::layout::for_version(&driver.driver_version().to_string())
        else {
            eprintln!(
                "kf-rm: display: no derived display layouts for guest driver {} — answering the M0 set only; the display \
                 plane sees nothing",
                driver.driver_version()
            );
            return DisplayPolicy::without_model(driver, row);
        };
        {
            let mut g = lock(shared);
            if g.layouts().version != layouts.version {
                eprintln!(
                    "kf-rm: display: the plane's model re-targeted {} -> {}",
                    g.layouts().version,
                    layouts.version
                );
                g.retarget(layouts);
            }
        }
        DisplayPolicy::over(driver, row, shared.clone())
    }

    /// The M0 link with no model — what a guest driver without derived display layouts gets.
    #[must_use]
    pub fn without_model(
        driver: kf_abi::versions::DriverAbiTable,
        row: &'static kf_chip::display::DisplayRow,
    ) -> DisplayPolicy {
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
        DisplayPolicy {
            driver,
            row,
            model,
            inst_mem: None,
            seen: Vec::new(),
            claimed,
            dispsw_pairing: None,
        }
    }

    /// ★ EXPERIMENT `x11-dispsw` (default off): offer the `GF100_DISP_SW` object — the model answers
    /// its constructor's `GET_ACTIVE_DISPLAY_DEVICES` query instead of refusing it
    /// ([`DisplayModel::offer_display_sw`]). `false` touches nothing (no lock is taken), so a
    /// default-off link is the link it was. ⊘ Only [`crate::served_chain`] calls this, and only
    /// together with the channel link's [`crate::chanlink::ChannelPolicy::with_display_sw_twins`]:
    /// offered without a host twin, the object's software methods trap on the host GPU (run m3c).
    /// With no derived layouts (the M0 link) the query is not claimed and stays refused.
    #[must_use]
    pub fn offering_display_sw(mut self, on: bool) -> DisplayPolicy {
        if on && let Some(m) = &self.model {
            lock(m).offer_display_sw(true);
            self.dispsw_pairing = Some(DispSwPairing::default());
            eprintln!(
                "kf-rm: display: EXPERIMENT x11-dispsw — GF100_DISP_SW is OFFERED (its constructor's query \
                 is answered; every alloc is twinned on the host or refused by name)"
            );
        }
        self
    }

    /// The separate lifecycle observer. It is seated inside the object policy, not in
    /// `respond`, so only successfully applied, fully reassembled events reach it.
    pub(crate) fn registry(&self) -> Option<DisplayRegistry> {
        self.model.clone().map(|model| DisplayRegistry {
            driver: self.driver,
            model,
            objects: BTreeMap::new(),
        })
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
            Some(m) => lock(m).inst_mem.map(|i| InstMem {
                phys: i.phys,
                size: i.size,
                addr_space: i.addr_space,
            }),
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
        let window_mask = if windows == 32 {
            u32::MAX
        } else {
            (1u32 << windows) - 1
        };
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
    /// `None` when the control is not this link's. A per-object control (`SET_RMFREE_FLAGS`) needs
    /// the object it names: [`Self::answer_on`].
    pub fn answer(&mut self, cmd: u32, params: &[u8]) -> Option<Result<Vec<u8>, u32>> {
        self.answer_at(None, cmd, params)
    }

    /// ★ [`Self::answer`] for the control `GSP_RM_CONTROL` addressed to `(client, object)`.
    pub fn answer_on(
        &mut self,
        client: u32,
        object: u32,
        cmd: u32,
        params: &[u8],
    ) -> Option<Result<Vec<u8>, u32>> {
        self.answer_at(Some((client, object)), cmd, params)
    }

    fn answer_at(
        &mut self,
        target: Option<(u32, u32)>,
        cmd: u32,
        params: &[u8],
    ) -> Option<Result<Vec<u8>, u32>> {
        let Some(m) = self.model.clone() else {
            return self.answer_m0(cmd, params);
        };
        let (r, st) = {
            let mut g = lock(&m);
            let r = match target {
                Some((c, o)) => g.control_on(c, o, cmd, params),
                None => g.control(cmd, params),
            };
            (r, settle(&mut g))
        };
        finish(st);
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
                let q =
                    |o: usize| u64::from_le_bytes(params[o..o + 8].try_into().unwrap_or([0; 8]));
                let d =
                    |o: usize| u32::from_le_bytes(params[o..o + 4].try_into().unwrap_or([0; 4]));
                self.inst_mem = Some(InstMem {
                    phys: q(0),
                    size: q(8),
                    addr_space: d(16),
                });
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
        let refuse = |status: u32| {
            Some(Reply {
                rpc_result: status,
                body: Vec::new(),
            })
        };
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            // ⊘ No control this link claims is FINN-serializable in the guest's RM: the 580 FINN
            // interface list has no NV0073 / NV5070 / NVC370 / NVC372 / NV2080-internal-display
            // entry (`ogkm-580: src/nvidia/interface/rmapi/src/g_finn_rm_api.c:803-850`,
            // `FinnRmApiGetUnserializedSize`), so a serialized one is a layout the model has not
            // measured — refused by name rather than decoded blind.
            eprintln!(
                "kf-rm: display: control {:#010x} arrived FINN-serialized — refused",
                req.cmd
            );
            return refuse(NV_ERR_NOT_SUPPORTED);
        }
        let Some(params) = req
            .params_at
            .checked_add(req.params_size as usize)
            .and_then(|e| cmd.payload.get(req.params_at..e))
        else {
            return refuse(NV_ERR_INVALID_ARGUMENT);
        };
        match self.answer_on(req.client, req.object, req.cmd, params)? {
            Ok(p) if p.len() == params.len() => {
                let mut body = cmd.payload.clone();
                body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
                    .copy_from_slice(&NV_OK.to_le_bytes());
                body[req.params_at..req.params_at + p.len()].copy_from_slice(&p);
                Some(Reply {
                    rpc_result: NV_OK,
                    body,
                })
            }
            // ⊘ An answer that is not the request's own size cannot be written back into it.
            Ok(_) => refuse(NV_ERR_INVALID_ARGUMENT),
            Err(st) => refuse(st),
        }
    }
}

/// The display object's accepted lifecycle, sharing only the model with the controls link.
pub(crate) struct DisplayRegistry {
    driver: kf_abi::versions::DriverAbiTable,
    model: SharedDisplayModel,
    /// `(hClient, hObject)` → `hParent`, bounded by `MAX_DISPLAY_OBJECTS`.
    objects: BTreeMap<(u32, u32), u32>,
}

impl DisplayRegistry {
    pub(crate) fn observe(&mut self, cmd: &RpcCommand) {
        match cmd.function {
            RpcFunction::RmAlloc => self.on_alloc(cmd),
            RpcFunction::Free => self.on_free(cmd),
            _ => {}
        }
    }

    /// Record an allocation the object seat has already accepted.
    fn on_alloc(&mut self, cmd: &RpcCommand) {
        let m = self.model.clone();
        let body = cmd.wire_body();
        let Ok(h) = self.driver.decode_rpc_alloc(body) else {
            return;
        };
        if h.class == kf_abi::generated::classes::NV01_EVENT_KERNEL_CALLBACK_EX {
            self.on_event_alloc(h.client, h.handle, h.parent, body);
            return;
        }
        if !is_display_class(h.class) {
            return;
        }
        // Defensive checks for the model's input, even though the object seat already accepted it.
        if !self
            .driver
            .capabilities()
            .alloc_class(kf_arch::ids::ClassId(h.class))
            .is_permitted()
        {
            return;
        }
        // ⊘ Serialized or short params: the object seat refuses the alloc (`rmrpc`'s
        // `SerializedParams` / the declared window), so there is nothing to record either.
        let Some(params) = crate::rmrpc::alloc_params_window(&self.driver, body) else {
            eprintln!(
                "kf-rm: display: alloc {:#x}:{:#x} class {:#06x}: params not readable — not recorded",
                h.client, h.handle, h.class
            );
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
            let recorded = channel && g.alloc(h.client, h.parent, h.handle, h.class, params);
            (channel, recorded, settle(&mut g))
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
        finish(st);
    }

    /// ★ Display step 3c (`V3_DISPLAY.md` §8.6): NVKMS's hotplug registration — an accepted
    /// `NV01_EVENT_KERNEL_CALLBACK_EX` whose `notifyIndex` is `NV2080_NOTIFIERS_HOTPLUG |
    /// NV01_EVENT_CLIENT_RM` (`ogkm-580: src/nvidia-modeset/src/nvkms-rm.c:1775-1800`;
    /// `event.c:148-170`) — becomes the `(hClient, hEvent)` a hotplug `POST_EVENT` names.
    ///
    /// ⊘ A SEPARATE, narrow seat: `crate::osevent` refuses this class by a pinned rule (its
    /// `osNotifyEvent` would wake guest-kernel state), and stays untouched. This seat records only
    /// this one notifier, and the post it feeds is a LIST post (`bNotifyList`) with the bare index,
    /// so the guest's own RM — gated by its `notifyActions` — decides whom to wake. Only
    /// `notifyIndex` (`NV0005_ALLOC_PARAMETERS` @ +12) is read; `data` @ +16 is a guest pointer.
    fn on_event_alloc(&mut self, client: u32, event: u32, parent: u32, body: &[u8]) {
        let Some(params) = crate::rmrpc::alloc_params_window(&self.driver, body) else {
            return;
        };
        let Some(idx) = params
            .get(12..16)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        else {
            return;
        };
        if idx != kf_disp::model::NOTIFIERS_HOTPLUG | kf_disp::model::EVENT_CLIENT_RM {
            return;
        }
        let kept = lock(&self.model).register_hotplug(kf_disp::model::HotplugRegistration {
            client,
            event,
            parent,
        });
        eprintln!(
            "kf-rm: display: hotplug event {client:#x}:{event:#x} (parent {parent:#x}) {}",
            if kept {
                "registered"
            } else {
                "NOT registered (too many live registrations)"
            }
        );
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

    /// ★ Step (1): an accepted `GSP_RM_FREE`. The guest's RM sends one per object
    /// (`ogkm-580: rs_client.c:785-843` → `alloc_free.c:959-990`), children before their parent
    /// (`rs_client.c:1085-1092`), so a channel's own free is the usual case; a free of its display object, device or client takes it too (a hostile guest may
    /// free a parent alone, and the object seat drops the subtree).
    fn on_free(&mut self, cmd: &RpcCommand) {
        let m = self.model.clone();
        let Ok(f) = self.driver.decode_free(&cmd.payload) else {
            return;
        };
        let (client, object) = (f.client, f.handle);
        let gone: BTreeSet<u32> = if object == client {
            self.objects
                .keys()
                .filter(|k| k.0 == client)
                .map(|k| k.1)
                .collect()
        } else {
            self.subtree(client, object)
        };
        for h in &gone {
            self.objects.remove(&(client, *h));
        }
        let st = {
            let mut g = lock(&m);
            // ★ 3c: a FREE of the hotplug event, its parent or its client retires the registration
            // (NVKMS frees the event on teardown, `nvkms-rm.c:1917-1924`); a post to a dead pair
            // would wedge the RPC path
            if g.retire_hotplug(client, object) > 0 {
                eprintln!(
                    "kf-rm: display: hotplug registration retired by the FREE of {client:#x}:{object:#x}"
                );
            }
            if object == client {
                g.free_client(client);
            } else {
                for h in &gone {
                    g.free(client, *h);
                }
            }
            g.end_free();
            settle(&mut g)
        };
        finish(st);
    }
}

fn put(p: &mut [u8], off: usize, v: u32) {
    p[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

impl DisplayPolicy {
    /// ★ x11-dispsw: the pairing so far (`None` with the switch off).
    #[must_use]
    pub fn dispsw_pairing(&self) -> Option<DispSwPairing> {
        self.dispsw_pairing
    }

    /// ★ x11-dispsw: note a display-SW constructor's query or alloc ([`DispSwPairing`]) — looks,
    /// never answers; a no-op with the switch off.
    fn note_dispsw(&mut self, cmd: &RpcCommand) {
        let Some(p) = &mut self.dispsw_pairing else {
            return;
        };
        match cmd.function {
            RpcFunction::RmControl
                if self
                    .driver
                    .decode_rpc_control(&cmd.payload)
                    .is_ok_and(|r| r.cmd == GET_ACTIVE_DISPLAY_DEVICES) =>
            {
                if p.query() {
                    eprintln!(
                        "kf-rm: display: x11-dispsw: a GF100_DISP_SW constructor's query was not followed by its alloc \
                         (unpaired={} of {} queries) — that constructor failed after its channel numbered it, so the \
                         channel's later display-SW objects are numbered one past their twins' (no channel is named: \
                         the query carries none)",
                        p.unpaired, p.queries
                    );
                }
            }
            RpcFunction::RmAlloc
                if self
                    .driver
                    .decode_rpc_alloc(cmd.wire_body())
                    .is_ok_and(|h| h.class == GF100_DISP_SW) =>
            {
                p.alloc();
            }
            _ => {}
        }
    }
}

impl CommandPolicy for DisplayPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        self.note_dispsw(cmd);
        match cmd.function {
            RpcFunction::RmControl => self.on_control(cmd),
            // ★ 3c: fn 1 starts every GSP boot — a re-init (a driver reload) makes every hotplug
            // registration of the previous life a dead pair (§40 Tier B). Observed, never answered.
            RpcFunction::SetGuestSystemInfo => {
                if let Some(m) = &self.model {
                    let n = lock(m).retire_all_hotplug();
                    if n > 0 {
                        eprintln!(
                            "kf-rm: display: GSP re-init — {n} hotplug registration(s) retired"
                        );
                    }
                }
                None
            }
            // Lifecycle observation is attached to the object seat's accepted event,
            // not to this speculative position at the front of the command chain.
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

    /// ★ `display-max-fps` (§8.16): the model the guest's controls are answered from carries the
    /// CONFIGURED monitor — the same one `monitors` hands the boot display's option ROM — and unset
    /// is exactly today's 1080p60 monitor. (Mutation: a `model_for` that ignores the property serves
    /// the 75 Hz range to a guest capped at 30.)
    #[test]
    fn the_model_serves_the_configured_monitor() {
        assert_eq!(monitors(0), vec![kf_disp::edid::Monitor::default_1080p()]);
        for fps in [0, 30, 60] {
            let m = model_for(&abi(), &kf_chip::display::AMPERE, fps).expect("derived");
            let mon = &m.connectors.first().expect("a connector").monitor;
            assert_eq!(mon, &monitors(fps)[0], "{fps}");
            let want = if fps == 0 { 75 } else { fps };
            assert_eq!(u32::from(mon.edid().unwrap()[78]), want, "{fps}");
        }
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
        RpcCommand {
            function,
            code: 0,
            sequence: 1,
            payload,
            elements: 1,
            delivered: Vec::new(),
        }
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
        for (i, v) in [client, parent, handle, class, 0, params.len() as u32]
            .iter()
            .enumerate()
        {
            b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(params);
        rpc(RpcFunction::RmAlloc, b)
    }

    /// A `GSP_RM_FREE` (`NVOS00`: hRoot hObjectParent hObjectOld status).
    fn free(client: u32, parent: u32, object: u32) -> RpcCommand {
        rpc(
            RpcFunction::Free,
            [client, parent, object, 0]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect(),
        )
    }

    /// `NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS` (or the PIO one) with `channelInstance`.
    fn chan_params(pio: bool, inst: u32) -> Vec<u8> {
        let mut p = zeroed(if pio {
            "NV50VAIO_CHANNELPIO_ALLOCATION_PARAMETERS"
        } else {
            "NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS"
        });
        p[0..4].copy_from_slice(&inst.to_le_bytes());
        p
    }

    /// ★ x11-dispsw (review 2026-10-03, MEDIUM): with the switch on, a display-SW constructor's
    /// query that is not followed by its alloc before the NEXT query is counted unpaired — the
    /// constructor failed after its channel numbered it. Query → alloc pairs; a trailing open
    /// query is not (yet) counted; other allocs pair nothing. Off: nothing is looked at.
    #[test]
    fn a_display_sw_query_without_its_alloc_is_counted_unpaired() {
        let q = || {
            control(
                super::GET_ACTIVE_DISPLAY_DEVICES,
                0,
                &zeroed("NV2080_CTRL_INTERNAL_DISPLAY_GET_ACTIVE_DISPLAY_DEVICES_PARAMS"),
            )
        };
        let dsw = |h: u32| alloc(0xc1d0_0021, 0xcafe_0013, h, super::GF100_DISP_SW, &[0; 12]);
        let mut off = policy();
        let _ = off.respond(&q());
        let _ = off.respond(&q());
        assert_eq!(off.dispsw_pairing(), None, "off: nothing looked at");
        let mut on = policy().offering_display_sw(true);
        for cmd in [q(), dsw(1), q(), dsw(2)] {
            let _ = on.respond(&cmd);
        }
        assert_eq!(
            on.dispsw_pairing().map(|p| (p.queries, p.unpaired)),
            Some((2, 0)),
            "two constructors, two allocs"
        );
        let _ = on.respond(&q()); // this constructor fails after its query: no alloc
        let _ = on.respond(&alloc(0xc1d0_0021, 0xcafe_0013, 3, 0xc797, &[])); // not display-SW
        assert_eq!(
            on.dispsw_pairing().map(|p| p.unpaired),
            Some(0),
            "not seen yet"
        );
        let _ = on.respond(&q());
        let _ = on.respond(&dsw(4));
        assert_eq!(
            on.dispsw_pairing().map(|p| (p.queries, p.unpaired)),
            Some((4, 1)),
            "the third query's alloc never came"
        );
    }

    /// ★ GA10x: the IP version a real GA106 reports, and a static info with four heads, eight
    /// windows and a channel table large enough for every channel number the guest computes.
    #[test]
    fn ga10x_reports_the_real_ip_version_and_a_four_head_static_info() {
        for mut p in [m0_policy(), policy()] {
            assert_eq!(
                p.answer(GET_IP_VERSION, &[0; 4]),
                Some(Ok(vec![0x00, 0x00, 0x01, 0x04]))
            );
            let si = p
                .answer(GET_STATIC_INFO, &[0; STATIC_INFO_SIZE])
                .expect("ours")
                .expect("ok");
            let w = |o: usize| u32::from_le_bytes(si[o..o + 4].try_into().unwrap());
            assert_eq!(w(0), 0xf, "HEAD_EXISTS 0..3");
            assert_eq!(w(4), 0xff, "windows 0..7");
            assert_eq!(w(12), 4);
            assert_eq!(w(16), NO_I2C_PORT);
            assert_eq!(w(32), NUM_DISP_CHANNELS);
        }
        const {
            assert!(
                NUM_DISP_CHANNELS > 73 + 7,
                "the last cursor channel number fits"
            )
        };
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
            assert_eq!(
                p.stated_inst_mem(),
                Some(InstMem {
                    phys: 0x1234_5000,
                    size: 0x1_0000,
                    addr_space: 2
                })
            );
            assert!(matches!(
                p.answer(INIT_BRIGHTC_STATE_LOAD, &[0u8; 4104]),
                Some(Ok(_))
            ));
            assert!(matches!(
                p.answer(SET_STATIC_EDID_DATA, &[0u8; 8388]),
                Some(Ok(_))
            ));
            assert_eq!(
                p.answer(GET_STATIC_INFO, &[0; 8]),
                Some(Err(NV_ERR_INVALID_ARGUMENT))
            );
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
            (
                INIT_BRIGHTC_STATE_LOAD,
                (0..4104).map(|i| i as u8).collect(),
            ),
            (
                SET_STATIC_EDID_DATA,
                (0..8388).map(|i| (i * 7) as u8).collect(),
            ),
            (WRITE_INST_MEM, im),
        ];
        for (cmd, req) in reqs {
            assert_eq!(m.answer(cmd, &req), m0.answer(cmd, &req), "{cmd:#010x}");
            let c = control(cmd, 0, &req);
            assert_eq!(
                m.respond(&c),
                m0.respond(&c),
                "{cmd:#010x} through respond()"
            );
        }
    }

    /// ★ Step (1): the link's claims are the model's — the M0 set, `CHANNEL_PUSHBUFFER`, and the
    /// NVKMS bring-up controls the m0a run found refused (`0x730101`, `0x730107`, `0x730102`,
    /// `0x730151`) — and a claimed control is answered, not left for the ledger.
    #[test]
    fn the_link_claims_what_the_model_claims() {
        let mut p = policy();
        let m = p.model().expect("the bench driver's layouts are derived");
        for cmd in M0_CONTROLS.into_iter().chain([
            kf_disp::model::CHANNEL_PUSHBUFFER,
            0x0073_0101,
            0x0073_0107,
            0x0073_0102,
            0x0073_0151,
        ]) {
            assert!(p.claims(cmd), "{cmd:#010x}");
            assert!(m.lock().unwrap().claims(cmd), "{cmd:#010x}");
        }
        for cmd in [0x2080_0101, 0x2080_0a70, 0xa06f_0103, 0x0073_ffff] {
            assert_eq!(p.claims(cmd), m.lock().unwrap().claims(cmd), "{cmd:#010x}");
            assert!(!p.claims(cmd), "{cmd:#010x}");
        }
        let s = "NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS";
        let r = p
            .respond(&control(
                k("NV0073_CTRL_CMD_SYSTEM_GET_NUM_HEADS"),
                0,
                &zeroed(s),
            ))
            .expect("answered");
        assert_eq!(r.rpc_result, NV_OK);
        let (off, _) = layouts().field(s, "numHeads").unwrap();
        assert_eq!(
            u32::from_le_bytes(r.body[40 + off..44 + off].try_into().unwrap()),
            4
        );
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
        assert_eq!(
            claimed.len(),
            35 + 6,
            "the NVKMS bring-up set (with the console pair, the display-SW object's query, the \
             internal hotplug state and SET_RMFREE_FLAGS) and the six internal controls"
        );
        assert_eq!(
            claimed.iter().copied().collect::<BTreeSet<u32>>(),
            *p.claimed(),
            "nothing claimed outside the set"
        );
        for cmd in &claimed {
            let r = p
                .respond(&control(*cmd, SERIALIZED, &[0; 16]))
                .expect("refused by name");
            assert_eq!(r.rpc_result, NV_ERR_NOT_SUPPORTED, "{cmd:#010x}");
            let mut short = control(*cmd, 0, &[]);
            short.payload[16..20].copy_from_slice(&64u32.to_le_bytes()); // declares 64, sends 0
            assert_eq!(
                p.respond(&short).map(|r| r.rpc_result),
                Some(NV_ERR_INVALID_ARGUMENT),
                "{cmd:#010x}"
            );
        }
        assert!(
            p.respond(&control(0x2080_0101, SERIALIZED, &[0; 16]))
                .is_none(),
            "not claimed: not ours"
        );
    }

    /// ★ Step (1), on: display channel allocs are recorded in the SHARED model (with their stated
    /// pushbuffer and GET = PUT = offset), frees release them — the channel's own free, its display
    /// object's, and its client's — and nothing is answered: the object seat still answers.
    #[test]
    fn display_allocs_are_tracked_and_frees_release_them() {
        let shared: SharedDisplayModel = Arc::new(Mutex::new(
            model_for(&abi(), &kf_chip::display::AMPERE, 0).expect("derived"),
        ));
        let mut p = DisplayPolicy::over(abi(), &kf_chip::display::AMPERE, shared.clone());
        let mut registry = p.registry().unwrap();
        let (c, dev, disp) = (0xc1d0_0001, 0xcafe_0001, 0xcafe_0070);
        // the core channel's pushbuffer is stated first (internal subdevice), then the objects
        let s = "NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS";
        let mut q = kf_disp::layout::Params::new(layouts(), s, &zeroed(s)).unwrap();
        q.set("hclass", 0xC67D);
        q.set("addressSpace", 1);
        q.set("physicalAddr", 0x1234_5000);
        q.set("limit", 0xfff);
        q.set("valid", 1);
        assert_eq!(
            p.respond(&control(kf_disp::model::CHANNEL_PUSHBUFFER, 0, &q.buf))
                .map(|r| r.rpc_result),
            Some(NV_OK)
        );
        registry.observe(&alloc(c, dev, disp, 0xC670, &[]));
        let mut core = chan_params(false, 0);
        core[12..16].copy_from_slice(&0x40u32.to_le_bytes()); // offset
        registry.observe(&alloc(c, disp, 0xcafe_0d00, 0xC67D, &core));
        for w in 0..8 {
            registry.observe(&alloc(
                c,
                disp,
                0xcafe_0e00 + w,
                0xC67E,
                &chan_params(false, w),
            ));
            registry.observe(&alloc(
                c,
                disp,
                0xcafe_0b00 + w,
                0xC67B,
                &chan_params(false, w),
            ));
        }
        for h in 0..4 {
            registry.observe(&alloc(
                c,
                disp,
                0xcafe_0a00 + h,
                0xC67A,
                &chan_params(true, h),
            ));
        }
        {
            let g = shared.lock().unwrap();
            assert_eq!(g.channels.len(), 1 + 8 + 8 + 4);
            let core = &g.channels[&(ChannelKind::Core, 0)];
            assert_eq!((core.handle, core.offset), (0xcafe_0d00, 0x40));
            assert_eq!(
                (g.ports.get(0), g.ports.put(0)),
                (0x40, 0x40),
                "the shared ports start at the alloc's offset"
            );
            assert_eq!(core.pb.map(|b| b.phys), Some(0x1234_5000));
            assert!(
                g.statements.is_empty(),
                "the link drains the statements into the log"
            );
        }
        // the channel's own free
        registry.observe(&free(c, disp, 0xcafe_0e03));
        assert!(
            !shared
                .lock()
                .unwrap()
                .channels
                .contains_key(&(ChannelKind::Window, 3))
        );
        // a free of the display object takes every channel under it
        registry.observe(&free(c, dev, disp));
        assert!(shared.lock().unwrap().channels.is_empty());
        // and a client's free takes whatever it still held
        registry.observe(&alloc(c, dev, disp, 0xC670, &[]));
        registry.observe(&alloc(c, disp, 0xcafe_0d00, 0xC67D, &chan_params(false, 0)));
        assert_eq!(shared.lock().unwrap().channels.len(), 1);
        registry.observe(&free(c, 0, c));
        assert!(shared.lock().unwrap().channels.is_empty());
        assert!(
            registry.objects.is_empty(),
            "no parent edge outlives its client"
        );
    }

    /// ★ Display step 3c: NVKMS's hotplug event (`0x7e`, `HOTPLUG | CLIENT_RM`) registers in the
    /// narrow seat; another notifier or an OS event does not; the FREE of the event retires it; fn 1
    /// (a GSP re-init) retires everything.
    #[test]
    fn the_hotplug_event_registers_and_retires() {
        let shared: SharedDisplayModel = Arc::new(Mutex::new(
            model_for(&abi(), &kf_chip::display::AMPERE, 0).expect("derived"),
        ));
        let mut p = DisplayPolicy::over_shared(abi(), &kf_chip::display::AMPERE, &shared);
        let mut registry = p.registry().unwrap();
        let ev = |idx: u32| {
            let mut v = vec![0u8; 24];
            v[12..16].copy_from_slice(&idx.to_le_bytes());
            v
        };
        let (c, sub) = (0xc1d0_0002, 0x5c00_2080);
        let target = |s: &SharedDisplayModel| {
            s.lock()
                .unwrap()
                .hotplug_target()
                .map(|r| (r.client, r.event, r.parent))
        };
        registry.observe(&alloc(c, sub, 0xe0, 0x7e, &ev(1 | 0x0400_0000)));
        registry.observe(&alloc(c, sub, 0xe1, 0x7e, &ev(5 | 0x0400_0000)));
        registry.observe(&alloc(c, sub, 0xe2, 0x79, &ev(1 | 0x0400_0000)));
        assert_eq!(target(&shared), Some((c, 0xe0, sub)));
        assert_eq!(
            shared.lock().unwrap().hotplug.len(),
            1,
            "only the hotplug notifier"
        );
        registry.observe(&free(c, sub, 0xe0));
        assert_eq!(target(&shared), None, "retired by its own FREE");
        registry.observe(&alloc(c, sub, 0xe0, 0x7e, &ev(1 | 0x0400_0000)));
        assert!(target(&shared).is_some());
        assert!(
            p.respond(&rpc(RpcFunction::SetGuestSystemInfo, vec![0; 64]))
                .is_none(),
            "fn 1 is observed, never answered"
        );
        assert_eq!(
            target(&shared),
            None,
            "a GSP re-init retires every registration"
        );
    }

    /// ★ Step (3): with a PLANE attached, the link leaves the statements queued for it and wakes it
    /// (after dropping the lock) instead of draining them into the log; a rebuild over the SAME
    /// shared model keeps what the plane will read.
    #[test]
    fn an_attached_plane_gets_the_statements_and_a_wake() {
        let shared: SharedDisplayModel = Arc::new(Mutex::new(
            model_for(&abi(), &kf_chip::display::AMPERE, 0).expect("derived"),
        ));
        let woke = Arc::new(std::sync::atomic::AtomicU32::new(0));
        {
            let w = woke.clone();
            shared
                .lock()
                .unwrap()
                .attach_plane(kf_disp::model::Waker(Arc::new(move || {
                    w.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                })));
        }
        let p = DisplayPolicy::over_shared(abi(), &kf_chip::display::AMPERE, &shared);
        let mut registry = p.registry().unwrap();
        registry.observe(&alloc(
            0xc1d0_0001,
            0xcafe_0070,
            0xcafe_0d00,
            0xC67D,
            &chan_params(false, 0),
        ));
        assert_eq!(woke.load(std::sync::atomic::Ordering::Relaxed), 1);
        // a rebuild (ReselectAtFn1) over the same model: the statement is still there for the plane
        let p2 = DisplayPolicy::over_shared(abi(), &kf_chip::display::AMPERE, &shared);
        assert!(Arc::ptr_eq(&p2.model().unwrap(), &shared));
        let st = shared.lock().unwrap().take_statements();
        assert!(
            matches!(
                st.as_slice(),
                [Statement::ChannelAllocated {
                    kind: ChannelKind::Core,
                    instance: 0,
                    life: 1,
                    ..
                }]
            ),
            "{st:?}"
        );
    }

    /// ★ 2026-10-03 (B5, the review of `v3-gop-unload`): `SET_RMFREE_FLAGS` reaches the model with
    /// the object its `GSP_RM_CONTROL` names, and only a channel allocated under THAT display object
    /// frees preserving (`disp_objs.c:563-578`; NVKMS sends it to `displayHandle`, the channels'
    /// parent, `nvkms-rm.c:2815-2819`, `:3010-3013`).
    #[test]
    fn rmfree_flags_mark_the_display_object_the_control_names() {
        let shared: SharedDisplayModel = Arc::new(Mutex::new(
            model_for(&abi(), &kf_chip::display::AMPERE, 0).expect("derived"),
        ));
        shared
            .lock()
            .unwrap()
            .attach_plane(kf_disp::model::Waker(Arc::new(|| {})));
        let mut p = DisplayPolicy::over_shared(abi(), &kf_chip::display::AMPERE, &shared);
        let mut registry = p.registry().unwrap();
        let (c, dev, disp, core) = (0xc1d0_0001, 0xcafe_0001, 0xcafe_0070, 0xcafe_0d00);
        let f = "NV5070_CTRL_SET_RMFREE_FLAGS_PARAMS";
        let mut q = kf_disp::layout::Params::new(layouts(), f, &zeroed(f)).unwrap();
        q.set("flags", 1);
        let mark = |p: &mut DisplayPolicy, object: u32| {
            let mut cmd = control(k("NV5070_CTRL_CMD_SET_RMFREE_FLAGS"), 0, &q.buf);
            cmd.payload[4..8].copy_from_slice(&object.to_le_bytes());
            p.respond(&cmd).map(|r| r.rpc_result)
        };
        let preserved = || -> Vec<bool> {
            shared
                .lock()
                .unwrap()
                .take_statements()
                .into_iter()
                .filter_map(|st| match st {
                    Statement::ChannelFreed { preserve, .. } => Some(preserve),
                    _ => None,
                })
                .collect()
        };
        registry.observe(&alloc(c, dev, disp, 0xC670, &[]));
        registry.observe(&alloc(c, disp, core, 0xC67D, &chan_params(false, 0)));
        assert_eq!(
            mark(&mut p, 0xcafe_0071),
            Some(NV_OK),
            "another display object"
        );
        registry.observe(&free(c, disp, core));
        assert_eq!(preserved(), vec![false]);
        registry.observe(&alloc(c, disp, core, 0xC67D, &chan_params(false, 0)));
        assert_eq!(mark(&mut p, disp), Some(NV_OK));
        registry.observe(&free(c, disp, core));
        assert_eq!(preserved(), vec![true]);
    }

    /// ★ Hostile guest: an alloc that is not a display class, a channel instance the display does
    /// not have, short or FINN-serialized params, and another family's class are never recorded;
    /// another client's free of the same handle releases nothing; the remembered edges are bounded.
    #[test]
    fn hostile_allocs_and_frees_are_bounded() {
        let p = policy();
        let m = p.model().unwrap();
        let mut registry = p.registry().unwrap();
        let (c, disp) = (0xc1d0_0001, 0xcafe_0070);
        registry.observe(&alloc(
            c,
            0xcafe_0001,
            0xcafe_0e09,
            0xC67E,
            &chan_params(false, 9),
        ));
        registry.observe(&alloc(c, disp, 0xcafe_0a04, 0xC67A, &chan_params(true, 4)));
        registry.observe(&alloc(
            c,
            disp,
            0xcafe_0e00,
            0xC67E,
            &chan_params(false, 0)[..8],
        ));
        registry.observe(&alloc(c, disp, 0xcafe_0e01, 0xC57E, &chan_params(false, 1)));
        let mut ser = alloc(c, disp, 0xcafe_0e02, 0xC67E, &chan_params(false, 2));
        ser.payload[24..28].copy_from_slice(&(1u32 << 1).to_le_bytes());
        registry.observe(&ser);
        registry.observe(&alloc(c, disp, 0xcafe_00c0, 0xC0B5, &[0; 8]));
        assert!(
            m.lock().unwrap().channels.is_empty(),
            "{:?}",
            m.lock().unwrap().channels
        );
        registry.observe(&alloc(c, disp, 0xcafe_0e07, 0xC67E, &chan_params(false, 7)));
        registry.observe(&free(0xc1d0_0002, disp, 0xcafe_0e07));
        registry.observe(&free(0xc1d0_0002, 0, 0xc1d0_0002));
        assert_eq!(
            m.lock().unwrap().channels.len(),
            1,
            "another client's free names another object"
        );
        for i in 0..(MAX_DISPLAY_OBJECTS as u32 + 10) {
            registry.observe(&alloc(c, disp, 0xd000_0000 + i, 0xC372, &[]));
        }
        assert_eq!(registry.objects.len(), MAX_DISPLAY_OBJECTS);
        registry.observe(&free(c, 0xcafe_0001, disp));
        assert!(m.lock().unwrap().channels.is_empty());
    }

    /// ★ The M0 link (a guest driver with no derived display layouts) observes nothing and claims
    /// only the M0 set — the behaviour before step (1).
    #[test]
    fn without_a_model_allocs_are_not_observed() {
        let mut p = m0_policy();
        assert!(p.model().is_none());
        assert!(
            p.respond(&alloc(
                0xc1d0_0001,
                0xcafe_0070,
                0xcafe_0d00,
                0xC67D,
                &chan_params(false, 0)
            ))
            .is_none()
        );
        assert!(p.registry().is_none());
        assert!(
            p.respond(&control(0x0073_0101, 0, &[0; 16])).is_none(),
            "not claimed without the model"
        );
    }

    /// ★ [`GUEST_CACHEABLE`]: the guest keeps these answers for the driver's life, so each is claimed,
    /// named as the export table names it, and identical before and after a display-channel
    /// alloc/free cycle (the only state the link changes today).
    #[test]
    fn guest_cacheable_answers_are_fixed_for_the_device_life() {
        let rows = [
            (
                "NV0073_CTRL_CMD_SYSTEM_GET_SUPPORTED",
                "NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS",
            ),
            (
                "NV0073_CTRL_CMD_SYSTEM_GET_INTERNAL_DISPLAYS",
                "NV0073_CTRL_SYSTEM_GET_INTERNAL_DISPLAYS_PARAMS",
            ),
            (
                "NV0073_CTRL_CMD_SPECIFIC_GET_TYPE",
                "NV0073_CTRL_SPECIFIC_GET_TYPE_PARAMS",
            ),
        ];
        let mut p = policy();
        assert_eq!(
            rows.map(|(c, _)| k(c)),
            GUEST_CACHEABLE,
            "the constant names the derived ids"
        );
        // SPECIFIC_GET_TYPE is asked of a real connector (a zero displayId is INVALID_ARGUMENT, and an
        // error is never cached): the one GET_SUPPORTED names
        let sup = p
            .answer(k(rows[0].0), &zeroed(rows[0].1))
            .expect("claimed")
            .expect("ok");
        let sup = kf_disp::layout::Params::new(layouts(), rows[0].1, &sup).expect("layout");
        let mask = sup.get("displayMask").expect("displayMask");
        assert!(mask != 0, "a connector is supported");
        let mut get_type =
            kf_disp::layout::Params::new(layouts(), rows[2].1, &zeroed(rows[2].1)).expect("layout");
        get_type.set("displayId", mask & mask.wrapping_neg());
        let params = [zeroed(rows[0].1), zeroed(rows[1].1), get_type.buf.clone()];
        let ask = |p: &mut DisplayPolicy| [0, 1, 2].map(|i| p.answer(k(rows[i].0), &params[i]));
        let first = ask(&mut p);
        assert!(
            first.iter().all(|a| matches!(a, Some(Ok(_)))),
            "claimed and answered OK: {first:?}"
        );
        let (c, dev, disp) = (0xc1d0_0001, 0xcafe_0001, 0xcafe_0070);
        let mut registry = p.registry().unwrap();
        registry.observe(&alloc(c, dev, disp, 0xC670, &[]));
        registry.observe(&alloc(c, disp, 0xcafe_0d00, 0xC67D, &chan_params(false, 0)));
        assert_eq!(ask(&mut p), first, "unchanged while a channel is live");
        registry.observe(&free(c, dev, disp));
        assert_eq!(ask(&mut p), first, "unchanged after the free");
    }
}
