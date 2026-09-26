//! ★★★★★ **w236 — the plane's FSM lock, made VISIBLE to the rank witness.**
//!
//! # The gap this closes, and it is the tree's deepest concurrency gate being blind
//!
//! `l1_concurrency.md` §2 declares a total lock order and `kayfabe_rt::lock::check_acquire`
//! **enforces** it: acquiring out of order panics deterministically, *before* the OS-level
//! acquire, so an inversion is diagnosable instead of occasionally deadlocking. That
//! mechanism has worked since L1.
//!
//! ⊘ **It only ever saw the locks that declared a rank.** `kayfabe_device::RegPlane::state`
//! WAS a bare `std::sync::Mutex<PlaneState>` held across the whole
//! command-policy chain on the vCPU's own MMIO trap — **the highest-traffic lock in the
//! device** — and it declared none. So:
//!
//! - `check_acquire` could not see an inversion involving it, and
//! - `assert_lock_free` passed **vacuously** while it was held, which is the
//!   `unranked_locks.rs` module doc's own admission: *"It does not detect the violation …
//!   that remains a review obligation."*
//!
//! ★★ **And the inversion is not hypothetical.** `plane.rs`' `ce_session` states the shipping
//! order — *"the command-policy chain already takes the core's ranked locks under this mutex,
//! so plane→core is the established order and core→plane is its inversion"* — while
//! `kayfabe_rt::device::forward_ring` runs under the **device lock and the proc mutex** (ranks
//! 1 and 2 since this rung renumbered them). §16.86 found that wiring route B's framebuffer read there would take the plane
//! mutex beneath both: **a guest that rings a doorbell on one vCPU while touching a register
//! on another builds the ABBA itself.**
//!
//! # ⊘ Why a locally-constructed `RankedRwLock` is the real thing, not a stand-in
//!
//! The tests below hold `RankedRwLock::new(LockRank::Device, …)` rather than reaching into a
//! `SharedDevice`. That is the **same type at the same rank**, and the witness state it
//! produces is byte-identical to `route_act`'s — the held mask is per-thread and records the
//! *rank*, never which object. ⊘ Using a real `SharedDevice` would add a device realization,
//! an isolate factory and a scenario to a test whose subject is two bits in a thread-local,
//! and every one of those is a way for the test to fail for a reason that is not its subject.

use std::panic::{AssertUnwindSafe, catch_unwind};

use kayfabe_device::plane::RegPlane;
use kayfabe_device::{NanoClock, SteppingClock};
use kayfabe_mmu::walker::FbRead;
use kayfabe_rt::lock::{LockRank, RankedRwLock};

