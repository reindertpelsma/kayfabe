// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **Lock-step frame requests and the cross-thread tick share** (`docs/design/V3_DISPLAY_PACING_20261017.md`
//! §11.4; owner decisions §AK and §AL, 2026-10-17).
//!
//! Two small pieces, both pure or lock-free, both used by the real relay and by the harness:
//!
//! - [`Requests`] — the relay's per-connection request book. A `WANT_FRAME` tick opens request `R`;
//!   a frame spends it; `NO_CHANGE` closes it. **At most one request is open**, a stale tick is
//!   dropped and counted, a frame is never sent without a request. Kayfabe ENFORCES the lock step: it
//!   never trusts the broker's timing.
//! - [`TickShare`] — what the relay (QEMU's main loop) tells the display worker, with no lock
//!   waited on: the LATEST tick through a single-writer seqlock, the newest unconsumed request in one
//!   atomic, and the worker's wake eventfd. A flood therefore leaves the worker one vblank per pass and
//!   at most one credit; the rest is counted.

use crate::wire::{TickPkt, seq_newer};
use kf_linux_raw::Notifier;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

/// ★ No tick for this long: the broker is SILENT; the free-running clock drives the guest's vblank
/// (`kf_disp::tick::BROKER_SILENCE_NS`, the same 50 ms) and the legacy frame credit sends frames.
pub const BROKER_SILENCE_MS: u64 = 50;
/// ★ The READ DEADLINE: a connection that holds a partial packet (fewer than 24 bytes) and receives
/// nothing for this long is dropped, loudly ("the broker stalled mid-packet"). A whole-packet-aligned
/// silence is the broker being quiet, which is the 50 ms fallback's business, not a fault.
pub const PARTIAL_READ_MS: u64 = 500;
/// ★ The most requests open at once.
pub const MAX_OPEN_REQUESTS: usize = 1;
/// ★ The most committed-and-unreleased frames (`slots::HELD_CAP`): the in-flight bound.
pub const MAX_FRAMES_IN_FLIGHT: u32 = crate::slots::HELD_CAP;

/// ★ What may be done with a ready frame (the relay's gate, R3/R6; pure so the harness drives the
/// same decision).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameGate {
    /// Send it (and spend the request, if one is open).
    Send,
    /// Ticks are live and no request is open: wait for one (counted).
    WaitRequest,
    /// Legacy rules and no credit: wait for `EV_FRAME` / `RELEASE` / the backstop.
    WaitCredit,
}

/// ★ The gate. Ticks negotiated and live: a frame goes only on an OPEN REQUEST; otherwise (no
/// ticks, or the broker is silent) the legacy credit decides — exactly as before phase 2.
#[must_use]
pub fn frame_gate(negotiated: bool, live: bool, open: Option<u32>, legacy_credit: bool) -> FrameGate {
    if negotiated && live {
        if open.is_some() {
            FrameGate::Send
        } else {
            FrameGate::WaitRequest
        }
    } else if legacy_credit {
        FrameGate::Send
    } else {
        FrameGate::WaitCredit
    }
}

/// ★ Ticks are LIVE: one arrived within [`BROKER_SILENCE_MS`] of `now` (inclusive: exactly 50 ms
/// is still live, 51 ms is silent).
#[must_use]
pub fn tick_live(last_rx_ms: Option<u64>, now: u64) -> bool {
    last_rx_ms.is_some_and(|r| now.saturating_sub(r) <= BROKER_SILENCE_MS)
}

