//! ★ The display engine's lock-free state shared by three threads — the vCPU (BAR0 writes), the
//! register drainer (the physical-RM controls) and the display worker (the engine).
//!
//! `THE_CONSTRAINTS.md`: nothing blocks on a vCPU and nothing a vCPU takes is held by a slow path.
//! So everything a BAR0 write touches, and everything a control answer needs to be consistent with
//! it, lives here as atomics:
//!
//! - **per channel number** (`NV_PDISP_CHN_NUM_*`, core 0 … cursors 73..80): the PUT the guest
//!   posted (the vCPU stores it and wakes the worker), the GET the worker PUBLISHED (after every
//!   effect of the methods before it is visible), and an allocation generation the control link
//!   bumps on alloc/free so a worker never publishes a GET for a channel's previous life;
//! - **the event and interrupt registers** the guest's ISR reads and write-1-clears
//!   (`NV_PDISP_FE_EVT_STAT_*`, `kern_disp_0300.c:543-760`): the worker SETS bits, the vCPU CLEARS
//!   them (W1C, applied in the trap — a pure bit operation), and the registers the ISR reads first
//!   (`RM_INTR_STAT_CTRL_DISP`, `RM_INTR_DISPATCH`, `RM_INTR_STAT_HEAD_TIMING(i)`) are DERIVED from
//!   them on every publication, never stored separately (a second source of truth for one bit).
//!
//! ⊘ **Channel idle** (`NVC370_CTRL_CMD_GET_CHANNEL_INFO`) is `GET == PUT` on THESE words: a PUT the
//! guest wrote is visible to the control the instant the vCPU returns, and a GET is published only
//! after the worker's effects — so a guest that polls idle never reads IDLE before the notifier its
//! methods asked for, nor BUSY after the engine finished.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Channel numbers the display has (`NV_UDISP_FE_CHN_ASSY_BASEADR__SIZE_1`, `dev_disp.h` v03_00).
pub const NUM_CHANNELS: usize = 81;
/// The most heads any family's register file indexes (`NV_PDISP_*_HEAD_TIMING__SIZE_1`).
pub const MAX_HEADS: usize = 8;

/// One channel number's posted PUT, published GET and allocation generation.
#[derive(Debug, Default)]
pub struct ChanPort {
    put: AtomicU32,
    get: AtomicU32,
    /// Bumped by the control link on every alloc AND free of this channel number (odd = allocated).
    life: AtomicU32,
}

/// How many PUT writes the arrival log keeps (a power of two is not required).
pub const PUT_LOG: usize = 1024;

/// What the worker finds when it asks the arrival log for the next PUT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutPoll {
    /// Channel number and the PUT value the guest wrote, in the order the writes were made.
    Put(u32, u32),
    /// Nothing more was written.
    Empty,
    /// A vCPU reserved the next slot and has not stored its write yet; the worker is woken again by
    /// that vCPU's signal.
    Pending,
}

/// ★ **The arrival order of the guest's PUT writes.** The display fetches each DMA channel as its PUT
/// arrives, so the order in which channels' UPDATEs become pending is the order of the PUT writes; a
/// worker that wakes late and applies "every channel whose PUT moved" in channel-number order pairs
/// UPDATEs differently (`[measured]` the overlay stall H1: window 0's UPDATE joined an earlier group of
/// window 4's and left window 4's first flip without its partners). The vCPU's write is one `fetch_add`
/// and one store; the single worker reads in order. Bounded (hostile guest): the oldest entries are
/// overwritten and counted, and the worker then resynchronises from the latest PUT of every channel,
/// as before the log existed.
#[derive(Debug)]
pub struct PutLog {
    slots: Box<[AtomicU64]>,
    tail: AtomicU64,
    dropped: AtomicU64,
}

impl Default for PutLog {
    fn default() -> PutLog {
        PutLog {
            slots: (0..PUT_LOG).map(|_| AtomicU64::new(0)).collect(),
            tail: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }
}

impl PutLog {
    const TAG: u64 = 0xFF_FFFF;

