//! CPU timing counters: RM/admission always on; nested ENS3 phases opt in. No GPU operation.

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::Instant;

thread_local! {
    static COUNTERS: Cell<Snapshot> = Cell::new(Snapshot::default());
}

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
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("KF3_DIAG_ENS3").ok().as_deref() == Some("1"))
}

/// Actual calling-thread CPU consumption, queried only by an opted-in diagnostic.
pub fn thread_cpu_ns() -> Option<u64> {
    enabled().then(kf_linux_raw::thread_cpu_ns).flatten()
}

/// Current thread only.
pub fn snapshot() -> Snapshot {
    COUNTERS.with(Cell::get)
}

impl Snapshot {
    /// Saturating interval counters; last/max remain thread-level gauges.
    pub fn since(mut self, before: Self) -> Self {
        for (now, old) in self.costs.iter_mut().zip(before.costs) {
            now.calls = now.calls.saturating_sub(old.calls);
            now.ns = now.ns.saturating_sub(old.ns);
            now.work = now.work.saturating_sub(old.work);
        }
        self
    }

    /// Apply wall split: residual (not CPU usage), other RM, invalidate.
    pub fn split(self, wall: u64) -> [u64; 3] {
        let rm = self.costs[0].ns.min(wall);
        let invalidate = self.costs[1].ns.min(wall.saturating_sub(rm));
        [
            wall.saturating_sub(rm).saturating_sub(invalidate),
            rm,
            invalidate,
        ]
    }
}

/// A diagnostic clock, absent when disabled.
pub struct Scope(Option<(Instant, Phase, u64)>);

impl Scope {
    /// No clock read on the disabled path.
    pub fn start(phase: Phase, work: u64) -> Self {
        Self::start_with(enabled(), phase, work)
    }

    fn start_with(active: bool, phase: Phase, work: u64) -> Self {
        Self(active.then(|| (Instant::now(), phase, work)))
    }

    /// Add actual work visited; does nothing for a disabled timer.
    pub fn add_work(&mut self, work: u64) {
        if let Some((_, _, count)) = &mut self.0 {
            *count = count.saturating_add(work);
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let Some((start, phase, work)) = self.0 else {
            return;
        };
        let ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        record(phase, ns, work);
    }
}

fn record(phase: Phase, ns: u64, work: u64) {
    COUNTERS.with(|cell| {
        let mut s = cell.get();
        let c = &mut s.costs[phase as usize];
        c.calls = c.calls.saturating_add(1);
        c.ns = c.ns.saturating_add(ns);
        c.work = c.work.saturating_add(work);
        c.last_ns = ns;
        c.max_ns = c.max_ns.max(ns);
        cell.set(s);
    });
}

/// Admission timing reuses the caller's existing clock; cheap counters are always on.
pub fn admission(ns: u64) {
    record(Phase::Admission, ns, 0);
}

/// Time precisely the supplied ioctl; result and errors pass through unchanged.
pub(crate) fn rm<T>(invalidate: bool, f: impl FnOnce() -> T) -> T {
    rm_with(true, invalidate, f)
}

fn rm_with<T>(active: bool, invalidate: bool, f: impl FnOnce() -> T) -> T {
    let _timer = Scope::start_with(
        active,
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

    #[test]
    fn ioctl_success_and_error_each_contribute_one_real_call() {
        let before = snapshot();
        assert_eq!(rm_with(true, false, || Ok::<_, u32>(7)), Ok(7));
        assert_eq!(rm_with(true, false, || Err::<(), _>(9)), Err(9));
        let delta = snapshot().since(before);
        assert_eq!(delta.costs[0].calls, 2);
        assert_eq!(delta.costs[1].calls, 0);
    }

    #[test]
    fn invalidate_is_not_counted_as_an_ordinary_rm_call() {
        let before = snapshot();
        assert_eq!(rm_with(true, true, || Err::<(), _>(123)), Err(123));
        let delta = snapshot().since(before);
        assert_eq!(delta.costs[0].calls, 0);
        assert_eq!(delta.costs[1].calls, 1);
    }
}
