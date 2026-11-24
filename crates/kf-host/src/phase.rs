//! (`docs/design/V3_VA_PROFILE.md`): **per-thread phase accumulators of the VA
//! thread's apply** — OBSERVATION ONLY. Nothing reads these to decide anything.
//!
//! Each [`Phase`] keeps a count, a sum and a maximum of nanoseconds in a thread-local cell, so the
//! VA thread's apply can be split into preparation, host RM map/unmap calls, stitching and so on
//! without a lock, an atomic shared with another thread, or an allocation. The VA manager takes
//! ([`take`]) the accumulators before and after one space's apply; what was recorded in between is
//! that apply's share. Another thread that makes host calls accumulates into its own cells, which
//! nobody reads (saturating, fixed size: no growth).
//!
//! Recorded where the work is done: the apply's own partitions (`Prep`, `UnmapPhase`, `MapPhase`,
//! `Takedown`, `Invalidate`) by `kf_mem::apply::apply_entry`; the host verbs (`RmMap`, `RmUnmap`,
//! `RmFree`, `RmReserve`) and the ledger's retirement (`LedgerRetire`, exclusive of the frees it
//! makes) by `kf_mem::batch::BatchedVas`; the batched map's `Stitch`, `Descriptor` and `Reap`
//! (subsets of that `RmMap` call) by [`crate::channel::HostRm::map_scattered_through`].

use std::cell::Cell;

/// One phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The apply's planning: kept/overlap pairs, linked failures, the host rows (before any host call).
    Prep = 0,
    /// The apply's unmap step (host unmaps and the ledger, wall).
    UnmapPhase = 1,
    /// The apply's map step (usermode/SKED verbs, batched and single maps, wall).
    MapPhase = 2,
    /// The apply's take-down of new pieces of a failed MAP run (wall).
    Takedown = 3,
    /// The apply's one host TLB invalidate.
    Invalidate = 4,
    /// One host map verb (a row, a SKED row, or a stitched batch whole: stitch + descriptor + reap + RM map).
    RmMap = 5,
    /// One host unmap verb (whole, row, range, or through a reservation).
    RmUnmap = 6,
    /// One host object free (a batch object or a micro reservation).
    RmFree = 7,
    /// One host VA reservation.
    RmReserve = 8,
    /// The ledger's own work after an unmap lands (own-map cut, book, emptied objects), frees excluded.
    LedgerRetire = 9,
    /// Inside a batched map: stitching the pieces into one view (the `MAP_FIXED` mmaps).
    Stitch = 10,
    /// Inside a batched map: the OS descriptor (RM pins the pages).
    Descriptor = 11,
    /// Inside a batched map: handing the view to the reaper.
    Reap = 12,
}

/// How many phases.
pub const COUNT: usize = 13;

impl Phase {
    /// Every phase, in index order.
    pub const ALL: [Phase; COUNT] = [
        Phase::Prep,
        Phase::UnmapPhase,
        Phase::MapPhase,
        Phase::Takedown,
        Phase::Invalidate,
        Phase::RmMap,
        Phase::RmUnmap,
        Phase::RmFree,
        Phase::RmReserve,
        Phase::LedgerRetire,
        Phase::Stitch,
        Phase::Descriptor,
        Phase::Reap,
    ];

    /// The field-name stem in the `vat[...]` line and the slow records.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Phase::Prep => "prep",
            Phase::UnmapPhase => "unmapph",
            Phase::MapPhase => "mapph",
            Phase::Takedown => "takedown",
            Phase::Invalidate => "inval",
            Phase::RmMap => "rm_map",
            Phase::RmUnmap => "rm_unmap",
            Phase::RmFree => "rm_free",
            Phase::RmReserve => "rm_resv",
            Phase::LedgerRetire => "ledger",
            Phase::Stitch => "stitch",
            Phase::Descriptor => "desc",
            Phase::Reap => "reap",
        }
    }
}

/// Count, sum and maximum of one phase, in nanoseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Acc {
    /// Occurrences.
    pub n: u64,
    /// Summed ns.
    pub ns: u64,
    /// The longest one, ns.
    pub max: u64,
}

impl Acc {
    /// The empty accumulator.
    pub const ZERO: Acc = Acc {
        n: 0,
        ns: 0,
        max: 0,
    };

    /// Count one occurrence of `ns`.
    pub fn add(&mut self, ns: u64) {
        self.n = self.n.saturating_add(1);
        self.ns = self.ns.saturating_add(ns);
        self.max = self.max.max(ns);
    }

    /// Fold `o` in (sums add, maxima max).
    pub fn merge(&mut self, o: &Acc) {
        self.n = self.n.saturating_add(o.n);
        self.ns = self.ns.saturating_add(o.ns);
        self.max = self.max.max(o.max);
    }
}

