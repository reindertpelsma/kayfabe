//! ★ The display engine a guest of this chip expects — per die GROUP, read from ogkm's own
//! generated rules, never from a board (`docs/design/V3_DISPLAY.md` §4.1).
//!
//! A GSP-client guest decides "this GPU has a display" from its chip id (the HAL binding of
//! `gpuFuseSupportsDisplay`, `ogkm-580: src/nvidia/generated/g_gpu_nvoc.c:1820-1839`), and then
//! selects its kernel display HAL by the IP version physical RM reports
//! (`NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_IP_VERSION`). The valid (chip, IP version) pairs are
//! ogkm's NVOC halspec rules (`generated/g_chips2halspec_nvoc.h:139-155`); the classes each chip
//! lists are `generated/g_gpu_class_list.c`. Both are transcribed here per die group and held to
//! ogkm by `tests/display_rows.rs` (which compiles nothing — it reads the two generated files).
//!
//! ⚠ The kernel display HAL is `v03_00` with per-version deltas (`generated/g_kern_disp_nvoc.c`
//! binds `kdisp*_v03_00` for most entries and `_v04_00/_v04_01/_v04_02/_v05_01` for a few), so the
//! register vocabulary the guest touches is `published/disp/v03_00/dev_disp.h` plus the delta
//! headers — derived from those bindings, not from one directory name per row.
//!
//! ⊘ The HEAD and WINDOW counts are not the host board's: the monitor is kayfabe's, so the
//! topology is a design choice — the family's real maximum (4 heads, 2 windows each), which keeps
//! NVKMS's window-to-head expectations those of the real part.
//!
//! Chips that return `None` have **no display engine on bare metal either** (GA100, GH100,
//! GB100/GB102/GB110/GB112 — `gpuFuseSupportsDisplay_3dd2c9`, hard-wired false) or are integrated
//! parts kayfabe refuses at realize. For those the VM display is a separate adapter
//! (`V3_DISPLAY.md` §2.2).

use crate::arch;

/// The display classes one chip lists (`g_gpu_class_list.c`, `ENG_KERNEL_DISPLAY` rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayClasses {
    /// `NV04_DISPLAY_COMMON` — the common display object.
    pub common: u32,
    /// The display object (`NVC670_DISPLAY` …) — selects the NVKMS HAL (`nvkms-hal.c`).
    pub display: u32,
    /// The core channel (`…7D_CORE_CHANNEL_DMA`).
    pub core: u32,
    /// The window channel (`…7E_WINDOW_CHANNEL_DMA`).
    pub window: u32,
    /// The window-immediate channel (`…7B_WINDOW_IMM_CHANNEL_DMA`).
    pub window_imm: u32,
    /// The cursor channel (`…7A_CURSOR_IMM_CHANNEL_PIO`).
    pub cursor: u32,
    /// The capabilities object (`…73_DISP_CAPABILITIES`) — a mapping of the caps registers.
    pub caps: u32,
    /// The SF user object (`…71_DISP_SF_USER`).
    pub sf_user: u32,
    /// `NVC372_DISPLAY_SW` — the software display object (IMP, `IS_MODE_POSSIBLE`).
    pub disp_sw: u32,
    /// `NVC77F_ANY_CHANNEL_DMA`, when the chip lists it (GA10x and later).
    pub any_channel: Option<u32>,
}

/// ★ One die group's display row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayRow {
    /// The group's chips, for messages.
    pub chips: &'static str,
    /// `GET_IP_VERSION`'s answer: the NVOC `DISPvXXYY` value (`ipver & 0xFFFF0000`).
    pub ip_version: u32,
    /// The classes.
    pub classes: DisplayClasses,
    /// Heads the virtual display advertises.
    pub heads: u32,
    /// Windows the virtual display advertises (two per head).
    pub windows: u32,
}

const COMMON: u32 = 0x0073;
const DISP_SW: u32 = 0xC372;

/// TU102/TU104/TU106/TU116/TU117 — `DISPv0400`, the C5 classes (`nvEvoC5`).
pub const TURING: DisplayRow = DisplayRow {
    chips: "TU102 TU104 TU106 TU116 TU117",
    ip_version: 0x0400_0000,
    classes: DisplayClasses {
        common: COMMON,
        display: 0xC570,
        core: 0xC57D,
        window: 0xC57E,
        window_imm: 0xC57B,
        cursor: 0xC57A,
        caps: 0xC573,
        sf_user: 0xC371,
        disp_sw: DISP_SW,
        any_channel: None,
    },
    heads: 4,
    windows: 8,
};

