//! 2026-10-20 -- **`PROGRESS_STRAND`**, the hardware endpoint of `docs/design/V3_USER_RING_EVENT_LOOP_20261014.md` section 10.0. MEASUREMENT ONLY,
//! default off (`KF3_DIAG_PROGRESS=<ms>`, the name the baton harness already sets).
//!
//! A relay is STRANDED when it holds an obligation (a demand `retry`, a recorded private-word wait, or an unannounced obligation) that no turn
//! and no look has touched for `>= <ms>` WHILE a worker kept iterating (the loop counters moved). In the sleep-safe design that cannot happen:
//! every iteration evaluates every waiting relay, a lost lock is rung by its holder, and an unannounced obligation keeps a worker spinning. A
//! parked worker with a relay waiting on a marker interrupt is NOT a strand (no iteration happened), so it is not reported.
//!
//! Lines (greppable; `PROGRESS_STRAND_END` carries the total):
//! `kf3: PROGRESS_STRAND for_ms=.. relay=.. host=.. pending=.. waiting_flag=.. unannounced=.. handoff_waiters=.. iters=..`
//! `kf3: PROGRESS_STRAND_END host=.. total_ms=..`
//! The endpoint: NO `PROGRESS_STRAND` line on the Linux baton reproducer.

use std::sync::OnceLock;
use std::sync::atomic::Ordering;
use std::time::Instant;

/// The threshold in ms, `None` = off. Read once.
pub(super) fn threshold_ms() -> Option<u64> {
    static MS: OnceLock<Option<u64>> = OnceLock::new();
    *MS.get_or_init(|| {
        std::env::var("KF3_DIAG_PROGRESS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&ms| ms > 0)
    })
}

/// Monotonic ns since the first call, never 0 (0 = "never touched").
pub(super) fn now_ns() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    let t0 = *START.get_or_init(Instant::now);
    u64::try_from(t0.elapsed().as_nanos())
        .unwrap_or(u64::MAX - 1)
        .saturating_add(1)
}

/// What a worker did since the start: bumped once per loop iteration by one of these (served pass, spin, doorbell / event wake).
fn iterations(s: &kf_chan::user_loop::Stats) -> u64 {
    [
        &s.busy_looks,
        &s.unannounced_spins,
        &s.wake_doorbell,
        &s.wake_event,
        &s.wake_spin,
    ]
    .iter()
    .map(|c| c.load(Ordering::Relaxed))
    .sum()
}

struct Seen {
    touched: u64,
    since: Instant,
    iters_then: u64,
    announced: bool,
}

impl super::ChanPlane {
    /// Start the diagnostic thread when `KF3_DIAG_PROGRESS` is set. Never takes a lock a worker or a vCPU waits on (`try_lock` only).
    pub(super) fn start_strand_diag(&'static self) {
        let Some(ms) = threshold_ms() else { return };
        let tick = std::time::Duration::from_millis((ms / 4).clamp(20, 250));
        let _ = std::thread::Builder::new()
            .name("kf3-strand-diag".into())
            .spawn(move || {
                let mut seen: std::collections::HashMap<u32, Seen> = std::collections::HashMap::new();
                let mut lines = 0u32;
                eprintln!("kf3: PROGRESS_STRAND diagnostic on (threshold {ms} ms)");
                loop {
                    std::thread::sleep(tick);
                    let iters = iterations(&self.relay_stats);
                    let Ok(rings) = self.user_rings.try_lock() else { continue };
                    let now = Instant::now();
                    let mut live: Vec<u32> = Vec::new();
                    for (&token, rec) in rings.iter() {
                        let obligation =
                            rec.pending() || rec.is_waiting() || rec.is_unannounced();
                        if !obligation {
                            continue;
                        }
                        live.push(token);
                        let touched = rec.last_touch_ns();
                        let e = seen.entry(token).or_insert(Seen {
                            touched,
                            since: now,
                            iters_then: iters,
                            announced: false,
                        });
                        if e.touched != touched {
                            // somebody evaluated or served it: the strand clock restarts
                            if e.announced && lines < 128 {
                                lines = lines.saturating_add(1);
                                eprintln!(
                                    "kf3: PROGRESS_STRAND_END host={:#x} total_ms={:#x}",
                                    rec.idx,
                                    now.duration_since(e.since).as_millis()
                                );
                            }
                            *e = Seen {
                                touched,
                                since: now,
                                iters_then: iters,
                                announced: false,
                            };
                            continue;
                        }
                        let for_ms = now.duration_since(e.since).as_millis();
                        if !e.announced
                            && for_ms >= u128::from(ms)
                            && iters > e.iters_then
                            && lines < 64
                        {
                            e.announced = true;
                            lines = lines.saturating_add(1);
                            eprintln!(
                                "kf3: PROGRESS_STRAND for_ms={for_ms:#x} relay={token:#x} host={:#x} pending={} waiting_flag={} unannounced={} handoff_waiters={:#x} iters={:#x}",
                                rec.idx,
                                yn(rec.pending()),
                                yn(rec.is_waiting()),
                                yn(rec.is_unannounced()),
                                self.handoff.waiters(),
                                iters.saturating_sub(e.iters_then),
                            );
                        }
                    }
                    seen.retain(|token, e| {
                        if live.contains(token) {
                            return true;
                        }
                        if e.announced && lines < 128 {
                            lines = lines.saturating_add(1);
                            eprintln!(
                                "kf3: PROGRESS_STRAND_END host={token:#x} total_ms={:#x}",
                                now.duration_since(e.since).as_millis()
                            );
                        }
                        false
                    });
                }
            });
    }
}

fn yn(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The loop counters the watcher calls "a worker kept iterating".
    #[test]
    fn iterations_sum_the_loop_counters() {
        let s = kf_chan::user_loop::Stats::default();
        assert_eq!(iterations(&s), 0);
        s.busy_looks.fetch_add(3, Ordering::Relaxed);
        s.unannounced_spins.fetch_add(4, Ordering::Relaxed);
        s.wake_event.fetch_add(5, Ordering::Relaxed);
        assert_eq!(iterations(&s), 12);
    }

    /// Never 0 (0 means "never touched"), monotonic, and the stamp is a no-op with the diagnostic off.
    #[test]
    fn stamps_are_never_zero_and_off_means_off() {
        let a = now_ns();
        let b = now_ns();
        assert!(a >= 1 && b >= a);
        if threshold_ms().is_none() {
            let c = super::super::userrelay::Cell::new(1, ());
            c.touch();
            assert_eq!(c.last_touch_ns(), 0, "the diagnostic is off: no stamp");
        }
    }
}
