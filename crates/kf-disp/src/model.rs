//! ★★ The virtual display's physical-RM side: what the guest's CPU-RM and NVKMS are told
//! (`docs/design/V3_DISPLAY.md` §4.2 (A)).
//!
//! One [`DisplayModel`] per device. It answers the physical-RM display controls from the chip's
//! display row (heads, windows, IP version) and from the monitors behind our connectors (EDIDs we
//! author), and it keeps the registry the display engine executes against: the instance memory the
//! guest declared, each display channel's pushbuffer, and each channel object's handle.
//!
//! Every answer is a pure function of this state and the request. Nothing is forwarded, nothing is
//! read from a capture, and every layout comes from [`crate::layout`] (derived from ogkm).
//!
//! ## The topology we present
//! - `heads` / `windows` from the chip's row (4 / 8 on every family we serve: two windows per head,
//!   windows `2h` and `2h+1` owned by head `h`, the layout NVKMS itself defaults to).
//! - one connector per configured monitor: a DVI-D connector on SOR `i`, protocol
//!   `SINGLE_TMDS_A`, display id `0x100 << i` — no DPCD, no link training, EDID over
//!   `SPECIFIC_GET_EDID_V2` (the simplest connector NVKMS accepts, §4.7).
//! - no crossbar (`SYSTEM_GET_CAPS_V2` clear), so NVKMS uses each output's own SOR and never asks
//!   `DFP_ASSIGN_SOR`.

use crate::edid::Monitor;
use crate::layout::{Layouts, Params};
use crate::ports::Ports;
use std::collections::BTreeMap;
use std::sync::Arc;

/// `NV_OK`.
pub const NV_OK: u32 = 0;
/// `NV_ERR_NOT_SUPPORTED`.
pub const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
/// `NV_ERR_INVALID_ARGUMENT`.
pub const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;
/// `NV_ERR_INVALID_OBJECT` — a channel query for a channel that does not exist.
pub const NV_ERR_INVALID_OBJECT: u32 = 0x31;

/// `NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_STATIC_INFO`.
pub const GET_STATIC_INFO: u32 = 0x2080_0a01;
/// `NV2080_CTRL_CMD_INTERNAL_DISPLAY_WRITE_INST_MEM`.
pub const WRITE_INST_MEM: u32 = 0x2080_0a49;
/// `NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_IP_VERSION`.
pub const GET_IP_VERSION: u32 = 0x2080_0a4b;
/// `NV2080_CTRL_CMD_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER`.
pub const CHANNEL_PUSHBUFFER: u32 = 0x2080_0a58;
/// `NV2080_CTRL_CMD_INTERNAL_INIT_BRIGHTC_STATE_LOAD`.
pub const INIT_BRIGHTC_STATE_LOAD: u32 = 0x2080_0ac6;
/// `NV2080_CTRL_CMD_INTERNAL_SET_STATIC_EDID_DATA`.
pub const SET_STATIC_EDID_DATA: u32 = 0x2080_0adf;

/// `NV402C_CTRL_NUM_I2C_PORTS` — "no external daughterboard" (`kern_disp.c:504-511`).
pub const NO_I2C_PORT: u32 = 16;
/// Display channel numbers run core `0`, windows `1..=32`, window-immediates `33..=64`, cursors
/// `73..=80` (`published/disp/v03_00/dev_disp.h`: `NV_PDISP_CHN_NUM_*`); `clientChannelTable` is
/// indexed by it without a bounds check (`disp_channel.c:254-263`), so the count is one past the
/// last cursor channel.
pub const NUM_DISP_CHANNELS: u32 = 81;
/// ★ Hostile guest: the most [`Statement`]s the model holds for a plane that has not drained them.
/// A boot states ~25 (instance memory + one per channel); past the bound a statement is counted in
/// [`DisplayModel::statements_dropped`] and not kept — the registry (`channels`, `inst_mem`) stays
/// authoritative, so a plane that fell this far behind re-reads it.
pub const MAX_STATEMENTS: usize = 1024;

/// The subdevice-internal display controls (`ctrl2080internal.h`) and the kind of answer each gets.
const INTERNAL_CONTROLS: &[(u32, &str)] = &[
    (GET_IP_VERSION, "ip_version"),
    (GET_STATIC_INFO, "static_info"),
    (INIT_BRIGHTC_STATE_LOAD, "echo"),
    (SET_STATIC_EDID_DATA, "echo"),
    (WRITE_INST_MEM, "inst_mem"),
    (CHANNEL_PUSHBUFFER, "pushbuffer"),
];

/// The NVKMS bring-up controls (§4.2 (A)) by NAME — their ids come from the derived layouts — and
/// the kind of answer each gets.
const NAMED_CONTROLS: &[(&str, &str)] = &[
    ("NV0073_CTRL_CMD_SYSTEM_GET_CAPS_V2", "caps0073"),
    ("NV0073_CTRL_CMD_SYSTEM_GET_NUM_HEADS", "num_heads"),
    ("NV0073_CTRL_CMD_SYSTEM_GET_SUPPORTED", "supported"),
    ("NV0073_CTRL_CMD_SYSTEM_GET_CONNECT_STATE", "connect_state"),
    ("NV0073_CTRL_CMD_SYSTEM_GET_ACTIVE", "active"),
    ("NV0073_CTRL_CMD_SYSTEM_GET_BOOT_DISPLAYS", "boot_displays"),
    (
        "NV0073_CTRL_CMD_SYSTEM_GET_HEAD_ROUTING_MAP",
        "head_routing",
    ),
    ("NV0073_CTRL_CMD_SYSTEM_MAP_SHARED_DATA", "echo"),
    (
        "NV0073_CTRL_CMD_SYSTEM_GET_INTERNAL_DISPLAYS",
        "internal_displays",
    ),
    ("NV0073_CTRL_CMD_SPECIFIC_GET_ALL_HEAD_MASK", "head_mask"),
    (
        "NV0073_CTRL_CMD_SPECIFIC_GET_VALID_HEAD_WINDOW_ASSIGNMENT",
        "window_assign",
    ),
    ("NV0073_CTRL_CMD_SPECIFIC_OR_GET_INFO", "or_info"),
    (
        "NV0073_CTRL_CMD_SPECIFIC_GET_CONNECTOR_DATA",
        "connector_data",
    ),
    ("NV0073_CTRL_CMD_SPECIFIC_GET_TYPE", "get_type"),
    ("NV0073_CTRL_CMD_SPECIFIC_GET_EDID_V2", "get_edid"),
    ("NV0073_CTRL_CMD_SPECIFIC_SET_EDID_V2", "set_edid"),
    ("NV0073_CTRL_CMD_SPECIFIC_GET_PCLK_LIMIT", "pclk_limit"),
    (
        "NV0073_CTRL_CMD_SPECIFIC_IS_DIRECTMODE_DISPLAY",
        "directmode",
    ),
    ("NV0073_CTRL_CMD_SPECIFIC_DISPLAY_CHANGE", "echo"),
    (
        "NV0073_CTRL_CMD_SPECIFIC_GET_BACKLIGHT_BRIGHTNESS",
        "not_supported",
    ),
    ("NV0073_CTRL_CMD_DFP_GET_INFO", "dfp_info"),
    ("NV0073_CTRL_CMD_DFP_GET_DISPLAYPORT_DONGLE_INFO", "dongle"),
    ("NV5070_CTRL_CMD_SYSTEM_GET_CAPS_V2", "caps5070"),
    ("NVC370_CTRL_CMD_IDLE_CHANNEL", "echo"),
    ("NVC370_CTRL_CMD_SET_ACCL", "echo"),
    ("NVC370_CTRL_CMD_GET_ACCL", "get_accl"),
    ("NVC370_CTRL_CMD_GET_CHANNEL_INFO", "channel_info"),
    ("NVC370_CTRL_CMD_GET_LOCKPINS_CAPS", "lockpins"),
    ("NVC370_CTRL_CMD_SET_SWAPRDY_GPIO_WAR", "echo"),
    ("NVC372_CTRL_CMD_IS_MODE_POSSIBLE", "mode_possible"),
    // ★ 2026-10-03 (B5, `V3_DISPLAY.md` §4.11.13): ROUTE_TO_PHYSICAL (`g_disp_objs_nvoc.c`, flags
    // 0x40), so it reaches us. NVKMS sends PRESERVE_HW before freeing each channel after it restored
    // the console (`nvkms-rm.c:2990-3017`): the display keeps scanning the console through the free.
    ("NV5070_CTRL_CMD_SET_RMFREE_FLAGS", "rmfree_flags"),
    // ★ M1 (`[measured m1a, 2026-09-30, GA106 / 580.159.04]` the ledger held 0x20800a76 after
    // nvidia-drm's fbdev took the console): the VGA console save/restore around a console switch
    // (`unix_console.c:74-140`). Our virtual engine has no VGA console and no VBIOS mode to save:
    // `bReturnEarly`, nothing to restore.
    (
        "NV2080_CTRL_CMD_INTERNAL_DISPLAY_PRE_UNIX_CONSOLE",
        "pre_console",
    ),
    ("NV2080_CTRL_CMD_INTERNAL_DISPLAY_POST_UNIX_CONSOLE", "echo"),
    // ★ M3: the NV9072 (GF100_DISP_SW) display-SW object's constructor asks physical RM which
    // displays are active and how many heads exist (`disp_sw.c:44-101`). ⊘ REFUSED BY NAME, and
    // that is measured: `[m3c, 2026-09-30, kf3 at 4475b9fe]` answering it lets the X driver and GL
    // allocate the object, whose methods are SOFTWARE methods RM services when the host engine
    // traps them — but the guest's channels run on the host GPU, whose RM has no such object: 186
    // host `Xid 32` (invalid pushbuffer stream), glxgears at 1.3 FPS, vkQueueSubmit failing
    // (commit `9adb26a8`; no m3c trace is committed). Refused
    // (`[m3b, 2026-09-30, GA106 / 580.159.04]`), X logs "Failed to allocate display software
    // resources" and GL runs vsync-locked at 60 FPS.
    (
        "NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_ACTIVE_DISPLAY_DEVICES",
        "no_display_sw",
    ),
];

