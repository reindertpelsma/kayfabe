//! ★★★ **The display plane** — the kf3 device's emulated NVDisplay, running (`docs/design/V3_DISPLAY.md`
//! §4.3–§4.5, steps (2)/(3)).
//!
//! Three threads touch it, and each does only what its constraints allow (`THE_CONSTRAINTS.md`):
//! - **a vCPU** (a BAR0 write into the display aperture): a PUT is posted to the shared
//!   [`Ports`] and the worker is woken — one store and one eventfd write; a cursor PIO method is
//!   posted likewise; an event register's write-1-to-clear is applied to its atomic word and the
//!   ISR's summary registers are republished — a pure bit operation. ⊘ No lock, no guest-memory
//!   access, no host call, and nothing reaches the privileged ring (the GSP state machine owns no
//!   display register).
//! - **the register drainer** (a physical-RM control or an alloc/free RPC): answered by the shared
//!   [`kf_disp::model::DisplayModel`] under its lock, which queues statements for the worker and
//!   wakes it (`kf_rm::display`). The drainer never waits on the worker.
//! - **the display worker** ([`crate::device::Device::display_loop`], its own thread): drains the
//!   statements, reads each channel's pushbuffer out of guest memory, runs the
//!   [`kf_disp::engine::Engine`], and performs its effects IN ORDER — the ARMED mirror first, then the
//!   notifiers and semaphores the guest waits on (written only after the engine applied the update
//!   they report), then GET, then the display interrupt. Guest system memory is read and written
//!   through the RAM QEMU registered; guest video memory (the context-DMA table in display
//!   instance memory, a video-memory notifier) only by the GPU, through the plane's own CUDA context
//!   ([`kf_cuda::display::DisplayGpu`], §38 — the CPU never reads guest vidmem).
//!
//! ⊘ Owner rule A.3: the display is an EMULATED device — no GPU work sits behind its channels
//! (§37) — so its completions are the end of its own processing, and none is written before the
//! state it reports is armed. Vblank is a host timer (the worker's poll deadline) at the armed
//! raster's refresh.
//!
//! ⊘ Hostile guest (only the guest KERNEL allocates display channels): every register offset is
//! decoded against the derived vocabulary, every pushbuffer and context-DMA access is bounded by
//! the guest RAM block or the store it falls in and by the context DMA's own limit, every PUT is a
//! channel number the display has; a malformed stream stops its channel by name.

use crate::device::Device;
use kf_cuda::display::{DisplayGpu, Frame, PitchRect};
use kf_disp::engine::{Acquire, Effect, Engine, PbLoc, ScanVocab, Scanout, Vocab};
use kf_disp::inst::{CtxDma, Layout, Target};
use kf_disp::model::{ChannelKind, Statement, Waker};
use kf_disp::ports::{EventReg, Ports};
use kf_disp::regs::Regs;
use kf_disp::scanout::{CopyPlan, ScanFormats};
use kf_linux_raw::{Notifier, PollTimeout, Poller, ReadyTokens};
use kf_rm::display::SharedDisplayModel;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Heads any family's register file indexes.
const MAX_HEADS: usize = kf_disp::ports::MAX_HEADS;

/// ★ Every BAR0 offset the plane decodes, RESOLVED from the derived register and class tables at
/// realize — so the vCPU path compares integers and a missing name refuses the device up front.
#[derive(Debug, Clone)]
pub struct RegMap {
    lo: u64,
    hi: u64,
    heads: u32,
    windows: u32,
    core_assy: u64,
    core_armed: u64,
    core_len: u64,
    win: (u64, u64),
    winim: (u64, u64),
    curs: (u64, u64),
    user_len: u64,
    put: u64,
    get: u64,
    cursor_free: u64,
    cursor_update: u32,
    evt_awaken_win: u64,
    evt_awaken_other: u64,
    awaken_core_bit: u32,
    evt_sem_win: Option<u64>,
    evt_head_timing: (u64, u64),
    head_last_data: u32,
    head_vblank: u32,
    rm_intr_en_head_timing: Option<(u64, u64)>,
    rm_intr_stat_head_timing: (u64, u64),
    rm_intr_dispatch: u64,
    rm_ctrl_disp: u64,
    rm_ctrl_awaken_bit: u8,
    rm_ctrl_win_sem_bit: Option<u8>,
    evt_dispatch: Option<(u64, u32)>,
    chnctl: [(u64, u64); 4],
    chnctl_alloc: u32,
    chnstatus: [(u64, u64); 4],
    chnstatus_state: [(u8, u8); 4],
    chnstatus_idle: [u32; 4],
    chnstatus_busy: [u32; 4],
    core_head_state: Option<(u64, u64, (u8, u8), u32, u32)>,
    rg_dpca: (u64, u64, (u8, u8)),
    loadv: Option<(u64, u64)>,
}

fn kind_index(k: ChannelKind) -> usize {
    match k {
        ChannelKind::Core => 0,
        ChannelKind::Window => 1,
        ChannelKind::WindowImm => 2,
        ChannelKind::Cursor => 3,
    }
}

/// What a vCPU write in the display aperture is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispWrite {
    /// PUT of DMA channel number `chn`.
    Put(u32),
    /// A write the channel's user area does not accept from the guest (GET, the ARMED half, a
    /// cursor's `Free`): dropped, the register keeps the engine's value.
    ReadOnly,
    /// A cursor PIO method: head, offset inside the cursor's user area.
    Cursor(u32, u32),
    /// An event / interrupt-enable register.
    Event(EventReg),
    /// Any other display register: a plain shadow store, as every B page.
    Plain,
}

