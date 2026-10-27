//! Host implementation of the reservation model; all calls stay off the vCPU path.

use super::*;
use crate::sparse::{Ops, Reservations, Stats};
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed};

pub(super) fn with_placement<T>(
    gate: &std::sync::RwLock<()>,
    f: impl FnOnce() -> Result<T, RmError>,
) -> Result<T, RmError> {
    let _placement = gate.write().map_err(|_| RmError::NoMemory)?;
    f()
}

struct SpaceOps<'a> {
    rm: &'a HostRm,
    space: u32,
}
impl Ops for SpaceOps<'_> {
    fn reserve(&self, lo: u64, len: u64, sparse: bool) -> Result<u32, RmError> {
        let result =
            self.rm
                .reserve_va_flags_inner(self.space, lo, len, guest_reserve_flags(sparse), true);
        // ENS6: the first refusals, with their arguments, so a refusal pattern is attributable
        // (7330: 120 000 refusals and no record of which VA or size). Bounded.
        const REFUSAL_LOG_CAP: u32 = 8;
        static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if let Err(e) = &result
            && LOGGED
                .try_update(Relaxed, Relaxed, |n| (n < REFUSAL_LOG_CAP).then_some(n + 1))
                .is_ok()
        {
            eprintln!(
                "kf-host: RM refused reserve space={:#x} lo={lo:#x} len={len:#x} \
                 sparse={sparse}: {e:?}",
                self.space
            );
        }
        result
    }
    fn release(&self, handle: u32) -> Result<(), RmError> {
        let result = self.rm.free(handle);
        if result.is_err() && !self.rm.layout_poisoned.swap(true, AcqRel) {
            eprintln!("kf-host: reservation release refused; further RM placements refused");
        }
        result
    }
}

impl HostRm {
    pub(super) fn with_sparse<T>(
        &self,
        space: u32,
        f: impl FnOnce(&mut Reservations, &dyn Ops) -> Result<T, RmError>,
    ) -> Option<Result<T, RmError>> {
        self.with_sparse_gate(space, false, f)
    }

    /// Plain map and unmap routing: a failed layout repair never refuses these.
    pub(super) fn with_sparse_route<T>(
        &self,
        space: u32,
        f: impl FnOnce(&mut Reservations, &dyn Ops) -> Result<T, RmError>,
    ) -> Option<Result<T, RmError>> {
        self.with_sparse_gate(space, true, f)
    }

    fn with_sparse_gate<T>(
        &self,
        space: u32,
        routing: bool,
        f: impl FnOnce(&mut Reservations, &dyn Ops) -> Result<T, RmError>,
    ) -> Option<Result<T, RmError>> {
        if !self.sparse_enabled {
            return None;
        }
        let layout = self.sparse_spaces.lock().ok()?.get(&space)?.clone();
        Some(with_placement(&self.placement_gate, || {
            if crate::sparse::poison_refuses(self.layout_poisoned.load(Acquire), routing) {
                return Err(RmError::NoMemory);
            }
            let mut tree = layout.lock().map_err(|_| RmError::NoMemory)?;
            let before = tree.stats;
            // Every reservation verb of this call is charged to the refresh's slice, and
            // optional layout work stops starting when it is spent (ENS6).
            let base = SpaceOps { rm: self, space };
            let clock = crate::sparse::slice::mono_ns;
            let ops = crate::sparse::slice::Sliced {
                inner: &base,
                slice: &self.slice,
                clock: &clock,
            };
            let result = f(&mut tree, &ops);
            let after = tree.counters();
            for (counter, delta) in self.sparse_totals.iter().zip(after.deltas(before)) {
                let _ = counter.try_update(Relaxed, Relaxed, |n| Some(n.saturating_add(delta)));
            }
            self.sparse_totals[11].fetch_max(after.admit_ns_max, Relaxed);
            self.sparse_totals[15].fetch_max(after.peak_blocks, Relaxed);
            if tree.poisoned && !self.layout_poisoned.swap(true, AcqRel) {
                eprintln!("kf-host: sparse layout repair failed; further RM placements refused");
            }
            result
        }))
    }

