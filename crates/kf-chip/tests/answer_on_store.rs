//! ★★★★★ w828 — `GspModel::answer_on_store` is served on the vCPU, lock-free, BEFORE the drainer
//! has applied the write. It is only correct if it is exactly what the drainer will publish
//! afterwards, whatever state the FSM is in — and it must never answer a completion edge.
use kf_arch::gsp::{BootPhase, GspModel, GspObservation, GspReg};

/// A discrete die's `MC_GET_ARCH_INFO` implementation (GA106/AD106/TU106 = 6): the `_GA102` group on
/// Ampere, where GA100 (0) is a different one (`kf_chip::Family::gsp_model`).
const DISCRETE_IMPL: u32 = 0x6;

const STAGES: [BootPhase; 6] = [
    BootPhase::Cold,
    BootPhase::ProtectedRegionUp,
    BootPhase::Booted,
    BootPhase::Running,
    BootPhase::Suspending,
    BootPhase::Halted,
];

/// Every observation a write could leave behind, for a register whose written value the FSM
/// records (`BCR_CTRL`) or does not (everything else).
fn observations(reg: GspReg, written: u64) -> Vec<GspObservation> {
    let mut v = Vec::new();
    for stage in STAGES {
        for suspended in [false, true] {
            #[allow(clippy::cast_possible_truncation)]
            let bcr = if reg == GspReg::GspRiscvBcrCtrl { Some(written as u32) } else { None };
            v.push(GspObservation {
                stage,
                wpr2_up: stage.wpr2_up(),
                riscv_active: stage.wpr2_up(),
                suspended,
                riscv_bcr_ctrl: bcr,
                ..GspObservation::default()
            });
        }
    }
    v
}

fn check(m: &dyn GspModel, name: &str) {
    let written = [0u64, 1, 0x10, 0x111, 0x2, 0xffff_ffff, 0x0000_0123_4567];
    let mut answered = 0;
    for reg in GspReg::FIXED {
        for &w in &written {
            let Some(now) = m.answer_on_store(reg, w) else { continue };
            answered += 1;
            for obs in observations(reg, w) {
                assert_eq!(
                    m.encode(reg, &obs),
                    Some(now),
                    "{name}: {reg:?} written {w:#x} answered {now:#x} on the store, but the drainer would publish another value at {obs:?}"
                );
            }
        }
    }
    assert!(answered > 0, "{name}: a sweep that reports zero must first report one");
    // ⊘⊘ The completion edges and everything the FSM's transitions move: NEVER on the store.
    for reg in [
        GspReg::GspFalconCpuctl,
        GspReg::Sec2FalconCpuctl,
        GspReg::GspRiscvCpuctl,
        GspReg::GspFalconIrqstat,
        GspReg::Wpr2AddrLo,
        GspReg::Wpr2AddrHi,
        GspReg::GspFalconMailbox0,
        GspReg::GspFalconMailbox1,
    ] {
        assert_eq!(m.answer_on_store(reg, 0x2), None, "{name}: {reg:?} orders an FSM effect — the drainer publishes it");
    }
    // The two the w828 boot needed.
    assert_eq!(m.answer_on_store(GspReg::GspFalconDmatrfcmd, 0x0000_0610) .map(|v| v & 0x3), Some(0x2), "{name}: DMATRFCMD reads IDLE, not FULL, the instant the command lands");
    assert_eq!(m.answer_on_store(GspReg::GspRiscvBcrCtrl, 0).map(|v| v & 0x11), Some(0x1), "{name}: CORE_SELECT_FALCON acknowledged VALID at once");
}

#[test]
fn a_store_answer_is_what_the_drainer_would_publish_in_every_state() {
    for (f, name) in [
        (kf_chip::Family::Ampere, "Ampere"),
        (kf_chip::Family::Ada, "Ada"),
        (kf_chip::Family::Hopper, "Hopper"),
        (kf_chip::Family::Blackwell, "Blackwell"),
    ] {
        check(&*f.gsp_model(DISCRETE_IMPL, 8192).unwrap(), name);
    }
}
