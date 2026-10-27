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
//! * A replacement stops starting children when demand time is spent; it must still release
//!   created children and restore the original. Repair is mandatory and can overrun the slice.
//! * Admission error cleanup checks the optional allowance between complete reservations.
//!   Retained reservations stay covered and their conversion resumes through the cleanup queue.
//! * What is not started waits for the next refresh (`retry`), keeping partial progress; an
//!   already sparse VA never loses coverage in between.
//!
//! This bounds admission work starts, not total apply/gate wall time. RM calls cannot be
//! preempted; repair and space-retirement cleanup can exceed it. The peak sums charged verbs,
//! including reset/retirement work between refresh renewals, rather than measuring all apply work.
use super::{Ops, RmError};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

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
    interrupt: Arc<AtomicBool>,
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
        if in_split() || self.interrupt.load(Relaxed) {
            return true;
        }
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

thread_local! {
    static CRITICAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// A thread-local split apply scope; it cannot cross threads.
pub struct CriticalGuard {
    previous: bool,
    _local: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for CriticalGuard {
    fn drop(&mut self) {
        CRITICAL.set(self.previous);
    }
}
/// Stop optional reservation changes while applying any split in this batch.
pub fn critical_scope(active: bool) -> CriticalGuard {
    let previous = CRITICAL.get();
    CRITICAL.set(previous || active);
    CriticalGuard {
        previous,
        _local: std::marker::PhantomData,
    }
}
/// Whether layout is on a split's critical path.
#[must_use]
pub fn in_split() -> bool {
    CRITICAL.get()
}
impl Slice {
    /// An arriving split interrupts optional work between verbs; repairs still finish.
    #[must_use]
    pub fn interrupt_handle(&self) -> Arc<AtomicBool> {
        self.interrupt.clone()
    }
}
