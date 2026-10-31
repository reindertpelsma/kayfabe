//! The watchdog: a thread with two deadlines (one per phase, one for the whole run). On expiry it prints
//! the phase, refresh and batch the program was in and exits with 124, which every suite already scores
//! CRASH. It never takes a lock a vCPU takes and does no RM call: it reads a small struct under a
//! mutex that only the arm's own thread writes, at a phase boundary.
//!
//! The decision is a pure function ([`expired`]) so it is tested without threads; the thread and the
//! exit action are separate so a test can observe an expiry without the process exiting.

use crate::clock::Clock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Where the arm says it is. The arm calls [`Progress::enter`] at every phase and batch boundary.
pub trait Progress {
    /// Record that the arm entered `phase` (a short stable word) at `refresh` and `batch`.
    fn enter(&self, phase: &'static str, refresh: u32, batch: u32);
}

/// A [`Progress`] that records nothing (tests, and runs without a watchdog).
#[derive(Debug, Default, Clone, Copy)]
pub struct NullProgress;

impl Progress for NullProgress {
    fn enter(&self, _phase: &'static str, _refresh: u32, _batch: u32) {}
}

/// A snapshot of where the arm is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseInfo {
    /// The phase word.
    pub phase: &'static str,
    /// Refresh index.
    pub refresh: u32,
    /// Batch index.
    pub batch: u32,
    /// Clock reading when the phase was entered.
    pub entered_ns: u64,
}

/// Which deadline expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    /// Still within both.
    None,
    /// One phase lasted longer than its limit.
    Phase,
    /// The whole run lasted longer than its limit.
    Total,
}

/// The pure decision: has the phase entered at `info.entered_ns` outlasted `phase_limit_ns`, or has the run
/// that began at `start_ns` outlasted `total_limit_ns`, at `now_ns`? The total takes precedence.
#[must_use]
pub fn expired(
    info: &PhaseInfo,
    start_ns: u64,
    now_ns: u64,
    phase_limit_ns: u64,
    total_limit_ns: u64,
) -> Expiry {
    if now_ns.saturating_sub(start_ns) >= total_limit_ns {
        Expiry::Total
    } else if now_ns.saturating_sub(info.entered_ns) >= phase_limit_ns {
        Expiry::Phase
    } else {
        Expiry::None
    }
}

/// What to do on expiry.
pub type OnExpire = Box<dyn Fn(Expiry, &PhaseInfo, u64) + Send + 'static>;

/// The real action: print one `WATCHDOG` line and exit 124.
pub fn exit_on_expiry() -> OnExpire {
    Box::new(|why, info, waited_ns| {
        use std::io::Write;
        let kind = if why == Expiry::Total {
            "total"
        } else {
            "phase"
        };
        let mut out = std::io::stdout().lock();
        let _ = writeln!(
            out,
            "WATCHDOG limit={kind} phase={} refresh={} batch={} waited_ms={}",
            info.phase,
            info.refresh,
            info.batch,
            waited_ns / 1_000_000
        );
        let _ = out.flush();
        std::process::exit(i32::from(crate::outcome::EXIT_WATCHDOG));
    })
}

struct Shared {
    info: Mutex<PhaseInfo>,
    stop: AtomicBool,
}

/// A running watchdog. Dropping it stops the thread.
pub struct Watchdog {
    shared: Arc<Shared>,
    clock: Arc<dyn Clock + Send + Sync>,
    handle: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Watchdog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Watchdog")
    }
}

