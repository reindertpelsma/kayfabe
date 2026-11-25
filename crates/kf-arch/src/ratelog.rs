//! ★ One rate limit for guest-reachable log lines. `STATUS: LIVE, 2026-11-13` (security review 1,
//! F-13 / S1-86).
//!
//! The owner's rules: production is quiet on input-serving threads, and the register drainer may
//! never block. One `eprintln!` per guest event breaks both: a guest that repeats an event floods
//! the log (a file sink grows without bound), and every line takes Rust's stderr lock, which a slow
//! sink turns into a block on the drainer and on every thread that logs.
//!
//! [`RateLog::admit`] says yes to the first [`FIRST`] events of a site and then only to event
//! numbers that are powers of two: after `n` events a site has written at most
//! `FIRST + log2(n)` lines, and each line carries the running count, so "it happened" and "how
//! often" both stay legible from the log (a silent hold once read as absence, `kf-gsp` w425).
//! Lock-free: one relaxed compare-and-swap loop, no allocation unless the line is admitted.
//!
//! [`crate::rate_eprintln!`] is the usual form: one counter per call site.

use core::sync::atomic::{AtomicU64, Ordering};

/// Events written in full before the power-of-two thinning starts.
pub const FIRST: u64 = 8;

/// A per-site event counter. `const`-constructible, so a site can own a `static`.
#[derive(Debug, Default)]
pub struct RateLog {
    seen: AtomicU64,
}

impl RateLog {
    /// A counter at zero.
    #[must_use]
    pub const fn new() -> RateLog {
        RateLog {
            seen: AtomicU64::new(0),
        }
    }

    /// Count one event; `Some(n)` (the 1-based event number) when its line should be written:
    /// `n <= FIRST`, or `n` a power of two. Saturates at `u64::MAX` (then stays quiet).
    pub fn admit(&self) -> Option<u64> {
        // A CAS loop, not `fetch_add`: the count must saturate, never wrap back to 1.
        let prev = self
            .seen
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .ok()?;
        let n = prev.saturating_add(1);
        admitted(n).then_some(n)
    }

    /// Events counted so far (written or not).
    #[must_use]
    pub fn seen(&self) -> u64 {
        self.seen.load(Ordering::Relaxed)
    }
}

/// Is event number `n` (1-based) written?
#[must_use]
pub const fn admitted(n: u64) -> bool {
    n != 0 && (n <= FIRST || n.is_power_of_two())
}

/// `eprintln!` behind a per-call-site [`RateLog`]: the first [`FIRST`] events of this site, then
/// powers of two, each line ending ` [#N, rate-limited]`.
#[macro_export]
macro_rules! rate_eprintln {
    ($($arg:tt)*) => {{
        static SITE: $crate::ratelog::RateLog = $crate::ratelog::RateLog::new();
        if let ::core::option::Option::Some(n) = SITE.admit() {
            ::std::eprintln!("{} [#{}, rate-limited]", ::core::format_args!($($arg)*), n);
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_eight_then_powers_of_two() {
        let r = RateLog::new();
        let written: Vec<u64> = (0..1100).filter_map(|_| r.admit()).collect();
        assert_eq!(
            written,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 16, 32, 64, 128, 256, 512, 1024]
        );
        assert_eq!(r.seen(), 1100);
    }

    #[test]
    fn a_million_events_write_a_handful_of_lines() {
        let r = RateLog::new();
        let lines = (0..1_000_000).filter(|_| r.admit().is_some()).count();
        assert_eq!(lines, 8 + 16, "8 in full, then 16, 32, ..., 2^19");
    }

    #[test]
    fn hostile_counts_never_admit_zero_or_wrap() {
        assert!(!admitted(0));
        assert!(!admitted(u64::MAX));
        assert!(admitted(1 << 63));
        assert!(!admitted((1 << 63) + 1));
        let r = RateLog {
            seen: AtomicU64::new(u64::MAX - 1),
        };
        assert_eq!(r.admit(), None, "u64::MAX is quiet");
        assert_eq!(r.admit(), None, "saturated: stays quiet, no wrap to 1");
        assert_eq!(r.admit(), None, "still saturated");
        assert_eq!(r.seen(), u64::MAX);
    }

    #[test]
    fn event_zero_does_not_exist() {
        // `admitted` is public: a caller passing a raw 0 must not get a line for a non-event.
        assert!(!admitted(0));
        assert!(admitted(1));
    }

    #[test]
    fn a_saturated_counter_never_restarts_its_first_lines() {
        let r = RateLog {
            seen: AtomicU64::new(u64::MAX),
        };
        let written = (0..100).filter(|_| r.admit().is_some()).count();
        assert_eq!(
            written, 0,
            "a wrapped counter would write its 'first 8' again"
        );
        assert_eq!(r.seen(), u64::MAX);
    }

    #[test]
    fn each_macro_site_has_its_own_counter() {
        fn site_a() {
            crate::rate_eprintln!("ratelog test site a");
        }
        for _ in 0..3 {
            site_a();
        }
        crate::rate_eprintln!("ratelog test site b {}", 1);
    }
}
