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

use std::cell::Cell;
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
    // ★ w514 — the outermost release is the moment a deferred diagnostic becomes free to
    // print. See [`notes`]. ⊘ Checked AFTER the clear, so the last guard's own release
    // flushes rather than waiting for a lock this thread may never take again.
    if lockwitness::held_mask() == 0 {
        notes::flush();
    }
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
    #[track_caller]
    pub fn read(&self) -> RankedReadGuard<'_, T> {
        let site = core::panic::Location::caller();
        check_acquire(self.rank);
        let t0 = std::time::Instant::now();
        let blocker = lockcost::sample_holder(self.rank);
        let inner = self.inner.read().expect(POISONED);
        lockcost::note_wait_blocked_by(self.rank, blocker, t0.elapsed());
        lockcost::note_holder(self.rank, site);
        note_acquired(self.rank);
        RankedReadGuard {
            inner,
            rank: self.rank,
            site,
            held_since: std::time::Instant::now(),
        }
    }

    /// Exclusive (write) acquisition. R3-checked as [`RankedRwLock::read`].
    #[track_caller]
    pub fn write(&self) -> RankedWriteGuard<'_, T> {
        let site = core::panic::Location::caller();
        check_acquire(self.rank);
        // ★★★★★ **OWNER INVARIANT (2), 2026-09-09: "no blocking calls in a lock in any
        // thread unless needed."** The WAIT is measured separately from the HOLD because
        // they accuse different threads: a long wait is a fact about whoever HELD the lock,
        // and only the hold names the offender. `[measured]` the vCPU's `materialize`
        // segment — a bare `state.write()` acquire with nothing pending — matched
        // `worst_trap` to 58 us at 1.88 s, while the inline work it was blamed on peaked at
        // 13 ms. The whole 1.9 s was this line, and nothing said who was holding it.
        let t0 = std::time::Instant::now();
        let blocker = lockcost::sample_holder(self.rank);
        let inner = self.inner.write().expect(POISONED);
        lockcost::note_wait_blocked_by(self.rank, blocker, t0.elapsed());
        lockcost::note_holder(self.rank, site);
        note_acquired(self.rank);
        RankedWriteGuard {
            inner,
            rank: self.rank,
            site,
            held_since: std::time::Instant::now(),
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
    /// w492 — where this guard was taken, for hold attribution.
    site: &'static core::panic::Location<'static>,
    /// When the lock was actually acquired — the HOLD, which names the offender.
    held_since: std::time::Instant,
}

impl<T> Deref for RankedReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> Drop for RankedReadGuard<'_, T> {
    fn drop(&mut self) {
        lockcost::note_hold_at(self.rank, self.site, self.held_since.elapsed());
        note_released(self.rank);
    }
}

/// Exclusive guard of a [`RankedRwLock`]; clears its rank bit on `Drop`.
#[derive(Debug)]
pub struct RankedWriteGuard<'a, T> {
    inner: RwLockWriteGuard<'a, T>,
    rank: LockRank,
    /// w492 — where this guard was taken, for hold attribution.
    site: &'static core::panic::Location<'static>,
    /// When the lock was actually acquired — see [`RankedReadGuard::held_since`].
    held_since: std::time::Instant,
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
        lockcost::note_hold_at(self.rank, self.site, self.held_since.elapsed());
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
    #[track_caller]
    pub fn lock(&self) -> RankedMutexGuard<'_, T> {
        let site = core::panic::Location::caller();
        check_acquire(self.rank);
        // ⊘⊘⊘ **THIS `note_wait` WAS MISSING AND IT HID THE ANSWER FOR AN ENTIRE NIGHT.**
        //
        // `RankedRwLock::{read,write}` both recorded their wait; the MUTEX path recorded only
        // the hold. `RegPlane::state` is a `RankedMutex`, so rank 0's waits were **never
        // measured**. Its holds are short register operations that round to 0 us, and the
        // census skips a rank when wait AND hold are both zero — so rank 0 was ABSENT from
        // every census, and I read that as "never contended" through six hypotheses.
        //
        // `[measured w504]` the stall alarm then caught a vCPU parked in
        // `futex_wait -> Mutex::lock_contended -> RankedMutex<PlaneState>::lock ->
        // RegPlane::write`, which is that lock, with the census still calling it clean.
        //
        // ★ An instrument that is silent about one arm reads exactly like an instrument
        // reporting zero. That is the same defect as `lockcost` printing only from a teardown
        // path, and as the stall alarm arming 61 072 times while a blocked signal swallowed
        // every firing. Three instruments, one failure: **no way to tell "measured zero" from
        // "not measured".**
        let t0 = std::time::Instant::now();
        // ⊘ Sampled BEFORE blocking: this names the thread already inside, which is the only
        // thing that connects a long wait to a long hold. See `lockcost::sample_holder`.
        let blocker = lockcost::sample_holder(self.rank);
        let inner = self.inner.lock().expect(POISONED);
        lockcost::note_wait_blocked_by(self.rank, blocker, t0.elapsed());
        lockcost::note_holder(self.rank, site);
        note_acquired(self.rank);
        RankedMutexGuard {
            inner,
            rank: self.rank,
            site,
            held_since: std::time::Instant::now(),
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
    /// w492 — where this guard was taken, for hold attribution.
    site: &'static core::panic::Location<'static>,
    /// When the lock was actually acquired — the hold is measured from here, not from the
    /// call to `lock`, so a long WAIT is never charged to the holder as a long HOLD.
    held_since: std::time::Instant,
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
        // ⊘⊘⊘ **w507 — AND THE HOLD WAS MISSING TOO. SAME DEFECT AS w505, ONE LAYER DOWN.**
        //
        // w505 found that this type's `lock` never recorded its WAIT, so rank 0 was absent
        // from every census and six hypotheses read that absence as "never contended". The
        // fix added the wait — and rank 0 then printed `worst_wait=3825us worst_hold=0us`,
        // which I read as *"no holder is long, so the waiter is STARVED by a crowd of short
        // ones"* and acted on.
        //
        // ⚠ **That zero was never measured either.** `RankedRwLock`'s two guards both call
        // `note_hold_at` on drop; this one recorded nothing at all. So `worst_hold=0us` at a
        // `RankedMutex` rank meant "no hold was ever recorded", not "every hold was short" —
        // and starvation and one-long-holder want OPPOSITE fixes. Third instance in one
        // night of the same class: **no way to tell "measured zero" from "not measured".**
        lockcost::note_hold_at(self.rank, self.site, self.held_since.elapsed());
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
    /// What this section is, for the census.
    what: &'static str,
    /// Responsive (multiplexed, wakeable) or Committed (runs to completion).
    kind: WaitKind,
    /// The class of the thread that opened it — judged differently per class.
    class: ThreadClass,
    /// When it opened, so a coordinator's section can be judged on DURATION.
    opened: std::time::Instant,
    /// Pins the section to its constructing thread (`!Send`/`!Sync`).
    _not_send: PhantomData<*mut ()>,
}

