//! ★ v3-gfx — **GMMU PTE kinds, per family, and their uncompressed equivalents.**
//!
//! A PTE's KIND tells the engines how to interpret the page (block-linear generic, depth/stencil
//! layouts, compression). The guest's page tables carry the kinds its UMD chose; the host twin's
//! mapping must carry the same layout kind or the engine refuses (`[measured vgfx 2026-09-26]`:
//! a GL depth buffer mapped PITCH ⇒ host Xid 13 "3D-Z KIND Violation"). This device backs NO
//! comptags, so every compressible kind maps to its uncompressed twin (compression off:
//! bandwidth, not correctness — `V3_HEADLESS_GRAPHICS.md` §2.1).
//!
//! ★ Data per family, and today it is ONE table: the `NV_MMU_PTE_KIND_*` block is identical in
//! `turing/tu102/dev_mmu.h:97-112` (Turing, Ampere GA10x, Ada), `hopper/gh100/dev_mmu.h` and
//! `blackwell/gb202/dev_mmu.h` (ogkm-580). GA100 overrides only `…_DISABLE_PLC 0x09` and
//! `SMSKED_MESSAGE 0x0F` with the same values. A family whose table diverges gets its own row.

use crate::Family;

/// `NV_MMU_PTE_KIND_PITCH`.
pub const PTE_KIND_PITCH: u8 = 0x00;
/// `NV_MMU_PTE_KIND_GENERIC_MEMORY`.
pub const PTE_KIND_GENERIC: u8 = 0x06;

/// `(kind, uncompressed kind)` for every layout kind the tree defines; INVALID `0x07` and
/// SMSKED_MESSAGE `0x0F` (not a surface layout) are absent on purpose.
const UNCOMPRESSED_TU102: &[(u8, u8)] = &[
    (0x00, 0x00), // PITCH
    (0x01, 0x01), // Z16
    (0x02, 0x02), // S8
    (0x03, 0x03), // S8Z24
    (0x04, 0x04), // ZF32_X24S8
    (0x05, 0x05), // Z24S8
    (0x06, 0x06), // GENERIC_MEMORY
    (0x08, 0x06), // GENERIC_MEMORY_COMPRESSIBLE
    (0x09, 0x06), // GENERIC_MEMORY_COMPRESSIBLE_DISABLE_PLC
    (0x0A, 0x02), // S8_COMPRESSIBLE_DISABLE_PLC
    (0x0B, 0x01), // Z16_COMPRESSIBLE_DISABLE_PLC
    (0x0C, 0x03), // S8Z24_COMPRESSIBLE_DISABLE_PLC
    (0x0D, 0x04), // ZF32_X24S8_COMPRESSIBLE_DISABLE_PLC
    (0x0E, 0x05), // Z24S8_COMPRESSIBLE_DISABLE_PLC
];

impl Family {
    /// This family's `(kind → uncompressed kind)` table.
    #[must_use]
    pub const fn pte_kinds(self) -> &'static [(u8, u8)] {
        match self {
            Family::Turing | Family::Ampere | Family::Ada | Family::Hopper | Family::Blackwell => UNCOMPRESSED_TU102,
        }
    }
}

/// The uncompressed equivalent of guest PTE `kind` — the same on every family today (see the
/// module docs); `None` for a kind the table does not define.
#[must_use]
pub fn uncompressed_pte_kind(kind: u8) -> Option<u8> {
    UNCOMPRESSED_TU102.iter().find(|(k, _)| *k == kind).map(|(_, u)| *u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compression_is_stripped_and_layout_is_kept() {
        assert_eq!(uncompressed_pte_kind(0x0E), Some(0x05), "Z24S8 compressible -> Z24S8");
        assert_eq!(uncompressed_pte_kind(0x08), Some(PTE_KIND_GENERIC));
        assert_eq!(uncompressed_pte_kind(0x05), Some(0x05));
        assert_eq!(uncompressed_pte_kind(0x07), None, "INVALID is never a mapping");
        assert_eq!(uncompressed_pte_kind(0x0F), None, "SMSKED_MESSAGE is not a surface");
        for (_, u) in UNCOMPRESSED_TU102 {
            assert!(*u <= 0x06, "an uncompressed kind is one of PITCH..GENERIC");
        }
        for f in [Family::Turing, Family::Ampere, Family::Ada, Family::Hopper, Family::Blackwell] {
            assert!(!f.pte_kinds().is_empty());
        }
    }
}