impl RegMap {
    /// Resolve the map for a family.
    ///
    /// # Errors
    /// The first name the derived tables lack.
    pub fn resolve(
        r: &Regs,
        t: &kf_disp::class::ClassTable,
        row: &kf_chip::display::DisplayRow,
    ) -> Result<RegMap, String> {
        let miss =
            |n: &str| format!("display register vocabulary: {n} is not derived for this family");
        let v = |n: &str| r.v(n).ok_or_else(|| miss(n));
        let a2 = |n: &str| -> Result<(u64, u64), String> {
            let b = r.a(n, 0).ok_or_else(|| miss(n))?;
            Ok((b, r.a(n, 1).ok_or_else(|| miss(n))? - b))
        };
        let f = |n: &str| r.f32(n).ok_or_else(|| miss(n));
        let bit = |n: &str| -> Result<u32, String> { f(n).map(|(_, lo)| 1u32 << lo) };
        let rt = r.table();
        let (hi, lo) = rt
            .f_in("v04_01", "NV_PDISP")
            .ok_or_else(|| miss("NV_PDISP"))?;
        let core_assy = v("NV_UDISP_FE_CHN_ASSY_BASEADR_CORE")?;
        let core_armed = v("NV_UDISP_FE_CHN_ARMED_BASEADR_CORE")?;
        let win = a2("NV_UDISP_FE_CHN_ASSY_BASEADR_WIN")?;
        let cls = |n: &str| {
            t.v(row.classes.core, n)
                .map(u64::from)
                .ok_or_else(|| miss(&format!("NV{:04X}_{n}", row.classes.core)))
        };
        let chn_state = |k: &str| -> Result<((u8, u8), u32, u32), String> {
            Ok((
                f(&format!("NV_PDISP_FE_CHNSTATUS_{k}_STATE"))?,
                r.v32(&format!("NV_PDISP_FE_CHNSTATUS_{k}_STATE_IDLE"))
                    .ok_or_else(|| miss(k))?,
                r.v32(&format!("NV_PDISP_FE_CHNSTATUS_{k}_STATE_BUSY"))
                    .ok_or_else(|| miss(k))?,
            ))
        };
        let (cs, ci, cb) = chn_state("CORE")?;
        let (ws, wi, wb) = chn_state("WIN")?;
        let (is, ii, ib) = chn_state("WINIM")?;
        let (ks, ki, kb) = chn_state("CURS")?;
        let core_head_state = match r.a("NV_PDISP_FE_CORE_HEAD_STATE", 0) {
            Some(b) => {
                let s = r
                    .a("NV_PDISP_FE_CORE_HEAD_STATE", 1)
                    .ok_or_else(|| miss("CORE_HEAD_STATE"))?
                    - b;
                Some((
                    b,
                    s,
                    f("NV_PDISP_FE_CORE_HEAD_STATE_OPERATING_MODE")?,
                    r.v32("NV_PDISP_FE_CORE_HEAD_STATE_OPERATING_MODE_AWAKE")
                        .ok_or_else(|| miss("OPERATING_MODE_AWAKE"))?,
                    r.v32("NV_PDISP_FE_CORE_HEAD_STATE_OPERATING_MODE_SLEEP")
                        .ok_or_else(|| miss("OPERATING_MODE_SLEEP"))?,
                ))
            }
            None => None,
        };
        let dpca = a2("NV_PDISP_RG_DPCA")?;
        Ok(RegMap {
            lo,
            hi,
            heads: row.heads.min(MAX_HEADS as u32),
            windows: row.windows.min(32),
            core_assy,
            core_armed,
            core_len: 2 * (core_armed - core_assy),
            win,
            winim: a2("NV_UDISP_FE_CHN_ASSY_BASEADR_WINIM")?,
            curs: a2("NV_UDISP_FE_CHN_ASSY_BASEADR_CURS")?,
            user_len: win.1,
            put: cls("PUT")?,
            get: cls("GET")?,
            cursor_free: t
                .v(row.classes.cursor, "FREE")
                .map(u64::from)
                .ok_or_else(|| miss("cursor FREE"))?,
            cursor_update: t
                .v(row.classes.cursor, "UPDATE")
                .ok_or_else(|| miss("cursor UPDATE"))?,
            evt_awaken_win: v("NV_PDISP_FE_EVT_STAT_AWAKEN_WIN")?,
            evt_awaken_other: v("NV_PDISP_FE_EVT_STAT_AWAKEN_OTHER")?,
            awaken_core_bit: bit("NV_PDISP_FE_EVT_STAT_AWAKEN_OTHER_CORE")?,
            evt_sem_win: r.v("NV_PDISP_FE_EVT_STAT_SEM_WIN"),
            evt_head_timing: a2("NV_PDISP_FE_EVT_STAT_HEAD_TIMING")?,
            head_last_data: bit("NV_PDISP_FE_EVT_STAT_HEAD_TIMING_LAST_DATA")?,
            head_vblank: bit("NV_PDISP_FE_EVT_STAT_HEAD_TIMING_VBLANK")?,
            rm_intr_en_head_timing: a2("NV_PDISP_FE_RM_INTR_EN_HEAD_TIMING").ok(),
            rm_intr_stat_head_timing: a2("NV_PDISP_FE_RM_INTR_STAT_HEAD_TIMING")?,
            rm_intr_dispatch: v("NV_PDISP_FE_RM_INTR_DISPATCH")?,
            rm_ctrl_disp: v("NV_PDISP_FE_RM_INTR_STAT_CTRL_DISP")?,
            rm_ctrl_awaken_bit: f("NV_PDISP_FE_RM_INTR_STAT_CTRL_DISP_AWAKEN")?.1,
            rm_ctrl_win_sem_bit: r
                .f32("NV_PDISP_FE_RM_INTR_STAT_CTRL_DISP_WIN_SEM")
                .map(|x| x.1),
            evt_dispatch: r.v("NV_PDISP_FE_EVT_DISPATCH").zip(
                r.f32("NV_PDISP_FE_EVT_DISPATCH_SEM_WIN")
                    .map(|x| 1u32 << x.1),
            ),
            chnctl: [
                (v("NV_PDISP_FE_CHNCTL_CORE")?, 0),
                a2("NV_PDISP_FE_CHNCTL_WIN")?,
                a2("NV_PDISP_FE_CHNCTL_WINIM")?,
                a2("NV_PDISP_FE_CHNCTL_CURS")?,
            ],
            chnctl_alloc: bit("NV_PDISP_FE_CHNCTL_CORE_ALLOCATION")?,
            chnstatus: [
                (v("NV_PDISP_FE_CHNSTATUS_CORE")?, 0),
                a2("NV_PDISP_FE_CHNSTATUS_WIN")?,
                a2("NV_PDISP_FE_CHNSTATUS_WINIM")?,
                a2("NV_PDISP_FE_CHNSTATUS_CURS")?,
            ],
            chnstatus_state: [cs, ws, is, ks],
            chnstatus_idle: [ci, wi, ii, ki],
            chnstatus_busy: [cb, wb, ib, kb],
            core_head_state,
            rg_dpca: (dpca.0, dpca.1, f("NV_PDISP_RG_DPCA_FRM_CNT")?),
            loadv: a2("NV_PDISP_POSTCOMP_HEAD_LOADV_COUNTER").ok(),
        })
    }

    /// Is BAR0 offset `off` in the display aperture (`NV_PDISP`)?
    #[must_use]
    pub fn owns(&self, off: u64) -> bool {
        off >= self.lo && off <= self.hi
    }

    /// The user area of channel `(kind, instance)`.
    #[must_use]
    pub fn user_base(&self, kind: ChannelKind, i: u32) -> u64 {
        match kind {
            ChannelKind::Core => self.core_assy,
            ChannelKind::Window => self.win.0 + u64::from(i) * self.win.1,
            ChannelKind::WindowImm => self.winim.0 + u64::from(i) * self.winim.1,
            ChannelKind::Cursor => self.curs.0 + u64::from(i) * self.curs.1,
        }
    }

    fn in_array(&self, off: u64, (base, stride): (u64, u64), n: u32) -> Option<(u32, u64)> {
        let rel = off.checked_sub(base)?;
        let i = rel / stride;
        (i < u64::from(n)).then(|| (i as u32, rel % stride))
    }

    /// ★ **vCPU**: what a write at `off` is (pure arithmetic on resolved offsets).
    #[must_use]
    pub fn classify(&self, off: u64) -> DispWrite {
        if (self.core_assy..self.core_assy + self.core_len).contains(&off) {
            let rel = off - self.core_assy;
            return if rel == self.put {
                DispWrite::Put(0)
            } else if rel == self.get || rel >= self.core_armed - self.core_assy {
                DispWrite::ReadOnly
            } else {
                DispWrite::Plain
            };
        }
        for (arr, first) in [(self.win, 1u32), (self.winim, 33u32)] {
            if let Some((i, rel)) = self.in_array(off, arr, self.windows) {
                return if rel == self.put {
                    DispWrite::Put(first + i)
                } else if rel == self.get || rel >= self.user_len / 2 {
                    DispWrite::ReadOnly
                } else {
                    DispWrite::Plain
                };
            }
        }
        if let Some((h, rel)) = self.in_array(off, self.curs, self.heads) {
            return if rel == self.cursor_free || rel >= self.user_len / 2 {
                DispWrite::ReadOnly
            } else {
                DispWrite::Cursor(h, rel as u32)
            };
        }
        if off == self.evt_awaken_win {
            return DispWrite::Event(EventReg::AwakenWin);
        }
        if off == self.evt_awaken_other {
            return DispWrite::Event(EventReg::AwakenOther);
        }
        if Some(off) == self.evt_sem_win {
            return DispWrite::Event(EventReg::SemWin);
        }
        if let Some((h, 0)) = self.in_array(off, self.evt_head_timing, self.heads) {
            return DispWrite::Event(EventReg::HeadTiming(h as usize));
        }
        if let Some((h, 0)) = self
            .rm_intr_en_head_timing
            .and_then(|a| self.in_array(off, a, self.heads))
        {
            return DispWrite::Event(EventReg::HeadTimingEn(h as usize));
        }
        // the ISR's summary registers are read-only (derived)
        if off == self.rm_ctrl_disp
            || off == self.rm_intr_dispatch
            || self
                .in_array(off, self.rm_intr_stat_head_timing, self.heads)
                .is_some_and(|(_, r)| r == 0)
        {
            return DispWrite::ReadOnly;
        }
        DispWrite::Plain
    }
}

/// One head's cursor PIO registers, posted by vCPUs (the latest value per register, and an
/// `Update` count the worker follows).
#[derive(Debug, Default)]
pub struct CursorPorts {
    regs: [AtomicU32; 8],
    offs: [AtomicU32; 8],
    updates: AtomicU32,
}

