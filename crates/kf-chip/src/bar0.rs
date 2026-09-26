//! ★★★ **The BAR0 boot registers — DERIVED from the host, never a captured die row.**
//!
//! The old tree served these from `kayfabe-device/ga10x.rs`'s seven `BootReg` rows, every value a
//! GA106 constant (`PMC_BOOT_0_GA106_A1`, `FB_SIZE_MB`, `XVE_LINK_CAPABILITIES_GA106`, …). v3 derives
//! each from a host fact (or an ogkm rule, or a documented advertisement), and the captured GA106
//! values survive only as the TEST ORACLE: on a GA106 host, derived == captured.
//!
//! Each row carries its [`Provenance`], so a reader can tell a host fact from a fabrication.

use crate::Family;

/// Where a served value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Composed from a host RM answer (named).
    Host(&'static str),
    /// A rule stated by ogkm's headers (named).
    Ogkm(&'static str),
    /// Fabricated so ogkm takes its success branch — a documented advertisement, not a model.
    Advertised(&'static str),
}

/// One served boot register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootReg {
    /// BAR0 offset.
    pub off: u64,
    /// The value served.
    pub value: u32,
    /// The register's name.
    pub name: &'static str,
    /// Where the value comes from.
    pub from: Provenance,
}

/// The host facts the boot registers are composed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bar0Facts {
    /// `MC_GET_ARCH_INFO.architecture` (e.g. `0x170`).
    pub architecture: u32,
    /// `MC_GET_ARCH_INFO.implementation`.
    pub implementation: u32,
    /// `MC_GET_ARCH_INFO.revision` — major in bits 7:4, minor in 3:0 (e.g. `0xA1`).
    pub revision: u32,
    /// The framebuffer the guest is told it has, in MiB — the size the STORE holds (constraint 15).
    pub fb_mb: u64,
    /// The PCIe link capabilities word to present (the host's own, see [`pcie_link_caps`]).
    pub pcie_link_caps: u32,
}

/// `NV_PMC_BOOT_0` (`ogkm-580 nv_ref.h:108-136`): architecture `28:24` (= `MC arch >> 4`),
/// implementation `23:20`, revision `7:0`. The layout is shared by every family.
#[must_use]
pub const fn pmc_boot_0(architecture: u32, implementation: u32, revision: u32) -> u32 {
    (((architecture >> 4) & 0x1F) << 24) | ((implementation & 0xF) << 20) | (revision & 0xFF)
}

/// `NV_PMC_BOOT_42` (`nv_ref.h:149`, fields from `nvswitch/ls10/dev_boot.h` — the same layout RM
/// decodes on every GPU): architecture `29:24`, implementation `23:20`, major `19:16`, minor `15:12`.
#[must_use]
pub const fn pmc_boot_42(architecture: u32, implementation: u32, revision: u32) -> u32 {
    (((architecture >> 4) & 0x3F) << 24)
        | ((implementation & 0xF) << 20)
        | (((revision >> 4) & 0xF) << 16)
        | ((revision & 0xF) << 12)
}

/// `NV_XVE_LINK_CAPABILITIES` for the HOST function's link: its maximum generation at its own
/// maximum width (sysfs `max_link_speed` / `max_link_width`, read at realize) — delegated to the
/// ABI's encoder (`kf_abi::businfo::PcieLinkCaps::host_link`). ⊘ Was `fully_trained` = ×16 on
/// every die; an AD106 is ×8 (2026-09-26, `V3_FAMILY_PORT_ADA.md` §2).
///
/// `None` for a width PCIe does not define.
#[must_use]
pub const fn pcie_link_caps(max_gen: kf_abi::businfo::PcieGen, max_width: u32) -> Option<u32> {
    match kf_abi::businfo::PcieLinkCaps::host_link(max_gen, max_width) {
        Some(l) => Some(l.encode()),
        None => None,
    }
}

