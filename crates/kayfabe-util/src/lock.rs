//! ★ The ranked locks and the ASSERTED invariants R1 and R3
//! (`l1_concurrency.md` §3.3, decision #37).
//!
//! The design promotes the lock discipline from prose to mechanism: every L1 lock
//! ⊘ **This module lives in `kayfabe-util`, at the BOTTOM of the crate graph, and that is
//! forced rather than tidy.** `kayfabe-device` owns the rank-0 plane mutex and `kayfabe-rt`
//! owns ranks 1-3; rt depends on device, so the vocabulary cannot live in rt without a
//! cycle. ★ Exactly the argument this file already made for the held-mask counter — *"those
//! are two crates … so the counter sits at the bottom of the graph where both can see it"*.
//! `kayfabe_rt::lock` re-exports every name, so call sites are unchanged.
//!
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

use crate::lockwitness;

/// The total, one-way lock order of L1 (`l1_concurrency.md` §2): **device (0) →
/// proc (1) → leaf (2)**. Every lock declares its rank at construction; a thread
/// may acquire only in strictly increasing rank, at most one lock per rank (R3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LockRank {
    /// ★★★★★ **Rank 0 — the register plane's FSM mutex** (`kayfabe_device::RegPlane::state`),
    /// held across the whole command-policy chain on the vCPU's own MMIO trap.
    ///
    /// # ⊘ Why it is BELOW the device rank, and this is the only assignment that works
    ///
    /// The shipping order is **plane → core**, stated in `plane.rs`' `ce_session`: *"the
    /// command-policy chain already takes the core's ranked locks under this mutex, so
    /// plane→core is the established order and core→plane is its inversion."* Ranks are
    /// acquired in **strictly increasing** order, so the established order is legal only if
    /// the plane sorts **first**. Ranking it above `Device` — the intuitive choice, since it
    /// is "further out" — would make every vCPU MMIO trap in the device panic.
    ///
    /// ⚠ `[w236, 2026-08-11]` Until this rank existed the lock was a bare
    /// `std::sync::Mutex`, so `check_acquire` could not see an inversion involving it and
    /// [`crate::lockwitness::assert_lock_free`] passed **vacuously** while it was held. §16.86
    /// found the live consequence: `forward_ring` runs under ranks `Device` **and** `Proc`,
    /// so reading a framebuffer byte there took this mutex beneath both — and the ABBA
    /// partner already ships, on another vCPU's trap. **A guest could build the deadlock by
    /// ringing a doorbell on one vCPU while touching a register on another.**
    Plane = 0,
    /// Rank 1 — the device `RwLock` guarding the spine (`Gpu::apply`, projection
    /// refresh, routing maps, delivery pump/poll/drained, target minting).
    Device = 1,
    /// Rank 2 — a per-`Proc` `Mutex` (µs bookkeeping only: publications, the
    /// doorbell act phase, worker checkout/commit — never a blocking call, R1).
    Proc = 2,
    /// Rank 3 — leaf structures: the executor inbox, the recorder.
    Leaf = 3,
}

impl LockRank {
    /// This rank's bit in the per-thread held mask.
    #[must_use]
    pub fn bit(self) -> u8 {
        lockwitness::bit(self as u8)
    }
}