impl CursorPorts {
    /// ★ **vCPU**: post a cursor method write. `update` = this was the `Update` method.
    fn post(&self, rel: u32, v: u32, update: bool) {
        if update {
            self.updates.fetch_add(1, Ordering::AcqRel);
            return;
        }
        // a small fixed register file: the slot of `rel`, or the first free one
        for i in 0..self.offs.len() {
            let o = self.offs[i].load(Ordering::Acquire);
            if o == rel + 1
                || (o == 0
                    && self.offs[i]
                        .compare_exchange(0, rel + 1, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok())
            {
                self.regs[i].store(v, Ordering::Release);
                return;
            }
        }
    }
}

/// Boot-log counters.
#[derive(Debug, Default)]
pub struct DispCounters {
    /// Display-aperture writes from vCPUs.
    pub writes: AtomicU64,
    /// Updates the engine completed.
    pub updates: AtomicU64,
    /// Methods executed.
    pub methods: AtomicU64,
    /// Notifiers written.
    pub notifies: AtomicU64,
    /// Semaphores released.
    pub releases: AtomicU64,
    /// Channel exceptions.
    pub exceptions: AtomicU64,
    /// Effects refused (a context DMA that did not resolve, an access outside its bounds).
    pub refused: AtomicU64,
    /// Display interrupts sent.
    pub irqs: AtomicU64,
    /// Vblanks ticked.
    pub vblanks: AtomicU64,
    /// Scanout copies completed (M2): frames the console can show.
    pub scanouts: AtomicU64,
    /// Scanouts refused (a surface the console cannot copy, by name in the log).
    pub scanout_refused: AtomicU64,
}

/// Console frame slots: one the console shows, one ready, one the GPU fills.
const SLOTS: usize = 3;
/// "No slot" in [`ConsoleShare`]'s state word.
const NO_SLOT: u32 = 0xF;

fn pack(front: u32, ready: u32) -> u32 {
    (front & 0xF) | ((ready & 0xF) << 4)
}

fn unpack(s: u32) -> (u32, u32) {
    (s & 0xF, (s >> 4) & 0xF)
}

/// One published frame's description (the slot's frame memory is the worker's).
#[derive(Debug, Default)]
struct FrameSlot {
    addr: AtomicUsize,
    width: AtomicU32,
    height: AtomicU32,
    stride: AtomicU32,
    format: AtomicU32,
    serial: AtomicU64,
}

/// ★ What the console reads: a frame's host address, geometry, format and serial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameView {
    /// Host address of the first pixel (page-locked memory the worker owns for the process).
    pub addr: usize,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes per row.
    pub stride: u32,
    /// [`kf_disp::scanout::PixelFormat`] code.
    pub format: u32,
    /// Increases with every frame the worker publishes.
    pub serial: u64,
}

/// ★★ M2 — the frames the display worker hands QEMU's console, lock-free (`V3_DISPLAY.md` §4.6).
///
/// Triple buffering in one atomic word `(front, ready)`: the console takes `ready` as its new
/// `front` ([`ConsoleShare::take`]); the worker fills a slot that is NEITHER (there is always one,
/// three slots minus two), then publishes it as `ready`, dropping an untaken older one. The console
/// can only move `ready` to `front`, so the slot the GPU is writing is never the one on screen.
/// ⊘ Frame memory is never freed while the device lives (a screendump may still hold a pixman image
/// of an old front after the console moved on — a stale read is harmless, a freed page is not).
#[derive(Debug)]
pub struct ConsoleShare {
    state: AtomicU32,
    slots: [FrameSlot; SLOTS],
    /// Milliseconds (since the plane's start) of the console's last request — the refresh rate
    /// follows demand.
    demand_ms: AtomicU64,
    epoch: Instant,
}

impl Default for ConsoleShare {
    fn default() -> ConsoleShare {
        ConsoleShare {
            state: AtomicU32::new(pack(NO_SLOT, NO_SLOT)),
            slots: core::array::from_fn(|_| FrameSlot::default()),
            demand_ms: AtomicU64::new(0),
            epoch: Instant::now(),
        }
    }
}

impl ConsoleShare {
    /// ★ **Console (QEMU's main thread)**: the newest frame — the ready one becomes the front — or
    /// `None` before the first. The returned memory stays valid and unwritten until the next call.
    pub fn take(&self) -> Option<FrameView> {
        let now = u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.demand_ms.store(now.max(1), Ordering::Relaxed);
        loop {
            let s = self.state.load(Ordering::Acquire);
            let (front, ready) = unpack(s);
            let show = if ready == NO_SLOT { front } else { ready };
            if show == NO_SLOT {
                return None;
            }
            if ready != NO_SLOT
                && self
                    .state
                    .compare_exchange(s, pack(ready, NO_SLOT), Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
            {
                continue;
            }
            let sl = &self.slots[show as usize];
            return Some(FrameView {
                addr: sl.addr.load(Ordering::Acquire),
                width: sl.width.load(Ordering::Acquire),
                height: sl.height.load(Ordering::Acquire),
                stride: sl.stride.load(Ordering::Acquire),
                format: sl.format.load(Ordering::Acquire),
                serial: sl.serial.load(Ordering::Acquire),
            });
        }
    }

    /// **Worker**: a slot neither shown nor ready — the next copy's target.
    fn free_slot(&self) -> usize {
        let (front, ready) = unpack(self.state.load(Ordering::Acquire));
        (0..SLOTS as u32)
            .find(|i| *i != front && *i != ready)
            .unwrap_or(0) as usize
    }

    /// **Worker**: describe slot `i`'s finished frame, then make it the ready one.
    fn publish(&self, i: usize, f: FrameView) {
        let sl = &self.slots[i];
        sl.addr.store(f.addr, Ordering::Release);
        sl.width.store(f.width, Ordering::Release);
        sl.height.store(f.height, Ordering::Release);
        sl.stride.store(f.stride, Ordering::Release);
        sl.format.store(f.format, Ordering::Release);
        sl.serial.store(f.serial, Ordering::Release);
        let mut s = self.state.load(Ordering::Acquire);
        loop {
            let (front, _) = unpack(s);
            match self.state.compare_exchange(
                s,
                pack(front, i as u32),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(now) => s = now,
            }
        }
    }

    /// Did the console ask for a frame within the last `ms` milliseconds?
    fn wanted_within(&self, ms: u64) -> bool {
        let last = self.demand_ms.load(Ordering::Relaxed);
        let now = u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX);
        last != 0 && now.saturating_sub(last) <= ms
    }
}

/// What the worker takes at its start.
struct WorkerInit {
    engine: Engine,
    gpu: Option<DisplayGpu>,
    layout: Layout,
    notifier_finished: u32,
}

/// ★ The plane (leaked for the process, like the device).
pub struct DisplayPlane {
    /// The model the served chain answers from — shared across `ReselectAtFn1` rebuilds.
    pub model: SharedDisplayModel,
    /// The lock-free ports.
    pub ports: Arc<Ports>,
    /// Resolved registers.
    pub map: RegMap,
    /// The caps page, published at seal.
    caps: kf_disp::caps::CapsPage,
    wake: Arc<Notifier>,
    init: Mutex<Option<WorkerInit>>,
    /// Per head: the cursor's posted PIO methods.
    cursor: [CursorPorts; MAX_HEADS],
    /// Counters.
    pub counters: DispCounters,
    /// ★ M2: the frames QEMU's console shows.
    pub console: ConsoleShare,
    /// The window-class methods a scanout reads (`None`: the family's windows name surfaces by
    /// address — no console yet, M5).
    scan: Option<ScanVocab>,
    /// The window formats the console can show.
    formats: ScanFormats,
}

impl std::fmt::Debug for DisplayPlane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayPlane")
            .field("map", &self.map)
            .finish_non_exhaustive()
    }
}

