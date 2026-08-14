//! ★★★★★ **w326 — A DRIVER FOR THE REVOCATION DRAIN THAT IS NOT THE GUEST.**
//!
//! # The defect, in one sentence
//!
//! `[measured w323, 2026-08-14, git grep, whole tree; re-confirmed here]` the revocation
//! drain has **exactly one driver and it is a guest MMIO write**:
//! `SharedDevice::drain_retired_budgeted`, `pin_reclaim_gone` and `reap_retired_held` are
//! all called from inside `Regs::write` and from nowhere else in production.
//!
//! ⇒ **a guest that frees its host-backed objects and then simply stops touching MMIO leaves
//! the residue undrained indefinitely** — and the residue is a live host-GPU translation into
//! guest pages Linux has already reused. That is fail-DANGEROUS, and it is the direction
//! `publication_off_the_bql.md` §4 rules may never be deferred.
//!
//! ★★★ **A bound discharged only by the adversary is not a bound.** `w317`'s budget made the
//! *per-trap* cost bounded and, in doing so, made *completion* depend on the guest continuing
//! to trap. Both halves are needed and only one was built.
//!
//! # Why this thread and not the publication lane
//!
//! `pubqueue`'s item type is `MapPublication` and `Revocation` has no route into it — by
//! construction, and pinned by a compile-fail row. That refusal is correct and this module
//! does not weaken it: revocation gets **its own tick on an off-trap thread**, never a slot
//! on the map lane.
//!
//! The thread is the one that already exists: `kayfabe-completion-observer`, 250 ms tick,
//! off-trap by construction. ⊘ It was deliberately handed a **read-only** closure so it could
//! not forge a completion, and that guarantee is untouched — this adds a *disposal* driver
//! beside the reader, not a writer into guest memory.
//!
//! # ⚠ THE HAZARD THIS EXISTS TO CLOSE — two drains of one queue
//!
//! `drain_retired_budgeted` plans under the rank-0 write guard and then **issues host verbs
//! with no lock held** (that is what makes it interruptible). Two threads running it
//! concurrently could therefore both plan the same retired object and both free it: a
//! **double disposal of a host RM object**, which is worse than the leak it fixes.
//!
//! ⇒ [`ReclaimTick::gate`], and the asymmetry is the whole design:
//!
//! | side | how it takes the gate | why |
//! |---|---|---|
//! | the **worker** | `lock()` — blocking | it is our own thread, off-trap; blocking here costs nothing and guarantees progress |
//! | the **vCPU** (`Regs::write`) | `try_lock()` — **never blocks** | it is under the BQL. A blocking acquire there is `INLINE-SAFE` clause (a) violated by construction: every vCPU and QEMU's main loop stop until a *different thread* finishes host I/O |
//!
//! ⊘ A vCPU that misses the gate **skips its drain for that trap**, which is safe: the worker
//! holding the gate is, by definition, draining the same queue right now. The residue is not
//! dropped — it is being spent by the other side.
//!
//! ⚠ **Lock classification (w300's census).** This is an `Arc<Mutex<()>>` reached from the
//! vCPU path, i.e. exactly the spelling that census was blind to until w300. The
//! discrimination is stated here so the row is checkable rather than remembered: **the vCPU
//! side may never block and cannot — it only ever calls `try_lock`; the worker side blocks
//! and is called only from the observer thread.** If a future caller reaches [`Self::spend`]
//! from a trap, that sentence is the thing that was wrong.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// How the tick is armed. ⊘ Off by default: this is a behaviour change on the teardown path
/// and the disarmed arm must be byte-comparable to master.
pub const RECLAIM_TICK_ENV: &str = "KAYFABE_RECLAIM_TICK";

