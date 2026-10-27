//! Default-off, status-preserving completion message diagnostic. Worker tick only.
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Config {
    period_ms: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Invalid {
    Period,
    Mode,
    OwedUnavailable,
}
impl Config {
    pub(crate) fn parse(
        _period: Option<&str>,
        _mode: Option<&str>,
    ) -> Result<Option<Self>, Invalid> {
        Ok(None)
    }
}
pub(crate) struct Storm {
    period_ns: u64,
    next_ns: AtomicU64,
    raises: AtomicU64,
    skipped_masked: AtomicU64,
}
impl Storm {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            period_ns: u64::from(config.period_ms) * 1_000_000,
            next_ns: AtomicU64::new(0),
            raises: AtomicU64::new(0),
            skipped_masked: AtomicU64::new(0),
        }
    }
    pub(crate) fn tick(
        &self,
        _now_ns: u64,
        _gate: &crate::reset::Gate,
        _tree: &kf_trap::cpuintr::CpuIntr,
        _vectors: impl IntoIterator<Item = u32>,
        _send: impl FnOnce() -> bool,
    ) {
    }
    pub(crate) fn summary(&self) -> String {
        String::new()
    }
}
#[cfg(test)]
#[path = "irqstorm_tests.rs"]
mod tests;
