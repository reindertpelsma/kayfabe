//! ★★★ **The PRAMIN window — the BAR0 moving window's register, decoded, per family**
//! (`V3_P4_PORT_MAP.md` §2.4; `THE_CONSTRAINTS.md` §53.1; `THE_BAR0_DISPOSITION_MAP.md`).
//!
//! PRAMIN (`BAR0 0x700000`, 1 MiB) is disposition **A**: plain RAM under one memslot, no exit in
//! either direction. What it SHOWS is chosen by a *different* register — the window base — whose
//! write is a disposition-**B** trap: the vCPU stores the raw word in the shadow (read-modify-write
//! must see it back) and re-points the 1 MiB region onto the store at the new base.
//!
//! ⊘ Nothing here reads or writes the window's bytes. This file is pure: the decode, and the plan
//! of which 64 KiB store granule each 64 KiB slot of the window shows.
//!
//! ## The register, per family (ogkm-580.159.04, read, not remembered)
//!
//! | family | register | `BASE` | `TARGET` |
//! |---|---|---|---|
//! | Turing, Ampere, Ada | `NV_PBUS_BAR0_WINDOW` `0x1700` (`maxwell/gm107/dev_bus.h:43-50`) | 23:0 | 25:24 (`0` vidmem, `2` sys coherent, `3` sys non-coherent) |
//! | Hopper | `NV_XAL_EP_BAR0_WINDOW` `0x10fd40` (`hopper/gh100/pri_nv_xal_ep.h:25-27`) | 21:0 | none — *"`_BAR0_WINDOW_TARGET` field is removed. It's always VIDMEM"* (`kern_bus_gh100.c:207`) |
//! | Blackwell | `NV_XAL_EP_BAR0_WINDOW` `0x10fd40` (`blackwell/gb100`, `gb202`, `gb10b` `pri_nv_xal_ep.h`) | 22:0 (GB100) / 24:0 (GB10B) — decoded at the widest, 24:0 | none |
//!
//! `BASE_SHIFT` is 16 on all of them: the window origin is a 64 KiB-aligned framebuffer address.

use crate::trappolicy::PRAMIN_LEN;
use kf_chip::Family;

/// The window is served in slots of this size: the register's own granularity.
pub const GRANULE: u64 = 0x1_0000;
/// Slots in the 1 MiB window.
pub const SLOTS: usize = (PRAMIN_LEN / GRANULE) as usize;
/// `…_BAR0_WINDOW_BASE_SHIFT`.
pub const BASE_SHIFT: u32 = 16;

/// The window-base register of one family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowReg {
    /// BAR0 offset.
    pub offset: u64,
    /// `BASE` mask (already shifted to bit 0).
    pub base_mask: u32,
    /// Whether the word carries `TARGET` in 25:24.
    pub has_target: bool,
}

impl WindowReg {
    /// The register for `family`.
    #[must_use]
    pub const fn for_family(family: Family) -> WindowReg {
        match family {
            Family::Turing | Family::Ampere | Family::Ada => {
                WindowReg { offset: 0x1700, base_mask: 0x00FF_FFFF, has_target: true }
            }
            Family::Hopper => WindowReg { offset: 0x0010_FD40, base_mask: 0x003F_FFFF, has_target: false },
            Family::Blackwell => WindowReg { offset: 0x0010_FD40, base_mask: 0x01FF_FFFF, has_target: false },
        }
    }

    /// Decode a word the guest wrote.
    #[must_use]
    pub fn decode(self, raw: u32) -> Window {
        let target = if self.has_target {
            match (raw >> 24) & 0x3 {
                0 => Target::Vidmem,
                2 => Target::SysCoherent,
                3 => Target::SysNonCoherent,
                n => Target::Reserved(n),
            }
        } else {
            Target::Vidmem
        };
        Window { base: u64::from(raw & self.base_mask) << BASE_SHIFT, target }
    }
}

/// Which memory the window looks into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The framebuffer — our store.
    Vidmem,
    /// Guest system memory, coherent.
    SysCoherent,
    /// Guest system memory, non-coherent.
    SysNonCoherent,
    /// `1`: the header defines no meaning for it.
    Reserved(u32),
}

/// A decoded window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// The origin: a framebuffer (or guest-physical) address, 64 KiB-aligned.
    pub base: u64,
    /// Where `base` points.
    pub target: Target,
}

impl Window {
    /// ★ The address slot `i` (0..[`SLOTS`]) shows: `base + i * GRANULE`. `+`, never `|` — the
    /// window is a linear 1 MiB run from a 64 KiB-aligned origin (the old tree's `fb_addr`).
    /// `None` on overflow (a guest-chosen base near the top of `u64` must not wrap low).
    #[must_use]
    pub fn slot_addr(self, i: usize) -> Option<u64> {
        (i < SLOTS).then_some(())?;
        self.base.checked_add(i as u64 * GRANULE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[cap3 #159760-#159763]`: the guest writes `0x2efba` and then reads `0x70e000` — the
    /// `kbusVerifyBar2` word at framebuffer `0x2_EFBA_E000`.
    #[test]
    fn the_measured_verify_window_decodes_to_the_measured_address() {
        let w = WindowReg::for_family(Family::Ampere).decode(0x0002_EFBA);
        assert_eq!(w, Window { base: 0x2_EFBA_0000, target: Target::Vidmem });
        assert_eq!(w.slot_addr(0xE), Some(0x2_EFBA_0000 + 0xE_0000));
        assert_eq!(w.slot_addr(SLOTS), None);
    }

    #[test]
    fn target_is_decoded_on_falcon_families_and_absent_on_xal_families() {
        let a = WindowReg::for_family(Family::Turing);
        assert_eq!(a.decode(0x0200_0010).target, Target::SysCoherent);
        assert_eq!(a.decode(0x0300_0010).target, Target::SysNonCoherent);
        assert_eq!(a.decode(0x0100_0010).target, Target::Reserved(1));
        for f in [Family::Hopper, Family::Blackwell] {
            let r = WindowReg::for_family(f);
            assert_eq!(r.offset, 0x10_FD40);
            assert_eq!(r.decode(0x0300_0010).target, Target::Vidmem, "{f:?}: always vidmem");
        }
        assert_eq!(WindowReg::for_family(Family::Hopper).decode(0xFFFF_FFFF).base, 0x3F_FFFF << 16);
    }

    #[test]
    fn slots_tile_the_window() {
        assert_eq!(SLOTS as u64 * GRANULE, PRAMIN_LEN);
        let w = Window { base: u64::MAX & !0xFFFF, target: Target::Vidmem };
        assert_eq!(w.slot_addr(1), None, "no wrap into a low address");
    }
}
