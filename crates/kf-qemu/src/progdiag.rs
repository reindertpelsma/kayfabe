//! ★ 2026-10-20 MEASUREMENT ONLY (`KF3_DIAG_PROGRESS=<ms>`, default off): the glue of the progress diagnostic
//! (`kf_chan::progress`, `docs/design/V3_USER_RING_EVENT_LOOP_20261014.md` section 11).
//!
//! A child of `userpump` (it reads the pump's private state; it only reads). It runs on ITS OWN thread (`kf3-progress`, started
//! only when the switch is on), never on a vCPU, the drainer or a worker: the worker tick would be silent exactly when the worker is
//! busy, which is the case under test. Every lock is a `try_lock` and none is held across the tick's sleep; a busy lock skips that
//! reading, never waits. The relay's recorded wait is evaluated with the SAME predicate and the SAME passive reads of the private
//! USERD that the worker's look uses (`user_ring_peek_one`); it never rings, never publishes and never touches guest memory.

use super::*;
use kf_chan::progress::{self, RelayBrief, RelayFacts, WaitKey};

/// What one relay's reading found.
struct Reading {
    brief: RelayBrief,
    /// Present when the record and its storage could be read this tick.
    wait_eval: Option<WaitEval>,
    last_look_ns: u64,
}

struct WaitEval {
    marks: u64,
    popped: u64,
    satisfied: bool,
}