impl DisplayPlane {
    /// ★ Build the plane for a chip's display row and the guest driver `table`: the derived
    /// vocabulary, the caps page, the shared model, and the plane's GPU context with the store
    /// imported (`store_fd` exported from the host RM for this context).
    ///
    /// # Errors
    /// By name — `display=on` never starts on a guessed display.
    pub fn build(
        row: &'static kf_chip::display::DisplayRow,
        table: &kf_abi::versions::DriverAbiTable,
        bdf: &str,
        store_fd: i32,
        store_bytes: u64,
    ) -> Result<DisplayPlane, String> {
        let version = table.driver_version().to_string();
        let regs = Regs::for_ip(&version, row.ip_version).ok_or_else(|| {
            format!(
                "display=on: no derived display registers for driver {version} / IP {:#010x}",
                row.ip_version
            )
        })?;
        let t = kf_disp::class::for_version(&version).ok_or_else(|| {
            format!("display=on: no derived display classes for driver {version}")
        })?;
        let classes = kf_disp::model::Classes::of(row);
        let vocab = Vocab::resolve(t, &classes, &regs)
            .map_err(|e| format!("display=on: method vocabulary: {} is not derived", e.0))?;
        let caps = kf_disp::caps::page(t, &regs, row.classes.caps, row.heads, row.windows)
            .map_err(|e| format!("display=on: caps page: {}", e.0))?;
        let layout = Layout::from_regs(&regs)
            .ok_or("display=on: the instance-memory layout is not derived")?;
        let map = RegMap::resolve(&regs, t, row)?;
        let notifier_finished = t
            .notifier_field("__0_STATUS")
            .zip(t.notifier_value("__0_STATUS_FINISHED"))
            .map(|(fld, v)| kf_disp::class::put(0, fld, v))
            .ok_or("display=on: NV_DISP_NOTIFIER__0_STATUS is not derived")?;
        let model = kf_rm::display::model_for(table, row).ok_or_else(|| {
            format!("display=on: no derived display layouts for driver {version}")
        })?;
        let ports = model.ports.clone();
        let wake = Arc::new(Notifier::create().map_err(|e| format!("display eventfd: {e:?}"))?);
        let model: SharedDisplayModel = Arc::new(Mutex::new(model));
        {
            let w = wake.clone();
            model
                .lock()
                .map_err(|_| "display model poisoned")?
                .attach_plane(Waker(Arc::new(move || {
                    let _ = w.signal();
                })));
        }
        let mut gpu = DisplayGpu::bring_up_on(bdf)
            .map_err(|e| format!("display=on: the display plane's GPU context on {bdf}: {e}"))?;
        gpu.import_store(store_fd, store_bytes)
            .map_err(|e| format!("display=on: store import into the display context: {e}"))?;
        let engine = Engine::new(vocab, row.heads, row.windows);
        let scan = ScanVocab::resolve(t, classes.window);
        let formats = ScanFormats::resolve(t, classes.window);
        Ok(DisplayPlane {
            model,
            ports,
            map,
            caps,
            wake,
            init: Mutex::new(Some(WorkerInit {
                engine,
                gpu: Some(gpu),
                layout,
                notifier_finished,
            })),
            cursor: core::array::from_fn(|_| CursorPorts::default()),
            counters: DispCounters::default(),
            console: ConsoleShare::default(),
            scan,
            formats,
        })
    }

    /// The caps page's `(BAR0 offset, value)` words, and each cursor's `Free` (what the shadow holds
    /// from the start: static register values of the engine we present).
    pub fn static_words(&self) -> Vec<(u64, u32)> {
        let mut v: Vec<(u64, u32)> = self
            .caps
            .words
            .iter()
            .map(|(o, w)| (self.caps.base + u64::from(*o), *w))
            .collect();
        for h in 0..self.map.heads {
            // `Free` counts the PIO slots the FE has for methods: never full (NVKMS waits for non-zero)
            v.push((
                self.map.user_base(ChannelKind::Cursor, h) + self.map.cursor_free,
                4,
            ));
        }
        v
    }

    /// ★★ **THE vCPU PATH** for a write in the display aperture. Lock-free: atomics, shadow stores,
    /// at most one eventfd write. `store` stores a 32-bit shadow word; `plain` stores the guest's
    /// write at its own width. Returns `true` when the write ENABLED an event that is already pending
    /// — the caller then raises the display interrupt, as a level-triggered source would.
    #[must_use]
    pub fn trap_write(
        &self,
        off: u64,
        val: u64,
        width: u8,
        store: &dyn Fn(u64, u32),
        plain: &dyn Fn(u64, u64, u8),
    ) -> bool {
        self.counters.writes.fetch_add(1, Ordering::Relaxed);
        #[allow(clippy::cast_possible_truncation)]
        let v = val as u32;
        match self.map.classify(off) {
            DispWrite::Put(chn) => {
                store(off, v);
                if self.ports.post_put(chn, v) {
                    let _ = self.wake.signal();
                }
            }
            DispWrite::ReadOnly => {}
            DispWrite::Cursor(h, rel) => {
                plain(off, val, width);
                if let Some(c) = self.cursor.get(h as usize) {
                    let update = rel == self.map.cursor_update;
                    c.post(rel, v, update);
                    if update {
                        let _ = self.wake.signal();
                    }
                }
            }
            DispWrite::Event(r) => {
                self.ports.guest_write(r, v);
                self.publish_events(store);
                if let EventReg::HeadTimingEn(h) = r {
                    return self.ports.rm_head_timing(h) != 0;
                }
            }
            DispWrite::Plain => plain(off, val, width),
        }
        false
    }

    /// ★ Publish every event register and the ISR's derived summary registers — from any thread.
    /// ⊘ Repeats until the atomics are stable across a full publication, so a vCPU clearing a bit
    /// and the worker raising one can never leave a stale word behind (the last publisher sees the
    /// final state).
    pub fn publish_events(&self, store: &dyn Fn(u64, u32)) {
        let m = &self.map;
        let p = &self.ports;
        let heads = m.heads as usize;
        let snap = || -> Vec<u32> {
            let mut s = vec![
                p.event(EventReg::AwakenWin),
                p.event(EventReg::AwakenOther),
                p.event(EventReg::SemWin),
            ];
            for h in 0..heads {
                s.push(p.event(EventReg::HeadTiming(h)));
                s.push(p.event(EventReg::HeadTimingEn(h)));
            }
            s
        };
        for _ in 0..8 {
            let before = snap();
            store(m.evt_awaken_win, before[0]);
            store(m.evt_awaken_other, before[1]);
            if let Some(o) = m.evt_sem_win {
                store(o, before[2]);
            }
            for h in 0..heads {
                store(
                    m.evt_head_timing.0 + h as u64 * m.evt_head_timing.1,
                    before[3 + 2 * h],
                );
                if let Some((b, s)) = m.rm_intr_en_head_timing {
                    store(b + h as u64 * s, before[4 + 2 * h]);
                }
                store(
                    m.rm_intr_stat_head_timing.0 + h as u64 * m.rm_intr_stat_head_timing.1,
                    p.rm_head_timing(h),
                );
            }
            store(m.rm_intr_dispatch, p.rm_dispatch(heads));
            store(
                m.rm_ctrl_disp,
                p.rm_ctrl_disp(m.rm_ctrl_awaken_bit, m.rm_ctrl_win_sem_bit),
            );
            if let Some((o, b)) = m.evt_dispatch {
                store(o, if before[2] != 0 { b } else { 0 });
            }
            if snap() == before {
                return;
            }
        }
    }

    fn take_init(&self) -> Option<WorkerInit> {
        self.init.lock().ok()?.take()
    }
}

/// The worker's I/O side (everything the acquire callback and the effects need, apart from the
/// engine it runs beside).
struct Io<'d> {
    dev: &'d Device,
    dp: &'d DisplayPlane,
    gpu: Option<DisplayGpu>,
    layout: Layout,
    inst: Option<kf_disp::model::InstMem>,
    /// The instance-memory image, read (by the GPU) at most once per worker pass.
    img: Option<Vec<u8>>,
    notifier_finished: u32,
    refusals_logged: u32,
}

