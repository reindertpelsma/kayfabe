//! ★★★★★ **The one question a vCPU may ask about the pending latches (w519).**
//!
//! `[measured w519]` the in-trap lock counter ranked its sites, and four were called almost
//! exactly **once per MMIO trap** across 89 310 traps — `peek_pending_engine_forwards`
//! 178 158 times, `peek_pending_channel_births` 89 079 — each taking the Device read lock to
//! ask *"is anything latched?"*. `[measured w517]` that rank is held for 5 ms at a stretch by
//! the page-table sweep's commit, and a vCPU waits behind it.
//!
//! One boot latches about **12** channel births and **9** engine objects, so the answer is
//! essentially always no, and it now costs one relaxed load.
//!
//! ⚠ The property under test is the **asymmetry**, not the count: the epoch may over-report
//! and may never under-report. A wasted peek costs one lock acquisition; a missed one leaves
//! a latched channel birth that nothing ever runs — and nothing else polls for it.

use kayfabe_arch::ids::{HClient, HObject};
use kayfabe_qemu_raw::shim::{DOORBELL_ASYNC_ENV, Regs};
use kayfabe_rt::device::pending_latch_epoch;

const CLIENT: HClient = HClient(0x5c00_0000);
const CHANNEL: HObject = HObject(0x5c00_0019);

fn device() -> std::sync::Arc<kayfabe_rt::device::SharedDevice> {
    // SAFETY: single-threaded test setup, before this process builds any `Regs`.
    unsafe {
        std::env::set_var(DOORBELL_ASYNC_ENV, "off");
    }
    Regs::create(0).expect("the shipped chip row realizes").object_model()
}

/// ★★★ **A latch must always move the epoch.** This is the direction that cannot be wrong:
/// if a push fails to move it, a vCPU skips the drain and the birth is never run.
#[test]
fn latching_a_channel_birth_moves_the_epoch_and_a_drain_does_not_lower_it() {
    let dev = device();

    let before = pending_latch_epoch();
    let _ = dev.latch_channel_birth(CLIENT, CHANNEL);
    let after_latch = pending_latch_epoch();
    assert!(
        after_latch > before,
        "a latched birth that does not move the epoch is a birth a vCPU will never notice: \
         {before} -> {after_latch}"
    );

    // ⊘ The negative half, in the SAME test: the epoch is process-global and `cargo test`
    // runs test functions in parallel, so a separate "my actions did not move it" test would
    // race this one's increment. That is not a flake to retry — a shared counter read from
    // two threads is not a per-test measurement.
    let _ = dev.run_pending_channel_births(&[]);
    assert!(
        pending_latch_epoch() >= after_latch,
        "the epoch is monotone: a drain must never lower it. Lowering it would let a push \
         that lands in the same window read as 'unchanged'"
    );
}