/// A servable GA106 register plane — the shipping constructor, so the lock under test is the
/// one the device actually runs on.
fn plane() -> RegPlane {
    RegPlane::new(
        &kayfabe_device::ga10x::GA106,
        kayfabe_device::abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable")
}

/// ★★★★★ **THE FALSIFIER — `core → plane` must PANIC, and it is the exact shape route B
/// would have shipped.**
///
/// The rank-1 device lock is held (as `route_act` holds it for the whole doorbell act), and the
/// framebuffer is then read through the only seam that serves bytes,
/// [`RegPlane::pt_bytes`] — which takes the plane's FSM mutex per read.
///
/// ⊘ **Before this rung this test FAILED**, and that is the whole finding: the read returned
/// cleanly, `check_acquire` saw nothing because the plane lock had no rank, and
/// `assert_lock_free` would have passed too. A gate that cannot see the lock in question
/// returns the same answer for a sound design and an unsound one.
#[test]
fn taking_the_plane_lock_under_a_core_lock_panics() {
    let plane = plane();
    let core = RankedRwLock::new(LockRank::Device, 0u32);
    let _held = core.read();

    let got = catch_unwind(AssertUnwindSafe(|| {
        let mut fb = plane.pt_bytes();
        let mut buf = [0u8; 8];
        // Exactly what wiring route B into `forward_ring` would do.
        let _ = fb.read(0x0102_4000, &mut buf);
    }));

    let err = got.expect_err(
        "★★★★★ THE INVERSION WENT UNDETECTED. A rank-1 core lock was held and the plane's \
         FSM mutex was then acquired beneath it — the exact `core → plane` order \
         `plane.rs`' `ce_session` names as the inversion of the shipping order, whose ABBA \
         partner already ships on another vCPU's MMIO trap. If this line is reached, the \
         rank witness is blind to the highest-traffic lock in the device.",
    );
    let msg = err
        .downcast_ref::<String>()
        .map(String::as_str)
        .unwrap_or_default();
    assert!(
        msg.contains("R3 lock-rank violation"),
        "★★ the panic must be the RANK MECHANISM's, by name — a panic from anywhere else \
         (a poisoned mutex, an unservable chip, a bad address) would also make this \
         `is_err()` true while proving nothing about lock order. Got: {msg}"
    );
}

/// ★★★★ **THE KNOWN-POSITIVE — the SHIPPING order must still be legal.**
///
/// `plane → core` is what the device does on every register access: the policy chain holds
/// the plane mutex and takes the core's ranked locks beneath it. Ranking the plane **below**
/// the device rank is what keeps that legal while making its inverse illegal.
///
/// ⊘ Without this arm, a "fix" that simply forbade the plane lock everywhere — or that
/// ranked it *above* the device — would pass the falsifier above and break every MMIO trap
/// in the device. §16.85.3's rule, applied here: name the known-positive and run it through
/// the same predicate.
#[test]
fn the_shipping_plane_then_core_order_is_still_legal() {
    let plane = plane();
    let core = RankedRwLock::new(LockRank::Device, 0u32);

    let got = catch_unwind(AssertUnwindSafe(|| {
        // ⊘⊘ The plane's guard is HELD across the core acquisition, and it has to be.
        // An earlier draft used `pt_bytes().read()` here and was **mis-specified**: that
        // door takes and releases the mutex per read (its own doc says so), so the plane
        // lock was already down by the time the core lock was taken and the test asserted
        // a sequence, not an ORDER. `clippy::drop_non_drop` caught it — `PlanePtBytes`
        // implements no `Drop`, because it holds nothing to drop.
        // ★ A test of a lock ORDER must hold the first lock while taking the second, or it
        // passes for a reason unrelated to its subject.
        let _plane_held = plane.state_guard_for_test();
        // … and the core's rank-1 lock beneath it, as the policy chain does.
        let _held = core.read();
    }));

    assert!(
        got.is_ok(),
        "★★★★ the ESTABLISHED order must remain legal. `plane → core` is what every vCPU \
         MMIO trap does; a rank assignment that makes it panic has not made the invariant \
         visible, it has broken the device."
    );
}

/// ★★★ **`assert_lock_free` now SEES the plane lock** — the second half of the win, and the
/// one the `unranked_locks.rs` module doc explicitly gave up on.
///
/// That doc says of the unranked set: *"⊘ **It does not detect the violation.** Nothing here
/// fires when a blocking call runs under an unranked lock; that remains a review
/// obligation."* `[measured 2026-08-06]` a control-path host call was being designed against
/// `assert_lock_free` while the caller held this very mutex — it *"would have compiled,
/// passed every assertion, and stalled every vCPU's MMIO for the duration of a host round
/// trip."*
///
/// ⇒ Ranking the lock converts that standing review obligation into an assertion.
#[test]
fn a_blocking_door_beneath_the_plane_lock_is_now_refused() {
    let plane = plane();

    let got = catch_unwind(AssertUnwindSafe(|| {
        let mut fb = plane.pt_bytes();
        let mut buf = [0u8; 8];
        let _ = fb.read(0x0102_4000, &mut buf);
        // ⊘ The read RELEASES the mutex per call by design (`pt_bytes`' own doc), so the
        // guard must be held explicitly to pose the question this test asks.
        let _guard = plane.state_guard_for_test();
        kayfabe_util::lockwitness::assert_lock_free("a host RM round trip");
    }));

    let err = got.expect_err(
        "★★★ a blocking door beneath the plane's FSM mutex must now be REFUSED. This is the \
         2026-08-06 hazard: a host round trip under this lock stalls every vCPU's register \
         access, and until this rung `assert_lock_free` returned cleanly while it was held.",
    );
    let msg = err
        .downcast_ref::<String>()
        .map(String::as_str)
        .unwrap_or_default();
    assert!(
        msg.contains("R1 no-blocking-under-lock violation"),
        "★ the refusal must be R1's, by name. Got: {msg}"
    );
}
