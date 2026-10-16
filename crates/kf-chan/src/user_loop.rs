//! ★ 2026-10-14 — **the USER-ring relay as an event loop** (`docs/design/V3_USER_RING_EVENT_LOOP_20261014.md`).
//!
//! The pure vocabulary the relay's turn (`crates/kf-qemu/src/userpump.rs`), the worker loop
//! ([`crate::worker`]) and the exhaustive model (`tests/user_loop_model.rs`) share. No I/O, no clock
//! in any decision.
//!
//! The owner's rule: correctness and performance never depend on timing. A turn that cannot go on
//! says WHAT it waits for ([`Need`] -> [`Wait`]) and the worker learns of it from an event: the
//! doorbell eventfd, the engine's non-stall event the batch marker raises, or (only where no event
//! can exist) a bounded spin and the [`BACKSTOP_MS`] safety net, whose every firing that finds work
//! is a counter ([`Stats`]). The doorbell contract stays narrow: a doorbell makes every `GP_PUT`
//! write before it visible and nothing more; `GP_PUT` ahead of the last doorbell is normal.

use std::sync::atomic::{AtomicU64, Ordering};

/// Bounded spin iterations (`epoll_wait(timeout=0)` + a peek of the not-yet-fd words) before the
/// worker blocks. A LATENCY optimisation only: the model proves liveness with 0.
pub const SPIN: u32 = 64;

/// How many CONSECUTIVE contended turns of one relay (a busy lock or admission nobody announces the
/// release of) the spin and event looks may re-ring before only the safety net does. Without this
/// bound a held lock would turn the spin into an endless busy loop: every look "satisfies" a
/// contended wait, so the budget would never run out.
pub const CONTENTION_RETRIES: u32 = SPIN;

/// The worker's longest sleep, and the ONLY timer of the relay: a safety net for a lost or
/// never-armed interrupt (and the control-witness gates that have none). 50 ms is the old park
/// tick, so the idle wake rate does not change (20/s), and it is far below the guest's 2 s timeout
/// detection. Every firing that finds work is counted ([`Stats`]).
pub const BACKSTOP_MS: u32 = 50;

/// Where a turn ended: what the relay needs next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Need {
    /// Nothing owed: `GP_PUT == cursor` and nothing in flight. Park until a doorbell.
    Idle,
    /// Progress was made: ring the token again (bounded by `REACT_ROUNDS`).
    Again,
    /// Wait until `target` reference-bearing markers were PROCESSED (the tagged `USERD.REFERENCE`
    /// count reaches it). A SYNC_WAIT gate's proof that the segment before the barrier -- its
    /// acquires included -- is done, and the wake for everything a batch marker announces (the
    /// guest's `GP_GET` write-back, the private ring's free slots: processing implies the pops).
    /// NOT `GP_GET`: measured on hardware, `GP_GET` runs ahead of processing. The marker raises the
    /// non-stall interrupt that announces it.
    Marker {
        /// Absolute count of reference-bearing markers to be processed.
        target: u64,
    },
    /// Wait until the PBDMA has POPPED `target` private entries (`GP_GET`), with NO marker behind
    /// the work (a packet was open, or no slot was left): no interrupt announces it, the spin and
    /// the safety net are the wake.
    Popped {
        /// Absolute count of private entries to be popped.
        target: u64,
    },
    /// A lock or admission was busy (`try_lock` failed, a retry was asked): nothing to wait on
    /// except looking again.
    Contended,
}

/// What a parked relay is waiting for ([`Need`] minus the cases that wait for nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Wait {
    /// See [`Need::Marker`].
    Marker {
        /// Absolute count of reference-bearing markers to be processed.
        target: u64,
    },
    /// See [`Need::Popped`].
    Popped {
        /// Absolute count of private entries to be popped.
        target: u64,
    },
    /// See [`Need::Contended`]: satisfied by looking again.
    Contended,
}

