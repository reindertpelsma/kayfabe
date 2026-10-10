//! ★★★ The display ENGINE — the emulated NVDisplay front end that executes the guest's display
//! channels (`docs/design/V3_DISPLAY.md` §4.3–§4.5). Pure and GPU-free: it takes a channel's
//! pushbuffer bytes and PUT, and returns what the plane must do, in order.
//!
//! ## What a display channel is, to this engine
//!
//! Every channel (core, window, window-immediate: DMA; cursor: PIO) accumulates method writes into
//! its **assembly** state. An `UPDATE` promotes assembly to **armed** — the state the head scans out
//! with — once every channel the update is **interlocked** with has issued its own `UPDATE`
//! (`SET_INTERLOCK_FLAGS` / `SET_WINDOW_INTERLOCK_FLAGS`, the window's `UPDATE.INTERLOCK_WITH_WIN_IMM`,
//! the immediate channel's `UPDATE.INTERLOCK_WITH_WINDOW`; NVKMS builds these groups in
//! `nvEvoUpdateC3`, `ogkm-580: nvkms-evo3.c:3121-3200`). A non-tearing window flip on an active head
//! then waits for that head's next **vblank**, and a flip with an **acquire semaphore** waits until the
//! semaphore holds its value. Until its update completes, a channel **stops** at the `UPDATE`: its
//! GET stands before it and the channel is busy — the guest's own idle poll (`GET_CHANNEL_INFO`)
//! and its free-space arithmetic (`nvEvoMakeRoom`) see exactly what the hardware would show.
//!
//! When an update completes, the engine ARMS the state and only THEN states the completions it asked
//! for — the notifier (`SET_CONTEXT_DMA_NOTIFIER` + `SET_NOTIFIER_CONTROL`), the release semaphore,
//! and AWAKEN when the mode asks for it — as [`Effect`]s the plane performs in order. ⊘ Owner rule
//! A.3: nothing here states a completion for work the engine has not applied. The display is an
//! EMULATED device with no GPU work behind its channels (`THE_ARCHITECTURE_v3.md` §5, §37); its
//! completions are the end of its own processing.
//!
//! ## Hostile guest (only the guest KERNEL can allocate these channels, `resource_list.h:1270-1318`)
//!
//! Every method address is bounded by the channel's method space (the ASSEMBLY half of its user
//! area, derived from the register vocabulary: 32 KiB core, 2 KiB window/immediate/cursor); an
//! address outside it, a malformed pushbuffer, or a queue past [`MAX_QUEUE`] stops the channel with
//! a named [`Effect::Exception`] — the guest's display stalls, loudly, and nothing reaches the host.
//! Every interlock set is a bit mask over the 81 channel numbers; a group never waits on a channel
//! that is not allocated. Nothing here sizes an allocation from a guest value.

use crate::class::{ClassTable, get as fld};
use crate::model::{ChannelKind, Classes};
use crate::pushbuf::{self, DecodeError, Located};
use crate::regs::Regs;
use std::collections::VecDeque;

/// Channel numbers (`NV_PDISP_CHN_NUM_*`).
pub const CHANNELS: usize = crate::ports::NUM_CHANNELS;
/// ★ Hostile guest: the most decoded-but-unapplied writes a channel may hold (a 4 KiB ring holds at
/// most 1023; the rest is a PUT that ignored GET).
pub const MAX_QUEUE: usize = 4096;
/// Host-enabled diagnostic ceiling per engine lifetime, including repeated writes.
pub const MAX_METHOD_TRACE: u32 = 65_536;

/// Where a DMA channel's pushbuffer lives (`INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PbLoc {
    /// Guest system memory (else the framebuffer).
    pub sysmem: bool,
    /// Guest physical address, or framebuffer offset.
    pub addr: u64,
    /// Bytes (≤ 4 KiB, [`pushbuf::MAX_PUSHBUFFER`]).
    pub bytes: u32,
}

/// Something the plane must do, in the order returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Publish these core ARMED words — `(method offset, value)` — in the user area's ARMED half.
    CoreArmed(Vec<(u32, u32)>),
    /// Write a 16-byte NVDisplay notifier `FINISHED` at `offset` of context DMA `handle` (bound to
    /// channel number `chn` by `client`), then raise AWAKEN for `chn` if `awaken`.
    Notify {
        /// Channel number.
        chn: u32,
        /// The channel's client (the hash key).
        client: u32,
        /// Context DMA handle.
        handle: u32,
        /// Byte offset in the context DMA.
        offset: u64,
        /// `MODE_WRITE_AWAKEN`.
        awaken: bool,
        /// The status is FINISHED (the core's completion notifier, or ⚠ a window entry flipped away under
        /// `notifier_finish_at_flip_away`); `false` for a window flip's own notifier at its latch.
        finished: bool,
    },
    /// Release a semaphore: write `value` (32 or 64 bits) at `offset` of context DMA `handle`, then
    /// raise the window's semaphore event if `awaken`.
    Release {
        /// Channel number.
        chn: u32,
        /// The channel's client.
        client: u32,
        /// Context DMA handle.
        handle: u32,
        /// Byte offset.
        offset: u64,
        /// Value.
        value: u64,
        /// 64-bit payload.
        wide: bool,
        /// `REL_MODE_WRITE_AWAKEN`.
        awaken: bool,
    },
    /// The armed head configuration changed: the plane re-reads [`Engine::heads`] (vblank timers,
    /// `CORE_HEAD_STATE`).
    Heads,
    /// Window `window`'s armed (latched) state changed — the scanout follows it.
    Latched {
        /// Window index.
        window: u32,
    },
    /// A trace line (only when [`Engine::trace`] is on).
    Trace(String),
    /// Channel `chn` stopped at byte `at`: the named reason. Nothing after it is executed.
    Exception {
        /// Channel number.
        chn: u32,
        /// Byte offset GET stands at.
        at: u32,
        /// Why.
        what: String,
    },
}

/// An acquire the plane must evaluate against guest memory before a flip may latch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Acquire {
    /// Channel number (a window).
    pub chn: u32,
    /// The channel's client.
    pub client: u32,
    /// Context DMA handle.
    pub handle: u32,
    /// Byte offset.
    pub offset: u64,
    /// The value awaited.
    pub value: u64,
    /// 64-bit payload.
    pub wide: bool,
    /// `EQ` (0), `CGEQ` (1) or `STRICT_GEQ` (2).
    pub mode: u32,
}

impl Acquire {
    /// Does `current` (read from the semaphore) satisfy the acquire?
    #[must_use]
    pub fn satisfied_by(&self, current: u64) -> bool {
        let (cur, want) = if self.wide {
            (current, self.value)
        } else {
            (current & 0xFFFF_FFFF, self.value & 0xFFFF_FFFF)
        };
        match self.mode {
            0 => cur == want,
            // CGEQ: circular (wrapping) greater-or-equal
            1 => {
                if self.wide {
                    cur.wrapping_sub(want) as i64 >= 0
                } else {
                    (cur as u32).wrapping_sub(want as u32) as i32 >= 0
                }
            }
            2 => cur >= want,
            _ => false,
        }
    }
}

/// One head's armed raster, as the vblank timer needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadMode {
    /// Head index.
    pub head: u32,
    /// Refresh period in nanoseconds (0 = inactive).
    pub period_ns: u64,
    /// Raster width and height (active + blanking).
    pub raster: (u32, u32),
}

/// ★ The per-family method vocabulary the engine interprets — every offset and field RESOLVED from
/// the derived class table at construction, so a missing name refuses the engine up front instead
/// of reading as method 0 at run time.
#[derive(Debug, Clone)]
pub struct Vocab {
    core_space: u32,
    other_space: u32,
    // core
    c_update: u32,
    c_ctxdma_notifier: u32,
    c_notifier_control: u32,
    c_interlock: u32,
    c_window_interlock: u32,
    c_window_set_control: (u32, u32),
    c_window_owner: (u8, u8),
    c_owner_none: u32,
    c_pclk: (u32, u32),
    c_pclk_hz: (u8, u8),
    c_pclk_adj: (u8, u8),
    c_raster_size: (u32, u32),
    c_raster_w: (u8, u8),
    c_raster_h: (u8, u8),
    c_sor_control: (u32, u32),
    c_sor_owner: (u8, u8),
    c_ilk_cursor0: (u8, u8),
    // notifier control fields (core and window share the layout; the window has no NOTIFY field)
    n_mode: (u8, u8),
    n_mode_awaken: u32,
    n_offset: (u8, u8),
    n_notify: (u8, u8),
    // window
    w_color_inline: Vec<InlineMethod>,
    w_update: u32,
    w_update_ilk_winim: (u8, u8),
    w_ctxdma_notifier: u32,
    w_notifier_control: u32,
    w_interlock: u32,
    w_ilk_core: (u8, u8),
    w_ilk_cursor0: (u8, u8),
    w_window_interlock: u32,
    w_present: u32,
    w_present_begin: (u8, u8),
    w_present_non_tearing: u32,
    w_ctxdma_sem: u32,
    w_sem_control: u32,
    w_sem_offset: (u8, u8),
    w_sem_payload: (u8, u8),
    w_sem_rel_mode: (u8, u8),
    w_sem_release: u32,
    w_sem_release_hi: Option<u32>,
    w_ctxdma_acq: u32,
    w_acq_control: u32,
    w_acq_offset: (u8, u8),
    w_acq_payload: (u8, u8),
    w_acq_mode: (u8, u8),
    w_acq_value: u32,
    w_acq_value_hi: Option<u32>,
    /// The window's first ISO surface word — a context DMA handle (C5x–C9x) or the surface address's
    /// low word with its ENABLE bit (CAx): non-zero iff the window scans a surface.
    w_iso0: u32,
    // window-immediate and cursor
    i_update: u32,
    i_ilk_window: (u8, u8),
    k_update: u32,
    k_interlock: u32,
    k_ilk_core: Option<(u8, u8)>,
    k_window_interlock: u32,
}

type InlineMethod = (u32, usize, bool, (u8, u8), (u8, u8));

/// Why a vocabulary could not be resolved (the missing name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved(pub String);

