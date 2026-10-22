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
}

impl InlineLedger {
    pub(crate) fn new(tokens: usize, trace: Trace) -> Self {
        Self {
            rung: (0..tokens).map(|_| AtomicU64::new(0)).collect(),
            rang: (0..tokens).map(|_| AtomicU64::new(0)).collect(),
            rung_at_us: (0..tokens).map(|_| AtomicU64::new(0)).collect(),
            trace,
        }
    }

    pub(crate) fn note(&self, idx: u32, reached: bool) {
        if let Some(c) = self.rung.get(idx as usize) {
            c.fetch_add(1, Ordering::Relaxed);
        }
        if (self.trace.enabled)()
            && let Some(c) = self.rung_at_us.get(idx as usize)
        {
            let t = (self.trace.clock)();
            c.store((t * 1e6) as u64, Ordering::Relaxed);
            let n = self
                .rung
                .get(idx as usize)
                .map_or(0, |c| c.load(Ordering::Relaxed));
            (self.trace.emit)(Witness { idx, n, reached, t });
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
        (rung, reached)
    }
}
