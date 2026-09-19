//! ★★★★★ **THE PAGE-TABLE SWEEP'S SKIP PREDICATE — all eight inputs, no GPU.**
//!
//! `SharedDoorbell::refresh_page_tables` decides, on every invalidate, whether to re-read the
//! guest's page tables. A wrong skip is not a slow boot — it is a stale GPU translation, and
//! then an `Xid 31 … FAULT_PDE` on an address the guest believes it mapped.
//!
//! # ⊘⊘⊘ THIS PREDICATE WAS WRONG FOR AN ENTIRE ARCHITECTURE AND NOTHING WENT RED
//!
//! `[measured w784, thin guest]` `PT-SWEEP ⊘ SKIPPED` **815 of 831 windows**. The sweep that
//! discovers the guest's mappings never ran, the user's address space held its four promoted
//! context rows and not one UVM mapping, and `GR0_PBDMA0` faulted on the first address
//! hardware was ever asked to translate. Finding it cost a rented GPU and most of a day.
//!
//! The guard was added by w763z as an honest PERFORMANCE measurement — 7.22 ms of a ~23 ms
//! invalidate hold, *"271 of 369 sweeps walk every proc's page tables and find nothing"* —
//! on a premise that was true when written: *"the only way those bytes change is the guest's
//! CPU writing them, which is exactly what `RegPlane::pt_witness` records."* The single store
//! made the framebuffer a shared region, its stores stopped trapping, and the second half of
//! that sentence quietly stopped being true. ⇒ `a_rulings_date_is_part_of_the_citation`.
//!
//! ⚠ **None of that needed hardware to find.** It is a three-input boolean whose broken input
//! is *"the witness is permanently zero"* — a value a test passes in. The only reason no test
//! caught it is that there was no function to call: it was three `&&`s at a call site. So the
//! deliverable of this file is as much the extraction as the assertions.
//!
//! > **Owner, 2026-09-19:** *"It seems to me a lot of them can be found by adding tests to
//! > the project that don't need a gpu to run."*

use kayfabe_qemu_raw::shim::sweep_should_skip;

/// ★★★ **THE REGRESSION, NAMED.** The single store's world: the witness is permanently zero,
/// and the only signal left is the guest's own declaration.
#[test]
fn a_declared_invalidate_runs_the_sweep_even_with_a_dead_witness() {
    assert!(
        !sweep_should_skip(true, 0, true),
        "★★★★★ w786: with `pt_drained == 0` FOREVER — which is what the single store does to \
         the CPU witness — a declared invalidate is the ONLY thing left that can run the \
         sweep. Skipping here is `PT-SWEEP ⊘ SKIPPED x815` and an `Xid 31 FAULT_PDE`"
    );
}

/// ⊘ **The negative control.** The guard must still do its job, or w763z's 7.22 ms comes
/// straight back and the heavy arms time out.
#[test]
fn nothing_witnessed_and_nothing_declared_still_skips() {
    assert!(
        sweep_should_skip(true, 0, false),
        "⊘ no evidence of any change through EITHER channel. Running here re-walks unchanged \
         bytes on every invalidate — the cost w763z measured at 7.22 ms of a 23 ms hold"
    );
}

/// ★★ The two evidence channels are **independent**: either one alone runs the sweep.
#[test]
fn either_channel_alone_runs_the_sweep() {
    assert!(
        !sweep_should_skip(true, 1, false),
        "★ the CPU transport witnessed a page-table write — the w763z-era signal, still valid \
         wherever the framebuffer does trap"
    );
    assert!(
        !sweep_should_skip(true, 0, true),
        "★ the guest declared an invalidate — the signal that survives the single store"
    );
}

/// ⊘ **Disarming the guard runs the sweep unconditionally.** The arm exists to be turned off
/// (w763z: *"the arm exists to be turned off and the census exists to prove the skip only
/// fires when the witness is empty"*), and a boot with it off must never skip.
#[test]
fn a_disarmed_guard_never_skips_whatever_the_evidence() {
    for drained in [0usize, 1] {
        for unseen in [false, true] {
            assert!(
                !sweep_should_skip(false, drained, unseen),
                "⊘ disarmed means ALWAYS sweep (drained={drained} unseen={unseen}); a guard \
                 that still skipped when turned off could not be bisected against"
            );
        }
    }
}

/// ★★★★★ **ALL EIGHT INPUTS, as a table.**
///
/// ⊘ Quantified rather than spot-checked, for `gates_quantified_over_a_list.md`'s reason: the
/// four tests above each pin one row, and a fifth input added later would slip past all of
/// them. This asserts the whole truth table, so a new term cannot be introduced without a
/// decision recorded here.
#[test]
fn the_whole_truth_table() {
    // (armed, pt_drained, invalidate_unseen) -> skip
    let table = [
        ((false, 0usize, false), false),
        ((false, 0, true), false),
        ((false, 1, false), false),
        ((false, 1, true), false),
        ((true, 0, false), true), // the ONLY skipping row
        ((true, 0, true), false),
        ((true, 1, false), false),
        ((true, 1, true), false),
    ];
    let mut skipping = 0;
    for ((armed, drained, unseen), want) in table {
        assert_eq!(
            sweep_should_skip(armed, drained, unseen),
            want,
            "armed={armed} pt_drained={drained} invalidate_unseen={unseen}"
        );
        skipping += usize::from(want);
    }
    assert_eq!(
        skipping, 1,
        "★ EXACTLY ONE of eight inputs may skip — armed, nothing witnessed, nothing declared. \
         A second skipping row means some evidence of change is being discarded, which is the \
         w784 failure arriving from a new direction"
    );
}
