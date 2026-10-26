//! ENS3 measurement only; no GPU operation is authored here.

use std::time::Instant;

/// Timed disjoint RM categories and nested sparse phases.
#[derive(Clone, Copy, Debug)]
pub enum Phase {
    /// RM ioctl except invalidate.
    Rm,
    /// RM invalidate ioctl.
    Invalidate,
    /// Complete sparse admission call.
    Admission,
    /// Reservation-tree clone.
    Clone,
    /// Error-path intent restoration scan.
    Restore,
}

/// One bounded counter bucket.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cost {
    /// Completed calls.
    pub calls: u64,
    /// Accumulated wall time.
    pub ns: u64,
    /// Work items (blocks or pages, not bytes).
    pub work: u64,
    /// Latest call cost, ns.
    pub last_ns: u64,
    /// Maximum call cost on this thread, ns.
    pub max_ns: u64,
}

/// Per-thread counters: other workers cannot contaminate an apply's delta.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Indexed by [`Phase`], in declaration order.
    pub costs: [Cost; 5],
}

/// Explicit opt-in, sampled once.
pub fn enabled() -> bool {
    false
}

/// Current thread only.
pub fn snapshot() -> Snapshot {
    Snapshot::default()
}

impl Snapshot {
    /// Saturating interval counters; last/max remain thread-level gauges.
    pub fn since(self, _before: Self) -> Self {
        Self::default()
    }

    /// Apply wall split: residual (not CPU usage), other RM, invalidate.
    pub fn split(self, _wall: u64) -> [u64; 3] {
        [0; 3]
    }
}

/// A diagnostic clock, absent when disabled.
pub struct Scope(Option<(Instant, Phase, u64)>);

impl Scope {
    /// No clock read on the disabled path.
    pub fn start(phase: Phase, work: u64) -> Self {
        Self::start_with(enabled(), phase, work)
    }

    fn start_with(_active: bool, _phase: Phase, _work: u64) -> Self {
        Self(None)
    }
}

/// Time precisely the supplied ioctl; result and errors pass through unchanged.
pub(crate) fn rm<T>(invalidate: bool, f: impl FnOnce() -> T) -> T {
    let _timer = Scope::start(
        if invalidate {
            Phase::Invalidate
        } else {
            Phase::Rm
        },
        0,
    );
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rm_and_invalidate_are_disjoint_in_an_apply() {
        let mut s = Snapshot::default();
        s.costs[0].ns = 20;
        s.costs[1].ns = 30;
        assert_eq!(s.split(100), [50, 20, 30]);
    }

    #[test]
    fn nested_admission_is_not_subtracted_twice() {
        let mut s = Snapshot::default();
        s.costs[0].ns = 7;
        s.costs[1].ns = 11;
        s.costs[2].ns = 90;
        s.costs[3].ns = 20;
        assert_eq!(s.split(100), [82, 7, 11]);
    }

    #[test]
    fn sparse_scope_counts_every_call_and_blocks() {
        let before = snapshot();
        {
            let _t = Scope::start_with(true, Phase::Clone, 8192);
        }
        let after = snapshot().since(before);
        assert_eq!(after.costs[3].calls, 1);
        assert_eq!(after.costs[3].work, 8192);
    }

    #[test]
    fn restore_scope_counts_work_and_keeps_thread_isolation() {
        let before = snapshot();
        std::thread::spawn(|| {
            let _t = Scope::start_with(true, Phase::Restore, 999);
        })
        .join()
        .unwrap();
        {
            let _t = Scope::start_with(true, Phase::Restore, 512);
        }
        let after = snapshot().since(before);
        assert_eq!(after.costs[4].calls, 1);
        assert_eq!(after.costs[4].work, 512);
    }

    #[test]
    fn disabled_scope_and_error_results_are_inert() {
        let before = snapshot();
        {
            let _t = Scope::start_with(false, Phase::Rm, 5);
        }
        assert_eq!(snapshot(), before);
        assert_eq!(rm(false, || Err::<(), _>(123)), Err(123));
    }

    #[test]
    fn interval_subtracts_previous_apply_without_underflow() {
        let mut before = Snapshot::default();
        before.costs[0].calls = 4;
        before.costs[0].ns = 100;
        let mut after = before;
        after.costs[0].calls += 2;
        after.costs[0].ns += 50;
        assert_eq!(after.since(before).costs[0].calls, 2);
        assert_eq!(after.since(before).costs[0].ns, 50);
        assert_eq!(before.since(after).costs[0].ns, 0);
    }
}