/// The wait a turn's [`Need`] leaves behind (`None` for the cases that need no wake).
#[must_use]
pub const fn wait_of(need: Need) -> Option<Wait> {
    match need {
        Need::Idle | Need::Again => None,
        Need::Marker { target } => Some(Wait::Marker { target }),
        Need::Popped { target } => Some(Wait::Popped { target }),
        Need::Contended => Some(Wait::Contended),
    }
}

impl Wait {
    /// Whether the wait is over. `marks` = the absolute count of gate/batch markers PROCESSED (the
    /// tagged `USERD.REFERENCE`, `Marks::peek`), `popped` = the absolute count of private entries
    /// the PBDMA has popped (`USERD.GP_GET`, `Fetch::peek`).
    #[must_use]
    pub const fn satisfied(self, marks: u64, popped: u64) -> bool {
        match self {
            Self::Marker { target } => marks >= target,
            Self::Popped { target } => popped >= target,
            Self::Contended => true,
        }
    }
    /// Whether no interrupt will announce this wait (only the spin and the safety net wake it).
    #[must_use]
    pub const fn carried(self) -> bool {
        matches!(self, Self::Popped { .. })
    }
}

/// Who looked at the relays' not-yet-fd state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peek {
    /// The bounded spin before blocking.
    Spin,
    /// A host non-stall event (the batch marker's interrupt) or a completion fd woke the worker.
    Event,
    /// The backstop timeout: the safety net.
    Backstop,
}

/// Whether a look by `why` may re-ring a relay that has been contended for `run` consecutive turns.
/// The safety net always may; the spin and event looks only while the retry budget lasts.
#[must_use]
pub const fn may_retry(run: u32, why: Peek) -> bool {
    matches!(why, Peek::Backstop) || run < CONTENTION_RETRIES
}

/// What a BACKSTOP peek that found a wait satisfied counts as (the model's `BACKSTOP_*` bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Found {
    /// A control-witness gate: no interrupt exists for it, expected, counted apart.
    Carried,
    /// A busy lock / admission: nothing announces its release, counted apart.
    Contended,
    /// A satisfied wait an interrupt should have announced: a lost or never-armed interrupt, a BUG.
    Work,
}

/// Classify a satisfied wait found by the backstop.
#[must_use]
pub const fn found(wait: Wait) -> Found {
    match wait {
        Wait::Popped { .. } => Found::Carried,
        Wait::Contended => Found::Contended,
        Wait::Marker { .. } => Found::Work,
    }
}

/// ★ PHASE 1b: whether a backstop wake of a Pointer-mode channel is LOUD: another one within a second of the
/// previous (`prev_ms`, `now_ms` are milliseconds on one monotone clock). The clock only decides how loudly a timer
/// wake is reported; nothing is decided by it.
#[must_use]
pub const fn loud_rate(prev_ms: Option<u64>, now_ms: u64) -> bool {
    match prev_ms {
        Some(p) => now_ms.saturating_sub(p) < 1000,
        None => false,
    }
}

/// What the worker does when no token is rung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Idle {
    /// Spin: poll the epoll set with timeout 0 and peek the waiting relays.
    Spin,
    /// Block (`try_park` -> `epoll_wait(BACKSTOP_MS)`).
    Park,
}

/// The worker's decision with nothing to serve: spin only while some relay waits for something
/// that only a look can find and the spin budget is not spent. Never a reason to skip the park.
#[must_use]
pub const fn idle(waiting: bool, spin_left: u32) -> Idle {
    if waiting && spin_left > 0 {
        Idle::Spin
    } else {
        Idle::Park
    }
}

/// log2 buckets of microseconds: bucket `i` holds `[2^i, 2^(i+1))` us (bucket 0 also 0 us), the
/// last one everything above. Bounded, lock-free.
pub const DRAIN_BUCKETS: usize = 16;

