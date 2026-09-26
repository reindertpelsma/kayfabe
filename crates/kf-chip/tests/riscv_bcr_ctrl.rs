//! ★ w827 — `NV_PRISCV_RISCV_BCR_CTRL` answers the core the guest last selected with `VALID` set,
//! on every GSP model that decodes it. Unmodelled, the shadow read back the guest's own
//! `CORE_SELECT_FALCON` write (`0`) and `kflcnSwitchToFalcon_GA102` spun its 4 s timeout on every
//! adapter shutdown (`ogkm-580: kernel_falcon_ga102.c:126-169`).
use kf_arch::gsp::{GspModel, GspObservation, GspReg};

/// A discrete die's `MC_GET_ARCH_INFO` implementation (GA106/AD106/TU106 = 6): the `_GA102` group on
/// Ampere, where GA100 (0) is a different one (`kf_chip::Family::gsp_model`).
const DISCRETE_IMPL: u32 = 0x6;

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
    check(&*kf_chip::Family::Ampere.gsp_model(DISCRETE_IMPL, 8192).unwrap(), "Ampere");
    check(&*kf_chip::Family::Ada.gsp_model(DISCRETE_IMPL, 8192).unwrap(), "Ada");
    check(&*kf_chip::Family::Hopper.gsp_model(DISCRETE_IMPL, 8192).unwrap(), "Hopper");
    check(&*kf_chip::Family::Blackwell.gsp_model(DISCRETE_IMPL, 8192).unwrap(), "Blackwell");
}

/// ★ Turing (and GA100) carry the `_TU102` RISC-V block: no `BCR_CTRL` at all
/// (`kflcnSwitchToFalcon_TU102` only updates software state), RISC-V "active" is
/// `CORE_SWITCH_RISCV_STATUS.ACTIVE_STAT` at `+0x240` bit 0, and `IRQMASK`/`IRQDEST` sit at
/// `+0x2b4`/`+0x2b8` (`ogkm-580: turing/tu102/dev_riscv_pri.h:27-33`).
#[test]
fn turing_has_the_tu102_riscv_block_and_no_core_switch() {
    let m = kf_chip::Family::Turing.gsp_model(DISCRETE_IMPL, 8192).expect("Turing has a model");
    assert_eq!(m.at(GspReg::GspRiscvBcrCtrl), None);
    assert_eq!(m.decode_reg(0, 0x0011_1668), None, "GA102's BCR_CTRL is not a Turing register");
    assert_eq!(m.at(GspReg::GspRiscvCpuctl), Some((0, 0x0011_1240)));
    assert_eq!(m.at(GspReg::GspRiscvIrqmask), Some((0, 0x0011_12b4)));
    assert_eq!(m.at(GspReg::GspRiscvIrqdest), Some((0, 0x0011_12b8)));
    assert_eq!(m.decode_reg(0, 0x0011_1240), Some(GspReg::GspRiscvCpuctl));
    let up = GspObservation { riscv_active: true, ..GspObservation::default() };
    assert_eq!(m.encode(GspReg::GspRiscvCpuctl, &up), Some(0x1), "ACTIVE_STAT is 0:0 on TU102");
    assert_eq!(m.encode(GspReg::GspRiscvCpuctl, &GspObservation::default()), Some(0));
    // Everything else is the GA102 map: the falcon, queue, GFW-boot and WPR2 offsets are shared.
    let ga = kf_chip::Family::Ampere.gsp_model(DISCRETE_IMPL, 8192).unwrap();
    for r in [GspReg::GspFalconMailbox0, GspReg::GspFalconCpuctl, GspReg::Wpr2AddrLo, GspReg::GfwBootProgress, GspReg::GspQueueHead(0)] {
        assert_eq!(m.at(r), ga.at(r), "{r:?}");
    }
}
