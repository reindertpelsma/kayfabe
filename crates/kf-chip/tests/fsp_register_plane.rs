//! 2026-10-31 -- the register-plane contracts of the retired `gb20x_doorbell_and_regs.rs`
//! (`archive/crates/kayfabe-chips-modules/tests/`), over v3's `FspGspModel` (Hopper and Blackwell).
//! Four of its nine tests were register-plane tests; the token half is in
//! `kf-trap/src/tokenindex.rs`, and the boot-gate value itself is
//! `bar0_boot_regs::fsp_families_get_the_boot_gate_and_config_cycle_link_caps_at_their_die_groups_offsets`.
use kf_arch::gsp::{ArchBootState, BootContext, GspModel, GspObservation, GspReg};
use kf_chip::Family;
use kf_chip::fsp_gsp::{WPR2_ADDR_HI_580, WPR2_ADDR_LO_580};

/// `MC_GET_ARCH_INFO` implementation of a discrete die (GB202 = 2 ... the FSP model does not read it).
const IMPL: u32 = 0x2;

fn model(f: Family) -> Box<dyn GspModel> {
    f.gsp_model(IMPL, 8192)
        .expect("the FSP families have a model")
}

/// `both_driver_generations_wpr2_addresses_decode`: the WPR2 register moved BY DRIVER VERSION on
/// Blackwell -- a 580 guest polls `NV_PFB_PRI_MMU_WPR2_ADDR_HI` (`ogkm-580: hopper/gh100/dev_fb.h:48`,
/// 0x1FA828), a 610 one `NV_HUBMMU0_PRI_BASE + NV_HUBMMU_PRI_MMU_WPR2_ADDR_HI`
/// (`ogkm-610: blackwell/gb100/dev_hubmmu_base.h:94`, 0x88A828). Serving one is the `dlen = 0`
/// shape: the unserved address is unclaimed, an unclaimed BAR0 read answers 0, and the guest sits in
/// `kgspWaitForGfwBootOk` forever with nothing refused. Hopper has only the 580 pair.
#[test]
fn both_driver_generations_wpr2_addresses_decode_on_blackwell_and_only_580s_on_hopper() {
    let bw = model(Family::Blackwell);
    for (off, want, era) in [
        (0x0088_A824u64, GspReg::Wpr2AddrLo, "610"),
        (0x0088_A828, GspReg::Wpr2AddrHi, "610"),
        (WPR2_ADDR_LO_580, GspReg::Wpr2AddrLo, "580"),
        (WPR2_ADDR_HI_580, GspReg::Wpr2AddrHi, "580"),
    ] {
        assert_eq!(
            bw.decode_reg(0, off),
            Some(want),
            "the {era}-era WPR2 address {off:#010x} is UNCLAIMED"
        );
    }
    assert_ne!(0x0088_A828u64, WPR2_ADDR_HI_580);
    let hopper = model(Family::Hopper);
    assert_eq!(
        hopper.decode_reg(0, WPR2_ADDR_HI_580),
        Some(GspReg::Wpr2AddrHi)
    );
    assert_eq!(
        hopper.decode_reg(0, 0x0088_A828),
        None,
        "Hopper has no HUBMMU pair: serving it would claim a register the die does not have"
    );
}

/// `the_gb202_fsp_boot_complete_gate_is_served_at_the_moved_offset`, the claim half: the gate is
/// CLAIMED at GB202's `NV_THERM_I2CS_SCRATCH` (0x00AD00BC, `ogkm-580: blackwell/gb202/dev_therm.h:27`)
/// and answers `_STATUS_SUCCESS` (0xFF); Hopper's 0x000200BC is NOT claimed on the Blackwell model
/// (reaching for the wrong header must be visible), and Hopper claims nothing.
#[test]
fn the_blackwell_boot_gate_is_claimed_at_its_own_offset_and_not_hoppers() {
    const GB202: u64 = 0x00AD_00BC;
    const HOPPER: u64 = 0x0002_00BC;
    let m = model(Family::Blackwell);
    let seq = m.boot_sequence();
    let (ctx, st) = (BootContext::default(), ArchBootState::default());
    assert!(
        seq.may_read(0, GB202),
        "an unclaimed gate reads a defaulted 0 with no fault"
    );
    assert_eq!(seq.on_read(&*m, 0, GB202, &ctx, &st), Some(0xFF));
    assert!(!seq.may_read(0, HOPPER));
    let h = model(Family::Hopper);
    assert!(
        !h.boot_sequence().may_read(0, HOPPER),
        "Hopper has no thermal gate"
    );
    assert!(!h.boot_sequence().may_read(0, GB202));
}

/// `the_five_absent_registers_refuse_in_both_directions` and
/// `every_claimed_register_round_trips_through_its_own_offset`: the registers the FSP regime does
/// not have (GFW boot progress/PLM -- `gpuWaitForGfwBootComplete` is the not-supported stub past
/// AD107, `ogkm-580: g_gpu_nvoc.c:2378-2385` -- and the SEC2 Booter registers) have no offset and
/// encode to nothing, never to zero; every register it does claim decodes back to itself from its
/// own offset, so a transposed constant cannot survive a spot check.
#[test]
fn the_absent_registers_refuse_in_both_directions_and_every_claimed_one_round_trips() {
    let obs = GspObservation::default();
    for f in [Family::Blackwell, Family::Hopper] {
        let m = model(f);
        for reg in [
            GspReg::GfwBootProgress,
            GspReg::GfwBootPlm,
            GspReg::Sec2FalconCpuctl,
            GspReg::Sec2FalconMailbox0,
            GspReg::Sec2FalconDmatrfcmd,
        ] {
            assert_eq!(m.at(reg), None, "{f:?} {reg:?} has no offset");
            assert_eq!(
                m.encode(reg, &obs),
                None,
                "{f:?} {reg:?} must be unserviceable, not zero"
            );
        }
        let claimed = [
            GspReg::GspFalconCpuctl,
            GspReg::GspFalconHwcfg2,
            GspReg::GspFalconDmatrfcmd,
            GspReg::GspFalconMailbox0,
            GspReg::GspFalconMailbox1,
            GspReg::GspFalconIrqstat,
            GspReg::GspFalconIrqmask,
            GspReg::GspFalconIrqdest,
            GspReg::GspFalconIrqsclr,
            GspReg::GspRiscvCpuctl,
            GspReg::GspRiscvIrqmask,
            GspReg::GspRiscvIrqdest,
            GspReg::GspRiscvBcrCtrl,
            GspReg::Wpr2AddrLo,
            GspReg::Wpr2AddrHi,
            GspReg::GspQueueHead(0),
            GspReg::GspQueueHead(7),
        ];
        for reg in claimed {
            let (bar, off) = m
                .at(reg)
                .unwrap_or_else(|| panic!("{f:?} {reg:?} is claimed"));
            assert_eq!(
                m.decode_reg(bar, off),
                Some(reg),
                "{f:?} {reg:?} at {off:#010x}"
            );
            assert!(
                m.encode(reg, &obs).is_some(),
                "{f:?} {reg:?} must be served"
            );
        }
        assert_eq!(claimed.len(), 17, "NON-VACUITY: seventeen registers");
        assert_eq!(
            m.at(GspReg::GspQueueHead(8)),
            None,
            "the queue-head array ends at 8"
        );
    }
}
