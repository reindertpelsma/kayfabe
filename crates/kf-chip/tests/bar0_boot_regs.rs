//! The boot registers are DERIVED; the old tree's captured GA106 row is the ORACLE, never the source.
use kf_abi::businfo::PcieGen;
use kf_chip::Family;
use kf_chip::bar0::{Bar0Facts, boot_regs, pcie_link_caps, pmc_boot_0, pmc_boot_42};

/// `kayfabe-device/ga10x.rs:578-582` — read off a real GA106 A1.
const PMC_BOOT_0_GA106_A1: u32 = 0x1760_00A1;
const PMC_BOOT_42_GA106_A1: u32 = 0x176A_1000;

#[test]
fn on_a_ga106_host_derived_equals_captured() {
    assert_eq!(pmc_boot_0(0x170, 6, 0xA1), PMC_BOOT_0_GA106_A1);
    assert_eq!(pmc_boot_42(0x170, 6, 0xA1), PMC_BOOT_42_GA106_A1);
}

#[test]
fn every_family_gets_its_own_architecture_in_boot_42() {
    for (arch, fam) in [(0x160, Family::Turing), (0x190, Family::Ada), (0x180, Family::Hopper), (0x1B0, Family::Blackwell)] {
        let f = Bar0Facts { architecture: arch, implementation: 2, revision: 0xA1, fb_mb: 8192, pcie_link_caps: pcie_link_caps(PcieGen::Gen4) };
        let regs = boot_regs(fam, &f);
        let b42 = regs.iter().find(|r| r.name == "NV_PMC_BOOT_42").unwrap().value;
        assert_eq!((b42 >> 24) & 0x3F, arch >> 4, "{fam:?}");
        let has_fb = regs.iter().any(|r| r.name == "NV_USABLE_FB_SIZE_IN_MB");
        assert_eq!(has_fb, matches!(fam, Family::Turing | Family::Ada), "{fam:?}: USABLE_FB_SIZE_IN_MB is ogkm-published only through Ada");
    }
}

#[test]
fn the_fb_size_served_is_the_stores_not_a_die_constant() {
    let f = Bar0Facts { architecture: 0x170, implementation: 6, revision: 0xA1, fb_mb: 11_857, pcie_link_caps: 0 };
    let r = boot_regs(Family::Ampere, &f);
    assert_eq!(r.iter().find(|r| r.name == "NV_USABLE_FB_SIZE_IN_MB").unwrap().value, 11_857);
}
