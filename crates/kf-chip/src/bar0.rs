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

/// `NV_XVE_LINK_CAPABILITIES` for a link trained at `max_gen` ×16 — delegated to the ABI's own
/// encoder (`kf_abi::businfo::PcieLinkCaps::fully_trained`), which documents why the real part's
/// measured word is the wrong thing to copy.
#[must_use]
pub const fn pcie_link_caps(max_gen: kf_abi::businfo::PcieGen) -> u32 {
    kf_abi::businfo::PcieLinkCaps::fully_trained(max_gen).encode()
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
/// dev_gc6_island_addendum.h:33`) — published for the falcon-boot families only.
const NV_USABLE_FB_SIZE_IN_MB: u64 = 0x0011_83A4;
/// `NV_PGC6_BSI_VPR_SECURE_SCRATCH_15` = `NV_PGC6_BSI_SECURE_SCRATCH_15` (`ogkm-580:
/// published/ada/ad102/dev_gc6_island.h:27`, addendum `:27-29`): `SCRUBBER_HANDOFF` is `31:29`,
/// `_DONE` = 3.
const NV_PGC6_BSI_VPR_SECURE_SCRATCH_15: u64 = 0x0011_80FC;
/// `SCRUBBER_HANDOFF_DONE << 29`.
const SCRUBBER_HANDOFF_DONE: u32 = 3 << 29;
/// Access-counter notify buffer: two pages of 32-byte entries — advertised, never written (the old
/// tree's `resume_from_fault.md` §S2 ruling: migration heuristics simply never fire).
const ACCESS_COUNTER_ENTRIES_ADVERTISED: u32 = 2 * (4096 / 32);

/// ★ The boot registers for `family`, from `facts`. ⊘ `USABLE_FB_SIZE_IN_MB` is served only where
/// ogkm publishes it (Turing … Ada); on Hopper/Blackwell the framebuffer size reaches RM by another
/// path, and inventing the register there would be a guess.
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
    if matches!(family, Family::Turing | Family::Ampere | Family::Ada) {
        v.push(BootReg {
            off: NV_USABLE_FB_SIZE_IN_MB,
            value: u32::try_from(f.fb_mb).unwrap_or(u32::MAX),
            name: "NV_USABLE_FB_SIZE_IN_MB",
            from: Provenance::Host("the store's reserved size (constraint 15)"),
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

/// ★ The VBIOS profile for THIS host: its PCI identity, plus the FWSEC geometry kf-abi documents as
/// GENERATED to satisfy the driver's inequalities (not transcribed from any card) — so it is
/// family-level for every falcon-boot family, never a per-die row keyed by device id.
///
/// ⊘ Hopper/Blackwell boot through FSP and do not run FWSEC from the VBIOS; what their ROM image must
/// carry is not yet established from ogkm, so they are refused by name here rather than handed a
/// falcon family's image.
///
/// # Errors
/// A family whose ROM content is not yet derived.
pub fn vbios_profile(family: Family, id: PciIdentity) -> Result<kf_abi::vbios::VbiosProfile, crate::RowUnbuilt> {
    if matches!(family, Family::Hopper | Family::Blackwell) {
        return Err(crate::RowUnbuilt {
            family,
            what: "VBIOS image: FSP families do not run FWSEC; their ROM contents are not yet derived from ogkm",
        });
    }
    let generated = kf_abi::vbios::VBIOS_PROFILES
        .first()
        .ok_or(crate::RowUnbuilt { family, what: "kf-abi carries no generated FWSEC geometry" })?;
    Ok(kf_abi::vbios::VbiosProfile {
        name: "derived (host PCI identity + generated FWSEC geometry)",
        pci_vendor_id: id.vendor,
        pci_device_id: id.device,
        pci_class_code: id.class,
        ..*generated
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