    fn tag_of(index: u64) -> u64 {
        (index / PUT_LOG as u64 + 1) & Self::TAG
    }

    /// **vCPU**: append a PUT write (lock-free).
    pub fn push(&self, chn: u32, put: u32) {
        let i = self.tail.fetch_add(1, Ordering::AcqRel);
        let v = Self::tag_of(i) << 40 | u64::from(chn & 0xFF) << 32 | u64::from(put);
        self.slots[(i % PUT_LOG as u64) as usize].store(v, Ordering::Release);
    }

    /// **Worker**: the next write after `*head`, advancing it. Entries the vCPUs overwrote before the
    /// worker read them are skipped and counted in [`PutLog::dropped`].
    pub fn next(&self, head: &mut u64) -> PutPoll {
        loop {
            let tail = self.tail.load(Ordering::Acquire);
            if *head >= tail {
                return PutPoll::Empty;
            }
            if tail - *head > PUT_LOG as u64 {
                self.dropped
                    .fetch_add(tail - *head - PUT_LOG as u64, Ordering::Relaxed);
                *head = tail - PUT_LOG as u64;
            }
            let i = *head;
            let v = self.slots[(i % PUT_LOG as u64) as usize].load(Ordering::Acquire);
            let (tag, want) = (v >> 40, Self::tag_of(i));
            if tag == want {
                *head = i + 1;
                return PutPoll::Put(((v >> 32) & 0xFF) as u32, v as u32);
            }
            // a later lap already overwrote the slot: that write is lost to the log
            if tag.wrapping_sub(want) & Self::TAG != 0 && tag.wrapping_sub(want) & Self::TAG < 0x80_0000 {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                *head = i + 1;
                continue;
            }
            return PutPoll::Pending;
        }
    }