// ★ The per-thread held mask itself lives in `kayfabe_util::lockwitness`, NOT here.
// The guard wrappers below MAINTAIN it; `kayfabe_isolate::Worker::execute` — the one
// door to a host RM verb — ASSERTS it. Those are two crates, and `kayfabe-isolate`
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
    [
        LockRank::Plane,
        LockRank::Device,
        LockRank::Proc,
        LockRank::Leaf,
    ]
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
/// `kayfabe_isolate::Worker::execute` asserts the same witness at the verb itself
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
    pub fn enter(what: &'static str) -> Self {
        Self::assert_lock_free("entering a BlockingSection");
        // ★★★★★ **OWNER RULING 2026-09-09 — THE ALLOWLIST, MEASURED.**
        //
        // > *"the thing is to avoid a blocking call on vcpu thread at all, unless its
        // > required like a memslot install, even mmaps in vmm va can often run largely off
        // > vcpu thread"*
        //
        // The default inverts: blocking work does NOT belong on a vCPU thread, and the
        // exceptions are a short NAMED list (a memslot install, because KVM requires it
        // there), not "whatever happens to be there already". So a section entered inside a
        // guest trap is recorded **by reason**, and the boot prints the list.
        // ⊘ Recorded, not panicked: the census must be able to report the CURRENT residue
        // before it is zero, and a gate that aborts the boot can only ever be turned on
        // after the work is finished — which is the wrong order for measuring it.
        if crate::trapwitness::in_trap() {
            note_vcpu_blocking(what, false);
        }
        BlockingSection {
            _not_send: PhantomData,
        }
    }

    /// ★★★ **The allowlist door.** A blocking thing that genuinely cannot leave the vCPU
    /// thread — a memslot install is the canonical member, because KVM requires it there.
    ///
    /// ⊘ Separate constructor rather than a boolean argument, so the exceptions are
    /// *greppable as a set*: `enter_required_on_vcpu` is the whole allowlist, and its call
    /// sites are the list. A boolean would let an exception be introduced by flipping a
    /// literal at a call site nobody re-reads.
    /// ⚠ "It was already there" is not a reason. Each call site must carry, in a comment,
    /// why the work cannot move — the owner's own example is that even `mmap` into the VMM's
    /// VA usually CAN move, so the bar is high.
    #[must_use]
    pub fn enter_required_on_vcpu(what: &'static str) -> Self {
        Self::assert_lock_free("entering a required-on-vCPU BlockingSection");
        if crate::trapwitness::in_trap() {
            note_vcpu_blocking(what, true);
        }
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

    /// The R1 assert — the SAME function `kayfabe_isolate::Worker::execute` calls, so
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
        let _section = BlockingSection::enter("existing lock.rs test"); // lock alive → panic
    }

    /// R1's second tooth: a section constructed legally, then used AFTER a lock
    /// was acquired, still panics at `run` — construction is not a laundering
    /// point.
    #[test]
    #[should_panic(expected = "R1 no-blocking-under-lock violation")]
    fn r1_blocking_section_run_under_a_late_lock_panics() {
        let mut section = BlockingSection::enter("existing lock.rs test"); // legal: nothing held
        let proc = RankedMutex::new(LockRank::Proc, 0u32);
        let _g = proc.lock();
        section.run(|| ()); // lock alive at the call → panic
    }

    /// R1's success polarity: with zero locks held the section constructs and runs.
    #[test]
    fn r1_blocking_section_with_no_locks_runs() {
        let mut section = BlockingSection::enter("existing lock.rs test");
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


// =====================================================================================
// ★★★★★ WHAT BLOCKED ON A vCPU THREAD — the owner's 2026-09-09 allowlist, as a census.
// =====================================================================================

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::OnceLock;

const VCPU_BLOCK_SLOTS: usize = 16;
static VB_CLAIMED: [AtomicUsize; VCPU_BLOCK_SLOTS] =
    [const { AtomicUsize::new(0) }; VCPU_BLOCK_SLOTS];
static VB_NAME: [OnceLock<&'static str>; VCPU_BLOCK_SLOTS] =
    [const { OnceLock::new() }; VCPU_BLOCK_SLOTS];
static VB_HITS: [AtomicU64; VCPU_BLOCK_SLOTS] = [const { AtomicU64::new(0) }; VCPU_BLOCK_SLOTS];
/// Whether the slot came through the allowlist door.
static VB_ALLOWED: [AtomicUsize; VCPU_BLOCK_SLOTS] =
    [const { AtomicUsize::new(0) }; VCPU_BLOCK_SLOTS];
static VB_OVERFLOW: AtomicU64 = AtomicU64::new(0);

/// Lock-free, because this records **on the vCPU thread inside a trap**, which is exactly
/// where a blocking site is forbidden. An instrument that took a mutex here would be inside
/// the hazard it measures.
fn note_vcpu_blocking(what: &'static str, allowed: bool) {
    let ptr = what.as_ptr() as usize;
    for i in 0..VCPU_BLOCK_SLOTS {
        let cur = VB_CLAIMED[i].load(AtomicOrdering::Relaxed);
        if cur != ptr {
            if cur != 0
                || VB_CLAIMED[i]
                    .compare_exchange(0, ptr, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
                    .is_err()
            {
                continue;
            }
            let _ = VB_NAME[i].set(what);
            VB_ALLOWED[i].store(usize::from(allowed), AtomicOrdering::Release);
        }
        VB_HITS[i].fetch_add(1, AtomicOrdering::Relaxed);
        return;
    }
    VB_OVERFLOW.fetch_add(1, AtomicOrdering::Relaxed);
}

/// `(reason, hits, was_allowlisted)` for everything that blocked on a vCPU thread.
#[must_use]
pub fn vcpu_blocking_rows() -> Vec<(&'static str, u64, bool)> {
    let mut out = Vec::new();
    for i in 0..VCPU_BLOCK_SLOTS {
        let hits = VB_HITS[i].load(AtomicOrdering::Relaxed);
        if hits == 0 {
            continue;
        }
        out.push((
            VB_NAME[i].get().copied().unwrap_or("⊘ (reason not yet published)"),
            hits,
            VB_ALLOWED[i].load(AtomicOrdering::Acquire) == 1,
        ));
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// One line for the boot report.
///
/// ⊘⊘ **IT STATES ITS OWN COVERAGE, and that is not decoration.** This census can only see
/// blocking work that goes through [`BlockingSection`], and `[measured 2026-09-09]` the whole
/// workspace had **one** such call site while a single MMIO trap was demonstrably held for
/// **1.79 s**. So an empty line here means *"nothing DECLARED blocked"*, which is a fact
/// about the declarations and not about the boot. Printing a bare `0` would be the
/// [`a_census_over_transports_is_as_complete_as_its_list`] failure exactly — and the
/// unforgiving cross-check is `trapwitness::census()`'s `worst_trap`/`slow_traps`, which
/// measures the HOLD itself and cannot be fooled by an undeclared site.
#[must_use]
pub fn vcpu_blocking_census() -> String {
    let rows = vcpu_blocking_rows();
    let over = VB_OVERFLOW.load(AtomicOrdering::Relaxed);
    let undeclared = "⊘ COVERAGE: only sites that go through BlockingSection are visible here;                       cross-check worst_trap/slow_traps, which measure the hold itself";
    if rows.is_empty() {
        return format!("VCPU-BLOCKING none declared — {undeclared}");
    }
    format!(
        "VCPU-BLOCKING {}{} — {undeclared}",
        rows.iter()
            .map(|(what, n, allowed)| format!(
                "[{n} × {what}{}]",
                if *allowed { " (ALLOWLISTED)" } else { " ⊘ NOT ALLOWLISTED" }
            ))
            .collect::<Vec<_>>()
            .join(" "),
        if over == 0 {
            String::new()
        } else {
            format!(" ⊘ INCOMPLETE: {over} site(s) did not fit the table")
        }
    )
}

#[cfg(test)]
mod the_vcpu_allowlist {
    //! ★★★★★ **OWNER RULING 2026-09-09, as an ALLOWLIST rather than a prohibition.**
    //!
    //! > *"the thing is to avoid a blocking call on vcpu thread at all, unless its required
    //! > like a memslot install, even mmaps in vmm va can often run largely off vcpu thread"*
    //!
    //! The default inverts: blocking work does not belong on a vCPU thread, and the
    //! exceptions are a short NAMED list. `enter_required_on_vcpu`'s call sites ARE that
    //! list, which is why it is a separate constructor and not a boolean argument — a
    //! boolean lets an exception appear by flipping a literal nobody re-reads.
    use super::*;

    #[test]
    fn a_blocking_section_outside_a_trap_is_not_a_violation_and_is_not_recorded() {
        let before = vcpu_blocking_rows().len();
        let _s = BlockingSection::enter("unit-test-off-trap");
        assert!(
            !vcpu_blocking_rows().iter().any(|r| r.0 == "unit-test-off-trap"),
            "off-trap blocking is the CORRECT shape and must not be reported as a violation"
        );
        assert_eq!(vcpu_blocking_rows().len(), before);
    }

    #[test]
    fn blocking_inside_a_trap_is_recorded_and_says_whether_it_was_allowlisted() {
        {
            let _t = crate::trapwitness::TrapGuard::enter();
            let _a = BlockingSection::enter("unit-test-undeclared-on-vcpu");
            let _b = BlockingSection::enter_required_on_vcpu("unit-test-memslot-install");
        }
        let rows = vcpu_blocking_rows();
        let bad = rows
            .iter()
            .find(|r| r.0 == "unit-test-undeclared-on-vcpu")
            .expect("the undeclared site must be recorded");
        let ok = rows
            .iter()
            .find(|r| r.0 == "unit-test-memslot-install")
            .expect("the allowlisted site must be recorded too");
        assert!(!bad.2, "an ordinary section on a vCPU thread is NOT allowlisted");
        assert!(ok.2, "the allowlist door must mark its entries");

        let line = vcpu_blocking_census();
        assert!(line.contains("⊘ NOT ALLOWLISTED"), "{line}");
        assert!(line.contains("(ALLOWLISTED)"), "{line}");
    }

    /// ⊘⊘ **THE COVERAGE STATEMENT IS THE POINT.** `[measured 2026-09-09]` the whole
    /// workspace had ONE `BlockingSection` call site while a single MMIO trap was held for
    /// 1.79 s — so this census can be empty and the boot can still be blocking for seconds.
    /// An empty line that printed a bare `0` would be the
    /// `a_census_over_transports_is_as_complete_as_its_list` failure exactly.
    #[test]
    fn the_census_always_states_what_it_cannot_see() {
        assert!(
            vcpu_blocking_census().contains("COVERAGE"),
            "every rendering, empty or not, must say that only DECLARED sites are visible: {}",
            vcpu_blocking_census()
        );
        assert!(
            vcpu_blocking_census().contains("worst_trap"),
            "and it must name the cross-check that CANNOT be fooled by an undeclared site"
        );
    }
}