impl Io<'_> {
    fn image(&mut self) -> Result<&[u8], String> {
        if self.img.is_none() {
            let im = self
                .inst
                .ok_or("no display instance memory was stated (WRITE_INST_MEM)")?;
            let n = im.size.min(self.layout.inst_bytes());
            let mut buf = vec![0u8; usize::try_from(n).map_err(|_| "instance memory size")?];
            match im.addr_space {
                2 => self
                    .gpu
                    .as_ref()
                    .ok_or("no display GPU context")?
                    .read_store(im.phys, &mut buf)
                    .map_err(|e| format!("instance memory read: {e}"))?,
                1 => {
                    let b = self
                        .dev
                        .ram
                        .block_for(im.phys, n)
                        .ok_or("sysmem instance memory is not in guest RAM")?;
                    if !b.mem.read_into((im.phys - b.gpa) as usize, &mut buf) {
                        return Err("sysmem instance memory read refused".into());
                    }
                }
                s => return Err(format!("instance memory address space {s}")),
            }
            self.img = Some(buf);
        }
        Ok(self.img.as_deref().unwrap_or(&[]))
    }

    fn resolve(&mut self, client: u32, handle: u32, chn: u32) -> Result<CtxDma, String> {
        let layout = self.layout;
        let img = self.image()?;
        layout
            .resolve(img, client, handle, chn)
            .map_err(|m| format!("context DMA {handle:#x} on channel {chn}: {m:?}"))
    }

    fn read(&self, dma: CtxDma, off: u64, buf: &mut [u8]) -> Result<(), String> {
        let at = dma
            .span(off, buf.len() as u64)
            .ok_or_else(|| format!("[{off:#x}, +{:#x}) is past the context DMA", buf.len()))?;
        match dma.target {
            Target::Sysmem => {
                let b = self
                    .dev
                    .ram
                    .block_for(at, buf.len() as u64)
                    .ok_or_else(|| format!("{at:#x} is not guest RAM"))?;
                b.mem
                    .read_into((at - b.gpa) as usize, buf)
                    .then_some(())
                    .ok_or_else(|| "guest RAM read refused".to_string())
            }
            Target::Vidmem => self
                .gpu
                .as_ref()
                .ok_or("no display GPU context")?
                .read_store(at, buf)
                .map_err(|e| e.to_string()),
        }
    }

    fn write(&self, dma: CtxDma, off: u64, bytes: &[u8]) -> Result<(), String> {
        if !dma.writable {
            return Err("the context DMA is read-only".into());
        }
        let at = dma
            .span(off, bytes.len() as u64)
            .ok_or_else(|| format!("[{off:#x}, +{:#x}) is past the context DMA", bytes.len()))?;
        match dma.target {
            Target::Sysmem => {
                let b = self
                    .dev
                    .ram
                    .block_for(at, bytes.len() as u64)
                    .ok_or_else(|| format!("{at:#x} is not guest RAM"))?;
                b.mem
                    .write_from((at - b.gpa) as usize, bytes)
                    .then_some(())
                    .ok_or_else(|| "guest RAM write refused".to_string())
            }
            Target::Vidmem => self
                .gpu
                .as_ref()
                .ok_or("no display GPU context")?
                .write_store(at, bytes)
                .map_err(|e| e.to_string()),
        }
    }

    /// Does acquire `a` hold against guest memory now?
    fn acquired(&mut self, a: &Acquire) -> bool {
        let dma = match self.resolve(a.client, a.handle, a.chn) {
            Ok(d) => d,
            Err(e) => {
                self.refuse(&format!("acquire: {e}"));
                return false;
            }
        };
        let mut b = [0u8; 8];
        let n = if a.wide { 8 } else { 4 };
        if let Err(e) = self.read(dma, a.offset, &mut b[..n]) {
            self.refuse(&format!("acquire read: {e}"));
            return false;
        }
        a.satisfied_by(u64::from_le_bytes(b))
    }

    fn refuse(&mut self, why: &str) {
        self.dp.counters.refused.fetch_add(1, Ordering::Relaxed);
        if self.refusals_logged < 64 {
            self.refusals_logged += 1;
            eprintln!("kf3: display: REFUSED {why}");
        }
    }

    fn pushbuffer(&self, pb: PbLoc) -> Result<Vec<u8>, String> {
        let mut buf = vec![0u8; pb.bytes as usize];
        if pb.sysmem {
            let b = self
                .dev
                .ram
                .block_for(pb.addr, u64::from(pb.bytes))
                .ok_or_else(|| format!("pushbuffer {:#x} is not guest RAM", pb.addr))?;
            if !b.mem.read_into((pb.addr - b.gpa) as usize, &mut buf) {
                return Err("pushbuffer read refused".into());
            }
        } else {
            self.gpu
                .as_ref()
                .ok_or("no display GPU context")?
                .read_store(pb.addr, &mut buf)
                .map_err(|e| format!("pushbuffer read: {e}"))?;
        }
        Ok(buf)
    }
}

