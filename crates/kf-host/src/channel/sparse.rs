//! Host implementation of the reservation model; all calls stay off the vCPU path.

use super::*;
use crate::sparse::{Ops, Reservations, Stats};
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed};

struct SpaceOps<'a> {
    rm: &'a HostRm,
    space: u32,
}
impl Ops for SpaceOps<'_> {
    fn reserve(&self, lo: u64, len: u64, sparse: bool) -> Result<u32, RmError> {
        self.rm
            .reserve_va_flags_inner(self.space, lo, len, guest_reserve_flags(sparse), true)
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
        if !self.sparse_enabled {
            return None;
        }
        let layout = self.sparse_spaces.lock().ok()?.get(&space)?.clone();
        Some((|| {
            let _placement = self.placement_gate.write().map_err(|_| RmError::NoMemory)?;
            if self.layout_poisoned.load(Acquire) {
                return Err(RmError::NoMemory);
            }
            let mut tree = layout.lock().map_err(|_| RmError::NoMemory)?;
            let before = tree.stats;
            let result = f(&mut tree, &SpaceOps { rm: self, space });
            let after = tree.stats;
            for (counter, delta) in self.sparse_totals.iter().zip([
                after.made.saturating_sub(before.made),
                after.freed.saturating_sub(before.freed),
                after.refused.saturating_sub(before.refused),
                after.overflow.saturating_sub(before.overflow),
                after
                    .unhonoured_bytes
                    .saturating_sub(before.unhonoured_bytes),
                after.retained_bytes.saturating_sub(before.retained_bytes),
            ]) {
                let _ = counter.try_update(Relaxed, Relaxed, |n| Some(n.saturating_add(delta)));
            }
            if tree.poisoned && !self.layout_poisoned.swap(true, AcqRel) {
                eprintln!("kf-host: sparse layout repair failed; further RM placements refused");
            }
            result
        })())
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

    /// Retry unhonoured intent only at the next refresh, after a normal block empties.
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
            coarse_maps: 0,
        }
    }

    /// Current bounded reservation counters for diagnostics on the VA/act threads.
    #[must_use]
    pub fn sparse_stats(&self, space: VaSpace) -> Option<Stats> {
        if !self.sparse_enabled {
            return None;
        }
        let tree = self.sparse_spaces.lock().ok()?.get(&space.space)?.clone();
        let mut stats = tree.lock().ok()?.stats;
        stats.overflow = stats
            .overflow
            .saturating_add(self.sparse_global_overflow.load(Relaxed));
        Some(stats)
    }

    pub(super) fn log_sparse(&self, space: u32, stats: Stats) {
        static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if LOGGED
            .try_update(Relaxed, Relaxed, |n| (n < 128).then_some(n + 1))
            .is_ok()
        {
            eprintln!(
                "kf-host: SPARSE space={space:#x} made={} freed={} refused={} overflow={} \
                 unhonoured_bytes={} retained_bytes={}",
                stats.made,
                stats.freed,
                stats.refused,
                stats
                    .overflow
                    .saturating_add(self.sparse_global_overflow.load(Relaxed)),
                stats.unhonoured_bytes,
                stats.retained_bytes,
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
        self.with_sparse(space.space, |tree, _| {
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
