//! ★ The ranked locks and the ASSERTED invariants R1 and R3
//! (`l1_concurrency.md` §3.3, decision #37).
//!
//! The design promotes the lock discipline from prose to mechanism: every L1 lock
//! declares a [`LockRank`] at construction, acquisition is legal only in **strictly
//! increasing rank with at most one lock per rank** (R3), and anything that may
//! block must be invoked through a [`BlockingSection`], whose constructor panics
//! unless the thread holds **zero** locks (R1). Enforcement is an ownership-shaped
//! API plus runtime asserts — it is explicitly NOT claimed to be compile-enforced:
//! safe Rust cannot express "no guard is alive on this thread", so the ownership
//! shape makes violations contortions instead of accidents, and the asserts are the
//! real teeth (§3.3, R1 *Enforcement* note).
//!
//! ## Why the asserts are ALWAYS-ON, not `debug_assert`
//!
//! §3.3 asks for the panics to be "caught in T1/T2/the mean test, never a silent
//! production deadlock" — and a silent production deadlock is *precisely* the
//! failure mode this module exists to prevent (the R3 rationale: "invisible until
//! the unlucky interleaving, and undiagnosable in production precisely because it
//! is silent"). The per-acquisition cost is one thread-local read-modify-write —
//! orders of magnitude cheaper than the lock acquisition it guards, on a path that
//! is low-frequency by the trap-minimization design (§1, inherited fact). So the
//! checks compile into release builds unconditionally; there is no cfg to turn the
//! teeth off.
//!
//! ## The thread-local is not shared state
//!
//! The per-thread held-rank mask lives in a `thread_local!` [`Cell`]. A
//! thread-local is not shared state — no other thread can observe or race it — so
//! the design's "no atomics, no lock-free structures, no hand-rolled
//! synchronization in L1 logic" rule (§4.2) is untouched, exactly as the R1
//! enforcement note in §3.3 spells out ("thread-local counters are not shared
//! state — the §4.2 no-atomics rule is untouched").
//!
//! ## Poisoning
//!
//! A panic while holding a `std::sync` lock poisons it; every subsequent
//! acquisition here panics loudly on the poison instead of `into_inner`-ing past
//! it. A poisoned device is a device whose invariants can no longer be trusted —
//! MISS=FAULT applies to lock state too.

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use nvkvm_util::lockwitness;

/// The total, one-way lock order of L1 (`l1_concurrency.md` §2): **device (0) →
/// proc (1) → leaf (2)**. Every lock declares its rank at construction; a thread
/// may acquire only in strictly increasing rank, at most one lock per rank (R3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LockRank {
    /// Rank 0 — the device `RwLock` guarding the spine (`Gpu::apply`, projection
    /// refresh, routing maps, delivery pump/poll/drained, target minting).
    Device = 0,
    /// Rank 1 — a per-`Proc` `Mutex` (µs bookkeeping only: publications, the
    /// doorbell act phase, worker checkout/commit — never a blocking call, R1).
    Proc = 1,
    /// Rank 2 — leaf structures: the executor inbox, the recorder.
    Leaf = 2,
}

impl LockRank {
    /// This rank's bit in the per-thread held mask.
    #[must_use]
    pub fn bit(self) -> u8 {
        lockwitness::bit(self as u8)
    }
}

// ★ The per-thread held mask itself lives in `nvkvm_util::lockwitness`, NOT here.
// The guard wrappers below MAINTAIN it; `nvkvm_isolate::Worker::execute` — the one
// door to a host RM verb — ASSERTS it. Those are two crates, and `nvkvm-isolate`
// cannot depend on this adapter, so the counter sits at the bottom of the graph
// where both can see it. That is what makes R1 guard the verb instead of a wrapper
// (§12.6's gap, closed in stage 3).