/// The display classes a chip lists (a copy of the chip row's, so this crate owns its inputs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classes {
    /// `NV04_DISPLAY_COMMON`.
    pub common: u32,
    /// The display object.
    pub display: u32,
    /// Core channel.
    pub core: u32,
    /// Window channel.
    pub window: u32,
    /// Window-immediate channel.
    pub window_imm: u32,
    /// Cursor PIO channel.
    pub cursor: u32,
    /// `NVC372_DISPLAY_SW`.
    pub disp_sw: u32,
}

impl Classes {
    /// From the chip's display row.
    #[must_use]
    pub fn of(r: &kf_chip::display::DisplayRow) -> Classes {
        let c = &r.classes;
        Classes {
            common: c.common,
            display: c.display,
            core: c.core,
            window: c.window,
            window_imm: c.window_imm,
            cursor: c.cursor,
            disp_sw: c.disp_sw,
        }
    }

    /// Which channel kind `class` is, if it is one of ours.
    #[must_use]
    pub fn channel_kind(&self, class: u32) -> Option<ChannelKind> {
        match class {
            c if c == self.core => Some(ChannelKind::Core),
            c if c == self.window => Some(ChannelKind::Window),
            c if c == self.window_imm => Some(ChannelKind::WindowImm),
            c if c == self.cursor => Some(ChannelKind::Cursor),
            _ => None,
        }
    }
}

/// The four display channel kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChannelKind {
    /// The core channel (one).
    Core,
    /// A window channel (per window).
    Window,
    /// A window-immediate channel (per window).
    WindowImm,
    /// A cursor PIO channel (per head).
    Cursor,
}

impl ChannelKind {
    /// The hardware channel number (`NV_PDISP_CHN_NUM_*`, `dev_disp.h` v03_00) for `instance`.
    #[must_use]
    pub fn channel_number(self, instance: u32) -> u32 {
        match self {
            ChannelKind::Core => 0,
            ChannelKind::Window => 1 + instance,
            ChannelKind::WindowImm => 33 + instance,
            ChannelKind::Cursor => 73 + instance,
        }
    }

    /// The channel's user page in BAR0 (`NV_UDISP_FE_CHN_ASSY_BASEADR_*`, `dev_disp.h` v03_00).
    #[must_use]
    pub fn user_base(self, instance: u32) -> u64 {
        match self {
            ChannelKind::Core => 0x0068_0000,
            ChannelKind::Window => 0x0069_0000 + u64::from(instance) * 0x1000,
            ChannelKind::WindowImm => 0x006B_0000 + u64::from(instance) * 0x1000,
            ChannelKind::Cursor => 0x006D_8000 + u64::from(instance) * 0x1000,
        }
    }
}

/// Where a channel's pushbuffer is (`CHANNEL_PUSHBUFFER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pushbuffer {
    /// `ADDR_SYSMEM` (1) or `ADDR_FBMEM` (2).
    pub addr_space: u32,
    /// Physical address: a guest physical address (sysmem) or an FB offset (vidmem).
    pub phys: u64,
    /// The ctxdma limit (inclusive).
    pub limit: u64,
    /// `channelPBSize` enum: 4 KiB `<< n`.
    pub size_enum: u32,
}

impl Pushbuffer {
    /// The pushbuffer's size in bytes (≤ 64 KiB by the enum; the decoder refuses > 4 KiB).
    #[must_use]
    pub fn bytes(&self) -> u32 {
        4096u32 << self.size_enum.min(4)
    }
}

/// One display channel object the guest allocated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    /// Its class.
    pub class: u32,
    /// Core, window, window-immediate or cursor.
    pub kind: ChannelKind,
    /// `channelInstance` (window / head index).
    pub instance: u32,
    /// The owning client and handle (a free names them).
    pub client: u32,
    /// The channel object's handle.
    pub handle: u32,
    /// Its pushbuffer (`None` for the cursor PIO channel).
    pub pb: Option<Pushbuffer>,
    /// The initial GET = PUT (byte offset) the alloc stated. ⊘ The live GET/PUT are the shared
    /// [`Ports`] words — the vCPU posts PUT there and the engine publishes GET — never a copy here.
    pub offset: u32,
    /// The allocation generation [`Ports::allocate`] minted for this life.
    pub life: u32,
}

/// The guest's display instance memory (`WRITE_INST_MEM`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstMem {
    /// Physical address (FB offset or guest physical address, per `addr_space`).
    pub phys: u64,
    /// Size in bytes.
    pub size: u64,
    /// `ADDR_FBMEM` (2) or `ADDR_SYSMEM` (1).
    pub addr_space: u32,
}

/// One connector and the monitor behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connector {
    /// The display id (one bit).
    pub display_id: u32,
    /// The SOR index.
    pub or_index: u32,
    /// The monitor.
    pub monitor: Monitor,
    /// A custom EDID the guest set (`SPECIFIC_SET_EDID_V2`), shadowing the monitor's.
    pub custom_edid: Option<Vec<u8>>,
}

