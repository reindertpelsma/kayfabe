//! ★★★★★ **Work moved to the doorbell worker must still happen when there is no worker (w532).**
//!
//! # The regression this file is paid for
//!
//! `start_doorbell_publish_worker` returns immediately when `!defers()`, so on the
//! `KAYFABE_DOORBELL_ASYNC=off` arm **there is no worker at all**. Anything moved into
//! `doorbell_publish_loop` without a non-worker caller is therefore **dead** on that arm —
//! silently, because the trap still returns and the guest still runs.
//!
//! `[measured w529]` that arm can no longer initialise the guest driver: *"NOTHING from
//! RmInitAdapter, and nvidia-smi did not succeed either"*, with
//! `BAR2-PASSTHROUGH MISS #5, #6, #7 … no memslot covered this page yet` at successive pages —
//! what "fills never install" looks like from outside.
//!
//! ⚠ It matters beyond one stale arm: **`off` is the arm the LLM last passed on** (w383), so
//! it is the shortest route back to a known-good baseline.
//!
//! # What is asserted, and what is not
//!
//! ⊘ This does NOT assert the driver boots — that needs a GPU and a guest. It asserts the
//! structural fact the boot failure follows from: on the no-worker arm, a trap drains the
//! mirror, so the queue cannot grow without bound. A test that needed hardware would not have
//! caught this and did not.

use kayfabe_qemu_raw::shim::{DOORBELL_ASYNC_ENV, Regs};

const BAR_REGS: u32 = 0;
/// A register this port does not decode — enough to reach the trap's tail, which is where the
/// drain lives. The value is irrelevant; the path is the subject.
const NOBODYS_OFFSET: u64 = 0x0000_9400;

fn regs_with(arm: &str) -> Regs {
    // SAFETY: single-threaded test setup, before this process builds any `Regs`.
    unsafe {
        std::env::set_var(DOORBELL_ASYNC_ENV, arm);
    }
    Regs::create(0).expect("the shipped chip row realizes")
}

/// ★★★ **On the no-worker arm, a trap drains the mirror; on the worker arm it does not.**
///
/// ⊘ Both halves in one test: the `off` assertion alone would pass for an implementation that
/// drained unconditionally, which would put the worker's job back on every shipping-arm trap —
/// the exact cost the w510–w525 campaign removed.
#[test]
fn the_no_worker_arm_drains_on_the_trap_and_the_worker_arm_does_not() {
    // ---- no worker: the trap is the only thing that can drain, so it must.
    let off = regs_with("off");
    let before = off.plane_mirror_drains_for_test();
    let _ = off.write(BAR_REGS, NOBODYS_OFFSET, 4, 0);
    let after = off.plane_mirror_drains_for_test();
    assert!(
        after > before,
        "with no worker the trap MUST drain the mirror — otherwise every queued fill is \
         dropped, no memslot is installed, and the BAR passthrough plane never engages \
         ({before} -> {after})"
    );

    // ---- worker arm: the trap must NOT, or the campaign's trap budget comes back.
    let on = regs_with("on");
    let before = on.plane_mirror_drains_for_test();
    let _ = on.write(BAR_REGS, NOBODYS_OFFSET, 4, 0);
    assert_eq!(
        on.plane_mirror_drains_for_test(),
        before,
        "on the shipping arm the worker owns the drain; doing it on the trap as well is the \
         per-trap cost w519-w525 removed"
    );

    // Leave the process on the shipping default for anything that runs after.
    // SAFETY: as above.
    unsafe {
        std::env::set_var(DOORBELL_ASYNC_ENV, "on");
    }
}
