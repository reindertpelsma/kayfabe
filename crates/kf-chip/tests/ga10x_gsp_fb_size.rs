//! The GA10x GSP model's WPR2 answers are a function of the FB size it was BUILT with — never a
//! compile-time constant (v3 w826: the GA106 `FB_SIZE_MB = 12288` default is deleted).
use kf_arch::gsp::{GspModel, GspObservation, GspReg};
use kf_chip::ga10x_gsp::{Ga10xGspModel, frts_offset_for, gsp_fw_wpr_end_for};

fn wpr2(m: &Ga10xGspModel) -> (u64, u64) {
    let obs = GspObservation { wpr2_up: true, ..GspObservation::default() };
    (m.encode(GspReg::Wpr2AddrLo, &obs).unwrap(), m.encode(GspReg::Wpr2AddrHi, &obs).unwrap())
}

#[test]
fn wpr2_follows_the_size_the_model_was_built_with() {
    let a = Ga10xGspModel::with_fb_size_mb(256);
    let b = Ga10xGspModel::with_fb_size_mb(11_857);
    assert_ne!(wpr2(&a), wpr2(&b), "two sizes, two WPR2 windows");
    // The derivation: FRTS sits 1 MiB below the aligned end of the PRAMIN-trimmed framebuffer.
    let end = gsp_fw_wpr_end_for(256);
    assert_eq!(end, (256 << 20) - 0x10_0000, "256 MiB is already 128 KiB-aligned below PRAMIN");
    assert_eq!(frts_offset_for(256), end - 0x10_0000);
    assert_eq!(wpr2(&a), ((frts_offset_for(256) >> 12) << 4, (end >> 12) << 4));
}