const NV_PMC_BOOT_0: u64 = 0x0000_0000;
const NV_PMC_BOOT_1: u64 = 0x0000_0004;
const NV_PMC_BOOT_42: u64 = 0x0000_0A00;
/// `NV_PCFG` (`0x88000`, every family's `dev_nv_xve.h`) + `NV_XVE_LINK_CAPABILITIES` (`0x84`).
const NV_XVE_LINK_CAPABILITIES: u64 = 0x0008_8084;
/// `NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET` (`0xB80000`) + `_PRIV_ACCESS_COUNTER_NOTIFY_BUFFER_SIZE`
/// (`0x3110`) — identical in `turing/tu102/dev_vm.h:209` and `blackwell/gb100/dev_vm.h:503`.
const NV_VF_ACCESS_COUNTER_NOTIFY_BUFFER_SIZE: u64 = 0x00B8_3110;
/// `NV_USABLE_FB_SIZE_IN_MB` = `NV_PGC6_AON_SECURE_SCRATCH_GROUP_42` (`ampere/ga102/
/// dev_gc6_island_addendum.h:33`). ⊘ CORRECTED 2026-09-26 (family port): read by
/// `kmemsysReadUsableFbSize_GA102`, which ogkm binds for GA102–GA107, AD102–AD107, GH100 and every
/// GB die (`g_kern_mem_sys_nvoc.c:353-357`) — compiled against the GA102 header, so the SAME offset
/// on Hopper/Blackwell. It is NOT read on Turing or GA100 (they bind `_GP102`, below).
const NV_USABLE_FB_SIZE_IN_MB: u64 = 0x0011_83A4;
/// `NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE` (`published/pascal/gp102/dev_fb.h:26-29`): `LOWER_SCALE`
/// `3:0`, `LOWER_MAG` `9:4`, size = `mag << (scale + 20)`. Read by `kmemsysReadUsableFbSize_GP102`,
/// bound for TU10x and GA100 (`g_kern_mem_sys_nvoc.c:349-352`).
const NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE_GP102: u64 = 0x0010_0CE0;
/// `NV_PGC6_BSI_VPR_SECURE_SCRATCH_15` = `NV_PGC6_BSI_SECURE_SCRATCH_15` (`ogkm-580:
/// published/ada/ad102/dev_gc6_island.h:27`, addendum `:27-29`): `SCRUBBER_HANDOFF` is `31:29`,
/// `_DONE` = 3.
const NV_PGC6_BSI_VPR_SECURE_SCRATCH_15: u64 = 0x0011_80FC;
/// `SCRUBBER_HANDOFF_DONE << 29`.
const SCRUBBER_HANDOFF_DONE: u32 = 3 << 29;
/// `NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE_STATUS_SUCCESS` (`blackwell/gb202/dev_therm_addendum.h:29`,
/// the same value in the gh100/gb100 addenda).
const FSP_BOOT_COMPLETE_SUCCESS: u32 = 0xFF;

/// `NV_THERM_I2CS_SCRATCH` for the host's architecture: `0xAD00BC` on consumer Blackwell (GB20x,
/// `blackwell/gb202/dev_therm.h:27`), `0x200BC` on Hopper and datacenter Blackwell
/// (`hopper/gh100/dev_therm.h:26`, `blackwell/gb100/dev_therm.h:27`).
#[must_use]
pub const fn therm_i2cs_scratch(architecture: u32) -> u64 {
    if architecture == crate::arch::GB200 { 0x00AD_00BC } else { 0x0002_00BC }
}
/// Access-counter notify buffer: two pages of 32-byte entries — advertised, never written (the old
/// tree's `resume_from_fault.md` §S2 ruling: migration heuristics simply never fire).
const ACCESS_COUNTER_ENTRIES_ADVERTISED: u32 = 2 * (4096 / 32);

/// `NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE` for `fb_mb` MiB: the largest scale that states it EXACTLY
/// with a 6-bit magnitude, or `None` (never a rounded size).
#[must_use]
pub const fn local_memory_range_gp102(fb_mb: u64) -> Option<u32> {
    let mut scale = 15u32;
    loop {
        let unit = 1u64 << scale;
        if fb_mb % unit == 0 && fb_mb / unit >= 1 && fb_mb / unit <= 0x3F {
            return Some((((fb_mb / unit) as u32) << 4) | scale);
        }
        if scale == 0 {
            return None;
        }
        scale -= 1;
    }
}