/// Bounded counters of the tick path (`u64`, cumulative per relay).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickCounters {
    /// Ticks accepted from the wire.
    pub ticks: u64,
    /// Ticks dropped because their number was not newer than the last.
    pub ticks_stale: u64,
    /// Numbers skipped between accepted ticks (the broker coalesced them).
    pub ticks_missed: u64,
    /// Ticks whose `present_ns` went backwards.
    pub ts_backwards: u64,
    /// Accepted ticks that carried `WANT_FRAME`.
    pub want_ticks: u64,
    /// A request opened while another was still open (the older one is replaced).
    pub credits_coalesced: u64,
    /// Frames sent on a request (exactly one per request).
    pub frames_on_credit: u64,
    /// `NO_CHANGE` answers sent.
    pub no_change_sent: u64,
    /// Times a ready frame had to wait because no request was open.
    pub req_open_blocked: u64,
    /// Silence → the free clock took over (tick-live went false).
    pub silence_fallbacks: u64,
    /// Silence → ticks again (tick-live went true).
    pub silence_resumes: u64,
    /// Connections dropped by the partial-read deadline.
    pub partial_timeouts: u64,
    /// Reads that stopped at the per-turn budget with more waiting in the socket.
    pub budget_hits: u64,
}

/// What [`Requests::on_tick`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickVerdict {
    /// Not newer than the last tick: dropped, counted, NOT acted on.
    Stale,
    /// Accepted.
    Fresh {
        /// Numbers skipped since the previous accepted tick.
        missed: u32,
        /// The tick opened a request.
        opened: bool,
    },
}

/// ★ The relay's request book for one connection.
#[derive(Debug, Clone, Default)]
pub struct Requests {
    last_seq: Option<u32>,
    last_present: Option<u64>,
    open: Option<u32>,
    /// The last refresh period a tick carried, ns (0 unknown): the floor of how often an idle
    /// request is answered.
    refresh_ns: u32,
    /// When the last request was answered (a frame or `NO_CHANGE`), ms.
    last_answer_ms: Option<u64>,
    /// Accepted ticks since the open request opened (the request that is idle at a LATER tick gets
    /// `NO_CHANGE`).
    ticks_since_open: u32,
    /// The counters.
    pub c: TickCounters,
}

impl Requests {
    /// A new book (a new connection).
    #[must_use]
    pub fn new() -> Requests {
        Requests::default()
    }

    /// The open request, if any.
    #[must_use]
    pub fn open(&self) -> Option<u32> {
        self.open
    }

    /// The last accepted tick number.
    #[must_use]
    pub fn last_seq(&self) -> Option<u32> {
        self.last_seq
    }

    /// ★ A tick arrived (already framing-checked by [`Pkt::tick`]).
    pub fn on_tick(&mut self, t: &TickPkt) -> TickVerdict {
        if self.last_seq.is_some_and(|l| !seq_newer(t.tick_seq, l)) {
            self.c.ticks_stale += 1;
            return TickVerdict::Stale;
        }
        // numbers skipped between two ticks; the wrap past 0 is not a skipped tick (0 is never sent)
        let missed = self.last_seq.map_or(0, |l| {
            let m = t.tick_seq.wrapping_sub(l).saturating_sub(1);
            if t.tick_seq < l { m.saturating_sub(1) } else { m }
        });
        self.c.ticks_missed += u64::from(missed);
        self.last_seq = Some(t.tick_seq);
        if self.last_present.is_some_and(|p| t.present_ns < p) {
            self.c.ts_backwards += 1;
        }
        self.last_present = Some(self.last_present.map_or(t.present_ns, |p| p.max(t.present_ns)));
        self.c.ticks += 1;
        self.refresh_ns = t.refresh_ns;
        self.ticks_since_open = self.ticks_since_open.saturating_add(1);
        let mut opened = false;
        if t.want {
            self.c.want_ticks += 1;
            if self.open.is_some() {
                self.c.credits_coalesced += 1;
            }
            self.open = Some(t.tick_seq);
            self.ticks_since_open = 0;
            opened = true;
        }
        TickVerdict::Fresh { missed, opened }
    }

    /// ★ A frame is being sent at `now_ms`: spend the open request (`None`: no request, the frame
    /// must wait).
    pub fn spend(&mut self, now_ms: u64) -> Option<u32> {
        let r = self.open.take();
        if r.is_some() {
            self.c.frames_on_credit += 1;
            self.last_answer_ms = Some(now_ms);
        }
        r
    }