impl Vocab {
    /// Resolve the vocabulary of a family's classes from the derived tables.
    ///
    /// # Errors
    /// [`Unresolved`] naming the first missing method, field or register.
    pub fn resolve(t: &ClassTable, c: &Classes, r: &Regs) -> Result<Vocab, Unresolved> {
        let miss = |n: &str| Unresolved(n.to_string());
        let v = |cl: u32, n: &str| t.v(cl, n).ok_or_else(|| miss(&format!("NV{cl:04X}_{n}")));
        let f = |cl: u32, n: &str| t.f(cl, n).ok_or_else(|| miss(&format!("NV{cl:04X}_{n}")));
        let fa0 = |cl: u32, n: &str| {
            t.fa(cl, n, 0)
                .ok_or_else(|| miss(&format!("NV{cl:04X}_{n}(0)")))
        };
        let arr = |cl: u32, n: &str| -> Result<(u32, u32), Unresolved> {
            let b = t
                .a(cl, n, 0)
                .ok_or_else(|| miss(&format!("NV{cl:04X}_{n}(i)")))?;
            let s = t
                .a(cl, n, 1)
                .ok_or_else(|| miss(&format!("NV{cl:04X}_{n}(i)")))?
                - b;
            Ok((b, s))
        };
        let assy = r
            .v("NV_UDISP_FE_CHN_ASSY_BASEADR_CORE")
            .ok_or_else(|| miss("NV_UDISP_FE_CHN_ASSY_BASEADR_CORE"))?;
        let armed = r
            .v("NV_UDISP_FE_CHN_ARMED_BASEADR_CORE")
            .ok_or_else(|| miss("NV_UDISP_FE_CHN_ARMED_BASEADR_CORE"))?;
        let win_stride = r
            .a("NV_UDISP_FE_CHN_ASSY_BASEADR_WIN", 1)
            .zip(r.a("NV_UDISP_FE_CHN_ASSY_BASEADR_WIN", 0))
            .map(|(a, b)| a - b)
            .ok_or_else(|| miss("NV_UDISP_FE_CHN_ASSY_BASEADR_WIN(i)"))?;
        let (core, win, imm, cur) = (c.core, c.window, c.window_imm, c.cursor);
        Ok(Vocab {
            // the core's user area is ASSEMBLY then ARMED; every other channel's page is split in two
            // (`kdispGetDisplayChannelUserBaseAndSize_v03_00`: "all other channels are 4KB (2K for
            // Armed and 2k for Assembly)")
            core_space: u32::try_from(armed - assy).map_err(|_| miss("core method space"))?,
            other_space: u32::try_from(win_stride / 2).map_err(|_| miss("window method space"))?,
            c_update: v(core, "UPDATE")?,
            c_ctxdma_notifier: v(core, "SET_CONTEXT_DMA_NOTIFIER")?,
            c_notifier_control: v(core, "SET_NOTIFIER_CONTROL")?,
            c_interlock: v(core, "SET_INTERLOCK_FLAGS")?,
            c_window_interlock: v(core, "SET_WINDOW_INTERLOCK_FLAGS")?,
            c_window_set_control: arr(core, "WINDOW_SET_CONTROL")?,
            c_window_owner: f(core, "WINDOW_SET_CONTROL_OWNER")?,
            c_owner_none: v(core, "WINDOW_SET_CONTROL_OWNER_NONE")?,
            c_pclk: arr(core, "HEAD_SET_PIXEL_CLOCK_FREQUENCY")?,
            c_pclk_hz: f(core, "HEAD_SET_PIXEL_CLOCK_FREQUENCY_HERTZ")?,
            c_pclk_adj: f(core, "HEAD_SET_PIXEL_CLOCK_FREQUENCY_ADJ1000DIV1001")?,
            c_raster_size: arr(core, "HEAD_SET_RASTER_SIZE")?,
            c_raster_w: f(core, "HEAD_SET_RASTER_SIZE_WIDTH")?,
            c_raster_h: f(core, "HEAD_SET_RASTER_SIZE_HEIGHT")?,
            c_sor_control: arr(core, "SOR_SET_CONTROL")?,
            c_sor_owner: f(core, "SOR_SET_CONTROL_OWNER_MASK")?,
            c_ilk_cursor0: fa0(core, "SET_INTERLOCK_FLAGS_INTERLOCK_WITH_CURSOR")?,
            n_mode: f(core, "SET_NOTIFIER_CONTROL_MODE")?,
            n_mode_awaken: v(core, "SET_NOTIFIER_CONTROL_MODE_WRITE_AWAKEN")?,
            n_offset: f(core, "SET_NOTIFIER_CONTROL_OFFSET")?,
            n_notify: f(core, "SET_NOTIFIER_CONTROL_NOTIFY")?,
            w_color_inline: (0..2)
                .flat_map(|stage| [true, false].map(move |entry| (stage, entry)))
                .map(|(stage, entry)| {
                    let name = format!(
                        "SET_CSC{stage}LUT_{}",
                        if entry { "ENTRY" } else { "SEGMENT_SIZE" }
                    );
                    Ok((
                        v(c.window, &name)?,
                        stage,
                        entry,
                        f(c.window, &format!("{name}_IDX"))?,
                        f(c.window, &format!("{name}_VALUE"))?,
                    ))
                })
                .collect::<Result<_, Unresolved>>()?,
            w_update: v(win, "UPDATE")?,
            w_update_ilk_winim: f(win, "UPDATE_INTERLOCK_WITH_WIN_IMM")?,
            w_ctxdma_notifier: v(win, "SET_CONTEXT_DMA_NOTIFIER")?,
            w_notifier_control: v(win, "SET_NOTIFIER_CONTROL")?,
            w_interlock: v(win, "SET_INTERLOCK_FLAGS")?,
            w_ilk_core: f(win, "SET_INTERLOCK_FLAGS_INTERLOCK_WITH_CORE")?,
            w_ilk_cursor0: fa0(win, "SET_INTERLOCK_FLAGS_INTERLOCK_WITH_CURSOR")?,
            w_window_interlock: v(win, "SET_WINDOW_INTERLOCK_FLAGS")?,
            w_present: v(win, "SET_PRESENT_CONTROL")?,
            w_present_begin: f(win, "SET_PRESENT_CONTROL_BEGIN_MODE")?,
            w_present_non_tearing: v(win, "SET_PRESENT_CONTROL_BEGIN_MODE_NON_TEARING")?,
            w_ctxdma_sem: v(win, "SET_CONTEXT_DMA_SEMAPHORE")?,
            w_sem_control: v(win, "SET_SEMAPHORE_CONTROL")?,
            w_sem_offset: f(win, "SET_SEMAPHORE_CONTROL_OFFSET")?,
            w_sem_payload: f(win, "SET_SEMAPHORE_CONTROL_PAYLOAD_SIZE")?,
            w_sem_rel_mode: f(win, "SET_SEMAPHORE_CONTROL_REL_MODE")?,
            w_sem_release: v(win, "SET_SEMAPHORE_RELEASE")?,
            w_sem_release_hi: t.v(win, "SET_SEMAPHORE_RELEASE_HI"),
            w_ctxdma_acq: v(win, "SET_CONTEXT_DMA_ACQ_SEMAPHORE")?,
            w_acq_control: v(win, "SET_ACQ_SEMAPHORE_CONTROL")?,
            w_acq_offset: f(win, "SET_ACQ_SEMAPHORE_CONTROL_OFFSET")?,
            w_acq_payload: f(win, "SET_ACQ_SEMAPHORE_CONTROL_PAYLOAD_SIZE")?,
            w_acq_mode: f(win, "SET_ACQ_SEMAPHORE_CONTROL_ACQ_MODE")?,
            w_acq_value: v(win, "SET_ACQ_SEMAPHORE_VALUE")?,
            w_acq_value_hi: t.v(win, "SET_ACQ_SEMAPHORE_VALUE_HI"),
            w_iso0: t
                .a(win, "SET_CONTEXT_DMA_ISO", 0)
                .or_else(|| t.a(win, "SET_SURFACE_ADDRESS_LO_ISO", 0))
                .ok_or_else(|| {
                    miss(&format!(
                        "NV{win:04X}_SET_CONTEXT_DMA_ISO(0) / SET_SURFACE_ADDRESS_LO_ISO(0)"
                    ))
                })?,
            i_update: v(imm, "UPDATE")?,
            i_ilk_window: f(imm, "UPDATE_INTERLOCK_WITH_WINDOW")?,
            k_update: v(cur, "UPDATE")?,
            k_interlock: v(cur, "SET_INTERLOCK_FLAGS")?,
            k_ilk_core: t.f(cur, "SET_INTERLOCK_FLAGS_INTERLOCK_WITH_CORE"),
            k_window_interlock: v(cur, "SET_WINDOW_INTERLOCK_FLAGS")?,
        })
    }

    fn space(&self, kind: ChannelKind) -> u32 {
        if kind == ChannelKind::Core {
            self.core_space
        } else {
            self.other_space
        }
    }

    fn update_of(&self, kind: ChannelKind) -> u32 {
        match kind {
            ChannelKind::Core => self.c_update,
            ChannelKind::Window => self.w_update,
            ChannelKind::WindowImm => self.i_update,
            ChannelKind::Cursor => self.k_update,
        }
    }
}

/// A 128-bit set of channel numbers.
type ChanSet = u128;

fn bit(chn: u32) -> ChanSet {
    if (chn as usize) < CHANNELS {
        1u128 << chn
    } else {
        0
    }
}

/// Where a pending update stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Executing methods.
    Running,
    /// Stopped at an `UPDATE` (data `update`) waiting for its interlock group; `ilk` = the channels
    /// it waits for.
    Interlock { update: u32, ilk: ChanSet },
    /// The group `group` is ready; waiting for head `head`'s vblank (`None`: only an acquire).
    Latch {
        update: u32,
        head: Option<u32>,
        group: ChanSet,
    },
}

/// One live channel.
#[derive(Debug)]
struct Chan {
    kind: ChannelKind,
    instance: u32,
    client: u32,
    life: u32,
    pb: Option<PbLoc>,
    /// GET: the header of the first unconsumed write (or `decoded` when none is queued).
    get: u32,
    /// Where decoding stopped (the next undecoded byte).
    decoded: u32,
    queue: VecDeque<Located>,
    assy_inline: Option<Box<[crate::color::InlineLut; 2]>>,
    armed_inline: Option<Box<[crate::color::InlineLut; 2]>>,
    assy: Vec<u32>,
    armed: Vec<u32>,
    stage: Stage,
    halted: bool,
    /// ⚠ DIAGNOSTIC ledger (TDR hunt): when the pending UPDATE reached the engine, and when its acquire first failed.
    commit: Option<std::time::Instant>,
    commit_info: String,
    acq_block: Option<std::time::Instant>,
    acq_info: String,
}

