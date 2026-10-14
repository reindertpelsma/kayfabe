//! ★ 2026-10-14 — **when a guest USER ring's new GP entries may be read** (`docs/design/
//! V3_USER_RING_SCAN_RACE.md`).
//!
//! The relay polls the guest's USERD `GP_PUT` (sysmem, write-back) and reads the pushbuffer each
//! entry names. Measured on Win11 + 580.88 (runs of 2026-10-14, `traces/user_ring_scan_20261014/`):
//! a pushbuffer in VRAM, written by the guest through its write-combined BAR1, can still read as
//! its OLD bytes for a while after `GP_PUT` already names it — whole 64-byte lines at the front of
//! the segment, the tail already new — and reads final ≤ 1 ms later. The guest rings the usermode
//! doorbell for only a few of its `GP_PUT` advances (10 of 472 new-work reads on one channel), so
//! the hardware it was written for learns of most of them later, on its own schedule. A relay that
//! scans the instant `GP_PUT` moves therefore scans bytes the GPU will never execute: it refused
//! legitimate streams (`Legacy` / `Header` / `SubDeviceMask` on stale words) or, worse, sized a
//! stale data word as a packet header and lost its place for every later segment.
//!
//! [`Settle`] is the rule: entries up to an observed `GP_PUT` become readable once either
//!  - a guest doorbell on this channel was counted AFTER that `GP_PUT` was read (the guest's own
//!    "published" signal; its trap drains the ringing core's write-combining buffers), or
//!  - [`SETTLE`] has passed since that `GP_PUT` was first read.
//!
//! [`Grace`] is the second half: a scan refusal of a segment that was never published is retried
//! (re-read, rescanned) for [`REFUSE_AFTER`] before it is final — the same bytes refusing for that
//! long are the guest's, not a read that was early.
//!
//! ⊘ Neither grants anything: they only DELAY a read. A refusal after the grace is exactly the
//! refusal it was before; hardware USER privilege still enforces isolation.

use std::time::{Duration, Instant};

/// How long an observed `GP_PUT` must have been visible before its entries are read without a
/// doorbell. Measured staleness ended within 1 ms in every re-read (refusals: +1 ms already final);
/// one tick of the worker's owed park (1 ms) is added on top by the cadence.
pub const SETTLE: Duration = Duration::from_millis(1);
/// How long the same unpublished segment must keep refusing before the refusal is final.
pub const REFUSE_AFTER: Duration = Duration::from_millis(50);
/// Observations kept while they age. More `GP_PUT` advances than this inside one [`SETTLE`] wait
/// for an older one to age (the next turn reads `GP_PUT` again) — never a merged, younger stamp.
pub const OBSERVATIONS: usize = 16;

/// Per-channel readability of GP entries ([`Settle::readable`]).
#[derive(Debug, Clone, Default)]
pub struct Settle {
    /// Ring index up to which (exclusive) entries may be read.
    readable: u32,
    /// `(GP_PUT, first read at, doorbells counted after that read)`, oldest first.
    pending: std::collections::VecDeque<(u32, Instant, u64)>,
}

impl Settle {
    /// A ring whose cursor is `cursor` and nothing beyond it is readable yet.
    pub fn new(cursor: u32) -> Self {
        Self {
            readable: cursor,
            pending: std::collections::VecDeque::new(),
        }
    }

    /// The ring was replaced (migration) or reset: start again at `cursor`.
    pub fn reset(&mut self, cursor: u32) {
        *self = Self::new(cursor);
    }

    /// One turn: `put` was read at `now`, and `doorbells` is the channel's doorbell count read
    /// AFTER that `put`. `cursor` is the next entry the relay would read. Returns the index up to
    /// which entries may be read now (equal to `cursor` when nothing may).
    pub fn observe(&mut self, cursor: u32, put: u32, now: Instant, doorbells: u64) -> u32 {
        if put == cursor {
            // Everything the guest published has been read; nothing older can matter.
            self.reset(cursor);
            return cursor;
        }
        let newest = self.pending.back().map_or(self.readable, |o| o.0);
        if put != newest && self.pending.len() < OBSERVATIONS {
            self.pending.push_back((put, now, doorbells));
        }
        // A doorbell counted after an observation covers it and every older one.
        while let Some(&(p, at, db)) = self.pending.front() {
            if doorbells > db || now.saturating_duration_since(at) >= SETTLE {
                self.readable = p;
                self.pending.pop_front();
            } else {
                break;
            }
        }
        self.readable
    }

    /// Whether observations are still ageing (the caller keeps its worker ticking).
    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// Bounded retry of a scan refusal ([`REFUSE_AFTER`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct Grace {
    first: Option<(u64, u32, Instant)>,
}