impl Device {
    /// ★★★ **The display worker** — its own thread; see the module docs. ⊘ Never a vCPU, never the
    /// drainer, and it holds the model's lock only to take statements.
    pub fn display_loop(&self) {
        let Some(dp) = self.display else { return };
        let Some(init) = dp.take_init() else { return };
        let WorkerInit {
            mut engine,
            gpu,
            layout,
            notifier_finished,
        } = init;
        if let Some(g) = &gpu
            && let Err(e) = g.make_current()
        {
            eprintln!(
                "kf3: display: the plane's GPU context cannot be made current: {e} — the display plane is DOWN"
            );
            return;
        }
        let Ok(poller) = Poller::create() else {
            eprintln!("kf3: display: epoll refused — the display plane is DOWN");
            return;
        };
        if poller.watch(dp.wake.as_source_fd(), 1).is_err() {
            eprintln!("kf3: display: epoll watch refused — the display plane is DOWN");
            return;
        }
        // ★ M2: the scanout copies' completion (`cuLaunchHostFunc` after each copy) wakes the worker
        if let Some(g) = &gpu
            && poller
                .watch(std::os::fd::AsFd::as_fd(g.completion_fd()), 2)
                .is_err()
        {
            eprintln!(
                "kf3: display: epoll watch of the scanout completion refused — the display plane is DOWN"
            );
            return;
        }
        let trace = std::env::var("KF3_DISPLAY_TRACE").is_ok_and(|v| v == "1");
        engine.trace = trace;
        eprintln!(
            "kf3: display worker up — engine {} heads / {} windows, caps page published",
            dp.map.heads, dp.map.windows
        );
        let store = |o: u64, v: u32| self.shadow_store(o, u64::from(v), 4);
        let mut io = Io {
            dev: self,
            dp,
            gpu,
            layout,
            inst: None,
            img: None,
            notifier_finished,
            refusals_logged: 0,
        };
        let mut next_vblank: [Option<(Instant, Duration)>; MAX_HEADS] = [None; MAX_HEADS];
        let mut scan = ScanState::default();
        let mut queue: VecDeque<Queued> = VecDeque::new();
        let mut cursor_seen = [0u32; MAX_HEADS];
        let mut published_get = [u32::MAX; kf_disp::ports::NUM_CHANNELS];
        let mut logged_updates = 0u32;
        while !self.stop.load(Ordering::Acquire) {
            // the deadline: the earliest vblank, a 2 ms acquire poll, or 50 ms
            let now = Instant::now();
            let mut deadline = now + Duration::from_millis(50);
            for (t, _) in next_vblank.iter().flatten() {
                deadline = deadline.min(*t);
            }
            if engine.acquire_pending() {
                deadline = deadline.min(now + Duration::from_millis(2));
            }
            if let Some(t) = scan.refresh_due(dp) {
                deadline = deadline.min(t);
            }
            let ms = u32::try_from(deadline.saturating_duration_since(now).as_millis())
                .unwrap_or(50)
                .max(1);
            let mut ready = ReadyTokens::new();
            let _ = poller.wait(&mut ready, PollTimeout::Millis(ms));
            let _ = dp.wake.drain();
            io.img = None;
            let mut effects: Vec<Effect> = Vec::new();
            let mut gets: Vec<(u32, u32, u32)> = Vec::new();
            // 1. statements (the model's lock, only to take them)
            let st = dp
                .model
                .lock()
                .map(|mut g| g.take_statements())
                .unwrap_or_default();
            for s in st {
                eprintln!("kf3: display: {s:?}");
                match s {
                    Statement::InstMem(im) => {
                        io.inst = Some(im);
                        // ⊘ The guest never zeroes instance memory (`disp_inst_mem.c:170-201`): a stale
                        // hash entry from an earlier driver life would resolve. Zeroed by the GPU.
                        if im.addr_space == 2
                            && let Some(g) = &io.gpu
                            && let Err(e) =
                                g.zero_store(im.phys, im.size.min(io.layout.inst_bytes()))
                        {
                            io.refuse(&format!("zeroing instance memory: {e}"));
                        }
                    }
                    Statement::ChannelAllocated {
                        kind,
                        instance,
                        offset,
                        client,
                        pb,
                        life,
                    } => {
                        // ⊘ bounded by the declared size, the context DMA's limit and the decoder's 4 KiB
                        let loc = pb.map(|p| {
                            let lim = u32::try_from(p.limit.saturating_add(1)).unwrap_or(u32::MAX);
                            PbLoc {
                                sysmem: p.addr_space == 1,
                                addr: p.phys,
                                bytes: p.bytes().min(lim).min(kf_disp::pushbuf::MAX_PUSHBUFFER)
                                    & !3,
                            }
                        });
                        if pb.is_some_and(|p| p.addr_space != 1 && p.addr_space != 2) {
                            io.refuse(&format!("{kind:?} {instance}: pushbuffer address space is neither sysmem nor vidmem"));
                        }
                        if let Some(chn) = engine.alloc(kind, instance, client, life, loc, offset) {
                            let base = dp.map.user_base(kind, instance);
                            store(base + dp.map.put, offset);
                            store(base + dp.map.get, offset);
                            published_get[chn as usize] = offset;
                            if kind == ChannelKind::Core {
                                // a new core life starts with an empty ARMED mirror
                                for o in (0..dp.map.core_len / 2).step_by(4) {
                                    store(dp.map.core_armed + o, 0);
                                }
                            }
                            self.display_chan_status(kind, instance, Some(true));
                        }
                    }
                    Statement::ChannelFreed { kind, instance } => {
                        engine.free(kind, instance);
                        self.display_chan_status(kind, instance, None);
                    }
                }
            }
            // 2. every DMA channel whose PUT moved
            for chn in 0..kf_disp::ports::NUM_CHANNELS as u32 {
                let Some((pb, decoded, life)) = engine.pushbuffer(chn) else {
                    continue;
                };
                let put = dp.ports.put(chn);
                if put == decoded || dp.ports.generation(chn) != life {
                    continue;
                }
                match io.pushbuffer(pb) {
                    Ok(bytes) => {
                        let s = engine.step(chn, &bytes, put, &mut |a| io.acquired(a));
                        if trace {
                            eprintln!(
                                "kf3: display: chn {chn} PUT {put:#x}: {} effects",
                                s.effects.len()
                            );
                        }
                        effects.extend(s.effects);
                        gets.extend(s.gets);
                    }
                    Err(e) => io.refuse(&format!("channel {chn}: {e}")),
                }
            }
            // 3. cursors: every posted Update
            for h in 0..dp.map.heads {
                let c = &dp.cursor[h as usize];
                let n = c.updates.load(Ordering::Acquire);
                if n == cursor_seen[h as usize] {
                    continue;
                }
                cursor_seen[h as usize] = n;
                for i in 0..c.offs.len() {
                    let o = c.offs[i].load(Ordering::Acquire);
                    if o != 0 {
                        let s = engine.cursor_write(
                            h,
                            o - 1,
                            c.regs[i].load(Ordering::Acquire),
                            &mut |a| io.acquired(a),
                        );
                        effects.extend(s.effects);
                    }
                }
                let s = engine.cursor_write(h, dp.map.cursor_update, 0, &mut |a| io.acquired(a));
                effects.extend(s.effects);
                gets.extend(s.gets);
            }
            // 4. vblanks whose time has come
            let now = Instant::now();
            let mut raised = false;
            for h in 0..dp.map.heads as usize {
                let Some((t, period)) = next_vblank[h] else {
                    continue;
                };
                if now < t {
                    continue;
                }
                let next = if now.duration_since(t) > period {
                    now + period
                } else {
                    t + period
                };
                next_vblank[h] = Some((next, period));
                dp.counters.vblanks.fetch_add(1, Ordering::Relaxed);
                let s = engine.vblank(h as u32, &mut |a| io.acquired(a));
                effects.extend(s.effects);
                gets.extend(s.gets);
                let f = dp.ports.frames[h]
                    .fetch_add(1, Ordering::AcqRel)
                    .wrapping_add(1);
                let (b, st, fld) = dp.map.rg_dpca;
                store(b + h as u64 * st, kf_disp::class::put(0, fld, f));
                if let Some((lb, ls)) = dp.map.loadv {
                    store(lb + h as u64 * ls, f);
                }
                dp.ports.raise(
                    EventReg::HeadTiming(h),
                    dp.map.head_last_data | dp.map.head_vblank,
                );
                raised |= dp.ports.rm_head_timing(h) != 0;
            }
            // 5. acquires waiting without a vblank
            if engine.acquire_pending() {
                let s = engine.poll_acquires(&mut |a| io.acquired(a));
                effects.extend(s.effects);
                gets.extend(s.gets);
            }
            // 6. ★ M2: the scanout. A flip of the window the console shows needs a copy of its NEW
            // surface, and every completion from that flip on waits for that copy to COMPLETE (the
            // flip-complete notifier and the release that frees the old surface, then GET): the
            // queue below keeps effect order and holds each item until its copy is done.
            let console = console_scanout(&engine, dp);
            scan.active = console.is_some();
            for e in effects {
                if let Effect::Latched { window } = &e
                    && console.is_some_and(|c| c.window == *window)
                {
                    scan.barrier = scan.started + 1;
                    scan.want = true;
                }
                queue.push_back(Queued {
                    need: scan.barrier,
                    item: Item::Effect(e),
                });
            }
            for (chn, life, get) in gets {
                queue.push_back(Queued {
                    need: scan.barrier,
                    item: Item::Get(chn, life, get),
                });
            }
            if io
                .gpu
                .as_ref()
                .is_some_and(|g| g.completion_fd().drain() > 0)
            {
                scan.completed(dp);
            }
            scan.give_up_if_stuck(dp);
            if console.is_some() && scan.refresh_due(dp).is_some_and(|t| t <= Instant::now()) {
                scan.want = true;
            }
            if scan.want && scan.inflight.is_none() {
                scan.start(&mut io, console);
            }
            // 7. completions, IN ORDER — each after the state it reports and the copy it follows
            while queue.front().is_some_and(|q| q.need <= scan.done) {
                let Some(Queued { item, .. }) = queue.pop_front() else {
                    break;
                };
                let e = match item {
                    Item::Effect(e) => e,
                    Item::Get(chn, life, get) => {
                        if published_get[chn as usize] == get && dp.ports.get(chn) == get {
                            continue;
                        }
                        if dp.ports.publish_get(chn, life, get) {
                            published_get[chn as usize] = get;
                            if let Some((kind, inst)) = chan_of(chn) {
                                store(dp.map.user_base(kind, inst) + dp.map.get, get);
                                self.display_chan_status(
                                    kind,
                                    inst,
                                    Some(engine_idle(&engine, dp, chn)),
                                );
                            }
                        }
                        continue;
                    }
                };
                match e {
                    Effect::CoreArmed(words) => {
                        for (m, v) in words {
                            store(dp.map.core_armed + u64::from(m), v);
                        }
                        // the display each head lights (`SYSTEM_GET_ACTIVE`), from the state just armed
                        for (h, sor) in engine.lit_sors().into_iter().enumerate() {
                            dp.ports.set_lit_sor(h, sor);
                        }
                    }
                    Effect::Notify {
                        chn,
                        client,
                        handle,
                        offset,
                        awaken,
                    } => {
                        let r = io.resolve(client, handle, chn).and_then(|dma| {
                            let ts = self.rm.gpu_time_ns().unwrap_or(0);
                            let mut n = [0u8; 16];
                            n[8..12].copy_from_slice(&(ts as u32).to_le_bytes());
                            n[12..16].copy_from_slice(&((ts >> 32) as u32).to_le_bytes());
                            // the timestamp words first, the status word (what the guest polls) last
                            io.write(dma, offset + 4, &n[4..16])?;
                            io.write(dma, offset, &io.notifier_finished.to_le_bytes())
                        });
                        if trace {
                            eprintln!(
                                "kf3: display: TRACE notify chn {chn} handle {handle:#x} +{offset:#x} awaken={awaken} -> {r:?}"
                            );
                        }
                        match r {
                            Ok(()) => {
                                dp.counters.notifies.fetch_add(1, Ordering::Relaxed);
                                if awaken {
                                    if chn == 0 {
                                        dp.ports
                                            .raise(EventReg::AwakenOther, dp.map.awaken_core_bit);
                                    } else if (1..=32).contains(&chn) {
                                        dp.ports.raise(EventReg::AwakenWin, 1 << (chn - 1));
                                    }
                                    raised = true;
                                }
                            }
                            Err(e) => io.refuse(&format!("notifier: {e}")),
                        }
                    }
                    Effect::Release {
                        chn,
                        client,
                        handle,
                        offset,
                        value,
                        wide,
                        awaken,
                    } => {
                        let r = io.resolve(client, handle, chn).and_then(|dma| {
                            let b = value.to_le_bytes();
                            io.write(dma, offset, if wide { &b[..] } else { &b[..4] })
                        });
                        if trace {
                            eprintln!(
                                "kf3: display: TRACE release chn {chn} handle {handle:#x} +{offset:#x} value {value:#x} -> {r:?}"
                            );
                        }
                        match r {
                            Ok(()) => {
                                dp.counters.releases.fetch_add(1, Ordering::Relaxed);
                                if awaken && (1..=32).contains(&chn) {
                                    dp.ports.raise(EventReg::SemWin, 1 << (chn - 1));
                                    raised = true;
                                }
                            }
                            Err(e) => io.refuse(&format!("semaphore release: {e}")),
                        }
                    }
                    Effect::Heads => {
                        for m in engine.heads_armed() {
                            let h = m.head as usize;
                            if h >= MAX_HEADS {
                                continue;
                            }
                            let active = m.period_ns > 0;
                            next_vblank[h] = match (active, next_vblank[h]) {
                                (false, _) => None,
                                (true, Some((t, p))) if p.as_nanos() == u128::from(m.period_ns) => {
                                    Some((t, p))
                                }
                                (true, _) => Some((
                                    Instant::now() + Duration::from_nanos(m.period_ns),
                                    Duration::from_nanos(m.period_ns),
                                )),
                            };
                            if let Some((b, s, fld, awake, sleep)) = dp.map.core_head_state {
                                store(
                                    b + m.head as u64 * s,
                                    kf_disp::class::put(0, fld, if active { awake } else { sleep }),
                                );
                            }
                            eprintln!(
                                "kf3: display: head {} {} raster {}x{} period {} us",
                                m.head,
                                if active { "ACTIVE" } else { "idle" },
                                m.raster.0,
                                m.raster.1,
                                m.period_ns / 1000
                            );
                        }
                    }
                    Effect::Latched { window } => {
                        if trace {
                            eprintln!("kf3: display: TRACE window {window} latched");
                        }
                    }
                    Effect::Trace(line) => eprintln!("kf3: display: TRACE {line}"),
                    Effect::Exception { chn, at, what } => {
                        dp.counters.exceptions.fetch_add(1, Ordering::Relaxed);
                        eprintln!("kf3: display: channel {chn} STOPPED at {at:#x}: {what}");
                    }
                }
            }
            if engine.updates > u64::from(logged_updates) && logged_updates < 64 {
                logged_updates = u32::try_from(engine.updates.min(64)).unwrap_or(64);
                eprintln!(
                    "kf3: display: {} updates completed, {} methods",
                    engine.updates, engine.methods
                );
            }
            dp.counters.updates.store(engine.updates, Ordering::Relaxed);
            dp.counters.methods.store(engine.methods, Ordering::Relaxed);
            // 8. the display interrupt — after the registers it announces
            if raised {
                dp.publish_events(&store);
                if dp.ports.anything_pending(dp.map.heads as usize) {
                    dp.counters.irqs.fetch_add(1, Ordering::Relaxed);
                    self.latch_and_deliver(kf_rm::authored::DISP_STALL_VECTOR);
                }
            }
        }
        // ⊘ QEMU's console may still point at a frame after the worker stops (its main loop refreshes
        // until it ends): the frames and their context stay mapped until the process exits.
        std::mem::forget(scan);
        std::mem::forget(io.gpu.take());
    }

