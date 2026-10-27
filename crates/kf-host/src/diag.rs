//! CPU timing counters: RM/admission always on; nested ENS3 phases opt in. No GPU operation.

use std::cell::Cell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

/// Session-wide verb counters: calls, wall ns, maximum single call ns.
static GLOBAL: [[AtomicU64; 3]; VERBS] = [const { [const { AtomicU64::new(0) }; 3] }; VERBS];

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

/// Host RM verb classes (ENS6). Each timed ioctl lands in exactly one class; the class also
/// feeds the disjoint [`Phase::Rm`] / [`Phase::Invalidate`] totals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// Allocation of an `NV50_MEMORY_VIRTUAL` VA reservation (sparse or normal).
    ReserveVa,
    /// Any other object allocation.
    AllocOther,
    /// Object free (a reservation release is one of these).
    Free,
    /// DMA map.
    Map,
    /// DMA unmap.
    Unmap,
    /// Any control other than the TLB invalidate.
    Control,
    /// The TLB invalidate control.
    Invalidate,
    /// CPU-view arm/release and other verbs.
    Other,
}

/// Number of [`Verb`] classes.
pub const VERBS: usize = 8;

/// Per-thread counters: other workers cannot contaminate an apply's delta.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Indexed by [`Phase`], in declaration order.
    pub costs: [Cost; 5],
    /// Indexed by [`Verb`], in declaration order (this thread only).
    pub verbs: [Cost; VERBS],
}

