// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Walk completion policy, separate from the CUDA binding for GPU-free ordering tests.

pub(crate) fn ready<E>(
    _signalled: bool,
    query: impl FnOnce() -> Result<bool, E>,
) -> Result<bool, E> {
    // An fd can carry a stale wake. Only the current GPU event proves completion.
    query()
}

pub(crate) const fn poll_ms(in_flight: bool) -> u32 {
    if in_flight { 1 } else { 50 }
}

/// The shortest wait between two looks at a pending walk: never a spin.
pub(crate) const PENDING_MIN_US: u32 = 50;
/// The longest: a long walk is still looked at every millisecond (the pre-2026-11-03 rate).
pub(crate) const PENDING_MAX_US: u32 = 1_000;
/// No walk in flight: the idle backstop.
pub(crate) const IDLE_US: u32 = 50_000;

/// ★ 2026-11-03 (Ada lane): how long the VA worker may sleep before it next queries a pending
/// walk's GPU event, given how long ago the walk was submitted (`None`: nothing in flight).
///
/// No host callback signals a walk (the R34 per-walk nonstall interrupt, `fc5c83379`), so the
/// only way to see a finished walk is to look. A fixed 1 ms look added ~0.9 ms to every 85 us
/// walk. Here the wait is half the walk's age, clamped to 50 us..1 ms: a walk is collected
/// within about 1.5x its own time (plus 50 us), and a long one costs a logarithmic number of
/// wakes before settling at one per millisecond. A look is a hint to query, never a proof
/// ([`ready`] decides).
pub(crate) fn poll_us(pending_for: Option<std::time::Duration>) -> u32 {
    let Some(age) = pending_for else {
        return IDLE_US;
    };
    let half = u32::try_from(age.as_micros() / 2).unwrap_or(PENDING_MAX_US);
    half.clamp(PENDING_MIN_US, PENDING_MAX_US)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::time::Duration;

    #[test]
    fn a_signal_cannot_complete_a_pending_gpu_event() {
        let queries = Cell::new(0);
        for signalled in [true, false, true] {
            let observed: Result<bool, ()> = ready(signalled, || {
                queries.set(queries.get() + 1);
                Ok(false)
            });
            assert_eq!(observed, Ok(false));
        }
        assert_eq!(queries.get(), 3);
    }

    #[test]
    fn a_signal_cannot_hide_a_gpu_failure() {
        assert_eq!(
            ready(true, || Err::<bool, _>("CUDA_ERROR_ILLEGAL_ADDRESS")),
            Err("CUDA_ERROR_ILLEGAL_ADDRESS")
        );
    }

    #[test]
    fn a_completed_event_needs_no_callback_signal() {
        assert_eq!(ready(false, || Ok::<_, ()>(true)), Ok(true));
    }

    #[test]
    fn a_pending_event_needs_another_look() {
        assert_eq!(ready(false, || Ok::<_, ()>(false)), Ok(false));
    }

    #[test]
    fn a_walk_is_checked_promptly_and_idle_keeps_the_backstop() {
        assert_eq!(poll_ms(true), 1);
        assert_eq!(poll_ms(false), 50);
    }

    fn us(n: u64) -> Duration {
        Duration::from_micros(n)
    }

    /// The VA worker's checks of a pending walk, simulated: the walk age at the first check
    /// at or after `done_us`, and how many checks (wakes) it took.
    fn collected_at(done_us: u64) -> (u64, u32) {
        let (mut t, mut checks) = (0u64, 0u32);
        while t < done_us {
            t += u64::from(poll_us(Some(us(t))));
            checks += 1;
        }
        (t, checks)
    }

    /// ★ Ada lane 2026-11-03: an 85-200 us walk was collected only when the 1 ms wait expired
    /// (`vat[walk_avg_us]` 1008-1011 us). The check follows the walk's own time scale.
    #[test]
    fn a_fresh_walk_is_checked_on_its_gpu_time_scale() {
        assert_eq!(poll_us(Some(Duration::ZERO)), 50);
        assert_eq!(poll_us(Some(us(99))), 50);
        assert_eq!(poll_us(Some(us(300))), 150);
        assert_eq!(poll_us(Some(us(5_000))), 1_000);
        // An 85 us Ada walk is seen by 100 us; a slower walk within about 1.5x of its time.
        assert!(collected_at(85).0 <= 100, "{:?}", collected_at(85));
        assert!(collected_at(200).0 <= 225, "{:?}", collected_at(200));
        assert!(collected_at(400).0 <= 600, "{:?}", collected_at(400));
    }

    /// Bounded both ways: never a spin (a zero or tiny wait), never slower than the old 1 ms
    /// check, and a long walk costs a bounded number of wakes.
    #[test]
    fn the_pending_check_never_spins_and_never_exceeds_a_millisecond() {
        for age in (0..20_000).step_by(7) {
            let w = poll_us(Some(us(age)));
            assert!(
                (PENDING_MIN_US..=PENDING_MAX_US).contains(&w),
                "age {age} us -> {w} us"
            );
        }
        assert!(collected_at(1_000).1 <= 10, "{:?}", collected_at(1_000));
        assert!(collected_at(10_000).1 <= 20, "{:?}", collected_at(10_000));
    }

    #[test]
    fn idle_keeps_the_fifty_millisecond_backstop() {
        assert_eq!(poll_us(None), 50_000);
    }
}