/// Production counters, all bounded (fixed atomics, no per-event allocation).
#[derive(Debug, Default)]
pub struct Stats {
    /// Parks that ended on the doorbell eventfd.
    pub wake_doorbell: AtomicU64,
    /// Parks that ended on an engine / completion event.
    pub wake_event: AtomicU64,
    /// Spin peeks that rang a relay.
    pub wake_spin: AtomicU64,
    /// Parks that ended on the backstop timeout.
    pub wake_backstop: AtomicU64,
    /// Backstop peeks that found a satisfied wait an interrupt should have announced (a BUG).
    pub backstop_found_work: AtomicU64,
    /// Backstop peeks that found a satisfied control-witness gate (expected, apart).
    pub backstop_carried: AtomicU64,
    /// Backstop peeks that found a contended relay.
    pub backstop_contended: AtomicU64,
    /// Backstop peeks that found `GP_PUT` ahead of the cursor with no doorbell (information).
    pub backstop_put_ahead: AtomicU64,
    /// Events after which no wait was satisfied (an interrupt before `GP_GET` was visible, or one
    /// of several markers).
    pub event_without_progress: AtomicU64,
    /// SYNC_WAIT gates authored (barrier + marker).
    pub gates: AtomicU64,
    /// SYNC_WAIT entries read WITHOUT a gate because a method packet was open across them (no
    /// witness can be placed inside an open packet; the hardware does not delay such a segment
    /// either, public `dev_pbdma.ref` `GP_ENTRY1_SYNC`).
    pub carried_ungated: AtomicU64,
    /// Turns that ended waiting for private-ring credit.
    pub credit_waits: AtomicU64,
    /// Turns deferred by a busy lock / admission.
    pub contended: AtomicU64,
    /// Looks that re-rang a relay only to write the guest's `GP_GET` back while a marker was still
    /// outstanding (pops carry no event; spin / safety net / unrelated events find them).
    pub pop_writebacks: AtomicU64,
    /// Batch markers published.
    pub markers: AtomicU64,
    /// SYNC_WAIT drain latency (gate published -> witness fetched), log2 buckets of microseconds.
    pub drain: [AtomicU64; DRAIN_BUCKETS],
    /// ★ 2026-10-16: the USER-ring lifecycle's counters (`crate::user_lifecycle`), printed on their own
    /// line (`USER_RING_LIFECYCLE SUMMARY`).
    pub life: crate::user_lifecycle::LifecycleStats,
}