impl Watchdog {
    /// Start the thread. `poll` is how often it looks (50 ms is plenty for limits in seconds).
    pub fn start(
        clock: Arc<dyn Clock + Send + Sync>,
        phase_limit_ns: u64,
        total_limit_ns: u64,
        poll: Duration,
        on_expire: OnExpire,
    ) -> Watchdog {
        let start_ns = clock.now_ns();
        let shared = Arc::new(Shared {
            info: Mutex::new(PhaseInfo {
                phase: "start",
                refresh: 0,
                batch: 0,
                entered_ns: start_ns,
            }),
            stop: AtomicBool::new(false),
        });
        let (s2, c2) = (Arc::clone(&shared), Arc::clone(&clock));
        let handle = std::thread::spawn(move || {
            while !s2.stop.load(Ordering::Acquire) {
                std::thread::sleep(poll);
                if s2.stop.load(Ordering::Acquire) {
                    break;
                }
                let now = c2.now_ns();
                let info = match s2.info.lock() {
                    Ok(g) => *g,
                    Err(p) => *p.into_inner(),
                };
                match expired(&info, start_ns, now, phase_limit_ns, total_limit_ns) {
                    Expiry::None => {}
                    why => {
                        on_expire(why, &info, now.saturating_sub(info.entered_ns));
                        // A test action returns; the real one exits. Either way, stop looking.
                        break;
                    }
                }
            }
        });
        Watchdog {
            shared,
            clock,
            handle: Some(handle),
        }
    }
}

impl Progress for Watchdog {
    fn enter(&self, phase: &'static str, refresh: u32, batch: u32) {
        let now = self.clock.now_ns();
        let mut g = match self.shared.info.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        *g = PhaseInfo {
            phase,
            refresh,
            batch,
            entered_ns: now,
        };
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::MonotonicClock;
    use std::sync::atomic::AtomicU32;

    fn info(entered: u64) -> PhaseInfo {
        PhaseInfo {
            phase: "map",
            refresh: 1,
            batch: 2,
            entered_ns: entered,
        }
    }

    #[test]
    fn the_decision_is_exact_at_the_edges() {
        let i = info(1_000);
        assert_eq!(expired(&i, 0, 1_999, 1_000, 10_000), Expiry::None);
        assert_eq!(expired(&i, 0, 2_000, 1_000, 10_000), Expiry::Phase);
        assert_eq!(
            expired(&i, 0, 10_000, 1_000, 10_000),
            Expiry::Total,
            "total wins"
        );
        // A clock that is behind the entry (impossible, but) never underflows into an expiry.
        assert_eq!(expired(&i, 5_000, 0, 1_000, 10_000), Expiry::None);
    }

    #[test]
    fn a_stuck_phase_fires_once_with_its_name_and_progress_resets_it() {
        let fired = Arc::new(AtomicU32::new(0));
        let seen = Arc::new(Mutex::new(None));
        let (f2, s2) = (Arc::clone(&fired), Arc::clone(&seen));
        let wd = Watchdog::start(
            Arc::new(MonotonicClock::new()),
            60_000_000,
            10_000_000_000,
            Duration::from_millis(5),
            Box::new(move |why, info, _| {
                f2.fetch_add(1, Ordering::SeqCst);
                *s2.lock().unwrap() = Some((why, info.phase, info.refresh, info.batch));
            }),
        );
        // Keep entering phases faster than the limit: no expiry.
        for b in 0..10 {
            wd.enter("map", 0, b);
            std::thread::sleep(Duration::from_millis(15));
        }
        assert_eq!(fired.load(Ordering::SeqCst), 0);
        // Stall in one phase.
        wd.enter("invalidate", 2, 7);
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(
            fired.load(Ordering::SeqCst),
            1,
            "fires once, then the thread stops"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            Some((Expiry::Phase, "invalidate", 2, 7))
        );
        drop(wd);
    }

    #[test]
    fn dropping_stops_the_thread_without_firing() {
        let fired = Arc::new(AtomicU32::new(0));
        let f2 = Arc::clone(&fired);
        let wd = Watchdog::start(
            Arc::new(MonotonicClock::new()),
            10_000_000_000,
            10_000_000_000,
            Duration::from_millis(5),
            Box::new(move |_, _, _| {
                f2.fetch_add(1, Ordering::SeqCst);
            }),
        );
        drop(wd);
        assert_eq!(fired.load(Ordering::SeqCst), 0);
    }
}
