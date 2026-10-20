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

/// The worker park's LAST-RESORT net (owner, 2026-10-20): designed and tested as if infinite (`worker::set_park_net(false)`);
/// no logic depends on it; a timeout after which the loop finds work is a counted BUG (`backstop_found_work`).
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
    /// A lock or admission was busy (`try_lock` failed, a retry was asked). ★ 2026-10-20 (simple relay,
    /// `docs/design/V3_RELAY_SIMPLE_20261020.md`): NOT a wait. The relay's durable demand flag (`Cell::retry`, the
    /// "again" flag of the single-owner claim) is set again, which puts it in the worker's RETRY SET: every look re-rings
    /// it and the worker does not block while the set is non-empty (the lock holder is a running thread). No budget.
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
}

/// The wait a turn's [`Need`] leaves behind (`None` for the cases that need no wake).
#[must_use]
pub const fn wait_of(need: Need) -> Option<Wait> {
    match need {
        Need::Idle | Need::Again | Need::Contended => None,
        Need::Marker { target } => Some(Wait::Marker { target }),
        Need::Popped { target } => Some(Wait::Popped { target }),
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
        }
    }
    /// Whether no interrupt will announce this wait (only the spin and the safety net wake it).
    #[must_use]
    pub const fn carried(self) -> bool {
        matches!(self, Self::Popped { .. })
    }
}

/// Who looked at the relays' not-yet-fd state. ★ 2026-10-20 (simple relay): a COUNTER LABEL only. No decision depends on
/// it any more ([`look`] takes no reason): every look re-evaluates the same level state the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peek {
    /// The bounded spin before blocking.
    Spin,
    /// A host non-stall event (the batch marker's interrupt) or a completion fd woke the worker.
    Event,
}

/// Whether a look by `why` has any reason to touch a relay at all.
///
/// ★ The doorbell contract (owner, 2026-10-17): the last doorbell covers every `GP_PUT` write before
/// it, and nothing else announces `GP_PUT`. So a look is warranted by exactly two things: demand a
/// turn left behind (`Cell::retry`, a busy lock or an unfinished act) or a recorded wait. `why` is
/// deliberately NOT a reason: the backstop timer finding an idle relay must do nothing and read
/// nothing (no `GP_PUT` poll). The parameter exists so that a test can pin that for every [`Peek`].
#[must_use]
pub const fn look_needed(_why: Peek, demand: bool, waiting: bool) -> bool {
    demand || waiting
}

/// ★ 2026-10-20 (simple relay, `docs/design/V3_RELAY_SIMPLE_20261020.md` section 2): what one LOOK at one relay finds. The
/// inputs are LEVEL state read fresh by the look; interrupts, doorbells and timer wakes only decide WHEN a look happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LookIn {
    /// Durable demand (`Cell::retry`): a doorbell not yet served, or a turn that lost a lock (the retry set).
    pub demand: bool,
    /// The record lock is held elsewhere (the look cannot read the relay's wait).
    pub record_busy: bool,
    /// The recorded wait, if any. With `record_busy` the wait cannot be read: `Some(_)` (any value) stands for "the relay's
    /// waiting flag is set" -- a look that consumed an engine event must not leave a waiting relay unrung (stress seed 20).
    pub wait: Option<Wait>,
    /// The private words, read fresh: `(markers processed, entries popped)`; `None` = unreadable now (a lock busy, or a
    /// value the turn must refuse by name): the turn decides.
    pub words: Option<(u64, u64)>,
    /// Entries the relay has already accounted as popped (`Fetch::fetched`).
    pub fetched: u64,
    /// The guest's `GP_GET` follows the PBDMA's pops (the old semantics, [`crate::user_turn::GuestGet::Popped`]): new pops
    /// are then owed to the guest even while the wait goes on.
    pub get_follows_pops: bool,
}

/// What the look does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// Nothing to do.
    Leave,
    /// Ring the relay's token.
    Ring {
        /// The recorded wait is satisfied.
        clear_wait: bool,
        /// Only a pop write-back is owed (counted as `pop_writebacks`).
        pops_only: bool,
    },
}

