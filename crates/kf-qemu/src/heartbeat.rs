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
