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
    /// PUT writes posted by vCPUs (boot-log counter).
    pub puts_posted: AtomicU64,
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
            puts_posted: AtomicU64::new(0),
            w1c_writes: AtomicU64::new(0),
        }
    }
}

impl Ports {
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