    /// How many writes the log lost (overwritten before the worker read them).
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// ★ A head's ARMED mode, as the worker publishes it for the physical-RM controls that report it
/// (`0x73011a`): the visible raster, the full raster with blanking, and the pixel clock. All three
/// are read from the core channel's armed words (`HEAD_SET_RASTER_SIZE`, `HEAD_SET_RASTER_BLANK_END`
/// / `_START`, `HEAD_SET_PIXEL_CLOCK_FREQUENCY`), never stored separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadTiming {
    /// Visible width and height (`BLANK_START - BLANK_END`, per axis).
    pub active: (u32, u32),
    /// Full raster width and height, blanking included (`HEAD_SET_RASTER_SIZE`).
    pub total: (u32, u32),
    /// The pixel clock in Hz (with the 1000/1001 adjust applied).
    pub pclk_hz: u64,
}

impl HeadTiming {
    /// Refresh rate in millihertz: pixel clock over the full raster. `None` for an empty raster.
    #[must_use]
    pub fn refresh_mhz(&self) -> Option<u64> {
        let tot = u64::from(self.total.0).checked_mul(u64::from(self.total.1))?;
        self.pclk_hz.checked_mul(1000)?.checked_div(tot)
    }
}

/// One head's published [`HeadTiming`]: a sequence lock over two words (one writer, the display
/// worker; readers never block and never see two modes mixed).
#[derive(Debug, Default)]
struct TimingSlot {
    seq: AtomicU32,
    /// `active.w | active.h << 16 | total.w << 32 | total.h << 48`; 0 = no mode.
    sizes: AtomicU64,
    pclk_hz: AtomicU64,
}

/// ★ The shared ports.
#[derive(Debug)]
pub struct Ports {
    chans: [ChanPort; NUM_CHANNELS],
    /// `NV_PDISP_FE_EVT_STAT_AWAKEN_WIN` — bit w: window channel w's WRITE_AWAKEN notifier (W1C).
    pub awaken_win: AtomicU32,
    /// `NV_PDISP_FE_EVT_STAT_AWAKEN_OTHER` — bit 0: the core channel (W1C).
    pub awaken_other: AtomicU32,
    /// `NV_PDISP_FE_EVT_STAT_SEM_WIN` — bit w: window w's WRITE_AWAKEN semaphore release (W1C).
    pub sem_win: AtomicU32,
    /// `NV_PDISP_FE_EVT_STAT_HEAD_TIMING(h)` — LAST_DATA / VBLANK / RG_LINE_* (W1C).
    pub head_timing: [AtomicU32; MAX_HEADS],
    /// `NV_PDISP_FE_RM_INTR_EN_HEAD_TIMING(h)` — which head-timing events reach RM (plain R/W).
    pub head_timing_en: [AtomicU32; MAX_HEADS],
    /// Per head: frames scanned out (`NV_PDISP_RG_DPCA(h)` FRM_CNT, and the LOADV counter).
    pub frames: [AtomicU32; MAX_HEADS],
    /// Per head: the SOR it lights, plus one (0 = none) — the worker publishes it from the ARMED core
    /// state, the model's `SYSTEM_GET_ACTIVE` answers from it.
    lit_sor: [AtomicU32; MAX_HEADS],
    /// Per head: the ARMED mode while the head's raster runs ([`HeadTiming`]).
    timing: [TimingSlot; MAX_HEADS],
    /// PUT writes posted by vCPUs (boot-log counter).
    pub puts_posted: AtomicU64,
    /// The PUT writes in arrival order (see [`PutLog`]).
    pub put_log: PutLog,
    /// W1C writes applied by vCPUs (boot-log counter).
    pub w1c_writes: AtomicU64,
}

impl Default for Ports {
    fn default() -> Ports {
        Ports {
            chans: core::array::from_fn(|_| ChanPort::default()),
            awaken_win: AtomicU32::new(0),
            awaken_other: AtomicU32::new(0),
            sem_win: AtomicU32::new(0),
            head_timing: core::array::from_fn(|_| AtomicU32::new(0)),
            head_timing_en: core::array::from_fn(|_| AtomicU32::new(0)),
            frames: core::array::from_fn(|_| AtomicU32::new(0)),
            lit_sor: core::array::from_fn(|_| AtomicU32::new(0)),
            timing: core::array::from_fn(|_| TimingSlot::default()),
            puts_posted: AtomicU64::new(0),
            put_log: PutLog::default(),
            w1c_writes: AtomicU64::new(0),
        }
    }
}

impl Ports {
    /// ★ **Worker**: publish the SOR head `h` lights (`None` = no display).
    pub fn set_lit_sor(&self, h: usize, sor: Option<u32>) {
        if let Some(a) = self.lit_sor.get(h) {
            a.store(sor.map_or(0, |s| s.saturating_add(1)), Ordering::Release);
        }
    }

    /// The SOR head `h` lights, if any (`SYSTEM_GET_ACTIVE`).
    #[must_use]
    pub fn lit_sor(&self, h: usize) -> Option<u32> {
        self.lit_sor
            .get(h)
            .map(|a| a.load(Ordering::Acquire))
            .filter(|v| *v != 0)
            .map(|v| v - 1)
    }

    /// ★ **Worker** (the only writer): publish head `h`'s armed mode (`None` = the raster is not
    /// running). A dimension past 16 bits (the hardware fields are 15 wide) publishes no mode.
    pub fn set_head_timing(&self, h: usize, t: Option<HeadTiming>) {
        let Some(slot) = self.timing.get(h) else {
            return;
        };
        let packed = t.and_then(|t| {
            let w = |v: u32| u16::try_from(v).ok().map(u64::from);
            let sizes =
                w(t.active.0)? | w(t.active.1)? << 16 | w(t.total.0)? << 32 | w(t.total.1)? << 48;
            (sizes != 0).then_some((sizes, t.pclk_hz))
        });
        let (sizes, pclk) = packed.unwrap_or((0, 0));
        // odd = being written
        slot.seq.fetch_add(1, Ordering::AcqRel);
        slot.sizes.store(sizes, Ordering::Release);
        slot.pclk_hz.store(pclk, Ordering::Release);
        slot.seq.fetch_add(1, Ordering::Release);
    }

