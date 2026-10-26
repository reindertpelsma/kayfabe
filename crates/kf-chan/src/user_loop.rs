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

/// The worker's longest sleep, and the ONLY timer of the relay: a safety net for a lost or
/// never-armed interrupt (and the control-witness gates that have none). It looks only at relays
/// that WAIT for something or left demand behind; it never reads `GP_PUT` (see [`look_needed`]). 50 ms is the old park
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

/// ★ 2026-10-20 (owner design): what a relay can hold that the worker must not sleep through, and what will announce it. A relay that lost a
/// `try_lock` (a busy `pt` map, USERD / RM-graph admission, source swap) is NOT re-rung by any look, on a budget or otherwise: that would be a
/// poll. It registers with the lock holder ([`crate::handoff::Handoff`]) BEFORE giving up and the holder rings it after releasing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Obligation {
    /// A draining marker: the batch marker's engine interrupt announces it.
    Marker,
    /// A carried wait (`Wait::Popped`): pops raise nothing.
    Popped,
    /// An interrupt came and the private words did not show the progress yet (visibility lag): the hint is spent, the state is not final.
    Lagging,
    /// A lost `try_lock`: registered with the lock holder before giving up.
    Contended,
}

/// What announces an [`Obligation`] to a worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WakeSource {
    /// The host engine's non-stall event fd, in every worker's poller (level-triggered, no EPOLLEXCLUSIVE).
    EngineEvent,
    /// The lock holder: it rings the relay's token after releasing (`crate::handoff`), which signals the worker eventfd.
    LockHolder,
}

/// One row of the sleep-safety table (owner, 2026-10-20): never block while an obligation could change in a way that needs handling and
/// no NON-MISSABLE wake source covers that change. Wake moments are a SUPERSET of the moments processing is needed; an eventfd is never
/// ground truth (every wake re-reads the state) but it may not MISS (it is armed before the state is evaluated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cover {
    /// The change that needs handling.
    pub change: &'static str,
    /// The wake source that covers it, if any.
    pub source: Option<WakeSource>,
    /// Can that wake source miss the change?
    pub can_miss: bool,
}

/// The table: per obligation, the change event, the covering wake source, whether it can miss; where there is no non-missable source the
/// worker does not block (it keeps evaluating inside its one loop).
#[must_use]
pub const fn cover(o: Obligation) -> Cover {
    match o {
        Obligation::Marker => Cover {
            change: "the private REFERENCE reaches the marker count (the marker's NON_STALL interrupt)",
            source: Some(WakeSource::EngineEvent),
            can_miss: false,
        },
        Obligation::Popped => Cover {
            change: "the private GP_GET passes the target (pops raise nothing)",
            source: None,
            can_miss: true,
        },
        Obligation::Lagging => Cover {
            change: "the progress an interrupt announced becomes visible in the private words (the hint is spent)",
            source: None,
            can_miss: true,
        },
        Obligation::Contended => Cover {
            change: "the lock the relay lost is released (registered with the holder before giving up; the holder re-checks the registry)",
            source: Some(WakeSource::LockHolder),
            can_miss: false,
        },
    }
}

/// The registered wake source of `o`, if any.
#[must_use]
pub const fn wake_source(o: Obligation) -> Option<WakeSource> {
    cover(o).source
}

/// Whether a worker may block in `epoll_wait` (no timeout) while `held` is outstanding: only when EVERY obligation has a wake source that
/// cannot miss its change.
#[must_use]
pub fn may_block(held: impl IntoIterator<Item = Obligation>) -> bool {
    held.into_iter().all(|o| {
        let c = cover(o);
        c.source.is_some() && !c.can_miss
    })
}

/// Whether a look by `why` has any reason to touch a relay at all.
///
/// ★ The doorbell contract (owner, 2026-10-17): the last doorbell covers every `GP_PUT` write before
/// it, and nothing else announces `GP_PUT`. So a look is warranted by a recorded private-word wait
/// (`waiting`: a marker or carried wait) and by nothing else. `demand` (contention a lost `try_lock` left) is NOT a reason for any look,
/// the safety net included: it belongs to the lock holder (hand-off). The backstop timer finding an idle relay must do nothing and
/// read nothing (no `GP_PUT` poll).
#[must_use]
pub const fn look_needed(_why: Peek, _demand: bool, waiting: bool) -> bool {
    waiting
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
    /// Do not block: the one loop runs again with `epoll_wait(timeout=0)` (an obligation no wake source announces is outstanding).
    Spin,
    /// Block (`try_park`, then `epoll_wait`).
    Park,
}

