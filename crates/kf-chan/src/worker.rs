//! ★★★★★ **The worker THREAD** — the OS half around `kf_core::Plane::worker_pass`.
//!
//! `kf-core` owns the plane (scan, claim, submit decision, §8 completion rule, release) and makes no
//! OS call. This is the loop a worker thread runs around it: pass; if nothing was served, park via
//! the wake word's `try_park(seen)` and wait in `epoll` on the worker eventfd and the session
//! completion fd. ★ A completion rings the in-flight channels' OWN tokens (`Plane::ring_internal`),
//! so it reaches the channel through the same claim path as a doorbell — never a direct call.
//! ⊘ There is exactly ONE worker loop in v3; this is it (it replaced a duplicate claim loop that
//! lived here at gate 4).

use crate::completions::Completions;
use kf_core::{HostOps, Plane};
use kf_linux_raw::{Notifier, PollTimeout, Poller, ReadyTokens};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The poller tag of the worker eventfd.
pub const WORKER_EFD_TAG: u64 = 0;
/// The poller tag of the session's completion fd ([`Completions`]).
pub const COMPLETIONS_TAG: u64 = 1 << 32;
const SCAN_LIMIT: usize = 64;
/// How long a parked worker sleeps before re-checking `stop` — never a correctness timeout.
const PARK_MS: u32 = 50;

/// Counters, for gates — never read on a hot path.
#[derive(Debug, Default)]
pub struct WorkerStats {
    /// Tokens served by `worker_pass`.
    pub served: AtomicU64,
    /// Times a worker parked in `epoll`.
    pub parks: AtomicU64,
    /// Completion wakes turned into internal rings.
    pub host_rings: AtomicU64,
}

/// One worker thread's loop, until `stop`. `poller` watches `efd` at [`WORKER_EFD_TAG`] and
/// `completions` at [`COMPLETIONS_TAG`].
pub fn run(
    plane: &Plane<'_>,
    host: &dyn HostOps,
    poller: &Poller,
    efd: &Notifier,
    completions: &Completions,
    stats: &WorkerStats,
    stop: &AtomicBool,
) {
    let mut scratch = Vec::with_capacity(SCAN_LIMIT);
    while !stop.load(Ordering::Acquire) {
        // §5.3: every `seen` load happens-before the scan.
        let seen = plane.worker_wake().seen();
        let n = plane.worker_pass(host, &mut scratch, SCAN_LIMIT);
        if n > 0 {
            stats.served.fetch_add(n as u64, Ordering::Relaxed);
            continue;
        }
        if !plane.worker_wake().try_park(seen) {
            continue;
        }
        stats.parks.fetch_add(1, Ordering::Relaxed);
        let mut ready = ReadyTokens::new();
        let got = poller.wait(&mut ready, PollTimeout::Millis(PARK_MS));
        plane.worker_wake().unpark();
        if got.is_ok() {
            for tag in ready.iter() {
                if tag == COMPLETIONS_TAG {
                    completions.for_each_inflight(|t| {
                        stats.host_rings.fetch_add(1, Ordering::Relaxed);
                        if plane.ring_internal(t) {
                            let _ = efd.signal();
                        }
                    });
                } else {
                    let _ = efd.drain();
                }
            }
        }
    }
}