/// ★ Something the display engine must do because of a control or an alloc (drained by the plane).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    /// `WRITE_INST_MEM`: the guest placed instance memory here — the plane zeroes it (the guest
    /// never does, `disp_inst_mem.c:170-201`) before any ctxdma is bound.
    InstMem(InstMem),
    /// A display channel came into being (its GET = PUT = `offset` from the alloc params).
    ChannelAllocated {
        /// Kind.
        kind: ChannelKind,
        /// Instance.
        instance: u32,
        /// Initial GET/PUT.
        offset: u32,
        /// The allocating client (the context-DMA hash key).
        client: u32,
        /// Its pushbuffer, as `CHANNEL_PUSHBUFFER` stated it (`None`: PIO, or never stated).
        pb: Option<Pushbuffer>,
        /// The generation the shared ports minted for this life.
        life: u32,
    },
    /// A display channel was freed.
    ChannelFreed {
        /// Kind.
        kind: ChannelKind,
        /// Instance.
        instance: u32,
        /// ★ 2026-10-03 (B5, `V3_DISPLAY.md` §4.11.13): the free carried
        /// `NV5070_CTRL_SET_RMFREE_FLAGS_PRESERVE_HW` — the display hardware keeps scanning what
        /// it scans (NVKMS sets it after a console restore, `nvkms-rm.c:2990-3017`).
        preserve: bool,
    },
}

/// ★ The model.
#[derive(Debug)]
pub struct DisplayModel {
    l: &'static Layouts,
    /// The chip's IP version.
    pub ip_version: u32,
    /// Heads presented.
    pub heads: u32,
    /// Windows presented.
    pub windows: u32,
    /// Classes.
    pub classes: Classes,
    /// Connectors, in display-id order.
    pub connectors: Vec<Connector>,
    /// Instance memory, once stated.
    pub inst_mem: Option<InstMem>,
    /// Pushbuffers by `(class, channelInstance)` — stated BEFORE the channel's alloc.
    pub pushbuffers: BTreeMap<(u32, u32), Pushbuffer>,
    /// Live channels by `(kind, instance)`.
    pub channels: BTreeMap<(ChannelKind, u32), Channel>,
    /// Statements for the plane, in order (at most [`MAX_STATEMENTS`]).
    pub statements: Vec<Statement>,
    /// Statements not kept because [`Self::statements`] was full.
    pub statements_dropped: u64,
    /// Every display control answered (the first 512), for the log.
    pub seen: Vec<u32>,
    /// ★ The lock-free state shared with the vCPU and the display worker (PUT/GET, events).
    pub ports: Arc<Ports>,
    /// ★ The display plane's wake: set when a plane consumes [`Self::statements`]; the control link
    /// then leaves them queued and calls it (after dropping the lock) instead of logging them.
    waker: Option<Waker>,
    /// ★ `NV5070_CTRL_CMD_SET_RMFREE_FLAGS` PRESERVE_HW, for the NEXT `RmFree` only
    /// (`ctrl5070chnc.h:899-918`); cleared by [`Self::end_free`].
    rmfree_preserve: bool,
}

/// The display plane's wake (an eventfd write, in the plane) — callable from any thread.
#[derive(Clone)]
pub struct Waker(pub Arc<dyn Fn() + Send + Sync>);

impl core::fmt::Debug for Waker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Waker")
    }
}

impl Waker {
    /// Wake the plane.
    pub fn wake(&self) {
        (self.0)();
    }
}

impl DisplayModel {
    /// A model for a chip's display row with `monitors` behind connectors `0..n`, answering with the
    /// layouts of `layouts`.
    #[must_use]
    pub fn new(
        row: &kf_chip::display::DisplayRow,
        monitors: Vec<Monitor>,
        layouts: &'static Layouts,
    ) -> DisplayModel {
        let connectors = monitors
            .into_iter()
            .enumerate()
            .take(row.heads as usize)
            .map(|(i, monitor)| Connector {
                display_id: 0x100 << i,
                or_index: i as u32,
                monitor,
                custom_edid: None,
            })
            .collect();
        DisplayModel {
            l: layouts,
            ip_version: row.ip_version,
            heads: row.heads.min(8),
            windows: row.windows.min(32),
            classes: Classes::of(row),
            connectors,
            inst_mem: None,
            pushbuffers: BTreeMap::new(),
            channels: BTreeMap::new(),
            statements: Vec::new(),
            statements_dropped: 0,
            seen: Vec::new(),
            ports: Arc::new(Ports::default()),
            waker: None,
            rmfree_preserve: false,
        }
    }

    /// ★ Attach the display plane: it drains [`Self::statements`] itself and is woken through `wake`.
    pub fn attach_plane(&mut self, wake: Waker) {
        self.waker = Some(wake);
    }

    /// The plane's wake, if a plane is attached (the control link calls it after dropping the lock).
    #[must_use]
    pub fn waker(&self) -> Option<Waker> {
        self.waker.clone()
    }

    /// ★ Answer with another driver version's `layouts` (a `ReselectAtFn1` rebuild for the guest's
    /// own version). Everything stated so far is kept; the ports and the plane stay attached.
    pub fn retarget(&mut self, layouts: &'static Layouts) {
        self.l = layouts;
    }

    /// ★ Hostile guest: how many instances of `kind` this display has — the bound every
    /// guest-stated `channelInstance` is checked against before it keys the registry. One core
    /// channel; a window and a window-immediate channel per window; a cursor per head
    /// (`kdispGetChannelNum_v03_00`, `ogkm-580: kern_disp_0300.c:47-125`, refuses the same window
    /// and cursor range on the guest's side before it RPCs; it does not check a core instance, and
    /// NVKMS allocates the core channel as instance 0, `nvkms-rm.c:3375-3384` — so a well-formed
    /// guest never meets this bound).
    #[must_use]
    pub fn instances(&self, kind: ChannelKind) -> u32 {
        match kind {
            ChannelKind::Core => 1,
            ChannelKind::Window | ChannelKind::WindowImm => self.windows,
            ChannelKind::Cursor => self.heads,
        }
    }

    fn state(&mut self, s: Statement) {
        if self.statements.len() < MAX_STATEMENTS {
            self.statements.push(s);
        } else {
            self.statements_dropped += 1;
        }
    }

