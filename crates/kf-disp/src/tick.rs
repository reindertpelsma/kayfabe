//! ★ **The tick source** — who decides when a head's emulated vblank happens
//! (`docs/design/V3_DISPLAY_PACING_20261017.md` §4; owner decision (c) of 2026-10-17, `OWNER_RULINGS.md`
//! §AK).
//!
//! A head's vblank tick has one of two sources, never both at once:
//! - [`Source::Free`] — kayfabe's own free-running clock (`display-max-fps`'s capped period,
//!   [`crate::pace`]). It is the source of a VM with no broker (headless), the source from the first
//!   instant, and the FALLBACK whenever the broker goes quiet;
//! - [`Source::Broker`] — the host presenter's refresh events (phase 2: the broker protocol feeds
//!   [`TickSource::broker_tick`]; in phase 1 nothing does, so a [`TickSource`] never leaves
//!   [`Source::Free`] and ticks exactly as the clock always did).
//!
//! The guest-visible vblank is NEVER hard-gated by the broker: a hung, hidden, occluded or crashed
//! broker costs [`BROKER_SILENCE_NS`] and the free clock takes over, because Windows resets its display
//! driver after about 2 s without a flip completion and a flip completes at a tick.
//!
//! Pure and GPU-free; the transition table is tested exhaustively (`tests`).

/// ★ No broker tick event for this long: the free-running clock takes over (owner decision (c)).
/// The slowest tick the cap allows is 24 Hz (41.7 ms, [`crate::pace::MIN_HZ`]), so a live broker
/// always speaks inside it with 8 ms to spare.
pub const BROKER_SILENCE_NS: u64 = 50_000_000;

/// ★ The double-tick guard: a broker event that comes less than `GUARD_NUM / GUARD_DEN` of a period
/// after the previous tick (from EITHER source) is swallowed — it still proves the broker is alive and
/// makes it the source, but it is not a tick. One half: the free clock's own late ticks can already be
/// that close, so the guard only has to forbid what a handoff must never do — two vblanks in a row.
pub const GUARD_NUM: u64 = 1;
/// See [`GUARD_NUM`].
pub const GUARD_DEN: u64 = 2;

/// Who ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// kayfabe's free-running clock.
    Free,
    /// The broker's tick events.
    Broker,
}

/// A handoff between the sources (the worker logs each one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handoff {
    /// A broker event arrived while the free clock ticked: the broker is the source again.
    ToBroker,
    /// The broker was silent for [`BROKER_SILENCE_NS`]: the free clock took over.
    ToFree,
}

/// One emitted tick, as the guest sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Emitted {
    /// The head's vblank number: 1 for the first tick, +1 per tick, never repeated or skipped.
    pub seq: u64,
    /// The tick's timestamp, nanoseconds on the worker's clock — non-decreasing.
    pub stamp_ns: u64,
    /// How far past its scheduled time a free tick came (0 for a broker tick).
    pub late_ns: u64,
    /// Who produced it.
    pub from: Source,
}

/// ★ One head's tick source.
#[derive(Debug, Clone)]
pub struct TickSource {
    period_ns: u64,
    source: Source,
    /// Free: when the next free tick is due.
    at_ns: u64,
    /// The last tick emitted, by either source.
    last_tick_ns: Option<u64>,
    /// The last broker event, swallowed or not.
    last_broker_ns: Option<u64>,
    stamp_ns: u64,
    seq: u64,
    swallowed: u64,
    to_broker: u64,
    to_free: u64,
}

impl TickSource {
    /// A free-running source whose first tick is due one `period_ns` after `now_ns` (what arming a
    /// head always did).
    #[must_use]
    pub fn new(now_ns: u64, period_ns: u64) -> TickSource {
        TickSource {
            period_ns,
            source: Source::Free,
            at_ns: now_ns.saturating_add(period_ns),
            last_tick_ns: None,
            last_broker_ns: None,
            stamp_ns: 0,
            seq: 0,
            swallowed: 0,
            to_broker: 0,
            to_free: 0,
        }
    }