/// R3 legality check, run BEFORE the OS-level acquire — so an inverted order
/// panics deterministically instead of sometimes deadlocking first (the panic must
/// beat the OS lock to be diagnosable). Panics name R3, the ranks involved, and
/// the design doc, per the invariant's spec.
fn check_acquire(rank: LockRank) {
    let mask = lockwitness::held_mask();
    if mask >> (rank as u8) != 0 {
        panic!(
            "R3 lock-rank violation (l1_concurrency.md §3.3): acquiring a rank-{} \
             ({rank:?}) lock while already holding rank(s) {held:?} — locks may only \
             be acquired in strictly increasing rank, at most ONE lock per rank. The \
             classic case this catches: taking the device lock while holding a proc \
             lock.",
            rank as u8,
            held = held_ranks_in(mask),
        );
    }
}

/// Mark `rank` held (called only after the OS-level acquire succeeded, so a poison
/// panic never leaks a phantom held bit).
fn note_acquired(rank: LockRank) {
    lockwitness::note_acquired(rank as u8);
}

/// Clear `rank` from the held mask (guard `Drop` — runs on unwind too, so a panic
/// under a lock leaves the thread-local consistent for `#[should_panic]` tests).
fn note_released(rank: LockRank) {
    lockwitness::note_released(rank as u8);
}

/// Decode a held mask into the ranks it names (diagnostics).
fn held_ranks_in(mask: u8) -> Vec<LockRank> {
    [LockRank::Device, LockRank::Proc, LockRank::Leaf]
        .into_iter()
        .filter(|r| mask & r.bit() != 0)
        .collect()
}

/// The set of ranks THIS thread currently holds, as a bit mask (bit = rank).
/// Test/introspection surface; the invariants themselves never need callers to
/// consult it.
#[must_use]
pub fn held_rank_mask() -> u8 {
    lockwitness::held_mask()
}

/// How many ranked locks THIS thread currently holds (0..=3). A leaked guard —
/// `mem::forget` on a guard, or a rank bit that never cleared — shows up here as a
/// count that fails to return to zero, which is a real bug the tests assert
/// against explicitly.
#[must_use]
pub fn held_depth() -> u32 {
    lockwitness::held_depth()
}

/// Cumulative acquisitions of `rank` by THIS thread since it started. Monotonic;
/// pure instrumentation (used to prove a spine op acquired **zero** rank-1 locks —
/// the `Mutex::get_mut` mechanic in `device.rs`).
#[must_use]
pub fn acquisitions(rank: LockRank) -> u64 {
    lockwitness::acquisitions(rank as u8)
}

/// Loud poison message: MISS=FAULT applies to lock state (module docs).
const POISONED: &str = "ranked lock poisoned: a holder panicked mid-critical-section, so the guarded \
     state can no longer be trusted (loud by design; never into_inner past this)";

/// A [`std::sync::RwLock`] that participates in the R3 rank discipline. Rank 0
/// (the device lock) is its only current occupant, but the rank is declared at
/// construction — every lock declares its place in the order, none infers it.
#[derive(Debug)]
pub struct RankedRwLock<T> {
    rank: LockRank,
    inner: RwLock<T>,
}

impl<T> RankedRwLock<T> {
    /// A new lock of `rank` guarding `value`.
    pub fn new(rank: LockRank, value: T) -> Self {
        RankedRwLock {
            rank,
            inner: RwLock::new(value),
        }
    }

    /// Shared (read) acquisition. R3-checked: panics if this thread already holds
    /// `self.rank` or any higher rank. NOTE the same-rank rule applies to reads
    /// too — read-read reentrancy on one `RwLock` is a writer-starvation deadlock
    /// waiting for the unlucky scheduling, so "at most one lock per rank" is
    /// deliberately not relaxed for shared mode.
    pub fn read(&self) -> RankedReadGuard<'_, T> {
        check_acquire(self.rank);
        let inner = self.inner.read().expect(POISONED);
        note_acquired(self.rank);
        RankedReadGuard {
            inner,
            rank: self.rank,
        }
    }

    /// Exclusive (write) acquisition. R3-checked as [`RankedRwLock::read`].
    pub fn write(&self) -> RankedWriteGuard<'_, T> {
        check_acquire(self.rank);
        let inner = self.inner.write().expect(POISONED);
        note_acquired(self.rank);
        RankedWriteGuard {
            inner,
            rank: self.rank,
        }
    }

    /// Consume the lock, yielding the value. No acquisition happens — ownership
    /// already proves exclusivity (the same soundness argument as
    /// [`RankedMutex::get_mut`]).
    pub fn into_inner(self) -> T {
        self.inner.into_inner().expect(POISONED)
    }
}