/// The shared gate and its numbers.
#[derive(Debug)]
pub struct ReclaimTick {
    /// ★★★ The mutual exclusion between the vCPU's budgeted drain and the worker's. See the
    /// module docs' hazard table — the two sides take it **differently on purpose**.
    gate: Mutex<()>,
    armed: AtomicBool,
    /// Ticks on which the worker actually took the gate and ran.
    ticks: AtomicU64,
    /// Ticks on which the worker found nothing to do.
    idle_ticks: AtomicU64,
    /// Objects the worker disposed. ⊘ Counted separately from the vCPU's, because *"the
    /// guest's own traps drained it"* and *"our thread drained it"* are the two states this
    /// module exists to tell apart, and one total could not.
    worker_disposed: AtomicU64,
    /// Reaps the worker completed.
    worker_reaped: AtomicU64,
    /// ⚠ vCPU traps that found the gate held and skipped. Non-zero is the mechanism
    /// working, not a fault — but a *large* number beside `ticks=0` would mean the gate is
    /// held by something that is not ticking.
    vcpu_skipped: AtomicU64,
    /// The worst wall time one worker tick spent draining, µs. ⊘ Off the BQL, so this is a
    /// throughput number and **not** a stall — stated so it is never read beside
    /// `max_drain_us`, which is one.
    worst_tick_us: AtomicU64,
}

impl Default for ReclaimTick {
    fn default() -> Self {
        Self::new()
    }
}

impl ReclaimTick {
    /// A disarmed tick.
    #[must_use]
    pub fn new() -> Self {
        Self {
            gate: Mutex::new(()),
            armed: AtomicBool::new(false),
            ticks: AtomicU64::new(0),
            idle_ticks: AtomicU64::new(0),
            worker_disposed: AtomicU64::new(0),
            worker_reaped: AtomicU64::new(0),
            vcpu_skipped: AtomicU64::new(0),
            worst_tick_us: AtomicU64::new(0),
        }
    }

    /// Read the arming out of the environment. ⊘ Read **once**, at the composition root: an
    /// arming flag consulted twice is a boot that can change its mind halfway through.
    #[must_use]
    pub fn from_env() -> Self {
        let t = Self::new();
        if std::env::var(RECLAIM_TICK_ENV).is_ok_and(|v| v == "on" || v == "1") {
            t.armed.store(true, Ordering::Release);
        }
        t
    }

    /// Is the worker tick armed?
    #[must_use]
    pub fn armed(&self) -> bool {
        self.armed.load(Ordering::Acquire)
    }

