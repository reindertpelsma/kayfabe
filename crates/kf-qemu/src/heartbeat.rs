//! Clock-domain selection for the GSP heartbeat, sampled only on the drainer.
//! Guest RM adds its own system-timer offset to this raw millisecond value.

pub(crate) fn sample(
    guest_timer_ns: impl FnOnce() -> Option<u64>,
    _host_usermode_ns: impl FnOnce() -> Option<u64>,
) -> Option<u32> {
    // Use the very page guest RM reads. NV_VIRTUAL_FUNCTION_TIME may already
    // include the system offset that FSP guests add to the heartbeat themselves.
    // The other reader remains lazy and is never a fallback for a missing clock.
    guest_timer_ns().map(|ns| (ns / 1_000_000) as u32)
}

#[cfg(test)]
mod tests {
    use super::sample;
    use std::cell::Cell;

    #[test]
    fn fsp_software_offset_is_applied_once_by_the_guest() {
        // Synthetic separate epochs; public595 _kgspIsHeartbeatTimedOut formula.
        let raw_gpu_ms = 20_000u32;
        let system_offset_ms = 7_980_000u32;
        let current_ms = raw_gpu_ms.wrapping_add(system_offset_ms);
        let beat = sample(
            || Some(u64::from(raw_gpu_ms) * 1_000_000),
            || Some(u64::from(current_ms) * 1_000_000),
        )
        .unwrap();
        let age = current_ms.wrapping_sub(beat.wrapping_add(system_offset_ms));
        assert!(
            age <= 5200,
            "a second system offset creates a false timeout: {age}"
        );
    }

    #[test]
    fn only_the_validated_guest_timer_is_read() {
        let guest_reads = Cell::new(0);
        let other_reads = Cell::new(0);
        let beat = sample(
            || {
                guest_reads.set(guest_reads.get() + 1);
                Some(42_000_000)
            },
            || {
                other_reads.set(other_reads.get() + 1);
                Some(99_000_000)
            },
        );
        assert_eq!(beat, Some(42));
        assert_eq!(guest_reads.get(), 1);
        assert_eq!(other_reads.get(), 0);
    }

    #[test]
    fn unavailable_guest_clock_never_falls_back_to_a_different_epoch() {
        assert_eq!(sample(|| None, || Some(99_000_000)), None);
    }

    #[test]
    fn host_usermode_clock_failure_does_not_remove_a_valid_guest_clock() {
        assert_eq!(sample(|| Some(42_000_000), || None), Some(42));
    }

    #[test]
    fn millisecond_wrap_matches_unsigned_guest_arithmetic() {
        let ms = u64::from(u32::MAX) + 7;
        assert_eq!(
            sample(|| Some(ms * 1_000_000 + 999_999), || Some(0)),
            Some(6)
        );
    }

    #[test]
    fn equal_epochs_remain_equal() {
        assert_eq!(sample(|| Some(42_000_000), || Some(42_000_000)), Some(42));
    }
}
