//! ENS6 layout time slice: the bound on RM reservation verbs one refresh may START.
//!
//! Measured on 7330 (2026-10-27, rev 6d95870c7): a reservation verb costs about 17 us on
//! the host (271 ms / 15 798 calls in the first slow apply), and the 14 slow applies of
//! 0.3 .. 1.95 s were reservation-verb churn (made/freed grew by ~20k per heartbeat, up to
//! ~59k verbs in the heaviest one), not single slow calls.
//! A refresh (one walker entry, `begin_refresh` .. `end_refresh`) holds the VM-wide walk gate and
//! the placement lock while it runs, so its layout work has a stated bound:
//!
//! * **Demand work** (an explicit sparse admission the guest asked for) may start until the
//!   refresh has spent [`SLICE_NS`] or [`SLICE_VERBS`].
//! * **Optional work** (plain-map refinement, recycling, converting retired sparse blocks) stops
//!   at half of that, so it can never starve demand work of the same refresh.
//! * Work that has started is finished or rolled back (a half-done replacement must restore its
//!   original reservation), so a refresh overshoots by at most one replacement and its restore.
//! * What is not started waits for the next refresh (`retry`), keeping partial progress; an
//!   already sparse VA never loses coverage in between.
use super::{Ops, RmError};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

/// Wall time of reservation verbs one refresh may start before demand work stops.
pub const SLICE_NS: u64 = 100_000_000;
/// Reservation verbs one refresh may start before demand work stops.
pub const SLICE_VERBS: u32 = 4096;
/// Wall time after which optional work stops starting.
pub const SPECULATIVE_NS: u64 = SLICE_NS / 2;
/// Verbs after which optional work stops starting.
pub const SPECULATIVE_VERBS: u32 = SLICE_VERBS / 2;

/// The per-refresh layout allowance.
#[derive(Debug, Default)]
pub struct Slice {
    verbs: AtomicU32,
    ns: AtomicU64,
    peak_verbs: AtomicU32,
    peak_ns: AtomicU64,
}

impl Slice {
    /// A new refresh begins: the previous one's spend becomes part of the peak.
    pub fn renew(&self) {
        let v = self.verbs.swap(0, Relaxed);
        let n = self.ns.swap(0, Relaxed);
        self.peak_verbs.fetch_max(v, Relaxed);
        self.peak_ns.fetch_max(n, Relaxed);
    }

    /// One verb of `ns` wall time was issued.
    pub fn spend(&self, ns: u64) {
        self.verbs.fetch_add(1, Relaxed);
        self.ns.fetch_add(ns, Relaxed);
    }

    /// Whether work of this kind must not START now.
    #[must_use]
    pub fn exhausted(&self, demand: bool) -> bool {
        let (verbs, ns) = self.spent();
        if demand {
            verbs >= SLICE_VERBS || ns >= SLICE_NS
        } else {
            verbs >= SPECULATIVE_VERBS || ns >= SPECULATIVE_NS
        }
    }

    /// `(verbs, ns)` spent in the current refresh.
    #[must_use]
    pub fn spent(&self) -> (u32, u64) {
        (self.verbs.load(Relaxed), self.ns.load(Relaxed))
    }

    /// `(verbs, ns)` of the busiest refresh so far, the running one included.
    #[must_use]
    pub fn peak(&self) -> (u32, u64) {
        let (v, n) = self.spent();
        (
            self.peak_verbs.load(Relaxed).max(v),
            self.peak_ns.load(Relaxed).max(n),
        )
    }
}

/// An [`Ops`] that charges its verbs to a [`Slice`]; `clock` is monotonic nanoseconds.
pub(crate) struct Sliced<'a> {
    pub(crate) inner: &'a dyn Ops,
    pub(crate) slice: &'a Slice,
    pub(crate) clock: &'a dyn Fn() -> u64,
}

impl Sliced<'_> {
    fn timed<T>(&self, f: impl FnOnce() -> T) -> T {
        let start = (self.clock)();
        let out = f();
        self.slice.spend((self.clock)().saturating_sub(start));
        out
    }
}

impl Ops for Sliced<'_> {
    fn reserve(&self, lo: u64, len: u64, sparse: bool) -> Result<u32, RmError> {
        self.timed(|| self.inner.reserve(lo, len, sparse))
    }
    fn release(&self, handle: u32) -> Result<(), RmError> {
        self.timed(|| self.inner.release(handle))
    }
    fn budget_left(&self, demand: bool) -> bool {
        !self.slice.exhausted(demand)
    }
}

/// Monotonic nanoseconds since the first call in this process.
#[must_use]
pub(crate) fn mono_ns() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    u64::try_from(
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_nanos(),
    )
    .unwrap_or(u64::MAX)
}