    /// ★★★ **The vCPU side.** `Some(guard)` ⇒ drain now; `None` ⇒ the worker holds it, skip.
    ///
    /// ⊘ `try_lock` and never `lock`, and the return type says so: there is no method on
    /// this struct a trap could call that blocks. That is `INLINE-SAFE` clause (a) enforced
    /// by the API rather than by a comment.
    ///
    /// ⊘ When **disarmed** this always succeeds, because no worker ever takes the gate —
    /// so the vCPU path is byte-identical to master.
    #[must_use]
    pub fn try_claim_on_trap(&self) -> Option<std::sync::MutexGuard<'_, ()>> {
        match self.gate.try_lock() {
            Ok(g) => Some(g),
            Err(_) => {
                self.vcpu_skipped.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// ★★★ **The worker side.** Blocking, and only ever called from the observer thread.
    ///
    /// `run` is handed the gate guard's lifetime; it returns `(disposed, reaped)`.
    pub fn spend<F>(&self, run: F)
    where
        F: FnOnce() -> (u64, u64),
    {
        if !self.armed() {
            return;
        }
        let started = std::time::Instant::now();
        let _g = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let (disposed, reaped) = run();
        let us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.worst_tick_us.fetch_max(us, Ordering::Relaxed);
        if disposed == 0 && reaped == 0 {
            self.idle_ticks.fetch_add(1, Ordering::Relaxed);
        } else {
            self.ticks.fetch_add(1, Ordering::Relaxed);
            self.worker_disposed.fetch_add(disposed, Ordering::Relaxed);
            self.worker_reaped.fetch_add(reaped, Ordering::Relaxed);
        }
    }

    /// One line for a boot log.
    ///
    /// ⊘ Printed even when disarmed and even when every number is zero — *"the tick never
    /// ran"* and *"the tick ran and found nothing"* are the two facts this whole module is
    /// about, and only the line distinguishes them.
    #[must_use]
    pub fn census(&self) -> String {
        let ticks = self.ticks.load(Ordering::Relaxed);
        let idle = self.idle_ticks.load(Ordering::Relaxed);
        format!(
            "RECLAIM-TICK armed={} working_ticks={} idle_ticks={} worker_disposed={} \
             worker_reaped={} vcpu_skipped={} worst_tick_us={}{}",
            self.armed(),
            ticks,
            idle,
            self.worker_disposed.load(Ordering::Relaxed),
            self.worker_reaped.load(Ordering::Relaxed),
            self.vcpu_skipped.load(Ordering::Relaxed),
            self.worst_tick_us.load(Ordering::Relaxed),
            if self.armed() && ticks == 0 && idle == 0 {
                " ⚠⚠ ARMED AND NEVER RAN — the thread did not start, or the feature is off. \
                 ⊘ Read this as UNMEASURED, never as 'there was nothing to drain'."
            } else {
                ""
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// ⊘⊘ **THE DISARMED ARM IS THE CONTROL** — the vCPU never misses the gate, so
    /// `Regs::write` behaves exactly as master's.
    #[test]
    fn disarmed_the_vcpu_never_misses_the_gate() {
        let t = ReclaimTick::new();
        for _ in 0..16 {
            assert!(t.try_claim_on_trap().is_some());
        }
        assert!(t.census().contains("vcpu_skipped=0"));
        assert!(t.census().contains("armed=false"));
    }

    /// ⊘ A disarmed `spend` does nothing at all, so it cannot take the gate and cannot make
    /// a vCPU skip. The control's byte-comparability rests on this.
    #[test]
    fn a_disarmed_spend_never_runs_and_never_takes_the_gate() {
        let t = ReclaimTick::new();
        t.spend(|| panic!("⊘ a disarmed tick must not reach its body"));
        assert!(t.try_claim_on_trap().is_some());
    }

    /// ★★★★★ **THE HAZARD, WATCHED**: while the worker holds the gate, a trap must be
    /// REFUSED rather than blocked — a double drain would double-free a host RM object.
    #[test]
    fn a_trap_is_refused_and_not_blocked_while_the_worker_drains() {
        let t = Arc::new(ReclaimTick::new());
        t.armed.store(true, Ordering::Release);
        let inside = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let (t2, i2, r2) = (Arc::clone(&t), Arc::clone(&inside), Arc::clone(&release));
        let h = std::thread::spawn(move || {
            t2.spend(|| {
                i2.wait();
                r2.wait();
                (3, 1)
            });
        });
        inside.wait();
        // ★ The claim: this returns, and it returns `None`. A `lock()` here would deadlock
        // the test — which is exactly what it would do to every vCPU under the BQL.
        assert!(
            t.try_claim_on_trap().is_none(),
            "★★★ the trap MUST be refused, never queued behind host I/O on another thread"
        );
        release.wait();
        h.join().unwrap();
        assert!(t.try_claim_on_trap().is_some(), "and it is available again after");
        let c = t.census();
        assert!(c.contains("vcpu_skipped=1"), "{c}");
        assert!(c.contains("working_ticks=1"), "{c}");
        assert!(c.contains("worker_disposed=3"), "{c}");
        assert!(c.contains("worker_reaped=1"), "{c}");
    }

    /// ★★ An idle tick is counted apart from a working one: *"our thread ran and there was
    /// nothing"* is the evidence that the drain no longer depends on the guest, and folding
    /// it into `working_ticks` would destroy exactly that.
    #[test]
    fn an_idle_tick_is_not_a_working_tick() {
        let t = ReclaimTick::new();
        t.armed.store(true, Ordering::Release);
        t.spend(|| (0, 0));
        let c = t.census();
        assert!(c.contains("working_ticks=0"), "{c}");
        assert!(c.contains("idle_ticks=1"), "{c}");
    }

    /// ⚠ Armed-and-never-ran must NOT read as "nothing to drain".
    #[test]
    fn armed_and_never_run_names_itself_unmeasured() {
        let t = ReclaimTick::new();
        t.armed.store(true, Ordering::Release);
        assert!(t.census().contains("ARMED AND NEVER RAN"));
    }
}
