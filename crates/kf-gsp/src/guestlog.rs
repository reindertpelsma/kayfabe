//! Shared admission for guest-driven diagnostic lines. Refused lines do not format.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const PER_SECOND: u64 = 32;
const COUNT_BITS: u32 = 6;
const COUNT_MASK: u64 = (1 << COUNT_BITS) - 1;

struct Limiter {
    state: AtomicU64,
    suppressed: AtomicU64,
}
impl Limiter {
    const fn new() -> Self {
        Self {
            state: AtomicU64::new(0),
            suppressed: AtomicU64::new(0),
        }
    }
    fn admit_at(&self, seconds: u64) -> Option<u64> {
        let before = self.state.load(Ordering::Relaxed);
        let old_epoch = before >> COUNT_BITS;
        // A backwards clock cannot replenish admission; clamp before packing.
        let epoch = seconds.min(u64::MAX >> COUNT_BITS).max(old_epoch);
        let used = if epoch == old_epoch {
            before & COUNT_MASK
        } else {
            0
        };
        if used >= PER_SECOND
            || self
                .state
                .compare_exchange(
                    before,
                    (epoch << COUNT_BITS) | (used + 1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_err()
        {
            self.suppressed.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        Some(self.suppressed.swap(0, Ordering::Relaxed))
    }
}
static LIMITER: Limiter = Limiter::new();

/// Macro implementation: one bounded atomic admission attempt. Admitted
/// suppression summaries use the existing stderr mechanism.
#[doc(hidden)]
#[must_use]
pub fn admit() -> bool {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let Some(suppressed) = LIMITER.admit_at(seconds) else {
        return false;
    };
    if suppressed != 0 {
        eprintln!("kf3: guest diagnostics suppressed={suppressed}");
    }
    true
}

/// Emit a guest-driven diagnostic under the process-wide budget. Arguments are
/// evaluated only after admission. At most 32 payloads and 32 suppression summaries
/// are emitted in a clock second, shared across every participating call site.
#[macro_export]
macro_rules! guest_log {
    ($($args:tt)*) => {
        if $crate::guestlog::admit() {
            eprintln!($($args)*);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_replenishes_only_in_a_later_second() {
        let limiter = Limiter::new();
        for _ in 0..PER_SECOND {
            assert_eq!(limiter.admit_at(10), Some(0));
        }
        assert_eq!(limiter.admit_at(10), None);
        assert_eq!(limiter.admit_at(9), None);
        assert_eq!(limiter.admit_at(11), Some(2));
    }
    #[test]
    fn two_sites_share_the_same_admission_inventory() {
        let limiter = Limiter::new();
        for _ in 0..PER_SECOND / 2 {
            assert_eq!(limiter.admit_at(100), Some(0));
            assert_eq!(limiter.admit_at(100), Some(0));
        }
        assert_eq!(limiter.admit_at(100), None);
    }
}