    /// Publish a channel's CHNCTL allocation bit and CHNSTATUS state: `Some(idle)` allocated, `None`
    /// freed.
    fn display_chan_status(&self, kind: ChannelKind, instance: u32, state: Option<bool>) {
        let Some(dp) = self.display else { return };
        let m = &dp.map;
        let k = kind_index(kind);
        let (cb, cs) = m.chnctl[k];
        let (sb, ss) = m.chnstatus[k];
        let ctl_off = cb + u64::from(instance) * cs;
        let st_off = sb + u64::from(instance) * ss;
        let ctl = self.shadow_word(ctl_off);
        let fld = m.chnstatus_state[k];
        match state {
            Some(idle) => {
                self.shadow_store(ctl_off, u64::from(ctl | m.chnctl_alloc), 4);
                let v = if idle {
                    m.chnstatus_idle[k]
                } else {
                    m.chnstatus_busy[k]
                };
                self.shadow_store(st_off, u64::from(kf_disp::class::put(0, fld, v)), 4);
            }
            None => {
                self.shadow_store(ctl_off, u64::from(ctl & !m.chnctl_alloc), 4);
                self.shadow_store(st_off, 0, 4);
            }
        }
    }
}

/// `(kind, instance)` of a channel number.
fn chan_of(chn: u32) -> Option<(ChannelKind, u32)> {
    match chn {
        0 => Some((ChannelKind::Core, 0)),
        1..=32 => Some((ChannelKind::Window, chn - 1)),
        33..=64 => Some((ChannelKind::WindowImm, chn - 33)),
        73..=80 => Some((ChannelKind::Cursor, chn - 73)),
        _ => None,
    }
}

fn engine_idle(engine: &Engine, dp: &DisplayPlane, chn: u32) -> bool {
    !engine.waiting(chn) && dp.ports.idle(chn)
}

/// ★ M2: what the console shows — the lowest running head's scanned-out window, if any.
fn console_scanout(engine: &Engine, dp: &DisplayPlane) -> Option<Scanout> {
    let sv = dp.scan.as_ref()?;
    engine
        .heads_armed()
        .iter()
        .filter(|m| m.period_ns > 0)
        .find_map(|m| engine.scanout(sv, m.head))
}

/// An effect or a GET on the worker's completion queue.
enum Item {
    Effect(Effect),
    Get(u32, u32, u32),
}

/// A queued completion: `item`, held until scanout copy number `need` completed.
struct Queued {
    need: u64,
    item: Item,
}

/// How long a scanout copy may take before the flips behind it stop waiting for it.
const STUCK_COPY: Duration = Duration::from_secs(2);

/// A frame the console shows up to 1080p fits here; a larger mode grows the slot once, to the max.
const FRAME_SMALL: usize = 1920 * 1080 * 4;
/// The largest frame ([`kf_disp::scanout::MAX_PIXELS`] at 4 bytes).
const FRAME_MAX: usize = kf_disp::scanout::MAX_PIXELS as usize * 4;

/// ★ M2 — the worker's scanout copies (`V3_DISPLAY.md` §4.6). One copy in flight at a time, on the
/// plane's own stream; its completion is the `cuLaunchHostFunc` signal queued after it.
#[derive(Default)]
struct ScanState {
    /// Copies started and completed; a flip of the console window makes every completion after it
    /// wait for copy `started + 1` — the first one that starts after the flip latched.
    started: u64,
    done: u64,
    /// The copy number items queued now wait for.
    barrier: u64,
    /// A copy is wanted (a flip of the console window, or the refresh clock).
    want: bool,
    /// The console showed a surface at the last pass (the refresh clock runs only then).
    active: bool,
    /// The copy in flight: its number, slot, plan and start.
    inflight: Option<(u64, usize, CopyPlan, Instant)>,
    /// Page-locked frames per slot — grown, never freed while the device lives ([`ConsoleShare`]).
    frames: [Option<Frame>; SLOTS],
    retired: Vec<Frame>,
    /// When the last copy started.
    last: Option<Instant>,
    serial: u64,
    refusals_logged: u32,
}

impl ScanState {
    /// When the next refresh copy is due — front-buffer rendering (fbcon, an X server drawing into
    /// its scanout surface) changes pixels with no flip: 30 Hz while the console is watched, 4 Hz
    /// otherwise (a screendump still sees a recent frame). `None` while a copy is in flight or
    /// nothing is shown.
    fn refresh_due(&self, dp: &DisplayPlane) -> Option<Instant> {
        if !self.active || self.inflight.is_some() {
            return None;
        }
        let every = if dp.console.wanted_within(2000) {
            Duration::from_millis(33)
        } else {
            Duration::from_millis(250)
        };
        Some(self.last.map_or_else(Instant::now, |t| t + every))
    }

