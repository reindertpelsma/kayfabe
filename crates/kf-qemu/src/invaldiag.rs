//! ⚠ DIAGNOSTIC (2026-10-10, TDR hunt; owner ruling §AD: no MMU invalidate may stay incomplete for longer than about a
//! dozen milliseconds): the GUEST-VISIBLE latency of every MMU invalidate — from the vCPU trap that armed the trigger
//! to the VA thread's publication of the idle word into the BAR0 read shadow (what the guest's spin-read sees).
//! Only when `KF3_COMPLETION_PROBE` is set; one `OnceLock` load per invalidate otherwise. Never a decision input.
//!
//! A re-arm while the trigger is still busy keeps the EARLIER arm time (the guest has seen "busy" continuously since it).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, OnceLock};

static ANCHOR: OnceLock<std::time::Instant> = OnceLock::new();
static ARM_NS: AtomicU64 = AtomicU64::new(0);
static REARMS: AtomicU64 = AtomicU64::new(0);
static SLOW: AtomicU64 = AtomicU64::new(0);
static PENDING_REPORTED: AtomicU64 = AtomicU64::new(0);
static HIST: LazyLock<crate::prof::Hist> = LazyLock::new(crate::prof::Hist::default);

/// The threshold of the owner ruling (12 ms).
pub const SLOW_NS: u64 = 12_000_000;

fn on() -> bool {
    crate::chan::completion_probe_ms().is_some()
}

fn now_ns() -> u64 {
    let a = ANCHOR.get_or_init(std::time::Instant::now);
    u64::try_from(a.elapsed().as_nanos()).unwrap_or(u64::MAX).max(1)
}

/// vCPU, in the trap: an invalidate armed the trigger.
pub fn armed() {
    if !on() {
        return;
    }
    if ARM_NS.compare_exchange(0, now_ns(), Ordering::AcqRel, Ordering::Acquire).is_err() {
        REARMS.fetch_add(1, Ordering::Relaxed);
    }
}

/// VA thread, right after it published the trigger word: `idle` = the guest now reads idle. `ctx` names what the VA
/// thread was doing; it is evaluated only for a slow one.
pub fn after_publish(idle: bool, ctx: impl FnOnce() -> String) {
    if !on() || !idle {
        return;
    }
    let a = ARM_NS.swap(0, Ordering::AcqRel);
    if a == 0 {
        return;
    }
    let d = now_ns().saturating_sub(a);
    HIST.add(d);
    if d > SLOW_NS {
        SLOW.fetch_add(1, Ordering::Relaxed);
        eprintln!(
            "kf3: INVAL-SLOW t={:.3} utc_ms={} guest-visible arm->idle {:.1} ms; {}",
            kf_mem::maplog::t(),
            crate::diagring::utc_ms_pub(),
            d as f64 / 1e6,
            ctx()
        );
    }
}

/// Probe thread: an invalidate still busy past [`SLOW_NS`] (named once per arm).
pub fn pending_line() -> Option<String> {
    let a = ARM_NS.load(Ordering::Acquire);
    if a == 0 {
        return None;
    }
    let age = now_ns().saturating_sub(a);
    if age <= SLOW_NS || PENDING_REPORTED.swap(a, Ordering::AcqRel) == a {
        return None;
    }
    Some(format!(
        "kf3: INVAL-PENDING t={:.3} utc_ms={} guest-visible busy for {:.1} ms (still armed)",
        kf_mem::maplog::t(),
        crate::diagring::utc_ms_pub(),
        age as f64 / 1e6
    ))
}

/// Probe thread: the distribution so far.
pub fn summary() -> String {
    format!(
        "kf3: INVAL-LAT t={:.3} guest-visible arm->idle {} slow_over_12ms={} rearms_while_busy={}",
        kf_mem::maplog::t(),
        HIST.line(),
        SLOW.load(Ordering::Relaxed),
        REARMS.load(Ordering::Relaxed)
    )
}
