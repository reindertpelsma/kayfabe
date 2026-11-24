//! ★ 2026-10-14 (die sweep, GB206 + Windows 580.88, `docs/design/V3_DIE_SWEEP_20261014.md` §3.1) — the GSP reset
//! handshake the Windows RM runs between the FSP capability query and its boot commands. Every expectation is
//! the ogkm header's / HAL body's, not a measurement of a die:
//! `kgspResetHw_GH100` (`kernel_gsp_gh100.c:113-150`) writes `NV_PGSP_FALCON_ENGINE.RESET = ASSERT`, waits for
//! `RESET_STATUS == ASSERTED` (0), writes `DEASSERT`, waits for `DEASSERTED` (2); `kgspResetHw_GB100`
//! (`kernel_gsp_gb100.c:68-91`) first waits for `NV_PGSP_FALCON_RESET_PRIV_LEVEL_MASK`'s `READ_PROTECTION_LEVEL0`
//! and `WRITE_PROTECTION_LEVEL0` to read `_ENABLE`.
use kf_abi::businfo::PcieGen;
use kf_arch::gsp::{GspModel, GspObservation, GspReg};
use kf_chip::Family;
use kf_chip::bar0::{Bar0Facts, boot_regs, pcie_link_caps};

const ENGINE: u64 = 0x0011_03c0;
const PLM: u64 = 0x0011_03c4;
const DISCRETE_IMPL: u32 = 0x6;

fn facts(arch: u32) -> Bar0Facts {
    Bar0Facts {
        architecture: arch,
        implementation: 3,
        revision: 0xA1,
        fb_mb: 8192,
        pcie_link_caps: pcie_link_caps(PcieGen::Gen4, 16).unwrap(),
        bar1_bytes: 128 << 20,
    }
}

/// What the guest reads from `NV_PGSP_FALCON_ENGINE` after writing `w`, the way the device serves it: the vCPU
/// stores `answer_on_store` at once; the drainer later publishes `encode` of the FSM's observation, which has
/// recorded the same write.
fn engine_after_write(m: &dyn GspModel, w: u64) -> (u64, u64) {
    let now = m.answer_on_store(GspReg::GspFalconEngine, w).unwrap();
    let obs = GspObservation {
        falcon_engine: Some(w as u32),
        ..GspObservation::default()
    };
    (now, m.encode(GspReg::GspFalconEngine, &obs).unwrap())
}

#[test]
fn the_reset_status_follows_each_reset_write_on_the_fsp_generations() {
    for fam in [Family::Hopper, Family::Blackwell] {
        let m = fam.gsp_model(DISCRETE_IMPL, 8192).unwrap();
        assert_eq!(m.at(GspReg::GspFalconEngine), Some((0, ENGINE)), "{fam:?}");
        assert_eq!(m.decode_reg(0, ENGINE), Some(GspReg::GspFalconEngine));
        // never written: out of reset, DEASSERTED (field 10:8 = 2)
        assert_eq!(
            m.encode(GspReg::GspFalconEngine, &GspObservation::default()),
            Some(0x200),
            "{fam:?}"
        );
        // RESET_ASSERT (1): RESET reads 1, RESET_STATUS ASSERTED (0) — what `_kgspWaitForAsserted` tests
        let (now, drained) = engine_after_write(&*m, 1);
        assert_eq!((now, drained), (0x1, 0x1), "{fam:?}");
        assert_eq!((now >> 8) & 0x7, 0);
        // RESET_DEASSERT (0): RESET reads 0, RESET_STATUS DEASSERTED (2) — what `_kgspWaitForDeasserted` tests
        let (now, drained) = engine_after_write(&*m, 0);
        assert_eq!((now, drained), (0x200, 0x200), "{fam:?}");
        assert_eq!((now >> 8) & 0x7, 2);
        // RESET_STATUS is read-only: bits the guest writes above RESET never come back
        assert_eq!(engine_after_write(&*m, 0xffff_fffe).0, 0x200, "{fam:?}");
        assert_eq!(engine_after_write(&*m, 0xffff_ffff).0, 0x1, "{fam:?}");
    }
}

#[test]
fn the_falcon_generations_have_no_reset_status_to_model() {
    for fam in [Family::Turing, Family::Ampere, Family::Ada] {
        let m = fam.gsp_model(DISCRETE_IMPL, 8192).unwrap();
        assert_eq!(m.at(GspReg::GspFalconEngine), None, "{fam:?}");
        assert_eq!(m.decode_reg(0, ENGINE), None, "{fam:?}");
    }
}

/// The reset PLM is served on Blackwell only, as the two fields `_kgspWaitForResetAccess` tests.
#[test]
fn the_blackwell_reset_plm_reads_level0_read_and_write_enabled() {
    let plm = |fam, arch| {
        boot_regs(fam, &facts(arch))
            .into_iter()
            .find(|r| r.name == "NV_PGSP_FALCON_RESET_PRIV_LEVEL_MASK")
    };
    let r = plm(Family::Blackwell, 0x1B0).expect("Blackwell serves the reset PLM");
    assert_eq!(r.off, PLM);
    assert_eq!(r.value & 1, 1, "READ_PROTECTION_LEVEL0 ENABLE (0:0)");
    assert_eq!(
        (r.value >> 4) & 1,
        1,
        "WRITE_PROTECTION_LEVEL0 ENABLE (4:4)"
    );
    assert_eq!(r.value, 0x11);
    for (fam, arch) in [
        (Family::Hopper, 0x180),
        (Family::Ada, 0x190),
        (Family::Ampere, 0x170),
        (Family::Turing, 0x160),
    ] {
        assert!(
            plm(fam, arch).is_none(),
            "{fam:?}: kgspResetHw_GH100 has no PLM wait"
        );
    }
}
