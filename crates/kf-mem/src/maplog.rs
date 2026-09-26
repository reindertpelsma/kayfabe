//! ★ `KF3_MAPLOG=1` — a **diagnostic timeline of the VA plane** (default OFF; read once).
//!
//! Every walk is logged with the wants that triggered it (which invalidate — its PDB, `ALL_PDB`,
//! `ALL_VA` — which `MEM_OP` split and the channel that issued it, which root statement), the
//! objects each want named, and every diff run applied with its verdict; the channel plane adds its
//! RC detections and doorbell counts on the same clock. It exists to answer ONE kind of question:
//! *"when the host faulted at VA X, which statement of the guest's had we applied there, and who
//! asked for it?"* — never a decision input.
//!
//! ⊘ The clock is `std::time::Instant` (CLOCK_MONOTONIC) printed as seconds, anchored once to
//! `/proc/uptime` so a line can be placed near a host `dmesg` line. The anchor is ±10 ms and the
//! kernel's printk clock is not NTP-slewed, so cross-log ordering at the millisecond is NOT
//! claimed; ordering WITHIN this process's lines is exact.

use std::sync::OnceLock;
use std::time::Instant;

/// Whether `KF3_MAPLOG` is set (read once).
#[must_use]
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KF3_MAPLOG").is_some())
}

/// Seconds since host boot (see the module note on precision).
#[must_use]
pub fn t() -> f64 {
    static ANCHOR: OnceLock<(Instant, f64)> = OnceLock::new();
    let (at, up) = ANCHOR.get_or_init(|| {
        let up = std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next().and_then(|w| w.parse::<f64>().ok()))
            .unwrap_or(0.0);
        (Instant::now(), up)
    });
    up + at.elapsed().as_secs_f64()
}

/// ⊘⊘ `KF3_DIAG_UNMAP_DELAY_MS=N` — a DIAGNOSTIC A/B only (default off): hold every diff that
/// unmaps for N ms before applying it (see its one use in `vasmgr`). Never a fix: it blocks the VA
/// thread and only moves a race.
#[must_use]
pub fn unmap_delay_ms() -> Option<u64> {
    static MS: OnceLock<Option<u64>> = OnceLock::new();
    *MS.get_or_init(|| std::env::var("KF3_DIAG_UNMAP_DELAY_MS").ok().and_then(|v| v.parse().ok()).filter(|&n| n > 0))
}
