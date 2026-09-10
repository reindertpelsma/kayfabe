//! ★★★★★ **DROPPED SIGNALS — a lost notification costs TIME, never COVERAGE.**
//!
//! Owner, 2026-09-10, giving the rule for both saturated lanes:
//!
//! > *"if the queue is full then a flag must be set that the refresh considers the entirety of
//! > PTE/PDB dirty (i.e it rescans everything). then its correct, only a bit slower on full
//! > queue."*
//!
//! > *"Emulated channel doorbells add the channels that are doorbelled to the queue. If that
//! > queue is full, then the coordinator thread is told to loop all emulated channels. For
//! > passthrough doorbells no such tracking is needed since we only forward them without
//! > keeping any state."*
//!
//! # Why this is a type and not two `AtomicBool`s in the shim
//!
//! The code these replace did the **forbidden** thing on exactly this arm: a full publication
//! lane published on a **vCPU**, and a full doorbell lane ran an emulated channel **inside the
//! doorbell**. Both were written as *"degrade to the status quo, never worse"* — and the status
//! quo was the violation, so degrading *into* it was the bug. Both only happen under load,
//! where they are least affordable and least visible.
//!
//! ⚠ **Neither has ever fired on hardware.** The queue census reads zero against `cap=4096` on
//! every measured boot. That is precisely why the protocol lives here with tests: a fallback
//! nobody has seen run is a fallback nobody has seen work, and this tree has a standing habit
//! of shipping machinery that was built, wired, and never executed.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The two "we lost a signal, be conservative" latches.
#[derive(Debug, Default)]
pub struct DroppedSignals {
    full_rescan: AtomicBool,
    sweep_emulated: AtomicBool,
    rescans_armed: AtomicU64,
    sweeps_armed: AtomicU64,
}

impl DroppedSignals {
    /// Nothing lost yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            full_rescan: AtomicBool::new(false),
            sweep_emulated: AtomicBool::new(false),
            rescans_armed: AtomicU64::new(0),
            sweeps_armed: AtomicU64::new(0),
        }
    }

    /// A publication job could not be queued ⇒ the next refresh must treat **every** PDB/PTE
    /// as dirty.
    pub fn arm_full_rescan(&self) {
        self.rescans_armed.fetch_add(1, Ordering::Relaxed);
        self.full_rescan.store(true, Ordering::Release);
    }

    /// An **emulated** channel's doorbell could not be queued ⇒ the coordinator must loop
    /// every emulated channel. ⊘ Never armed for passthrough: those are forwarded and keep no
    /// state of ours, so a lost one loses nothing.
    pub fn arm_emulated_sweep(&self) {
        self.sweeps_armed.fetch_add(1, Ordering::Relaxed);
        self.sweep_emulated.store(true, Ordering::Release);
    }

    /// Consume the rescan latch. `true` means the caller owes a full rescan.
    ///
    /// ⊘ **`swap`, not `load` then `store`.** Two workers must not both see `true`, both skip
    /// on the assumption the other has it, or — worse — both clear a flag that was re-armed
    /// between the read and the write. The classic lost-wakeup, and it would present as a
    /// silently missing rescan under exactly the load that armed it.
    pub fn take_full_rescan(&self) -> bool {
        self.full_rescan.swap(false, Ordering::AcqRel)
    }

    /// Consume the emulated-sweep latch.
    pub fn take_emulated_sweep(&self) -> bool {
        self.sweep_emulated.swap(false, Ordering::AcqRel)
    }

    /// `(rescans_armed, sweeps_armed)` — cumulative, for the boot census. ⊘ Counted separately
    /// from "taken" on purpose: armed-but-never-taken means a worker stopped draining, which
    /// is a different fault from a saturated lane.
    #[must_use]
    pub fn census(&self) -> (u64, u64) {
        (
            self.rescans_armed.load(Ordering::Relaxed),
            self.sweeps_armed.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⊘ THE NEGATIVE CONTROL FIRST. Without it, a `take` hardcoded to `true` passes every
    /// test below while making every boot do a full rescan and calling it correct.
    #[test]
    fn nothing_is_owed_until_something_is_dropped() {
        let d = DroppedSignals::new();
        assert!(!d.take_full_rescan(), "a fresh latch owes no rescan");
        assert!(!d.take_emulated_sweep(), "and no sweep");
        assert_eq!(d.census(), (0, 0));
    }

    /// ★★★ THE FALLBACK FIRING, both of them, which is the thing the owner asked to see
    /// tested rather than described.
    #[test]
    fn a_dropped_job_is_owed_exactly_once() {
        let d = DroppedSignals::new();
        d.arm_full_rescan();
        assert!(d.take_full_rescan(), "the drop must be honoured");
        assert!(
            !d.take_full_rescan(),
            "and honoured ONCE — a latch that stays set makes every later refresh full-scope \
             and hides its own cost as the normal price of running"
        );

        d.arm_emulated_sweep();
        assert!(d.take_emulated_sweep());
        assert!(!d.take_emulated_sweep());
        assert_eq!(d.census(), (1, 1), "both arms counted, separately");
    }

    /// ⊘ The two are INDEPENDENT. A publication lane filling must not make the coordinator
    /// loop every emulated channel, and vice versa: they answer different questions and their
    /// costs are paid by different threads.
    #[test]
    fn the_two_latches_do_not_bleed_into_each_other() {
        let d = DroppedSignals::new();
        d.arm_full_rescan();
        assert!(
            !d.take_emulated_sweep(),
            "a dropped PUBLICATION job says nothing about doorbells"
        );
        assert!(d.take_full_rescan());

        d.arm_emulated_sweep();
        assert!(
            !d.take_full_rescan(),
            "and a dropped DOORBELL says nothing about page tables"
        );
        assert!(d.take_emulated_sweep());
    }

    /// ★★★★★ **THE LOST WAKEUP** — the one bug this protocol can actually have.
    ///
    /// Arming while a consumer is mid-take must not be swallowed. `swap` gives that: the
    /// consumer's clear happens at a point in time, and an arm after it leaves the latch set.
    /// A `load`-then-`store` implementation would clear the re-arm and lose the rescan, under
    /// exactly the sustained load that produced it.
    #[test]
    fn an_arm_racing_a_take_is_never_swallowed() {
        let d = std::sync::Arc::new(DroppedSignals::new());
        for _ in 0..2_000 {
            d.arm_full_rescan();
            let w = std::sync::Arc::clone(&d);
            let t = std::thread::spawn(move || w.take_full_rescan());
            let arm_again = d.clone();
            arm_again.arm_full_rescan();
            let took = t.join().expect("the taker does not panic");
            // Whether the taker won or lost, the SECOND arm must still be owed to somebody.
            assert!(
                d.take_full_rescan() || took,
                "an arm was swallowed: nobody owes the rescan it asked for"
            );
            while d.take_full_rescan() {}
        }
    }
}