    /// A ready frame found no request open.
    pub fn blocked(&mut self) {
        self.c.req_open_blocked += 1;
    }

    /// The least time between two answers: half the broker's refresh period (the double-tick
    /// guard's width), at least 1 ms — a flood of ticks cannot make kayfabe answer faster than the
    /// display refreshes.
    #[must_use]
    pub fn answer_gap_ms(&self) -> u64 {
        let ns = if self.refresh_ns == 0 { 16_666_666 } else { u64::from(self.refresh_ns) };
        (ns / 2_000_000).max(1)
    }

    /// ★ The request to answer `NO_CHANGE`: open at a LATER tick than the one that opened it, with
    /// nothing ready, held or owed, and not sooner than [`Requests::answer_gap_ms`] after the last
    /// answer.
    #[must_use]
    pub fn idle_request(&self, nothing_to_send: bool, now_ms: u64) -> Option<u32> {
        self.open.filter(|_| {
            nothing_to_send
                && self.ticks_since_open >= 1
                && self
                    .last_answer_ms
                    .is_none_or(|t| now_ms >= t + self.answer_gap_ms())
        })
    }

    /// `NO_CHANGE` for the open request went out at `now_ms`.
    pub fn no_change_done(&mut self, now_ms: u64) {
        if self.open.take().is_some() {
            self.c.no_change_sent += 1;
            self.last_answer_ms = Some(now_ms);
        }
    }
}

/// One tick as the worker reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickSnap {
    /// Ticks published so far (the relay's; gaps between two snapshots are coalesced ticks).
    pub count: u64,
    /// The latest tick's number.
    pub tick_seq: u32,
    /// Its presentation time, ns.
    pub present_ns: u64,
    /// Its refresh period, ns (0 unknown).
    pub refresh_ns: u32,
    /// `present_ns` is the broker's own clock reading.
    pub clock_only: bool,
}

/// ★ The relay → display worker share. Lock-free: the relay is the single writer.
#[derive(Debug, Default)]
pub struct TickShare {
    /// Seqlock: odd while the writer is inside.
    lock: AtomicU64,
    present: AtomicU64,
    seq_refresh: AtomicU64,
    clock_only: AtomicBool,
    /// Ticks published.
    count: AtomicU64,
    /// The newest request (`tick_seq`) not yet taken by the worker; 0 = none.
    want: AtomicU32,
    /// Requests replaced before the worker took them.
    want_overwritten: AtomicU64,
    /// The VMM's vblank ceiling in Hz, told to the broker at opt-in (0 = unset: 75).
    cap_hz: AtomicU32,
    /// The worker's wake eventfd (one write per tick).
    wake: OnceLock<Arc<Notifier>>,
}

impl TickShare {
    /// A new share.
    #[must_use]
    pub fn new() -> TickShare {
        TickShare::default()
    }

    /// ★ **Worker** (once, at start): the eventfd the relay writes on every tick.
    pub fn set_wake(&self, wake: Arc<Notifier>) {
        let _ = self.wake.set(wake);
    }

    /// ★ **Worker** (once): the vblank ceiling to tell the broker (`display-max-fps`).
    pub fn set_cap_hz(&self, hz: u32) {
        self.cap_hz.store(hz, Ordering::Release);
    }

    /// The ceiling, 0 = unset.
    #[must_use]
    pub fn cap_hz(&self) -> u32 {
        self.cap_hz.load(Ordering::Acquire)
    }

