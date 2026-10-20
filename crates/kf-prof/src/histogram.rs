//! Single-owner, saturating exact counts and log2 latency distribution.

/// A fixed histogram; zero has its own bucket, positive values use floor(log2)+1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Histogram {
    /// Number of observations, saturating.
    pub count: u64,
    /// Sum of durations, saturating.
    pub total_ns: u64,
    /// Largest duration.
    pub max_ns: u64,
    /// Bounded integer argument of the earliest maximum observation.
    pub max_arg: u64,
    /// Bucket counts, including zero and all u64 values.
    pub buckets: [u64; 65],
    /// At least one count or duration saturated.
    pub saturated: bool,
}
impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}
impl Histogram {
    /// Empty, allocation-free histogram.
    pub const fn new() -> Self {
        Self {
            count: 0,
            total_ns: 0,
            max_ns: 0,
            max_arg: 0,
            buckets: [0; 65],
            saturated: false,
        }
    }
    /// Bucket of any unsigned nanosecond duration.
    pub fn bucket(ns: u64) -> usize {
        if ns == 0 {
            0
        } else {
            (64 - ns.leading_zeros()) as usize
        }
    }
    /// Record an observation. Requires only stores to this owner's memory.
    pub fn record(&mut self, ns: u64, arg: u64) {
        let b = Self::bucket(ns);
        self.saturated |= self.count == u64::MAX
            || self.buckets[b] == u64::MAX
            || self.total_ns.checked_add(ns).is_none();
        self.count = self.count.saturating_add(1);
        self.total_ns = self.total_ns.saturating_add(ns);
        self.buckets[b] = self.buckets[b].saturating_add(1);
        if ns > self.max_ns || self.count == 1 {
            self.max_ns = ns;
            self.max_arg = arg;
        }
    }
}