    /// The tick period.
    #[must_use]
    pub fn period_ns(&self) -> u64 {
        self.period_ns
    }

    /// Who ticks now.
    #[must_use]
    pub fn source(&self) -> Source {
        self.source
    }

    /// Ticks emitted so far — the head's vblank counter.
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Broker events swallowed by the double-tick guard.
    #[must_use]
    pub fn swallowed(&self) -> u64 {
        self.swallowed
    }

    /// Handoffs so far: `(to the broker, back to the free clock)`.
    #[must_use]
    pub fn handoffs(&self) -> (u64, u64) {
        (self.to_broker, self.to_free)
    }

    /// ★ When the worker must look at this source next: the free clock's next tick, or the moment the
    /// broker's silence runs out.
    #[must_use]
    pub fn deadline_ns(&self) -> u64 {
        match self.source {
            Source::Free => self.at_ns,
            Source::Broker => self
                .last_broker_ns
                .unwrap_or(0)
                .saturating_add(BROKER_SILENCE_NS),
        }
    }

    /// The same period again (a second `Heads` effect the cap clamps to the same tick): keep the
    /// phase and the source. A different period: the head is re-armed one period from `now_ns`, on the
    /// free clock — the broker's next event makes it the source again.
    pub fn retime(&mut self, now_ns: u64, period_ns: u64) -> bool {
        if period_ns == self.period_ns {
            return false;
        }
        self.period_ns = period_ns;
        self.source = Source::Free;
        self.at_ns = now_ns.saturating_add(period_ns);
        self.last_broker_ns = None;
        true
    }

    fn emit(&mut self, now_ns: u64, late_ns: u64, from: Source) -> Emitted {
        self.seq += 1;
        self.stamp_ns = self.stamp_ns.max(now_ns);
        self.last_tick_ns = Some(self.stamp_ns);
        Emitted {
            seq: self.seq,
            stamp_ns: self.stamp_ns,
            late_ns,
            from,
        }
    }

    /// The first grid point `last_broker + k * period` after `now_ns`: where a free clock that takes
    /// over from the broker resumes, so the guest's tick phase is the broker's.
    fn grid_after(&self, now_ns: u64) -> u64 {
        let base = self.last_broker_ns.unwrap_or(now_ns);
        let period = self.period_ns.max(1);
        let k = now_ns.saturating_sub(base) / period + 1;
        base.saturating_add(k.saturating_mul(period))
    }

    /// ★ Time has reached `now_ns`: the free clock's tick if one is due (rescheduled one period after
    /// its scheduled time, or — more than a period late — after `now_ns`: no catch-up burst, exactly as
    /// the clock always did), or the end of the broker's silence (a handoff, no tick yet: the free
    /// clock's first tick is the next grid point of the broker's phase).
    pub fn due(&mut self, now_ns: u64) -> (Option<Emitted>, Option<Handoff>) {
        match self.source {
            Source::Free => {
                if now_ns < self.at_ns {
                    return (None, None);
                }
                let late = now_ns - self.at_ns;
                self.at_ns = if late > self.period_ns {
                    now_ns.saturating_add(self.period_ns)
                } else {
                    self.at_ns.saturating_add(self.period_ns)
                };
                (Some(self.emit(now_ns, late, Source::Free)), None)
            }
            Source::Broker => {
                if now_ns < self.deadline_ns() {
                    return (None, None);
                }
                self.source = Source::Free;
                self.to_free += 1;
                self.at_ns = self.grid_after(now_ns);
                (None, Some(Handoff::ToFree))
            }
        }
    }