/// ★ The boot registers for `family`, from `facts`. The framebuffer size is served through BOTH
/// usable-size HALs' registers wherever a die of the family reads one: `USABLE_FB_SIZE_IN_MB` on
/// Ampere (GA10x) … Blackwell, `LOCAL_MEMORY_RANGE` on Turing and Ampere (GA100) — a register a die
/// does not read is an unread shadow word.
#[must_use]
pub fn boot_regs(family: Family, f: &Bar0Facts) -> Vec<BootReg> {
    let mut v = vec![
        BootReg {
            off: NV_PMC_BOOT_0,
            value: pmc_boot_0(f.architecture, f.implementation, f.revision),
            name: "NV_PMC_BOOT_0",
            from: Provenance::Host("NV2080_CTRL_CMD_MC_GET_ARCH_INFO"),
        },
        BootReg {
            off: NV_PMC_BOOT_1,
            value: 0,
            name: "NV_PMC_BOOT_1",
            from: Provenance::Ogkm("VGPU = REAL: this device advertises no virtualization of its own"),
        },
        BootReg {
            off: NV_PMC_BOOT_42,
            value: pmc_boot_42(f.architecture, f.implementation, f.revision),
            name: "NV_PMC_BOOT_42",
            from: Provenance::Host("NV2080_CTRL_CMD_MC_GET_ARCH_INFO"),
        },
        BootReg {
            off: NV_XVE_LINK_CAPABILITIES,
            value: f.pcie_link_caps,
            name: "NV_XVE_LINK_CAPABILITIES",
            from: Provenance::Host("the host's PCIe link capability"),
        },
        BootReg {
            off: NV_VF_ACCESS_COUNTER_NOTIFY_BUFFER_SIZE,
            value: ACCESS_COUNTER_ENTRIES_ADVERTISED,
            name: "NV_VIRTUAL_FUNCTION_PRIV_ACCESS_COUNTER_NOTIFY_BUFFER_SIZE",
            from: Provenance::Advertised("uvmInitializeAccessCntrBuffer refuses a zero size; nothing is ever written"),
        },
    ];
    if !matches!(family, Family::Turing) {
        v.push(BootReg {
            off: NV_USABLE_FB_SIZE_IN_MB,
            value: u32::try_from(f.fb_mb).unwrap_or(u32::MAX),
            name: "NV_USABLE_FB_SIZE_IN_MB",
            from: Provenance::Host("the store's reserved size (constraint 15)"),
        });
    }
    if matches!(family, Family::Turing | Family::Ampere) {
        if let Some(value) = local_memory_range_gp102(f.fb_mb) {
            v.push(BootReg {
                off: NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE_GP102,
                value,
                name: "NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE",
                from: Provenance::Host("the store's reserved size (constraint 15), kmemsysReadUsableFbSize_GP102 encoding"),
            });
        }
    }
    // ★ 2026-09-26 (`V3_FAMILY_PORT_BLACKWELL.md`): the FSP families' FIRST boot poll.
    // `kfspPrepareBootCommands_GH100` opens with `kfspWaitForSecureBoot_HAL` (`ogkm-580:
    // kern_fsp_gh100.c:1299`), which spins (4–5 s, then "FSP secure boot partition timed out") until
    // `NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE` (the whole register, `dev_therm_addendum.h`) reads
    // `_STATUS_SUCCESS` = `0xFF`. FSP writes it "after completion of boot out of chip reset"; we are
    // the FSP and there is no earlier instant, so it is a static boot register. The OFFSET is per die
    // group, and the host's architecture draws the line: GB20x (`arch 0x1B0`) binds
    // `kfspWaitForSecureBoot_GB202` over `blackwell/gb202/dev_therm.h:27` = `0xAD00BC`; GH100 and
    // GB10x bind `_GH100`/`_GB100` over `hopper/gh100` / `blackwell/gb100` `dev_therm.h:26-27` =
    // `0x200BC`. ⊘ The model's `on_read` answer for it was never published (only `GspReg`s are).
    if matches!(family, Family::Hopper | Family::Blackwell) {
        v.push(BootReg {
            off: therm_i2cs_scratch(f.architecture),
            value: FSP_BOOT_COMPLETE_SUCCESS,
            name: "NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE",
            from: Provenance::Ogkm("kfspWaitForSecureBoot_{GH100,GB100,GB202}: FSP boot complete = 0xFF"),
        });
    }
    // ★ 2026-09-26 (`V3_FAMILY_PORT_BLACKWELL.md` §4): in a VM RM takes the PASSTHROUGH branch of
    // `gpuReadBusConfigCycle_GM107` (`bIsPassthru`, `ogkm-580: gpu.c:4745-4769`; `kern_gpu_gm107.c:
    // 103-106`), and on GH100/GB20x `gpuReadPassThruConfigReg_GH100` reads the config space's BAR0
    // MIRROR: `DEVICE_BASE(NV_EP_PCFGM)` = `0x92000` (`hopper/gh100/dev_xtl_ep_pri.h:26`) + the
    // offset (`kern_gpu_gh100.c:99-109`, bound for GH100 + GB20x, `g_gpu_nvoc.c:1197-1207`). So the
    // link capabilities `config_words` presents at `0x6C` are ALSO served at `0x9206C` — the XVE
    // mirror GA10x reads at `0x88084`, one family later. `[measured bws2]` without it UVM still saw
    // "Unknown PCIe speed". ⊘ GB10x binds `_GB100` = a real config cycle; nothing to mirror.
    if matches!(family, Family::Hopper | Family::Blackwell) && f.architecture != crate::arch::GB100 {
        v.push(BootReg {
            off: NV_EP_PCFGM + u64::from(NV_EP_PCFG_GPU_LINK_CAPABILITIES),
            value: f.pcie_link_caps,
            name: "NV_EP_PCFGM + NV_EP_PCFG_GPU_LINK_CAPABILITIES",
            from: Provenance::Host("the host's PCIe link capability"),
        });
    }
    // ★ Ada only (`kgspExecuteScrubberIfNeeded_AD102`, `ogkm-580: kernel_gsp_ad102.c`; the image is
    // ALWAYS allocated on Ada — `kernel_gsp.c:3734` "WAR for Bug 5016200"). Before the booter runs,
    // RM reads SCRUBBER_HANDOFF and, below DONE, resets SEC2 and runs a scrubber HS ucode on it —
    // a falcon run our GSP FSM would read as the booter. ⇒ We are the GSP and the store is ours,
    // so the top of FB is "already scrubbed": advertise DONE, and RM skips it by its own branch.
    if matches!(family, Family::Ada) {
        v.push(BootReg {
            off: NV_PGC6_BSI_VPR_SECURE_SCRATCH_15,
            value: SCRUBBER_HANDOFF_DONE,
            name: "NV_PGC6_BSI_VPR_SECURE_SCRATCH_15",
            from: Provenance::Advertised("kgspExecuteScrubberIfNeeded_AD102 skips the SEC2 scrubber when HANDOFF >= DONE"),
        });
    }
    v
}