impl Stats {
    /// Bump a counter.
    pub fn bump(c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }
    /// Record one drain latency.
    pub fn record_drain(&self, micros: u64) {
        let i = (u64::BITS - micros.max(1).leading_zeros() - 1) as usize;
        self.drain[i.min(DRAIN_BUCKETS - 1)].fetch_add(1, Ordering::Relaxed);
    }
    /// One bounded line for logs / the status report.
    #[must_use]
    pub fn summary(&self) -> String {
        let g = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let drain: Vec<u64> = self.drain.iter().map(g).collect();
        format!(
            "wakes(doorbell={} event={} spin={} backstop={}) backstop_found(work={} carried={} contended={} put_ahead={}) event_without_progress={} gates={} carried_ungated={} credit_waits={} contended_turns={} pop_writebacks={} markers={} drain_us_log2={drain:?}",
            g(&self.wake_doorbell),
            g(&self.wake_event),
            g(&self.wake_spin),
            g(&self.wake_backstop),
            g(&self.backstop_found_work),
            g(&self.backstop_carried),
            g(&self.backstop_contended),
            g(&self.backstop_put_ahead),
            g(&self.event_without_progress),
            g(&self.gates),
            g(&self.carried_ungated),
            g(&self.credit_waits),
            g(&self.contended),
            g(&self.pop_writebacks),
            g(&self.markers),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_wait_is_over_when_enough_markers_were_processed_whatever_get_says() {
        let w = Wait::Marker { target: 3 };
        // GP_GET runs ahead of processing: an advanced pop count proves nothing about the gate
        assert!(!w.satisfied(2, 1000));
        assert!(w.satisfied(3, 0));
        assert!(w.satisfied(4, 0));
    }

    #[test]
    fn a_popped_wait_is_over_at_its_target_pop_count() {
        let w = Wait::Popped { target: 7 };
        assert!(!w.satisfied(100, 6));
        assert!(w.satisfied(0, 7));
    }

    #[test]
    fn a_contended_wait_is_over_by_looking_again() {
        assert!(Wait::Contended.satisfied(0, 0));
    }

    #[test]
    fn contended_relays_are_re_rung_by_the_spin_a_bounded_number_of_times() {
        assert!(may_retry(0, Peek::Spin));
        assert!(may_retry(CONTENTION_RETRIES - 1, Peek::Event));
        assert!(!may_retry(CONTENTION_RETRIES, Peek::Spin));
        assert!(!may_retry(u32::MAX, Peek::Event));
        assert!(may_retry(u32::MAX, Peek::Backstop));
    }

    #[test]
    fn needs_map_to_waits() {
        assert_eq!(wait_of(Need::Idle), None);
        assert_eq!(wait_of(Need::Again), None);
        assert_eq!(wait_of(Need::Contended), Some(Wait::Contended));
        assert_eq!(
            wait_of(Need::Marker { target: 3 }),
            Some(Wait::Marker { target: 3 })
        );
        assert_eq!(
            wait_of(Need::Popped { target: 3 }),
            Some(Wait::Popped { target: 3 })
        );
        assert!(Wait::Popped { target: 1 }.carried());
        assert!(!Wait::Marker { target: 1 }.carried());
    }

    /// The backstop's classification is the model's: only a satisfied wait an interrupt should have
    /// announced is a bug.
    #[test]
    fn a_pointer_mode_backstop_wake_is_loud_only_beyond_about_one_a_second() {
        assert!(!loud_rate(None, 10));
        assert!(
            !loud_rate(Some(0), 1000),
            "exactly a second apart is not loud"
        );
        assert!(loud_rate(Some(0), 999));
        assert!(loud_rate(Some(500), 500), "two in the same millisecond");
        assert!(
            loud_rate(Some(5_000), 100),
            "a clock that went backwards saturates (loud), never panics"
        );
    }

    #[test]
    fn only_an_announced_wait_found_by_the_backstop_is_work() {
        assert_eq!(found(Wait::Popped { target: 1 }), Found::Carried);
        assert_eq!(found(Wait::Marker { target: 1 }), Found::Work);
        assert_eq!(found(Wait::Contended), Found::Contended);
    }

    /// The spin is bounded and never replaces the park.
    #[test]
    fn the_spin_is_bounded_and_needs_a_waiting_relay() {
        assert_eq!(idle(false, SPIN), Idle::Park);
        assert_eq!(idle(true, 0), Idle::Park);
        assert_eq!(idle(true, 1), Idle::Spin);
        let mut left = SPIN;
        let mut spins = 0;
        while idle(true, left) == Idle::Spin {
            left -= 1;
            spins += 1;
        }
        assert_eq!(spins, SPIN);
    }

    #[test]
    fn the_drain_histogram_is_log2_microseconds_and_bounded() {
        let s = Stats::default();
        for us in [0, 1, 2, 3, 4, 1000, u64::MAX] {
            s.record_drain(us);
        }
        let b: Vec<u64> = s.drain.iter().map(|c| c.load(Ordering::Relaxed)).collect();
        assert_eq!(b[0], 2); // 0 and 1 us
        assert_eq!(b[1], 2); // 2 and 3
        assert_eq!(b[2], 1); // 4
        assert_eq!(b[9], 1); // 1000
        assert_eq!(b[DRAIN_BUCKETS - 1], 1); // everything above
        assert!(s.summary().len() < 600);
    }
}
