//! Bounded worker-side non-stall observations for P1. This entire module and its call sites
//! are absent unless compiled with `diag-nsi`; `KF3_DIAG_NSI_TRACE=1` is also required.
//!
//! Records are a tail of relay decisions, not raw GPU interrupts: the completion fd has both
//! FIFO and host-CE subscriptions, and host dataless readiness may already have coalesced events.
//! The recorder never controls delivery. Workers use only `try_lock`, fixed-capacity storage
//! and already-required event timestamps. No recorder call runs on a vCPU. A drainer-only
//! request-file check freezes/dumps the tail outside the recorder lock. Trace contention is
//! counted; a trace with contention cannot support an exact event census.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use kf_chan::ptnsi::Verdict;

/// Maximum retained records; a long boot cannot exhaust the subsequent measurement window.
pub const CAPACITY: usize = 32_768;

/// Host readiness source, or the loss-free pacer's trailing delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Shared dataless fd: FIFO_EVENT_MTHD OR the configured asynchronous host CE notifier.
    CompletionFd,
    /// Dedicated engine notification fd, identified by its host engine type.
    Engine(u32),
    /// One vector raised later for one or more previously owed source edges.
    Trailing,
}

/// One decision before invoking the guest delivery callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    /// Reservation identity; records may arrive from two workers in a different order.
    pub seq: u64,
    /// Nanoseconds on the channel plane's monotonic, realization-relative clock.
    pub at_ns: u64,
    /// Host readiness source.
    pub source: Source,
    /// Guest non-stall subscription slot (not a guest object handle).
    pub slot: Option<usize>,
    /// Guest CPU interrupt-tree vector, not the physical MSI-X vector (which is zero).
    pub vector: Option<u32>,
    /// Arming sampled for this decision; trailing deliveries carry an earlier owed edge.
    pub armed: Option<bool>,
    /// Relay outcome; retained even when no IRQ is requested.
    pub verdict: Verdict,
}

struct Buffer {
    records: VecDeque<Record>,
    overwritten: u64,
    finished: bool,
}

/// A frozen tail and its explicit trace losses. Counts are exact once producers are quiescent.
pub struct Snapshot {
    /// Offered diagnostic records.
    pub seen: u64,
    /// Older records replaced by newer records at the fixed bound.
    pub overwritten: u64,
    /// Records the diagnostic refused instead of making a worker block.
    pub contended: u64,
    /// Retained records in insertion order; use time/sequence to compare concurrent workers.
    pub records: Vec<Record>,
}

/// Optional fixed-capacity trace; no guest state or delivery decision depends on it.
pub struct Trace {
    active: AtomicBool,
    seen: AtomicU64,
    contended: AtomicU64,
    buffer: Option<Mutex<Buffer>>,
    dump_request: Option<PathBuf>,
}

impl Trace {
    /// Read the runtime opt-in and optional request-file path once at realize.
    #[must_use]
    pub fn from_env() -> Self {
        let setting = std::env::var("KF3_DIAG_NSI_TRACE").ok();
        let mut trace = Self::from_setting(setting.as_deref());
        if trace.buffer.is_some() {
            trace.dump_request = std::env::var_os("KF3_DIAG_NSI_TRACE_DUMP").map(PathBuf::from);
            eprintln!(
                "kf3: DIAG_NSI_TRACE enabled cap={CAPACITY} clock=channel-relative-monotonic-ns dump_request={:?}; completion-fd=FIFO-or-host-CE, not raw IRQs",
                trace.dump_request
            );
        }
        trace
    }

    /// Construct without process-global environment changes (unset and every value but `1` off).
    #[must_use]
    pub fn from_setting(setting: Option<&str>) -> Self {
        let on = setting == Some("1");
        Self {
            active: AtomicBool::new(on),
            seen: AtomicU64::new(0),
            contended: AtomicU64::new(0),
            buffer: on.then(|| {
                Mutex::new(Buffer {
                    records: VecDeque::with_capacity(CAPACITY),
                    overwritten: 0,
                    finished: false,
                })
            }),
            dump_request: None,
        }
    }

    /// Allocated record slots, fixed at creation (zero with the runtime flag off).
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buffer.as_ref().map_or(0, |_| CAPACITY)
    }

    /// Worker only: retain one decision without waiting, allocating or doing I/O.
    pub fn note(
        &self,
        at_ns: u64,
        source: Source,
        slot: Option<usize>,
        vector: Option<u32>,
        armed: Option<bool>,
        verdict: Verdict,
    ) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let Some(buffer) = &self.buffer else { return };
        let seq = self.seen.fetch_add(1, Ordering::Relaxed) + 1;
        let Ok(mut b) = buffer.try_lock() else {
            self.contended.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if b.finished {
            self.contended.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if b.records.len() == CAPACITY {
            b.records.pop_front();
            b.overwritten += 1;
        }
        b.records.push_back(Record {
            seq,
            at_ns,
            source,
            slot,
            vector,
            armed,
            verdict,
        });
    }

    /// Freeze once. Never waits for a worker; a busy buffer may be retried by the drainer.
    /// A concurrent note can still finish its accounting after this snapshot; the dump reports
    /// the accounting difference and must not be treated as a complete census when nonzero.
    #[must_use]
    pub fn finish(&self) -> Option<Snapshot> {
        self.active.store(false, Ordering::Release);
        let mut b = self.buffer.as_ref()?.try_lock().ok()?;
        if b.finished {
            return None;
        }
        b.finished = true;
        Some(Snapshot {
            seen: self.seen.load(Ordering::Acquire),
            overwritten: b.overwritten,
            contended: self.contended.load(Ordering::Acquire),
            records: std::mem::take(&mut b.records).into_iter().collect(),
        })
    }

    /// Drainer only, from the existing diagnostic heartbeat: dump once when the host-created
    /// request file exists. Never sleeps, changes a wake/interval, or checks a file on a vCPU.
    pub fn poll_dump(&self) {
        if !self.dump_request.as_ref().is_some_and(|p| p.is_file()) {
            return;
        }
        let Some(s) = self.finish() else { return };
        let accounted = s.records.len() as u64 + s.overwritten + s.contended;
        eprintln!(
            "kf3: DIAG_NSI_TRACE_BEGIN seen={} retained={} overwritten={} contended={} accounting_delta={} clock=channel-relative-monotonic-ns",
            s.seen,
            s.records.len(),
            s.overwritten,
            s.contended,
            s.seen.abs_diff(accounted)
        );
        for r in s.records {
            eprintln!(
                "kf3: DIAG_NSI_EDGE seq={} ns={} source={:?} slot={:?} vector={:?} armed={:?} verdict={:?}",
                r.seq, r.at_ns, r.source, r.slot, r.vector, r.armed, r.verdict
            );
        }
        eprintln!("kf3: DIAG_NSI_TRACE_END");
    }
}