impl Chan {
    fn new(
        kind: ChannelKind,
        instance: u32,
        client: u32,
        life: u32,
        pb: Option<PbLoc>,
        offset: u32,
        space: u32,
    ) -> Chan {
        let words = (space / 4) as usize;
        Chan {
            kind,
            instance,
            client,
            life,
            pb,
            get: offset,
            decoded: offset,
            queue: VecDeque::new(),
            assy_inline: (kind == ChannelKind::Window)
                .then(|| Box::new(std::array::from_fn(|_| crate::color::InlineLut::default()))),
            armed_inline: (kind == ChannelKind::Window)
                .then(|| Box::new(std::array::from_fn(|_| crate::color::InlineLut::default()))),
            assy: vec![0; words],
            armed: vec![0; words],
            stage: Stage::Running,
            halted: false,
            commit: None,
            commit_info: String::new(),
            acq_block: None,
            acq_info: String::new(),
        }
    }
    fn a(&self, m: u32) -> u32 {
        self.assy.get((m / 4) as usize).copied().unwrap_or(0)
    }
    fn armed(&self, m: u32) -> u32 {
        self.armed.get((m / 4) as usize).copied().unwrap_or(0)
    }
}

/// What one [`Engine::step`] produced.
#[derive(Debug, Default)]
pub struct Step {
    /// Effects, in order.
    pub effects: Vec<Effect>,
    /// `(channel number, generation, GET)` to publish after the effects.
    pub gets: Vec<(u32, u32, u32)>,
}

/// ★ `display-max-fps` (`crate::pace`): one head's presents, by path — cumulative.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaceCounts {
    /// Latches that completed a window of this (active) head — counted once per latch, on every
    /// path: at a tick, at once, and with the core.
    pub presents: u64,
    /// Of those, latches holding a TEARING (immediate) flip.
    pub tearing: u64,
    /// Tearing flips the gate parked for the head's next tick (D1).
    pub tear_held: u64,
    /// Latches of groups holding the CORE: they latch at once, so the tick does not bound them.
    pub core_imm: u64,
}

/// ⚠ DIAGNOSTIC (2026-10-10, TDR hunt; owner challenge "no unanswered flips"): per display channel, UPDATEs the
/// engine received (`committed`) against updates it completed (`completed`: armed, completions stated), the
/// slowest commit-to-complete time, the acquire waits, and a bounded list of the slow ones (> 50 ms) with what they
/// carried. Plain counters, display thread only: nothing on a vCPU or the drainer.
#[derive(Debug, Clone)]
pub struct FlipLedger {
    /// UPDATEs reached, per channel number.
    pub committed: [u64; CHANNELS],
    /// UPDATEs completed, per channel number.
    pub completed: [u64; CHANNELS],
    /// Completions whose acquire had failed at least once.
    pub acq_blocked: [u64; CHANNELS],
    /// Slowest commit-to-complete (ms), per channel number.
    pub max_ms: [u64; CHANNELS],
    /// Completions slower than 50 ms, per channel number.
    pub slow_n: [u64; CHANNELS],
    /// Lines for the slow completions not yet drained (bounded).
    pub slow: Vec<String>,
}

impl Default for FlipLedger {
    fn default() -> Self {
        FlipLedger {
            committed: [0; CHANNELS],
            completed: [0; CHANNELS],
            acq_blocked: [0; CHANNELS],
            max_ms: [0; CHANNELS],
            slow_n: [0; CHANNELS],
            slow: Vec::new(),
        }
    }
}

/// ★ The engine.
#[derive(Debug)]
pub struct Engine {
    vocab: Vocab,
    /// Immutable construction diagnostic; every method ingress refuses.
    constructor_probe: bool,
    heads: u32,
    windows: u32,
    chans: Vec<Option<Chan>>,
    /// Updates completed (boot log).
    pub updates: u64,
    /// Methods executed (boot log).
    pub methods: u64,
    /// Exceptions raised (boot log).
    pub exceptions: u64,
    /// Emit [`Effect::Trace`] lines (update arrivals, groups, latches).
    pub trace: bool,
    method_trace_remaining: u32,
    method_trace_configured: bool,
    /// ★ D1 (`OWNER_RULINGS.md` §M, 2026-10-04): a TEARING (immediate) flip on an active head that
    /// already presented since the head's last tick waits for the next one, so async flips count
    /// against the cap too. In kayfabe a flip copies a finished buffer, so it never tears; the gate
    /// is about rate only. On by default.
    pub tear_gate: bool,
    /// ⚠ EXPERIMENT (default `false`, 2026-10-08, H-corelatch; `KF3_DISPLAY_CORE_AT_VBLANK=1`): an
    /// update group that includes the CORE latches at the next vblank of an active head, as the
    /// hardware's does, instead of at once. `[measured, VFIO DVI reference boot3, RTX 4070,
    /// 2026-10-08]` Windows' modeset core PUTs come one per frame (12.520125, 12.535982, 12.552647 s:
    /// it waits a frame for each), while under kf3 the same PUTs complete within 3 ms (run 93).
    pub core_latch_at_vblank: bool,
    /// Per head: a window of it latched since its last tick (the gate's state).
    presented: [bool; 8],
    /// Per head: presents by path.
    pub pace: [PaceCounts; 8],
    /// ⚠ DIAGNOSTIC flip ledger (TDR hunt; always on, plain counters on the display thread).
    pub ledger: FlipLedger,
    /// ⚠ DIAGNOSTIC (default `false`, 2026-10-10 TDR hunt; `KF3_DIAG_RELEASE_AT_LATCH=1`): the pre-2026-10-10
    /// behaviour — a window's release written at its OWN entry's latch, not at flip-away — for the A/B runs.
    pub release_at_latch: bool,
    /// ★ 2026-10-10 (TDR hunt, default `true`): at a window latch the OUTGOING entry's notifier is written FINISHED
    /// (its flip-away), and the incoming entry's own notifier BEGUN (the display side writes the status words). Open
    /// NVKMS: "when EVO performs the flip, it changes the notifier to BEGUN" (`ogkm-595.84:
    /// nvidia-modeset/src/nvkms-headsurface.c:1925-1952`). [measured, run 286, read-watchpoints] after programming flip
    /// N the Windows driver polls flip N's AND flip N-1's notifier status words together and leaves the loop only once
    /// N-1 is FINISHED; [measured, runs 282-284 vs 287] with the hardware vblank order every first flip stuck without the
    /// flip-away FINISHED (5 TDR cycles in boot), none with it. ⚠ `false` (`KF3_DIAG_WINDOW_NOTIFIER_FINISHED_AT_LATCH=1`):
    /// the pre-2026-10-10 behaviour, every window notifier FINISHED at its own latch (diagnostic A/B only).
    pub notifier_finish_at_flip_away: bool,
}

impl Engine {
    /// An engine for `heads` heads and `windows` windows speaking `vocab`.
    #[must_use]
    pub fn new(vocab: Vocab, heads: u32, windows: u32) -> Engine {
        Engine {
            vocab,
            constructor_probe: false,
            heads: heads.min(8),
            windows: windows.min(32),
            chans: (0..CHANNELS).map(|_| None).collect(),
            updates: 0,
            methods: 0,
            exceptions: 0,
            trace: false,
            method_trace_remaining: 0,
            method_trace_configured: false,
            tear_gate: true,
            core_latch_at_vblank: false,
            presented: [false; 8],
            pace: [PaceCounts::default(); 8],
            ledger: FlipLedger::default(),
            release_at_latch: false,
            notifier_finish_at_flip_away: true,
        }
    }

    /// Enable a bounded method diagnostic before any guest work. It cannot be replenished
    /// after a method or update executes, or by freeing/reallocating a channel.
    pub fn trace_methods(&mut self, budget: u32) {
        if self.methods == 0 && self.updates == 0 && !self.method_trace_configured {
            self.method_trace_configured = true;
            self.method_trace_remaining = budget.min(MAX_METHOD_TRACE);
        }
    }

    /// Construction-only diagnostic. No display method can execute or complete.
    /// There is deliberately no switch for an already running engine.
    #[must_use]
    pub fn new_constructor_probe(vocab: Vocab, heads: u32, windows: u32) -> Engine {
        let mut engine = Self::new(vocab, heads, windows);
        engine.constructor_probe = true;
        engine
    }

    fn refuse_constructor_method(&mut self, chn: u32) -> Step {
        let mut st = Step::default();
        if let Some(c) = self.chans.get_mut(chn as usize).and_then(Option::as_mut) {
            if !c.halted {
                c.halted = true;
                self.exceptions += 1;
                st.effects.push(Effect::Exception {
                    chn,
                    at: c.get,
                    what: "constructor-only probe refuses all display methods".into(),
                });
            }
            st.gets.push((chn, c.life, c.get));
        }
        st
    }

    /// The channel number of `(kind, instance)` if the display has it.
    #[must_use]
    pub fn channel_number(&self, kind: ChannelKind, instance: u32) -> Option<u32> {
        let n = match kind {
            ChannelKind::Core => 1,
            ChannelKind::Window | ChannelKind::WindowImm => self.windows,
            ChannelKind::Cursor => self.heads,
        };
        (instance < n).then(|| kind.channel_number(instance))
    }

    /// ★ A channel came into being (the control link accepted its alloc): GET = PUT = `offset`, empty
    /// state. A previous life of the same number is discarded.
    pub fn alloc(
        &mut self,
        kind: ChannelKind,
        instance: u32,
        client: u32,
        life: u32,
        pb: Option<PbLoc>,
        offset: u32,
    ) -> Option<u32> {
        let chn = self.channel_number(kind, instance)?;
        let space = self.vocab.space(kind);
        self.chans[chn as usize] = Some(Chan::new(kind, instance, client, life, pb, offset, space));
        Some(chn)
    }

    /// A channel was freed.
    pub fn free(&mut self, kind: ChannelKind, instance: u32) {
        if let Some(chn) = self.channel_number(kind, instance) {
            self.chans[chn as usize] = None;
        }
    }

    /// The live channel numbers with a DMA pushbuffer whose decoded position is not `put`.
    #[must_use]
    pub fn pushbuffer(&self, chn: u32) -> Option<(PbLoc, u32, u32)> {
        let c = self.chans.get(chn as usize)?.as_ref()?;
        Some((c.pb?, c.decoded, c.life))
    }

    /// Is `chn` allocated (with generation `life`)?
    #[must_use]
    pub fn generation(&self, chn: u32) -> Option<u32> {
        self.chans.get(chn as usize)?.as_ref().map(|c| c.life)
    }

    /// The owning client of a live channel, for bounded colour DMA resolution.
    #[must_use]
    pub fn client(&self, chn: u32) -> Option<u32> {
        self.chans.get(chn as usize)?.as_ref().map(|c| c.client)
    }