/// ★ The ONE look decision (production `user_ring_peek_one` and every model call it). Level-triggered and budget-free: demand is
/// always re-rung (a ring is a hint: the turn re-checks everything and a lost lock re-sets the demand), a satisfied wait is
/// cleared and rung, a wait the look cannot read is rung (the turn reads it), new pops are rung when the guest's `GP_GET`
/// follows pops. An idle relay (no demand, no wait) is never touched: no `GP_PUT` is read by any look (doorbell contract).
#[must_use]
pub const fn look(i: LookIn) -> Look {
    if i.record_busy {
        return if i.demand || i.wait.is_some() {
            Look::Ring {
                clear_wait: false,
                pops_only: false,
            }
        } else {
            Look::Leave
        };
    }
    if let Some(w) = i.wait {
        let Some((marks, popped)) = i.words else {
            return Look::Ring {
                clear_wait: true,
                pops_only: false,
            };
        };
        if w.satisfied(marks, popped) {
            return Look::Ring {
                clear_wait: true,
                pops_only: false,
            };
        }
        if i.get_follows_pops && popped > i.fetched && !i.demand {
            return Look::Ring {
                clear_wait: false,
                pops_only: true,
            };
        }
    }
    if i.demand {
        Look::Ring {
            clear_wait: false,
            pops_only: false,
        }
    } else {
        Look::Leave
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
    /// Looks a BUSY worker made between passes (`worker::LoopPolicy`, bounded fairness): events dispatched and waiting relays peeked
    /// although every pass served something. Non-zero on a hot run is the fix doing its job; the starvation it prevents is silent.
    pub busy_looks: AtomicU64,
    /// ★ 2026-10-20 (simple relay): looks an otherwise idle worker made instead of blocking because the retry set was non-empty
    /// (demand left behind by a lost `try_lock`). Non-zero is normal under contention; it is CPU the worker spent waiting for a
    /// running lock holder (risk R3 of `docs/design/V3_RELAY_SIMPLE_20261020.md`).
    pub retry_looks: AtomicU64,
    /// Backstop peeks that found a satisfied wait an interrupt should have announced (a BUG).
    pub backstop_found_work: AtomicU64,
    /// Backstop peeks that found a satisfied control-witness gate (expected, apart).
    pub backstop_carried: AtomicU64,
    /// Backstop peeks that found a contended relay.
    pub backstop_contended: AtomicU64,
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
    /// ★ 2026-10-17: lazy observe's counters (`crate::user_lazy`), printed on their own line
    /// (`USER_RING_LAZY_OBSERVE SUMMARY`).
    pub lazy: crate::user_lazy::LazyStats,
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
            "wakes(doorbell={} event={} spin={} backstop={} busy_looks={} retry_looks={}) backstop_found(work={} carried={} contended={}) event_without_progress={} gates={} carried_ungated={} credit_waits={} contended_turns={} pop_writebacks={} markers={} drain_us_log2={drain:?}",
            g(&self.wake_doorbell),
            g(&self.wake_event),
            g(&self.wake_spin),
            g(&self.wake_backstop),
            g(&self.busy_looks),
            g(&self.retry_looks),
            g(&self.backstop_found_work),
            g(&self.backstop_carried),
            g(&self.backstop_contended),
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

    /// ★ Replaces `contended_relays_are_re_rung_by_the_spin_a_bounded_number_of_times` (the deleted retry budget): demand is
    /// re-rung by EVERY look, however many turns it lost, and a contended turn leaves no wait.
    #[test]
    fn demand_is_re_rung_by_every_look_without_a_budget() {
        let base = LookIn {
            demand: true,
            record_busy: false,
            wait: None,
            words: Some((0, 0)),
            fetched: 0,
            get_follows_pops: false,
        };
        for record_busy in [false, true] {
            assert!(matches!(
                look(LookIn {
                    record_busy,
                    ..base
                }),
                Look::Ring { .. }
            ));
        }
        assert_eq!(wait_of(Need::Contended), None);
        assert_eq!(
            look(LookIn {
                demand: false,
                ..base
            }),
            Look::Leave
        );
    }

    /// A satisfied wait is cleared and rung; an unsatisfied one with new pops is rung for the write-back only when the guest's
    /// `GP_GET` follows pops (F2); an unreadable wait is rung (the turn decides).
    #[test]
    fn a_wait_is_re_evaluated_from_the_level_words() {
        let w = LookIn {
            demand: false,
            record_busy: false,
            wait: Some(Wait::Popped { target: 4 }),
            words: Some((0, 2)),
            fetched: 1,
            get_follows_pops: false,
        };
        assert_eq!(look(w), Look::Leave);
        assert_eq!(
            look(LookIn {
                get_follows_pops: true,
                ..w
            }),
            Look::Ring {
                clear_wait: false,
                pops_only: true
            }
        );
        assert_eq!(
            look(LookIn {
                words: Some((0, 4)),
                ..w
            }),
            Look::Ring {
                clear_wait: true,
                pops_only: false
            }
        );
        assert_eq!(
            look(LookIn { words: None, ..w }),
            Look::Ring {
                clear_wait: true,
                pops_only: false
            }
        );
        assert!(matches!(
            look(LookIn {
                record_busy: true,
                ..w
            }),
            Look::Ring {
                clear_wait: false,
                ..
            }
        ));
    }

    #[test]
    fn needs_map_to_waits() {
        assert_eq!(wait_of(Need::Idle), None);
        assert_eq!(wait_of(Need::Again), None);
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

    /// ★ DOORBELL CONTRACT: no look, of any kind, has a reason to touch an idle relay -- in
    /// particular the backstop timer never reads `GP_PUT` for one.
    #[test]
    fn no_look_touches_a_relay_with_no_demand_and_no_wait() {
        for why in [Peek::Spin, Peek::Event] {
            assert!(
                !look_needed(why, false, false),
                "{why:?} must leave an idle relay alone"
            );
            assert!(
                look_needed(why, true, false),
                "{why:?}: demand left behind is a reason"
            );
            assert!(
                look_needed(why, false, true),
                "{why:?}: a recorded wait is a reason"
            );
            assert!(look_needed(why, true, true));
        }
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
