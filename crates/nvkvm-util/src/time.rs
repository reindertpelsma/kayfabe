//! Virtual time. The core is deterministic: it never reads a wall clock.
//!
//! [`Instant`] is a plain value handed to the core by the VMM adapter
//! (`Vmm::now()`) or by a test's `MockVmm`, which advances it **explicitly**
//! (`mode2_rust_testing_strategy.md` §4). This is deliberately *not*
//! `std::time::Instant` (which is OS-backed and non-deterministic).

use core::time::Duration;

/// A point on the device's virtual timeline, in nanoseconds since an arbitrary
/// epoch chosen by the VMM adapter. Purely a value; comparing/adding it never
/// touches the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Instant(pub u64);

impl Instant {
    /// The zero point of the virtual timeline.
    pub const ZERO: Instant = Instant(0);

    /// Saturating advance by `d`.
    #[must_use]
    pub fn advanced(self, d: Duration) -> Instant {
        Instant(self.0.saturating_add(d.as_nanos().min(u64::MAX as u128) as u64))
    }

    /// Duration elapsed since `earlier` (zero if `earlier` is later).
    #[must_use]
    pub fn since(self, earlier: Instant) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_and_since_are_pure_arithmetic() {
        let t0 = Instant::ZERO;
        let t1 = t0.advanced(Duration::from_micros(5));
        assert_eq!(t1.since(t0), Duration::from_micros(5));
        assert_eq!(t0.since(t1), Duration::ZERO);
    }
}
