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
    assy: Vec<u32>,
    armed: Vec<u32>,
    stage: Stage,
    halted: bool,
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
            assy: vec![0; words],
            armed: vec![0; words],
            stage: Stage::Running,
            halted: false,
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

/// ★ The engine.
#[derive(Debug)]
pub struct Engine {
    vocab: Vocab,
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
}

impl Engine {
    /// An engine for `heads` heads and `windows` windows speaking `vocab`.
    #[must_use]
    pub fn new(vocab: Vocab, heads: u32, windows: u32) -> Engine {
        Engine {
            vocab,
            heads: heads.min(8),
            windows: windows.min(32),
            chans: (0..CHANNELS).map(|_| None).collect(),
            updates: 0,
            methods: 0,
            exceptions: 0,
            trace: false,
        }
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

    /// ★ Feed channel `chn` the pushbuffer bytes `pb` (its whole ring) up to `put`, and run every
    /// channel whose update became ready. `acquired` answers an acquire against guest memory.
    pub fn step(
        &mut self,
        chn: u32,
        pb: &[u8],
        put: u32,
        acquired: &mut dyn FnMut(&Acquire) -> bool,
    ) -> Step {
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
        let space = self.vocab.other_space;
        if let Some(c) = self.chans[chn as usize].as_mut() {
            if off % 4 != 0 || off >= space {
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

    /// ★ Head `head`'s vblank: latch every update waiting for it (whose acquires hold).
    pub fn vblank(&mut self, head: u32, acquired: &mut dyn FnMut(&Acquire) -> bool) -> Step {
        let mut st = Step::default();
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
            // (`[measured m1a]` GET 4040 : PUT 4032, the JUMP's own offset).
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
            if m == update {
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
            // the closure of `start` over interlock edges, among pending channels
            let mut group = bit(start);
            let mut ready = true;
            loop {
                let mut want = 0;
                for n in 0..CHANNELS as u32 {
                    if group & bit(n) != 0
                        && let Some(Stage::Interlock { ilk, .. }) =
                            self.chans[n as usize].as_ref().map(|c| c.stage)
                    {
                        want |= ilk & live;
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
        // a non-tearing window on an active head latches at that head's vblank (with the group)
        let heads = self.heads_armed();
        let mut vblank_head = None;
        for &n in group {
            let Some(c) = self.chans[n as usize].as_ref() else {
                continue;
            };
            if c.kind == ChannelKind::Window
                && fld(c.a(self.vocab.w_present), self.vocab.w_present_begin)
                    == self.vocab.w_present_non_tearing
            {
                let owner = self.owner_head(c.instance);
                if let Some(h) =
                    owner.filter(|h| heads.iter().any(|m| m.head == *h && m.period_ns > 0))
                {
                    vblank_head = Some(vblank_head.unwrap_or(h));
                }
            }
        }
        let has_core = group.contains(&0);
        let park = if has_core { None } else { vblank_head };
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
                return;
            }
        }
        let heads_before = self.heads_armed();
        // ★ A window's flip raises its FLIP event (AWAKEN) only if the window was scanning a surface on
        // an active head BEFORE this update: nvidia-drm queues events only for "planes which were
        // active previously" — "Hardware generates flip event for only those planes"
        // (`ogkm-580: kernel-open/nvidia-drm/nvidia-drm-modeset.c:93-135`), and WARNs on any other
        // (`[measured m1b]` the first fbdev modeset: `WARN_ON(nv_flip == NULL)`). Snapshotted before
        // any member of the group — the core among them — is armed.
        // ⊘ The rule's other half is the NEW state's notifier (`complete`): a window flipped to NO
        // surface is programmed with no notifier (`nvkms-evo3.c:3901-3904`, the KAPI sets one only
        // for a non-NULL surface, `nvkms-kapi.c:2956-2970`, `:3175-3189`), so it raises nothing,
        // although nvidia-drm counted an event for it. `[measured m1c]` that is the one "Flip event
        // timeout" of the lane: the probe exited with its framebuffer on the plane, the kernel's
        // `atomic_remove_fb` disabled the plane in a blocking commit, and nvidia-drm waited 3 s for
        // an event no hardware sends — real GPUs log the same (NVIDIA/open-gpu-kernel-modules#1361,
        // "framebuffer removal on DRM file close"). Raising an event there instead would be a
        // completion for work that has no notifier; the probe restores its CRTC instead.
        let was_active: Vec<(u32, bool)> = members
            .iter()
            .map(|n| (*n, self.window_was_active(*n, &heads_before)))
            .collect();
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
        let Some(c) = self.chans.get_mut(n as usize).and_then(|c| c.as_mut()) else {
            return;
        };
        let Stage::Latch { .. } = c.stage else { return };
        // 1. arm
        let mut changed = Vec::new();
        for (i, (a, b)) in c.assy.iter().zip(c.armed.iter_mut()).enumerate() {
            if *a != *b {
                *b = *a;
                changed.push(((i * 4) as u32, *a));
            }
        }
        self.updates += 1;
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
                    });
                }
            }
            ChannelKind::Window => {
                st.effects.push(Effect::Latched { window: c.instance });
                let sem = c.armed(v.w_ctxdma_sem);
                if sem != 0 {
                    let ctl = c.armed(v.w_sem_control);
                    let wide = fld(ctl, v.w_sem_payload) == 1;
                    let hi = v.w_sem_release_hi.map_or(0, |m| c.armed(m));
                    let lo = u64::from(c.armed(v.w_sem_release));
                    st.effects.push(Effect::Release {
                        chn: n,
                        client: c.client,
                        handle: sem,
                        offset: u64::from(fld(ctl, v.w_sem_offset)) * 16,
                        value: if wide { u64::from(hi) << 32 | lo } else { lo },
                        wide,
                        awaken: fld(ctl, v.w_sem_rel_mode) == 1,
                    });
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
}

impl ScanVocab {
    /// Resolve for window class `win`.
    #[must_use]
    pub fn resolve(t: &ClassTable, win: u32) -> Option<ScanVocab> {
        let wh = |n: &str| -> Option<(u32, (u8, u8), (u8, u8))> {
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
        })
    }
}

impl Engine {
    /// ★ What head `head` scans out now (its lowest enabled window), or `None`.
    #[must_use]
    pub fn scanout(&self, sv: &ScanVocab, head: u32) -> Option<Scanout> {
        (0..self.windows).find_map(|w| {
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
            })
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
