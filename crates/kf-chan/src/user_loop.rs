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
    /// Wait until the private `GP_GET` reaches absolute fetch count `target` (a draining SYNC_WAIT
    /// gate's witness, or in-flight work whose progress frees the guest's ring). `carried` = the
    /// marker that ends it carries no interrupt (a packet was open).
    Private {
        /// Absolute private fetch count to reach.
        target: u64,
        /// The wake for it is the spin or the backstop, not an interrupt.
        carried: bool,
    },
    /// The private ring is full: any private progress frees a credit. `carried` = no marker could
    /// be published behind the work (a packet was open or no slot was left), so no interrupt will
    /// announce the progress.
    Credit {
        /// The wake for it is the spin or the backstop, not an interrupt.
        carried: bool,
    },
    /// A lock or admission was busy (`try_lock` failed, a retry was asked): nothing to wait on
    /// except looking again.
    Contended,
}

/// What a parked relay is waiting for ([`Need`] minus the cases that wait for nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Wait {
    /// See [`Need::Private`].
    Fetched {
        /// Absolute private fetch count to reach.
        target: u64,
        /// The wake for it is the spin or the backstop, not an interrupt.
        carried: bool,
    },
    /// See [`Need::Credit`].
    Credit {
        /// The wake for it is the spin or the backstop, not an interrupt.
        carried: bool,
    },
    /// See [`Need::Contended`]: satisfied by looking again.
    Contended,
}

/// The wait a turn's [`Need`] leaves behind (`None` for the cases that need no wake).
#[must_use]
pub const fn wait_of(need: Need) -> Option<Wait> {
    match need {
        Need::Idle | Need::Again => None,
        Need::Private { target, carried } => Some(Wait::Fetched { target, carried }),
        Need::Credit { carried } => Some(Wait::Credit { carried }),
        Need::Contended => Some(Wait::Contended),
    }
}

impl Wait {
    /// Whether the wait is over. `observed` = the absolute fetch count the private `GP_GET` word
    /// implies right now (`Fetch::peek`), `fetched` = what the relay has already accounted.
    #[must_use]
    pub const fn satisfied(self, observed: u64, fetched: u64) -> bool {
        match self {
            Self::Fetched { target, .. } => observed >= target,
            Self::Credit { .. } => observed > fetched,
            Self::Contended => true,
        }
    }
    /// Whether no interrupt will announce this wait.
    #[must_use]
    pub const fn carried(self) -> bool {
        matches!(
            self,
            Self::Fetched { carried: true, .. } | Self::Credit { carried: true }
        )
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
        Wait::Fetched { carried: true, .. } | Wait::Credit { carried: true } => Found::Carried,
        Wait::Contended => Found::Contended,
        Wait::Fetched { .. } | Wait::Credit { .. } => Found::Work,
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
    /// SYNC_WAIT gates authored.
    pub gates: AtomicU64,
    /// ... of them with a control witness (a packet was open).
    pub carried_gates: AtomicU64,
    /// Turns that ended waiting for private-ring credit.
    pub credit_waits: AtomicU64,
    /// Turns deferred by a busy lock / admission.
    pub contended: AtomicU64,
    /// Batch markers published.
    pub markers: AtomicU64,
    /// SYNC_WAIT drain latency (gate published -> witness fetched), log2 buckets of microseconds.
    pub drain: [AtomicU64; DRAIN_BUCKETS],
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
            "wakes(doorbell={} event={} spin={} backstop={}) backstop_found(work={} carried={} contended={} put_ahead={}) event_without_progress={} gates={} carried_gates={} credit_waits={} contended_turns={} markers={} drain_us_log2={drain:?}",
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
            g(&self.carried_gates),
            g(&self.credit_waits),
            g(&self.contended),
            g(&self.markers),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fetched_wait_is_over_at_its_target_and_not_before() {
        let w = Wait::Fetched {
            target: 7,
            carried: false,
        };
        assert!(!w.satisfied(6, 3));
        assert!(w.satisfied(7, 3));
        assert!(w.satisfied(9, 3));
    }

    #[test]
    fn a_credit_wait_is_over_at_any_private_progress() {
        let w = Wait::Credit { carried: false };
        assert!(!w.satisfied(5, 5));
        assert!(w.satisfied(6, 5));
    }

    #[test]
    fn a_contended_wait_is_over_by_looking_again() {
        assert!(Wait::Contended.satisfied(0, 100));
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
        assert_eq!(
            wait_of(Need::Credit { carried: false }),
            Some(Wait::Credit { carried: false })
        );
        assert_eq!(wait_of(Need::Contended), Some(Wait::Contended));
        assert_eq!(
            wait_of(Need::Private {
                target: 3,
                carried: true
            }),
            Some(Wait::Fetched {
                target: 3,
                carried: true
            })
        );
        assert!(
            Wait::Fetched {
                target: 1,
                carried: true
            }
            .carried()
        );
        assert!(!Wait::Credit { carried: false }.carried());
        assert!(Wait::Credit { carried: true }.carried());
    }

    /// The backstop's classification is the model's: only a satisfied wait an interrupt should have
    /// announced is a bug.
    #[test]
    fn only_an_announced_wait_found_by_the_backstop_is_work() {
        assert_eq!(
            found(Wait::Fetched {
                target: 1,
                carried: true
            }),
            Found::Carried
        );
        assert_eq!(
            found(Wait::Fetched {
                target: 1,
                carried: false
            }),
            Found::Work
        );
        assert_eq!(found(Wait::Credit { carried: false }), Found::Work);
        assert_eq!(found(Wait::Credit { carried: true }), Found::Carried);
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