/// Shared guard of a [`RankedRwLock`]; clears its rank bit on `Drop`. `!Send` (as
/// all `std` guards are), which the rank discipline relies on: the thread-local
/// held mask must be restored on the acquiring thread.
#[derive(Debug)]
pub struct RankedReadGuard<'a, T> {
    inner: RwLockReadGuard<'a, T>,
    rank: LockRank,
}

impl<T> Deref for RankedReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> Drop for RankedReadGuard<'_, T> {
    fn drop(&mut self) {
        note_released(self.rank);
    }
}

/// Exclusive guard of a [`RankedRwLock`]; clears its rank bit on `Drop`.
#[derive(Debug)]
pub struct RankedWriteGuard<'a, T> {
    inner: RwLockWriteGuard<'a, T>,
    rank: LockRank,
}

impl<T> Deref for RankedWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> DerefMut for RankedWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> Drop for RankedWriteGuard<'_, T> {
    fn drop(&mut self) {
        note_released(self.rank);
    }
}

/// A [`std::sync::Mutex`] that participates in the R3 rank discipline (rank 1 =
/// per-`Proc` cells, rank 2 = leaf structures like the executor inbox).
#[derive(Debug)]
pub struct RankedMutex<T> {
    rank: LockRank,
    inner: Mutex<T>,
}

impl<T> RankedMutex<T> {
    /// A new mutex of `rank` guarding `value`.
    pub fn new(rank: LockRank, value: T) -> Self {
        RankedMutex {
            rank,
            inner: Mutex::new(value),
        }
    }

    /// Acquire. R3-checked: strictly-increasing rank, at most one per rank.
    pub fn lock(&self) -> RankedMutexGuard<'_, T> {
        check_acquire(self.rank);
        let inner = self.inner.lock().expect(POISONED);
        note_acquired(self.rank);
        RankedMutexGuard {
            inner,
            rank: self.rank,
        }
    }

    /// ★ Lock-free exclusive access — **zero acquisition, zero rank state**.
    ///
    /// Sound precisely because `&mut self` already proves no other reference (let
    /// alone a guard) can exist. This is the key mechanic of the sharded device
    /// (`device.rs`): under the device *write* guard, a spine op reaches every
    /// `Proc` through `get_mut` and therefore acquires **no** rank-1 lock at all —
    /// N procs visited, zero R3 interactions, provable via [`acquisitions`].
    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut().expect(POISONED)
    }

    /// Consume the mutex, yielding the value (the `ProcSet::remove` path — a
    /// retiring or split-borrowed `Proc` leaves its lock cell behind). No
    /// acquisition, same `&mut`/ownership soundness as [`RankedMutex::get_mut`].
    pub fn into_inner(self) -> T {
        self.inner.into_inner().expect(POISONED)
    }
}

/// Guard of a [`RankedMutex`]; clears its rank bit on `Drop`.
#[derive(Debug)]
pub struct RankedMutexGuard<'a, T> {
    inner: MutexGuard<'a, T>,
    rank: LockRank,
}

impl<T> Deref for RankedMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> DerefMut for RankedMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> Drop for RankedMutexGuard<'_, T> {
    fn drop(&mut self) {
        note_released(self.rank);
    }
}

