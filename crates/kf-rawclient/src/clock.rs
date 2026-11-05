//! A monotonic clock behind a trait, so every cap and every timing in the arm logic is testable with a
//! fake that advances by a chosen amount per call.

use std::time::Instant;

/// Nanoseconds since an arbitrary origin, monotonic.
pub trait Clock {
    /// The current reading.
    fn now_ns(&self) -> u64;
}

/// The real clock. `std::time::Instant` is `clock_gettime(CLOCK_MONOTONIC)` on Linux, which is what the
/// design asks for; the origin is the moment of construction.
#[derive(Debug)]
pub struct MonotonicClock {
    origin: Instant,
}

impl MonotonicClock {
    /// A clock whose zero is now.
    #[must_use]
    pub fn new() -> Self {
        MonotonicClock {
            origin: Instant::now(),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn now_ns(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

/// Time `f` on `clock`: its result and the elapsed nanoseconds (never negative: the clock is monotonic,
/// and a misbehaving fake is clamped to zero).
pub fn timed<T>(clock: &dyn Clock, f: impl FnOnce() -> T) -> (T, u64) {
    let t0 = clock.now_ns();
    let out = f();
    (out, clock.now_ns().saturating_sub(t0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_clock_never_goes_backwards() {
        let c = MonotonicClock::new();
        let mut last = 0;
        for _ in 0..1000 {
            let n = c.now_ns();
            assert!(n >= last);
            last = n;
        }
    }

    #[test]
    fn timed_returns_the_value_and_a_duration() {
        let c = MonotonicClock::new();
        let (v, ns) = timed(&c, || {
            std::thread::sleep(std::time::Duration::from_millis(2));
            7
        });
        assert_eq!(v, 7);
        assert!(ns >= 1_000_000, "{ns}");
    }
}
