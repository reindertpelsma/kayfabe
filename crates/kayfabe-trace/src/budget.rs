//! Perf-budget counters — per-plane event counts, maintained by the recorder.
//!
//! Two jobs, and the second is the important one.
//!
//! 1. **The budget** (lesson L6 / R5 mitigation): every port step gets before/after
//!    numbers on the same bench. "How many doorbell dispatches did that workload cost"
//!    is a counter, not a stream — you can keep it on with the stream sink discarded.
//!
//! 2. ★★ **Non-vacuity.** `testing_doctrine.md` §1: *a green instrument on an
//!    unexercised path is worse than no instrument, because it reads as evidence.* For a
//!    tracing crate the obvious trap is a sink that records nothing while every test
//!    passes. The counters are kept by the [`crate::Recorder`], **not** by the sink, so
//!    the two can be compared: a sink that silently drops shows up as
//!    `counters.total() != sink.len()`, and a run that never reached a plane shows up in
//!    [`Counters::silent_kinds`]. Both are assertions a test can make, and both fail
//!    loudly on the exact failure a bare `assert!(log.is_empty() == false)` would miss.

use crate::event::EventKind;

/// Per-kind counts of the events a recorder was **offered**.
///
/// "Offered", not "stored", on purpose — see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counters {
    by_kind: [u64; EventKind::COUNT],
    total: u64,
}

impl Default for Counters {
    fn default() -> Self {
        Counters::new()
    }
}

impl Counters {
    /// All zero.
    #[must_use]
    pub const fn new() -> Counters {
        Counters {
            by_kind: [0; EventKind::COUNT],
            total: 0,
        }
    }

    /// Count one event.
    pub fn bump(&mut self, kind: EventKind) {
        self.by_kind[kind.index()] += 1;
        self.total += 1;
    }

    /// How many events of `kind` were offered.
    #[must_use]
    pub fn of(&self, kind: EventKind) -> u64 {
        self.by_kind[kind.index()]
    }

    /// How many events in total.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total
    }

    /// ★ Non-vacuity, negative form: the kinds this run never produced.
    ///
    /// A test that claims to exercise a plane asserts this does **not** contain that
    /// plane's kind. That is a bound on what the instrument saw, which §1 rule 1 asks
    /// for — and unlike `assert!(!log.is_empty())` it cannot be satisfied by some other
    /// plane's traffic.
    #[must_use]
    pub fn silent_kinds(&self) -> Vec<EventKind> {
        EventKind::ALL
            .into_iter()
            .filter(|k| self.of(*k) == 0)
            .collect()
    }

    /// ★ Non-vacuity, positive form: the kinds this run produced.
    #[must_use]
    pub fn seen_kinds(&self) -> Vec<EventKind> {
        EventKind::ALL
            .into_iter()
            .filter(|k| self.of(*k) > 0)
            .collect()
    }

    /// True if nothing at all was recorded — the "green instrument on an unexercised
    /// path" state, named so a test can refuse it explicitly.
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.total == 0
    }

    /// Sum of the wire-plane kinds (the recorded device stream).
    #[must_use]
    pub fn wire_total(&self) -> u64 {
        EventKind::ALL
            .into_iter()
            .filter(|k| k.is_wire())
            .map(|k| self.of(k))
            .sum()
    }

    /// Sum of the core decision planes.
    #[must_use]
    pub fn core_total(&self) -> u64 {
        self.total - self.wire_total()
    }

    /// Merge another set of counters into this one — for per-thread recorders whose
    /// *counts* are additive even though their [`crate::Seq`] spaces are not comparable
    /// (see [`crate::Recorder`]'s ordering contract).
    pub fn absorb(&mut self, other: &Counters) {
        for k in EventKind::ALL {
            self.by_kind[k.index()] += other.of(k);
        }
        self.total += other.total;
    }
}

impl core::fmt::Display for Counters {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "total={}", self.total)?;
        for k in EventKind::ALL {
            let n = self.of(k);
            if n > 0 {
                write!(f, " {k}={n}")?;
            }
        }
        Ok(())
    }
}