    /// A new refresh begins (the VA thread, `begin_refresh`): the layout time slice is renewed.
    pub fn begin_layout_slice(&self) {
        self.slice.renew();
    }

    /// `(verbs, ns)` of reservation verbs in the busiest refresh so far (the running one included).
    #[must_use]
    pub fn layout_slice_peak(&self) -> (u32, u64) {
        self.slice.peak()
    }

    pub(super) fn initial_sparse_roots(
        &self,
        space: u32,
        ranges: &[(u64, u64)],
    ) -> Result<[GuestVaRange; 3], RmError> {
        let _placement = self.placement_gate.write().map_err(|_| RmError::NoMemory)?;
        if self.layout_poisoned.load(Acquire) {
            return Err(RmError::NoMemory);
        }
        crate::sparse::initial::reserve_initial(&SpaceOps { rm: self, space }, ranges)
    }

    /// Whether this session opted into selective sparse mappings (default OFF).
    #[must_use]
    pub fn sparse_twin(&self) -> bool {
        self.sparse_enabled
    }

    /// Admit derived sparse intent before removing any backed mapping. `false` is the
    /// counted closed fallback; the normal reservation is never freed while populated.
    pub fn admit_sparse(&self, space: VaSpace, va: u64, len: u64) -> Result<bool, RmError> {
        self.with_sparse(space.space, |tree, ops| tree.admit(ops, va, len))
            .unwrap_or(Ok(false))
    }

    /// Retire sparse intent, preserving the block for an incoming replacement. Returns
    /// bytes that must stay sparse because another mapping or sparse intent holds the block.
    pub fn retire_sparse(
        &self,
        space: VaSpace,
        va: u64,
        len: u64,
        replacement: bool,
    ) -> Result<u64, RmError> {
        self.with_sparse(space.space, |tree, ops| {
            tree.retire(ops, va, len, replacement)
        })
        .unwrap_or(Ok(0))
    }

    /// Retry unhonoured intent after a normal block empties, including before final invalidate.
    pub fn retry_sparse(&self, space: VaSpace) -> Result<(), RmError> {
        self.with_sparse(space.space, |tree, ops| tree.retry(ops))
            .unwrap_or(Ok(()))
    }

    /// Clear all sparse state before this empty host space becomes a spare for a
    /// different guest space. A refusal makes it ineligible for reuse.
    pub fn reset_sparse(&self, space: VaSpace) -> Result<(), RmError> {
        let result = self
            .with_sparse(space.space, |tree, ops| tree.clear(ops))
            .unwrap_or(Ok(()));
        if self.sparse_enabled {
            self.log_sparse(space.space, self.sparse_totals());
        }
        result
    }

    /// Session totals read with atomics only; safe for the existing heartbeat.
    #[must_use]
    pub fn sparse_totals(&self) -> Stats {
        Stats {
            made: self.sparse_totals[0].load(Relaxed),
            freed: self.sparse_totals[1].load(Relaxed),
            refused: self.sparse_totals[2].load(Relaxed),
            overflow: self.sparse_totals[3]
                .load(Relaxed)
                .saturating_add(self.sparse_global_overflow.load(Relaxed)),
            unhonoured_bytes: self.sparse_totals[4].load(Relaxed),
            retained_bytes: self.sparse_totals[5].load(Relaxed),
            coarse_maps: self.sparse_totals[6].load(Relaxed),
            admit_calls: self.sparse_totals[7].load(Relaxed),
            admit_blocks: self.sparse_totals[16].load(Relaxed),
            admit_hard_err: self.sparse_totals[8].load(Relaxed),
            admit_decline: self.sparse_totals[9].load(Relaxed),
            admit_ns_total: self.sparse_totals[10].load(Relaxed),
            admit_ns_max: self.sparse_totals[11].load(Relaxed),
            negative_hits: self.sparse_totals[12].load(Relaxed),
            negative_evicted: self.sparse_totals[13].load(Relaxed),
            occupancy_capped: self.sparse_totals[14].load(Relaxed),
            peak_blocks: self.sparse_totals[15].load(Relaxed),
            live_blocks: self.sparse_object_count.load(Relaxed) as u64,
            rm_declined: self.sparse_totals[17].load(Relaxed),
            backoff_skips: self.sparse_totals[18].load(Relaxed),
            slice_deferred: self.sparse_totals[19].load(Relaxed),
            speculative_skipped: self.sparse_totals[20].load(Relaxed),
            window_hits: self.sparse_totals[21].load(Relaxed),
            current_unhonoured_bytes: 0,
            missing_capped: false,
            withheld_windows: 0,
        }
    }