/// ★ R1's teeth: the capability to invoke something that may block.
///
/// **R1 (ASSERTED): no blocking call under ANY lock, ever** (`l1_concurrency.md`
/// §3.3). The blocking-verb path (stage 3's plan/execute/commit round-trip, a
/// pool-full wait, any potentially-blocking syscall) must be invoked through a
/// value of this type — that is the ownership shape: the natural call site holds
/// no guard, because constructing the section already required zero locks. The
/// runtime asserts are the real enforcement (compile-time "no guard alive" is not
/// expressible in safe Rust):
///
/// - [`BlockingSection::enter`] panics — naming R1 — unless this thread holds
///   **zero** ranked locks;
/// - [`BlockingSection::run`] re-asserts the same at every invocation, so a
///   section constructed early and smuggled past a later acquisition still
///   panics at the moment it matters.
///
/// `!Send` by construction (the raw-pointer `PhantomData`): the asserts are
/// against *this thread's* lock state, so the capability must not migrate to a
/// thread whose state was never checked.
///
/// **Stage-3 status.** This type is no longer where R1's teeth live for host verbs:
/// `nvkvm_isolate::Worker::execute` asserts the same witness at the verb itself
/// (§12.6's gap, closed). `BlockingSection` remains the general marker for any OTHER
/// potentially-blocking thing the shell does — notably the pool-full condvar wait —
/// and its two asserts (construction and every `run`) are unchanged.
#[derive(Debug)]
pub struct BlockingSection {
    /// Pins the section to its constructing thread (`!Send`/`!Sync`).
    _not_send: PhantomData<*mut ()>,
}

impl BlockingSection {
    /// Open a blocking section. **Panics (naming R1) if this thread holds any
    /// ranked lock** — the assert that turns "held a guard across a blocking
    /// call" from a production deadlock into an immediate, named test failure.
    #[must_use]
    pub fn enter() -> Self {
        Self::assert_lock_free("entering a BlockingSection");
        BlockingSection {
            _not_send: PhantomData,
        }
    }

    /// Run `f` — the thing that may block — re-asserting the R1 precondition at
    /// the call itself (a section constructed early must not launder a later
    /// acquisition past the invariant).
    pub fn run<R>(&mut self, f: impl FnOnce() -> R) -> R {
        Self::assert_lock_free("invoking a blocking operation");
        f()
    }

