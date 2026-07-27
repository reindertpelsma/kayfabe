//! ★★ The adapter's **own** lock witness — the half `kayfabe_util::lockwitness` cannot see.
//!
//! `l1_concurrency.md` §12.43 residual 2, verbatim:
//!
//! > *"The leaf lock's rank is documented, not enforced. There is no ranked-lock type in
//! > `kayfabe-vmm` (the port must not depend on the shell), so an adapter that gets it
//! > wrong trips no gate."*
//!
//! This module is that gate, built where the lock actually is. The point is sharper than
//! bookkeeping, and it is the thing this milestone exists to find:
//!
//! `Vmm::gpa_read`/`gpa_write` are in-lock **legal**, so they are entered with one of the
//! core's ranked locks held, and serving them takes **this adapter's** map lock. The
//! syscall-shaped methods are in-lock **illegal**, and `kayfabe_util::lockwitness` proves
//! they hold no *ranked* lock. But an adapter's own lock has no rank — the witness is
//! blind to it — so an adapter that held its map lock across a memslot ioctl would have
//! rebuilt §6.3's inversion out of its own parts, with the ranked-lock assert green
//! throughout: thread A takes rank 0 and blocks on the map lock; thread B holds the map
//! lock and blocks in a multi-millisecond ioctl; and nothing in the workspace says a word.
//!
//! So the adapter carries a second, unranked-lock witness of its own, and every
//! syscall-performing method asserts **both**: zero ranked locks
//! ([`kayfabe_util::lockwitness::assert_lock_free`], inside `kayfabe-linux-raw`) **and**
//! zero of ours ([`assert_leaf_free`], here).
//!
//! It is a thread-local `Cell`, exactly as the ranked witness is, so it introduces no
//! shared state and no atomics on the hot path.

use std::cell::Cell;

thread_local! {
    /// How many of this adapter's own leaf locks this thread holds. A depth, not a bit
    /// mask: unlike the ranked ladder there is no "at most one per rank" rule to encode
    /// here, and a nested acquisition is exactly what we want to be able to observe.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// How many adapter-owned leaf locks this thread currently holds.
#[must_use]
pub fn depth() -> u32 {
    DEPTH.with(Cell::get)
}

/// ★ **The adapter's half of R1.** Panics unless this thread holds none of the adapter's
/// own locks.
///
/// Called at every syscall-performing entry point in this crate, *in addition to* the
/// ranked assert inside `kayfabe-linux-raw` — see the module docs for why one without the
/// other is a green instrument.
///
/// # Panics
/// If this thread holds any adapter leaf lock.
pub fn assert_leaf_free(what: &str) {
    let d = depth();
    assert!(
        d == 0,
        "R1 violation, adapter half (l1_concurrency.md §3.3 + §12.43 residual 2): {what} \
         while holding {d} of this adapter's own leaf lock(s). An adapter lock has no rank, \
         so `lockwitness` cannot see this — and a blocking syscall beneath one rebuilds \
         §6.3's inversion out of our own parts: a peer thread holding a RANKED lock blocks \
         on this leaf while we sit in a multi-millisecond ioctl. Plan under the lock, \
         execute with every lock dropped, commit and RE-VALIDATE."
    );
}

/// Held for the duration of an adapter leaf-lock critical section.
#[derive(Debug)]
pub struct Held;

impl Held {
    /// Note that this thread just took an adapter leaf lock.
    #[must_use]
    pub fn enter() -> Self {
        DEPTH.with(|d| d.set(d.get() + 1));
        Held
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_depth_tracks_entry_and_exit_including_nesting() {
        assert_eq!(depth(), 0);
        let outer = Held::enter();
        assert_eq!(depth(), 1);
        {
            let _inner = Held::enter();
            assert_eq!(depth(), 2);
        }
        assert_eq!(depth(), 1);
        drop(outer);
        assert_eq!(depth(), 0);
    }

    #[test]
    fn the_assert_passes_free_and_panics_held() {
        assert_leaf_free("a test syscall");
        let held = Held::enter();
        let r = std::panic::catch_unwind(|| assert_leaf_free("a test syscall"));
        drop(held);
        assert!(
            r.is_err(),
            "a syscall under the adapter's own lock must be loud"
        );
    }
}
