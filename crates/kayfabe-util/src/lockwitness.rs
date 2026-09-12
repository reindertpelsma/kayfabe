//! ★ The per-thread **lock witness** — the mechanism invariant R1 is asserted with
//! (`l1_concurrency.md` §3.3).
//!
//! R1 says: *no blocking call under ANY lock, ever.* Its enforcement is "a
//! thread-local lock-depth counter, maintained by the L1 guard wrappers, asserted
//! zero at every blocking-verb entry".
//!
//! ## ⊘⊘ WHAT THIS WITNESS CANNOT SEE — read before trusting a green assert
//!
//! ★★★ **R1 says "ANY lock". This module sees only RANKED ones.** The mask is over
//! `MAX_RANKS` small integers that the L1 adapter assigns; a `std::sync::Mutex` nobody
//! ranked is invisible to it, so [`assert_lock_free`] passes **vacuously** while such a
//! lock is held. The panic message is careful and says "ranked"; this paragraph exists
//! because the *name* is not, and a reader budgets their trust by the name.
//!
//! This is the same defect the raw-surface keyword has (`l1_os_shell.md` §4.2.1.1): the
//! instrument sees the thing it was told to see, scores perfectly, and is structurally
//! blind to the class that actually bites. `[measured]` 2026-08-06, an agent designing a
//! control-path host call was about to rely on this assert while the caller held
//! `RegPlane`'s unranked FSM mutex — which would have compiled, passed, and stalled every
//! vCPU's MMIO for the duration of a host round trip.
//!
//! ⇒ The unranked locks a vCPU thread can hold are **enumerated** in
//! `l1_concurrency.md` §3.3.1 and gated by `tests/tests/unranked_locks.rs`, so a new one
//! cannot be added without being classified. ⊘ Enumeration is not detection: this witness
//! still will not fire on them. It is a reading aid, and the rule they carry is a review
//! obligation. Those are **two different crates**: the guard
//! wrappers are the L1 adapter's (`kayfabe-rt`), while the blocking-verb entry is the
//! isolate port's (`kayfabe_isolate::Worker::execute`, the one door to a host RM verb).
//! For the assert to guard the thing it names rather than a wrapper someone must
//! remember to use, the counter has to be visible to both — so it lives here, at the
//! bottom of the dependency graph.
//!
//! Nothing in this module names a lock, a rank, or a layer *by role*: it is a bare
//! per-thread mask over small integer ranks, exactly as generic as this crate's
//! charter requires (decision #2). The L1 adapter supplies the meaning
//! (`crate::lock::LockRank`: **plane = 0**, device = 1, proc = 2, leaf = 3 — the plane rank
//! was added by w236, §16.87; ranks renumbered, the order itself is unchanged).
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
/// ⚠ Raised from 4 to 5 at w521, when `LockRank::PlaneMem` was inserted between `Plane`
/// and `Device`. ⊘ The witness panics by name on an undeclared rank rather than silently
/// dropping it — which is why adding a variant without this line failed loudly in three
/// test crates instead of quietly under-counting in a boot.
pub const MAX_RANKS: u8 = 5;

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
    let _ = HELD.try_with(|h| h.set(h.get() | bit(rank)));
    ACQUIRED.with(|c| {
        let mut a = c.get();
        a[rank as usize] += 1;
        c.set(a);
    });
}

/// Record that this thread released its rank-`rank` lock (guard `Drop` — runs on
/// unwind too, so a panic under a lock leaves the thread-local consistent).
pub fn note_released(rank: u8) {
    let _ = HELD.try_with(|h| h.set(h.get() & !bit(rank)));
}