    /// ★ **Relay**: publish an accepted, FRESH tick and wake the worker.
    pub fn publish(&self, t: &TickPkt) {
        let g = self.lock.load(Ordering::Relaxed);
        self.lock.store(g.wrapping_add(1), Ordering::Release); // odd: writing
        self.present.store(t.present_ns, Ordering::Relaxed);
        self.seq_refresh.store(
            u64::from(t.tick_seq) | (u64::from(t.refresh_ns) << 32),
            Ordering::Relaxed,
        );
        self.clock_only.store(t.clock_only, Ordering::Relaxed);
        self.lock.store(g.wrapping_add(2), Ordering::Release); // even: done
        if t.want && self.want.swap(t.tick_seq, Ordering::AcqRel) != 0 {
            self.want_overwritten.fetch_add(1, Ordering::Relaxed);
        }
        self.count.fetch_add(1, Ordering::AcqRel);
        if let Some(w) = self.wake.get() {
            let _ = w.signal();
        }
    }

    /// ★ **Relay**, on a closed connection: no request survives it.
    pub fn clear(&self) {
        self.want.store(0, Ordering::Release);
    }

    /// ★ **Worker**: the latest tick if one arrived since the caller last saw `seen`; the caller
    /// derives `count - seen - 1` coalesced ticks from the gap.
    #[must_use]
    pub fn snapshot(&self, seen: u64) -> Option<TickSnap> {
        for _ in 0..4 {
            let g0 = self.lock.load(Ordering::Acquire);
            if g0 & 1 == 1 {
                continue;
            }
            let count = self.count.load(Ordering::Acquire);
            let present_ns = self.present.load(Ordering::Relaxed);
            let sr = self.seq_refresh.load(Ordering::Relaxed);
            let clock_only = self.clock_only.load(Ordering::Relaxed);
            if self.lock.load(Ordering::Acquire) != g0 {
                continue;
            }
            return (count != seen && count != 0).then_some(TickSnap {
                count,
                tick_seq: sr as u32,
                present_ns,
                refresh_ns: (sr >> 32) as u32,
                clock_only,
            });
        }
        None
    }

    /// ★ **Worker**: take the newest unconsumed request (`tick_seq`), at most one.
    pub fn take_want(&self) -> Option<u32> {
        match self.want.swap(0, Ordering::AcqRel) {
            0 => None,
            r => Some(r),
        }
    }