impl ChanPlane {
    /// The diagnostic thread's body: one reading per tick until `stop`.
    pub(crate) fn progress_loop(&self, stop: &std::sync::atomic::AtomicBool) {
        let ms = progress::threshold_ms();
        if ms == 0 {
            return;
        }
        let thr_ns = ms.saturating_mul(1_000_000);
        let tick_ms = (ms / 4).clamp(5, 250);
        eprintln!(
            "kf3: PROGRESS_ARMED threshold_ms={ms:#x} tick_ms={tick_ms:#x} points={:#x} (KF3_DIAG_PROGRESS)",
            progress::POINTS
        );
        let mut tracker = progress::Tracker::new();
        let mut printer = progress::Printer::default();
        let first = progress::BOARD.snapshot(progress::now_ns(), progress::START_NS);
        let mut monitor = progress::Monitor::new(ms, &first);
        while !stop.load(std::sync::atomic::Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(tick_ms));
            let now = progress::now_ns();
            let snap = progress::BOARD.snapshot(now, progress::START_NS);
            let (readings, complete) = match self.progress_read_relays() {
                Some(r) => (r, true),
                // the relay map was busy this tick: judge nothing, forget nothing
                None => (Vec::new(), false),
            };
            let live: Vec<u64> = readings.iter().map(|r| u64::from(r.brief.host)).collect();
            if complete {
                tracker.retain(&live);
            }
            let counters = self.progress_counters();
            let mut threads: Option<Vec<progress::ThreadCpu>> = None;
            for r in &readings {
                let Some(eval) = &r.wait_eval else {
                    continue;
                };
                let id = u64::from(r.brief.host);
                let Some(age) = tracker.observe(
                    now,
                    id,
                    r.brief.wait,
                    eval.satisfied,
                    r.last_look_ns,
                    thr_ns,
                ) else {
                    continue;
                };
                let Some(wait) = r.brief.wait else {
                    continue;
                };
                match printer.block() {
                    progress::Admit::Yes => {
                        let facts = RelayFacts {
                            key: r.brief.key,
                            host: r.brief.host,
                            wait,
                            marks: eval.marks,
                            popped: eval.popped,
                            unpeeked_ms: age / 1_000_000,
                            last_peek_age_ms: (r.last_look_ns != 0)
                                .then(|| now.saturating_sub(r.last_look_ns) / 1_000_000),
                            threshold_ms: ms,
                            brief: r.brief.clone(),
                            counters: counters.clone(),
                        };
                        let t = threads.get_or_insert_with(read_threads);
                        for line in progress::render_relay_block(&facts, &snap, t) {
                            eprintln!("{line}");
                        }
                    }
                    progress::Admit::Suppress => {
                        eprintln!("{}", progress::suppression_line("blocks"));
                    }
                    progress::Admit::No => {}
                }
            }
            let mut printed_starved = false;
            for s in monitor.evaluate(&snap) {
                match printer.starved() {
                    progress::Admit::Yes => {
                        eprintln!("{}", progress::render_starved(&s, ms));
                        printed_starved = true;
                    }
                    progress::Admit::Suppress => {
                        eprintln!("{}", progress::suppression_line("starved"));
                    }
                    progress::Admit::No => {}
                }
            }
            if printed_starved {
                let briefs: Vec<RelayBrief> = readings.iter().map(|r| r.brief.clone()).collect();
                for line in progress::render_summary_context(&snap, &counters, &briefs) {
                    eprintln!("{line}");
                }
            }
        }
    }

    /// The worker-level counters the starvation hypotheses are decided by (relaxed loads of the production counters).
    fn progress_counters(&self) -> Vec<(&'static str, u64)> {
        let g = |c: &std::sync::atomic::AtomicU64| c.load(std::sync::atomic::Ordering::Relaxed);
        let rs = &self.relay_stats;
        vec![
            ("contended_turns", g(&rs.contended)),
            ("slot_contended", g(&self.contended)),
            ("wake_doorbell", g(&rs.wake_doorbell)),
            ("wake_event", g(&rs.wake_event)),
            ("wake_spin", g(&rs.wake_spin)),
            ("wake_backstop", g(&rs.wake_backstop)),
            ("busy_looks", g(&rs.busy_looks)),
            ("backstop_found_work", g(&rs.backstop_found_work)),
            ("backstop_carried", g(&rs.backstop_carried)),
            ("backstop_contended", g(&rs.backstop_contended)),
            ("event_without_progress", g(&rs.event_without_progress)),
            ("pop_writebacks", g(&rs.pop_writebacks)),
            ("credit_waits", g(&rs.credit_waits)),
            ("markers", g(&rs.markers)),
            ("gates", g(&rs.gates)),
        ]
    }

    /// One reading of every live relay: the atomics always, the recorded wait and the private words when the locks are free.
    fn progress_read_relays(&self) -> Option<Vec<Reading>> {
        let (walk_busy, walk_waiters) = self.inbox.walk_diag();
        let Ok(rings) = self.user_rings.try_lock() else {
            return None;
        };
        let mut out = Vec::new();
        for record in rings.values() {
            if record.is_canceled() || out.len() >= progress::MAX_RELAYS {
                continue;
            }
            let mut brief = RelayBrief {
                key: (0, 0),
                host: record.idx,
                wait: None,
                contended_run: record.contended_run(),
                retry: record.retry.load(std::sync::atomic::Ordering::Acquire),
                pending: record.pending(),
                waiting: record.is_waiting(),
                canceled: record.is_canceled(),
                stopped: None,
                disabled: None,
                walk_busy,
                walk_waiters,
            };
            let mut wait_eval = None;
            if let Ok(r) = record.try_lock()
                && !r.retired
                && !r.free_attempted
                && r.dead.is_none()
            {
                brief.key = r.key;
                if let Some(p) = r.pump.as_ref() {
                    brief.wait = p.wait.map(WaitKey::of);
                    if let Some(w) = p.wait {
                        wait_eval = r.lease.storage.try_lock().ok().and_then(|storage| {
                            let (Ok(get), Ok(reference)) =
                                (storage.gpu_get(), storage.gpu_reference())
                            else {
                                return None;
                            };
                            // outside the ring / beyond what was issued: the worker's turn refuses it by name; not a starvation
                            let popped = p.core.fetch.peek(get).ok()?;
                            let marks = p.core.marks.peek(reference);
                            Some(WaitEval {
                                marks,
                                popped,
                                satisfied: w.satisfied(marks, popped),
                            })
                        });
                    }
                }
            }
            if let Ok(pt) = self.pt.try_lock()
                && let Some(t) = pt.get(&brief.key).filter(|t| t.user_ring)
            {
                brief.stopped = Some(t.stopped);
                brief.disabled = Some(t.disabled);
            }
            out.push(Reading {
                last_look_ns: record.last_look_ns(),
                wait_eval,
                brief,
            });
        }
        Some(out)
    }
}

/// The CPU state of the VMM's own service threads (`kf3-*`), from `/proc/self/task`: bounded, the diagnostic thread only.
fn read_threads() -> Vec<progress::ThreadCpu> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/proc/self/task") else {
        return out;
    };
    for e in dir.flatten() {
        if out.len() >= progress::MAX_THREADS {
            break;
        }
        let Some(tid) = e.file_name().to_str().and_then(|n| n.parse::<u64>().ok()) else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/self/task/{tid}/stat")) else {
            continue;
        };
        if let Some(t) = progress::parse_thread_stat(tid, &stat)
            && t.comm.starts_with("kf3-")
        {
            out.push(t);
        }
    }
    out
}
