// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ **The register a guest RM writes to give BAR1 back to PHYSICAL mode** — per die group, every
//! offset and field from the generated hwref table (`docs/design/V3_DISPLAY.md` §4.11.13).
//!
//! At a non-preserving unload RM tears BAR1 down and then returns it to physical mode with the
//! vidmem target, *"so we don't accidentally corrupt sysmem"*: `kbusStatePreUnload_GM107` →
//! `kbusDestroyBar1_HAL` + `kbusTeardownMailbox_HAL`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:769-787`). The mailbox
//! teardown is a read-modify-write of one register, and which one is a HAL choice
//! (`ogkm-580: src/nvidia/generated/g_kern_bus_nvoc.c:1825-1834`):
//!
//! | HAL | die groups | register | `MODE` | source |
//! |---|---|---|---|---|
//! | `kbusTeardownMailbox_GM107` | TU10x, GA100, GA10x, AD10x | `NV_PBUS_BAR1_BLOCK` | `31:31` | `kern_bus_gm107.c:746-765` |
//! | `kbusTeardownMailbox_GH100` | GH100, GB10x, GB20x | `NV_VIRTUAL_FUNCTION_PRIV_FUNC_BAR1_BLOCK_LOW_ADDR` through `GPU_VREG_WR32` | `9:9` | `arch/hopper/kern_bus_gh100.c:78-101` |
//!
//! `GPU_VREG_WR32` adds `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` outside SR-IOV
//! (`gpuGetVirtRegPhysOffset_TU102`, `src/kernel/gpu/arch/turing/kern_gpu_tu102.c:93-100`).
//!
//! ⊘ The HAL row is the one hand-maintained fact here (per die group, `g_kern_bus_nvoc.c`'s
//! ChipHal list); every number comes from the table, and an unresolved name is a named refusal.

use crate::hwref::{DieGroup, Resolved, table};

/// One die group's BAR1-mode register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bar1ModeReg {
    /// BAR0 offset of the register the guest writes.
    pub offset: u64,
    /// `MODE`'s bit range `(hi, lo)`.
    pub mode: (u64, u64),
    /// `MODE_PHYSICAL`'s value.
    pub physical: u64,
    /// The register's name (for the log).
    pub name: &'static str,
}

impl Bar1ModeReg {
    /// Whether a written `value` selects PHYSICAL mode.
    #[must_use]
    pub fn is_physical(&self, value: u64) -> bool {
        let (hi, lo) = self.mode;
        let width = hi.saturating_sub(lo) + 1;
        let mask = if width >= 64 {
            u64::MAX
        } else {
            (1 << width) - 1
        };
        (value >> lo) & mask == self.physical
    }
}

/// Which `kbusTeardownMailbox` body a die group binds (`g_kern_bus_nvoc.c:1825-1834`).
const fn uses_vf_register(g: DieGroup) -> bool {
    matches!(g, DieGroup::Gh100 | DieGroup::Gb10x | DieGroup::Gb20x)
}

/// ★ The register for die group `g`.
///
/// # Errors
/// A name the table does not decide for `g`, by name.
pub fn bar1_mode_reg(g: DieGroup) -> Result<Bar1ModeReg, String> {
    let t = table();
    let val = |n: &str| {
        t.value(g, n)
            .map_err(|r: Resolved| format!("{g:?} {n}: not a decided value in ogkm ({r:?})"))
    };
    let range = |n: &str| {
        t.range(g, n)
            .map_err(|r: Resolved| format!("{g:?} {n}: not a decided range in ogkm ({r:?})"))
    };
    if uses_vf_register(g) {
        const R: &str = "NV_VIRTUAL_FUNCTION_PRIV_FUNC_BAR1_BLOCK_LOW_ADDR";
        let (_, vf_base) = range("NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET")?;
        Ok(Bar1ModeReg {
            offset: vf_base + val(R)?,
            mode: range("NV_VIRTUAL_FUNCTION_PRIV_FUNC_BAR1_BLOCK_LOW_ADDR_MODE")?,
            physical: val("NV_VIRTUAL_FUNCTION_PRIV_FUNC_BAR1_BLOCK_LOW_ADDR_MODE_PHYSICAL")?,
            name: R,
        })
    } else {
        Ok(Bar1ModeReg {
            offset: val("NV_PBUS_BAR1_BLOCK")?,
            mode: range("NV_PBUS_BAR1_BLOCK_MODE")?,
            physical: val("NV_PBUS_BAR1_BLOCK_MODE_PHYSICAL")?,
            name: "NV_PBUS_BAR1_BLOCK",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ Every die group resolves — all families are first-class.
    #[test]
    fn every_die_group_has_its_register() {
        for g in DieGroup::ALL {
            let r = bar1_mode_reg(g).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(r.physical, 0, "{g:?}: MODE_PHYSICAL is 0 in every header");
        }
    }

    /// The two HAL bodies, as their headers state them (`dev_bus.h`: `0x1704`, `31:31`;
    /// `hopper/gh100/dev_vm.h`: `0xF60`, `9:9`, behind the VF window at `0xB80000`).
    #[test]
    fn the_two_hal_bodies_name_their_own_register() {
        let ga = bar1_mode_reg(DieGroup::Ga10x).unwrap();
        assert_eq!(
            (ga.offset, ga.mode, ga.name),
            (0x1704, (31, 31), "NV_PBUS_BAR1_BLOCK")
        );
        let gh = bar1_mode_reg(DieGroup::Gh100).unwrap();
        assert_eq!((gh.offset, gh.mode), (0xB8_0F60, (9, 9)));
        // kbusTeardownMailbox_GM107's write: MODE=PHYSICAL, TARGET=VID_MEM over a VIRTUAL value.
        assert!(ga.is_physical(0x0000_0614));
        assert!(!ga.is_physical(0x8000_0614));
        assert!(gh.is_physical(0x0000_0001));
        assert!(!gh.is_physical(0x0000_0200));
    }
}