    /// Head `h`'s armed mode, if its raster runs (`0x73011a`). Never blocks: a read that keeps
    /// colliding with a publication gives up and reports no mode.
    #[must_use]
    pub fn head_timing(&self, h: usize) -> Option<HeadTiming> {
        let slot = self.timing.get(h)?;
        for _ in 0..8 {
            let s1 = slot.seq.load(Ordering::Acquire);
            if s1 % 2 == 1 {
                core::hint::spin_loop();
                continue;
            }
            let sizes = slot.sizes.load(Ordering::Acquire);
            let pclk_hz = slot.pclk_hz.load(Ordering::Acquire);
            if slot.seq.load(Ordering::Acquire) != s1 {
                continue;
            }
            let f = |sh: u32| ((sizes >> sh) & 0xffff) as u32;
            return (sizes != 0).then(|| HeadTiming {
                active: (f(0), f(16)),
                total: (f(32), f(48)),
                pclk_hz,
            });
        }
        None
    }

    fn chan(&self, chn: u32) -> Option<&ChanPort> {
        self.chans.get(chn as usize)
    }

    /// ★ **vCPU**: the guest wrote PUT for channel number `chn`. Lock-free: one store. `false` for a
    /// channel number the display does not have (the write is then counted and dropped).
    pub fn post_put(&self, chn: u32, put: u32) -> bool {
        let Some(c) = self.chan(chn) else {
            return false;
        };
        c.put.store(put, Ordering::Release);
        self.put_log.push(chn, put);
        self.puts_posted.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// The PUT last posted.
    #[must_use]
    pub fn put(&self, chn: u32) -> u32 {
        self.chan(chn).map_or(0, |c| c.put.load(Ordering::Acquire))
    }

    /// The GET last published.
    #[must_use]
    pub fn get(&self, chn: u32) -> u32 {
        self.chan(chn).map_or(0, |c| c.get.load(Ordering::Acquire))
    }

    /// The allocation generation (odd = allocated).
    #[must_use]
    pub fn generation(&self, chn: u32) -> u32 {
        self.chan(chn).map_or(0, |c| c.life.load(Ordering::Acquire))
    }

    /// ★ **Control link**, on an accepted alloc: GET = PUT = `offset`, and a new (odd) generation.
    /// Returns the generation the worker must match before it publishes anything for this life.
    pub fn allocate(&self, chn: u32, offset: u32) -> Option<u32> {
        let c = self.chan(chn)?;
        c.put.store(offset, Ordering::Release);
        c.get.store(offset, Ordering::Release);
        // Only the control link (one thread: the register drainer) moves generations.
        let cur = c.life.load(Ordering::Acquire);
        let g = cur.wrapping_add(if cur % 2 == 0 { 1 } else { 2 });
        c.life.store(g, Ordering::Release);
        Some(g)
    }

    /// ★ **Control link**, on an accepted free: a new (even) generation — the worker's results for
    /// the freed life are discarded.
    pub fn release(&self, chn: u32) {
        if let Some(c) = self.chan(chn) {
            let g = c.life.load(Ordering::Acquire);
            if g % 2 == 1 {
                c.life.store(g.wrapping_add(1), Ordering::Release);
            }
        }
    }

    /// ★ **Worker**: publish GET for channel `chn`, IF the channel is still the life `life`. `false`
    /// when it was freed (or reborn) meanwhile — nothing is published then.
    pub fn publish_get(&self, chn: u32, life: u32, get: u32) -> bool {
        let Some(c) = self.chan(chn) else {
            return false;
        };
        if c.life.load(Ordering::Acquire) != life {
            return false;
        }
        c.get.store(get, Ordering::Release);
        true
    }

    /// Is channel `chn` idle — allocated, and everything posted consumed and published?
    #[must_use]
    pub fn idle(&self, chn: u32) -> bool {
        self.chan(chn)
            .is_some_and(|c| c.get.load(Ordering::Acquire) == c.put.load(Ordering::Acquire))
    }
}

/// Which W1C event register a BAR0 offset is (resolved by the plane from the derived registers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventReg {
    /// `EVT_STAT_AWAKEN_WIN`.
    AwakenWin,
    /// `EVT_STAT_AWAKEN_OTHER`.
    AwakenOther,
    /// `EVT_STAT_SEM_WIN`.
    SemWin,
    /// `EVT_STAT_HEAD_TIMING(h)`.
    HeadTiming(usize),
    /// `RM_INTR_EN_HEAD_TIMING(h)` — plain read/write, not W1C.
    HeadTimingEn(usize),
}

impl Ports {
    fn event_word(&self, r: EventReg) -> Option<&AtomicU32> {
        match r {
            EventReg::AwakenWin => Some(&self.awaken_win),
            EventReg::AwakenOther => Some(&self.awaken_other),
            EventReg::SemWin => Some(&self.sem_win),
            EventReg::HeadTiming(h) => self.head_timing.get(h),
            EventReg::HeadTimingEn(h) => self.head_timing_en.get(h),
        }
    }