/// The worker's decision with nothing to serve: it blocks only when every obligation any relay holds has a registered wake source
/// ([`may_block`]); otherwise it keeps evaluating inside the same loop. No spin budget, no counter, no clock.
#[must_use]
pub const fn idle(unannounced: bool) -> Idle {
    if unannounced { Idle::Spin } else { Idle::Park }
}

/// log2 buckets of microseconds: bucket `i` holds `[2^i, 2^(i+1))` us (bucket 0 also 0 us), the
/// last one everything above. Bounded, lock-free.
pub const DRAIN_BUCKETS: usize = 16;

/// Production counters, all bounded (fixed atomics, no per-event allocation).
#[derive(Debug, Default)]
pub struct Stats {
    /// Passive receipt invariant: validated seen count beyond successful native publication.
    pub ens4_unissued_seen: AtomicU64,
    /// Passive receipt invariant: a count regressed during one channel lifetime.
    pub ens4_sequence_regressed: AtomicU64,
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
    /// Loop iterations spent NOT blocking because an obligation without a wake source was outstanding (a carried `Popped` wait, a lagging
    /// marker): the cost of obeying "a worker blocks only when every obligation has a registered wake source".
    pub unannounced_spins: AtomicU64,
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
            "wakes(doorbell={} event={} spin={} backstop={} busy_looks={} unannounced_spins={}) \
            backstop_found(work={} carried={} contended={}) event_without_progress={} gates={} \
            carried_ungated={} credit_waits={} contended_turns={} pop_writebacks={} markers={} \
            drain_us_log2={drain:?} \
            ENS4_UNISSUED_SEEN={} ENS4_SEQUENCE_REGRESSED={}",
            g(&self.wake_doorbell),
            g(&self.wake_event),
            g(&self.wake_spin),
            g(&self.wake_backstop),
            g(&self.busy_looks),
            g(&self.unannounced_spins),
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
            g(&self.ens4_unissued_seen),
            g(&self.ens4_sequence_regressed),
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

    /// ★ The sleep invariant: a worker may block only when every obligation any relay holds has a registered wake source.
    #[test]
    fn a_worker_blocks_only_when_every_obligation_has_a_wake_source() {
        use Obligation::{Contended, Lagging, Marker, Popped};
        assert!(may_block([]), "an idle worker blocks");
        assert!(may_block([Marker, Contended, Marker]));
        for o in [Popped, Lagging] {
            assert_eq!(wake_source(o), None);
            assert!(
                !may_block([Marker, o, Contended]),
                "{o:?} has no wake source: the worker spins"
            );
        }
        assert_eq!(wake_source(Marker), Some(WakeSource::EngineEvent));
        assert_eq!(wake_source(Contended), Some(WakeSource::LockHolder));
        assert_eq!(idle(true), Idle::Spin);
        assert_eq!(idle(false), Idle::Park);
    }

    /// ★ The sleep-safety table: every obligation names its change, its covering wake source and whether the source can miss; the worker blocks
    /// exactly when none of the obligations is uncovered or missable.
    #[test]
    fn the_sleep_table_covers_every_obligation_and_only_non_missable_sources_allow_blocking() {
        for (o, source, can_miss, blocks) in [
            (
                Obligation::Marker,
                Some(WakeSource::EngineEvent),
                false,
                true,
            ),
            (
                Obligation::Contended,
                Some(WakeSource::LockHolder),
                false,
                true,
            ),
            (Obligation::Popped, None, true, false),
            (Obligation::Lagging, None, true, false),
        ] {
            let c = cover(o);
            assert!(!c.change.is_empty());
            assert_eq!((c.source, c.can_miss), (source, can_miss), "{o:?}");
            assert_eq!(may_block([o]), blocks, "{o:?}");
        }
    }

    /// Contention is nobody's look: not the spin's, not the event's, not the safety net's.
    #[test]
    fn no_look_is_warranted_by_contention_alone() {
        for why in [Peek::Spin, Peek::Event, Peek::Backstop] {
            assert!(!look_needed(why, true, false), "{why:?}");
            assert!(look_needed(why, false, true), "{why:?}");
        }
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

    /// ★ DOORBELL CONTRACT: no look, of any kind, has a reason to touch an idle relay -- in
    /// particular the backstop timer never reads `GP_PUT` for one.
    #[test]
    fn no_look_touches_a_relay_with_no_demand_and_no_wait() {
        for why in [Peek::Spin, Peek::Event, Peek::Backstop] {
            assert!(
                !look_needed(why, false, false),
                "{why:?} must leave an idle relay alone"
            );
            assert!(
                !look_needed(why, true, false),
                "{why:?}: contention a lost try_lock left behind is the lock holder's business, not a look's"
            );
            assert!(
                look_needed(why, false, true),
                "{why:?}: a recorded wait is a reason"
            );
            assert!(look_needed(why, true, true));
        }
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
