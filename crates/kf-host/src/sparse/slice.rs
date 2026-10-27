//! ENS6 layout time slice. STUB for the failing-first commit: spends nothing, never exhausts.
use super::{Ops, RmError};
use std::sync::atomic::{AtomicU32, AtomicU64};

/// Wall time of reservation verbs one refresh may START before optional layout work stops.
pub const SLICE_NS: u64 = 50_000_000;
/// Reservation verbs one refresh may start before optional layout work stops.
pub const SLICE_VERBS: u32 = 1024;

/// The per-refresh layout allowance.
#[derive(Debug, Default)]
pub struct Slice {
    verbs: AtomicU32,
    ns: AtomicU64,
    peak_verbs: AtomicU32,
    peak_ns: AtomicU64,
}

impl Slice {
    /// A new refresh begins.
    pub fn renew(&self) {}
    /// One verb of `ns` wall time was issued.
    pub fn spend(&self, _ns: u64) {}
    /// Optional layout work must not start.
    #[must_use]
    pub fn exhausted(&self) -> bool {
        false
    }
    /// `(verbs, ns)` spent in the current refresh.
    #[must_use]
    pub fn spent(&self) -> (u32, u64) {
        (0, 0)
    }
    /// `(verbs, ns)` of the busiest refresh so far.
    #[must_use]
    pub fn peak(&self) -> (u32, u64) {
        (0, 0)
    }
}

/// An [`Ops`] that charges its verbs to a [`Slice`].
pub(crate) struct Sliced<'a> {
    pub(crate) inner: &'a dyn Ops,
    pub(crate) slice: &'a Slice,
    pub(crate) clock: &'a dyn Fn() -> u64,
}

impl Ops for Sliced<'_> {
    fn reserve(&self, lo: u64, len: u64, sparse: bool) -> Result<u32, RmError> {
        self.inner.reserve(lo, len, sparse)
    }
    fn release(&self, handle: u32) -> Result<(), RmError> {
        self.inner.release(handle)
    }
    fn budget_left(&self) -> bool {
        let _ = (&self.clock, &self.slice);
        true
    }
}