/// The set of ranks THIS thread currently holds, as a bit mask (bit = rank).
#[must_use]
pub fn held_mask() -> u8 {
    // ⊘ `try_with`: a thread whose TLS is already being destroyed holds no ranked lock by
    // construction, and `with` would PANIC there. `[measured w524]` that panic landed inside
    // an `extern "C"` frame that cannot unwind and aborted the whole VM at teardown.
    HELD.try_with(Cell::get).unwrap_or(0)
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

/// ★★★★★ **w471 — THE vCPU ROLE, AND WHY IT LIVES ON THE SAME DOOR AS R1.**
///
/// R1 ("no blocking call under any lock") and the owner's first invariant ("no blocking
/// call on a vCPU thread") are two halves of one rule, and this tree enforced only the
/// first. ⇒ `SharedDevice::materialize_pending` was moved OUT from under the plane's rank-0
/// lock and drained at the tail of `Regs::write` instead — which satisfies `assert_lock_free`
/// completely and still forks a process and blocks in `recv()` **on the vCPU**
/// (`[measured w470]`, `worst_trap=1.62 s`). The assert was green the whole time.
///
/// ⊘ The defect this fixes is not the spawn. It is that **finding a blocking call on a vCPU
/// required sampling stacks at 10 Hz and getting lucky**, four separate times in one session,
/// each time on a different call. Every door to a potentially-blocking operation already
/// calls [`assert_lock_free`]; asking the vCPU question at that same door makes the whole
/// class report itself in one boot, by name, instead of being discovered one at a time.
///
/// ⚠ **Census by default, fatal only when armed.** Flipping straight to a panic would abort
/// the boot on the first known-bad door and hide every other one — the census has to come
/// first, or the list is one entry long by construction. `KAYFABE_VCPU_BLOCK_FATAL=1` makes
/// it a panic once the list is empty.
mod vcpu {
    use core::cell::Cell;
    use core::sync::atomic::{AtomicU64, Ordering};
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock, PoisonError};

    thread_local! {
        static IS_VCPU: Cell<bool> = const { Cell::new(false) };
    }

    static TOTAL: AtomicU64 = AtomicU64::new(0);
    static OVERFLOW: AtomicU64 = AtomicU64::new(0);
    /// ⊘⊘ **KEYED ON CONTENT, NOT ON A POINTER.** The first cut keyed on a `&'static str`'s
    /// address, like the trap witness's inline-reason table — and the two public doors take
    /// `&str`, so `[measured w471]` all **1239** hits folded into ONE bucket named
    /// *"(dynamically-named blocking door)"*, while the five real door names existed only in
    /// capped `eprintln` lines. A census whose rows are unnamed is a count, not a census, and
    /// the print cap means a door first reached late in a boot would not appear at all.
    /// ⊘ A `Mutex` is acceptable here precisely because this is reached only when the
    /// invariant is ALREADY violated — it is not on a path that is supposed to exist.
    static TABLE: OnceLock<Mutex<BTreeMap<String, u64>>> = OnceLock::new();

    fn table() -> &'static Mutex<BTreeMap<String, u64>> {
        TABLE.get_or_init(|| Mutex::new(BTreeMap::new()))
    }

    /// Declare the calling thread a vCPU. Idempotent; called at the MMIO trap entry.
    pub fn mark() {
        IS_VCPU.with(|c| c.set(true));
    }

    #[must_use]
    pub fn is_vcpu() -> bool {
        IS_VCPU.with(Cell::get)
    }

    /// Record one blocking door reached on a vCPU thread. Returns the running total.
    pub fn note(what: &str) -> u64 {
        {
            let mut g = table().lock().unwrap_or_else(PoisonError::into_inner);
            if g.len() < 256 || g.contains_key(what) {
                *g.entry(what.to_string()).or_insert(0) += 1;
            } else {
                OVERFLOW.fetch_add(1, Ordering::Relaxed);
            }
        }
        TOTAL.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// One line naming every blocking door a vCPU reached, most-hit first. ⊘ Says so
    /// explicitly when the list is EMPTY, because "no line printed" and "nothing happened"
    /// must not read the same.
    #[must_use]
    pub fn census() -> String {
        let mut rows: Vec<(u64, String)> = table()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(k, v)| (*v, k.clone()))
            .collect();
        rows.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        if rows.is_empty() {
            return "VCPU-BLOCKING none — no blocking door was reached on a vCPU thread \
                    (the owner's first invariant holds by measurement, not by assumption)"
                .to_string();
        }
        let total = TOTAL.load(Ordering::Relaxed);
        let mut out = format!("VCPU-BLOCKING total={total} doors={}", rows.len());
        for (h, t) in rows {
            out.push_str(&format!(" [{h} × {t}]"));
        }
        let of = OVERFLOW.load(Ordering::Relaxed);
        if of > 0 {
            out.push_str(&format!(" (+{of} unnamed, table full)"));
        }
        out
    }
}

pub use vcpu::{census as vcpu_blocking_census, is_vcpu as on_vcpu_thread, mark as mark_vcpu_thread};

/// ★ **The R1 assert.** Panics — naming R1 — unless this thread holds zero **ranked**
/// locks.
///
/// ⊘ The name says `lock_free` and that is stronger than what it checks: an unranked
/// `Mutex` is invisible here and this returns cleanly while one is held. See the module
/// doc's "what this witness cannot see"; the enumerated set is `l1_concurrency.md` §3.3.1.
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
    assert_not_on_vcpu(what);
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

