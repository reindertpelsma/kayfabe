//! ★ 2026-11-18 — **the placement lock** (`docs/design/V3_SPARSE_RESIDENCY_RCA.md` §8.3, §8.9).
//!
//! `[measured 2026-11-18, kf-va-tiling-probe M3, traces/sparse_c2_tiling_probe_20261118]` while a
//! fence is freed and re-reserved (one re-tile of [`crate::tiling`]), a NON-FIXED placement in the same
//! space — a map at an address RM chooses, or a context buffer RM allocates for an object — lands in
//! the freed VA (`0x120010000`, the second carve), and the re-reserve is then refused. Only this
//! client's own calls place in its spaces, so ONE kf-owned lock per [`crate::HostRm`] closes the
//! window: every re-tile holds it from its first free to its last reserve (or its rollback), and
//! every call that can place without FIXED — `raw_alloc` (an object's context buffers) and a map
//! without an address — takes it too.
//!
//! The blocking rule (CLAUDE.md: nothing blocks on a vCPU or under a lock a vCPU takes): re-tiles run
//! on the VA-manager thread (the apply of a walk, `kf_mem::vasmgr`), and RM allocations on act and
//! worker threads; a vCPU only classifies and enqueues. So no vCPU takes this lock. That is CHECKED,
//! not assumed: [`PlacementLock::hold`] on a thread marked as a vCPU
//! (`kf_util::lockwitness::on_vcpu_thread`) or inside a guest trap (`kf_util::trapwitness::in_trap`)
//! is counted ([`PlacementLock::vcpu_holds`]) and named in a rate-limited line, and a re-tile there is
//! refused before any host call ([`PlacementLock::hold_for_retile`]).
//!
//! The lock is RE-ENTRANT on its own thread (a thread-local depth): a re-tile holds it and then calls
//! `raw_alloc` to reserve a block, which takes it again.

use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

thread_local! {
    /// How deep this thread holds a [`PlacementLock`], and which one (its address): re-entry is
    /// for the SAME lock only.
    static DEPTH: Cell<(usize, u32)> = const { Cell::new((0, 0)) };
}

/// Why a re-tile was refused before any host call.
pub const RETILE_ON_VCPU: &str =
    "a re-tile on a vCPU or guest-trap thread is refused (placement lock)";

/// One client's placement lock.
#[derive(Debug, Default)]
pub struct PlacementLock {
    m: Mutex<()>,
    holds: AtomicU64,
    vcpu_holds: AtomicU64,
}

/// A held [`PlacementLock`]. Not `Send`: the depth it maintains is the holding thread's.
#[derive(Debug)]
pub struct PlacementGuard<'a> {
    _g: Option<MutexGuard<'a, ()>>,
    /// The depth this guard restores on drop (the holder before it).
    restore: (usize, u32),
    _not_send: PhantomData<*const ()>,
}

impl Drop for PlacementGuard<'_> {
    fn drop(&mut self) {
        let r = self.restore;
        DEPTH.with(|d| d.set(r));
    }
}

/// Mark the calling thread as a vCPU in `kf_util`'s witness (what the MMIO trap entry does), so a
/// GPU-free test can show that a re-tile there is refused. Production marks through `kf_util`.
pub fn mark_vcpu_thread() {
    kf_util::lockwitness::mark_vcpu_thread();
}

fn on_vcpu() -> bool {
    kf_util::lockwitness::on_vcpu_thread() || kf_util::trapwitness::in_trap()
}

impl PlacementLock {
    /// Take the lock (or re-enter it on this thread). `what` names the caller in the vCPU line.
    pub fn hold(&self, what: &'static str) -> PlacementGuard<'_> {
        let me = std::ptr::from_ref(self) as usize;
        let before = DEPTH.with(Cell::get);
        if before.0 == me && before.1 > 0 {
            DEPTH.with(|d| d.set((me, before.1.saturating_add(1))));
            return PlacementGuard {
                _g: None,
                restore: before,
                _not_send: PhantomData,
            };
        }
        if on_vcpu() {
            self.vcpu_holds.fetch_add(1, Ordering::Relaxed);
            kf_arch::rate_eprintln!(
                "kf-host: placement lock taken on a vCPU or guest-trap thread by {what}: the blocking rule says no vCPU takes it"
            );
        }
        self.holds.fetch_add(1, Ordering::Relaxed);
        let g = self.m.lock().unwrap_or_else(PoisonError::into_inner);
        DEPTH.with(|d| d.set((me, 1)));
        PlacementGuard {
            _g: Some(g),
            restore: before,
            _not_send: PhantomData,
        }
    }

    /// [`PlacementLock::hold`] for a re-tile: refused (no lock, no host call) on a vCPU or
    /// guest-trap thread — re-tiles belong to the VA-manager thread.
    ///
    /// # Errors
    /// [`RETILE_ON_VCPU`].
    pub fn hold_for_retile(&self) -> Result<PlacementGuard<'_>, &'static str> {
        if on_vcpu() {
            self.vcpu_holds.fetch_add(1, Ordering::Relaxed);
            return Err(RETILE_ON_VCPU);
        }
        Ok(self.hold("re-tile"))
    }

    /// Whether this thread holds this lock now.
    #[must_use]
    pub fn held_here(&self) -> bool {
        let d = DEPTH.with(Cell::get);
        d.0 == std::ptr::from_ref(self) as usize && d.1 > 0
    }

    /// Outermost acquisitions so far.
    #[must_use]
    pub fn holds(&self) -> u64 {
        self.holds.load(Ordering::Relaxed)
    }

    /// Acquisitions (or refused re-tiles) on a vCPU or guest-trap thread: 0 by design.
    #[must_use]
    pub fn vcpu_holds(&self) -> u64 {
        self.vcpu_holds.load(Ordering::Relaxed)
    }

    /// Whether another thread holds the lock now (a test probe; never blocks).
    #[must_use]
    pub fn busy(&self) -> bool {
        matches!(self.m.try_lock(), Err(std::sync::TryLockError::WouldBlock))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_reentrant_on_its_thread_and_exclusive_across_threads() {
        let l = PlacementLock::default();
        let g1 = l.hold("outer");
        let g2 = l.hold("inner (raw_alloc under a re-tile)");
        assert!(l.held_here());
        std::thread::scope(|s| {
            s.spawn(|| assert!(l.busy(), "another thread sees it held"));
        });
        drop(g2);
        assert!(l.held_here());
        drop(g1);
        assert!(!l.held_here());
        std::thread::scope(|s| {
            s.spawn(|| assert!(!l.busy()));
        });
        assert_eq!(l.holds(), 1, "one outermost acquisition");
    }

    /// ★ The blocking rule, checked: a re-tile on a vCPU thread is refused before any host call,
    /// and any other acquisition there is counted.
    #[test]
    fn a_vcpu_thread_never_runs_a_retile() {
        let l = PlacementLock::default();
        std::thread::scope(|s| {
            s.spawn(|| {
                kf_util::lockwitness::mark_vcpu_thread();
                assert_eq!(l.hold_for_retile().err(), Some(RETILE_ON_VCPU));
                drop(l.hold("an alloc on a vCPU (a violation, counted)"));
            });
        });
        assert_eq!(l.vcpu_holds(), 2);
        assert!(
            l.hold_for_retile().is_ok(),
            "the VA-manager thread re-tiles"
        );
    }
}