/// `DEVICE_BASE(NV_EP_PCFGM)` — the BAR0 mirror of PCI config space on GH100/GB20x
/// (`ogkm-580: hopper/gh100/dev_xtl_ep_pri.h:26`, `0x92FFF:0x92000`).
const NV_EP_PCFGM: u64 = 0x0009_2000;
/// `NV_EP_PCFG_GPU_LINK_CAPABILITIES` (`hopper/gh100/dev_xtl_ep_pcfg_gpu.h:73`).
const NV_EP_PCFG_GPU_LINK_CAPABILITIES: u16 = 0x6C;

/// ★ 2026-09-26 (`V3_FAMILY_PORT_BLACKWELL.md` §4) — one dword of the device's **PCI configuration
/// space** the guest driver reads with a real config cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigWord {
    /// Config-space offset (dword-aligned).
    pub off: u16,
    /// The value, read-only to the guest.
    pub value: u32,
    /// The register's name.
    pub name: &'static str,
    /// Where the value comes from.
    pub from: Provenance,
}

/// ★ The config-space words the guest reads by **config cycle** on this family — the same PCIe
/// facts [`boot_regs`] serves through the BAR0 `NV_XVE` mirror on Turing … Ada.
///
/// `[measured GB203 bws1]` UVM_REGISTER_GPU failed `0x40` after *"calculatePCIELinkRateMBps:
/// Unknown PCIe speed"*: from Hopper on, `gpuReadBusConfigReg` binds `_GH100` →
/// `gpuReadBusConfigCycle_HAL`, an OS config-space read (`ogkm-580: kern_gpu_gh100.c:79-87`,
/// `g_gpu_nvoc.c:1143-1155`), NOT the BAR0 mirror — and our conventional-PCI device had nothing
/// there. `kbifGetGpuLinkCapabilities_IMPL` (`kernel_bif.c:879-902`) reads the address
/// `kbifGetBusOptionsAddr_HAL` names, which is per die group:
/// - GH100 + GB20x: `_GH100` → `NV_EP_PCFG_GPU_LINK_CAPABILITIES` = `0x6C`
///   (`hopper/gh100/dev_xtl_ep_pcfg_gpu.h:73`; GB20x binds it, `g_kernel_bif_nvoc.c:965-982`);
/// - GB10x: `_GB100` → `NV_PF0_LINK_CAPABILITIES` = `0x4C` (`blackwell/gb100/dev_pcfg_pf0.h:126`).
///
/// The value is the host's own link word, in the PCIe Link Capabilities layout the XVE mirror
/// already uses (`pcie_link_caps`). Empty for Turing … Ada: their reads go through BAR0.
#[must_use]
pub fn config_words(family: Family, f: &Bar0Facts) -> Vec<ConfigWord> {
    let off = match family {
        Family::Turing | Family::Ampere | Family::Ada => return Vec::new(),
        Family::Blackwell if f.architecture == crate::arch::GB100 => 0x4C,
        Family::Hopper | Family::Blackwell => NV_EP_PCFG_GPU_LINK_CAPABILITIES,
    };
    vec![ConfigWord {
        off,
        value: f.pcie_link_caps,
        name: "PCIe LINK_CAPABILITIES (config cycle)",
        from: Provenance::Host("the host's PCIe link capability"),
    }]
}

