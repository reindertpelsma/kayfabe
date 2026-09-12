//! ★★★★★ **The lock-free "not mine" filter must never hide a register that IS ours (w543).**
//!
//! # What this is guarding
//!
//! `[measured w539]` **121 793 of 241 874 BAR0 reads in a boot are unclaimed** — half the read
//! surface. Each one used to take the register plane's **rank-0 mutex**, ask the boot FSM, and
//! be told nothing claimed it. That is the same lock a vCPU's `RegPlane::write` waits on.
//!
//! w543 answers "could this possibly be ours?" without the lock, from two state-free sources:
//! `GspModel::decode_reg` and `BootSequence::may_read`.
//!
//! ⊘ **The failure mode is silent and asymmetric.** A filter that says *yes* too often merely
//! takes a lock it did not need — a lost optimisation. A filter that says *no* where the
//! locked path would have answered turns a **served register into an unclaimed one**, and the
//! guest reads a defaulted zero instead of a value. Nothing logs it, no counter moves, and the
//! driver fails somewhere else entirely. So this sweeps for the second direction only.
//!
//! ⚠ It sweeps the whole BAR0 offset space at dword stride rather than a hand-picked list,
//! because a hand-picked list tests the offsets I already thought of — and the registers this
//! would break are by definition the ones nobody listed.

use kayfabe_device::{NanoClock, SteppingClock, abi};
use kayfabe_device::plane::RegPlane;

fn plane() -> RegPlane {
    RegPlane::new(
        &kayfabe_device::ga10x::GA106,
        abi::gsp_abi_for(kayfabe_abi::versions::BENCH_DRIVER).expect("bench table"),
        Box::new(SteppingClock::new(1)) as Box<dyn NanoClock>,
    )
    .expect("GA106 is servable")
}

/// ★★★ **Every offset the filter rejects, the locked path must also reject.**
///
/// ⊘ Asserted over a SWEEP, not a sample: the whole point is offsets nobody enumerated.
#[test]
fn a_state_free_filter_never_hides_a_served_register() {
    let p = plane();
    let (mut rejected, mut admitted) = (0u64, 0u64);

    // BAR0 is 16 MiB on this chip; dword stride covers every register it can decode.
    for off in (0..0x0100_0000u64).step_by(4) {
        if p.model_may_claim_for_test(0, off) {
            admitted += 1;
        } else {
            rejected += 1;
            assert!(
                !p.model_would_serve_for_test(0, off),
                "offset {off:#x} is REJECTED by the state-free filter but the locked path \
                 serves it — the guest would read a defaulted zero where a value belongs, \
                 with nothing logged and no counter moved"
            );
        }
    }

    // ⊘ NON-VACUITY, both ways. A filter that admitted everything would pass the assertion
    // above while saving nothing; one that rejected everything would pass it only because the
    // locked path is never consulted. Both numbers must be non-zero for the sweep to mean
    // anything.
    assert!(
        admitted > 0,
        "the filter admitted NOTHING — then the assertion above was never exercised against a \
         served register and this test proves nothing"
    );
    assert!(
        rejected > 0,
        "the filter rejected NOTHING — then it saves no lock at all and the 121 793 unclaimed \
         reads still pay rank 0"
    );
}