    /// The R1 assert — the SAME function `nvkvm_isolate::Worker::execute` calls, so
    /// a section and a bare verb cannot drift into two different notions of "held".
    fn assert_lock_free(what: &str) {
        lockwitness::assert_lock_free(what);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R3 pins the classic inversion: holding a proc-rank lock and then taking the
    /// device-rank lock panics immediately with a message naming R3 — the exact
    /// case §3.3 calls "the classic, invisible until the unlucky interleaving".
    #[test]
    #[should_panic(expected = "R3 lock-rank violation")]
    fn r3_taking_device_while_holding_proc_panics() {
        let device = RankedRwLock::new(LockRank::Device, 0u32);
        let proc = RankedMutex::new(LockRank::Proc, 0u32);
        let _p = proc.lock();
        let _d = device.read(); // inversion: rank 0 under rank 1 → panic
    }

    /// R3's "at most one lock per rank": two proc-rank locks held at once panic —
    /// the multi-proc-lock shape the `ProcSet`/`get_mut` mechanic exists to avoid.
    #[test]
    #[should_panic(expected = "R3 lock-rank violation")]
    fn r3_two_locks_of_the_same_rank_panics() {
        let a = RankedMutex::new(LockRank::Proc, 0u32);
        let b = RankedMutex::new(LockRank::Proc, 0u32);
        let _ga = a.lock();
        let _gb = b.lock(); // same rank doubled → panic
    }

    /// The legal shape passes, repeatedly, and the thread-local state provably
    /// returns to zero after every release — a leaked guard count (a bit that
    /// never clears) is a real bug this assert catches.
    #[test]
    fn r3_increasing_rank_order_passes_and_state_returns_to_zero() {
        let device = RankedRwLock::new(LockRank::Device, 7u32);
        let proc = RankedMutex::new(LockRank::Proc, 11u32);
        let leaf = RankedMutex::new(LockRank::Leaf, 13u32);

        assert_eq!(held_depth(), 0, "fresh thread holds nothing");
        for _ in 0..100 {
            let d = device.read();
            let mut p = proc.lock();
            let l = leaf.lock();
            assert_eq!(held_depth(), 3);
            assert_eq!(
                held_rank_mask(),
                LockRank::Device.bit() | LockRank::Proc.bit() | LockRank::Leaf.bit()
            );
            *p += *d + *l;
            drop(l);
            drop(p);
            drop(d);
            assert_eq!(held_depth(), 0, "every guard released its rank bit");
        }
        // Re-acquiring the SAME rank after release is legal (sequential reuse).
        let _w = device.write();
        assert_eq!(held_depth(), 1);
        drop(_w);
        assert_eq!(held_depth(), 0);
    }

    /// A panic UNDER a lock still restores the thread-local on unwind (guard Drop
    /// runs during unwinding) — without this, every `#[should_panic]` R3 test
    /// would poison its thread's rank state for whatever the harness runs next.
    #[test]
    fn rank_state_restored_on_unwind() {
        let proc = RankedMutex::new(LockRank::Proc, 0u32);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = proc.lock();
            panic!("deliberate");
        }));
        assert!(r.is_err());
        assert_eq!(held_depth(), 0, "unwind released the rank bit");
    }

    /// R1 fires: constructing a [`BlockingSection`] while ANY ranked lock is held
    /// panics with a message naming R1.
    #[test]
    #[should_panic(expected = "R1 no-blocking-under-lock violation")]
    fn r1_blocking_section_under_a_lock_panics() {
        let device = RankedRwLock::new(LockRank::Device, 0u32);
        let _g = device.read();
        let _section = BlockingSection::enter(); // lock alive → panic
    }

    /// R1's second tooth: a section constructed legally, then used AFTER a lock
    /// was acquired, still panics at `run` — construction is not a laundering
    /// point.
    #[test]
    #[should_panic(expected = "R1 no-blocking-under-lock violation")]
    fn r1_blocking_section_run_under_a_late_lock_panics() {
        let mut section = BlockingSection::enter(); // legal: nothing held
        let proc = RankedMutex::new(LockRank::Proc, 0u32);
        let _g = proc.lock();
        section.run(|| ()); // lock alive at the call → panic
    }

    /// R1's success polarity: with zero locks held the section constructs and runs.
    #[test]
    fn r1_blocking_section_with_no_locks_runs() {
        let mut section = BlockingSection::enter();
        assert_eq!(section.run(|| 41 + 1), 42);
        // And the lock-then-release-then-block shape (the R1-compliant verb
        // round-trip: drop every guard, THEN block) is legal.
        let device = RankedRwLock::new(LockRank::Device, 0u32);
        let g = device.write();
        drop(g);
        assert_eq!(section.run(|| 7), 7);
    }

    /// [`acquisitions`] counts per-thread acquisitions per rank — the
    /// instrumentation the "spine ops acquire no proc lock" proof rides on.
    #[test]
    fn acquisition_counters_are_per_rank() {
        let before_d = acquisitions(LockRank::Device);
        let before_p = acquisitions(LockRank::Proc);
        let device = RankedRwLock::new(LockRank::Device, 0u32);
        let proc = RankedMutex::new(LockRank::Proc, 0u32);
        for _ in 0..5 {
            let _d = device.read();
        }
        {
            let _d = device.write();
        }
        {
            let _p = proc.lock();
        }
        assert_eq!(acquisitions(LockRank::Device) - before_d, 6);
        assert_eq!(acquisitions(LockRank::Proc) - before_p, 1);
        // get_mut / into_inner are NOT acquisitions (the ★ mechanic).
        let mut m = RankedMutex::new(LockRank::Proc, 3u32);
        *m.get_mut() += 1;
        assert_eq!(m.into_inner(), 4);
        assert_eq!(acquisitions(LockRank::Proc) - before_p, 1);
    }
}