/// The host GPU's PCI identity — what the guest's driver binds on (read from the host's own config
/// space; never a table row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciIdentity {
    /// Vendor id (`0x10DE`).
    pub vendor: u16,
    /// Device id.
    pub device: u16,
    /// Class code, low byte first.
    pub class: [u8; 3],
}

/// ★ The VBIOS profile for THIS host: its PCI identity and its own VBIOS version
/// (`BIOS_GET_INFO_V2` `REVISION`/`OEM_REVISION`, `0x20800810`, NON_PRIVILEGED — asked at realize,
/// `kf_rm::HostFacts::vbios_version`), plus the FWSEC geometry kf-abi documents as GENERATED to
/// satisfy the driver's inequalities ([`kf_abi::vbios::GENERATED_FWSEC`]) — so it is family-level
/// for every falcon-boot family, never a per-die row keyed by device id.
///
/// ⊘ Was `VBIOS_PROFILES.first()`: the GA106 row's version `0x9418_0000` on every die
/// (2026-09-26, `V3_FAMILY_PORT_ADA.md` §2).
///
/// ★ 2026-09-26 (`V3_FAMILY_PORT_BLACKWELL.md`): **Hopper/Blackwell read no VBIOS image.** Their
/// `kgspExtractVbiosFromRom` binds `_395e98` = `NV_ERR_NOT_SUPPORTED` (every chip outside the
/// TU10x…AD10x mask, `ogkm-580: generated/g_kernel_gsp_nvoc.c:1283-1301`, `g_kernel_gsp_nvoc.h:1803`),
/// which `kgspPrepareForBootstrap` treats as *"not supported"* ⇒ no FWSEC parse
/// (`kernel_gsp.c:3990-4015`); FSP runs FWSEC itself. So the ROM carries only what the PCI layer
/// reads (identity + version) and the FWSEC geometry in it is inert — the same image, never a
/// per-family variant. ⊘ Was a `RowUnbuilt` refusal, which stopped every FSP-family realize.
///
/// # Errors
/// None today; kept fallible for a family whose ROM would need content we cannot derive.
pub fn vbios_profile(
    family: Family,
    id: PciIdentity,
    version: (u32, u8),
) -> Result<kf_abi::vbios::VbiosProfile, crate::RowUnbuilt> {
    let _ = family;
    Ok(kf_abi::vbios::VbiosProfile {
        name: "derived (host PCI identity + host VBIOS version + generated FWSEC geometry)",
        pci_vendor_id: id.vendor,
        pci_device_id: id.device,
        pci_class_code: id.class,
        vbios_version: version.0,
        vbios_oem_version: version.1,
        fwsec: kf_abi::vbios::GENERATED_FWSEC,
    })
}

