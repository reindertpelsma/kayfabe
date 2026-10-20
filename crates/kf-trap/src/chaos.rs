//! Test-only schedule-chaos hook points (feature `chaos-hooks`; absent from every normal build).
//!
//! A hook is a plain `fn(Point)` the stress harness registers once per process; it may yield, spin or sleep to widen a race
//! window. It never changes a decision: every call site is `#[cfg(feature = "chaos-hooks")] kf_trap::chaos::at(..)`
//! between two statements that already exist.

use std::sync::OnceLock;

/// Where in production code a hook sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Point {
    /// `TrapPath::doorbell`: the token word is RUNG, the bitmap bit is not yet published.
    DoorbellAfterRing,
    /// `TrapPath::doorbell`: the bit is published, the wake word is not yet bumped.
    DoorbellAfterPublish,
    /// `Plane::worker_pass`: the scan returned this token, the claim CAS has not run.
    ScanToClaim,
    /// `Plane::worker_pass`: the claim was won, the host has not been asked yet.
    AfterClaim,
    /// `worker::run_with_relay`: `seen` was loaded, the pass has not run.
    WorkerAfterSeen,
    /// `worker::run_with_relay`: nothing to do, `try_park` has not run.
    WorkerBeforePark,
    /// `worker::run_with_relay`: the park wait returned, `unpark` has not run.
    WorkerAfterWait,
}

static HOOK: OnceLock<fn(Point)> = OnceLock::new();

/// Register the process-wide hook (the first call wins).
pub fn set_hook(f: fn(Point)) {
    let _ = HOOK.set(f);
}

/// Run the hook, if one is registered.
#[inline]
pub fn at(p: Point) {
    if let Some(f) = HOOK.get() {
        f(p);
    }
}
