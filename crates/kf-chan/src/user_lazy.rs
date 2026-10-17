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
//! No clock, no timer, no environment: these functions take their inputs as arguments. The (scratch) call site
//! decides whether the optimisation is enabled and may override the interval.
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
        || turn_no % interval == 0
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
}
