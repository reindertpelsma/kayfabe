//! The two "do not look again yet" decisions of the USER-ring relay's turn, as pure functions.
//!
//! Every relay turn today (a) re-reads the guest's `GP_PUT` from the guest `USERD`, and (b) reads the private
//! channel's `USERD` (`GP_GET` and `REFERENCE`: two loads through the BAR1 / store view) and writes the guest's
//! `GP_GET` back. At ~1,700 GP entries per Heaven D3D11 frame that is most of the relay's CPU. Two independent
//! decisions remove it without changing what is published:
//!
//! * [`PutCache`] (A): an entry below a `GP_PUT` the relay has already read stays valid, so the guest `GP_PUT` is
//!   re-read only when the cursor has reached the last value read.
//! * [`must_observe`] (B): the private `USERD` is observed every [`OBSERVE_INTERVAL`]-th turn, and EVERY turn while
//!   the relay is waiting for something, holds a segment open, or is short of private-ring credits.
//!
//! No clock, no timer, no environment: these functions take their inputs as arguments. ★ 2026-10-17 (PRODUCTION):
//! [`Lazy`] is the per-relay state that `crate::user_turn::Relay` carries and consults INSIDE its turn (behind the
//! `Io` trait), so the model, property and stress suites that drive the real turn also drive these rules. The pump
//! (`kf-qemu`) only supplies the accessors and reads the two measurement switches once ([`LazyConfig`]).
//! Active ONLY in Pointer mode of the Windows USER-ring relay (`docs/design/V3_USER_RING_LIFECYCLE_20261016.md`,
//! "Lazy observe (production)"); Scan and Window turns always read fresh.
//!
//! # Invariants these rules rely on (and the only things they rely on)
//!
//! * **Staleness is conservative** (B). A stale `GP_GET` can only LOWER the credit count (`Fetch::available`
//!   counts entries not yet seen popped), never raise it; a stale `REFERENCE` can only DELAY the release of a
//!   `SYNC_WAIT`-gated segment (`user_gate::Marks::peek` never implies progress that has not happened, and
//!   `SyncGate::entry` is monotone in `processed`). Correctness therefore never depends on
//!   [`OBSERVE_INTERVAL`]; it only trades latency of credits and of the guest's `GP_GET` write-back for CPU.
//! * **I8 / monotone PUT** (A). Within one ring generation the guest only advances `GP_PUT`, so a PUT read earlier
//!   is never beyond the real PUT: every entry strictly before the cached PUT (cursor included, cached PUT
//!   excluded) was published by the guest before that read. No entry at or after the loaded PUT is ever read.
//! * **The doorbell contract** (A). Every `GP_PUT` advance is followed by a doorbell that re-runs a turn. A cached
//!   PUT is never the reason to go idle: [`PutCache::select`] returns `None` when the cursor equals the cached
//!   PUT, so "nothing to do" (`end_batch`, idle, the batch marker) is decided ONLY on a fresh read. After any
//!   doorbell the relay therefore cannot park with an unread PUT advance.
//! * **A new ring generation forgets everything** (A): the migrated ring may have a LOWER PUT; the cache is keyed
//!   by generation and a different generation never matches.

/// Turns between observations of the private `USERD` when nothing forces an earlier look.
///
/// Relies only on the conservative-staleness invariant in the module docs: any value >= 1 is correct; 1 observes
/// every turn (today's behaviour, bit for bit). The value trades the latency of credit release and of the guest
/// `GP_GET` write-back (the guest ring has 32,768+ entries) against CPU.
pub const OBSERVE_INTERVAL: u64 = 16;

/// Private-ring credits below which every turn observes (a nearly full ring must learn about pops at once, or
/// the relay could wait for credits it already has).
pub const CREDIT_FLOOR: u32 = 64;

/// Whether this turn must read the private `USERD` (`GP_GET`, `REFERENCE`) and write the guest `GP_GET` back.
///
/// `turn_no` counts turns of this channel; `interval` is [`OBSERVE_INTERVAL`] (or an override; `<= 1` = every
/// turn); `prev_waiting` is true when the previous turn ended waiting for something (a marker, pops, a contended
/// lock); `available` is the private ring's free credits; `segment_open` is true while a split segment is being
/// published. Liveness: a waiting relay observes on EVERY turn, so a wake never meets a stale view.
#[must_use]
pub fn must_observe(
    turn_no: u64,
    interval: u64,
    prev_waiting: bool,
    available: u32,
    segment_open: bool,
) -> bool {
    interval <= 1
        || prev_waiting
        || segment_open
        || available < CREDIT_FLOOR
        || turn_no.is_multiple_of(interval)
}