    /// ⚠ DIAGNOSTIC: `ch<N>:committed/completed(slowest ms, slow n, acq-blocked n, pending age ms)` for every
    /// channel that committed anything.
    #[must_use]
    pub fn ledger_summary(&self) -> String {
        let l = &self.ledger;
        (0..CHANNELS)
            .filter(|&i| l.committed[i] > 0)
            .map(|i| {
                let age = self.chans[i]
                    .as_ref()
                    .and_then(|c| c.commit)
                    .map_or(0, |t| t.elapsed().as_millis());
                format!(
                    "ch{i}:{}/{}(max {}ms slow {} acqblk {} pend {}ms)",
                    l.committed[i], l.completed[i], l.max_ms[i], l.slow_n[i], l.acq_blocked[i], age
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A GPU scanout failure stops the display engine without publishing successful
    /// notifiers, semaphore releases, or GETs. Reset/free may discard the stopped channels.
    pub fn halt_scanout(&mut self) {
        for c in self.chans.iter_mut().flatten() {
            c.halted = true;
            c.queue.clear();
        }
    }

    /// ★ Feed channel `chn` the pushbuffer bytes `pb` (its whole ring) up to `put`, and run every
    /// channel whose update became ready. `acquired` answers an acquire against guest memory.
    pub fn step(
        &mut self,
        chn: u32,
        pb: &[u8],
        put: u32,
        acquired: &mut dyn FnMut(&Acquire) -> bool,
    ) -> Step {
        if self.constructor_probe {
            return self.refuse_constructor_method(chn);
        }
        let mut st = Step::default();
        self.decode(chn, pb, put, &mut st);
        self.run(&mut st, acquired);
        st
    }

    /// ★ A cursor PIO write (`off` inside the cursor channel's user area) — applied at once; an
    /// `Update` is an update like any other.
    pub fn cursor_write(
        &mut self,
        head: u32,
        off: u32,
        val: u32,
        acquired: &mut dyn FnMut(&Acquire) -> bool,
    ) -> Step {
        let mut st = Step::default();
        let Some(chn) = self.channel_number(ChannelKind::Cursor, head) else {
            return st;
        };
        if self.constructor_probe {
            return self.refuse_constructor_method(chn);
        }
        let space = self.vocab.other_space;
        if let Some(c) = self.chans[chn as usize].as_mut() {
            if c.halted {
                return st;
            }
            if !off.is_multiple_of(4) || off >= space {
                self.exceptions += 1;
                st.effects.push(Effect::Exception {
                    chn,
                    at: 0,
                    what: format!(
                        "cursor PIO write at {off:#x} is outside its {space:#x}-byte method space"
                    ),
                });
                return st;
            }
            c.queue.push_back(Located {
                write: pushbuf::MethodWrite {
                    method: off,
                    data: val,
                },
                header: 0,
                end: 0,
            });
        }
        self.run(&mut st, acquired);
        st
    }

    /// ★ Head `head`'s vblank: latch every update waiting for it (whose acquires hold). The head
    /// may present again: the tearing gate's state clears first.
    pub fn vblank(&mut self, head: u32, acquired: &mut dyn FnMut(&Acquire) -> bool) -> Step {
        let mut st = Step::default();
        if let Some(p) = self.presented.get_mut(head as usize) {
            *p = false;
        }
        for group in self.latching(|h| h == Some(head)) {
            self.latch_group(&group, &mut st, acquired);
        }
        self.run(&mut st, acquired);
        st
    }

    /// ★ Re-evaluate every update waiting only for an acquire (a periodic poll while one is pending).
    pub fn poll_acquires(&mut self, acquired: &mut dyn FnMut(&Acquire) -> bool) -> Step {
        let mut st = Step::default();
        for group in self.latching(|h| h.is_none()) {
            self.latch_group(&group, &mut st, acquired);
        }
        self.run(&mut st, acquired);
        st
    }

    /// The distinct groups parked in the Latch stage whose head matches `which`.
    fn latching(&self, which: impl Fn(Option<u32>) -> bool) -> Vec<Vec<u32>> {
        let mut groups: Vec<ChanSet> = Vec::new();
        for c in self.chans.iter().flatten() {
            if let Stage::Latch { head, group, .. } = c.stage
                && which(head)
                && !groups.contains(&group)
            {
                groups.push(group);
            }
        }
        groups
            .into_iter()
            .map(|g| (0..CHANNELS as u32).filter(|n| g & bit(*n) != 0).collect())
            .collect()
    }

    /// Is any update waiting for an acquire without a vblank to re-check it?
    #[must_use]
    pub fn acquire_pending(&self) -> bool {
        self.chans
            .iter()
            .flatten()
            .any(|c| matches!(c.stage, Stage::Latch { head: None, .. }))
    }

    fn decode(&mut self, chn: u32, pb: &[u8], put: u32, st: &mut Step) {
        let Some(c) = self.chans.get_mut(chn as usize).and_then(|c| c.as_mut()) else {
            return;
        };
        if c.halted || c.decoded == put {
            return;
        }
        let (groups, end, err) = pushbuf::decode_groups(pb, c.decoded, put);
        let over = c.queue.len() + groups.len() > MAX_QUEUE;
        if !over {
            c.queue.extend(groups);
            c.decoded = end;
            // ⊘ A pass that decodes NO write (the wrap JUMP NVKMS writes at the end of the ring, NOPs,
            // SET_SUBDEVICE_MASK) still moves the fetch pointer: GET follows it, or NVKMS — which
            // waits for GET to pass its PUT after a wrap (`nvEvoMakeRoom`) — waits forever
            // (`[measured m1a, 2026-09-30, GA106 / 580.159.04]` GET 4040 : PUT 4032, the JUMP's
            // own offset).
            if c.queue.is_empty() {
                c.get = c.decoded;
            }
        }
        if over || err.is_some() {
            let what = match err {
                _ if over => {
                    format!("more than {MAX_QUEUE} undispatched methods (a PUT that ignored GET)")
                }
                Some(DecodeError::BadOpcode { word, .. }) => {
                    format!("opcode {} in word {word:#010x}", word >> 29)
                }
                Some(DecodeError::Truncated { .. }) => {
                    "a method's data runs past PUT or the pushbuffer".to_string()
                }
                Some(DecodeError::BadPointers) => format!(
                    "PUT {put:#x} / GET {:#x} outside the {}-byte pushbuffer",
                    c.decoded,
                    pb.len()
                ),
                Some(DecodeError::Runaway) => "a JUMP cycle".to_string(),
                None => String::new(),
            };
            c.halted = true;
            self.exceptions += 1;
            st.effects.push(Effect::Exception { chn, at: end, what });
        }
    }

    /// Execute every channel as far as it can go, completing each ready update group.
    fn run(&mut self, st: &mut Step, acquired: &mut dyn FnMut(&Acquire) -> bool) {
        // Bounded: each pass either consumes a write, completes a group, or stops.
        for _ in 0..(CHANNELS * (MAX_QUEUE + 2)) {
            let mut progressed = false;
            for n in 0..CHANNELS as u32 {
                progressed |= self.exec(n, st);
            }
            // interlock groups whose every member has arrived
            if let Some(group) = self.ready_group() {
                self.group_ready(&group, st, acquired);
                progressed = true;
            }
            // a group parked for a head that went idle (F7: its tick will never come)
            progressed |= self.unpark_idle(st, acquired);
            if !progressed {
                break;
            }
        }
        // publish every channel's GET (a no-op for the plane when unchanged)
        for (n, c) in self.chans.iter().enumerate() {
            if let Some(c) = c {
                st.gets.push((n as u32, c.life, c.get));
            }
        }
    }

    /// Run channel `n` until it stops. Returns whether it consumed anything.
    fn exec(&mut self, n: u32, st: &mut Step) -> bool {
        let vocab = self.vocab.clone();
        let Some(c) = self.chans.get_mut(n as usize).and_then(|c| c.as_mut()) else {
            return false;
        };
        if c.halted || c.stage != Stage::Running {
            return false;
        }
        let space = vocab.space(c.kind);
        let update = vocab.update_of(c.kind);
        let mut any = false;
        while let Some(l) = c.queue.front().copied() {
            let m = l.write.method;
            if m % 4 != 0 || m >= space {
                c.halted = true;
                c.get = l.header;
                self.exceptions += 1;
                st.effects.push(Effect::Exception {
                    chn: n,
                    at: l.header,
                    what: format!(
                        "method {m:#x} outside the {space:#x}-byte method space of {:?} {}",
                        c.kind, c.instance
                    ),
                });
                return any;
            }
            if self.method_trace_remaining > 0 {
                self.method_trace_remaining -= 1;
                st.effects.push(Effect::Trace(format!(
                    "METHOD chn={n} kind={:?} method={m:#x} data={:#x} remaining={}",
                    c.kind, l.write.data, self.method_trace_remaining
                )));
            }
            if m == update {
                self.ledger.committed[n as usize] += 1;
                c.commit = Some(std::time::Instant::now());
                c.acq_block = None;
                c.commit_info = format!(
                    "update={:#x} iso0_assy={:#x} sem_ctxdma_assy={:#x} notif_ctxdma_assy={:#x}",
                    l.write.data,
                    c.a(vocab.w_iso0),
                    c.a(vocab.w_ctxdma_sem),
                    c.a(vocab.w_ctxdma_notifier)
                );
                let ilk = interlock_set(&vocab, c, l.write.data);
                c.stage = Stage::Interlock {
                    update: l.write.data,
                    ilk,
                };
                c.get = l.header;
                if self.trace {
                    st.effects.push(Effect::Trace(format!(
                        "chn {n} UPDATE {:#x} at {:#x} waits for {ilk:#x}",
                        l.write.data, l.header
                    )));
                }
                return any;
            }
            if c.kind == ChannelKind::Window {
                for &(method, stage, entry, idx, value) in &vocab.w_color_inline {
                    if method != m {
                        continue;
                    }
                    let index = fld(l.write.data, idx) as usize;
                    let data = fld(l.write.data, value);
                    let table =
                        &mut c.assy_inline.as_mut().expect("window inline allocation")[stage];
                    let valid = if entry {
                        table
                            .entries
                            .get_mut(index)
                            .map(|slot| *slot = Some(data as u16))
                            .is_some()
                    } else {
                        table
                            .segments
                            .get_mut(index)
                            .map(|slot| *slot = Some(data as u8))
                            .is_some()
                    };
                    if !valid {
                        c.halted = true;
                        c.get = l.header;
                        self.exceptions += 1;
                        st.effects.push(Effect::Exception {
                            chn: n,
                            at: l.header,
                            what: "inline CSC LUT index outside fixed extent".into(),
                        });
                        return any;
                    }
                }
            }
            c.assy[(m / 4) as usize] = l.write.data;
            self.methods += 1;
            c.queue.pop_front();
            // GET stays at a group's header until its last write is consumed, then moves past it
            c.get = match c.queue.front() {
                Some(f) if f.header == l.header => l.header,
                Some(f) => f.header,
                None => c.decoded,
            };
            any = true;
        }
        any
    }

    /// The first set of channels stopped at an UPDATE whose interlocks are all satisfied.
    fn ready_group(&self) -> Option<Vec<u32>> {
        let pending: ChanSet = (0..CHANNELS as u32)
            .filter(|n| {
                matches!(
                    self.chans[*n as usize].as_ref().map(|c| c.stage),
                    Some(Stage::Interlock { .. })
                )
            })
            .fold(0, |m, n| m | bit(n));
        if pending == 0 {
            return None;
        }
        let live: ChanSet = (0..CHANNELS as u32)
            .filter(|n| self.chans[*n as usize].is_some())
            .fold(0, |m, n| m | bit(n));
        for start in 0..CHANNELS as u32 {
            if pending & bit(start) == 0 {
                continue;
            }
            // the closure of `start` over interlock edges IN BOTH DIRECTIONS, among pending channels:
            // what a member waits for, and every pending update that waits for a member.
            // ★ 2026-10-10 (Windows TDR hunt, run 293, measured): an overlay window's UPDATE interlocked
            // with window 0 and its immediate channel, while window 0's own UPDATEs named nothing; with
            // forward edges only, window 0 latched alone at every vblank (54 times) and the overlay
            // update starved until the TDR. An interlocked UPDATE waits for an UPDATE on each channel
            // it names, and is latched TOGETHER with it (`ogkm-595.84:
            // nvidia-modeset/src/nvkms-evo3.c:2829-2837`), so a pending update another pending update
            // waits for cannot latch without it.
            let mut group = bit(start);
            let mut ready = true;
            loop {
                let mut want = 0;
                for n in 0..CHANNELS as u32 {
                    if let Some(Stage::Interlock { ilk, .. }) =
                        self.chans[n as usize].as_ref().map(|c| c.stage)
                    {
                        if group & bit(n) != 0 {
                            want |= ilk & live;
                        } else if ilk & group != 0 {
                            want |= bit(n);
                        }
                    }
                }
                if want & !pending != 0 {
                    ready = false;
                    break;
                }
                let next = group | want;
                if next == group {
                    break;
                }
                group = next;
            }
            if ready {
                return Some(
                    (0..CHANNELS as u32)
                        .filter(|n| group & bit(*n) != 0)
                        .collect(),
                );
            }
        }
        None
    }

    /// A group is ready: latch it now, or park it for a vblank / an acquire.
    fn group_ready(
        &mut self,
        group: &[u32],
        st: &mut Step,
        acquired: &mut dyn FnMut(&Acquire) -> bool,
    ) {
        // a non-tearing window on an active head latches at that head's vblank (with the group);
        // ★ D1: so does a tearing one whose head already presented since its last tick
        let heads = self.heads_armed();
        let mut vblank_head = None;
        let mut tear_head = None;
        for &n in group {
            let Some(c) = self.chans[n as usize].as_ref() else {
                continue;
            };
            if c.kind != ChannelKind::Window {
                continue;
            }
            let Some(h) = self
                .owner_head(c.instance)
                .filter(|h| heads.iter().any(|m| m.head == *h && m.period_ns > 0))
            else {
                continue;
            };
            if self.tearing(c) {
                tear_head = Some(tear_head.unwrap_or(h));
            } else {
                vblank_head = Some(vblank_head.unwrap_or(h));
            }
        }
        let has_core = group.contains(&0);
        let gated = tear_head
            .filter(|h| self.tear_gate && vblank_head.is_none() && self.presented[*h as usize]);
        let park = if has_core {
            // ⚠ H-corelatch: the first active head's next vblank (none active: at once, as before)
            self.core_latch_at_vblank
                .then(|| heads.iter().find(|m| m.period_ns > 0).map(|m| m.head))
                .flatten()
        } else {
            vblank_head.or(gated)
        };
        if let Some(h) = gated.filter(|_| !has_core) {
            self.pace[h as usize].tear_held += 1;
        }
        let set = group.iter().fold(0, |m, n| m | bit(*n));
        for &n in group {
            if let Some(c) = self.chans[n as usize].as_mut()
                && let Stage::Interlock { update, .. } = c.stage
            {
                c.stage = Stage::Latch {
                    update,
                    head: park,
                    group: set,
                };
            }
        }
        if self.trace {
            st.effects.push(Effect::Trace(format!(
                "group {group:?} ready, latch {}",
                park.map_or("now".to_string(), |h| format!("at head {h}'s vblank"))
            )));
        }
        if park.is_none() {
            self.latch_group(group, st, acquired);
        }
    }

    /// Is window channel `c`'s pending flip a TEARING one (`SET_PRESENT_CONTROL.BEGIN_MODE` other
    /// than `NON_TEARING`: nvidia-drm's async flips program `IMMEDIATE`)?
    fn tearing(&self, c: &Chan) -> bool {
        fld(c.a(self.vocab.w_present), self.vocab.w_present_begin)
            != self.vocab.w_present_non_tearing
    }

    /// ★ F7: groups parked for a head that is no longer active would wait for a tick that never
    /// comes — they become acquire-only waits and are latched now when their acquires hold (the
    /// acquire poll re-evaluates the rest). Returns whether any was unparked.
    fn unpark_idle(&mut self, st: &mut Step, acquired: &mut dyn FnMut(&Acquire) -> bool) -> bool {
        let heads = self.heads_armed();
        let active = |h: u32| heads.iter().any(|m| m.head == h && m.period_ns > 0);
        let mut any = false;
        for c in self.chans.iter_mut().flatten() {
            if let Stage::Latch {
                update,
                head: Some(h),
                group,
            } = c.stage
                && !active(h)
            {
                c.stage = Stage::Latch {
                    update,
                    head: None,
                    group,
                };
                any = true;
            }
        }
        if any {
            for group in self.latching(|h| h.is_none()) {
                self.latch_group(&group, st, acquired);
            }
        }
        any
    }

    /// Latch the members of `group` that are in the Latch stage, if every acquire among them holds.
    fn latch_group(
        &mut self,
        group: &[u32],
        st: &mut Step,
        acquired: &mut dyn FnMut(&Acquire) -> bool,
    ) {
        let members: Vec<u32> = group
            .iter()
            .copied()
            .filter(|n| {
                matches!(
                    self.chans[*n as usize].as_ref().map(|c| c.stage),
                    Some(Stage::Latch { .. })
                )
            })
            .collect();
        if members.is_empty() {
            return;
        }
        // every acquire of the group must hold; otherwise the whole group keeps waiting (its vblank
        // or the acquire poll re-evaluates it)
        for &n in &members {
            if let Some(a) = self.acquire_of(n)
                && !acquired(&a)
            {
                if let Some(c) = self.chans[n as usize].as_mut()
                    && c.acq_block.is_none()
                {
                    c.acq_block = Some(std::time::Instant::now());
                    c.acq_info = format!(
                        "acquire ctxdma={:#x} +{:#x} want={:#x} wide={} mode={}",
                        a.handle, a.offset, a.value, a.wide, a.mode
                    );
                }
                return;
            }
        }
        let heads_before = self.heads_armed();
        // ★ A window's flip raises its FLIP event (AWAKEN) only if the window was scanning a surface on
        // an active head BEFORE this update: nvidia-drm queues events only for "planes which were
        // active previously" — "Hardware generates flip event for only those planes"
        // (`ogkm-580: kernel-open/nvidia-drm/nvidia-drm-modeset.c:93-135`), and WARNs on any other
        // (`[measured m1b, 2026-09-30, GA106 / 580.159.04]` the first fbdev modeset:
        // `WARN_ON(nv_flip == NULL)`). Snapshotted before any member of the group — the core among
        // them — is armed.
        // ⊘ The rule's other half is the NEW state's notifier (`complete`): a window flipped to NO
        // surface is programmed with no notifier (`nvkms-evo3.c:3901-3904`, the KAPI sets one only
        // for a non-NULL surface, `nvkms-kapi.c:2956-2970`, `:3175-3189`), so it raises nothing,
        // although nvidia-drm counted an event for it.
        // `[measured m1c, 2026-09-30, GA106 / 580.159.04]` that is the one "Flip event timeout" of
        // the lane: the probe exited with its framebuffer on the plane, the kernel's
        // `atomic_remove_fb` disabled the plane in a blocking commit, and nvidia-drm waited 3 s for
        // an event no hardware sends — real GPUs log the same (NVIDIA/open-gpu-kernel-modules#1361,
        // "framebuffer removal on DRM file close"). Raising an event there instead would be a
        // completion for work that has no notifier; the probe restores its CRTC instead.
        let was_active: Vec<(u32, bool)> = members
            .iter()
            .map(|n| (*n, self.window_was_active(*n, &heads_before)))
            .collect();
        // ★ `display-max-fps`: one present per head with a window in this latch (an active head
        // before it), by path — taken before any member (the core among them) is armed
        let mut presented = [None::<bool>; 8];
        for &n in &members {
            if let Some(c) = self.chans[n as usize].as_ref()
                && c.kind == ChannelKind::Window
                && let Some(h) = self
                    .owner_head(c.instance)
                    .filter(|h| heads_before.iter().any(|m| m.head == *h && m.period_ns > 0))
                && let Some(p) = presented.get_mut(h as usize)
            {
                *p = Some(p.unwrap_or(false) | self.tearing(c));
            }
        }
        let has_core = members.contains(&0);
        for (h, p) in presented.iter().enumerate() {
            if let Some(tearing) = *p {
                self.presented[h] = true;
                let pc = &mut self.pace[h];
                pc.presents += 1;
                pc.tearing += u64::from(tearing);
                pc.core_imm += u64::from(has_core);
            }
        }
        if self.trace {
            st.effects.push(Effect::Trace(format!(
                "latch {members:?} (previously active: {was_active:?})"
            )));
        }
        for &(n, active) in &was_active {
            self.complete(n, active, st);
        }
        if self.heads_armed() != heads_before {
            st.effects.push(Effect::Heads);
        }
    }

    /// Was channel `n` a window scanning a surface on an active head (per `heads`)?
    fn window_was_active(&self, n: u32, heads: &[HeadMode]) -> bool {
        let Some(c) = self.chans.get(n as usize).and_then(|c| c.as_ref()) else {
            return false;
        };
        c.kind == ChannelKind::Window
            && c.armed(self.vocab.w_iso0) != 0
            && self
                .owner_head(c.instance)
                .is_some_and(|h| heads.iter().any(|m| m.head == h && m.period_ns > 0))
    }

    /// The acquire a window's pending flip waits on, if any.
    fn acquire_of(&self, n: u32) -> Option<Acquire> {
        let c = self.chans.get(n as usize)?.as_ref()?;
        if c.kind != ChannelKind::Window {
            return None;
        }
        let v = &self.vocab;
        let handle = c.a(v.w_ctxdma_acq);
        if handle == 0 {
            return None;
        }
        let ctl = c.a(v.w_acq_control);
        let wide = fld(ctl, v.w_acq_payload) == 1;
        let hi = v.w_acq_value_hi.map_or(0, |m| c.a(m));
        Some(Acquire {
            chn: n,
            client: c.client,
            handle,
            offset: u64::from(fld(ctl, v.w_acq_offset)) * 16,
            value: if wide {
                u64::from(hi) << 32 | u64::from(c.a(v.w_acq_value))
            } else {
                u64::from(c.a(v.w_acq_value))
            },
            wide,
            mode: fld(ctl, v.w_acq_mode),
        })
    }

    /// ★ Complete channel `n`'s update: ARM, then state its completions, then consume the UPDATE.
    /// `was_active`: a window that scanned a surface on an active head before the update (only then
    /// does its notifier raise the flip event).
    fn complete(&mut self, n: u32, was_active: bool, st: &mut Step) {
        let v = self.vocab.clone();
        let at_latch = self.release_at_latch;
        let finish_away = self.notifier_finish_at_flip_away;
        let Some(c) = self.chans.get_mut(n as usize).and_then(|c| c.as_mut()) else {
            return;
        };
        let Stage::Latch { .. } = c.stage else { return };
        // ★ 2026-10-10 (TDR hunt, shape F): a window's RELEASE semaphore is written when the entry it was programmed
        // with is FLIPPED AWAY — replaced by the next latched update — not when that entry itself latches. Open NVKMS
        // states the EVO/NVDisplay behaviour it simulates in software: "We write the semaphore's release value when the
        // NVHsChannelFlipQueueEntry is removed from current (i.e., when we do the equivalent of 'flip away')"
        // (`ogkm-595.84: nvidia-modeset/include/nvkms-headsurface-priv.h:236-244`). So the release written at this latch
        // is the OUTGOING armed state's, read before the arm below. [measured, runs 268-279] writing the incoming
        // entry's release at its own latch told a guest whose VSync handling ran after the latch pass that the flip it
        // was waiting for had already been flipped away; it never reported that present, its flip queue timed out (TDR).
        // (⚠ `release_at_latch`, diagnostic: the incoming entry's release — its ASSEMBLY words, armed just below)
        let outgoing_release = (c.kind == ChannelKind::Window)
            .then(|| Self::release_of(&v, c, n, if at_latch { Chan::a } else { Chan::armed }))
            .flatten();
        // the outgoing entry's notifier, FINISHED at its flip-away — read before the arm
        let outgoing_notify = (finish_away && c.kind == ChannelKind::Window)
            .then(|| {
                let handle = c.armed(v.w_ctxdma_notifier);
                (handle != 0).then(|| Effect::Notify {
                    chn: n,
                    client: c.client,
                    handle,
                    offset: u64::from(fld(c.armed(v.w_notifier_control), v.n_offset)) * 16,
                    awaken: false,
                    finished: true,
                })
            })
            .flatten();
        // 1. arm
        let mut changed = Vec::new();
        for (i, (a, b)) in c.assy.iter().zip(c.armed.iter_mut()).enumerate() {
            if *a != *b {
                *b = *a;
                changed.push(((i * 4) as u32, *a));
            }
        }
        if let (Some(assy), Some(armed)) = (&c.assy_inline, &mut c.armed_inline) {
            armed.clone_from(assy);
        }
        self.updates += 1;
        {
            let idx = n as usize;
            self.ledger.completed[idx] += 1;
            if let Some(t0) = c.commit.take() {
                let ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
                let blocked = c.acq_block.take().map(|t| t.elapsed().as_millis());
                if blocked.is_some() {
                    self.ledger.acq_blocked[idx] += 1;
                }
                self.ledger.max_ms[idx] = self.ledger.max_ms[idx].max(ms);
                if ms > 50 {
                    self.ledger.slow_n[idx] += 1;
                    if self.ledger.slow.len() < 64 {
                        self.ledger.slow.push(format!(
                            "chn {n} {:?}#{} commit->complete {ms} ms (acquire blocked {:?} ms: {}); {}",
                            c.kind, c.instance, blocked, c.acq_info, c.commit_info
                        ));
                    }
                }
            }
        }
        // 2. the completions it asked for — only now that the state is armed
        match c.kind {
            ChannelKind::Core => {
                if !changed.is_empty() {
                    st.effects.push(Effect::CoreArmed(changed));
                }
                let ctl = c.armed(v.c_notifier_control);
                let handle = c.armed(v.c_ctxdma_notifier);
                if fld(ctl, v.n_notify) == 1 && handle != 0 {
                    st.effects.push(Effect::Notify {
                        chn: n,
                        client: c.client,
                        handle,
                        offset: u64::from(fld(ctl, v.n_offset)) * 16,
                        awaken: fld(ctl, v.n_mode) == v.n_mode_awaken,
                        finished: true,
                    });
                }
            }
            ChannelKind::Window => {
                st.effects.push(Effect::Latched { window: c.instance });
                // the entry this latch flipped away (see `outgoing_release` above), never the one just armed
                if let Some(r) = outgoing_release {
                    st.effects.push(r);
                }
                if let Some(o) = outgoing_notify {
                    st.effects.push(o);
                }
                let handle = c.armed(v.w_ctxdma_notifier);
                if handle != 0 {
                    let ctl = c.armed(v.w_notifier_control);
                    st.effects.push(Effect::Notify {
                        chn: n,
                        client: c.client,
                        handle,
                        offset: u64::from(fld(ctl, v.n_offset)) * 16,
                        awaken: was_active && fld(ctl, v.n_mode) == v.n_mode_awaken,
                        finished: false,
                    });
                }
            }
            ChannelKind::WindowImm | ChannelKind::Cursor => {}
        }
        // 3. consume the UPDATE and run on
        let consumed = c.queue.pop_front();
        c.stage = Stage::Running;
        if let Some(l) = consumed
            && c.queue.front().is_none_or(|f| f.header != l.header)
        {
            c.get = c.queue.front().map_or(c.decoded, |f| f.header);
        }
    }

    /// ⚠ DIAGNOSTIC (slot history, 2026-10-10 TDR hunt): window channel `n`'s notifier and release slots as its
    /// ASSEMBLY (`armed == false`: the request just committed) or ARMED state names them, as `Notify` / `Release`
    /// effects (never queued — the display thread only samples the slots they name).
    #[must_use]
    pub fn window_slots(&self, n: u32, armed: bool) -> Vec<Effect> {
        let Some(c) = self.chans.get(n as usize).and_then(|c| c.as_ref()) else {
            return Vec::new();
        };
        if c.kind != ChannelKind::Window {
            return Vec::new();
        }
        let v = &self.vocab;
        let get: fn(&Chan, u32) -> u32 = if armed { Chan::armed } else { Chan::a };
        let mut out = Vec::new();
        let handle = get(c, v.w_ctxdma_notifier);
        if handle != 0 {
            let ctl = get(c, v.w_notifier_control);
            out.push(Effect::Notify {
                chn: n,
                client: c.client,
                handle,
                offset: u64::from(fld(ctl, v.n_offset)) * 16,
                awaken: false,
                finished: false,
            });
        }
        out.extend(Self::release_of(v, c, n, get));
        out
    }

    /// The release a window entry asks for, read through `get` (its ARMED or ASSEMBLY words): `None` without a
    /// semaphore context DMA.
    fn release_of(v: &Vocab, c: &Chan, n: u32, get: fn(&Chan, u32) -> u32) -> Option<Effect> {
        let sem = get(c, v.w_ctxdma_sem);
        if sem == 0 {
            return None;
        }
        let ctl = get(c, v.w_sem_control);
        let wide = fld(ctl, v.w_sem_payload) == 1;
        let hi = v.w_sem_release_hi.map_or(0, |m| get(c, m));
        let lo = u64::from(get(c, v.w_sem_release));
        Some(Effect::Release {
            chn: n,
            client: c.client,
            handle: sem,
            offset: u64::from(fld(ctl, v.w_sem_offset)) * 16,
            value: if wide { u64::from(hi) << 32 | lo } else { lo },
            wide,
            awaken: fld(ctl, v.w_sem_rel_mode) == 1,
        })
    }

    /// The head window `w` is owned by in the ARMED core state (`None`: no owner or no core).
    fn owner_head(&self, w: u32) -> Option<u32> {
        let core = self.chans[0].as_ref()?;
        let (b, s) = self.vocab.c_window_set_control;
        let o = fld(core.armed(b + w * s), self.vocab.c_window_owner);
        (o != self.vocab.c_owner_none && o < self.heads).then_some(o)
    }

    /// ★ Every head's armed raster (the vblank timers follow this).
    #[must_use]
    pub fn heads_armed(&self) -> Vec<HeadMode> {
        let v = &self.vocab;
        (0..self.heads)
            .map(|h| {
                let Some(core) = self.chans[0].as_ref() else {
                    return HeadMode {
                        head: h,
                        period_ns: 0,
                        raster: (0, 0),
                    };
                };
                let pclk = core.armed(v.c_pclk.0 + h * v.c_pclk.1);
                let mut hz = u64::from(fld(pclk, v.c_pclk_hz));
                if fld(pclk, v.c_pclk_adj) == 1 {
                    hz = hz * 1000 / 1001;
                }
                let rs = core.armed(v.c_raster_size.0 + h * v.c_raster_size.1);
                let (w, ht) = (fld(rs, v.c_raster_w), fld(rs, v.c_raster_h));
                let period_ns = if hz == 0 || w == 0 || ht == 0 {
                    0
                } else {
                    u64::from(w) * u64::from(ht) * 1_000_000_000 / hz
                };
                HeadMode {
                    head: h,
                    period_ns,
                    raster: (w, ht),
                }
            })
            .collect()
    }

    /// The core's armed words (for a full republication, e.g. after a core re-allocation).
    #[must_use]
    pub fn core_armed(&self) -> Option<&[u32]> {
        self.chans[0].as_ref().map(|c| c.armed.as_slice())
    }

    /// A channel's armed word at method `m` (the scanout reads windows' surfaces through this).
    #[must_use]
    pub fn armed(&self, kind: ChannelKind, instance: u32, m: u32) -> Option<u32> {
        let n = self.channel_number(kind, instance)?;
        Some(self.chans[n as usize].as_ref()?.armed(m))
    }

    /// Indexed tables from the same armed window incarnation as its method bank.
    #[must_use]
    pub fn armed_inline(&self, window: u32) -> Option<&[crate::color::InlineLut; 2]> {
        let n = self.channel_number(ChannelKind::Window, window)?;
        self.chans[n as usize].as_ref()?.armed_inline.as_deref()
    }

    /// Is channel `chn` stopped at an update (busy even with nothing left to decode)?
    #[must_use]
    pub fn waiting(&self, chn: u32) -> bool {
        self.chans
            .get(chn as usize)
            .and_then(|c| c.as_ref())
            .is_some_and(|c| c.stage != Stage::Running)
    }
}

impl Engine {
    /// ★ Per head, the SOR it lights: the lowest SOR whose ARMED `SOR_SET_CONTROL.OWNER_MASK` names
    /// the head, while the head's raster runs — what `NV0073_CTRL_CMD_SYSTEM_GET_ACTIVE` reports
    /// (as the connector on that SOR). `None` for an idle head or one no SOR drives.
    #[must_use]
    pub fn lit_sors(&self) -> Vec<Option<u32>> {
        let v = &self.vocab;
        let heads = self.heads_armed();
        (0..self.heads)
            .map(|h| {
                let core = self.chans[0].as_ref()?;
                if !heads.iter().any(|m| m.head == h && m.period_ns > 0) {
                    return None;
                }
                (0..self.heads).find(|s| {
                    let ctl = core.armed(v.c_sor_control.0 + s * v.c_sor_control.1);
                    h < 8 && fld(ctl, v.c_sor_owner) & (1 << h) != 0
                })
            })
            .collect()
    }
}

/// ★ What a head scans out: the first enabled window it owns, read from the ARMED state (M2's scanout
/// copy source). Raw class values — the plane resolves the context DMA and bounds every byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scanout {
    /// Window index.
    pub window: u32,
    /// The owning head.
    pub head: u32,
    /// The window's channel number and client (the context-DMA hash key).
    pub chn: u32,
    /// Client.
    pub client: u32,
    /// The ISO context DMA handle (`SET_CONTEXT_DMA_ISO(0)`).
    pub handle: u32,
    /// Byte offset into it (`SET_OFFSET(0)` is in 256-byte units, `nvCtxDmaOffsetFromBytes`).
    pub offset: u64,
    /// The source rectangle (`SET_SIZE_IN`).
    pub width: u32,
    /// Height of the source rectangle.
    pub height: u32,
    /// Its origin in the surface (`SET_POINT_IN`).
    pub x: u32,
    /// Origin row.
    pub y: u32,
    /// The surface's own size (`SET_SIZE`).
    pub surface_width: u32,
    /// The surface's height.
    pub surface_height: u32,
    /// `SET_PLANAR_STORAGE(0).PITCH`: 64-byte units (pitch) or blocks (block-linear) — the
    /// context DMA's KIND decides which (`nvkms-evo3.c:4065-4085`).
    pub pitch: u32,
    /// `SET_STORAGE.BLOCK_HEIGHT` (log2 GOBs per block, block-linear only).
    pub block_height_log2: u32,
    /// `SET_PARAMS.FORMAT`.
    pub format: u32,
    /// Where the window sits in the head's composition space (window-immediate
    /// `SET_POINT_OUT(0)`).
    pub out_x: u32,
    /// Output row.
    pub out_y: u32,
    /// The window's output size (`SET_SIZE_OUT`; the console does not scale: it shows `SIZE_IN`
    /// clipped to this).
    pub out_width: u32,
    /// Output height.
    pub out_height: u32,
    /// `SET_COMPOSITION_CONTROL.DEPTH` — smaller is closer to the front (`nvkms-evo3.c:4813`).
    pub depth: u32,
    /// `SET_COMPOSITION_CONSTANT_ALPHA.K1` / `.K2`.
    pub k1: u32,
    /// K2.
    pub k2: u32,
    /// `SET_COMPOSITION_FACTOR_SELECT` `SRC_COLOR_FACTOR_NO_MATCH_SELECT` (color key disabled).
    pub src_factor: u32,
    /// `DST_COLOR_FACTOR_NO_MATCH_SELECT`.
    pub dst_factor: u32,
}

/// ★ What a head shows: its composition space and every enabled window, BACK TO FRONT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composition {
    /// The head.
    pub head: u32,
    /// `HEAD_SET_VIEWPORT_SIZE_IN` — the composed image the windows land in.
    pub width: u32,
    /// Its height.
    pub height: u32,
    /// Enabled windows, deepest first.
    pub layers: Vec<Scanout>,
}

/// The window-class methods a scanout reads — resolved from the derived table; `None` for a family
/// whose windows name surfaces by address (GB20x `CA7E`, M5).
#[derive(Debug, Clone, Copy)]
pub struct ScanVocab {
    iso0: u32,
    offset0: u32,
    size: (u32, (u8, u8), (u8, u8)),
    size_in: (u32, (u8, u8), (u8, u8)),
    point_in: (u32, (u8, u8), (u8, u8)),
    pitch0: (u32, (u8, u8)),
    params: (u32, (u8, u8)),
    storage: (u32, (u8, u8)),
    size_out: (u32, (u8, u8), (u8, u8)),
    comp_depth: (u32, (u8, u8)),
    comp_alpha: (u32, (u8, u8), (u8, u8)),
    comp_factor: (u32, (u8, u8), (u8, u8)),
    /// Window-immediate `SET_POINT_OUT(0)`.
    point_out: (u32, (u8, u8), (u8, u8)),
    /// Core `HEAD_SET_VIEWPORT_SIZE_IN(head)`: base, stride, width, height.
    viewport_in: (u32, u32, (u8, u8), (u8, u8)),
}

/// A method's offset and two `(hi, lo)` fields of its data word — e.g. `SET_SIZE`'s `_WIDTH` and
/// `_HEIGHT`.
type MethodTwoFields = (u32, (u8, u8), (u8, u8));

impl ScanVocab {
    /// Resolve for window class `win`, window-immediate class `winim` and core class `core`.
    #[must_use]
    pub fn resolve(t: &ClassTable, win: u32, winim: u32, core: u32) -> Option<ScanVocab> {
        let wh = |n: &str| -> Option<MethodTwoFields> {
            Some((
                t.v(win, n)?,
                t.f(win, &format!("{n}_WIDTH"))?,
                t.f(win, &format!("{n}_HEIGHT"))?,
            ))
        };
        Some(ScanVocab {
            iso0: t.a(win, "SET_CONTEXT_DMA_ISO", 0)?,
            offset0: t.a(win, "SET_OFFSET", 0)?,
            size: wh("SET_SIZE")?,
            size_in: wh("SET_SIZE_IN")?,
            point_in: (
                t.a(win, "SET_POINT_IN", 0)?,
                t.f(win, "SET_POINT_IN_X")?,
                t.f(win, "SET_POINT_IN_Y")?,
            ),
            pitch0: (
                t.a(win, "SET_PLANAR_STORAGE", 0)?,
                t.f(win, "SET_PLANAR_STORAGE_PITCH")?,
            ),
            params: (t.v(win, "SET_PARAMS")?, t.f(win, "SET_PARAMS_FORMAT")?),
            storage: (
                t.v(win, "SET_STORAGE")?,
                t.f(win, "SET_STORAGE_BLOCK_HEIGHT")?,
            ),
            size_out: wh("SET_SIZE_OUT")?,
            comp_depth: (
                t.v(win, "SET_COMPOSITION_CONTROL")?,
                t.f(win, "SET_COMPOSITION_CONTROL_DEPTH")?,
            ),
            comp_alpha: (
                t.v(win, "SET_COMPOSITION_CONSTANT_ALPHA")?,
                t.f(win, "SET_COMPOSITION_CONSTANT_ALPHA_K1")?,
                t.f(win, "SET_COMPOSITION_CONSTANT_ALPHA_K2")?,
            ),
            comp_factor: (
                t.v(win, "SET_COMPOSITION_FACTOR_SELECT")?,
                t.f(
                    win,
                    "SET_COMPOSITION_FACTOR_SELECT_SRC_COLOR_FACTOR_NO_MATCH_SELECT",
                )?,
                t.f(
                    win,
                    "SET_COMPOSITION_FACTOR_SELECT_DST_COLOR_FACTOR_NO_MATCH_SELECT",
                )?,
            ),
            point_out: (
                t.a(winim, "SET_POINT_OUT", 0)?,
                t.f(winim, "SET_POINT_OUT_X")?,
                t.f(winim, "SET_POINT_OUT_Y")?,
            ),
            viewport_in: {
                let b = t.a(core, "HEAD_SET_VIEWPORT_SIZE_IN", 0)?;
                let s = t.a(core, "HEAD_SET_VIEWPORT_SIZE_IN", 1)? - b;
                (
                    b,
                    s,
                    t.f(core, "HEAD_SET_VIEWPORT_SIZE_IN_WIDTH")?,
                    t.f(core, "HEAD_SET_VIEWPORT_SIZE_IN_HEIGHT")?,
                )
            },
        })
    }
}

impl Engine {
    /// ★ What head `head` scans out now (its lowest enabled window), or `None`.
    #[must_use]
    pub fn scanout(&self, sv: &ScanVocab, head: u32) -> Option<Scanout> {
        (0..self.windows).find_map(|w| self.window_scan(sv, head, w))
    }

    /// ★ Everything head `head` shows: its composition space and its enabled windows, back to front
    /// (deepest `DEPTH` first; equal depths in window order). `None` when no window is enabled.
    #[must_use]
    pub fn composition(&self, sv: &ScanVocab, head: u32) -> Option<Composition> {
        let mut layers: Vec<Scanout> = (0..self.windows)
            .filter_map(|w| self.window_scan(sv, head, w))
            .collect();
        if layers.is_empty() {
            return None;
        }
        layers.sort_by(|a, b| b.depth.cmp(&a.depth).then(a.window.cmp(&b.window)));
        let core = self.chans[0].as_ref();
        let (vb, vs, vw, vh) = sv.viewport_in;
        let vp = core.map_or(0, |c| c.armed(vb + head * vs));
        let (mut width, mut height) = (fld(vp, vw), fld(vp, vh));
        if width == 0 || height == 0 {
            // no viewport stated: the deepest window's output size
            (width, height) = (layers[0].out_width, layers[0].out_height);
        }
        Some(Composition {
            head,
            width,
            height,
            layers,
        })
    }

    /// Window `w` as head `head` scans it, if it is the head's and enabled.
    fn window_scan(&self, sv: &ScanVocab, head: u32, w: u32) -> Option<Scanout> {
        {
            if self.owner_head(w) != Some(head) {
                return None;
            }
            let chn = ChannelKind::Window.channel_number(w);
            let c = self.chans.get(chn as usize)?.as_ref()?;
            let handle = c.armed(sv.iso0);
            if handle == 0 {
                return None;
            }
            let (sm, sw, sh) = sv.size;
            let (im, iw, ih) = sv.size_in;
            let (pm, px, py) = sv.point_in;
            let (om, ow, oh) = sv.size_out;
            let imm = self
                .chans
                .get(ChannelKind::WindowImm.channel_number(w) as usize)
                .and_then(|c| c.as_ref());
            let (qm, qx, qy) = sv.point_out;
            let point_out = imm.map_or(0, |i| i.armed(qm));
            let alpha = c.armed(sv.comp_alpha.0);
            let factor = c.armed(sv.comp_factor.0);
            Some(Scanout {
                window: w,
                head,
                chn,
                client: c.client,
                handle,
                offset: u64::from(c.armed(sv.offset0)) << 8,
                width: fld(c.armed(im), iw),
                height: fld(c.armed(im), ih),
                x: fld(c.armed(pm), px),
                y: fld(c.armed(pm), py),
                surface_width: fld(c.armed(sm), sw),
                surface_height: fld(c.armed(sm), sh),
                pitch: fld(c.armed(sv.pitch0.0), sv.pitch0.1),
                block_height_log2: fld(c.armed(sv.storage.0), sv.storage.1),
                format: fld(c.armed(sv.params.0), sv.params.1),
                out_x: fld(point_out, qx),
                out_y: fld(point_out, qy),
                out_width: fld(c.armed(om), ow),
                out_height: fld(c.armed(om), oh),
                depth: fld(c.armed(sv.comp_depth.0), sv.comp_depth.1),
                k1: fld(alpha, sv.comp_alpha.1),
                k2: fld(alpha, sv.comp_alpha.2),
                src_factor: fld(factor, sv.comp_factor.1),
                dst_factor: fld(factor, sv.comp_factor.2),
            })
        }
    }
}

/// ★ Display step 3d (`docs/design/V3_DISPLAY.md` §8.6): the core- and cursor-class methods a
/// head's cursor is read from — RESOLVED from the derived class table; `None` for a family whose
/// table lacks one (its cursor is then not composed, and nothing else changes).
#[derive(Debug, Clone, Copy)]
pub struct CursorVocab {
    /// Core `HEAD_SET_CONTEXT_DMA_CURSOR(head, 0)`: base, head stride.
    ctxdma: (u32, u32),
    /// Core `HEAD_SET_OFFSET_CURSOR(head, 0)`: base, head stride (256-byte units).
    offset: (u32, u32),
    /// Core `HEAD_SET_CONTROL_CURSOR(head)`: base, head stride.
    control: (u32, u32),
    enable: (u8, u8),
    format: (u8, u8),
    size: (u8, u8),
    hot_x: (u8, u8),
    hot_y: (u8, u8),
    /// `HEAD_SET_CONTROL_CURSOR_FORMAT_A8R8G8B8` — the only format NVKMS programs
    /// (`ogkm-580: src/nvidia-modeset/src/nvkms-evo3.c:6517-6524`).
    a8r8g8b8: u32,
    /// Core `HEAD_SET_CONTROL_CURSOR_COMPOSITION(head)`: base, head stride.
    comp: (u32, u32),
    k1: (u8, u8),
    cursor_factor: (u8, u8),
    viewport_factor: (u8, u8),
    mode: (u8, u8),
    /// Cursor PIO `SET_CURSOR_HOT_SPOT_POINT_OUT(0)` and its `X`, `Y`.
    point_out: MethodTwoFields,
}

impl CursorVocab {
    /// Resolve for core class `core` and cursor PIO class `cursor`.
    #[must_use]
    pub fn resolve(t: &ClassTable, core: u32, cursor: u32) -> Option<CursorVocab> {
        let a = |n: &str| -> Option<(u32, u32)> {
            let b = t.a(core, n, 0)?;
            Some((b, t.a(core, n, 1)?.checked_sub(b)?))
        };
        let a2 = |n: &str| -> Option<(u32, u32)> {
            let b = t.a2(core, n, 0, 0)?;
            Some((b, t.a2(core, n, 1, 0)?.checked_sub(b)?))
        };
        let f = |n: &str| t.f(core, n);
        Some(CursorVocab {
            ctxdma: a2("HEAD_SET_CONTEXT_DMA_CURSOR")?,
            offset: a2("HEAD_SET_OFFSET_CURSOR")?,
            control: a("HEAD_SET_CONTROL_CURSOR")?,
            enable: f("HEAD_SET_CONTROL_CURSOR_ENABLE")?,
            format: f("HEAD_SET_CONTROL_CURSOR_FORMAT")?,
            size: f("HEAD_SET_CONTROL_CURSOR_SIZE")?,
            hot_x: f("HEAD_SET_CONTROL_CURSOR_HOT_SPOT_X")?,
            hot_y: f("HEAD_SET_CONTROL_CURSOR_HOT_SPOT_Y")?,
            a8r8g8b8: t.v(core, "HEAD_SET_CONTROL_CURSOR_FORMAT_A8R8G8B8")?,
            comp: a("HEAD_SET_CONTROL_CURSOR_COMPOSITION")?,
            k1: f("HEAD_SET_CONTROL_CURSOR_COMPOSITION_K1")?,
            cursor_factor: f("HEAD_SET_CONTROL_CURSOR_COMPOSITION_CURSOR_COLOR_FACTOR_SELECT")?,
            viewport_factor: f("HEAD_SET_CONTROL_CURSOR_COMPOSITION_VIEWPORT_COLOR_FACTOR_SELECT")?,
            mode: f("HEAD_SET_CONTROL_CURSOR_COMPOSITION_MODE")?,
            point_out: (
                t.a(cursor, "SET_CURSOR_HOT_SPOT_POINT_OUT", 0)?,
                t.f(cursor, "SET_CURSOR_HOT_SPOT_POINT_OUT_X")?,
                t.f(cursor, "SET_CURSOR_HOT_SPOT_POINT_OUT_Y")?,
            ),
        })
    }
}

/// ★ A head's enabled cursor, as the armed core state and the cursor channel's last `Update` place
/// it — the TOP layer of the head's composition (§8.6, display step 3d).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorScan {
    /// The head.
    pub head: u32,
    /// The core channel's client (the context DMA's hash key, with channel 0).
    pub client: u32,
    /// `HEAD_SET_CONTEXT_DMA_CURSOR(head, 0)`.
    pub handle: u32,
    /// Byte offset into it (`HEAD_SET_OFFSET_CURSOR` is in 256-byte units, `nvCtxDmaOffsetFromBytes`).
    pub offset: u64,
    /// The image's edge in pixels (`SIZE`: 32, 64, 128 or 256; square).
    pub size: u32,
    /// `FORMAT` is `A8R8G8B8`.
    pub argb8888: bool,
    /// `HOT_SPOT_X` / `_Y` inside the image (NVKMS programs 0, `nvkms-evo3.c:6568-6569`).
    pub hot_x: u32,
    /// Hot spot row.
    pub hot_y: u32,
    /// Where the hot spot lands on the head (`SET_CURSOR_HOT_SPOT_POINT_OUT`, signed 16-bit: the
    /// cursor may hang off the top or left edge).
    pub x: i32,
    /// Hot spot row on the head.
    pub y: i32,
    /// `HEAD_SET_CONTROL_CURSOR_COMPOSITION`: `K1`, the two factor selects (window-factor
    /// numbering), and `MODE` (0 blend, 1 XOR).
    pub k1: u32,
    /// `CURSOR_COLOR_FACTOR_SELECT`.
    pub cursor_factor: u32,
    /// `VIEWPORT_COLOR_FACTOR_SELECT`.
    pub viewport_factor: u32,
    /// `MODE`.
    pub mode: u32,
}

impl Engine {
    /// ★ Head `head`'s cursor, if it is enabled and names a surface.
    #[must_use]
    pub fn cursor_scan(&self, cv: &CursorVocab, head: u32) -> Option<CursorScan> {
        if head >= self.heads {
            return None;
        }
        let core = self.chans.first()?.as_ref()?;
        let at = |(b, s): (u32, u32)| b.checked_add(head.checked_mul(s)?);
        let ctl = core.armed(at(cv.control)?);
        if fld(ctl, cv.enable) == 0 {
            return None;
        }
        let handle = core.armed(at(cv.ctxdma)?);
        if handle == 0 {
            return None;
        }
        let comp = core.armed(at(cv.comp)?);
        let (pm, px, py) = cv.point_out;
        let point = self.armed(ChannelKind::Cursor, head, pm).unwrap_or(0);
        // two's-complement 16-bit fields: the cursor may hang off the top or left edge
        let signed = |v: u32| i32::from((v & 0xFFFF) as u16 as i16);
        Some(CursorScan {
            head,
            client: core.client,
            handle,
            offset: u64::from(core.armed(at(cv.offset)?)) << 8,
            size: 32 << fld(ctl, cv.size).min(3),
            argb8888: fld(ctl, cv.format) == cv.a8r8g8b8,
            hot_x: fld(ctl, cv.hot_x),
            hot_y: fld(ctl, cv.hot_y),
            x: signed(fld(point, px)),
            y: signed(fld(point, py)),
            k1: fld(comp, cv.k1),
            cursor_factor: fld(comp, cv.cursor_factor),
            viewport_factor: fld(comp, cv.viewport_factor),
            mode: fld(comp, cv.mode),
        })
    }
}

/// The channels an UPDATE on `c` (with data `update`) waits for.
fn interlock_set(v: &Vocab, c: &Chan, update: u32) -> ChanSet {
    let mut s = 0;
    let cursors = |flags: u32, first: (u8, u8)| -> ChanSet {
        (0..8u32)
            .filter(|h| flags >> (u32::from(first.1) + h) & 1 == 1)
            .fold(0, |m, h| m | bit(ChannelKind::Cursor.channel_number(h)))
    };
    let windows = |flags: u32| -> ChanSet {
        (0..32u32)
            .filter(|w| flags >> w & 1 == 1)
            .fold(0, |m, w| m | bit(ChannelKind::Window.channel_number(w)))
    };
    match c.kind {
        ChannelKind::Core => {
            s |= cursors(c.a(v.c_interlock), v.c_ilk_cursor0);
            s |= windows(c.a(v.c_window_interlock));
        }
        ChannelKind::Window => {
            if fld(c.a(v.w_interlock), v.w_ilk_core) == 1 {
                s |= bit(0);
            }
            s |= cursors(c.a(v.w_interlock), v.w_ilk_cursor0);
            s |= windows(c.a(v.w_window_interlock))
                & !bit(ChannelKind::Window.channel_number(c.instance));
            if fld(update, v.w_update_ilk_winim) == 1 {
                s |= bit(ChannelKind::WindowImm.channel_number(c.instance));
            }
        }
        ChannelKind::WindowImm => {
            if fld(update, v.i_ilk_window) == 1 {
                s |= bit(ChannelKind::Window.channel_number(c.instance));
            }
        }
        ChannelKind::Cursor => {
            if v.k_ilk_core
                .is_some_and(|f| fld(c.a(v.k_interlock), f) == 1)
            {
                s |= bit(0);
            }
            s |= windows(c.a(v.k_window_interlock));
        }
    }
    s
}

#[cfg(test)]
mod tests;
