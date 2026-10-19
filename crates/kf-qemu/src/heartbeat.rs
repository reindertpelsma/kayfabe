//! Heartbeat clock seams. Initial extraction preserves the candidate-5 formula.
use kf_abi::DriverVersion;
use kf_gsp::{StashedSystemInfo, SystemInfoCell};
use std::sync::Mutex;

pub(crate) fn offset_ns(_version: DriverVersion, _info: &StashedSystemInfo) -> Option<u64> {
    Some(0)
}

pub(crate) fn sample(
    system_timer_ns: impl FnOnce() -> Option<u64>,
    _guest_offset_ns: impl FnOnce() -> Option<u64>,
) -> Option<u32> {
    Some((system_timer_ns()? / 1_000_000) as u32)
}

// Not wired to the drainer until the following failing-first topic is fixed.
pub(crate) fn try_sample<S>(
    state: &Mutex<S>,
    info: &SystemInfoCell,
    current: impl FnOnce(&S) -> Option<DriverVersion>,
    clock: impl FnOnce() -> Option<u64>,
) -> Option<u32> {
    let guard = state.lock().ok()?;
    let version = current(&guard)?;
    info.try_with_latest(|body| sample(clock, || offset_ns(version, body)))?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v() -> DriverVersion {
        DriverVersion {
            major: 595,
            minor: 91,
            patch: 7,
        }
    }
    #[test]
    fn public595_formula_applies_guest_offset_once() {
        let offset = 7_980_000u64 * 1_000_000;
        let current = 8_000_000u64 * 1_000_000;
        let beat = sample(|| Some(current), || Some(offset)).unwrap();
        assert_eq!(
            ((current / 1_000_000) as u32)
                .wrapping_sub(beat.wrapping_add((offset / 1_000_000) as u32)),
            0
        );
    }
    #[test]
    fn round_before_subtracting_like_guest_not_after() {
        assert_eq!(sample(|| Some(10_100_000), || Some(3_900_000)), Some(7));
    }
    #[test]
    fn millisecond_wrap_is_unsigned_in_both_operands() {
        let ms = u64::from(u32::MAX) + 7;
        assert_eq!(sample(|| Some(ms * 1_000_000), || Some(3_000_000)), Some(3));
    }
    #[test]
    fn zero_offset_preserves_older_family_clock() {
        assert_eq!(sample(|| Some(42_000_000), || Some(0)), Some(42));
    }
    #[test]
    fn missing_offset_never_publishes_wrong_epoch() {
        assert_eq!(sample(|| Some(42_000_000), || None), None);
    }
    #[test]
    fn missing_clock_never_forges_freshness() {
        assert_eq!(sample(|| None, || Some(0)), None);
    }
    #[test]
    fn offset_is_decoded_at_every_exact_compiled_tag() {
        for &version in kf_abi::generated::matrix::MEASURED {
            let layout = kf_abi::generated::matrix::GSPSYSTEMINFO
                .at(version)
                .unwrap()
                .unwrap();
            let f = layout.field("sysTimerOffsetNs").unwrap();
            let mut info = StashedSystemInfo {
                declared_len: layout.size(),
                bytes: vec![0; layout.size()],
                sequence: 1,
            };
            let start = f.off as usize;
            info.bytes[start..start + 8].copy_from_slice(&u64::MAX.to_le_bytes());
            assert_eq!(offset_ns(version, &info), Some(u64::MAX));
        }
    }
    #[test]
    fn truncated_or_wrong_size_or_unknown_abi_is_refused() {
        let layout = kf_abi::generated::matrix::GSPSYSTEMINFO
            .at(v())
            .unwrap()
            .unwrap();
        let mut info = StashedSystemInfo {
            declared_len: layout.size(),
            bytes: vec![0; layout.size()],
            sequence: 1,
        };
        assert_eq!(offset_ns(v(), &info), Some(0));
        info.declared_len += 1;
        assert_eq!(offset_ns(v(), &info), None);
        info.declared_len -= 1;
        info.bytes.pop();
        assert_eq!(offset_ns(v(), &info), None);
        assert_eq!(
            offset_ns(
                DriverVersion {
                    major: 595,
                    minor: 91,
                    patch: 8
                },
                &info
            ),
            None
        );
    }
    #[test]
    fn hostile_offset_extremes_are_bounded_clock_arithmetic() {
        let beat = sample(|| Some(0), || Some(u64::MAX)).unwrap();
        assert_eq!(beat.wrapping_add((u64::MAX / 1_000_000) as u32), 0);
    }

    #[test]
    fn fractional_operands_are_rounded_separately_again() {
        assert_eq!(sample(|| Some(20_000_001), || Some(8_999_999)), Some(12));
    }
    #[test]
    fn absent_offset_does_not_even_read_the_clock() {
        assert_eq!(
            sample(|| panic!("clock called without metadata"), || None),
            None
        );
    }
    #[test]
    fn second_failed_clock_cannot_publish_freshness() {
        assert_eq!(sample(|| None, || Some(u64::MAX)), None);
    }
    #[test]
    fn nonzero_offset_decodes_the_guest_body_not_a_default() {
        let layout = kf_abi::generated::matrix::GSPSYSTEMINFO
            .at(v())
            .unwrap()
            .unwrap();
        let start = layout.field("sysTimerOffsetNs").unwrap().off as usize;
        let mut info = StashedSystemInfo {
            declared_len: layout.size(),
            bytes: vec![0; layout.size()],
            sequence: 7,
        };
        info.bytes[start..start + 8].copy_from_slice(&9_123_456u64.to_le_bytes());
        assert_eq!(offset_ns(v(), &info), Some(9_123_456));
    }
    #[test]
    fn wrong_declaration_never_supplies_an_offset() {
        let layout = kf_abi::generated::matrix::GSPSYSTEMINFO
            .at(v())
            .unwrap()
            .unwrap();
        let info = StashedSystemInfo {
            declared_len: layout.size() + 8,
            bytes: vec![0; layout.size()],
            sequence: 1,
        };
        assert_eq!(offset_ns(v(), &info), None);
    }
    fn state_contention(read_clock: bool) {
        use std::sync::{Arc, mpsc};
        use std::time::Duration;
        let state = Arc::new(Mutex::new(v()));
        let held = state.lock().unwrap();
        let worker_state = state.clone();
        let info = SystemInfoCell::new();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let value = try_sample(
                &worker_state,
                &info,
                |version| Some(*version),
                || {
                    assert!(read_clock, "busy state must skip the clock");
                    Some(0)
                },
            );
            tx.send(value).unwrap();
        });
        let observed = rx.recv_timeout(Duration::from_millis(250));
        drop(held); // Always unblock a deliberately broken mutant before failing.
        worker.join().unwrap();
        assert_eq!(observed, Ok(None), "heartbeat waited on the state lock");
    }
    #[test]
    fn busy_gsp_state_returns_without_waiting() {
        state_contention(true);
    }
    #[test]
    fn busy_gsp_state_never_enters_clock_read() {
        state_contention(false);
    }
    #[test]
    fn inactive_life_refuses_an_old_cell() {
        let state = Mutex::new(());
        let info = SystemInfoCell::new();
        info.store(&[0; 8], 1);
        assert_eq!(
            try_sample(&state, &info, |_| None, || panic!("inactive life")),
            None
        );
    }
    #[test]
    fn actual_drainer_uses_nonblocking_clock_path_at_500ms() {
        let src = include_str!("device.rs");
        let path = src
            .split("pub fn drainer_loop(&self)")
            .nth(1)
            .unwrap()
            .split("// ⚠ DIAGNOSTIC")
            .next()
            .unwrap();
        assert!(path.contains("crate::heartbeat::try_sample("));
        assert!(path.contains(".read_ns().ok()"));
        assert!(path.contains("from_millis(500)"));
        for blocking in ["gpu_time_ns(", ".lock(", ".latest("] {
            assert!(!path.contains(blocking), "{blocking}");
        }
    }
}