impl Grace {
    /// A scan of the unpublished entry `(generation, cursor)` refused at `now`: `true` = retry
    /// later (re-read and rescan), `false` = the refusal is final.
    pub fn refused(&mut self, generation: u64, cursor: u32, now: Instant) -> bool {
        match self.first {
            Some((g, c, at)) if g == generation && c == cursor => {
                now.saturating_duration_since(at) < REFUSE_AFTER
            }
            _ => {
                self.first = Some((generation, cursor, now));
                true
            }
        }
    }
    /// The entry scanned clean (or was published): forget the refusal.
    pub fn clear(&mut self) {
        self.first = None;
    }
    /// Whether a refusal is being retried.
    pub fn retrying(&self) -> bool {
        self.first.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(t: Instant, n: u64) -> Instant {
        t + Duration::from_micros(n * 1000)
    }

    /// The measured failure: the relay read GP_PUT the moment it moved and scanned stale words.
    /// FAILS on the old rule (read at once): nothing may be read before it settles or is rung.
    #[test]
    fn a_fresh_put_is_not_readable_until_it_settles() {
        let t = Instant::now();
        let mut s = Settle::new(10);
        assert_eq!(s.observe(10, 14, t, 0), 10, "read the instant PUT moved");
        assert!(s.waiting());
        assert_eq!(
            s.observe(10, 14, ms(t, 0) + Duration::from_micros(900), 0),
            10
        );
        assert_eq!(s.observe(10, 14, ms(t, 1), 0), 14);
        assert!(!s.waiting());
    }

    #[test]
    fn a_doorbell_after_the_read_publishes_at_once() {
        let t = Instant::now();
        let mut s = Settle::new(0);
        assert_eq!(s.observe(0, 3, t, 7), 0);
        // the same count again is not a new doorbell
        assert_eq!(s.observe(0, 3, t + Duration::from_micros(10), 7), 0);
        assert_eq!(s.observe(0, 3, t + Duration::from_micros(20), 8), 3);
    }

    /// A doorbell counted BEFORE the PUT was read never covers it (it may predate the PUT write).
    #[test]
    fn a_doorbell_before_the_read_covers_nothing() {
        let t = Instant::now();
        let mut s = Settle::new(0);
        assert_eq!(s.observe(0, 5, t, 9), 0);
        assert_eq!(s.observe(0, 5, t + Duration::from_micros(100), 9), 0);
    }

    /// A guest that keeps advancing PUT faster than SETTLE never starves the oldest observation:
    /// it ages on ITS stamp, and younger advances stay unreadable until theirs age.
    #[test]
    fn continuous_advances_age_on_their_own_stamps() {
        let t = Instant::now();
        let mut s = Settle::new(0);
        let mut readable = 0;
        for i in 0..40u32 {
            let now = t + Duration::from_micros(u64::from(i) * 250);
            readable = s.observe(readable.min(i), i + 1, now, 0);
            // never past the PUT observed at least SETTLE ago
            let aged = (i as i64) - 4; // 4 × 250 µs = 1 ms
            assert!(
                i64::from(readable) <= (aged + 1).max(0),
                "i={i} readable={readable}"
            );
        }
        assert!(readable >= 30);
    }

    #[test]
    fn caught_up_resets_and_wraps() {
        let t = Instant::now();
        let mut s = Settle::new(7);
        assert_eq!(s.observe(7, 1, t, 0), 7); // wrapped ring: PUT behind cursor numerically
        assert_eq!(s.observe(7, 1, ms(t, 2), 0), 1);
        assert_eq!(s.observe(1, 1, ms(t, 3), 0), 1);
        assert!(!s.waiting());
    }

    #[test]
    fn observations_are_bounded_and_never_merged_younger() {
        let t = Instant::now();
        let mut s = Settle::new(0);
        for i in 1..=(OBSERVATIONS as u32 + 8) {
            s.observe(0, i, t + Duration::from_micros(u64::from(i)), 0);
        }
        // the 17th+ advances were not recorded with an older stamp
        let r = s.observe(
            0,
            40,
            t + SETTLE + Duration::from_micros(OBSERVATIONS as u64),
            0,
        );
        assert_eq!(r, OBSERVATIONS as u32);
    }

    #[test]
    fn grace_retries_the_same_entry_then_is_final() {
        let t = Instant::now();
        let mut g = Grace::default();
        assert!(g.refused(1, 5, t));
        assert!(g.refused(1, 5, ms(t, 49)));
        assert!(!g.refused(1, 5, ms(t, 50)));
        // another entry, or another ring generation, starts its own grace
        assert!(g.refused(1, 6, ms(t, 60)));
        assert!(g.refused(2, 6, ms(t, 61)));
        g.clear();
        assert!(!g.retrying());
    }
}
