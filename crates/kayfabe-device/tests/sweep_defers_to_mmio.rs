//! ★★★★★ **The page-table sweep must not starve a vCPU on the plane lock (w507).**
//!
//! # The defect this file exists for
//!
//! `[measured w506]` the rank-0 census read **`worst_wait=3825us` against `worst_hold=0us`**.
//! Read that pair carefully, because the two halves point at opposite fixes: **no holder was
//! long**, so nothing was doing slow work under the lock — a waiter was simply **starved**.
//! `std::sync::Mutex` is *barging*, not FIFO, so a loop that unlocks and immediately re-locks
//! keeps winning the handoff against a thread parked in `futex_wait`, however short each of
//! its holds is. `[measured w504]` the stall alarm caught a vCPU parked in exactly that frame:
//! `futex_wait -> Mutex::lock_contended -> RankedMutex<PlaneState>::lock -> RegPlane::write`.
//!
//! The barging loop is the sweep: `PlanePtBytes` takes the plane lock **once per table page**,
//! over a whole VAS, on a worker thread. The fix is for the sweep to defer to a vCPU that is
//! inside an MMIO trap — ⊘ **not** to hold the lock longer across the sweep, which would
//! convert starvation into a *guaranteed* multi-millisecond stall for every waiter.
//!
//! # Why a periodic `yield_now()` was not the fix, and is not what is tested here
//!
//! The bench box is 24-core and ~90% idle. `sched_yield` with nothing else runnable on the
//! core returns immediately, so a blind periodic yield is very nearly a no-op — it would have
//! measured as "the fix did nothing" and cost a boot to learn. The sweep therefore waits on a
//! **named condition** (`mmio_in_flight`) rather than on the scheduler's goodwill.
//!
//! ⚠ These tests check the **mechanism**, not the latency. Whether `rank0 worst_wait` actually
//! falls is a boot measurement and cannot be asserted here.

use kayfabe_device::plane::RegPlane;
use kayfabe_device::{NanoClock, SteppingClock, abi};
use kayfabe_mmu::walker::FbRead;

fn plane() -> RegPlane {
    RegPlane::new(
        &kayfabe_device::ga10x::GA106,
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable")
}

/// ★★★ **The counter must come back to zero out of EVERY trap, including the early returns.**
///
/// `RegPlane::write` returns from a dozen places — the window latch, the framebuffer windows,
/// the GSP model, the unclaimed arm. The count is raised by a `Drop` guard precisely so that
/// none of those can leak it, and this test is the check on that claim: a stuck count would
/// make the sweep yield forever and turn the anti-starvation fix into the stall it exists to
/// prevent. ⊘ That failure would be **silent** — the guest keeps running, the sweep just stops
/// making progress — which is why it gets a test rather than a code comment.
#[test]
fn every_trap_path_returns_the_in_flight_count_to_zero() {
    let p = plane();
    assert_eq!(p.mmio_in_flight(), 0, "a fresh plane has no trap in flight");

    // The BAR0 window latch, PRAMIN, a plainly unclaimed register, and a BAR1 offset: four
    // different arms of the classifier, chosen because they return from different places.
    for (bar, off) in [(0u8, 0x1700u64), (0, 0x0070_0000), (0, 0x0000_9400), (1, 0x1000)] {
        p.write(bar, off, 4, 0xABCD_ABCD);
        assert_eq!(
            p.mmio_in_flight(),
            0,
            "write(bar{bar}, {off:#x}) left the in-flight count raised"
        );
        let _ = p.read(bar, off, 4);
        assert_eq!(
            p.mmio_in_flight(),
            0,
            "read(bar{bar}, {off:#x}) left the in-flight count raised"
        );
    }
}

/// ★★★ **The sweep gives up rather than livelocking, and it SAYS SO.**
///
/// A guest that traps continuously would otherwise hold the sweep off forever — and a sweep
/// that never commits is its own stall, because the page-table mirror is what the next
/// doorbell resolves against. So the deferral is bounded.
///
/// ★ The assertion is on the **census**, not on wall time. `no counter fired` is not `no
/// record exists`: a bound that is never reached and a bound that does not exist look
/// identical from outside, so the mechanism has to report that it ran.
#[test]
fn a_permanently_trapping_guest_does_not_livelock_the_sweep() {
    let p = plane();
    let before = kayfabe_device::plane::sweep_defer_census();

    let held = p.hold_mmio_in_flight_for_test();
    assert_eq!(p.mmio_in_flight(), 1, "the test hold is the mechanism's input");

    // One page read, with the flag held for its whole duration. It must return — bounded —
    // rather than spin until the test times out.
    let mut r = p.pt_bytes();
    let mut buf = [0u8; 8];
    let _ = r.read(0, &mut buf);
    drop(held);

    let after = kayfabe_device::plane::sweep_defer_census();
    assert!(
        after.0 > before.0,
        "the sweep never deferred, so this test proves nothing about the bound \
         (deferrals {} -> {})",
        before.0,
        after.0
    );
    assert!(
        after.1 > before.1,
        "the sweep deferred but never hit its bound — with the flag held for the whole read \
         it must (giveups {} -> {})",
        before.1,
        after.1
    );
}

/// ★ The negative control for the test above: with no trap in flight, the sweep does not
/// defer at all. ⊘ Without this, a `breathe` that yielded unconditionally would pass the
/// bounded-livelock test and still be the no-op periodic yield this design rejected.
#[test]
fn with_no_trap_in_flight_the_sweep_does_not_defer() {
    let p = plane();
    let before = kayfabe_device::plane::sweep_defer_census();
    let mut r = p.pt_bytes();
    let mut buf = [0u8; 8];
    for at in 0..64u64 {
        let _ = r.read(at * 8, &mut buf);
    }
    let after = kayfabe_device::plane::sweep_defer_census();
    assert_eq!(
        after, before,
        "64 page reads with an idle guest must cost zero deferrals"
    );
}