/// ★ **The named exception's assert** — `l1_os_shell.md` §4.5.
///
/// [`assert_lock_free`] is the rule; this is the *enumerated* set of exceptions. §4.5
/// permits a deliberately non-blocking discharge (the irqfd-shaped completion edge is the
/// only one today) to run under a lock, on condition that the entry point is named
/// `*_under_lock` and **names the rank it permits in its signature**. `permitted` is that
/// declaration as a rank mask, and this function is what makes it a mechanism rather than
/// a comment: a call site that drifts into holding a rank it did not declare panics
/// exactly as an ordinary blocking door would.
///
/// `assert_only_ranks(what, 0)` is [`assert_lock_free`] — deliberately, so the ordinary
/// door and the exception cannot diverge.
///
/// # Panics
/// If this thread holds any rank outside `permitted`.
pub fn assert_only_ranks(what: &str, permitted: u8) {
    assert_not_on_vcpu(what);
    let held = held_mask();
    let undeclared = held & !permitted;
    assert!(
        undeclared == 0,
        "R1 no-blocking-under-lock violation (l1_concurrency.md §3.3, \
         l1_os_shell.md §4.5): {what} while holding UNDECLARED rank(s) {undeclared:?} \
         (declared: {declared:?}, held: {held_r:?}). An `*_under_lock` door is an \
         enumerated exception, not an exemption: widen the declared mask only with the \
         argument for why the call cannot block under that rank.",
        undeclared = held_ranks(undeclared),
        declared = held_ranks(permitted),
        held_r = held_ranks(held),
    );
}

/// ★★★★★ **w471 — the owner's FIRST invariant, asserted at the same door as R1.**
///
/// ⚠ `what` is `&str`, not `&'static str`, because that is what the two public doors take.
/// The census keys on a `&'static str`, so a non-static name is folded into one bucket
/// rather than dropped — a door reached with a dynamic name still COUNTS, it just does not
/// get its own row. ⊘ Counted rather than skipped: a census that silently ignores the
/// awkward cases reads as clean exactly where it is blind.
fn assert_not_on_vcpu(what: &str) {
    if !vcpu::is_vcpu() {
        return;
    }
    let n = vcpu::note(what);
    static FATAL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let fatal = *FATAL.get_or_init(|| std::env::var("KAYFABE_VCPU_BLOCK_FATAL").is_ok());
    assert!(
        !fatal,
        "R0 no-blocking-on-vCPU violation: `{what}` was reached on a vCPU thread. A vCPU \
         MMIO trap may only classify, update O(1) shadow state, enqueue and wake — the work \
         belongs on a worker, and the guest blocks on ITS OWN wait primitive."
    );
    // ⊘ Census mode prints each door once; the COUNT is the census line, not these.
    // `[measured w471]` a per-hit print at 1239 hits is a second workload, and a global cap
    // would hide a door first reached late in a boot — which is why the table above is
    // keyed on content and printed in full at teardown.
    static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::BTreeSet<String>>> =
        std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(|| std::sync::Mutex::new(std::collections::BTreeSet::new()));
    let first = {
        let mut g = seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.insert(what.to_string())
    };
    if first {
        eprintln!(
            "kayfabe: ⊘ VCPU-BLOCKING (door #{}) `{what}` reached on a vCPU thread — this \
             trap cannot be microseconds; the work belongs on a worker (hit {n} overall)",
            vcpu::census().matches(" × ").count()
        );
    }
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

    /// ★ The enumerated exception (§4.5): a declared rank passes, an undeclared one
    /// panics, and declaring nothing is exactly `assert_lock_free`.
    #[test]
    fn assert_only_ranks_permits_exactly_what_it_declares() {
        assert_only_ranks("a nothing-held call", 0);
        note_acquired(0);
        assert_only_ranks("a declared-rank-0 call", bit(0));
        // Declaring MORE than is held is fine — the declaration is a ceiling.
        assert_only_ranks("a declared-0-and-1 call", bit(0) | bit(1));
        let undeclared =
            std::panic::catch_unwind(|| assert_only_ranks("a rank-1-only call", bit(1)));
        note_released(0);
        assert!(
            undeclared.is_err(),
            "holding rank 0 while declaring only rank 1 must panic"
        );
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
