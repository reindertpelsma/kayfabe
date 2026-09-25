//! ★★★★★ w828 — **what each family's GSP looks like after the guest's last close**, and that it
//! is the state the next `RmInitAdapter` needs: WPR2 down (`_kgspBootGspRm`'s gate,
//! `ogkm-580: kernel_gsp.c:3872-3880`), and on the FSP regime the two edges `kgspUnloadRm`
//! waits on — the suspend sentinel in `MAILBOX0` (`kernel_gsp_tu102.c:1226-1249`) and the RISC-V
//! core HALTED (`kgspTeardown_GH100`, `kernel_gsp_gh100.c:995-1004`).
use kf_arch::gsp::{AfterSuspend, BootPhase, GspModel, GspObservation, GspReg};

fn obs(stage: BootPhase, suspended: bool) -> GspObservation {
    GspObservation { stage, wpr2_up: stage.wpr2_up(), riscv_active: stage.wpr2_up(), suspended, ..GspObservation::default() }
}

fn wpr2_hi(m: &dyn GspModel, o: &GspObservation) -> u64 {
    m.encode(GspReg::Wpr2AddrHi, o).unwrap()
}

#[test]
fn the_falcon_regime_waits_for_its_teardown_ucode() {
    for f in [kf_chip::Family::Ampere, kf_chip::Family::Ada] {
        let m = f.gsp_model(8192).unwrap();
        assert_eq!(m.boot_sequence().after_suspend(), AfterSuspend::AwaitsTeardownUcode, "{f:?}");
        let susp = obs(BootPhase::Suspending, true);
        assert_eq!(m.encode(GspReg::GspFalconMailbox0, &susp), Some(0x8000_0000), "{f:?}: the sentinel, whole");
        assert_ne!(wpr2_hi(&*m, &susp), 0, "{f:?}: WPR2 is still up until FWSEC-SB / Booter Unload");
        assert_eq!(wpr2_hi(&*m, &obs(BootPhase::Halted, false)), 0, "{f:?}: after E2/E4 the next boot's gate passes");
    }
}

#[test]
fn the_fsp_regime_halts_itself_and_reads_as_the_driver_waits_for() {
    for f in [kf_chip::Family::Hopper, kf_chip::Family::Blackwell] {
        let m = f.gsp_model(8192).unwrap();
        assert_eq!(m.boot_sequence().after_suspend(), AfterSuspend::FirmwareHalts, "{f:?}");
        // What the FSM shows after E9 → E13 (`kf_gsp` a_life_ends_and_the_next_one_boots).
        let done = obs(BootPhase::Halted, true);
        assert_eq!(m.encode(GspReg::GspFalconMailbox0, &done), Some(0x8000_0000), "{f:?}: kgspWaitForProcessorSuspend");
        let cpuctl = m.encode(GspReg::GspRiscvCpuctl, &done).unwrap();
        assert_eq!(cpuctl & 0x10, 0x10, "{f:?}: kflcnWaitForHaltRiscv sees HALTED");
        assert_eq!(cpuctl & 0x80, 0, "{f:?}: and not ACTIVE");
        assert_eq!(wpr2_hi(&*m, &done), 0, "{f:?}: WPR2 down — the next open's gate passes");
        // A boot in progress: MAILBOX0 is the FMC's error channel again, and must read zero.
        for s in [BootPhase::ProtectedRegionUp, BootPhase::Booted, BootPhase::Running] {
            assert_eq!(m.encode(GspReg::GspFalconMailbox0, &obs(s, false)), Some(0), "{f:?} {s:?}: no FMC error");
            assert_eq!(m.encode(GspReg::GspRiscvCpuctl, &obs(s, false)).map(|v| v & 0x10), Some(0), "{f:?} {s:?}: not halted");
        }
    }
}