/// The last guest `GP_PUT` the relay read, keyed by the ring generation it was read in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PutCache {
    last: Option<(u64, u32)>,
}

impl PutCache {
    /// The `GP_PUT` this turn may use WITHOUT reading the guest `USERD`, or `None` (read it).
    ///
    /// `Some` only when the cache is `enabled`, was recorded in the same ring `generation`, and the cursor has
    /// not yet reached it. `None` whenever `cursor == cached PUT`: the relay then reads fresh, so it can only
    /// go idle on a fresh read (the doorbell contract). Never returns a value beyond the real PUT (I8).
    #[must_use]
    pub fn select(&self, enabled: bool, generation: u64, cursor: u32) -> Option<u32> {
        if !enabled {
            return None;
        }
        match self.last {
            Some((g, put)) if g == generation && put != cursor => Some(put),
            _ => None,
        }
    }

    /// Record the `GP_PUT` this turn used (read fresh, or the selected cached value).
    pub fn record(&mut self, generation: u64, put: u32) {
        self.last = Some((generation, put));
    }

    /// Forget the cache (a ring teardown or migration may call this; a generation change already never matches).
    pub fn invalidate(&mut self) {
        self.last = None;
    }

    /// Whether a `GP_PUT` is remembered.
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.last.is_some()
    }

    /// The remembered `(generation, GP_PUT)`.
    #[must_use]
    pub fn get(&self) -> Option<(u64, u32)> {
        self.last
    }
}

// ------------------------------------------------------------------------------------------------
// ★ 2026-10-17 production integration

use crate::user_lifecycle::Mode;

/// Environment switch that DISABLES the `GP_PUT` cache (A) when set to `0` (measurement class: default ON, never needed
/// to boot; read ONCE when the first relay is created).
pub const ENV_PUT_CACHE: &str = "KF3_PUT_CACHE";
/// Environment switch that DISABLES the lazy private-`USERD` observation (B) when set to `0` (same class). The interval is
/// the constant [`OBSERVE_INTERVAL`]; no other value changes it.
pub const ENV_LAZY_OBSERVE: &str = "KF3_LAZY_OBSERVE";

/// Whether a switch is ON: unset, or anything but `0`.
#[must_use]
pub fn switch_on(value: Option<&str>) -> bool {
    value.is_none_or(|v| v.trim() != "0")
}

/// Which of the two rules a relay applies. Pure data; the environment is read by the owner, once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LazyConfig {
    /// A: cache the guest `GP_PUT` until the cursor reaches it.
    pub put_cache: bool,
    /// B: turns between observations of the private `USERD` (`<= 1` = every turn = B off).
    pub interval: u64,
}

impl LazyConfig {
    /// Both rules off: the relay behaves bit for bit as it did before lazy observe existed.
    pub const OFF: Self = Self {
        put_cache: false,
        interval: 1,
    };
    /// The production default: A on, B on with [`OBSERVE_INTERVAL`].
    pub const ON: Self = Self {
        put_cache: true,
        interval: OBSERVE_INTERVAL,
    };
    /// The production default with both switches' values (`None` = unset): `0` turns that rule off, anything else leaves
    /// it on.
    #[must_use]
    pub fn from_switches(put_cache: Option<&str>, lazy_observe: Option<&str>) -> Self {
        Self {
            put_cache: switch_on(put_cache),
            interval: if switch_on(lazy_observe) {
                OBSERVE_INTERVAL
            } else {
                1
            },
        }
    }
    /// Test hook: another interval.
    #[must_use]
    pub const fn with_interval(self, interval: u64) -> Self {
        Self { interval, ..self }
    }
    /// Test hook: A on or off.
    #[must_use]
    pub const fn with_put_cache(self, put_cache: bool) -> Self {
        Self { put_cache, ..self }
    }
}