    /// ★ A broker tick event at `now_ns`. The broker becomes the source (a handoff if the free clock
    /// was); the event is a tick unless it comes closer than [`GUARD_NUM`]`/`[`GUARD_DEN`] of a period
    /// after the previous one, from either source. The free clock's pending deadline is dropped by the
    /// switch, so the next tick is the broker's.
    pub fn broker_tick(&mut self, now_ns: u64) -> (Option<Emitted>, Option<Handoff>) {
        self.last_broker_ns = Some(now_ns);
        let handoff = (self.source == Source::Free).then(|| {
            self.source = Source::Broker;
            self.to_broker += 1;
            Handoff::ToBroker
        });
        let guard = self.period_ns / GUARD_DEN * GUARD_NUM;
        if self
            .last_tick_ns
            .is_some_and(|t| now_ns.saturating_sub(t) < guard)
        {
            self.swallowed += 1;
            return (None, handoff);
        }
        (Some(self.emit(now_ns, 0, Source::Broker)), handoff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: u64 = 16_666_666;
    const GUARD: u64 = P / 2;

    /// A free source armed at 0: first tick at `P`.
    fn free() -> TickSource {
        TickSource::new(0, P)
    }

    /// A source the broker owns: one broker event at `t0` (a tick, since nothing ticked before).
    fn broker_at(t0: u64) -> TickSource {
        let mut s = free();
        let (e, h) = s.broker_tick(t0);
        assert_eq!(h, Some(Handoff::ToBroker));
        assert_eq!(e.map(|e| e.from), Some(Source::Broker));
        s
    }

    fn assert_ticked(e: Option<Emitted>, seq: u64, from: Source, late: u64) {
        let e = e.unwrap_or_else(|| panic!("a tick was expected (seq {seq})"));
        assert_eq!((e.seq, e.from, e.late_ns), (seq, from, late), "{e:?}");
    }

    /// ★ THE TRANSITION TABLE, exhaustively: every state of the source × every event, with the exact
    /// outcome — the tick (or none), the handoff (or none), the next state and the next deadline.
    /// States: Free before / at / far past its deadline; Broker live / at the silence's edge.
    /// Events: `due(now)`, `broker_tick(now)` (first / inside the guard / on its edge / past it),
    /// `retime` (same period / another). (Mutations: see the table in the design doc §7 — each
    /// edit of `due`, `broker_tick`, `retime`, the guard or the silence turns a row red.)
    #[test]
    fn the_transition_table() {
        // ---- Free × due
        let mut s = free();
        assert_eq!(s.due(P - 1), (None, None), "Free, before the deadline");
        assert_eq!(s.deadline_ns(), P);
        let (e, h) = s.due(P);
        assert_ticked(e, 1, Source::Free, 0);
        assert_eq!(
            (h, s.source(), s.deadline_ns()),
            (None, Source::Free, 2 * P)
        );
        let (e, _) = s.due(2 * P + 5);
        assert_ticked(e, 2, Source::Free, 5);
        assert_eq!(
            s.deadline_ns(),
            3 * P,
            "late by less than a period: on the grid"
        );
        let (e, _) = s.due(3 * P + P + 1);
        assert_ticked(e, 3, Source::Free, P + 1);
        assert_eq!(
            s.deadline_ns(),
            4 * P + 1 + P,
            "more than a period late: a period from now, no catch-up burst"
        );
        assert_eq!(s.due(4 * P + 1 + P - 1), (None, None));

        // ---- Free × broker_tick
        let mut s = free();
        let (e, h) = s.broker_tick(5_000_000);
        assert_ticked(e, 1, Source::Broker, 0);
        assert_eq!((h, s.source()), (Some(Handoff::ToBroker), Source::Broker));
        assert_eq!(
            s.deadline_ns(),
            5_000_000 + BROKER_SILENCE_NS,
            "the silence's deadline"
        );
        assert_eq!(s.handoffs(), (1, 0));
        // the free clock's tick, then a broker event inside the guard: swallowed, but the broker owns
        let mut s = free();
        assert_ticked(s.due(P).0, 1, Source::Free, 0);
        let (e, h) = s.broker_tick(P + GUARD - 1);
        assert_eq!(
            (e, h),
            (None, Some(Handoff::ToBroker)),
            "inside the guard: no double tick"
        );
        assert_eq!((s.source(), s.swallowed(), s.seq()), (Source::Broker, 1, 1));
        assert_eq!(
            s.due(2 * P),
            (None, None),
            "the free deadline was dropped by the switch"
        );
        // on the guard's edge: a tick
        let mut s = free();
        s.due(P);
        let (e, h) = s.broker_tick(P + GUARD);
        assert_ticked(e, 2, Source::Broker, 0);
        assert_eq!(h, Some(Handoff::ToBroker));

        // ---- Broker × broker_tick
        let mut s = broker_at(100_000_000);
        let (e, h) = s.broker_tick(100_000_000 + GUARD - 1);
        assert_eq!((e, h), (None, None), "inside the guard, no handoff");
        assert_eq!((s.swallowed(), s.seq()), (1, 1));
        assert_eq!(
            s.deadline_ns(),
            100_000_000 + GUARD - 1 + BROKER_SILENCE_NS,
            "a swallowed event still proves the broker alive"
        );
        let (e, h) = s.broker_tick(100_000_000 + GUARD - 1 + GUARD);
        assert_ticked(e, 2, Source::Broker, 0);
        assert_eq!(h, None);
        assert_eq!(s.handoffs(), (1, 0));

        // ---- Broker × due: live, the edge of the silence, past it
        let t0 = 100_000_000;
        let mut s = broker_at(t0);
        assert_eq!(
            s.due(t0 + 1),
            (None, None),
            "the free clock never ticks while the broker speaks"
        );
        let mut s = broker_at(t0);
        assert_eq!(
            s.due(t0 + BROKER_SILENCE_NS - 1),
            (None, None),
            "49.999999 ms: still the broker's"
        );
        assert_eq!(s.source(), Source::Broker);
        let (e, h) = s.due(t0 + BROKER_SILENCE_NS);
        assert_eq!(
            (e, h),
            (None, Some(Handoff::ToFree)),
            "50 ms: a handoff, no tick yet"
        );
        assert_eq!((s.source(), s.handoffs()), (Source::Free, (1, 1)));
        // the free clock resumes on the broker's phase grid, strictly after now, at most one period on
        let grid = t0 + 4 * P;
        assert_eq!(
            s.deadline_ns(),
            grid,
            "t0 + k*P, the first one after t0 + 50 ms"
        );
        assert!(grid > t0 + BROKER_SILENCE_NS && grid <= t0 + BROKER_SILENCE_NS + P);
        assert_eq!(s.due(grid - 1), (None, None));
        assert_ticked(s.due(grid).0, 2, Source::Free, 0);
        // a broker that speaks again takes the source back at once
        let (e, h) = s.broker_tick(grid + 9_000_000);
        assert_ticked(e, 3, Source::Broker, 0);
        assert_eq!((h, s.handoffs()), (Some(Handoff::ToBroker), (2, 1)));

        // ---- retime
        let mut s = free();
        assert!(!s.retime(7, P), "the same period again: nothing");
        assert_eq!(s.deadline_ns(), P, "phase kept");
        assert!(s.retime(1_000, 2 * P));
        assert_eq!((s.deadline_ns(), s.period_ns()), (1_000 + 2 * P, 2 * P));
        let mut s = broker_at(t0);
        assert!(!s.retime(t0 + 5, P));
        assert_eq!(
            s.source(),
            Source::Broker,
            "the same period keeps the source"
        );
        assert!(s.retime(t0 + 5, 2 * P));
        assert_eq!(
            (s.source(), s.deadline_ns()),
            (Source::Free, t0 + 5 + 2 * P),
            "a new period re-arms on the free clock"
        );
        assert_eq!(s.seq(), 1, "the vblank number goes on");
    }

    /// ★ Hostile order: the broker's events and the clock's deadlines interleave in every order the
    /// handoff can see — the broker event just before the free deadline, just after it, at the same
    /// instant, twice in a row, a free deadline during silence, the silence ending exactly as an event
    /// arrives. In every one: one tick source at a time, the vblank number +1 per tick, timestamps
    /// non-decreasing, no tick of a handoff closer than the guard to the one before it.
    #[test]
    fn hostile_event_orders_never_double_tick_or_go_backwards() {
        for off in [
            -3i64,
            -1,
            0,
            1,
            3,
            (GUARD as i64) - 1,
            GUARD as i64,
            (GUARD as i64) + 1,
        ] {
            for second in [0u64, 1, GUARD - 1, GUARD, P, 2 * P] {
                let mut s = free();
                let mut ticks: Vec<Emitted> = Vec::new();
                let mut check = |e: Option<Emitted>, s: &TickSource, ticks: &mut Vec<Emitted>| {
                    if let Some(e) = e {
                        if let Some(prev) = ticks.last() {
                            assert_eq!(e.seq, prev.seq + 1);
                            assert!(e.stamp_ns >= prev.stamp_ns);
                            if e.from == Source::Broker {
                                assert!(
                                    e.stamp_ns - prev.stamp_ns >= GUARD,
                                    "off {off} second {second}: {e:?} after {prev:?}"
                                );
                            }
                        }
                        assert_eq!(e.from == Source::Broker, s.source() == Source::Broker);
                        ticks.push(e);
                    }
                };
                // the free clock ticks once, then a broker event around its second deadline
                let (e, _) = s.due(P);
                check(e, &s, &mut ticks);
                let at = (2 * P as i64 + off) as u64;
                let (e, _) = s.due(at.saturating_sub(1).max(P));
                check(e, &s, &mut ticks);
                let (e, _) = s.broker_tick(at);
                check(e, &s, &mut ticks);
                let (e, _) = s.due(at); // the free deadline at the same instant
                check(e, &s, &mut ticks);
                let (e, _) = s.broker_tick(at + second);
                check(e, &s, &mut ticks);
                let (e, _) = s.due(at + second + 1);
                check(e, &s, &mut ticks);
                // the silence ends exactly as an event arrives
                let end = at + second + BROKER_SILENCE_NS;
                let (e, _) = s.broker_tick(end);
                check(e, &s, &mut ticks);
                let (e, _) = s.due(end);
                check(e, &s, &mut ticks);
                assert_eq!(
                    s.source(),
                    Source::Broker,
                    "the event after the silence took it back"
                );
            }
        }
    }

    /// ★ A fuzzed interleaving of events with clock readings that may go BACKWARDS (a hostile or
    /// skewed caller): no panic, the vblank number exactly +1 per emitted tick, the timestamps never
    /// decrease, and a tick of the broker's is never closer than the guard to the previous one.
    #[test]
    fn a_fuzzed_interleaving_keeps_the_counter_and_the_timestamps() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for case in 0..200 {
            let mut s = TickSource::new(next() % 1000, P);
            let (mut now, mut seq, mut stamp) = (0u64, 0u64, 0u64);
            for _ in 0..400 {
                let r = next();
                now = if r % 11 == 0 {
                    now.saturating_sub(r % 40_000_000) // back in time
                } else {
                    now + r % 30_000_000
                };
                let (e, _) = if r % 3 == 0 {
                    s.broker_tick(now)
                } else {
                    s.due(now)
                };
                if let Some(e) = e {
                    seq += 1;
                    assert_eq!(e.seq, seq, "case {case}");
                    assert!(e.stamp_ns >= stamp, "case {case}: the stamp went back");
                    if e.from == Source::Broker && stamp != 0 {
                        assert!(
                            e.stamp_ns - stamp >= GUARD,
                            "case {case}: a broker tick {} ns after the previous tick",
                            e.stamp_ns - stamp
                        );
                    }
                    stamp = e.stamp_ns;
                }
            }
        }
    }

    /// The constants are what the design says: 50 ms, one half, and the slowest legal tick (24 Hz)
    /// still speaks inside the silence with margin.
    #[test]
    fn the_constants() {
        assert_eq!(BROKER_SILENCE_NS, 50_000_000);
        assert_eq!((GUARD_NUM, GUARD_DEN), (1, 2));
        let slowest = 1_000_000_000 / u64::from(crate::pace::MIN_HZ);
        assert!(slowest + 8_000_000 <= BROKER_SILENCE_NS, "{slowest}");
    }
}
