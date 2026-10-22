//! Inline-doorbell diagnostic counters, separated from GPU channel construction.
#![cfg_attr(
    not(test),
    deny(clippy::arithmetic_side_effects, clippy::indexing_slicing)
)]

use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) struct Witness {
    pub(crate) idx: u32,
    pub(crate) n: u64,
    pub(crate) reached: bool,
    pub(crate) t: f64,
}

pub(crate) struct Trace {
    pub(crate) enabled: Box<dyn Fn() -> bool + Send + Sync>,
    pub(crate) clock: Box<dyn Fn() -> f64 + Send + Sync>,
    pub(crate) emit: Box<dyn Fn(Witness) + Send + Sync>,
}

pub(crate) struct InlineLedger {
    pub(crate) rung: Box<[AtomicU64]>,
    pub(crate) rang: Box<[AtomicU64]>,
    pub(crate) rung_at_us: Box<[AtomicU64]>,
    trace: Trace,
    diagnostic_on: bool,
}

impl InlineLedger {
    pub(crate) fn new(tokens: usize, trace: Trace) -> Self {
        // Realize owns configuration and any cold clock initialization. Neither
        // capability may run for the first time while a vCPU is in a trap.
        let diagnostic_on = (trace.enabled)();
        if diagnostic_on {
            let _ = (trace.clock)();
        }
        Self {
            rung: (0..tokens).map(|_| AtomicU64::new(0)).collect(),
            rang: (0..tokens).map(|_| AtomicU64::new(0)).collect(),
            rung_at_us: (0..tokens).map(|_| AtomicU64::new(0)).collect(),
            trace,
            diagnostic_on,
        }
    }

    pub(crate) fn note(&self, idx: u32, reached: bool) {
        if let Some(c) = self.rung.get(idx as usize) {
            c.fetch_add(1, Ordering::Relaxed);
        }
        if self.diagnostic_on
            && let Some(c) = self.rung_at_us.get(idx as usize)
        {
            let t = (self.trace.clock)();
            c.store((t * 1e6) as u64, Ordering::Relaxed);
        }
        if reached && let Some(c) = self.rang.get(idx as usize) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn take(&self, idx: u32) -> (u64, u64) {
        let rung = self
            .rung
            .get(idx as usize)
            .map_or(0, |c| c.swap(0, Ordering::Relaxed));
        let reached = self
            .rang
            .get(idx as usize)
            .map_or(0, |c| c.swap(0, Ordering::Relaxed));
        // Existing birth/free callers run on the act owner. Formatting and the
        // potentially unavailable sink are reached only after counter capture,
        // never by note(). This is an aggregate since the previous take.
        if self.diagnostic_on && rung != 0 {
            let us = self
                .rung_at_us
                .get(idx as usize)
                .map_or(0, |c| c.load(Ordering::Relaxed));
            (self.trace.emit)(Witness {
                idx,
                n: rung,
                reached: reached != 0,
                t: us as f64 / 1e6,
            });
        }
        (rung, reached)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::{Arc, Mutex};

    fn record_while_sink_is_unavailable(reached: bool) {
        let blocked = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(AtomicUsize::new(0));
        let flags = Arc::new(AtomicUsize::new(0));
        let clocks = Arc::new(AtomicUsize::new(0));
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let trace = Trace {
            enabled: Box::new({
                let f = flags.clone();
                move || {
                    f.fetch_add(1, Ordering::Relaxed);
                    true
                }
            }),
            clock: Box::new({
                let c = clocks.clone();
                move || {
                    c.fetch_add(1, Ordering::Relaxed);
                    1.25
                }
            }),
            emit: Box::new({
                let (b, a, e) = (blocked.clone(), attempts.clone(), emitted.clone());
                move |w| {
                    // A fake slow holder: entering this capability would wait in
                    // production. Count the forbidden step instead of sleeping.
                    a.fetch_add(1, Ordering::Relaxed);
                    if !b.load(Ordering::Relaxed) {
                        e.lock().unwrap().push(w);
                    }
                }
            }),
        };
        let ledger = InlineLedger::new(2, trace);
        let flags_before = flags.load(Ordering::Relaxed);
        blocked.store(true, Ordering::Relaxed);
        ledger.note(1, reached);
        assert_eq!(
            attempts.load(Ordering::Relaxed),
            0,
            "trap entered the unavailable output sink"
        );
        assert_eq!(
            flags.load(Ordering::Relaxed),
            flags_before,
            "configuration lookup on trap"
        );
        assert_eq!(
            clocks.load(Ordering::Relaxed),
            2,
            "clock warmed at construction, one read on trap"
        );
        assert_eq!(ledger.rung_at_us[1].load(Ordering::Relaxed), 1_250_000);
        blocked.store(false, Ordering::Relaxed);
        assert_eq!(ledger.take(1), (1, u64::from(reached)));
        assert_eq!(
            attempts.load(Ordering::Relaxed),
            1,
            "off-trap take reports once"
        );
        let e = emitted.lock().unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(
            (e[0].idx, e[0].n, e[0].reached, e[0].t),
            (1, 1, reached, 1.25)
        );
        drop(e);
        assert_eq!(ledger.take(1), (0, 0));
        assert_eq!(
            attempts.load(Ordering::Relaxed),
            1,
            "empty take cannot repeat output"
        );
    }

    #[test]
    fn worst_trap_successful_ring_never_enters_a_slow_diagnostic_sink() {
        record_while_sink_is_unavailable(true);
    }

    #[test]
    fn worst_trap_failed_ring_never_enters_a_slow_diagnostic_sink() {
        record_while_sink_is_unavailable(false);
    }
}
