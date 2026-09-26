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
        let f = Bar0Facts { architecture: arch, implementation: 2, revision: 0xA1, fb_mb: 8192, pcie_link_caps: pcie_link_caps(PcieGen::Gen4, 16).unwrap() };
        let regs = boot_regs(fam, &f);
        let b42 = regs.iter().find(|r| r.name == "NV_PMC_BOOT_42").unwrap().value;
        assert_eq!((b42 >> 24) & 0x3F, arch >> 4, "{fam:?}");
        let has_fb = regs.iter().any(|r| r.name == "NV_USABLE_FB_SIZE_IN_MB");
        // ⊘ CORRECTED 2026-09-26: kmemsysReadUsableFbSize_GA102 is bound for GA10x, AD10x, GH100 and
        // every GB die (g_kern_mem_sys_nvoc.c:353-357); Turing binds _GP102 (LOCAL_MEMORY_RANGE).
        assert_eq!(has_fb, fam != Family::Turing, "{fam:?}: USABLE_FB_SIZE_IN_MB");
        let has_range = regs.iter().any(|r| r.name == "NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE");
        assert_eq!(has_range, fam == Family::Turing, "{fam:?}: LOCAL_MEMORY_RANGE (TU10x/GA100 only)");
    }
}

#[test]
fn the_fb_size_served_is_the_stores_not_a_die_constant() {
    let f = Bar0Facts { architecture: 0x170, implementation: 6, revision: 0xA1, fb_mb: 11_857, pcie_link_caps: 0 };
    let r = boot_regs(Family::Ampere, &f);
    assert_eq!(r.iter().find(|r| r.name == "NV_USABLE_FB_SIZE_IN_MB").unwrap().value, 11_857);
}

#[test]
fn the_fb_layout_reproduces_the_captured_bar1_pde_base_at_12_gib() {
    let l = kf_chip::bar0::fb_layout(12288 << 20).unwrap();
    assert_eq!(l.bar1_pde_base, 0x2_F1CA_C000, "the captured RTX 3060 value (cap1b record 141977)");
    assert_eq!(l.bar2_pde_base, 0x2_F339_2000, "the captured RTX 3060 bar2PdeBase (same record, byte 1672)");
    // ★ Both roots sit inside the carve-out (reserved == size), so the guest's heap never hands
    // them out, and they do not overlap each other.
    let carve = l.regions[1].base;
    for r in [l.bar1_pde_base, l.bar2_pde_base] {
        assert!(r >= carve && r + kf_chip::bar0::ROOT_PAGE_BYTES <= 12288 << 20, "{r:#x}");
    }
    assert!(l.bar1_pde_base.abs_diff(l.bar2_pde_base) >= kf_chip::bar0::ROOT_PAGE_BYTES);
    assert_eq!(l.regions[1].base + l.regions[1].reserved, 12288 << 20);
    assert!(kf_chip::bar0::fb_layout(0x1000_0000).is_none(), "a 256 MiB store cannot hold the carve-out");
}

#[test]
fn ada_advertises_the_sec2_scrubber_as_already_run() {
    // `kgspExecuteScrubberIfNeeded_AD102` skips when SECURE_SCRATCH_15[31:29] >= 3.
    for fam in Family::ALL {
        let f = Bar0Facts { architecture: 0x190, implementation: 6, revision: 0xA1, fb_mb: 8192, pcie_link_caps: 0 };
        let r = boot_regs(fam, &f);
        let s = r.iter().find(|r| r.off == 0x0011_80FC);
        assert_eq!(s.is_some(), fam == Family::Ada, "{fam:?}");
        if let Some(s) = s {
            assert!((s.value >> 29) & 7 >= 3);
        }
    }
}

#[test]
fn local_memory_range_states_the_store_size_exactly_or_not_at_all() {
    use kf_chip::bar0::local_memory_range_gp102 as enc;
    let size = |v: u32| u64::from((v >> 4) & 0x3F) << (v & 0xF);
    for mb in [8192u64, 11_857, 6144, 12_288, 24_576, 1, 63, 64] {
        match enc(mb) {
            Some(v) => assert_eq!(size(v), mb, "{mb} MiB"),
            None => assert!(mb == 11_857, "{mb} MiB is representable"),
        }
    }
}

/// ★ The link word carries the HOST's width: an x16 GA106 host reproduces the old ×16 word, an x8
/// AD106 host presents ×8 (2026-09-26). A width PCIe does not define is refused.
#[test]
fn the_link_width_is_the_hosts() {
    let x16 = pcie_link_caps(PcieGen::Gen4, 16).unwrap();
    assert_eq!(x16, kf_abi::businfo::PcieLinkCaps::fully_trained(PcieGen::Gen4).encode(), "GA106 unchanged");
    let x8 = pcie_link_caps(PcieGen::Gen4, 8).unwrap();
    assert_eq!(kf_abi::businfo::PcieLinkCaps::decode(x8).unwrap().max_width, 8);
    assert!(pcie_link_caps(PcieGen::Gen4, 0).is_none());
    assert!(pcie_link_caps(PcieGen::Gen4, 3).is_none());
}

/// ★ The ROM is the host's identity + the host's VBIOS version + the generated geometry, for
/// every falcon-boot family — never a die row's version.
#[test]
fn the_vbios_carries_the_hosts_version_and_the_generated_geometry() {
    let id = kf_chip::bar0::PciIdentity { vendor: 0x10de, device: 0x2803, class: [0, 0, 3] };
    // ★ FSP families read no VBIOS image (`kgspExtractVbiosFromRom_395e98`): same image, inert FWSEC.
    for fam in Family::ALL {
        let p = kf_chip::bar0::vbios_profile(fam, id, (0x9507_1d00, 0x28)).unwrap();
        assert_eq!((p.vbios_version, p.vbios_oem_version), (0x9507_1d00, 0x28), "{fam:?}");
        assert_eq!(p.pci_device_id, 0x2803);
        assert_eq!(p.fwsec, kf_abi::vbios::GENERATED_FWSEC);
    }
}

/// ★ A host without a VBIOS answer gets the NAMED neutral version, not another die's.
#[test]
fn the_neutral_vbios_version_is_named_and_not_a_die_row() {
    let id = kf_chip::bar0::PciIdentity { vendor: 0x10de, device: 0x2504, class: [0, 0, 3] };
    let p = kf_chip::bar0::vbios_profile(Family::Ampere, id, kf_abi::vbios::NEUTRAL_VBIOS_VERSION).unwrap();
    assert_eq!((p.vbios_version, p.vbios_oem_version), (0, 0));
    assert!(kf_abi::vbios::VBIOS_PROFILES.iter().all(|r| r.vbios_version != p.vbios_version));
}
