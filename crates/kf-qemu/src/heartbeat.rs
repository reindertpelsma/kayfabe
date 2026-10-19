//! GSP heartbeat clock conversion, sampled only on the drainer.
//! Public595 kernel_gsp.c passes sysTimerOffsetNs in fn72 and adds its millisecond
//! value back when checking the heartbeat. PTIMER and VF_TIME are system clocks.
use kf_abi::DriverVersion;
use kf_gsp::StashedSystemInfo;

pub(crate) fn offset_ns(version: DriverVersion, info: &StashedSystemInfo) -> Option<u64> {
    let layout = kf_abi::generated::matrix::GSPSYSTEMINFO
        .at(version)
        .ok()??;
    if info.declared_len != layout.size() || info.bytes.len() != layout.size() {
        return None;
    }
    let field = layout.field("sysTimerOffsetNs")?;
    if field.size != 8 || field.elem != 0 {
        return None;
    }
    let start = usize::try_from(field.off).ok()?;
    Some(u64::from_le_bytes(
        info.bytes
            .get(start..start.checked_add(8)?)?
            .try_into()
            .ok()?,
    ))
}

pub(crate) fn sample(
    system_timer_ns: impl FnOnce() -> Option<u64>,
    guest_offset_ns: impl FnOnce() -> Option<u64>,
) -> Option<u32> {
    let offset_ms = (guest_offset_ns()? / 1_000_000) as u32;
    let system_ms = (system_timer_ns()? / 1_000_000) as u32;
    // Mirror _kgspIsHeartbeatTimedOut's separate divisions exactly: subtracting
    // nanoseconds first can place the millisecond heartbeat one tick in the future.
    Some(system_ms.wrapping_sub(offset_ms))
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
            (current / 1_000_000) as u32 - beat.wrapping_add((offset / 1_000_000) as u32),
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
}
