//! ★ w827 — `NV_PRISCV_RISCV_BCR_CTRL` answers the core the guest last selected with `VALID` set,
//! on every GSP model that decodes it. Unmodelled, the shadow read back the guest's own
//! `CORE_SELECT_FALCON` write (`0`) and `kflcnSwitchToFalcon_GA102` spun its 4 s timeout on every
//! adapter shutdown (`ogkm-580: kernel_falcon_ga102.c:126-169`).
use kf_arch::gsp::{GspModel, GspObservation, GspReg};

fn check(m: &dyn GspModel, name: &str) {
    let (bar, off) = m.at(GspReg::GspRiscvBcrCtrl).unwrap_or_else(|| panic!("{name}: BCR_CTRL has no offset"));
    assert_eq!((bar, off), (0, 0x0011_1668), "{name}: NV_FALCON2_GSP_BASE + 0x668");
    assert_eq!(m.decode_reg(0, off), Some(GspReg::GspRiscvBcrCtrl), "{name}: decodes back");
    let never = GspObservation::default();
    assert_eq!(m.encode(GspReg::GspRiscvBcrCtrl, &never), Some(0), "{name}: reset value FALCON, not VALID");
    // Boot: kflcnRiscvProgramBcr writes CORE_SELECT_RISCV | VALID | BRFETCH.
    let boot = GspObservation { riscv_bcr_ctrl: Some(0x111), ..GspObservation::default() };
    assert_eq!(m.encode(GspReg::GspRiscvBcrCtrl, &boot), Some(0x111), "{name}: RISC-V selected, VALID");
    // Teardown: CORE_SELECT_FALCON (0) — the switch completes: VALID, FALCON.
    let down = GspObservation { riscv_bcr_ctrl: Some(0), ..GspObservation::default() };
    let v = m.encode(GspReg::GspRiscvBcrCtrl, &down).unwrap();
    assert_eq!(v & 1, 1, "{name}: VALID_TRUE ends kflcnSwitchToFalcon's wait");
    assert_eq!(v & 0x10, 0, "{name}: CORE_SELECT_FALCON as written");
}

#[test]
fn every_gsp_model_acknowledges_the_core_switch() {
    check(&*kf_chip::Family::Ampere.gsp_model(8192).unwrap(), "Ampere");
    check(&*kf_chip::Family::Ada.gsp_model(8192).unwrap(), "Ada");
    check(&*kf_chip::Family::Hopper.gsp_model(8192).unwrap(), "Hopper");
    check(&*kf_chip::Family::Blackwell.gsp_model(8192).unwrap(), "Blackwell");
}