/// The interval actually used on a guest ring of `src_entries` slots: never more than half the ring. The relay relays at
/// most ONE guest entry per turn, so between two write-backs of the guest's `GP_GET` it consumes at most this many slots;
/// on a small ring the interval shrinks to 1 and the relay is as eager as before (design doc, D6).
#[must_use]
pub fn effective_interval(interval: u64, src_entries: u32) -> u64 {
    if interval <= 1 {
        return 1;
    }
    interval.min(u64::from(src_entries / 2).max(1))
}

/// Why a turn reads the private `USERD` (counted by reason).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObserveReason {
    /// The previous turn ended waiting for something (a marker, pops, a lock), or a SYNC_WAIT gate is draining.
    Wait,
    /// A split segment is being published.
    OpenSegment,
    /// Fewer than [`CREDIT_FLOOR`] private-ring credits.
    LowCredit,
    /// The guest ring looked FULL at the previous turn's `GP_PUT` read and that turn did not write `GP_GET` back (D6).
    RingFull,
    /// The periodic look (every `interval`-th turn).
    Interval,
}

/// [`must_observe`] with the reason and the D6 guard. `ring_full` = the previous turn that skipped its observation saw
/// the guest ring full. `must_observe(..) == observe_reason(.., false).is_some()` (tested).
#[must_use]
pub fn observe_reason(
    turn_no: u64,
    interval: u64,
    prev_waiting: bool,
    available: u32,
    segment_open: bool,
    ring_full: bool,
) -> Option<ObserveReason> {
    if interval <= 1 {
        Some(ObserveReason::Interval)
    } else if prev_waiting {
        Some(ObserveReason::Wait)
    } else if segment_open {
        Some(ObserveReason::OpenSegment)
    } else if available < CREDIT_FLOOR {
        Some(ObserveReason::LowCredit)
    } else if ring_full {
        Some(ObserveReason::RingFull)
    } else if turn_no.is_multiple_of(interval) {
        Some(ObserveReason::Interval)
    } else {
        None
    }
}

/// Why the cached `GP_PUT` was dropped (counted by reason, only when something was cached).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Invalidate {
    /// The lifecycle mode changed (Scan -> Pointer, Pointer -> Window, Window -> Pointer).
    ModeChange,
    /// The guest ring was switched (ring growth): the new ring may have a LOWER `GP_PUT`.
    Migration,
    /// The relay was refused and stopped (the guest's channel is reset).
    Reset,
    /// The relay was retired (channel freed).
    Retire,
}

