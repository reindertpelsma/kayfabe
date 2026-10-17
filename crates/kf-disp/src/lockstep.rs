//! ★★ **The worker's half of the lock step** — what kayfabe does with a broker credit
//! (`docs/design/V3_DISPLAY_PACING_20261017.md` §11.4; owner decision §AL, 2026-10-17: kayfabe
//! ENFORCES the lock step and never trusts the broker's timing).
//!
//! A broker tick may carry `WANT_FRAME` — a request, `R = tick_seq`. [`CreditGate`] decides WHEN the
//! display worker may act on it:
//!
//! - **G1** an event whose number is not newer than the last is dropped and counted;
//! - **G2** a credit whose tick the double-tick guard SWALLOWED is deferred to the guard's edge — never
//!   acted on early (the broker cannot make kayfabe copy faster than it can tick);
//! - **G3** credits COALESCE: at most one is pending in the gate and at most one in the
//!   [`crate::pace::Mailbox`]; the extras are counted and dropped, never queued;
//! - **G5** the gate decides nothing about FLIPS: an immediate flip latches and completes at once
//!   whether or not any credit exists (the credit gates the host's COPY only, `pace::flip_completion_need`
//!   unchanged).
//!
//! Pure; the display worker and the harness drive the same code.

use crate::tick::seq_newer;

/// ★ The most credits pending in the gate (and, after [`CreditGate::poll`], in the mailbox).
pub const MAX_PENDING_CREDITS: usize = 1;

/// Counters of the gate (bounded `u64`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GateCounters {
    /// Events whose number was not newer than the last: dropped.
    pub stale: u64,
    /// Requests whose number was not newer than the last request: dropped.
    pub stale_credits: u64,
    /// Numbers the worker never saw (the relay coalesced them).
    pub ticks_coalesced: u64,
    /// Credits that found one pending: replaced (the extra is dropped).
    pub credits_coalesced: u64,
    /// Credits deferred because their tick was inside the guard.
    pub credits_deferred: u64,
    /// Credits handed to the mailbox.
    pub granted: u64,
}

/// ★ One head's credit gate.
#[derive(Debug, Clone, Default)]
pub struct CreditGate {
    last_seq: Option<u32>,
    last_req: Option<u32>,
    /// `(request, usable at ns)`.
    pending: Option<(u32, u64)>,
    /// The counters.
    pub c: GateCounters,
}

impl CreditGate {
    /// A new gate.
    #[must_use]
    pub fn new() -> CreditGate {
        CreditGate::default()
    }

    /// Would an event numbered `tick_seq` be fresh (newer than the last)?
    #[must_use]
    pub fn fresh(&self, tick_seq: u32) -> bool {
        self.last_seq.is_none_or(|l| seq_newer(tick_seq, l))
    }

    /// The pending credit's request and when it becomes usable.
    #[must_use]
    pub fn pending(&self) -> Option<(u32, u64)> {
        self.pending
    }

    /// ★ A broker tick reached the worker. `tick_seq` is the latest tick's number, `coalesced` the
    /// ticks the relay published that the worker never saw, `want` the newest unconsumed request,
    /// `emitted` the vblank stamp when the tick passed the pacer's guard (`None`: swallowed),
    /// `last_tick_ns` the previous tick's stamp and `guard_ns` the guard.
    ///
    /// Returns whether the event was FRESH (not stale).
    pub fn on_tick(
        &mut self,
        tick_seq: u32,
        coalesced: u64,
        want: Option<u32>,
        emitted: Option<u64>,
        last_tick_ns: Option<u64>,
        guard_ns: u64,
    ) -> bool {
        if self.last_seq.is_some_and(|l| !seq_newer(tick_seq, l)) {
            self.c.stale += 1;
            if want.is_some() {
                self.c.stale_credits += 1;
            }
            return false;
        }
        self.last_seq = Some(tick_seq);
        self.c.ticks_coalesced += coalesced;
        let Some(req) = want else { return true };
        if self.last_req.is_some_and(|l| !seq_newer(req, l)) {
            self.c.stale_credits += 1;
            return true;
        }
        self.last_req = Some(req);
        if self.pending.is_some() {
            self.c.credits_coalesced += 1;
        }
        let at = match emitted {
            Some(stamp) => stamp,
            None => {
                self.c.credits_deferred += 1;
                last_tick_ns.map_or(0, |t| t.saturating_add(guard_ns))
            }
        };
        self.pending = Some((req, at));
        true
    }

    /// ★ The credit that is usable at `now_ns`, handed over ONCE (to the mailbox), or `None`.
    pub fn poll(&mut self, now_ns: u64) -> Option<u32> {
        let (req, at) = self.pending?;
        if now_ns < at {
            return None;
        }
        self.pending = None;
        self.c.granted += 1;
        Some(req)
    }

    /// When the pending credit becomes usable (the worker wakes for it).
    #[must_use]
    pub fn next_ns(&self) -> Option<u64> {
        self.pending.map(|(_, at)| at)
    }

    /// A guest life ended / the connection changed: nothing pends, numbers start over.
    pub fn reset(&mut self) {
        self.last_seq = None;
        self.last_req = None;
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: u64 = 8_000_000;

    /// ★ The decision table of the gate, one row per rule (G1–G3), each with its exact counters.
    #[test]
    fn the_gate_table() {
        let mut g = CreditGate::new();
        // a credit on an emitted tick is usable at once, and handed over once
        assert!(g.on_tick(1, 0, Some(1), Some(100), None, G));
        assert_eq!(g.poll(99), None, "not before its tick's stamp");
        assert_eq!(g.poll(100), Some(1));
        assert_eq!(g.poll(100), None, "handed over once");
        // G1: stale and duplicate events are dropped, with their credit
        assert!(!g.on_tick(1, 0, Some(1), Some(200), Some(100), G));
        assert!(!g.on_tick(0xffff_ffff, 0, Some(9), Some(200), Some(100), G));
        assert_eq!((g.c.stale, g.c.stale_credits), (2, 2));
        assert_eq!(g.pending(), None);
        // G2: a swallowed tick's credit waits for the guard's edge, not for the next tick
        assert!(g.on_tick(2, 0, Some(2), None, Some(1_000), G));
        assert_eq!(g.c.credits_deferred, 1);
        assert_eq!(g.poll(1_000 + G - 1), None, "inside the guard: not acted on");
        assert_eq!(g.next_ns(), Some(1_000 + G));
        assert_eq!(g.poll(1_000 + G), Some(2), "at the edge");
        // G3: a second credit while one pends replaces it: ONE pending, the extra counted
        assert!(g.on_tick(3, 0, Some(3), Some(5_000), Some(1_000), G));
        assert!(g.on_tick(4, 0, Some(4), Some(6_000), Some(5_000), G));
        assert_eq!(g.c.credits_coalesced, 1);
        assert_eq!(g.pending(), Some((4, 6_000)), "the newest request wins");
        // a request that is not newer than the last is dropped, the tick still counts
        assert!(g.on_tick(5, 0, Some(4), Some(7_000), Some(6_000), G));
        assert_eq!(g.c.stale_credits, 3);
        // coalesced ticks are counted from the gap
        assert!(g.on_tick(9, 3, None, Some(8_000), Some(7_000), G));
        assert_eq!(g.c.ticks_coalesced, 3);
        // reset forgets numbers (a new connection starts at 1 again)
        g.reset();
        assert!(g.on_tick(1, 0, Some(1), Some(1), None, G));
    }

    #[test]
    fn the_bound_is_one() {
        assert_eq!(MAX_PENDING_CREDITS, 1);
    }
}
