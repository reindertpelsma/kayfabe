//! The conformance suite's stake in the C↔Rust trace differential (task #47).
//!
//! The differential itself lives in `crates/kayfabe-crec` (decoder, GA10x register model,
//! oracle, replay, ledger classifier) with its own two test files. What belongs **here**
//! is the one thing that crate cannot check about itself: that the Axis-A bundle it
//! replays with is the *same* bundle the rest of the suite drives the GSP through.
//!
//! `kayfabe_crec::bench_abi()` and `gspworld::P580.abi()` are two derivations of the same
//! thing from the same production version tables. The duplication exists because
//! `kayfabe-crec` must be usable without the test crate while the test crate depends on
//! it, and it is held together by a **mechanism rather than a comment**: if either
//! derivation drifts — a different element layout, a different function id, a different
//! `queueElementSizeMax` — this file fails, and the differential's numbers stop meaning
//! what the rest of the suite's numbers mean.

use kayfabe_tests::gspworld::P580;

#[test]
fn the_differentials_axis_a_bundle_is_the_suites_axis_a_bundle() {
    assert_eq!(
        kayfabe_crec::bench_abi(),
        P580.abi(),
        "the trace differential replays with a different driver ABI than the rest of the \
         conformance suite drives the GSP with — every number it reports is then about a \
         different implementation than the one under test"
    );
}

#[test]
fn the_ga10x_model_is_a_second_real_implementation_of_the_axis_b_seam() {
    // ★ The seam's whole claim is that one FSM drives several register models. The suite
    // already drives it through two deliberately-fake ones; this is the first model built
    // from a real chip's published registers, and the property that matters is that it
    // agrees with *nothing* in the fakes except the abstract vocabulary.
    use kayfabe_arch::gsp::{GspModel, GspReg};
    use kayfabe_tests::gspworld::{MODEL_A, MODEL_B};

    let real = kayfabe_crec::Ga10xGspModel::new();
    for reg in [
        GspReg::GspFalconCpuctl,
        GspReg::GspFalconMailbox0,
        GspReg::Wpr2AddrHi,
        GspReg::GspQueueHead(0),
    ] {
        let (bar, off) = kayfabe_crec::Ga10xGspModel::at(reg).expect("GA10x places it");
        assert_eq!(real.decode_reg(bar, off), Some(reg), "round-trip {reg:?}");
        // The fakes must not answer the real chip's offsets as that register, or the
        // "one FSM, several models" claim would be an accident of overlapping maps.
        for fake in [MODEL_A, MODEL_B] {
            assert_ne!(
                (fake.decode_reg(bar, off), fake.at(reg)),
                (Some(reg), (bar, off)),
                "a fake model shares GA10x's encoding for {reg:?}, so the seam is untested \
                 there"
            );
        }
    }
    // An offset in the GSP falcon block that `GspReg` has no variant for must come back
    // `None` — "another model owns it", never a defaulted zero. `0x110094` is the falcon's
    // `DEBUGINFO`, which this capture shows the guest reading before **every** RPC submit
    // (177 times) as its GSP health check.
    assert_eq!(real.decode_reg(0, 0x0011_0094), None);
}