    /// The layouts this model answers with.
    #[must_use]
    pub fn layouts(&self) -> &'static Layouts {
        self.l
    }

    fn connector(&self, display_id: u32) -> Option<&Connector> {
        self.connectors.iter().find(|c| c.display_id == display_id)
    }

    fn all_displays(&self) -> u32 {
        self.connectors.iter().fold(0, |m, c| m | c.display_id)
    }

    /// Is `cmd` one of this model's controls? (What a chain link claims.)
    #[must_use]
    pub fn claims(&self, cmd: u32) -> bool {
        self.kind_of(cmd).is_some()
    }

    /// ★ Every control this model claims, in a fixed order: the subdevice-internal ones, then the
    /// named ones whose ids the derived layouts carry (a name the TSV lacks is not claimed — never
    /// a guessed id). A link caches this set; [`Self::claims`] is membership in it.
    #[must_use]
    pub fn claimed(&self) -> Vec<u32> {
        INTERNAL_CONTROLS
            .iter()
            .map(|(c, _)| *c)
            .chain(NAMED_CONTROLS.iter().filter_map(|(n, _)| self.l.k32(n)))
            .collect()
    }

    fn kind_of(&self, cmd: u32) -> Option<&'static str> {
        if let Some((_, k)) = INTERNAL_CONTROLS.iter().find(|(c, _)| *c == cmd) {
            return Some(k);
        }
        NAMED_CONTROLS
            .iter()
            .find(|(n, _)| self.l.k32(n) == Some(cmd))
            .map(|(_, k)| *k)
    }

    /// ★ Answer one display control: `Some(Ok(reply params))`, `Some(Err(status))`, or `None` when
    /// the control is not the display plane's.
    pub fn control(&mut self, cmd: u32, params: &[u8]) -> Option<Result<Vec<u8>, u32>> {
        let kind = self.kind_of(cmd)?;
        if self.seen.len() < 512 {
            self.seen.push(cmd);
        }
        Some(self.answer(kind, params))
    }

    fn view<'a>(&self, s: &'a str, params: &[u8]) -> Result<Params<'a>, u32> {
        Params::new(self.l, s, params).ok_or(NV_ERR_INVALID_ARGUMENT)
    }

    fn answer(&mut self, kind: &str, params: &[u8]) -> Result<Vec<u8>, u32> {
        let l = self.l;
        let k = |n: &str| l.konst(n).ok_or(NV_ERR_NOT_SUPPORTED);
        match kind {
            "echo" => Ok(params.to_vec()),
            "not_supported" => Err(NV_ERR_NOT_SUPPORTED),
            "ip_version" => {
                if params.len() != 4 {
                    return Err(NV_ERR_INVALID_ARGUMENT);
                }
                Ok(self.ip_version.to_le_bytes().to_vec())
            }
            "static_info" => {
                let mut p = self.view(
                    "NV2080_CTRL_INTERNAL_DISPLAY_GET_STATIC_INFO_PARAMS",
                    params,
                )?;
                p.buf.fill(0);
                p.set("feHwSysCap", u64::from((1u32 << self.heads) - 1)); // HEAD_EXISTS(i) = bit i
                let wmask = if self.windows >= 32 {
                    u32::MAX
                } else {
                    (1u32 << self.windows) - 1
                };
                p.set("windowPresentMask", u64::from(wmask));
                p.set("numHeads", u64::from(self.heads));
                p.set("i2cPort", u64::from(NO_I2C_PORT));
                p.set("numDispChannels", u64::from(NUM_DISP_CHANNELS));
                Ok(p.buf)
            }
            "inst_mem" => {
                let p = self.view("NV2080_CTRL_INTERNAL_DISPLAY_WRITE_INST_MEM_PARAMS", params)?;
                let im = InstMem {
                    phys: p.get("instMemPhysAddr").unwrap_or(0),
                    size: p.get("instMemSize").unwrap_or(0),
                    addr_space: p.get("instMemAddrSpace").unwrap_or(0) as u32,
                };
                self.inst_mem = Some(im);
                self.state(Statement::InstMem(im));
                Ok(p.buf)
            }
            "pushbuffer" => {
                let p = self.view(
                    "NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS",
                    params,
                )?;
                let class = p.get("hclass").unwrap_or(0) as u32;
                let inst = p.get("channelInstance").unwrap_or(0) as u32;
                // ★ Hostile guest: `(class, instance)` keys the registry, so both are bounded —
                // one of this family's channel classes, an instance that exists. The guest's CPU-RM
                // sends it for every DMA channel (`valid`) and every PIO channel (`!valid`), after
                // its own channel-number check (`disp_channel.c:786-863`), and ignores the status.
                match self.classes.channel_kind(class) {
                    Some(kd) if inst < self.instances(kd) => {}
                    _ => return Err(NV_ERR_INVALID_ARGUMENT),
                }
                if p.get("valid").unwrap_or(0) != 0 {
                    self.pushbuffers.insert(
                        (class, inst),
                        Pushbuffer {
                            addr_space: p.get("addressSpace").unwrap_or(0) as u32,
                            phys: p.get("physicalAddr").unwrap_or(0),
                            limit: p.get("limit").unwrap_or(0),
                            size_enum: p.get("channelPBSize").unwrap_or(0) as u32,
                        },
                    );
                } else {
                    self.pushbuffers.remove(&(class, inst));
                }
                Ok(p.buf)
            }
            "caps0073" => {
                let mut p = self.view("NV0073_CTRL_SYSTEM_GET_CAPS_V2_PARAMS", params)?;
                p.set_bytes("capsTbl", &[]); // no crossbar, no MIO power quirk, no ACR bug
                Ok(p.buf)
            }
            "caps5070" => {
                let mut p = self.view("NV5070_CTRL_SYSTEM_GET_CAPS_V2_PARAMS", params)?;
                p.set_bytes("capsTbl", &[]); // ⊘ never BUG_644815_DNISO_VIDMEM_ONLY: pushbuffers stay in sysmem
                Ok(p.buf)
            }
            "num_heads" => {
                let mut p = self.view("NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS", params)?;
                p.set("numHeads", u64::from(self.heads));
                Ok(p.buf)
            }
            "head_mask" => {
                let mut p = self.view("NV0073_CTRL_SPECIFIC_GET_ALL_HEAD_MASK_PARAMS", params)?;
                p.set("headMask", u64::from((1u32 << self.heads) - 1));
                Ok(p.buf)
            }
            "window_assign" => {
                let mut p = self.view(
                    "NV0073_CTRL_SPECIFIC_GET_VALID_HEAD_WINDOW_ASSIGNMENT_PARAMS",
                    params,
                )?;
                let masks: Vec<u8> = (0..self.windows).map(|w| (1u32 << (w / 2)) as u8).collect();
                p.set_bytes("windowHeadMask", &masks);
                Ok(p.buf)
            }
            "supported" => {
                let mut p = self.view("NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS", params)?;
                let all = u64::from(self.all_displays());
                p.set("displayMask", all);
                p.set("displayMaskDDC", all);
                Ok(p.buf)
            }
            "connect_state" => {
                let mut p = self.view("NV0073_CTRL_SYSTEM_GET_CONNECT_STATE_PARAMS", params)?;
                let asked = p.get("displayMask").unwrap_or(0);
                p.set("displayMask", asked & u64::from(self.all_displays()));
                p.set("retryTimeMs", 0);
                Ok(p.buf)
            }
            "active" => {
                // ★ the connector on the SOR the head's ARMED state drives while its raster runs (the
                // worker publishes it, `Engine::lit_sors`); 0 when the head lights nothing — as at boot
                let mut p = self.view("NV0073_CTRL_SYSTEM_GET_ACTIVE_PARAMS", params)?;
                let head = p.get("head").unwrap_or(u64::MAX);
                let id = usize::try_from(head)
                    .ok()
                    .and_then(|h| self.ports.lit_sor(h))
                    .and_then(|sor| self.connectors.iter().find(|c| c.or_index == sor))
                    .map_or(0, |c| c.display_id);
                p.set("displayId", u64::from(id));
                Ok(p.buf)
            }
            "boot_displays" => {
                let mut p = self.view("NV0073_CTRL_SYSTEM_GET_BOOT_DISPLAYS_PARAMS", params)?;
                p.set("bootDisplayMask", 0);
                Ok(p.buf)
            }
            "internal_displays" => {
                let mut p = self.view("NV0073_CTRL_SYSTEM_GET_INTERNAL_DISPLAYS_PARAMS", params)?;
                p.set("availableInternalDisplaysMask", 0);
                Ok(p.buf)
            }
            "head_routing" => {
                // NVKMS checks only that every requested display comes back (`nvkms-rm.c:4753-4824`);
                // one display per head is always routable here.
                let mut p = self.view("NV0073_CTRL_SYSTEM_GET_HEAD_ROUTING_MAP_PARAMS", params)?;
                let asked = p.get("displayMask").unwrap_or(0) & u64::from(self.all_displays());
                p.set("displayMask", asked);
                Ok(p.buf)
            }
            "or_info" => {
                let mut p = self.view("NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS", params)?;
                let id = p.get("displayId").unwrap_or(0) as u32;
                let c = self.connector(id).ok_or(NV_ERR_INVALID_ARGUMENT)?.clone();
                p.set("index", u64::from(c.or_index));
                p.set("type", k("NV0073_CTRL_SPECIFIC_OR_TYPE_SOR")?);
                p.set(
                    "protocol",
                    k("NV0073_CTRL_SPECIFIC_OR_PROTOCOL_SOR_SINGLE_TMDS_A")?,
                );
                p.set("location", k("NV0073_CTRL_SPECIFIC_OR_LOCATION_CHIP")?);
                p.set("bIsLitByVbios", 0);
                p.set("bIsDispDynamic", 0);
                Ok(p.buf)
            }
            "connector_data" => {
                let mut p = self.view("NV0073_CTRL_SPECIFIC_GET_CONNECTOR_DATA_PARAMS", params)?;
                let id = p.get("displayId").unwrap_or(0) as u32;
                let c = self.connector(id).ok_or(NV_ERR_INVALID_ARGUMENT)?.clone();
                p.set("flags", 0);
                p.set("count", 1);
                p.set("data[0].index", u64::from(c.or_index));
                p.set(
                    "data[0].type",
                    k("NV0073_CTRL_SPECIFIC_CONNECTOR_DATA_TYPE_DVI_D")?,
                );
                p.set("data[0].location", 0);
                p.set(
                    "platform",
                    k("NV0073_CTRL_SPECIFIC_CONNECTOR_PLATFORM_DEFAULT_ADD_IN_CARD")?,
                );
                Ok(p.buf)
            }
            "get_type" => {
                let mut p = self.view("NV0073_CTRL_SPECIFIC_GET_TYPE_PARAMS", params)?;
                let id = p.get("displayId").unwrap_or(0) as u32;
                self.connector(id).ok_or(NV_ERR_INVALID_ARGUMENT)?;
                p.set("displayType", k("NV0073_CTRL_SPECIFIC_DISPLAY_TYPE_DFP")?);
                Ok(p.buf)
            }
            "get_edid" => {
                let mut p = self.view("NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS", params)?;
                let id = p.get("displayId").unwrap_or(0) as u32;
                let c = self.connector(id).ok_or(NV_ERR_INVALID_ARGUMENT)?.clone();
                let edid = match &c.custom_edid {
                    Some(e) => e.clone(),
                    None => c.monitor.edid().map_err(|_| NV_ERR_NOT_SUPPORTED)?.to_vec(),
                };
                if !p.set_bytes("edidBuffer", &edid) {
                    return Err(NV_ERR_INVALID_ARGUMENT);
                }
                p.set("bufferSize", edid.len() as u64);
                Ok(p.buf)
            }
            "set_edid" => {
                let p = self.view("NV0073_CTRL_SPECIFIC_SET_EDID_V2_PARAMS", params)?;
                let id = p.get("displayId").unwrap_or(0) as u32;
                let n = p.get("bufferSize").unwrap_or(0) as usize;
                let buf = p.bytes("edidBuffer").ok_or(NV_ERR_INVALID_ARGUMENT)?;
                if n > buf.len() {
                    return Err(NV_ERR_INVALID_ARGUMENT);
                }
                let custom = (n > 0).then(|| buf[..n].to_vec());
                let c = self
                    .connectors
                    .iter_mut()
                    .find(|c| c.display_id == id)
                    .ok_or(NV_ERR_INVALID_ARGUMENT)?;
                c.custom_edid = custom;
                Ok(p.buf)
            }
            "pclk_limit" => {
                let mut p = self.view("NV0073_CTRL_SPECIFIC_GET_PCLK_LIMIT_PARAMS", params)?;
                let id = p.get("displayId").unwrap_or(0) as u32;
                let khz = u64::from(
                    self.connector(id)
                        .ok_or(NV_ERR_INVALID_ARGUMENT)?
                        .monitor
                        .max_pixel_khz,
                );
                p.set("pclkLimit", khz);
                p.set("orPclkLimit", khz);
                p.set("vbPclkLimit", khz);
                Ok(p.buf)
            }
            "directmode" => {
                let mut p =
                    self.view("NV0073_CTRL_SPECIFIC_IS_DIRECTMODE_DISPLAY_PARAMS", params)?;
                p.set("bIsDirectmode", 0);
                Ok(p.buf)
            }
            "dfp_info" => {
                let mut p = self.view("NV0073_CTRL_DFP_GET_INFO_PARAMS", params)?;
                let id = p.get("displayId").unwrap_or(0) as u32;
                self.connector(id).ok_or(NV_ERR_INVALID_ARGUMENT)?;
                // SIGNAL (2:0) = TMDS, LINK (21:20) = SINGLE; not HDMI-capable (a DVI monitor)
                let flags = k("NV0073_CTRL_DFP_FLAGS_SIGNAL_TMDS")?
                    | (k("NV0073_CTRL_DFP_FLAGS_LINK_SINGLE")? << 20);
                p.set("flags", flags);
                p.set("UHBRSupportedByDfp", 0);
                Ok(p.buf)
            }
            "dongle" => {
                let mut p =
                    self.view("NV0073_CTRL_DFP_GET_DISPLAYPORT_DONGLE_INFO_PARAMS", params)?;
                p.set(
                    "flags",
                    k("NV0073_CTRL_DFP_GET_DISPLAYPORT_DONGLE_INFO_FLAGS_ATTACHED_FALSE")?,
                );
                p.set("maxTmdsClkRateHz", 0);
                Ok(p.buf)
            }
            "get_accl" => {
                let mut p = self.view("NVC370_CTRL_GET_ACCL_PARAMS", params)?;
                p.set("accelerators", 0);
                Ok(p.buf)
            }
            "lockpins" => {
                let mut p = self.view("NVC370_CTRL_GET_LOCKPINS_CAPS_PARAMS", params)?;
                p.set(
                    "frameLockPin",
                    k("NVC370_CTRL_GET_LOCKPINS_CAPS_FRAME_LOCK_PIN_NONE")?,
                );
                p.set(
                    "rasterLockPin",
                    k("NVC370_CTRL_GET_LOCKPINS_CAPS_RASTER_LOCK_PIN_NONE")?,
                );
                p.set(
                    "flipLockPin",
                    k("NVC370_CTRL_GET_LOCKPINS_CAPS_FLIP_LOCK_PIN_NONE")?,
                );
                p.set(
                    "stereoPin",
                    k("NVC370_CTRL_GET_LOCKPINS_CAPS_STEREO_PIN_NONE")?,
                );
                p.set("numScanLockPins", 0);
                p.set("numFlipLockPins", 0);
                p.set("numStereoPins", 0);
                Ok(p.buf)
            }
            "channel_info" => {
                let mut p = self.view("NVC370_CTRL_CMD_GET_CHANNEL_INFO_PARAMS", params)?;
                let class = p.get("channelClass").unwrap_or(0) as u32;
                let inst = p.get("channelInstance").unwrap_or(0) as u32;
                // ★ idle = the engine has consumed (and published the effects of) everything the guest
                // posted: GET == PUT on the shared ports — a PUT is visible here the instant the vCPU
                // stored it, a GET only after its notifier/semaphore/armed state is out.
                let state = match self
                    .classes
                    .channel_kind(class)
                    .and_then(|kd| self.channels.get(&(kd, inst)))
                {
                    Some(ch) if self.ports.idle(ch.kind.channel_number(ch.instance)) => {
                        k("NVC370_CTRL_GET_CHANNEL_INFO_STATE_IDLE")?
                    }
                    Some(_) => k("NVC370_CTRL_GET_CHANNEL_INFO_STATE_BUSY")?,
                    None => k("NVC370_CTRL_GET_CHANNEL_INFO_STATE_DEALLOC")?,
                };
                p.set("IsChannelInDebugMode", 0);
                p.set("channelState", state);
                Ok(p.buf)
            }
            // see NAMED_CONTROLS: the display-SW object is not offered
            "no_display_sw" => Err(NV_ERR_NOT_SUPPORTED),
            "pre_console" => {
                let mut p = self.view(
                    "NV2080_CTRL_CMD_INTERNAL_DISPLAY_PRE_UNIX_CONSOLE_PARAMS",
                    params,
                )?;
                p.set("bReturnEarly", 1);
                Ok(p.buf)
            }
            "rmfree_flags" => {
                let p = self.view("NV5070_CTRL_SET_RMFREE_FLAGS_PARAMS", params)?;
                let preserve = k("NV5070_CTRL_SET_RMFREE_FLAGS_PRESERVE_HW")?;
                self.rmfree_preserve = p.get("flags").unwrap_or(0) & preserve != 0;
                Ok(p.buf)
            }
            "mode_possible" => {
                // A virtual head has no isochronous memory pool to exhaust: every mode NVKMS validated
                // against the EDID and the pixel-clock limit is possible. The bandwidth numbers are
                // what NVKMS carries forward (`nvkms-evo3.c:3440-3451`), so they are stated, not zero.
                let mut p = self.view("NVC372_CTRL_IS_MODE_POSSIBLE_PARAMS", params)?;
                p.set("bIsPossible", 1);
                p.set("minImpVPState", 0);
                p.set("minPState", 0);
                p.set("minRequiredBandwidthKBPS", 1_000_000);
                p.set("floorBandwidthKBPS", 1_000_000);
                p.set("minRequiredHubclkKHz", 0);
                p.set("dispClkKHz", 1_000_000);
                p.set("worstCaseMargin", 0);
                Ok(p.buf)
            }
            _ => Err(NV_ERR_NOT_SUPPORTED),
        }
    }

    /// ★ A display object the guest allocated (after the RPC's header was decoded). Returns `true`
    /// when it was one of ours and was recorded.
    ///
    /// ⊘ Hostile guest: params that are not the derived allocation struct, or an instance this
    /// display does not have ([`Self::instances`]), are NOT recorded — never read as instance 0.
    /// The registry is keyed by `(kind, instance)`, so it holds at most one entry per channel the
    /// display has, whatever the guest sends. (The guest's CPU-RM does not check a CORE instance,
    /// `kern_disp_0300.c:96-99`: there is one core channel, number 0, and NVKMS allocates it as
    /// instance 0.)
    pub fn alloc(&mut self, client: u32, handle: u32, class: u32, params: &[u8]) -> bool {
        let Some(kind) = self.classes.channel_kind(class) else {
            return false;
        };
        let decoded = if kind == ChannelKind::Cursor {
            Params::new(self.l, "NV50VAIO_CHANNELPIO_ALLOCATION_PARAMETERS", params)
                .and_then(|p| Some((p.get("channelInstance")? as u32, 0)))
        } else {
            Params::new(self.l, "NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS", params)
                .and_then(|p| Some((p.get("channelInstance")? as u32, p.get("offset")? as u32)))
        };
        let Some((inst, offset)) = decoded.filter(|(i, _)| *i < self.instances(kind)) else {
            return false;
        };
        let pb = self.pushbuffers.get(&(class, inst)).copied();
        let life = self
            .ports
            .allocate(kind.channel_number(inst), offset)
            .unwrap_or(0);
        self.channels.insert(
            (kind, inst),
            Channel {
                class,
                kind,
                instance: inst,
                client,
                handle,
                pb,
                offset,
                life,
            },
        );
        self.state(Statement::ChannelAllocated {
            kind,
            instance: inst,
            offset,
            client,
            pb,
            life,
        });
        true
    }

    /// A free of `(client, handle)`: drops the channel if it was one. Returns `true` when it was.
    pub fn free(&mut self, client: u32, handle: u32) -> bool {
        let key = self
            .channels
            .iter()
            .find(|(_, c)| c.client == client && c.handle == handle)
            .map(|(k, _)| *k);
        if let Some(k) = key {
            self.channels.remove(&k);
            self.ports.release(k.0.channel_number(k.1));
            self.state(Statement::ChannelFreed {
                kind: k.0,
                instance: k.1,
                preserve: self.rmfree_preserve,
            });
        }
        key.is_some()
    }

    /// ★ One guest `RmFree` is done (every object it freed was passed to [`Self::free`] or
    /// [`Self::free_client`]): `SET_RMFREE_FLAGS` applied to it and to nothing after it.
    pub fn end_free(&mut self) {
        self.rmfree_preserve = false;
    }

    /// A free of the CLIENT `client` (the guest's RM frees each object first, but a client free is
    /// the last word on everything it held): drops every channel it owned. Returns how many.
    pub fn free_client(&mut self, client: u32) -> usize {
        let keys: Vec<_> = self
            .channels
            .iter()
            .filter(|(_, c)| c.client == client)
            .map(|(k, _)| *k)
            .collect();
        for k in &keys {
            self.channels.remove(k);
            self.ports.release(k.0.channel_number(k.1));
            self.state(Statement::ChannelFreed {
                kind: k.0,
                instance: k.1,
                preserve: self.rmfree_preserve,
            });
        }
        keys.len()
    }

    /// Drain the statements for the plane.
    pub fn take_statements(&mut self) -> Vec<Statement> {
        std::mem::take(&mut self.statements)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> DisplayModel {
        DisplayModel::new(
            &kf_chip::display::AMPERE,
            vec![Monitor::default_1080p()],
            crate::layout::for_version("580.159.04").expect("layouts"),
        )
    }
    fn cmd(m: &DisplayModel, n: &str) -> u32 {
        m.layouts().k32(n).expect(n)
    }
    fn size(m: &DisplayModel, s: &str) -> usize {
        m.layouts().size(s).expect(s)
    }
    fn get(m: &DisplayModel, s: &'static str, buf: &[u8], f: &str) -> u64 {
        Params::new(m.layouts(), s, buf)
            .expect("view")
            .get(f)
            .expect(f)
    }

    /// ★ The NVKMS bring-up conversation, in its order (`nvkms-rm.c:1665-1882`): caps, heads, the
    /// window assignment, the supported displays, the output and its connector, the EDID.
    #[test]
    fn the_nvkms_bringup_conversation_describes_one_dvi_monitor() {
        let mut m = model();
        let s = "NV0073_CTRL_SYSTEM_GET_NUM_HEADS_PARAMS";
        let r = m
            .control(
                cmd(&m, "NV0073_CTRL_CMD_SYSTEM_GET_NUM_HEADS"),
                &vec![0; size(&m, s)],
            )
            .unwrap()
            .unwrap();
        assert_eq!(get(&m, s, &r, "numHeads"), 4);
        let s = "NV0073_CTRL_SPECIFIC_GET_VALID_HEAD_WINDOW_ASSIGNMENT_PARAMS";
        let r = m
            .control(
                cmd(
                    &m,
                    "NV0073_CTRL_CMD_SPECIFIC_GET_VALID_HEAD_WINDOW_ASSIGNMENT",
                ),
                &vec![0; size(&m, s)],
            )
            .unwrap()
            .unwrap();
        let wm = Params::new(m.layouts(), s, &r)
            .unwrap()
            .bytes("windowHeadMask")
            .unwrap()
            .to_vec();
        assert_eq!(
            &wm[..9],
            &[1, 1, 2, 2, 4, 4, 8, 8, 0],
            "windows 2h, 2h+1 -> head h; window 8 absent"
        );
        let s = "NV0073_CTRL_SYSTEM_GET_SUPPORTED_PARAMS";
        let r = m
            .control(
                cmd(&m, "NV0073_CTRL_CMD_SYSTEM_GET_SUPPORTED"),
                &vec![0; size(&m, s)],
            )
            .unwrap()
            .unwrap();
        let id = get(&m, s, &r, "displayMask") as u32;
        assert_eq!(id, 0x100);
        let s = "NV0073_CTRL_SPECIFIC_OR_GET_INFO_PARAMS";
        let mut q = vec![0; size(&m, s)];
        let mut qp = Params::new(m.layouts(), s, &q).unwrap();
        qp.set("displayId", u64::from(id));
        q = qp.buf;
        let r = m
            .control(cmd(&m, "NV0073_CTRL_CMD_SPECIFIC_OR_GET_INFO"), &q)
            .unwrap()
            .unwrap();
        assert_eq!(get(&m, s, &r, "type"), 2, "SOR");
        assert_eq!(get(&m, s, &r, "protocol"), 1, "SINGLE_TMDS_A");
        let s = "NV0073_CTRL_SPECIFIC_GET_EDID_V2_PARAMS";
        let mut q = Params::new(m.layouts(), s, &vec![0; size(&m, s)]).unwrap();
        q.set("displayId", u64::from(id));
        let r = m
            .control(cmd(&m, "NV0073_CTRL_CMD_SPECIFIC_GET_EDID_V2"), &q.buf)
            .unwrap()
            .unwrap();
        let rp = Params::new(m.layouts(), s, &r).unwrap();
        assert_eq!(rp.get("bufferSize"), Some(128));
        assert_eq!(
            &rp.bytes("edidBuffer").unwrap()[..8],
            &[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]
        );
        // a display id we do not have is refused, never answered for connector 0
        let mut q = Params::new(m.layouts(), s, &vec![0; size(&m, s)]).unwrap();
        q.set("displayId", 0x200);
        assert_eq!(
            m.control(cmd(&m, "NV0073_CTRL_CMD_SPECIFIC_GET_EDID_V2"), &q.buf),
            Some(Err(NV_ERR_INVALID_ARGUMENT))
        );
    }

    /// ★ `SYSTEM_GET_ACTIVE` reports what the engine's ARMED state lights (the worker publishes it
    /// into the shared ports): nothing at boot, the connector on the head's SOR while it runs, and
    /// nothing again once the head goes idle. A head the engine does not have answers 0.
    #[test]
    fn get_active_reports_the_display_the_armed_state_lights() {
        let mut m = model();
        let s = "NV0073_CTRL_SYSTEM_GET_ACTIVE_PARAMS";
        let ask = |m: &mut DisplayModel, head: u64| {
            let mut q = Params::new(m.layouts(), s, &vec![0; size(m, s)]).unwrap();
            q.set("head", head);
            let r = m
                .control(cmd(m, "NV0073_CTRL_CMD_SYSTEM_GET_ACTIVE"), &q.buf)
                .unwrap()
                .unwrap();
            get(m, s, &r, "displayId")
        };
        assert_eq!(ask(&mut m, 3), 0, "nothing is lit at boot");
        m.ports.set_lit_sor(3, Some(0));
        assert_eq!(ask(&mut m, 3), 0x100, "head 3 drives SOR 0 -> connector 0");
        assert_eq!(ask(&mut m, 0), 0, "head 0 lights nothing");
        assert_eq!(ask(&mut m, 99), 0, "a head we do not have");
        m.ports.set_lit_sor(3, Some(2));
        assert_eq!(ask(&mut m, 3), 0, "SOR 2 has no connector");
        m.ports.set_lit_sor(3, None);
        assert_eq!(ask(&mut m, 3), 0, "idle again");
        // the display-SW object's constructor query is refused by name (its software methods would
        // trap on the host GPU): claimed, so it never reaches the ledger as unserviced
        let c = cmd(
            &m,
            "NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_ACTIVE_DISPLAY_DEVICES",
        );
        assert!(m.claims(c));
        let s = "NV2080_CTRL_INTERNAL_DISPLAY_GET_ACTIVE_DISPLAY_DEVICES_PARAMS";
        assert_eq!(
            m.control(c, &vec![0; size(&m, s)]),
            Some(Err(NV_ERR_NOT_SUPPORTED))
        );
    }

    /// A custom EDID shadows the monitor's until cleared (NVKMS clears on every read,
    /// `nvkms-dpy.c:2015-2045`); sizes that do not match the derived struct are refused.
    #[test]
    fn set_edid_shadows_and_clears_and_bad_sizes_are_refused() {
        let mut m = model();
        let s = "NV0073_CTRL_SPECIFIC_SET_EDID_V2_PARAMS";
        let mut q = Params::new(m.layouts(), s, &vec![0; size(&m, s)]).unwrap();
        q.set("displayId", 0x100);
        q.set("bufferSize", 3);
        q.set_bytes("edidBuffer", &[1, 2, 3]);
        assert!(matches!(
            m.control(cmd(&m, "NV0073_CTRL_CMD_SPECIFIC_SET_EDID_V2"), &q.buf),
            Some(Ok(_))
        ));
        assert_eq!(
            m.connectors[0].custom_edid.as_deref(),
            Some(&[1u8, 2, 3][..])
        );
        q.set("bufferSize", 0);
        assert!(matches!(
            m.control(cmd(&m, "NV0073_CTRL_CMD_SPECIFIC_SET_EDID_V2"), &q.buf),
            Some(Ok(_))
        ));
        assert_eq!(m.connectors[0].custom_edid, None);
        assert_eq!(
            m.control(cmd(&m, "NV0073_CTRL_CMD_SYSTEM_GET_NUM_HEADS"), &[0; 4]),
            Some(Err(NV_ERR_INVALID_ARGUMENT))
        );
        assert_eq!(m.control(0x2080_0101, &[0; 4]), None, "not ours");
    }

    /// Channels: the pushbuffer is stated first, then the alloc; the channel's idle state is its own
    /// GET == PUT; a free removes it; IS_MODE_POSSIBLE says yes.
    #[test]
    fn channel_registry_and_idle_state() {
        let mut m = model();
        let s = "NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS";
        let mut q = Params::new(m.layouts(), s, &vec![0; size(&m, s)]).unwrap();
        q.set("hclass", 0xC67D);
        q.set("channelInstance", 0);
        q.set("addressSpace", 1);
        q.set("physicalAddr", 0x1234_5000);
        q.set("limit", 0xfff);
        q.set("valid", 1);
        assert!(matches!(m.control(CHANNEL_PUSHBUFFER, &q.buf), Some(Ok(_))));
        let s = "NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS";
        assert!(m.alloc(0xc1d0_0001, 0xc67d_0000, 0xC67D, &vec![0; size(&m, s)]));
        let ch = &m.channels[&(ChannelKind::Core, 0)];
        assert_eq!(ch.pb.map(|p| p.phys), Some(0x1234_5000));
        let life = ch.life;
        let s = "NVC370_CTRL_CMD_GET_CHANNEL_INFO_PARAMS";
        let mut q = Params::new(m.layouts(), s, &vec![0; size(&m, s)]).unwrap();
        q.set("channelClass", 0xC67D);
        let r = m
            .control(cmd(&m, "NVC370_CTRL_CMD_GET_CHANNEL_INFO"), &q.buf)
            .unwrap()
            .unwrap();
        assert_eq!(get(&m, s, &r, "channelState"), 1, "IDLE");
        m.ports.post_put(0, 0x40);
        let r = m
            .control(cmd(&m, "NVC370_CTRL_CMD_GET_CHANNEL_INFO"), &q.buf)
            .unwrap()
            .unwrap();
        assert_eq!(
            get(&m, s, &r, "channelState"),
            0x40,
            "BUSY: the guest posted a PUT the engine has not consumed"
        );
        assert!(m.ports.publish_get(0, life, 0x40));
        let r = m
            .control(cmd(&m, "NVC370_CTRL_CMD_GET_CHANNEL_INFO"), &q.buf)
            .unwrap()
            .unwrap();
        assert_eq!(
            get(&m, s, &r, "channelState"),
            1,
            "IDLE once the engine published GET"
        );
        m.free(0xc1d0_0001, 0xc67d_0000);
        let r = m
            .control(cmd(&m, "NVC370_CTRL_CMD_GET_CHANNEL_INFO"), &q.buf)
            .unwrap()
            .unwrap();
        assert_eq!(get(&m, s, &r, "channelState"), 0x80, "DEALLOC");
        let s = "NVC372_CTRL_IS_MODE_POSSIBLE_PARAMS";
        let r = m
            .control(
                cmd(&m, "NVC372_CTRL_CMD_IS_MODE_POSSIBLE"),
                &vec![0; size(&m, s)],
            )
            .unwrap()
            .unwrap();
        assert_eq!(get(&m, s, &r, "bIsPossible"), 1);
        assert!(matches!(
            m.take_statements().as_slice(),
            [
                Statement::ChannelAllocated {
                    kind: ChannelKind::Core,
                    ..
                },
                Statement::ChannelFreed { .. }
            ]
        ));
    }

    /// ★ 2026-10-03 (B5): `SET_RMFREE_FLAGS` PRESERVE_HW marks the channels of the NEXT free — and
    /// only those (`ctrl5070chnc.h:899-918`); a free without it is not preserving.
    #[test]
    fn preserve_hw_marks_the_next_free_only() {
        let mut m = model();
        let s = "NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS";
        let core = |m: &mut DisplayModel| {
            assert!(m.alloc(0xc1d0_0001, 0xc67d_0000, 0xC67D, &vec![0; size(m, s)]));
        };
        let flags = |m: &mut DisplayModel, v: u64| {
            let f = "NV5070_CTRL_SET_RMFREE_FLAGS_PARAMS";
            let mut q = Params::new(m.layouts(), f, &vec![0; size(m, f)]).unwrap();
            q.set("flags", v);
            let c = cmd(m, "NV5070_CTRL_CMD_SET_RMFREE_FLAGS");
            assert!(matches!(m.control(c, &q.buf), Some(Ok(_))));
        };
        let freed = |m: &mut DisplayModel| {
            m.take_statements()
                .into_iter()
                .filter_map(|st| match st {
                    Statement::ChannelFreed { preserve, .. } => Some(preserve),
                    _ => None,
                })
                .collect::<Vec<bool>>()
        };
        core(&mut m);
        flags(&mut m, 1);
        assert!(m.free(0xc1d0_0001, 0xc67d_0000));
        m.end_free();
        assert_eq!(freed(&mut m), vec![true]);
        core(&mut m);
        assert!(m.free(0xc1d0_0001, 0xc67d_0000));
        m.end_free();
        assert_eq!(freed(&mut m), vec![false], "the flag was for one free");
        core(&mut m);
        flags(&mut m, 1);
        m.end_free(); // an unrelated free in between
        assert!(m.free(0xc1d0_0001, 0xc67d_0000));
        assert_eq!(freed(&mut m), vec![false]);
        core(&mut m);
        flags(&mut m, 0);
        assert!(m.free(0xc1d0_0001, 0xc67d_0000));
        assert_eq!(freed(&mut m), vec![false], "flags 0 (NONE) clears it");
    }

    /// ★ The claim set is enumerable and exact: six subdevice-internal controls plus the thirty
    /// named ones, every name resolved through the derived layouts, no id twice, and nothing in the
    /// display interfaces' command pages claimed that the set does not list.
    #[test]
    fn the_claim_set_is_enumerable_and_exact() {
        let m = model();
        let set = m.claimed();
        assert_eq!(set.len(), INTERNAL_CONTROLS.len() + NAMED_CONTROLS.len());
        assert_eq!(set.len(), 40);
        let distinct: std::collections::BTreeSet<u32> = set.iter().copied().collect();
        assert_eq!(distinct.len(), set.len(), "no id twice");
        assert!(set.iter().all(|c| m.claims(*c)));
        let scanned: std::collections::BTreeSet<u32> =
            [0x0073_0000u32, 0x5070_0000, 0xc370_0000, 0xc372_0000]
                .iter()
                .flat_map(|b| *b..*b + 0x2000)
                .chain(0x2080_0a00..0x2080_0b00)
                .filter(|c| m.claims(*c))
                .collect();
        assert_eq!(scanned, distinct);
    }

    /// ★ Hostile guest: every guest-stated key of the registry is bounded. A pushbuffer for a class
    /// that is not one of this family's channels, or for an instance the display does not have, is
    /// refused; an alloc whose params are not the derived struct, or whose instance does not exist,
    /// is not recorded (never read as instance 0); the statement queue stops at its bound and counts
    /// the rest; a client free drops every channel it owned.
    #[test]
    fn guest_stated_keys_are_bounded_and_the_statement_queue_is_capped() {
        let mut m = model();
        assert_eq!(
            (
                m.instances(ChannelKind::Core),
                m.instances(ChannelKind::Window)
            ),
            (1, 8)
        );
        assert_eq!(
            (
                m.instances(ChannelKind::WindowImm),
                m.instances(ChannelKind::Cursor)
            ),
            (8, 4)
        );
        let s = "NV2080_CTRL_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER_PARAMS";
        let pb = |m: &mut DisplayModel, class: u32, inst: u32| {
            let mut q = Params::new(m.layouts(), s, &vec![0; size(m, s)]).unwrap();
            q.set("hclass", u64::from(class));
            q.set("channelInstance", u64::from(inst));
            q.set("valid", 1);
            m.control(CHANNEL_PUSHBUFFER, &q.buf)
        };
        assert_eq!(
            pb(&mut m, 0xC67E, 8),
            Some(Err(NV_ERR_INVALID_ARGUMENT)),
            "window 8 does not exist"
        );
        assert_eq!(
            pb(&mut m, 0xC57E, 0),
            Some(Err(NV_ERR_INVALID_ARGUMENT)),
            "a Turing class on a GA10x display"
        );
        assert_eq!(
            pb(&mut m, 0xC67D, 1),
            Some(Err(NV_ERR_INVALID_ARGUMENT)),
            "there is one core channel"
        );
        assert!(matches!(pb(&mut m, 0xC67E, 7), Some(Ok(_))));
        assert_eq!(m.pushbuffers.len(), 1);
        let dma = size(&m, "NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS");
        let pio = size(&m, "NV50VAIO_CHANNELPIO_ALLOCATION_PARAMETERS");
        let with_inst = |n: usize, inst: u32| {
            let mut v = vec![0u8; n];
            v[0..4].copy_from_slice(&inst.to_le_bytes());
            v
        };
        let c = 0xc1d0_0001;
        assert!(
            !m.alloc(c, 0x10, 0xC67E, &with_inst(dma - 4, 0)),
            "short params are not a window at instance 0"
        );
        assert!(
            !m.alloc(c, 0x11, 0xC67E, &with_inst(dma, 0xffff_ffff)),
            "an instance the display does not have"
        );
        assert!(
            !m.alloc(c, 0x12, 0xC67A, &with_inst(pio, 4)),
            "cursor 4 on a four-head display"
        );
        assert!(
            !m.alloc(c, 0x13, 0xC670, &with_inst(dma, 0)),
            "the display object is not a channel"
        );
        assert!(m.channels.is_empty());
        assert!(m.alloc(c, 0x20, 0xC67E, &with_inst(dma, 7)));
        assert_eq!(
            m.channels[&(ChannelKind::Window, 7)].pb.map(|p| p.phys),
            Some(0),
            "its pushbuffer was stated first"
        );
        assert!(m.alloc(c, 0x21, 0xC67A, &with_inst(pio, 3)));
        assert!(m.alloc(0xc1d0_0002, 0x22, 0xC67D, &with_inst(dma, 0)));
        assert_eq!(m.free_client(c), 2);
        assert_eq!(
            m.channels.keys().copied().collect::<Vec<_>>(),
            vec![(ChannelKind::Core, 0)]
        );
        assert!(!m.free(c, 0x20), "already gone");
        m.take_statements();
        for i in 0..MAX_STATEMENTS + 5 {
            m.alloc(c, 0x100 + i as u32, 0xC67B, &with_inst(dma, (i % 8) as u32));
        }
        assert_eq!(m.statements.len(), MAX_STATEMENTS);
        assert_eq!(m.statements_dropped, 5);
        assert_eq!(
            m.channels.len(),
            1 + 8,
            "the registry holds one entry per channel the display has"
        );
    }
}