    /// ★ **vCPU**: apply a guest write to an event register — write-1-to-clear, or a plain store for
    /// the enable. Lock-free. The caller republishes the derived registers.
    pub fn guest_write(&self, r: EventReg, v: u32) {
        let Some(w) = self.event_word(r) else { return };
        match r {
            EventReg::HeadTimingEn(_) => w.store(v, Ordering::Release),
            _ => {
                w.fetch_and(!v, Ordering::AcqRel);
            }
        }
        self.w1c_writes.fetch_add(1, Ordering::Relaxed);
    }

    /// The value the guest reads at an event register.
    #[must_use]
    pub fn event(&self, r: EventReg) -> u32 {
        self.event_word(r).map_or(0, |w| w.load(Ordering::Acquire))
    }

    /// ★ **Worker**: raise event bits.
    pub fn raise(&self, r: EventReg, bits: u32) {
        if let Some(w) = self.event_word(r) {
            w.fetch_or(bits, Ordering::AcqRel);
        }
    }

    /// `RM_INTR_STAT_HEAD_TIMING(h)`: the head's pending events that reach RM (enabled ones).
    #[must_use]
    pub fn rm_head_timing(&self, h: usize) -> u32 {
        match (self.head_timing.get(h), self.head_timing_en.get(h)) {
            (Some(s), Some(e)) => s.load(Ordering::Acquire) & e.load(Ordering::Acquire),
            _ => 0,
        }
    }

    /// `RM_INTR_DISPATCH`: bit h = head h has an RM head-timing event pending.
    #[must_use]
    pub fn rm_dispatch(&self, heads: usize) -> u32 {
        (0..heads.min(MAX_HEADS))
            .filter(|h| self.rm_head_timing(*h) != 0)
            .fold(0, |m, h| m | (1 << h))
    }

    /// `RM_INTR_STAT_CTRL_DISP`: AWAKEN (any window or core AWAKEN pending) and WIN_SEM, at the
    /// derived bit positions `awaken_bit` / `win_sem_bit` (`None` on a family without WinSem).
    #[must_use]
    pub fn rm_ctrl_disp(&self, awaken_bit: u8, win_sem_bit: Option<u8>) -> u32 {
        let mut v = 0;
        if self.awaken_win.load(Ordering::Acquire) != 0
            || self.awaken_other.load(Ordering::Acquire) != 0
        {
            v |= 1 << awaken_bit;
        }
        if let Some(b) = win_sem_bit
            && self.sem_win.load(Ordering::Acquire) != 0
        {
            v |= 1 << b;
        }
        v
    }