    /// Requests replaced before the worker took them, cumulative.
    #[must_use]
    pub fn want_overwritten(&self) -> u64 {
        self.want_overwritten.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(seq: u32, want: bool, present: u64) -> TickPkt {
        TickPkt {
            tick_seq: seq,
            refresh_ns: 16_666_666,
            want,
            clock_only: false,
            present_ns: present,
        }
    }

    /// The whole decision table of the request book, one row per rule R1–R4.
    #[test]
    fn the_request_book_table() {
        let mut r = Requests::new();
        // R3: no request, no spend
        assert_eq!(r.spend(0), None);
        // a tick without WANT opens nothing
        assert_eq!(r.on_tick(&tick(1, false, 10)), TickVerdict::Fresh { missed: 0, opened: false });
        assert_eq!(r.open(), None);
        // R2: WANT opens request = its tick number
        assert_eq!(r.on_tick(&tick(2, true, 20)), TickVerdict::Fresh { missed: 0, opened: true });
        assert_eq!(r.open(), Some(2));
        // R1: a duplicate, an older one and the same number again are dropped and change nothing
        for stale in [2, 1, 0xffff_ffff] {
            assert_eq!(r.on_tick(&tick(stale, true, 30)), TickVerdict::Stale, "{stale}");
        }
        assert_eq!(r.open(), Some(2), "a stale WANT opens nothing");
        assert_eq!(r.c.ticks_stale, 3);
        // R2: a second WANT replaces the open one; ONE open request, counted
        assert_eq!(r.on_tick(&tick(5, true, 40)), TickVerdict::Fresh { missed: 2, opened: true });
        assert_eq!(r.open(), Some(5));
        assert_eq!((r.c.credits_coalesced, r.c.ticks_missed), (1, 2));
        // R3: a frame spends it, exactly once
        assert_eq!(r.spend(0), Some(5));
        assert_eq!(r.spend(0), None);
        assert_eq!(r.c.frames_on_credit, 1);
        // R4: NO_CHANGE only at a LATER tick, with nothing to send
        r.on_tick(&tick(6, true, 50));
        assert_eq!(r.idle_request(true, 100), None, "the tick that opened it is not 'later'");
        r.on_tick(&tick(7, false, 60));
        assert_eq!(r.idle_request(false, 100), None, "something ready: not idle");
        assert_eq!(r.idle_request(true, 100), Some(6));
        // ... but not sooner than half a refresh (8 ms) after the last answer (a frame at 0 ms)
        assert_eq!(r.answer_gap_ms(), 8);
        assert_eq!(r.idle_request(true, 7), None, "7 ms after the answer: too soon");
        assert_eq!(r.idle_request(true, 8), Some(6));
        r.no_change_done(100);
        assert_eq!((r.open(), r.c.no_change_sent), (None, 1));
        // a backwards timestamp is counted and the tick still counts
        assert_eq!(r.on_tick(&tick(8, false, 5)), TickVerdict::Fresh { missed: 0, opened: false });
        assert_eq!(r.c.ts_backwards, 1);
        // the wrap skips 0: 0xffff_ffff then 1
        let mut w = Requests::new();
        w.on_tick(&tick(u32::MAX, true, 1));
        assert_eq!(w.on_tick(&tick(1, true, 2)), TickVerdict::Fresh { missed: 0, opened: true });
    }

    /// The share: the latest tick, coalescing counted from the gap, the newest request kept, one
    /// request taken once.
    #[test]
    fn the_share_keeps_the_latest_tick_and_the_newest_request() {
        let s = TickShare::new();
        assert_eq!(s.snapshot(0), None);
        s.publish(&tick(1, true, 100));
        s.publish(&tick(2, false, 200));
        s.publish(&tick(3, false, 300));
        let a = s.snapshot(0).unwrap();
        assert_eq!((a.count, a.tick_seq, a.present_ns), (3, 3, 300));
        assert_eq!(a.count - 0 - 1, 2, "two ticks were coalesced into this one");
        assert_eq!(s.snapshot(3), None, "nothing new");
        assert_eq!(s.take_want(), Some(1), "the WANT of a coalesced tick is NOT lost");
        assert_eq!(s.take_want(), None, "taken once");
        s.publish(&tick(4, true, 400));
        s.publish(&tick(5, true, 500));
        assert_eq!(s.want_overwritten(), 1);
        assert_eq!(s.take_want(), Some(5), "the newest request wins");
        s.publish(&tick(6, true, 600));
        s.clear();
        assert_eq!(s.take_want(), None, "a closed connection leaves no request");
    }

    /// The frame gate, every input combination.
    #[test]
    fn the_frame_gate_table() {
        use FrameGate::*;
        for (neg, live, open, credit, want) in [
            (true, true, Some(3), false, Send),
            (true, true, Some(3), true, Send),
            (true, true, None, true, WaitRequest),
            (true, true, None, false, WaitRequest),
            (true, false, None, true, Send),
            (true, false, Some(3), false, WaitCredit),
            (true, false, None, false, WaitCredit),
            (false, true, None, true, Send),
            (false, false, Some(3), false, WaitCredit),
            (false, false, None, true, Send),
        ] {
            assert_eq!(frame_gate(neg, live, open, credit), want, "{neg} {live} {open:?} {credit}");
        }
        assert!(tick_live(Some(100), 150), "exactly 50 ms is still live");
        assert!(!tick_live(Some(100), 151));
        assert!(!tick_live(None, 0));
    }

    #[test]
    fn the_bounds_are_the_named_constants() {
        assert_eq!(MAX_OPEN_REQUESTS, 1);
        assert_eq!(MAX_FRAMES_IN_FLIGHT, 2);
        assert_eq!(BROKER_SILENCE_MS, 50);
    }
}