/// The framebuffer layout this device's (emulated) GSP declares in `GspStaticConfigInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FbLayout {
    /// Bytes of framebuffer (the store).
    pub fb_length: u64,
    /// The regions: the usable heap, then the firmware carve-out at the top.
    pub regions: Vec<kf_abi::gspstaticinfo::FbRegion>,
    /// Where OUR BAR1 page directory lives (v3: our roots are declared, never adopted).
    pub bar1_pde_base: u64,
    /// ★ P4 (w826): where OUR BAR2 root page directory lives. The guest publishes its own
    /// `PDE3[0]` into entry 0 of this page (`UPDATE_BAR_PDE`, fn 70) and from then on names
    /// THIS page in every BAR2 `MMU_INVALIDATE` (`kbusPatchBar2Pdb_GSPCLIENT`,
    /// `ogkm-580: kern_bus.c:826-878`). Declared, never adopted — `V3_P4_PORT_MAP.md` Q4.
    pub bar2_pde_base: u64,
}

/// The contiguous carve-out at the top of FB the GSP keeps for itself — read off a real RTX 3060's
/// posted `GspStaticConfigInfo` (`traces/mode2_c_reference/cap1b_coldboot_hermetic_d6` record 141977:
/// regions 2-4, contiguous, `reserved == size`, the top `0x1042_0000` bytes). ★ In v3 WE are the
/// GSP, so this is our declared layout; it deliberately over-reserves beyond WPR2 + FRTS + the VGA
/// workspace (over-reserving costs heap; under-reserving hands the guest its firmware's memory).
pub const FW_CARVE_OUT_BYTES: u64 = 0x1042_0000;
/// How far above the carve-out base the same GSP placed the BAR1 page directory
/// (`0x2_F1CA_C000 - (12 GiB - FW_CARVE_OUT_BYTES)`) — a LAYOUT offset, preserved at any size.
const BAR1_PDE_ABOVE_CARVE_OUT: u64 = 0x20C_C000;
/// How far above the carve-out base the same GSP placed the BAR2 root page directory
/// (`0x2_F339_2000 - (12 GiB - FW_CARVE_OUT_BYTES)`, the value `gspstaticinfo::BAR2_PDE_BASE_OFF`
/// carries in that capture) — a LAYOUT offset, preserved at any size, inside the carve-out.
const BAR2_PDE_ABOVE_CARVE_OUT: u64 = 0x37B_2000;
/// The bytes of each root page we declare (one 4 KiB page: a VER2 `PDE3` root is 32 bytes and a
/// VER3 `PD4` root 16, so one page holds either family's root with its unused tail zero).
pub const ROOT_PAGE_BYTES: u64 = 0x1000;