    /// Anything pending that the display interrupt announces?
    #[must_use]
    pub fn anything_pending(&self, heads: usize) -> bool {
        self.awaken_win.load(Ordering::Acquire) != 0
            || self.awaken_other.load(Ordering::Acquire) != 0
            || self.sem_win.load(Ordering::Acquire) != 0
            || self.rm_dispatch(heads) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ The worker reads the guests' PUT writes in the order they were made, not in channel-number order
    /// (the overlay stall H1), and a flood loses the oldest entries and says so.
    #[test]
    fn put_writes_are_read_in_arrival_order_and_a_flood_is_counted() {
        let p = Ports::default();
        let mut head = 0u64;
        assert_eq!(p.put_log.next(&mut head), PutPoll::Empty);
        for (chn, put) in [(5, 0x950), (1, 0xf20), (37, 0x20), (5, 0xa10), (1, 0xf20)] {
            assert!(p.post_put(chn, put));
        }
        let mut got = Vec::new();
        while let PutPoll::Put(c, v) = p.put_log.next(&mut head) {
            got.push((c, v));
        }
        assert_eq!(got, vec![(5, 0x950), (1, 0xf20), (37, 0x20), (5, 0xa10), (1, 0xf20)]);
        assert_eq!(p.put_log.dropped(), 0);
        // a flood past the log: only the newest PUT_LOG entries survive, the loss is counted
        for i in 0..(PUT_LOG as u32 + 10) {
            p.post_put(2, i * 4);
        }
        let mut n = 0u32;
        let mut first = None;
        while let PutPoll::Put(_, v) = p.put_log.next(&mut head) {
            first.get_or_insert(v);
            n += 1;
        }
        assert_eq!(n as usize, PUT_LOG);
        assert_eq!(first, Some(10 * 4));
        assert_eq!(p.put_log.dropped(), 10);
    }

    /// ★ A channel is idle exactly when its published GET equals its posted PUT; a free (or a
    /// rebirth) makes the worker's pending publication for the old life a no-op.
    #[test]
    fn idle_is_get_equals_put_and_generations_fence_old_lives() {
        let p = Ports::default();
        let g = p.allocate(0, 0).unwrap();
        assert_eq!(g % 2, 1, "allocated generations are odd");
        assert!(p.idle(0));
        assert!(p.post_put(0, 0x40));
        assert!(!p.idle(0), "a posted PUT is visible to the control at once");
        assert!(p.publish_get(0, g, 0x40));
        assert!(p.idle(0));
        p.post_put(0, 0x80);
        p.release(0);
        assert!(
            !p.publish_get(0, g, 0x80),
            "freed: the old life's GET is discarded"
        );
        let g2 = p.allocate(0, 0).unwrap();
        assert_ne!(g, g2);
        assert!(!p.publish_get(0, g, 0x10), "reborn: still discarded");
        assert!(p.publish_get(0, g2, 0));
        assert!(
            !p.post_put(NUM_CHANNELS as u32, 4),
            "a channel number past the file is refused"
        );
    }

    /// ★ The worker sets, the guest write-1-clears, and the ISR's first reads are derived.
    #[test]
    fn events_are_w1c_and_the_summary_registers_are_derived() {
        let p = Ports::default();
        p.raise(EventReg::AwakenWin, 1 << 3);
        assert_eq!(p.rm_ctrl_disp(8, Some(9)), 1 << 8);
        p.guest_write(EventReg::AwakenWin, 1 << 3);
        assert_eq!(p.event(EventReg::AwakenWin), 0);
        assert_eq!(p.rm_ctrl_disp(8, Some(9)), 0);
        p.raise(EventReg::HeadTiming(1), 0b10);
        assert_eq!(
            p.rm_dispatch(4),
            0,
            "a disabled head event does not reach RM"
        );
        p.guest_write(EventReg::HeadTimingEn(1), 0b10);
        assert_eq!(p.rm_dispatch(4), 0b10);
        assert_eq!(p.rm_head_timing(1), 0b10);
        p.guest_write(EventReg::HeadTiming(1), 0b10);
        assert!(!p.anything_pending(4));
        p.raise(EventReg::SemWin, 1);
        assert_eq!(p.rm_ctrl_disp(8, None), 0, "no WinSem bit on Turing");
        assert_eq!(p.rm_ctrl_disp(8, Some(9)), 1 << 9);
    }
}