    /// Current bounded reservation counters for diagnostics on the VA/act threads.
    #[must_use]
    pub fn sparse_stats(&self, space: VaSpace) -> Option<Stats> {
        if !self.sparse_enabled {
            return None;
        }
        let tree = self.sparse_spaces.lock().ok()?.get(&space.space)?.clone();
        let mut stats = tree.lock().ok()?.counters();
        stats.overflow = stats
            .overflow
            .saturating_add(self.sparse_global_overflow.load(Relaxed));
        Some(stats)
    }

    /// Top128 current spaces by live blocks. No RM call; contended rows are counted, not waited on.
    #[must_use]
    pub fn sparse_census(&self) -> crate::sparse::Census {
        let Ok(spaces) = self.sparse_spaces.try_lock() else {
            return crate::sparse::Census {
                rows: vec![],
                total: 0,
                skipped: 1,
            };
        };
        let mut rows = std::collections::BinaryHeap::new();
        let mut skipped = 0usize;
        for (&space, tree) in spaces.iter() {
            let Ok(tree) = tree.try_lock() else {
                skipped = skipped.saturating_add(1);
                continue;
            };
            let s = tree.counters();
            rows.push(std::cmp::Reverse((s.live_blocks, space, s.peak_blocks)));
            if rows.len() > 128 {
                rows.pop();
            }
        }
        let mut rows: Vec<_> = rows
            .into_iter()
            .map(|std::cmp::Reverse((live, space, peak))| (space, live, peak))
            .collect();
        rows.sort_unstable_by_key(|r| std::cmp::Reverse(r.1));
        crate::sparse::Census {
            rows,
            total: spaces.len(),
            skipped,
        }
    }

    pub(super) fn log_sparse(&self, space: u32, stats: Stats) {
        static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if LOGGED
            .try_update(Relaxed, Relaxed, |n| (n < 128).then_some(n + 1))
            .is_ok()
        {
            eprintln!(
                "kf-host: SPARSE space={space:#x} made={} freed={} refused={} overflow={} \
                 unhonoured_bytes={} retained_bytes={} coarse_maps={}",
                stats.made,
                stats.freed,
                stats.refused,
                stats
                    .overflow
                    .saturating_add(self.sparse_global_overflow.load(Relaxed)),
                stats.unhonoured_bytes,
                stats.retained_bytes,
                stats.coarse_maps,
            );
        }
    }

    pub(super) fn unmap_sparse_pieces(
        &self,
        space: VaSpace,
        va: u64,
        len: u64,
        by_range: bool,
        defer: bool,
    ) -> Option<Result<(), RmError>> {
        self.with_sparse_route(space.space, |tree, _| {
            let pieces = tree.pieces(space.range, va, len)?;
            let mut first = Ok(());
            for (h, pva, plen) in pieces {
                match self.raw_unmap_dma_range(
                    h,
                    pva,
                    if by_range { plen } else { 0 },
                    unmap_flags(defer),
                ) {
                    Ok(()) => tree.mark(pva, plen, false),
                    Err(e) => {
                        if first.is_ok() {
                            first = Err(e);
                        }
                    }
                }
            }
            first
        })
    }
}