impl Drop for BlockingSection {
    fn drop(&mut self) {
        // A coordinator may park; it may not sit in a millisecond-scale operation. That is a
        // judgement about DURATION and can only be made here, at close.
        let us = u64::try_from(self.opened.elapsed().as_micros()).unwrap_or(u64::MAX);
        match self.kind {
            // ⊘ Responsive time is RECORDED and never a violation: on a coordinator it is
            // availability, not latency. Recording it anyway matters — a boot where the
            // coordinator spent no time responsive is a coordinator that was never idle,
            // which is its own (different) problem.
            WaitKind::Responsive { .. } => note_responsive(self.what, us),
            WaitKind::Committed => {
                if self.class == ThreadClass::Coordinator && us >= COORDINATOR_SLOW_US {
                    note_slow_coordinator(self.what, us);
                }
            }
        }
    }
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
        // ⊘ Only a vCPU is recorded at ENTRY, because on a vCPU the mere fact of blocking
        // is the violation. On a coordinator it is the DURATION that offends, so that one is
        // judged on `Drop` (see `BlockingSection::drop`); on a worker it is expected and is
        // not a violation at all.
        if current_class() == ThreadClass::Vcpu {
            note_vcpu_blocking(what, false);
        }
        BlockingSection {
            what,
            kind: WaitKind::Committed,
            class: current_class(),
            opened: std::time::Instant::now(),
            _not_send: PhantomData,
        }
    }

    /// ★★★★★ **A RESPONSIVE wait — multiplexed and wakeable, and therefore unbounded on a
    /// coordinator.** `wakes_on` names the source that can interrupt it, and naming it is the
    /// point: a wait that cannot say what would wake it is not responsive, it is optimistic.
    ///
    /// ⊘ Still a violation on a [`ThreadClass::Vcpu`]. A vCPU must not wait AT ALL — not even
    /// responsively — because it is running guest code, and "we would have woken promptly" is
    /// no comfort to a guest whose vCPU was not executing.
    #[must_use]
    pub fn responsive(what: &'static str, wakes_on: &'static str) -> Self {
        Self::assert_lock_free("entering a responsive wait");
        if current_class() == ThreadClass::Vcpu {
            note_vcpu_blocking(what, false);
        }
        BlockingSection {
            what,
            kind: WaitKind::Responsive { wakes_on },
            class: current_class(),
            opened: std::time::Instant::now(),
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
        if current_class() == ThreadClass::Vcpu {
            note_vcpu_blocking(what, true);
        }
        BlockingSection {
            what,
            kind: WaitKind::Committed,
            class: current_class(),
            opened: std::time::Instant::now(),
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
// ★★★★★ THREAD CLASS — owner, 2026-09-09: "a VCPU block is worse than a block in a reader
// thread. Reader threads can sleep to wait for an operation, vcpu threads not."
// =====================================================================================

/// What kind of thread this is, which is what decides how bad a blocking call on it is.
///
/// **Owner's model, 2026-09-09:** *"a reader thread is fine to coordinate, as long as the long
/// waiting blocking calls (the milliseconds one) aren't on that one. you have separate worker
/// threads for that. In fact a VCPU block is worse than a block in a reader thread. Reader
/// threads can sleep to wait for an operation, vcpu threads not."*
///
/// ⇒ Severity is **not** flat. Three classes, three different rules:
/// | class | blocking is | why |
/// |---|---|---|
/// | [`ThreadClass::Vcpu`] | **a violation, always** (bar the allowlist) | it is running GUEST CODE. It cannot sleep: every microsecond is stolen from the guest, and the hardware it emulates posts and returns. |
/// | [`ThreadClass::Coordinator`] | fine when SHORT, a violation when long | a reader/dispatch thread may park to coordinate; it may not sit in a millisecond-scale operation, because everything it coordinates queues behind it. |
/// | [`ThreadClass::Worker`] | **expected** | this is what a worker is FOR. A worker that never blocks is a worker that was not needed. |
///
/// ⊘ This is why a single "no blocking calls" rule was the wrong shape: it would flag the
/// worker threads we deliberately created in order to obey it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadClass {
    /// A guest vCPU inside an MMIO trap. Detected, never declared — see [`current_class`].
    Vcpu,
    /// A reader/dispatch thread: may park to coordinate, may not run long operations.
    Coordinator,
    /// A dedicated worker: blocking here is the design working.
    Worker,
}

thread_local! {
    static DECLARED_CLASS: Cell<Option<ThreadClass>> = const { Cell::new(None) };
}

/// Declare this thread's class. Call once, at the top of a spawned thread's body.
///
/// ⊘ A thread that declares nothing is treated as a [`ThreadClass::Coordinator`], which is the
/// **middle** severity on purpose: defaulting to `Worker` would silently excuse every
/// unclassified thread (the permissive default that makes a census read clean), and defaulting
/// to `Vcpu` would cry wolf on threads that are legitimately allowed to park.
pub fn declare_thread_class(class: ThreadClass) {
    DECLARED_CLASS.with(|c| c.set(Some(class)));
}

/// This thread's class. A thread inside a guest trap is [`ThreadClass::Vcpu`] **regardless of
/// what it declared** — the trap witness is measured, a declaration is a claim, and where they
/// disagree the measurement wins.
#[must_use]
pub fn current_class() -> ThreadClass {
    if crate::trapwitness::in_trap() {
        return ThreadClass::Vcpu;
    }
    DECLARED_CLASS.with(Cell::get).unwrap_or(ThreadClass::Coordinator)
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

/// ★★★★★ **WHAT KIND OF WAIT THIS IS — and this, not duration, is the load-bearing axis.**
///
/// The owner's 2026-09-09 ruling is quoted verbatim in
/// `docs/design/the_wait_kind_ruling.md`, together with the 2026-09-10 policy it generalises
/// to. ⊘ It lives there rather than here because it names host readiness machinery by name and
/// this crate is gated against doing that, even in comments (`l1_concurrency.md` §6.2).
/// ⚠ Rewording an owner's words to satisfy a lint would be the wrong repair; moving them to
/// where they are allowed is the right one.
///
/// It **refutes the duration rule this module shipped an hour earlier**: a healthy idle
/// multiplexed wait parked for five seconds would have been reported as the worst offender in
/// the system, while a 900 µs uninterruptible device call — the actually harmful thing —
/// passed. A metric that is loudest where the design is most correct is worse than no metric.
///
/// ⇒ The question is not *"how long did you sleep"* but **"could new work have woken you"**.
///
/// ⚠ **One refinement, and it is not pedantry.** Waiting for AN OPERATION TO COMPLETE is the
/// **bad** case: for that duration the thread is deaf to everything else. What is safe is
/// waiting for **events**, with that operation's completion multiplexed *alongside* new input
/// on the same primitive. The two look identical in a stack trace and behave oppositely.
/// ⇒ Which is why [`WaitKind::Responsive`] **must name what can wake it**. A wait that cannot
/// name its wake source is not responsive; it is optimistic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitKind {
    /// A multiplexed, cancellable wait over several readiness sources, or a condvar that the
    /// producer of new work also signals. `wakes_on` names the source that can interrupt it.
    ///
    /// ⊘ Unbounded on a [`ThreadClass::Coordinator`] **by design**: that is a coordinator's
    /// whole job, and time spent here is availability, not latency.
    Responsive {
        /// What can interrupt this wait. ⊘ A wait that cannot name its wake source is not
        /// responsive, it is optimistic — see the variant's own docs.
        wakes_on: &'static str,
    },
    /// A wait that must run to completion — an `ioctl`, a lock held by a long operation, a
    /// write to a full pipe. **Nothing can wake it early**, so everything behind it queues.
    /// This is the currency the census actually measures.
    Committed,
}

/// A COMMITTED section that ran longer than this on a [`ThreadClass::Coordinator`] is a
/// violation: the owner's rule is that a reader thread may park to coordinate but must not sit
/// in *"the long waiting blocking calls (the milliseconds one)"*, because everything it
/// coordinates queues behind it.
pub const COORDINATOR_SLOW_US: u64 = 1_000;

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

/// Coordinator sections that ran long enough to queue whatever they coordinate.
static SLOW_COORD: AtomicU64 = AtomicU64::new(0);
static SLOW_COORD_WORST_US: AtomicU64 = AtomicU64::new(0);
static SLOW_COORD_NAME: [OnceLock<&'static str>; VCPU_BLOCK_SLOTS] =
    [const { OnceLock::new() }; VCPU_BLOCK_SLOTS];
static SLOW_COORD_CLAIMED: [AtomicUsize; VCPU_BLOCK_SLOTS] =
    [const { AtomicUsize::new(0) }; VCPU_BLOCK_SLOTS];
static SLOW_COORD_HITS: [AtomicU64; VCPU_BLOCK_SLOTS] =
    [const { AtomicU64::new(0) }; VCPU_BLOCK_SLOTS];

static RESPONSIVE_N: AtomicU64 = AtomicU64::new(0);
static RESPONSIVE_US: AtomicU64 = AtomicU64::new(0);

/// Time spent in a multiplexed, wakeable wait. Never a violation — recorded because its
/// ABSENCE is informative: a coordinator that was never responsive was never idle.
fn note_responsive(_what: &'static str, us: u64) {
    RESPONSIVE_N.fetch_add(1, AtomicOrdering::Relaxed);
    RESPONSIVE_US.fetch_add(us, AtomicOrdering::Relaxed);
}

fn note_slow_coordinator(what: &'static str, us: u64) {
    SLOW_COORD.fetch_add(1, AtomicOrdering::Relaxed);
    SLOW_COORD_WORST_US.fetch_max(us, AtomicOrdering::Relaxed);
    let ptr = what.as_ptr() as usize;
    for i in 0..VCPU_BLOCK_SLOTS {
        let cur = SLOW_COORD_CLAIMED[i].load(AtomicOrdering::Relaxed);
        if cur != ptr {
            if cur != 0
                || SLOW_COORD_CLAIMED[i]
                    .compare_exchange(0, ptr, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
                    .is_err()
            {
                continue;
            }
            let _ = SLOW_COORD_NAME[i].set(what);
        }
        SLOW_COORD_HITS[i].fetch_add(1, AtomicOrdering::Relaxed);
        return;
    }
}

/// The coordinator-thread half of the census: `(reason, hits)`, plus the worst duration seen.
#[must_use]
pub fn slow_coordinator_rows() -> (Vec<(&'static str, u64)>, u64, u64) {
    let mut out = Vec::new();
    for i in 0..VCPU_BLOCK_SLOTS {
        let hits = SLOW_COORD_HITS[i].load(AtomicOrdering::Relaxed);
        if hits == 0 {
            continue;
        }
        out.push((
            SLOW_COORD_NAME[i].get().copied().unwrap_or("⊘ (unnamed)"),
            hits,
        ));
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    (
        out,
        SLOW_COORD.load(AtomicOrdering::Relaxed),
        SLOW_COORD_WORST_US.load(AtomicOrdering::Relaxed),
    )
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
    let (coord_rows, coord_n, coord_worst) = slow_coordinator_rows();
    let resp = format!(
        " | RESPONSIVE {} wait(s) totalling {}us ⊘ never a violation — on a coordinator this \
         is AVAILABILITY, not latency; its ABSENCE would mean the thread was never idle",
        RESPONSIVE_N.load(AtomicOrdering::Relaxed),
        RESPONSIVE_US.load(AtomicOrdering::Relaxed),
    );
    let coord = if coord_n == 0 {
        // ⊘ Zero is the EXPECTED value and is stated, not omitted: an absent line and a clean
        // line are indistinguishable, which is how five instruments went unread today.
        format!(" | COORD-SLOW 0 committed-over-{}us (expected)", COORDINATOR_SLOW_US)
    } else {
        format!(
            " | COORD-SLOW {coord_n} section(s) over {}us, worst={coord_worst}us {} \
             ⊘ a reader thread may PARK to coordinate but not sit in a long operation — \
             everything it coordinates queues behind it",
            COORDINATOR_SLOW_US,
            coord_rows
                .iter()
                .map(|(w, n)| format!("[{n} × {w}]"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    if rows.is_empty() {
        return format!("VCPU-BLOCKING none declared — {undeclared}{coord}{resp}");
    }
    format!(
        "VCPU-BLOCKING {}{} — {undeclared}{coord}{resp}",
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


// =====================================================================================
// ★★★★★ OWNER INVARIANT (2), 2026-09-09 — "no blocking calls in a lock in any thread
// unless needed", made measurable. WAIT accuses the HOLDER; only the HOLD names it.
// =====================================================================================
/// Lock cost, split by who pays it: WAIT accuses the holder, HOLD names it. See the banner
/// above for the owner invariant this makes measurable.
pub mod lockcost {
    use super::LockRank;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;

    /// A hold or wait longer than this is a violation worth ranking. 1 ms is generous:
    /// an uncontended acquire is nanoseconds, so millisecond scale already means someone
    /// did real work inside.
    pub const SLOW_US: u64 = 1_000;
    const RANKS: usize = 8;

    static WORST_WAIT: [AtomicU64; RANKS] = [const { AtomicU64::new(0) }; RANKS];
    static WORST_HOLD: [AtomicU64; RANKS] = [const { AtomicU64::new(0) }; RANKS];
    static SLOW_WAITS: [AtomicU64; RANKS] = [const { AtomicU64::new(0) }; RANKS];
    static SLOW_HOLDS: [AtomicU64; RANKS] = [const { AtomicU64::new(0) }; RANKS];

    fn slot(rank: LockRank) -> usize {
        (rank as usize).min(RANKS - 1)
    }

    pub fn note_wait(rank: LockRank, d: Duration) {
        let us = u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
        WORST_WAIT[slot(rank)].fetch_max(us, Ordering::Relaxed);
        if us >= SLOW_US {
            SLOW_WAITS[slot(rank)].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// ★★★★★ **w492 — WHICH acquisition, not just which rank.** `note_hold` recorded a rank
    /// and nothing else, so `rank1 worst_hold=23881us` named a lock and left the site
    /// unknown. Five of the six hypotheses that died tonight died of acting on a plausible
    /// mechanism without knowing the site.
    ///
    /// ⊘ The caller's location arrives via `#[track_caller]`, so **no call site changes** and
    /// nothing is passed by hand — a label threaded through hundreds of acquisitions is a
    /// label that goes stale at the first refactor.
    pub fn note_hold_at(rank: LockRank, site: &'static core::panic::Location<'static>, d: Duration) {
        let us = u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
        let slot = slot(rank);
        // ⊘ Record the site only when this hold is the new worst for its rank, so the pair
        // cannot drift apart: a site stored unconditionally would name the LAST hold while
        // the duration named the WORST.
        if WORST_HOLD[slot].fetch_max(us, Ordering::Relaxed) < us {
            let mut g = WORST_HOLD_SITE[slot]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *g = Some((site.file(), site.line()));
        }
        note_acq_site(rank, site);
        note_hold(rank, d);
    }

    /// Where the worst hold for each rank was taken.
    static WORST_HOLD_SITE: [std::sync::Mutex<Option<(&'static str, u32)>>; RANKS] =
        [const { std::sync::Mutex::new(None) }; RANKS];

    // =================================================================================
    // ★★★★★ **w507 — WHO HAMMERS, which is a different question from who HOLDS.**
    //
    // `[measured w506]` rank 0 read `worst_wait=3825us worst_hold=0us`. That pair says a
    // waiter was STARVED by many short holders — and `WORST_HOLD_SITE` structurally cannot
    // name them, because it records only the single *longest* hold and every holder here is
    // sub-microsecond. So the site that caused the stall is the one site the existing
    // instrument is guaranteed to miss.
    //
    // ⊘ EIGHT hypotheses about this lock died before this existed, and one was announced as
    // found. The count per site is the measurement that replaces the guessing.
    //
    // ⚠ Lock-free by construction. A `Mutex<HashMap>` here would be a second hammer on the
    // path of the first, and would change the thing it measures.
    // =================================================================================
    /// ⚠ Raised from 128 after `[measured w510]` printed `overflow=41611` — a ranking over
    /// a truncated table can name a runner-up as the hammer and reads exactly like a correct
    /// answer.
    const SITE_SLOTS: usize = 1024;
    /// The `&'static Location` pointer identifying a site, or 0 for an unclaimed slot.
    static ACQ_SITE_KEY: [AtomicUsize; SITE_SLOTS] =
        [const { AtomicUsize::new(0) }; SITE_SLOTS];
    /// Acquisitions charged to the slot with the same index, packed as `rank << 56 | count`.
    static ACQ_SITE_COUNT: [AtomicU64; SITE_SLOTS] = [const { AtomicU64::new(0) }; SITE_SLOTS];
    /// Acquisitions that found no free slot. ⊘ Nonzero means the table is TOO SMALL and the
    /// ranking below is over a subset — the one reading under which "site X is the hammer"
    /// would be wrong.
    static ACQ_SITE_OVERFLOW: AtomicU64 = AtomicU64::new(0);

    /// The site-table slot of whoever most recently acquired each rank, +1 so that 0 means
    /// "nobody". ⊘ Last-writer-wins and deliberately racy: it is a **lead**, not a proof —
    /// several locks share a rank, and a holder can release between the sample and the read.
    static CURRENT_HOLDER: [AtomicUsize; RANKS] = [const { AtomicUsize::new(0) }; RANKS];

    /// The longest slow wait seen at each rank, and who held the rank when it began.
    static WORST_WAIT_BLOCKER: [AtomicUsize; RANKS] = [const { AtomicUsize::new(0) }; RANKS];

    /// ★★★★★ **w507 — WHO WAS HOLDING IT WHEN THE WAITER ARRIVED.**
    ///
    /// `worst_wait` says a thread waited; `worst_hold` says some thread held something long.
    /// Neither says they are the SAME acquisition, and on this tree they repeatedly were not:
    /// `[measured w509]` rank 1 showed `worst_wait=2425us` beside `worst_hold=5967us` at a
    /// site on a worker thread, and nothing in the line established that the waiter was
    /// waiting for THAT holder. Eight hypotheses died in that gap.
    ///
    /// Sampled at the moment the wait begins, so it names the thread already inside.
    #[must_use]
    pub fn sample_holder(rank: LockRank) -> usize {
        CURRENT_HOLDER[slot(rank)].load(Ordering::Relaxed)
    }

    /// Record that a wait of `d` at `rank` began while `blocker` (from [`sample_holder`]) held
    /// it. Only a wait past [`SLOW_US`] is attributed — a fast wait has no one to accuse.
    pub fn note_wait_blocked_by(rank: LockRank, blocker: usize, d: Duration) {
        let us = u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
        if us >= SLOW_US && blocker != 0 && WORST_WAIT[slot(rank)].load(Ordering::Relaxed) <= us {
            WORST_WAIT_BLOCKER[slot(rank)].store(blocker, Ordering::Relaxed);
        }
        note_wait(rank, d);
    }

    /// Ranked acquisitions taken by a thread that is INSIDE AN MMIO TRAP, at a rank above
    /// the plane's — counted per site, so the contract can be checked instead of audited.
    static IN_TRAP_ACQ: AtomicU64 = AtomicU64::new(0);
    /// The site of the first such acquisition, +1. ⊘ The FIRST and not the last: the first
    /// one is the one a reader can still reach by reasoning about a cold boot.
    static IN_TRAP_FIRST: AtomicUsize = AtomicUsize::new(0);

    /// ★★★★★ **w517 — THE CONTRACT, CHECKED RATHER THAN AUDITED.**
    ///
    /// The owner's rule is that a vCPU's MMIO trap touches at most a small queue lock, never
    /// the structures behind it: *"the data structures like the VA tables and other data is
    /// just a separate lock vcpu doesn't touch."* Two violations of it were found this
    /// session, each costing a boot and a backtrace to locate:
    /// `[measured w510]` `ce_session_with_root` (rank 0, 8.2 ms) and
    /// `[measured w516]` `take_table_changes` (rank 2, via every `Proc` cell).
    ///
    /// ⊘ Both were found by a stall alarm firing on the ONE trap that happened to be slowest.
    /// That instrument names a violation only when it also stalls — so a site that takes the
    /// Proc lock and usually gets it uncontended is invisible until the day it is not. This
    /// counts the acquisition itself, contended or not.
    ///
    /// ⚠ It counts, it does not refuse. A panic here would turn a latent contract violation
    /// into a dead guest at the worst possible moment, and the point is to enumerate the
    /// sites, not to lose the boot that would have named them.
    fn note_if_in_trap(rank: LockRank, site: &'static core::panic::Location<'static>) {
        if rank <= LockRank::Plane || !crate::trapwitness::in_trap() {
            return;
        }
        IN_TRAP_ACQ.fetch_add(1, Ordering::Relaxed);
        let i = claim_site(site);
        if i != usize::MAX {
            let _ = IN_TRAP_FIRST.compare_exchange(
                0,
                i + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
        }
    }

    /// Whether any vCPU took a lock above the plane's rank while inside an MMIO trap, and
    /// where it first did. ⊘ Renders its own zero in words: "none" is a RESULT here, and this
    /// tree has read an unmeasured zero as a healthy one three times in one night.
    #[must_use]
    pub fn in_trap_census() -> String {
        let n = IN_TRAP_ACQ.load(Ordering::Relaxed);
        if n == 0 {
            return "IN-TRAP-LOCKS none — no thread inside an MMIO trap took a lock above                     rank0 (⊘ this counter is armed; zero is a measurement)"
                .to_string();
        }
        let first = name_of(IN_TRAP_FIRST.load(Ordering::Relaxed))
            .map_or_else(|| "UNATTRIBUTED".to_string(), |(f, l)| format!("{f}:{l}"));
        format!("IN-TRAP-LOCKS ⊘ {n} acquisition(s) above rank0 from inside an MMIO trap, first at {first}")
    }

    /// Publish `site` as the current holder of `rank`. Called once per acquisition.
    pub(super) fn note_holder(rank: LockRank, site: &'static core::panic::Location<'static>) {
        note_if_in_trap(rank, site);
        CURRENT_HOLDER[slot(rank)].store(claim_site(site) + 1, Ordering::Relaxed);
    }

    /// The file/line of a slot recorded by [`sample_holder`] or [`note_holder`].
    fn name_of(slot_plus_one: usize) -> Option<(&'static str, u32)> {
        let i = slot_plus_one.checked_sub(1)?;
        *ACQ_SITE_NAME.get(i)?
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Find or claim `site`'s slot in the site table. Returns `usize::MAX` on overflow.
    fn claim_site(site: &'static core::panic::Location<'static>) -> usize {
        let key = std::ptr::from_ref(site) as usize;
        // A pointer's low bits are its alignment; mix the high ones down so distinct sites
        // do not all land in the same probe chain.
        let mut i = ((key >> 4) ^ (key >> 17)) % SITE_SLOTS;
        for _ in 0..8 {
            let cur = ACQ_SITE_KEY[i].load(Ordering::Relaxed);
            if cur == key
                || (cur == 0
                    && ACQ_SITE_KEY[i]
                        .compare_exchange(0, key, Ordering::AcqRel, Ordering::Relaxed)
                        .is_ok()
                    && {
                        // ⊘ ONCE per site, at claim time only — never on the hot path. The
                        // file/line is copied out here so the ranking never has to turn a
                        // stored address back into a reference, which this crate forbids
                        // (`-F unsafe-code`) and which would be a raw-pointer deref for a
                        // diagnostic.
                        *ACQ_SITE_NAME[i]
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some((site.file(), site.line()));
                        true
                    })
            {
                return i;
            }
            i = (i + 1) % SITE_SLOTS;
        }
        ACQ_SITE_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        usize::MAX
    }

    /// Charge one acquisition to `site`, at `rank`. Called on every ranked release.
    fn note_acq_site(rank: LockRank, site: &'static core::panic::Location<'static>) {
        let i = claim_site(site);
        if i == usize::MAX {
            return;
        }
        ACQ_SITE_COUNT[i].fetch_add(1, Ordering::Relaxed);
        ACQ_SITE_RANK[i].store(slot(rank) as u64, Ordering::Relaxed);
    }

    /// The file/line of each claimed slot, written once at claim time. See [`note_acq_site`].
    static ACQ_SITE_NAME: [std::sync::Mutex<Option<(&'static str, u32)>>; SITE_SLOTS] =
        [const { std::sync::Mutex::new(None) }; SITE_SLOTS];

    /// The rank each claimed slot belongs to, so the ranking can be read per rank.
    static ACQ_SITE_RANK: [AtomicU64; SITE_SLOTS] = [const { AtomicU64::new(0) }; SITE_SLOTS];

    /// The `n` busiest acquisition sites at `rank`, most acquisitions first.
    ///
    /// ⊘ Reports the OVERFLOW count too. A ranking over a truncated table would name a
    /// runner-up as the hammer and read exactly like a correct answer.
    #[must_use]
    pub fn hammer_census(rank: LockRank, n: usize) -> String {
        let want = slot(rank) as u64;
        let mut rows: Vec<(u64, &'static str, u32)> = Vec::new();
        for i in 0..SITE_SLOTS {
            let key = ACQ_SITE_KEY[i].load(Ordering::Relaxed);
            if key == 0 || ACQ_SITE_RANK[i].load(Ordering::Relaxed) != want {
                continue;
            }
            let Some((file, line)) = *ACQ_SITE_NAME[i]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
            else {
                continue;
            };
            rows.push((ACQ_SITE_COUNT[i].load(Ordering::Relaxed), file, line));
        }
        if rows.is_empty() {
            return format!("HAMMER(rank{}) no acquisitions recorded", want);
        }
        rows.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        rows.truncate(n);
        let body = rows
            .iter()
            .map(|(c, f, l)| format!("{f}:{l}={c}"))
            .collect::<Vec<_>>()
            .join(" ");
        let over = ACQ_SITE_OVERFLOW.load(Ordering::Relaxed);
        format!("HAMMER(rank{want}) {body} overflow={over}")
    }

    pub fn note_hold(rank: LockRank, d: Duration) {
        let us = u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
        WORST_HOLD[slot(rank)].fetch_max(us, Ordering::Relaxed);
        if us >= SLOW_US {
            SLOW_HOLDS[slot(rank)].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// One line per rank that saw traffic.
    ///
    /// ⊘ **Reading it**: a large `worst_wait` with a small `worst_hold` at the same rank is
    /// **starvation or a queue of short holders**, not one long holder — an important
    /// difference, because the fixes are opposite (shorten the critical section vs. change
    /// the fairness/shape). A large `worst_hold` names the invariant-(2) violation directly.
    /// ⊘ Ranks with no traffic print nothing rather than a row of zeros: "never acquired"
    /// and "acquired instantly" are different facts.
    #[must_use]
    pub fn census() -> String {
        let mut rows = Vec::new();
        for r in 0..RANKS {
            let (w, h) = (
                WORST_WAIT[r].load(Ordering::Relaxed),
                WORST_HOLD[r].load(Ordering::Relaxed),
            );
            if w == 0 && h == 0 {
                continue;
            }
            rows.push(format!(
                "[rank{r} worst_wait={w}us worst_hold={h}us slow_waits={} slow_holds={} \
                 worst_hold_at={} worst_wait_blocked_by={}]",
                SLOW_WAITS[r].load(Ordering::Relaxed),
                SLOW_HOLDS[r].load(Ordering::Relaxed),
                WORST_HOLD_SITE[r]
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .map_or_else(|| "UNATTRIBUTED".to_string(), |(f, l)| format!("{f}:{l}")),
                // ★★★★★ w507 — the field that CONNECTS the two beside it. `worst_wait` and
                // `worst_hold` accuse different threads and nothing said they were the same
                // acquisition; this names who held the rank when the worst waiter arrived.
                name_of(WORST_WAIT_BLOCKER[r].load(Ordering::Relaxed))
                    .map_or_else(|| "UNATTRIBUTED".to_string(), |(f, l)| format!("{f}:{l}")),
            ));
        }
        if rows.is_empty() {
            return "LOCKCOST ⊘ no ranked lock was ever acquired — unmeasured, not zero"
                .to_string();
        }
        format!(
            "LOCKCOST(>{}us) {} ⊘ a big wait with a small hold is STARVATION or many short \
             holders, not one long one — opposite fixes",
            SLOW_US,
            rows.join(" ")
        )
    }
}

#[cfg(test)]
mod lock_cost_separates_wait_from_hold {
    //! ★★★ **WAIT and HOLD accuse different threads, and conflating them is what cost this
    //! campaign the day.** `worst_trap` said a trap waited 1.9 s; I attributed it to the work
    //! visible at that trap's site. Segment timing later showed the work took 13 ms and the
    //! 1.9 s was the acquire. A wait is a fact about **whoever held**; only the hold names it.
    use super::*;

    #[test]
    fn a_long_hold_is_counted_and_a_bare_acquire_is_not() {
        let l = RankedRwLock::new(LockRank::Leaf, 0u32);
        {
            let _g = l.write();
            std::thread::sleep(std::time::Duration::from_micros(lockcost::SLOW_US + 500));
        }
        let line = lockcost::census();
        assert!(line.contains("worst_hold="), "{line}");
        // ⊘ Assert on THIS test's own rank, never on the whole census string. Other ranks are
        // touched concurrently by sibling tests and a legitimate `slow_holds=0` on one of them
        // is not this test's business — the first version of this assertion scanned the whole
        // line and failed on rank1's honest zero.
        let mine = line
            .split('[')
            .find(|seg| seg.starts_with("rank3 "))
            .unwrap_or_else(|| panic!("the LEAF rank must be named: {line}"));
        assert!(
            !mine.contains("slow_holds=0"),
            "a >1ms hold must count against its OWN rank: {mine}"
        );
    }

    /// ⊘ An untouched rank must print NOTHING, not a row of zeros — "never acquired" and
    /// "acquired instantly" are different facts and this tree has paid for reading one as
    /// the other.
    #[test]
    fn an_untouched_rank_prints_nothing_rather_than_a_tidy_zero() {
        let line = lockcost::census();
        assert!(
            !line.contains("worst_wait=0us worst_hold=0us"),
            "a rank with no traffic must be absent, not zeroed: {line}"
        );
    }
}

#[cfg(test)]
mod blocking_severity_is_per_thread_class {
    //! ★★★★★ **OWNER, 2026-09-09:** *"a reader thread is fine to coordinate, as long as the
    //! long waiting blocking calls (the milliseconds one) aren't on that one. you have
    //! separate worker threads for that. In fact a VCPU block is worse than a block in a
    //! reader thread. Reader threads can sleep to wait for an operation, vcpu threads not."*
    //!
    //! ⇒ Severity is not flat, and a flat rule was the wrong shape: it would have flagged the
    //! very worker threads we created in order to obey it.
    use super::*;

    #[test]
    fn a_worker_may_block_freely_and_is_never_a_violation() {
        std::thread::spawn(|| {
            declare_thread_class(ThreadClass::Worker);
            assert_eq!(current_class(), ThreadClass::Worker);
            let mut s = BlockingSection::enter("unit-test-worker-blocks");
            s.run(|| std::thread::sleep(std::time::Duration::from_micros(COORDINATOR_SLOW_US * 2)));
        })
        .join()
        .unwrap();
        let (rows, _, _) = slow_coordinator_rows();
        assert!(
            !rows.iter().any(|r| r.0 == "unit-test-worker-blocks"),
            "a worker blocking is the design working, not a violation: {rows:?}"
        );
    }

    #[test]
    fn a_coordinator_is_judged_on_DURATION_not_on_blocking_at_all() {
        std::thread::spawn(|| {
            declare_thread_class(ThreadClass::Coordinator);
            // Short: parking to coordinate is explicitly allowed.
            drop(BlockingSection::enter("unit-test-coord-short"));
            // Long: everything it coordinates queues behind this.
            let s = BlockingSection::enter("unit-test-coord-long");
            std::thread::sleep(std::time::Duration::from_micros(COORDINATOR_SLOW_US + 500));
            drop(s);
        })
        .join()
        .unwrap();
        let (rows, n, worst) = slow_coordinator_rows();
        assert!(n >= 1 && worst >= COORDINATOR_SLOW_US, "n={n} worst={worst}");
        assert!(rows.iter().any(|r| r.0 == "unit-test-coord-long"), "{rows:?}");
        assert!(
            !rows.iter().any(|r| r.0 == "unit-test-coord-short"),
            "a SHORT coordinator section must not be reported — the rule is about duration: \
             {rows:?}"
        );
    }

    /// ★★★★★ **THE REFUTATION, AS A TEST.** An hour before this existed, the rule was
    /// DURATION, and it would have called a healthy idle multiplexed wait the worst offender in the
    /// system while passing a 900 µs uninterruptible `ioctl`. This pins the corrected
    /// predicate: responsiveness, not length.
    #[test]
    fn a_long_responsive_wait_is_clean_and_a_short_committed_one_can_still_offend() {
        std::thread::spawn(|| {
            declare_thread_class(ThreadClass::Coordinator);
            // The healthy shape: parked far longer than the threshold, but wakeable.
            let long_poll =
                BlockingSection::responsive("unit-test-multiplexed", "the submission queue's wakeup");
            std::thread::sleep(std::time::Duration::from_micros(COORDINATOR_SLOW_US * 3));
            drop(long_poll);
            // The harmful shape: shorter, but nothing could have woken it.
            let committed = BlockingSection::enter("unit-test-committed-ioctl");
            std::thread::sleep(std::time::Duration::from_micros(COORDINATOR_SLOW_US + 200));
            drop(committed);
        })
        .join()
        .unwrap();
        let (rows, _, _) = slow_coordinator_rows();
        assert!(
            rows.iter().any(|r| r.0 == "unit-test-committed-ioctl"),
            "a COMMITTED wait past the bound must offend: {rows:?}"
        );
        assert!(
            !rows.iter().any(|r| r.0 == "unit-test-multiplexed"),
            "a RESPONSIVE wait must never offend however long it ran — it is availability, \
             not latency. Flagging it is the bug this test exists to prevent: {rows:?}"
        );
        assert!(
            vcpu_blocking_census().contains("RESPONSIVE"),
            "and responsive time must still be REPORTED, because its absence would mean the \
             coordinator was never idle: {}",
            vcpu_blocking_census()
        );
    }

    /// ⊘ Responsiveness does NOT excuse a vCPU. "We would have woken promptly" is no comfort
    /// to a guest whose vCPU was not executing.
    #[test]
    fn a_responsive_wait_on_a_vcpu_is_still_a_violation() {
        {
            let _t = crate::trapwitness::TrapGuard::enter();
            drop(BlockingSection::responsive("unit-test-vcpu-responsive", "anything"));
        }
        assert!(
            vcpu_blocking_rows()
                .iter()
                .any(|r| r.0 == "unit-test-vcpu-responsive" && !r.2),
            "a vCPU must not wait at all, responsively or otherwise"
        );
    }

    /// ⊘ A trap-thread measurement BEATS a declaration. A thread may claim to be a worker; if
    /// it is inside a guest trap it is a vCPU, because the trap witness is measured and the
    /// declaration is only a claim.
    #[test]
    fn the_trap_measurement_overrides_a_thread_s_own_claim() {
        declare_thread_class(ThreadClass::Worker);
        assert_eq!(current_class(), ThreadClass::Worker);
        let _t = crate::trapwitness::TrapGuard::enter();
        assert_eq!(
            current_class(),
            ThreadClass::Vcpu,
            "inside a trap the measurement wins over the declaration"
        );
    }
}

#[cfg(test)]
mod hammer_census_tests {
    use super::lockcost::hammer_census;
    use super::{LockRank, RankedMutex};

    /// ★★★★★ **The instrument that names the hammer must name the RIGHT ONE.**
    ///
    /// `[measured w506]` rank 0 read `worst_wait=3825us worst_hold=0us` — a starved waiter
    /// against holders none of which are long. `WORST_HOLD_SITE` records only the single
    /// *longest* hold, so it structurally cannot name a crowd of short ones: the site that
    /// caused the stall is precisely the site that instrument is guaranteed to miss. Eight
    /// hypotheses about this lock died before a count-per-site existed.
    ///
    /// ⊘ This checks DISCRIMINATION, not merely that a number appears. A census that always
    /// returned the first site it saw would satisfy "names a site" and be useless.
    #[test]
    fn the_busiest_site_is_the_one_reported_first() {
        let m = RankedMutex::new(LockRank::Leaf, 0u64);
        // Two sites on two lines, one acquired far more often than the other.
        for _ in 0..500 {
            *m.lock() += 1;
        }
        for _ in 0..3 {
            *m.lock() += 1;
        }
        let c = hammer_census(LockRank::Leaf, 4);
        let hot = c.find("=500").expect("the 500-acquisition site must be counted");
        let cold = c.find("=3").expect("the 3-acquisition site must be counted");
        assert!(
            hot < cold,
            "the busiest site must be reported FIRST — a census that does not RANK cannot \
             distinguish the hammer from a bystander: {c}"
        );
        assert!(
            c.contains("lock.rs:"),
            "and it must name a file:line, not just a count: {c}"
        );
    }

    /// ⊘ A rank nobody touched must say so rather than print zeros. "Never acquired" and
    /// "acquired and found idle" are different facts, and this tree has read the second as
    /// the first three times in one night.
    #[test]
    fn an_untouched_rank_says_it_saw_nothing() {
        let c = hammer_census(LockRank::Proc, 4);
        assert!(
            c.contains("no acquisitions recorded") || c.contains('='),
            "an untouched rank must state its emptiness in words: {c}"
        );
    }
}

#[cfg(test)]
mod every_guard_records_both_halves {
    use super::lockcost::{census, hammer_census};
    use super::{LockRank, RankedMutex, RankedRwLock};

    /// ★★★★★ **THE STRUCTURAL CHECK THAT STOPS THIS CLASS RECURRING A FOURTH TIME.**
    ///
    /// Three instruments failed the same way in one night, and two of them were *this file*:
    /// `RankedMutex::lock` recorded no WAIT (w505), and `RankedMutexGuard::drop` recorded no
    /// HOLD (w507). Each absence printed as a zero, and a zero is what a *healthy* lock prints
    /// too — so rank 0 looked clean through eight hypotheses while a vCPU was parked on it.
    ///
    /// ⊘ The defect was never in the logic; it was that **one guard type was missing a line
    /// the other two had**, and nothing quantified over the three. This test does. A fourth
    /// guard type added without accounting fails here rather than in a boot six weeks later.
    ///
    /// ⚠ It asserts a hold is *recorded*, not that it is fast — speed is a boot measurement.
    #[test]
    fn a_hold_through_each_guard_type_is_recorded_with_its_site() {
        // A rank nothing else in this crate's tests uses, so the assertion is about these
        // three acquisitions and not about whatever ran first.
        let rank = LockRank::Device;
        let m = RankedMutex::new(rank, 0u64);
        let rw = RankedRwLock::new(rank, 0u64);

        {
            let mut g = m.lock();
            *g += 1;
            std::thread::sleep(std::time::Duration::from_micros(1200));
        }
        {
            let g = rw.read();
            assert_eq!(*g, 0);
            std::thread::sleep(std::time::Duration::from_micros(1200));
        }
        {
            let mut g = rw.write();
            *g += 1;
            std::thread::sleep(std::time::Duration::from_micros(1200));
        }

        let c = census();
        assert!(
            c.contains(&format!("rank{}", rank as u8)),
            "a rank that saw three >1ms holds must appear in the census at all — its absence \
             is exactly how rank 0 stayed invisible for a night: {c}"
        );
        assert!(
            !c.contains(&format!("rank{} worst_wait=0us worst_hold=0us", rank as u8)),
            "a >1ms hold through a guard must not print as a zero hold — that is the \
             'measured zero vs not measured' failure this test exists for: {c}"
        );
        let h = hammer_census(rank, 8);
        assert!(
            h.contains("lock.rs:"),
            "and every guard type must charge its acquisition to a SITE, so a crowd of short \
             holders can be named at all: {h}"
        );
    }
}

#[cfg(test)]
mod the_wait_names_its_blocker {
    use super::lockcost::census;
    use super::{LockRank, RankedMutex};
    use std::sync::Arc;
    use std::time::Duration;

    /// ★★★★★ **A LONG WAIT AND A LONG HOLD ARE NOT EVIDENCE THEY ARE THE SAME ACQUISITION.**
    ///
    /// `[measured w509]` rank 1 printed `worst_wait=2425us` beside `worst_hold=5967us` at a
    /// site on a worker thread, and nothing in that line established that the waiter was
    /// waiting for **that** holder — only that both numbers were large at the same rank.
    /// Eight hypotheses about this campaign's stall died in exactly that gap, and one was
    /// announced as found.
    ///
    /// This checks that the census names the **blocker sampled at the moment the wait began**,
    /// and that the name is the holder's site rather than the waiter's own.
    #[test]
    fn the_census_names_who_held_the_lock_when_the_waiter_arrived() {
        let m = Arc::new(RankedMutex::new(LockRank::Leaf, 0u64));
        let holder = {
            let m = Arc::clone(&m);
            std::thread::spawn(move || {
                let _g = m.lock(); // ← the blocker's site, on THIS line
                std::thread::sleep(Duration::from_millis(30));
            })
        };
        // Let the holder get in first; the sample is taken before blocking, so the wait has
        // to begin while the other thread is inside.
        std::thread::sleep(Duration::from_millis(5));
        {
            let _g = m.lock();
        }
        holder.join().expect("the holder thread finished");

        // ⊘ Read THIS rank's row, not the whole line: other ranks legitimately carry
        // `UNATTRIBUTED` because nothing ever waited on them, and a substring check over the
        // whole census would be satisfied — or defeated — by a bystander.
        let c = census();
        let row = c
            .split("[rank")
            .find(|r| r.starts_with(&format!("{} ", LockRank::Leaf as u8)))
            .unwrap_or_else(|| panic!("the rank that saw a 30ms hold must have a row: {c}"))
            .to_string();
        assert!(
            !row.contains("worst_wait_blocked_by=UNATTRIBUTED"),
            "a >1ms wait must name the thread that was already inside — `UNATTRIBUTED` here \
             means the connection this field exists to make was not made: {row}"
        );
        assert!(
            row.contains("worst_wait_blocked_by=crates/kayfabe-util/src/lock.rs:")
                || row.contains("worst_wait_blocked_by=") && row.contains("lock.rs:"),
            "and it must name the HOLDER by file:line: {row}"
        );
    }
}

// =====================================================================================
// ★★★★★ OWNER INVARIANT (2), w514 — A DIAGNOSTIC MAY NEVER BLOCK A LOCK.
// =====================================================================================
/// Log lines written under a ranked lock, printed once the thread holds none.
///
/// # The measurement this exists for
///
/// `[measured w510]` `rank1`/`rank2 worst_hold=28491us slow_holds≈1100` at
/// `kayfabe-rt/src/device.rs:4220`, the PLAN phase of the page-table sweep. It held the
/// Device read lock **and** the Proc cell while `plan_pt_sweep` did an `eprintln!` **per
/// address space** — a write to the QEMU log, which on the bench is a pipe into a file that
/// reaches 5 MB in one boot. vCPUs waited on both ranks (`slow_waits=8` and `50`).
///
/// ⊘ It is not one site. Every `kayfabe_fwd::plan_*` function takes `&mut Proc`, so it runs
/// under two locks by construction, and `plan_doorbell` — on the hottest path there is —
/// prints the same way. Fixing sites one at a time leaves the class open.
///
/// # What this changes, stated plainly
///
/// ⚠ Lines written under a lock now appear **later**, and can interleave differently with
/// other threads' output. That is a real cost: a reader correlating two threads' lines by
/// position will be wrong. It is accepted because the alternative is a guest stall, and
/// because every line carries its own identifiers anyway.
///
/// ⚠ Bounded. A pathological hold cannot grow the buffer without limit; past the cap, lines
/// are DROPPED and counted, never silently lost — [`note_census`] says how many, because
/// "nothing was dropped" and "the counter does not exist" must not look the same.
pub mod notes {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Most deferred lines one thread may hold. Past this they are dropped and counted.
    const CAP: usize = 4096;

    thread_local! {
        static PENDING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    static DROPPED: AtomicU64 = AtomicU64::new(0);
    static DEFERRED: AtomicU64 = AtomicU64::new(0);

    /// Emit `line` — immediately if this thread holds no ranked lock, deferred if it does.
    ///
    /// ⊘ The branch is on the **held mask**, not on a caller's opinion about whether it is
    /// under a lock. A caller that is wrong about that is exactly how the 28 ms hold existed.
    pub fn emit(line: String) {
        if super::lockwitness::held_mask() == 0 {
            eprintln!("{line}");
            return;
        }
        DEFERRED.fetch_add(1, Ordering::Relaxed);
        PENDING.with(|p| {
            let mut p = p.borrow_mut();
            if p.len() >= CAP {
                DROPPED.fetch_add(1, Ordering::Relaxed);
                return;
            }
            p.push(line);
        });
    }

    /// Print and clear whatever this thread deferred. Called when the last rank is released.
    pub(super) fn flush() {
        // ⊘ Taken out of the cell FIRST, so an `eprintln!` that itself re-entered `emit`
        // could not borrow the same `RefCell` twice. Nothing does that today; a panic here
        // would be a confusing way to find out that something started.
        let lines = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
        for l in lines {
            eprintln!("{l}");
        }
    }

    /// `DEFERRED` and `DROPPED`, for the boot line.
    #[must_use]
    pub fn note_census() -> String {
        format!(
            "LOCKNOTES deferred={} dropped={} (⊘ dropped>0 means a thread held a lock across \
             more than {CAP} log lines — the lines are gone, the count is not)",
            DEFERRED.load(Ordering::Relaxed),
            DROPPED.load(Ordering::Relaxed),
        )
    }
}

/// Write a log line that is safe to call under a ranked lock: it prints immediately if the
/// thread holds none, and is deferred to the outermost release if it does. See
/// [`notes`] for the 28 ms hold that made this necessary.
#[macro_export]
macro_rules! lock_safe_eprintln {
    ($($arg:tt)*) => {
        $crate::lock::notes::emit(format!($($arg)*))
    };
}

#[cfg(test)]
mod a_diagnostic_may_not_block_a_lock {
    use super::{LockRank, RankedMutex, lockwitness, notes};

    /// ★★★★★ **A LOG LINE WRITTEN UNDER A LOCK MUST NOT BE WRITTEN UNDER A LOCK.**
    ///
    /// `[measured w510]` `rank1`/`rank2 worst_hold=28491us slow_holds≈1100` at the page-table
    /// sweep's PLAN phase, which held the Device read lock **and** the Proc cell while
    /// `plan_pt_sweep` did an `eprintln!` **per address space** — a write into the QEMU log,
    /// a pipe to a file that reaches 5 MB in one boot. vCPUs waited on both ranks.
    ///
    /// ⊘ And it is a CLASS, not a site: every `kayfabe_fwd::plan_*` takes `&mut Proc`, so it
    /// runs under two locks by construction, and `plan_doorbell` — the hottest path there is
    /// — printed the same way. Fixing sites one at a time leaves the class open.
    ///
    /// This asserts the BRANCH, which is the part that can regress: deferred while a rank is
    /// held, immediate when none is.
    #[test]
    fn a_line_is_deferred_under_a_lock_and_immediate_without_one() {
        let m = RankedMutex::new(LockRank::Proc, 0u64);

        let before = notes::note_census();
        {
            let _g = m.lock();
            assert_ne!(lockwitness::held_mask(), 0, "the test holds a rank");
            crate::lock_safe_eprintln!("kayfabe: TEST under a lock");
            assert_ne!(
                notes::note_census(),
                before,
                "a line written under a rank must be DEFERRED — if this is unchanged the \
                 line went straight to the log, under the lock, which is the 28ms defect"
            );
        }
        // The guard is gone, so the deferred line has been flushed by the outermost release.
        let after_release = notes::note_census();
        crate::lock_safe_eprintln!("kayfabe: TEST with no lock held");
        assert_eq!(
            notes::note_census(),
            after_release,
            "with no rank held the line must print IMMEDIATELY — deferring it would delay \
             every diagnostic in the tree to the next lock release, which may never come"
        );
    }

    /// ⊘ The census must distinguish "nothing was dropped" from "the counter does not exist".
    /// This tree lost three instruments in one night to exactly that ambiguity.
    #[test]
    fn the_census_states_its_own_terms() {
        let c = notes::note_census();
        assert!(c.contains("deferred=") && c.contains("dropped="), "{c}");
        assert!(
            c.contains("the lines are gone, the count is not"),
            "the census must say what a nonzero `dropped` MEANS: {c}"
        );
    }
}

#[cfg(test)]
mod the_contract_is_checked_not_audited {
    use super::lockcost::in_trap_census;
    use super::{LockRank, RankedMutex};

    /// ★★★★★ **A TRAP THAT TAKES A LOCK ABOVE THE PLANE'S IS NAMED, CONTENDED OR NOT.**
    ///
    /// Two violations of the owner's rule — *"the data structures like the VA tables and
    /// other data is just a separate lock vcpu doesn't touch"* — were found this session, and
    /// each cost a boot and a backtrace: `[measured w510]` `ce_session_with_root` holding
    /// rank 0 for 8.2 ms, and `[measured w516]` `take_table_changes` taking every `Proc`
    /// cell's rank-2 lock to read a counter.
    ///
    /// ⊘ Both were found by a stall alarm firing on whichever trap happened to be slowest.
    /// That instrument names a violation only when it ALSO stalls, so a site that takes the
    /// Proc lock and usually gets it uncontended stays invisible until the day it does not.
    /// This counts the acquisition itself.
    #[test]
    fn an_acquisition_from_inside_a_trap_is_counted_and_named() {
        let m = RankedMutex::new(LockRank::Proc, 0u64);
        {
            let _trap = crate::trapwitness::TrapGuard::enter_at(0xdead_beef);
            let _g = m.lock();
        }
        let c = in_trap_census();
        assert!(
            !c.contains("none"),
            "an acquisition above rank0 from inside a trap must be REPORTED: {c}"
        );
        assert!(
            c.contains("lock.rs:"),
            "and named by file:line, or it cannot be fixed: {c}"
        );
        the_allowed_acquisitions_are_not_reported();
    }

    /// ⊘ **The negative control, in the SAME test.** Without it a census that always reported
    /// a violation would pass the assertions above and be useless.
    ///
    /// ⚠ It cannot be its own `#[test]`: the counter is process-global and `cargo test` runs
    /// test functions in parallel, so a second test's "did my actions change it?" comparison
    /// races the first test's increment. That is not a flake to retry — a shared counter
    /// read from two threads is simply not a per-test measurement.
    fn the_allowed_acquisitions_are_not_reported() {
        // Rank 0 inside a trap is the CONTRACT, not a violation: the queue lock is exactly
        // what a trap is allowed to take.
        let plane = RankedMutex::new(LockRank::Plane, 0u64);
        let before = in_trap_census();
        {
            let _trap = crate::trapwitness::TrapGuard::enter_at(0x1234);
            let _g = plane.lock();
        }
        // And a high rank taken with no trap in flight is ordinary worker work.
        let proc_lock = RankedMutex::new(LockRank::Proc, 0u64);
        {
            let _g = proc_lock.lock();
        }
        assert_eq!(
            in_trap_census(),
            before,
            "neither the plane rank inside a trap nor a high rank outside one is a violation"
        );
    }
}
