//! Default-off, status-preserving completion message diagnostic. Probe timer only.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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
    pub(crate) fn parse(period: Option<&str>, mode: Option<&str>) -> Result<Option<Self>, Invalid> {
        let Some(period) = period else {
            return Ok(None);
        };
        let period_ms = period.trim().parse::<u16>().map_err(|_| Invalid::Period)?;
        if period_ms == 0 {
            return Ok(None);
        }
        if period_ms > 1000 {
            return Err(Invalid::Period);
        }
        match mode {
            None | Some("all") => Ok(Some(Self { period_ms })),
            Some("owed") => Err(Invalid::OwedUnavailable),
            Some(_) => Err(Invalid::Mode),
        }
    }
}
pub(crate) struct Storm {
    period_ns: u64,
    emitting: AtomicBool,
    next_ns: AtomicU64,
    raises: AtomicU64,
    skipped_masked: AtomicU64,
}
// Single non-spinning emission owner, never acquired by a trap handler.
struct Emission<'a>(&'a AtomicBool);
impl Drop for Emission<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Storm {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            period_ns: u64::from(config.period_ms) * 1_000_000,
            emitting: AtomicBool::new(false),
            next_ns: AtomicU64::new(0),
            raises: AtomicU64::new(0),
            skipped_masked: AtomicU64::new(0),
        }
    }
    pub(crate) fn tick(
        &self,
        clock: impl Fn() -> u64,
        gate: &crate::reset::Gate,
        tree: &kf_trap::cpuintr::CpuIntr,
        vectors: impl IntoIterator<Item = u32>,
        send: impl FnOnce() -> bool,
    ) {
        let Some(_pass) = gate.enter() else { return };
        if self
            .emitting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let _emission = Emission(&self.emitting);
        let now_ns = clock();
        let Some(next) = now_ns.checked_add(self.period_ns) else {
            return;
        };
        let was = self.next_ns.load(Ordering::Acquire);
        if now_ns < was
            || self
                .next_ns
                .compare_exchange(was, next, Ordering::AcqRel, Ordering::Relaxed)
                .is_err()
        {
            return;
        }
        // One shared MSI-X message covers all selected logical vectors. Deduplicate the
        // host-authored vector list and bound both storage and inspection independently.
        let mut seen = [0u64; kf_trap::cpuintr::MAX_LEAF / 2];
        let mut eligible = false;
        for v in vectors.into_iter().take(kf_trap::cpuintr::MAX_LEAF * 32) {
            let Some(word) = seen.get_mut(v as usize / 64) else {
                continue;
            };
            let bit = 1u64 << (v % 64);
            if *word & bit != 0 {
                continue;
            }
            *word |= bit;
            match tree.diagnostic_message(v) {
                kf_trap::cpuintr::Raise::Message => eligible = true,
                kf_trap::cpuintr::Raise::None => {
                    self.skipped_masked.fetch_add(1, Ordering::Relaxed);
                }
                kf_trap::cpuintr::Raise::OutOfRange => {}
            }
        }
        let sent = eligible && send();
        // Rebase after the actual write/inspection, not its reservation: a descheduled
        // sender cannot create a catch-up burst when it resumes. No overlap is allowed.
        let finished = clock().max(now_ns);
        self.next_ns
            .store(finished.saturating_add(self.period_ns), Ordering::Release);
        if sent {
            self.raises.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(crate) fn summary(&self) -> String {
        format!(
            " irq_storm[mode=all period_ms={} storm_raises={} storm_skipped_masked={}]",
            self.period_ns / 1_000_000,
            self.raises.load(Ordering::Relaxed),
            self.skipped_masked.load(Ordering::Relaxed)
        )
    }
}
/// Share the existing probe timer without changing work discovery or relay park policy.
pub(crate) fn timer_interval(probe: bool, storm: Option<&Storm>) -> Option<std::time::Duration> {
    let diagnostic = storm.map(|s| std::time::Duration::from_nanos(s.period_ns));
    match (probe, diagnostic) {
        (false, interval) => interval,
        (true, Some(interval)) => Some(interval.min(std::time::Duration::from_millis(100))),
        (true, None) => Some(std::time::Duration::from_millis(100)),
    }
}
/// Host startup configuration only. No guest register can change the diagnostic period.
pub(crate) fn from_env() -> Option<Storm> {
    let period = std::env::var("KF3_DIAG_IRQ_STORM_MS").ok();
    let mode = std::env::var("KF3_DIAG_IRQ_STORM_MODE").ok();
    match Config::parse(period.as_deref(), mode.as_deref()) {
        Ok(Some(config)) => {
            eprintln!(
                "kf3: DIAGNOSTIC IRQ storm mode=all period_ms={} shared MSI-X0; status unchanged",
                config.period_ms
            );
            Some(Storm::new(config))
        }
        Ok(None) => None,
        Err(reason) => {
            eprintln!("kf3: DIAGNOSTIC IRQ storm disabled: {reason:?}");
            None
        }
    }
}
#[cfg(test)]
#[path = "irqstorm_tests.rs"]
mod tests;