/// GA102/GA103/GA104/GA106/GA107 — `DISPv0401`, the C6 classes (`nvEvoC6`). A real GA106 answers
/// `GET_IP_VERSION` with exactly this value (`kf-abi` `oracle.rs`, measured 2026-08-01).
pub const AMPERE: DisplayRow = DisplayRow {
    chips: "GA102 GA103 GA104 GA106 GA107",
    ip_version: 0x0401_0000,
    classes: DisplayClasses {
        common: COMMON,
        display: 0xC670,
        core: 0xC67D,
        window: 0xC67E,
        window_imm: 0xC67B,
        cursor: 0xC67A,
        caps: 0xC673,
        sf_user: 0xC671,
        disp_sw: DISP_SW,
        any_channel: Some(0xC77F),
    },
    heads: 4,
    windows: 8,
};

/// AD102/AD103/AD104/AD106/AD107 — `DISPv0404`: the C7 core with the C6 window classes (`nvEvoC6`).
pub const ADA: DisplayRow = DisplayRow {
    chips: "AD102 AD103 AD104 AD106 AD107",
    ip_version: 0x0404_0000,
    classes: DisplayClasses {
        common: COMMON,
        display: 0xC770,
        core: 0xC77D,
        window: 0xC67E,
        window_imm: 0xC67B,
        cursor: 0xC67A,
        caps: 0xC773,
        sf_user: 0xC771,
        disp_sw: DISP_SW,
        any_channel: Some(0xC77F),
    },
    heads: 4,
    windows: 8,
};

/// GB202/GB203/GB205/GB206/GB207 — `DISPv0502`, the CA classes (`nvEvoCA`).
pub const BLACKWELL_GB20X: DisplayRow = DisplayRow {
    chips: "GB202 GB203 GB205 GB206 GB207",
    ip_version: 0x0502_0000,
    classes: DisplayClasses {
        common: COMMON,
        display: 0xCA70,
        core: 0xCA7D,
        window: 0xCA7E,
        window_imm: 0xCA7B,
        cursor: 0xCA7A,
        caps: 0xCA73,
        sf_user: 0xCA71,
        disp_sw: DISP_SW,
        any_channel: Some(0xC77F),
    },
    heads: 4,
    windows: 8,
};

/// ★ The display row for a chip, or `None` when the chip has no display engine on bare metal
/// (the guest driver hard-wires that) or is not a discrete part kayfabe serves.
#[must_use]
pub fn display_for(architecture: u32, implementation: u32) -> Option<&'static DisplayRow> {
    match (architecture, implementation) {
        (arch::TU100, 0x2 | 0x4 | 0x6 | 0x7 | 0x8) => Some(&TURING),
        (arch::GA100, 0x2 | 0x3 | 0x4 | 0x6 | 0x7) => Some(&AMPERE),
        (arch::AD100, 0x2 | 0x3 | 0x4 | 0x6 | 0x7) => Some(&ADA),
        (arch::GB200, 0x2 | 0x3 | 0x5 | 0x6 | 0x7) => Some(&BLACKWELL_GB20X),
        _ => None,
    }
}

/// Every display row, for the tests that hold them to ogkm.
pub const ALL: [&DisplayRow; 4] = [&TURING, &AMPERE, &ADA, &BLACKWELL_GB20X];

#[cfg(test)]
mod tests {
    use super::*;

    /// Datacenter parts have no display engine; the four consumer groups do, with the IP version
    /// and display class NVKMS's HAL table pairs (`nvkms-hal.c`: C5 Turing, C6 Ampere, C7 core +
    /// C6 window Ada, CA GB20x).
    #[test]
    fn datacenter_chips_are_displayless_and_consumer_groups_map_to_their_hal() {
        assert!(display_for(arch::GA100, 0x0).is_none(), "GA100");
        assert!(display_for(arch::GH100, 0x0).is_none(), "GH100");
        assert!(display_for(arch::GB100, 0x0).is_none(), "GB100");
        assert!(display_for(arch::GB100, 0x2).is_none(), "GB102");
        assert_eq!(display_for(arch::GA100, 0x6).map(|r| r.ip_version), Some(0x0401_0000));
        assert_eq!(display_for(arch::GB200, 0x3).map(|r| r.classes.display), Some(0xCA70));
        assert_eq!(display_for(arch::AD100, 0x4).map(|r| (r.classes.core, r.classes.window)), Some((0xC77D, 0xC67E)));
        for r in ALL {
            assert_eq!(r.windows, 2 * r.heads, "{}: two windows per head", r.chips);
            assert_eq!(r.ip_version & 0xFFFF, 0, "{}: a halspec value is ipver & 0xFFFF0000", r.chips);
        }
    }
}