/// Session-wide verb totals, read with relaxed atomics only (safe for the heartbeat).
/// `work`/`last_ns` are not tracked session-wide and read 0.
#[must_use]
pub fn verb_totals() -> [Cost; VERBS] {
    let mut out = [Cost::default(); VERBS];
    for (cost, atoms) in out.iter_mut().zip(&GLOBAL) {
        cost.calls = atoms[0].load(Relaxed);
        cost.ns = atoms[1].load(Relaxed);
        cost.max_ns = atoms[2].load(Relaxed);
    }
    out
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
        for (now, old) in self.verbs.iter_mut().zip(before.verbs) {
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

/// The verb class of an object allocation of `class`.
#[must_use]
pub(crate) fn alloc_verb(class: u32) -> Verb {
    if class == crate::channel::NV50_MEMORY_VIRTUAL {
        Verb::ReserveVa
    } else {
        Verb::AllocOther
    }
}

/// The verb class of a control `cmd`.
#[must_use]
pub(crate) fn control_verb(cmd: u32) -> Verb {
    if cmd == crate::channel::NV2080_CTRL_CMD_DMA_INVALIDATE_TLB {
        Verb::Invalidate
    } else {
        Verb::Control
    }
}

/// Time precisely the supplied ioctl as `verb`; result and errors pass through unchanged.
pub(crate) fn rm_verb<T>(verb: Verb, f: impl FnOnce() -> T) -> T {
    let _activity = crate::latency::activity(crate::latency::Activity::Rm(verb as u8));
    rm_verb_with(true, verb, f)
}

fn rm_verb_with<T>(active: bool, verb: Verb, f: impl FnOnce() -> T) -> T {
    if !active {
        return f();
    }
    let start = Instant::now();
    let out = f();
    let ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
    let phase = if verb == Verb::Invalidate {
        Phase::Invalidate
    } else {
        Phase::Rm
    };
    record(phase, ns, 0);
    record_verb(verb, ns);
    out
}

fn record_verb(verb: Verb, ns: u64) {
    COUNTERS.with(|cell| {
        let mut s = cell.get();
        let c = &mut s.verbs[verb as usize];
        c.calls = c.calls.saturating_add(1);
        c.ns = c.ns.saturating_add(ns);
        c.last_ns = ns;
        c.max_ns = c.max_ns.max(ns);
        cell.set(s);
    });
    let atoms = &GLOBAL[verb as usize];
    atoms[0].fetch_add(1, Relaxed);
    atoms[1].fetch_add(ns, Relaxed);
    atoms[2].fetch_max(ns, Relaxed);
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
        assert_eq!(rm_verb(Verb::Other, || Err::<(), _>(123)), Err(123));
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
        assert_eq!(rm_verb_with(true, Verb::Other, || Ok::<_, u32>(7)), Ok(7));
        assert_eq!(rm_verb_with(true, Verb::Other, || Err::<(), _>(9)), Err(9));
        let delta = snapshot().since(before);
        assert_eq!(delta.costs[0].calls, 2);
        assert_eq!(delta.costs[1].calls, 0);
    }

    #[test]
    fn invalidate_is_not_counted_as_an_ordinary_rm_call() {
        let before = snapshot();
        assert_eq!(
            rm_verb_with(true, Verb::Invalidate, || Err::<(), _>(123)),
            Err(123)
        );
        let delta = snapshot().since(before);
        assert_eq!(delta.costs[0].calls, 0);
        assert_eq!(delta.costs[1].calls, 1);
    }

    #[test]
    fn each_rm_verb_lands_in_its_own_bucket_and_in_the_rm_total() {
        for (i, verb) in [
            Verb::ReserveVa,
            Verb::AllocOther,
            Verb::Free,
            Verb::Map,
            Verb::Unmap,
            Verb::Control,
            Verb::Other,
        ]
        .into_iter()
        .enumerate()
        {
            let before = snapshot();
            assert_eq!(rm_verb(verb, || Ok::<_, u32>(1)), Ok(1));
            let delta = snapshot().since(before);
            assert_eq!(delta.verbs[verb as usize].calls, 1, "verb {i}");
            assert_eq!(delta.verbs.iter().map(|c| c.calls).sum::<u64>(), 1);
            assert_eq!(delta.costs[0].calls, 1, "total Rm phase, verb {i}");
            assert_eq!(delta.costs[1].calls, 0);
        }
    }

    #[test]
    fn the_invalidate_verb_is_the_invalidate_phase_and_not_an_rm_call() {
        let before = snapshot();
        assert_eq!(rm_verb(Verb::Invalidate, || Err::<(), _>(5)), Err(5));
        let delta = snapshot().since(before);
        assert_eq!(delta.verbs[Verb::Invalidate as usize].calls, 1);
        assert_eq!(delta.costs[1].calls, 1);
        assert_eq!(delta.costs[0].calls, 0);
    }

    #[test]
    fn verb_time_is_accumulated_and_the_max_is_kept() {
        let before = snapshot();
        rm_verb(Verb::ReserveVa, || {
            std::thread::sleep(std::time::Duration::from_millis(3));
        });
        rm_verb(Verb::ReserveVa, || ());
        let delta = snapshot().since(before);
        let c = delta.verbs[Verb::ReserveVa as usize];
        assert_eq!(c.calls, 2);
        assert!(c.ns >= 3_000_000, "{c:?}");
        assert!(c.max_ns >= 3_000_000 && c.max_ns <= c.ns, "{c:?}");
    }

    #[test]
    fn interval_subtracts_verb_buckets_without_underflow() {
        let mut before = Snapshot::default();
        before.verbs[2].calls = 9;
        before.verbs[2].ns = 90;
        let mut after = before;
        after.verbs[2].calls += 3;
        after.verbs[2].ns += 30;
        assert_eq!(after.since(before).verbs[2].calls, 3);
        assert_eq!(after.since(before).verbs[2].ns, 30);
        assert_eq!(before.since(after).verbs[2].calls, 0);
    }

    #[test]
    fn session_totals_add_every_thread_and_keep_the_global_maximum() {
        let base = verb_totals()[Verb::Unmap as usize];
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    rm_verb(Verb::Unmap, || {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    });
                });
            }
        });
        let now = verb_totals()[Verb::Unmap as usize];
        assert!(now.calls >= base.calls + 4, "{base:?} -> {now:?}");
        assert!(now.ns >= base.ns + 8_000_000, "{base:?} -> {now:?}");
        assert!(now.max_ns >= 2_000_000, "{now:?}");
        assert!(now.max_ns <= now.ns);
    }

    #[test]
    fn a_disabled_verb_timer_records_nothing_session_wide() {
        let base = verb_totals()[Verb::Control as usize];
        rm_verb_with(false, Verb::Control, || ());
        assert_eq!(verb_totals()[Verb::Control as usize], base);
    }

    #[test]
    fn a_va_reservation_alloc_is_the_reserve_verb_and_other_classes_are_not() {
        assert_eq!(
            alloc_verb(crate::channel::NV50_MEMORY_VIRTUAL),
            Verb::ReserveVa
        );
        assert_eq!(
            alloc_verb(kf_abi::bringup::NV01_MEMORY_VIRTUAL),
            Verb::AllocOther
        );
        assert_eq!(alloc_verb(0), Verb::AllocOther);
    }

    #[test]
    fn the_tlb_invalidate_control_is_the_invalidate_verb_and_others_are_control() {
        assert_eq!(
            control_verb(crate::channel::NV2080_CTRL_CMD_DMA_INVALIDATE_TLB),
            Verb::Invalidate
        );
        assert_eq!(control_verb(0x2080_0101), Verb::Control);
    }

    #[test]
    fn the_session_maximum_survives_a_later_faster_call() {
        rm_verb(Verb::AllocOther, || {
            std::thread::sleep(std::time::Duration::from_millis(3));
        });
        rm_verb(Verb::AllocOther, || ());
        assert!(verb_totals()[Verb::AllocOther as usize].max_ns >= 3_000_000);
    }

    #[test]
    fn the_session_maximum_is_the_slowest_call_of_any_thread_in_any_order() {
        std::thread::spawn(|| {
            rm_verb(Verb::AllocOther, || {
                std::thread::sleep(std::time::Duration::from_millis(6));
            });
        })
        .join()
        .unwrap();
        std::thread::spawn(|| rm_verb(Verb::AllocOther, || ()))
            .join()
            .unwrap();
        assert!(verb_totals()[Verb::AllocOther as usize].max_ns >= 6_000_000);
    }

    #[test]
    fn only_the_virtual_memory_class_is_a_reservation_whatever_the_neighbour() {
        for class in [0x0080u32, 0x50a1, 0x509f, 0x00f1, 0xc56f, u32::MAX] {
            assert_eq!(alloc_verb(class), Verb::AllocOther, "{class:#x}");
        }
        assert_eq!(alloc_verb(0x50a0), Verb::ReserveVa);
    }

    #[test]
    fn only_the_tlb_invalidate_control_is_the_invalidate_verb() {
        for cmd in [
            0x2080_2501u32,
            0x2080_2503,
            0x0000_0000,
            0x2080_0101,
            u32::MAX,
        ] {
            assert_eq!(control_verb(cmd), Verb::Control, "{cmd:#x}");
        }
        assert_eq!(control_verb(0x2080_2502), Verb::Invalidate);
    }
}
