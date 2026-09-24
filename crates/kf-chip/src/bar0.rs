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
    v
}
