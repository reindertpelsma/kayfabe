//! ★ The per-thread **lock witness** — the mechanism invariant R1 is asserted with
//! (`l1_concurrency.md` §3.3).
//!
//! R1 says: *no blocking call under ANY lock, ever.* Its enforcement is "a
//! thread-local lock-depth counter, maintained by the L1 guard wrappers, asserted
//! zero at every blocking-verb entry". Those are **two different crates**: the guard
//! wrappers are the L1 adapter's (`nvkvm-rt`), while the blocking-verb entry is the
//! isolate port's (`nvkvm_isolate::Worker::execute`, the one door to a host RM verb).
//! For the assert to guard the thing it names rather than a wrapper someone must
//! remember to use, the counter has to be visible to both — so it lives here, at the
//! bottom of the dependency graph.
//!
//! Nothing in this module names a lock, a rank, or a layer *by role*: it is a bare
//! per-thread mask over small integer ranks, exactly as generic as this crate's
//! charter requires (decision #2). The L1 adapter supplies the meaning
//! (`nvkvm_rt::lock::LockRank`: device = 0, proc = 1, leaf = 2).
//!
//! ## Not shared state
//!
//! The mask lives in a `thread_local!` [`Cell`]. No other thread can observe or race
//! it, so the "no atomics, no lock-free structures, no hand-rolled synchronization"
//! rule (§4.2) is untouched — as the R1 enforcement note spells out.
//!
//! ## Always on
//!
//! These are not `debug_assert`s (§12.2). One thread-local read costs far less than
//! the lock acquisition it guards, and R1's whole point is that its violation is
//! invisible until the unlucky interleaving — compiling the detector out of the build
//! that runs in production inverts the argument.

use std::cell::Cell;

/// How many distinct ranks the witness tracks. Three are used today (device / proc /
/// leaf); the extra slot is headroom, not an invitation — a fourth rank is a design
/// change (§3.3 declares the order), never an implementation detail.
pub const MAX_RANKS: u8 = 4;

thread_local! {
    /// Bit `r` set ⇔ this thread currently holds a lock of rank `r`. A bit, not a
    /// counter: "at most one lock per rank" (R3) makes a second acquisition at a
    /// held rank a violation, not a recursion depth.
    static HELD: Cell<u8> = const { Cell::new(0) };
    /// Cumulative per-rank acquisition counts for THIS thread — test
    /// instrumentation (e.g. proving a spine op entered rank 1 exactly zero times).
    static ACQUIRED: Cell<[u64; MAX_RANKS as usize]> = const { Cell::new([0; MAX_RANKS as usize]) };
}

/// This rank's bit in the per-thread held mask.
///
/// # Panics
/// If `rank >= MAX_RANKS` — an undeclared rank is a design error, not a runtime
/// condition to tolerate.
#[must_use]
pub fn bit(rank: u8) -> u8 {
    assert!(
        rank < MAX_RANKS,
        "rank {rank} exceeds MAX_RANKS {MAX_RANKS}"
    );
    1 << rank
}

/// Record that this thread acquired a lock of `rank`. Called by the L1 guard
/// wrappers **after** the underlying acquire succeeded, so a failed/poisoned acquire
/// never leaks a phantom held bit.
pub fn note_acquired(rank: u8) {
    HELD.with(|h| h.set(h.get() | bit(rank)));
    ACQUIRED.with(|c| {
        let mut a = c.get();
        a[rank as usize] += 1;
        c.set(a);
    });
}

/// Record that this thread released its rank-`rank` lock (guard `Drop` — runs on
/// unwind too, so a panic under a lock leaves the thread-local consistent).
pub fn note_released(rank: u8) {
    HELD.with(|h| h.set(h.get() & !bit(rank)));
}

/// The set of ranks THIS thread currently holds, as a bit mask (bit = rank).
#[must_use]
pub fn held_mask() -> u8 {
    HELD.with(Cell::get)
}

/// How many locks THIS thread currently holds (0..=[`MAX_RANKS`]). A leaked guard
/// shows up here as a count that fails to return to zero.
#[must_use]
pub fn held_depth() -> u32 {
    held_mask().count_ones()
}

/// The ranks a mask names, ascending (diagnostics).
#[must_use]
pub fn held_ranks(mask: u8) -> Vec<u8> {
    (0..MAX_RANKS).filter(|&r| mask & (1 << r) != 0).collect()
}

/// Cumulative acquisitions of `rank` by THIS thread since it started. Monotonic;
/// pure instrumentation.
///
/// # Panics
/// If `rank >= MAX_RANKS`.
#[must_use]
pub fn acquisitions(rank: u8) -> u64 {
    assert!(
        rank < MAX_RANKS,
        "rank {rank} exceeds MAX_RANKS {MAX_RANKS}"
    );
    ACQUIRED.with(Cell::get)[rank as usize]
}

/// ★ **The R1 assert.** Panics — naming R1 — unless this thread holds ZERO locks.
///
/// `what` names the operation being attempted, so the message says which blocking
/// thing was about to run under a lock. Every door to a potentially-blocking
/// operation calls this at the call itself, not at some earlier "capability"
/// construction: a capability minted while lock-free must not launder a later
/// acquisition past the invariant.
///
/// # Panics
/// If any rank bit is set for the current thread.
pub fn assert_lock_free(what: &str) {
    let mask = held_mask();
    assert!(
        mask == 0,
        "R1 no-blocking-under-lock violation (l1_concurrency.md §3.3): {what} while \
         holding rank(s) {held:?} — a blocking call may only be made with ZERO ranked \
         locks held; drop every guard, round-trip on the checked-out worker, then \
         re-acquire and RE-VALIDATE (R5).",
        held = held_ranks(mask),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mask is a set of ranks, acquisitions count per rank, and release clears
    /// exactly its own bit.
    #[test]
    fn witness_tracks_rank_bits_and_counts() {
        let before = acquisitions(1);
        assert_eq!(held_mask(), 0);
        note_acquired(0);
        note_acquired(1);
        assert_eq!(held_ranks(held_mask()), vec![0, 1]);
        assert_eq!(held_depth(), 2);
        note_released(0);
        assert_eq!(held_ranks(held_mask()), vec![1]);
        note_released(1);
        assert_eq!(held_depth(), 0);
        assert_eq!(acquisitions(1) - before, 1);
    }

    /// The R1 assert's two polarities, in the crate that owns the counter.
    #[test]
    fn assert_lock_free_passes_when_nothing_is_held() {
        assert_lock_free("a test operation");
    }

    #[test]
    #[should_panic(expected = "R1 no-blocking-under-lock violation")]
    fn assert_lock_free_panics_under_a_held_rank() {
        note_acquired(1);
        let r = std::panic::catch_unwind(|| assert_lock_free("a test operation"));
        note_released(1); // keep the harness thread clean for whatever runs next
        std::panic::resume_unwind(r.expect_err("must have panicked"));
    }
}