/// One accumulator per [`Phase`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Phases(pub [Acc; COUNT]);

impl Phases {
    /// The accumulator of `p`.
    #[must_use]
    pub fn get(&self, p: Phase) -> Acc {
        self.0[p as usize]
    }

    /// Count one occurrence of `ns` in `p`.
    pub fn add(&mut self, p: Phase, ns: u64) {
        self.0[p as usize].add(ns);
    }

    /// Fold `o` in, phase by phase.
    pub fn merge(&mut self, o: &Phases) {
        for (a, b) in self.0.iter_mut().zip(o.0.iter()) {
            a.merge(b);
        }
    }

    /// The sums and counts accumulated since `prev` (a later cumulative copy minus an earlier
    /// one); the maxima are `self`'s (a delta of a maximum is meaningless).
    #[must_use]
    pub fn since(&self, prev: &Phases) -> Phases {
        let mut out = *self;
        for (a, b) in out.0.iter_mut().zip(prev.0.iter()) {
            a.n = a.n.saturating_sub(b.n);
            a.ns = a.ns.saturating_sub(b.ns);
        }
        out
    }
}

thread_local! {
    static ACC: Cell<Phases> = const { Cell::new(Phases([Acc::ZERO; COUNT])) };
}

/// Record one occurrence of `ns` nanoseconds in `p`, on this thread.
pub fn record(p: Phase, ns: u64) {
    ACC.with(|c| {
        let mut v = c.get();
        v.add(p, ns);
        c.set(v);
    });
}

/// This thread's accumulators since the last [`take`], and reset them.
#[must_use]
pub fn take() -> Phases {
    ACC.with(|c| c.replace(Phases::default()))
}

/// This thread's summed ns of `p` since the last [`take`] (for an exclusive time: a nested phase's
/// share is subtracted by the caller).
#[must_use]
pub fn total_ns(p: Phase) -> u64 {
    ACC.with(|c| c.get().get(p).ns)
}

/// Nanoseconds since `t0`, saturated.
#[must_use]
pub fn ns_since(t0: std::time::Instant) -> u64 {
    u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Run `f`, recording its wall time in `p`. Returns what `f` returned, unchanged.
pub fn timed<R>(p: Phase, f: impl FnOnce() -> R) -> R {
    let t0 = std::time::Instant::now();
    let r = f();
    record(p, ns_since(t0));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_record_counts_sums_and_keeps_the_maximum() {
        let _ = take();
        record(Phase::RmMap, 5);
        record(Phase::RmMap, 9);
        record(Phase::RmUnmap, 3);
        let p = take();
        assert_eq!(
            p.get(Phase::RmMap),
            Acc {
                n: 2,
                ns: 14,
                max: 9
            }
        );
        assert_eq!(
            p.get(Phase::RmUnmap),
            Acc {
                n: 1,
                ns: 3,
                max: 3
            }
        );
        assert_eq!(p.get(Phase::Prep), Acc::ZERO);
    }

    #[test]
    fn phase_take_resets_this_thread_only() {
        let _ = take();
        record(Phase::Stitch, 7);
        let other = std::thread::spawn(|| {
            record(Phase::Stitch, 1000);
            take().get(Phase::Stitch)
        })
        .join()
        .unwrap_or_default();
        assert_eq!(other.ns, 1000);
        assert_eq!(total_ns(Phase::Stitch), 7);
        assert_eq!(take().get(Phase::Stitch).ns, 7);
        assert_eq!(take().get(Phase::Stitch), Acc::ZERO);
    }

    #[test]
    fn phase_since_subtracts_sums_and_keeps_the_later_maximum() {
        let mut a = Phases::default();
        a.add(Phase::Prep, 10);
        let mut b = a;
        b.add(Phase::Prep, 4);
        let d = b.since(&a);
        assert_eq!(
            d.get(Phase::Prep),
            Acc {
                n: 1,
                ns: 4,
                max: 10
            }
        );
        let mut m = Phases::default();
        m.merge(&a);
        m.merge(&b);
        assert_eq!(
            m.get(Phase::Prep),
            Acc {
                n: 3,
                ns: 24,
                max: 10
            }
        );
    }

    #[test]
    fn phase_timed_returns_the_closure_value_and_records_once() {
        let _ = take();
        let v = timed(Phase::Invalidate, || 42u32);
        assert_eq!(v, 42);
        assert_eq!(take().get(Phase::Invalidate).n, 1);
    }

    #[test]
    fn phase_names_are_distinct_word_stems() {
        let mut seen = std::collections::BTreeSet::new();
        for (i, p) in Phase::ALL.iter().enumerate() {
            assert_eq!(*p as usize, i);
            assert!(
                p.name()
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            );
            assert!(seen.insert(p.name()));
        }
    }
}
