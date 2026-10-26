//! Host-configured, bounded delay on the ordinary VA-manager apply thread.
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};
/// Diagnostic policy, never guest input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Requested delay, ms; zero is OFF.
    pub ms: u64,
    /// Minimum attempted diff runs in an apply.
    pub min_runs: usize,
}
impl Config {
    /// Strict bounded operator configuration; missing delay is OFF.
    pub fn parse(ms: Option<&str>, runs: Option<&str>) -> Result<Self, &'static str> {
        fn number(value: Option<&str>, default: u64) -> Result<u64, &'static str> {
            let Some(v) = value else { return Ok(default) };
            if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
                return Err("expected unsigned decimal operator configuration");
            }
            v.parse().map_err(|_| "operator configuration overflow")
        }
        let ms = number(ms, 0)?;
        let runs = number(runs, 1000)?;
        if ms > 5000 {
            return Err("apply delay exceeds 5000 ms");
        }
        if !(1..=1_000_000).contains(&runs) {
            return Err("run threshold outside 1..1000000");
        }
        Ok(Self {
            ms,
            min_runs: usize::try_from(runs).map_err(|_| "threshold overflow")?,
        })
    }
    /// Delay of this apply; does not alter gate policy.
    pub fn for_runs(self, runs: usize) -> u64 {
        if runs >= self.min_runs { self.ms } else { 0 }
    }
    /// Execute on the caller, with a fake sleeper seam for GPU-free checks.
    pub fn run(self, runs: usize, mut sleep: impl FnMut(u64)) -> u64 {
        let ms = self.for_runs(runs);
        if ms != 0 {
            sleep(ms);
        }
        ms
    }
}
/// Sample only host-operator environment; invalid input disables injection and names the error.
pub fn config() -> Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    *CONFIG.get_or_init(|| {
        let ms = std::env::var("KF3_DIAG_APPLY_DELAY_MS").ok();
        let runs = std::env::var("KF3_DIAG_APPLY_DELAY_MIN_RUNS").ok();
        Config::parse(ms.as_deref(), runs.as_deref()).unwrap_or_else(|why| {
            eprintln!("kf3: ENS4 apply delay DISABLED: {why}");
            Config {
                ms: 0,
                min_runs: 1000,
            }
        })
    })
}
static CALLS: AtomicU64 = AtomicU64::new(0);
static WALL_NS: AtomicU64 = AtomicU64::new(0);
/// Sleep on the VA-manager caller, before apply_entry and inside ordinary gate exposure.
/// Returns actual injected wall time; no clock or sleep when disabled/below threshold.
pub fn inject(runs: usize) -> u64 {
    let c = config();
    if c.for_runs(runs) == 0 {
        return 0;
    }
    let start = std::time::Instant::now();
    c.run(runs, |ms| {
        std::thread::sleep(std::time::Duration::from_millis(ms))
    });
    let ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
    let _ = CALLS.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
        Some(n.saturating_add(1))
    });
    let _ = WALL_NS.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
        Some(n.saturating_add(ns))
    });
    ns
}
/// Existing status-line fragment; counters are not decisions.
pub fn summary() -> String {
    let c = config();
    format!(
        "ens4_apply_delay_ms={} ens4_delay_min_runs={} ens4_delayed_applies={} \
        ens4_injected_wall_ns={}",
        c.ms,
        c.min_runs,
        CALLS.load(Ordering::Relaxed),
        WALL_NS.load(Ordering::Relaxed)
    )
}