/// ★ The layout for a store of `fb_length` bytes. ⊘ `None` if the store cannot hold the carve-out
/// (the VM must not start on a framebuffer smaller than its own firmware reservation).
#[must_use]
pub fn fb_layout(fb_length: u64) -> Option<FbLayout> {
    let carve = fb_length.checked_sub(FW_CARVE_OUT_BYTES)?;
    if carve == 0 {
        return None;
    }
    let regions = vec![
        kf_abi::gspstaticinfo::FbRegion {
            base: 0,
            limit: carve - 1,
            reserved: 0,
            performance: 6,
            support_compressed: true,
            support_iso: true,
            protected: false,
        },
        kf_abi::gspstaticinfo::FbRegion {
            base: carve,
            limit: fb_length - 1,
            reserved: FW_CARVE_OUT_BYTES,
            performance: 0,
            support_compressed: false,
            support_iso: false,
            protected: false,
        },
    ];
    Some(FbLayout {
        fb_length,
        regions,
        bar1_pde_base: carve + BAR1_PDE_ABOVE_CARVE_OUT,
        bar2_pde_base: carve + BAR2_PDE_ABOVE_CARVE_OUT,
    })
}

/// ★ Every BAR0 boot register and config word above, at the offset the ogkm-580 header of each
/// die group gives it (`kf_chip::hwref`, `docs/design/V3_HW_BOUNDARY_INVENTORY.md`).
#[cfg(test)]
mod hwref_check {
    use super::*;
    use crate::hwref::expect::{base, range, val};
    use crate::hwref::{DieGroup, table};

    fn facts(g: DieGroup) -> Bar0Facts {
        let (architecture, implementation) = match g {
            DieGroup::Tu10x => (crate::arch::TU100, 2),
            DieGroup::Ga100 => (crate::arch::GA100, crate::arch::IMPL_GA100),
            DieGroup::Ga10x => (crate::arch::GA100, 6),
            DieGroup::Ad10x => (crate::arch::AD100, 6),
            DieGroup::Gh100 => (crate::arch::GH100, 0),
            DieGroup::Gb10x => (crate::arch::GB100, 0),
            DieGroup::Gb20x => (crate::arch::GB200, 3),
        };
        Bar0Facts { architecture, implementation, revision: 0xA1, fb_mb: 8192, pcie_link_caps: 0x0040_4103 }
    }

    /// Rows served to a die group whose own code never reads them — an unread shadow word, named.
    const UNREAD: &[(DieGroup, &str, &str)] = &[
        (DieGroup::Ga100, "NV_USABLE_FB_SIZE_IN_MB", "GA100 binds kmemsysReadUsableFbSize_GP102 (g_kern_mem_sys_nvoc.c:349-352)"),
    ];

    #[test]
    fn every_boot_register_sits_at_its_die_groups_header_offset() {
        for g in DieGroup::ALL {
            for r in boot_regs(g.family(), &facts(g)) {
                let want = match r.name {
                    "NV_XVE_LINK_CAPABILITIES" => {
                        Some(base(g, "NV_PCFG") + val(g, "NV_XVE_LINK_CAPABILITIES"))
                    }
                    "NV_VIRTUAL_FUNCTION_PRIV_ACCESS_COUNTER_NOTIFY_BUFFER_SIZE" => Some(
                        base(g, "NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET")
                            + val(g, "NV_VIRTUAL_FUNCTION_PRIV_ACCESS_COUNTER_NOTIFY_BUFFER_SIZE"),
                    ),
                    "NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE" => Some(val(g, "NV_THERM_I2CS_SCRATCH")),
                    "NV_EP_PCFGM + NV_EP_PCFG_GPU_LINK_CAPABILITIES" => {
                        Some(base(g, "NV_EP_PCFGM") + val(g, "NV_EP_PCFG_GPU_LINK_CAPABILITIES"))
                    }
                    name => table().value(g, name).ok(),
                };
                match want {
                    Some(off) => assert_eq!(r.off, off, "{g:?} {}", r.name),
                    None => assert!(
                        UNREAD.iter().any(|(ug, n, _)| *ug == g && *n == r.name),
                        "{g:?} {}: served at {:#x}, and no header of the die group's lineage defines it",
                        r.name,
                        r.off
                    ),
                }
            }
        }
    }