    /// The copy in flight completed: publish its frame to the console.
    fn completed(&mut self, dp: &DisplayPlane) {
        let Some((n, slot, plan, _)) = self.inflight.take() else {
            return;
        };
        self.done = n;
        if let Some(f) = &self.frames[slot] {
            self.serial += 1;
            dp.console.publish(
                slot,
                FrameView {
                    addr: f.addr(),
                    width: plan.width,
                    height: plan.height,
                    stride: u32::try_from(plan.row_bytes).unwrap_or(0),
                    format: plan.format as u32,
                    serial: self.serial,
                },
            );
            dp.counters.scanouts.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// ⊘ A copy whose completion never came (a CUDA fault loses the host signal): after
    /// [`STUCK_COPY`] the flips behind it complete anyway — a display that stops is worse than a
    /// console that misses a frame. The slot is not published.
    fn give_up_if_stuck(&mut self, dp: &DisplayPlane) {
        if let Some((n, _, _, t)) = self.inflight
            && t.elapsed() > STUCK_COPY
        {
            self.inflight = None;
            self.done = n;
            self.refuse(dp, &format!("copy {n} did not complete in {STUCK_COPY:?}"));
        }
    }

    fn refuse(&mut self, dp: &DisplayPlane, why: &str) {
        dp.counters.scanout_refused.fetch_add(1, Ordering::Relaxed);
        if self.refusals_logged < 16 {
            self.refusals_logged += 1;
            eprintln!("kf3: display: scanout REFUSED {why}");
        }
    }

    /// ★ Start the next copy of what the console shows. A copy that cannot be made (nothing shown,
    /// a surface refused by name) completes at once: the flip it follows still completes — the
    /// engine latched it; only the console keeps its previous frame.
    fn start(&mut self, io: &mut Io<'_>, console: Option<Scanout>) {
        self.want = false;
        self.last = Some(Instant::now());
        self.started += 1;
        let n = self.started;
        let dp = io.dp;
        let Some(so) = console else {
            self.done = n;
            return;
        };
        let dma = match io.resolve(so.client, so.handle, so.chn) {
            Ok(d) => d,
            Err(e) => {
                self.refuse(dp, &e);
                self.done = n;
                return;
            }
        };
        let plan = match kf_disp::scanout::plan(&so, &dma, &dp.formats) {
            Ok(p) => p,
            Err(r) => {
                self.refuse(dp, &r.0);
                self.done = n;
                return;
            }
        };
        let Some(gpu) = io.gpu.as_ref() else {
            self.done = n;
            return;
        };
        let slot = dp.console.free_slot();
        let need = usize::try_from(plan.frame_bytes()).unwrap_or(usize::MAX);
        if self.frames[slot].as_ref().is_none_or(|f| f.len() < need) {
            let cap = if need <= FRAME_SMALL {
                FRAME_SMALL
            } else {
                FRAME_MAX
            };
            match gpu.frame(cap) {
                Ok(f) => {
                    if let Some(old) = self.frames[slot].replace(f) {
                        self.retired.push(old);
                    }
                }
                Err(e) => {
                    self.refuse(dp, &format!("a {cap:#x}-byte console frame: {e}"));
                    self.done = n;
                    return;
                }
            }
        }
        let Some(frame) = self.frames[slot].as_ref() else {
            self.done = n;
            return;
        };
        let r = PitchRect {
            src: plan.src,
            src_pitch: plan.src_pitch,
            row_bytes: plan.row_bytes,
            rows: plan.rows,
            dst_pitch: plan.row_bytes,
        };
        match gpu.scanout_pitch(r, frame) {
            Ok(()) => self.inflight = Some((n, slot, plan, Instant::now())),
            Err(e) => {
                self.refuse(dp, &format!("the copy of {r:?}: {e}"));
                self.done = n;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> RegMap {
        let r = Regs::for_ip("580.159.04", 0x0401_0000).unwrap();
        let t = kf_disp::class::for_version("580.159.04").unwrap();
        RegMap::resolve(&r, t, &kf_chip::display::AMPERE).expect("GA10x register map")
    }

    /// ★ The vCPU decode on GA10x: each DMA channel's PUT, its read-only GET and ARMED half, the
    /// cursor PIO window and its read-only `Free`, the W1C event registers, the derived summaries.
    #[test]
    fn the_vcpu_decode_is_the_derived_register_file() {
        let m = map();
        assert!(
            m.owns(0x0068_0000)
                && m.owns(0x0061_1C30)
                && !m.owns(0x0070_0000)
                && !m.owns(0x0060_FFFC)
        );
        assert_eq!(m.classify(0x0068_0000), DispWrite::Put(0));
        assert_eq!(
            m.classify(0x0068_0004),
            DispWrite::ReadOnly,
            "GET is the engine's"
        );
        assert_eq!(
            m.classify(0x0068_8000 + 0x1000),
            DispWrite::ReadOnly,
            "the ARMED half"
        );
        assert_eq!(m.classify(0x0069_3000), DispWrite::Put(4), "window 3");
        assert_eq!(
            m.classify(0x0069_8000),
            DispWrite::Plain,
            "window 8 does not exist"
        );
        assert_eq!(
            m.classify(0x006B_7000),
            DispWrite::Put(33 + 7),
            "window-immediate 7"
        );
        assert_eq!(m.classify(0x006D_9208), DispWrite::Cursor(1, 0x208));
        assert_eq!(m.classify(0x006D_9008), DispWrite::ReadOnly, "cursor Free");
        assert_eq!(
            m.classify(0x0061_1858),
            DispWrite::Event(EventReg::AwakenWin)
        );
        assert_eq!(
            m.classify(0x0061_185C),
            DispWrite::Event(EventReg::AwakenOther)
        );
        assert_eq!(m.classify(0x0061_1868), DispWrite::Event(EventReg::SemWin));
        assert_eq!(
            m.classify(0x0061_1804),
            DispWrite::Event(EventReg::HeadTiming(1))
        );
        assert_eq!(
            m.classify(0x0061_1D8C),
            DispWrite::Event(EventReg::HeadTimingEn(3))
        );
        assert_eq!(m.classify(0x0061_1C30), DispWrite::ReadOnly);
        assert_eq!(m.classify(0x0061_1EC0), DispWrite::ReadOnly);
        assert_eq!(m.classify(0x0061_2078), DispWrite::Plain);
    }

    fn frame(addr: usize, serial: u64) -> FrameView {
        FrameView {
            addr,
            width: 1920,
            height: 1080,
            stride: 7680,
            format: 1,
            serial,
        }
    }

    /// ★ M2 triple buffering: the console only ever takes the newest READY frame; the worker's next
    /// target is never the one shown nor the one ready; an untaken frame is replaced, not queued.
    #[test]
    fn the_console_takes_the_newest_frame_and_the_gpu_never_writes_the_shown_one() {
        let c = ConsoleShare::default();
        assert_eq!(c.take(), None, "no frame before the first copy");
        let a = c.free_slot();
        c.publish(a, frame(0x1000, 1));
        let shown = c.take().unwrap();
        assert_eq!((shown.addr, shown.serial), (0x1000, 1));
        assert_eq!(
            c.take().unwrap().serial,
            1,
            "nothing new: the same front again"
        );
        // two copies complete before the console asks: the second replaces the first
        let b = c.free_slot();
        assert_ne!(b, a, "never the shown slot");
        c.publish(b, frame(0x2000, 2));
        let d = c.free_slot();
        assert!(d != a && d != b, "neither shown nor ready");
        c.publish(d, frame(0x3000, 3));
        let e = c.free_slot();
        assert_ne!(e, a, "the shown slot is still the console's");
        assert_ne!(e, d, "the ready slot is not a target");
        assert_eq!(
            c.take().unwrap().serial,
            3,
            "the newest, never the stale one"
        );
        for _ in 0..100 {
            let (front, ready) = unpack(c.state.load(Ordering::Acquire));
            let t = c.free_slot() as u32;
            assert!(t != front && t != ready);
            c.publish(t as usize, frame(0x4000, 4));
            if t % 2 == 0 {
                c.take();
            }
        }
    }
}