macro_rules! lazy_counts {
    ($($(#[$doc:meta])* $f:ident),* $(,)?) => {
        /// Bounded per-relay counters of lazy observe (plain integers; the pump folds them into [`LazyStats`]).
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub struct LazyCounts {
            $( $(#[$doc])* pub $f: u64, )*
        }
        impl LazyCounts {
            /// Field-wise `self - prev`.
            #[must_use]
            pub fn since(&self, prev: &Self) -> Self {
                Self { $( $f: self.$f - prev.$f, )* }
            }
        }
        /// VM-wide lazy-observe counters (fixed atomics, no per-event allocation).
        #[derive(Debug, Default)]
        pub struct LazyStats {
            $( $(#[$doc])* pub $f: std::sync::atomic::AtomicU64, )*
        }
        impl LazyStats {
            /// Merge one relay's delta.
            pub fn add(&self, d: &LazyCounts) {
                use std::sync::atomic::Ordering::Relaxed;
                $( if d.$f != 0 { self.$f.fetch_add(d.$f, Relaxed); } )*
            }
            /// Whether any relay ever took a turn.
            #[must_use]
            pub fn any(&self) -> bool {
                self.turns.load(std::sync::atomic::Ordering::Relaxed) > 0
            }
            /// One bounded line.
            #[must_use]
            pub fn summary(&self) -> String {
                use std::sync::atomic::Ordering::Relaxed;
                let mut s = String::new();
                $( s.push_str(&format!("{}={} ", stringify!($f), self.$f.load(Relaxed))); )*
                s.trim_end().to_owned()
            }
        }
        impl LazyCounts {
            /// One bounded line.
            #[must_use]
            pub fn summary(&self) -> String {
                let mut s = String::new();
                $( s.push_str(&format!("{}={} ", stringify!($f), self.$f)); )*
                s.trim_end().to_owned()
            }
        }
    };
}
lazy_counts!(
    /// Turns the relay began (every `Phase::Obs`).
    turns,
    /// Turns that used the cached `GP_PUT` (A).
    put_cache_hits,
    /// Turns that READ `GP_PUT` from the guest `USERD` (any mode).
    put_fresh_reads,
    /// Turns that skipped the private-`USERD` observation (B).
    obs_skipped,
    /// Turns that observed because B does not apply (Scan / Window turns, or B switched off).
    obs_unconditional,
    /// B forced a look: the previous turn ended waiting / a gate is draining.
    obs_forced_wait,
    /// B forced a look: fewer than `CREDIT_FLOOR` private credits.
    obs_forced_low_credit,
    /// B forced a look: a split segment is open.
    obs_forced_open_segment,
    /// B forced a look: the guest ring looked full (D6).
    obs_forced_ring_full,
    /// B looked on its interval.
    obs_forced_interval,
    /// The cache was dropped because the lifecycle mode changed.
    inval_mode_change,
    /// The cache was dropped because the guest ring was switched.
    inval_migration,
    /// The cache was dropped because the relay was refused (channel reset).
    inval_reset,
    /// The cache was dropped because the relay was retired.
    inval_retire,
);

/// The per-relay lazy-observe state (see the module docs).
#[derive(Clone, Debug)]
pub struct Lazy {
    cfg: LazyConfig,
    cache: PutCache,
    mode: Option<Mode>,
    turn_no: u64,
    /// The relay ended a turn waiting for something and has not observed since.
    wait_pending: bool,
    /// The guest ring looked full in a turn that skipped its observation (D6).
    ring_full: bool,
    /// This turn skipped its observation.
    skipped: bool,
    /// Counters (taken in deltas).
    pub counts: LazyCounts,
    reported: LazyCounts,
}

impl Lazy {
    /// Fresh state.
    #[must_use]
    pub fn new(cfg: LazyConfig) -> Self {
        Self {
            cfg,
            cache: PutCache::default(),
            mode: None,
            turn_no: 0,
            wait_pending: false,
            ring_full: false,
            skipped: false,
            counts: LazyCounts::default(),
            reported: LazyCounts::default(),
        }
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> LazyConfig {
        self.cfg
    }

    /// Rule A applies in this mode (Pointer only).
    #[must_use]
    pub fn a_active(&self, mode: Mode) -> bool {
        self.cfg.put_cache && mode == Mode::Pointer
    }

    /// Rule B applies in this mode (Pointer only, interval above 1).
    #[must_use]
    pub fn b_active(&self, mode: Mode) -> bool {
        self.cfg.interval > 1 && mode == Mode::Pointer
    }

    /// The cached `(generation, GP_PUT)`, if any.
    #[must_use]
    pub fn cached(&self) -> Option<(u64, u32)> {
        self.cache.get()
    }

    /// A turn begins in `mode`: count it, and drop the cache if the mode is not the one it was filled in.
    pub fn turn_start(&mut self, mode: Mode) {
        self.turn_no = self.turn_no.wrapping_add(1);
        self.counts.turns += 1;
        self.skipped = false;
        if self.mode != Some(mode) {
            if self.mode.is_some() {
                self.invalidate(Invalidate::ModeChange);
            }
            self.mode = Some(mode);
        }
    }

    /// Whether this turn reads the private `USERD`; counts the decision. `src_entries` = the guest ring's slots.
    pub fn observe_decision(
        &mut self,
        mode: Mode,
        src_entries: u32,
        available: u32,
        segment_open: bool,
        sync_waiting: bool,
    ) -> bool {
        if !self.b_active(mode) {
            self.counts.obs_unconditional += 1;
            return true;
        }
        let why = observe_reason(
            self.turn_no,
            effective_interval(self.cfg.interval, src_entries),
            self.wait_pending || sync_waiting,
            available,
            segment_open,
            self.ring_full,
        );
        match why {
            None => {
                self.counts.obs_skipped += 1;
                self.skipped = true;
                false
            }
            Some(ObserveReason::Wait) => {
                self.counts.obs_forced_wait += 1;
                true
            }
            Some(ObserveReason::OpenSegment) => {
                self.counts.obs_forced_open_segment += 1;
                true
            }
            Some(ObserveReason::LowCredit) => {
                self.counts.obs_forced_low_credit += 1;
                true
            }
            Some(ObserveReason::RingFull) => {
                self.counts.obs_forced_ring_full += 1;
                true
            }
            Some(ObserveReason::Interval) => {
                self.counts.obs_forced_interval += 1;
                true
            }
        }
    }

    /// The private `USERD` was read (and the guest `GP_GET` write-back that follows is owed): the view is fresh.
    pub fn observed(&mut self) {
        self.wait_pending = false;
        self.ring_full = false;
    }

    /// The relay ended a turn waiting for something: the next turn must look.
    pub fn wait_set(&mut self) {
        self.wait_pending = true;
    }

    /// The `GP_PUT` this turn may use without reading the guest `USERD` (A), or `None` (read it).
    pub fn put_select(
        &mut self,
        mode: Mode,
        generation: u64,
        cursor: u32,
        src_entries: u32,
    ) -> Option<u32> {
        if !self.a_active(mode) {
            return None;
        }
        let put = self.cache.select(true, generation, cursor)?;
        if put >= src_entries {
            // never trust a remembered value outside the ring (the ring may have been replaced)
            return None;
        }
        self.counts.put_cache_hits += 1;
        Some(put)
    }

    /// The turn read `put` from the guest `USERD`: count it and, in a mode where A applies and the value is inside the
    /// ring, remember it.
    pub fn put_fresh(&mut self, mode: Mode, generation: u64, put: u32, src_entries: u32) {
        self.counts.put_fresh_reads += 1;
        if self.a_active(mode) && put < src_entries {
            self.cache.record(generation, put);
        }
    }

    /// D6: the guest ring looked full at this turn's `GP_PUT`. If the turn skipped its observation, the guest's `GP_GET`
    /// write-back may be the very thing it waits for: the NEXT turn observes.
    pub fn ring_full_seen(&mut self) {
        if self.skipped {
            self.ring_full = true;
        }
    }

    /// Drop the cached `GP_PUT`; counted by reason when something was cached.
    pub fn invalidate(&mut self, why: Invalidate) {
        if self.cache.is_set() {
            match why {
                Invalidate::ModeChange => self.counts.inval_mode_change += 1,
                Invalidate::Migration => self.counts.inval_migration += 1,
                Invalidate::Reset => self.counts.inval_reset += 1,
                Invalidate::Retire => self.counts.inval_retire += 1,
            }
        }
        self.cache.invalidate();
    }

    /// Counters since the last call.
    pub fn take_delta(&mut self) -> LazyCounts {
        let d = self.counts.since(&self.reported);
        self.reported = self.counts;
        d
    }

    /// Whether the owner should fold the counters into the VM-wide ones now (every 256th turn).
    #[must_use]
    pub fn flush_due(&self) -> bool {
        self.turn_no & 0xff == 0
    }

    /// What decides behaviour (model checking); empty when both rules are off, so the key of an OFF relay is unchanged.
    #[must_use]
    pub fn key(&self) -> String {
        if !self.cfg.put_cache && self.cfg.interval <= 1 {
            return String::new();
        }
        format!(
            "|L{}:{:?}:{}{}{}",
            self.turn_no % self.cfg.interval.max(1),
            self.cache.get(),
            u8::from(self.wait_pending),
            u8::from(self.ring_full),
            u8::from(self.skipped),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switches_default_on_and_only_zero_turns_a_rule_off() {
        assert_eq!(LazyConfig::from_switches(None, None), LazyConfig::ON);
        assert_eq!(
            LazyConfig::from_switches(Some("1"), Some("16")),
            LazyConfig::ON
        );
        assert_eq!(
            LazyConfig::from_switches(Some("yes"), Some("")),
            LazyConfig::ON
        );
        let a_off = LazyConfig::from_switches(Some("0"), None);
        assert!(!a_off.put_cache && a_off.interval == OBSERVE_INTERVAL);
        let b_off = LazyConfig::from_switches(None, Some("0"));
        assert!(b_off.put_cache && b_off.interval == 1);
        assert_eq!(
            LazyConfig::from_switches(Some("0"), Some("0")),
            LazyConfig::OFF
        );
        assert_eq!(
            LazyConfig::from_switches(Some(" 0 "), Some("0")),
            LazyConfig::OFF
        );
    }

    #[test]
    fn the_interval_never_exceeds_half_the_guest_ring() {
        assert_eq!(effective_interval(16, 512), 16);
        assert_eq!(effective_interval(16, 32), 16);
        assert_eq!(effective_interval(16, 16), 8);
        assert_eq!(effective_interval(16, 8), 4);
        assert_eq!(effective_interval(16, 2), 1);
        assert_eq!(effective_interval(16, 0), 1);
        assert_eq!(effective_interval(1, 512), 1);
        assert_eq!(effective_interval(0, 512), 1);
    }

    #[test]
    fn observe_reason_agrees_with_must_observe_when_the_ring_guard_is_off() {
        for interval in [0u64, 1, 2, 3, 16] {
            for turn in 0..40u64 {
                for waiting in [false, true] {
                    for avail in [0u32, 63, 64, 500] {
                        for open in [false, true] {
                            assert_eq!(
                                must_observe(turn, interval, waiting, avail, open),
                                observe_reason(turn, interval, waiting, avail, open, false)
                                    .is_some()
                            );
                            // the guard only ever ADDS a look
                            if must_observe(turn, interval, waiting, avail, open) {
                                assert!(
                                    observe_reason(turn, interval, waiting, avail, open, true)
                                        .is_some()
                                );
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(
            observe_reason(3, 16, false, 500, false, true),
            Some(ObserveReason::RingFull)
        );
        assert_eq!(
            observe_reason(3, 16, true, 500, false, true),
            Some(ObserveReason::Wait)
        );
    }

    #[test]
    fn the_cache_is_dropped_and_counted_by_reason_only_when_something_was_cached() {
        let mut l = Lazy::new(LazyConfig::ON);
        l.invalidate(Invalidate::Migration);
        assert_eq!(
            l.counts.inval_migration, 0,
            "nothing cached: nothing to count"
        );
        l.put_fresh(Mode::Pointer, 1, 5, 64);
        assert_eq!(l.cached(), Some((1, 5)));
        l.invalidate(Invalidate::Migration);
        assert_eq!((l.counts.inval_migration, l.cached()), (1, None));
        for (why, field) in [
            (Invalidate::ModeChange, 0),
            (Invalidate::Reset, 1),
            (Invalidate::Retire, 2),
        ] {
            l.put_fresh(Mode::Pointer, 1, 5, 64);
            l.invalidate(why);
            let c = &l.counts;
            let got = [c.inval_mode_change, c.inval_reset, c.inval_retire];
            assert_eq!(got[field], 1, "{why:?}");
        }
    }

    #[test]
    fn rule_a_applies_in_pointer_mode_only() {
        let mut l = Lazy::new(LazyConfig::ON);
        for m in [Mode::Scan, Mode::Window, Mode::Dead] {
            l.put_fresh(m, 1, 5, 64);
            assert_eq!(l.cached(), None, "{m:?} records nothing");
            assert_eq!(l.put_select(m, 1, 0, 64), None);
        }
        l.put_fresh(Mode::Pointer, 1, 5, 64);
        assert_eq!(l.put_select(Mode::Pointer, 1, 0, 64), Some(5));
        assert_eq!(
            l.put_select(Mode::Pointer, 1, 5, 64),
            None,
            "cursor == cached PUT: read fresh"
        );
        assert_eq!(
            l.put_select(Mode::Pointer, 2, 0, 64),
            None,
            "another generation"
        );
        assert_eq!(
            l.put_select(Mode::Pointer, 1, 0, 4),
            None,
            "outside the (replaced) ring"
        );
        assert_eq!(l.put_select(Mode::Window, 1, 0, 64), None);
    }

    #[test]
    fn the_counters_account_for_every_turn() {
        let mut l = Lazy::new(LazyConfig::ON);
        for t in 0..100u64 {
            let mode = if t % 7 == 0 {
                Mode::Window
            } else {
                Mode::Pointer
            };
            l.turn_start(mode);
            let obs = l.observe_decision(
                mode,
                512,
                if t % 11 == 0 { 3 } else { 500 },
                t % 13 == 0,
                false,
            );
            if obs {
                l.observed();
            }
            if t % 5 == 0 {
                l.wait_set();
            }
        }
        let c = l.counts;
        assert_eq!(
            c.turns,
            c.obs_skipped
                + c.obs_unconditional
                + c.obs_forced_wait
                + c.obs_forced_low_credit
                + c.obs_forced_open_segment
                + c.obs_forced_ring_full
                + c.obs_forced_interval
        );
        let d = l.take_delta();
        assert_eq!(d, c);
        assert_eq!(l.take_delta(), LazyCounts::default());
    }
}