    #[test]
    fn every_boot_register_encoding_is_the_header_field() {
        // PMC_BOOT_0 / _42 field positions (`nv_ref.h`, every lineage's root).
        for g in DieGroup::ALL {
            assert_eq!(range(g, "NV_PMC_BOOT_0_ARCHITECTURE_0"), (28, 24));
            assert_eq!(range(g, "NV_PMC_BOOT_0_IMPLEMENTATION"), (23, 20));
            assert_eq!((range(g, "NV_PMC_BOOT_0_MAJOR_REVISION"), range(g, "NV_PMC_BOOT_0_MINOR_REVISION")), ((7, 4), (3, 0)));
            assert_eq!(range(g, "NV_PMC_BOOT_42_ARCHITECTURE"), (29, 24));
            assert_eq!(range(g, "NV_PMC_BOOT_42_IMPLEMENTATION"), (23, 20));
            assert_eq!(range(g, "NV_PMC_BOOT_42_MAJOR_REVISION"), (19, 16));
            assert_eq!(range(g, "NV_PMC_BOOT_42_MINOR_REVISION"), (15, 12));
            assert_eq!(val(g, "NV_PMC_BOOT_1_VGPU_REAL"), 0, "BOOT_1 = 0 advertises VGPU = REAL");
        }
        // ⚠ `pmc_boot_0` writes the architecture's low five bits at 28:24 only; `NV_PMC_BOOT_0_
        // ARCHITECTURE_1` (8:8) holds a sixth. Every architecture ogkm-580 names fits in five.
        assert_eq!(range(DieGroup::Gb20x, "NV_PMC_BOOT_0_ARCHITECTURE_1"), (8, 8));
        assert!(val(DieGroup::Gb20x, "NV_PMC_BOOT_0_ARCHITECTURE_GB200") <= 0x1F);
        // LOCAL_MEMORY_RANGE (TU10x, GA100: `_GP102`): scale 3:0, mag 9:4.
        for g in [DieGroup::Tu10x, DieGroup::Ga100] {
            assert_eq!(range(g, "NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE_LOWER_SCALE"), (3, 0));
            assert_eq!(range(g, "NV_PFB_PRI_MMU_LOCAL_MEMORY_RANGE_LOWER_MAG"), (9, 4));
        }
        // Ada's scrubber handoff: 31:29, DONE = 3.
        let g = DieGroup::Ad10x;
        assert_eq!(range(g, "NV_PGC6_BSI_VPR_SECURE_SCRATCH_15_SCRUBBER_HANDOFF"), (31, 29));
        assert_eq!(u64::from(SCRUBBER_HANDOFF_DONE), val(g, "NV_PGC6_BSI_VPR_SECURE_SCRATCH_15_SCRUBBER_HANDOFF_DONE") << 29);
        for g in [DieGroup::Gh100, DieGroup::Gb10x, DieGroup::Gb20x] {
            assert_eq!(u64::from(FSP_BOOT_COMPLETE_SUCCESS), val(g, "NV_THERM_I2CS_SCRATCH_FSP_BOOT_COMPLETE_STATUS_SUCCESS"));
        }
    }

    #[test]
    fn every_config_word_sits_at_its_die_groups_config_offset() {
        for g in DieGroup::ALL {
            let words = config_words(g.family(), &facts(g));
            let want = match g {
                DieGroup::Tu10x | DieGroup::Ga100 | DieGroup::Ga10x | DieGroup::Ad10x => None,
                DieGroup::Gb10x => Some(val(g, "NV_PF0_LINK_CAPABILITIES")),
                DieGroup::Gh100 | DieGroup::Gb20x => Some(val(g, "NV_EP_PCFG_GPU_LINK_CAPABILITIES")),
            };
            assert_eq!(words.first().map(|w| u64::from(w.off)), want, "{g:?}");
        }
    }
}
