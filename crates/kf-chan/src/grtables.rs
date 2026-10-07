// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ **The kernel-GR tier's method allowlists** (owner rulings 2026-10-07, `OWNER_RULINGS.md` §S,
//! items 2-4; `traces/windows_code43_walls_20261007/README.md`, run 32 onward).
//!
//! Windows' guest kernel puts graphics work on its kernel GR channel during StartDevice: run31
//! (c15c2628) logged `SET_OBJECT` of `KEPLER_INLINE_TO_MEMORY_B` (`0xA140`) and `FERMI_TWOD_A`
//! (`0x902D`), then 2D state methods. The T-mode rewriter ([`crate::tmode`]) runs such work on an
//! UNPRIVILEGED host channel only by RE-AUTHORING each method from this table:
//!
//! - one table per class, keyed by the exact class id (the id fixes the method layout);
//! - a row admits ONE method and names the field rule its argument must satisfy; the word emitted is
//!   rebuilt from the decoded fields, never the guest's word copied;
//! - the rows are exactly the methods a run has OBSERVED and the native oracle has validated on
//!   bare metal (ruling 3). A method no row admits is refused BY NAME and kills only that channel:
//!   privileged state, address registers and triggers under their class-header names
//!   ([`TWOD_REFUSED`]), everything else as "not in the allowlist".
//!
//! ⊘ **Written by hand from the class header, with each line cited** — like [`crate::ttables`]. The
//! source is `ogkm-580.65.06: src/common/sdk/nvidia/inc/class/cl902d.h` (the Windows guest's driver
//! branch; byte-identical in 580.159.04). OGKM's `cla140.h` names only the class id (line 27), so
//! the I2M table admits no method yet.
//!
//! ⊘ No row in this file carries an address. When a 2D surface or an I2M destination is admitted
//! later, its address register must go through the T-mode address perimeter
//! ([`crate::tspace_unsafe`]) like a CE operand, with a footprint bound — never a row here.

/// `FERMI_TWOD_A` (`cl902d.h:27`).
pub const FERMI_TWOD_A: u32 = 0x902D;
/// `KEPLER_INLINE_TO_MEMORY_B` (`cla140.h:27`).
pub const KEPLER_INLINE_TO_MEMORY_B: u32 = 0xA140;

/// A graphics class the GR tier knows a table for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GrClass {
    /// `FERMI_TWOD_A`.
    TwoD,
    /// `KEPLER_INLINE_TO_MEMORY_B`.
    InlineToMemory,
}

impl GrClass {
    /// Every class the tier has a table for.
    pub const ALL: [GrClass; 2] = [GrClass::TwoD, GrClass::InlineToMemory];

    /// The tier's class for `class`, or `None`.
    #[must_use]
    pub const fn of_class(class: u32) -> Option<GrClass> {
        match class {
            FERMI_TWOD_A => Some(GrClass::TwoD),
            KEPLER_INLINE_TO_MEMORY_B => Some(GrClass::InlineToMemory),
            _ => None,
        }
    }

    /// The class id.
    #[must_use]
    pub const fn id(self) -> u32 {
        match self {
            GrClass::TwoD => FERMI_TWOD_A,
            GrClass::InlineToMemory => KEPLER_INLINE_TO_MEMORY_B,
        }
    }

    /// The class-header name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            GrClass::TwoD => "FERMI_TWOD_A",
            GrClass::InlineToMemory => "KEPLER_INLINE_TO_MEMORY_B",
        }
    }
}

/// How an admitted method's argument is checked and re-authored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// `V` 0:0, `FALSE`/`TRUE`.
    Bool,
    /// `V` 2:0 or wider, one of the listed values.
    OneOf(&'static [u32]),
    /// `V` 31:0: the whole word is the one named field (a coordinate or a fixed-point scale).
    Word,
}

/// One admitted method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The method offset.
    pub method: u32,
    /// The class-header name.
    pub name: &'static str,
    /// The argument's rule.
    pub field: Field,
}

/// `NV902D_SET_OPERATION_V_*` (`cl902d.h:572-580`): `SRCCOPY_AND` 0 … `BLEND_PREMULT` 6.
const OPERATIONS: [u32; 7] = [0, 1, 2, 3, 4, 5, 6];
/// `NV902D_SET_PIXELS_FROM_CPU_COLOR_FORMAT_V_*` (`cl902d.h:815-832`).
const CPU_COLOR_FORMATS: [u32; 16] = [
    0xCF, 0xDF, 0xD5, 0xD1, 0xE6, 0xF9, 0xE8, 0xE9, 0xF8, 0xF3, 0xEE, 0xFF, 0xFB, 0xFC, 0xFD, 0xFE,
];

/// ★ `FERMI_TWOD_A`, admitted: exactly the address-free state methods Windows' kernel GR channel
/// sent in run31's first segment (decoded in the traces README), each from `cl902d.h` (line).
pub const TWOD_ALLOWED: [Row; 17] = [
    // :537-540
    row(0x0290, "SET_CLIP_ENABLE", Field::Bool),
    // :555-558
    row(0x029C, "SET_COLOR_KEY_ENABLE", Field::Bool),
    // :572-580
    row(0x02AC, "SET_OPERATION", Field::OneOf(&OPERATIONS)),
    // :815-832
    row(
        0x0804,
        "SET_PIXELS_FROM_CPU_COLOR_FORMAT",
        Field::OneOf(&CPU_COLOR_FORMATS),
    ),
    // :868-890
    row(0x0840, "SET_PIXELS_FROM_CPU_DX_DU_FRAC", Field::Word),
    row(0x0844, "SET_PIXELS_FROM_CPU_DX_DU_INT", Field::Word),
    row(0x0848, "SET_PIXELS_FROM_CPU_DY_DV_FRAC", Field::Word),
    row(0x084C, "SET_PIXELS_FROM_CPU_DY_DV_INT", Field::Word),
    row(0x0850, "SET_PIXELS_FROM_CPU_DST_X0_FRAC", Field::Word),
    row(0x0854, "SET_PIXELS_FROM_CPU_DST_X0_INT", Field::Word),
    row(0x0858, "SET_PIXELS_FROM_CPU_DST_Y0_FRAC", Field::Word),
    row(0x085C, "SET_PIXELS_FROM_CPU_DST_Y0_INT", Field::Word),
    // :935-938
    row(0x0888, "SET_PIXELS_FROM_MEMORY_SAFE_OVERLAP", Field::Bool),
    // :960-970
    row(0x08C0, "SET_PIXELS_FROM_MEMORY_DU_DX_FRAC", Field::Word),
    row(0x08C4, "SET_PIXELS_FROM_MEMORY_DU_DX_INT", Field::Word),
    row(0x08C8, "SET_PIXELS_FROM_MEMORY_DV_DY_FRAC", Field::Word),
    row(0x08CC, "SET_PIXELS_FROM_MEMORY_DV_DY_INT", Field::Word),
];

const fn row(method: u32, name: &'static str, field: Field) -> Row {
    Row {
        method,
        name,
        field,
    }
}

/// ★ `FERMI_TWOD_A` methods refused under their own names `(first, last, name)`: privileged or
/// firmware state, address registers no row authors, and the triggers and inline data whose
/// footprint the tier does not yet bound (`cl902d.h`, line of the first define).
pub const TWOD_REFUSED: [(u32, u32, &str); 16] = [
    (0x0104, 0x0108, "SET_NOTIFY_A/B (an address)"), // :238
    (0x010C, 0x010C, "NOTIFY"),                      // :244
    (0x0114, 0x0120, "LOAD_MME_* (macro engine programming)"), // :252
    (0x0124, 0x0124, "SET_MME_SHADOW_RAM_CONTROL"),  // :264
    (
        0x0130,
        0x0138,
        "SET_GLOBAL_RENDER_ENABLE_A/B/C (an address)",
    ), // :271
    (0x0140, 0x0140, "PM_TRIGGER"),                  // :288
    (0x0150, 0x0154, "SET_INSTRUMENTATION_METHOD_*"), // :291
    (0x01EC, 0x01EC, "SET_MME_SWITCH_STATE"),        // :297
    (0x0220, 0x0224, "SET_DST_OFFSET_UPPER/LOWER (an address)"), // :383
    (0x0250, 0x0254, "SET_SRC_OFFSET_UPPER/LOWER (an address)"), // :478
    (0x0264, 0x026C, "SET_RENDER_ENABLE_A/B/C (an address)"), // :499
    (0x0860, 0x0860, "PIXELS_FROM_CPU_DATA (inline data)"), // :892
    (
        0x08DC,
        0x08DC,
        "PIXELS_FROM_MEMORY_SRC_Y0_INT (the blit trigger)",
    ), // :981
    (0x08E0, 0x095C, "SET_FALCON00-31 (firmware methods)"), // :984
    (0x0DEC, 0x0DEC, "MME_DMA_WRITE_METHOD_BARRIER"), // :1080
    (
        0x3400,
        0x3FFC,
        "SET_MME_SHADOW_SCRATCH / CALL_MME_MACRO / CALL_MME_DATA",
    ), // :1083-1089
];

/// What the tier does with one `(class, method)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Admitted: re-author the argument by this row.
    Allowed(Row),
    /// Refused under this name.
    Refused(&'static str),
}

/// The disposition of `method` (at or above `0x100`) on a subchannel bound to `class`.
#[must_use]
pub fn gr_method(class: GrClass, method: u32) -> Disposition {
    match class {
        GrClass::TwoD => {
            if let Some(r) = TWOD_ALLOWED.iter().find(|r| r.method == method) {
                return Disposition::Allowed(*r);
            }
            if let Some(&(_, _, name)) = TWOD_REFUSED
                .iter()
                .find(|&&(lo, hi, _)| (lo..=hi).contains(&method))
            {
                return Disposition::Refused(name);
            }
            Disposition::Refused("not in the FERMI_TWOD_A allowlist")
        }
        GrClass::InlineToMemory => {
            Disposition::Refused("not in the KEPLER_INLINE_TO_MEMORY_B allowlist (none admitted)")
        }
    }
}

/// Re-author `v` by `r`'s field rule: the word rebuilt from the named field, or `Err(what)`.
///
/// # Errors
/// A word with a bit outside the field, or a value the field does not name.
pub fn reauthor(r: &Row, v: u32) -> Result<u32, &'static str> {
    match r.field {
        Field::Bool => {
            if v & !1 != 0 {
                return Err("a bit beyond V (0:0)");
            }
            Ok(v & 1)
        }
        Field::OneOf(vals) => vals
            .iter()
            .copied()
            .find(|&x| x == v)
            .ok_or("a value the class header does not name"),
        Field::Word => Ok(u32::from_le_bytes(v.to_le_bytes())),
    }
}

/// Is `class` a known engine class on any family (so a software-subchannel bind of it is NOT an
/// inert value)? Derived from kf-chip's generated per-family sets.
#[must_use]
pub fn is_known_class(class: u32) -> bool {
    kf_chip::Family::ALL
        .iter()
        .any(|f| kf_chip::classes::classes_for(*f).kind_of(class).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_the_run31_methods_are_admitted() {
        let run31 = [
            0x888, 0x290, 0x29c, 0x2ac, 0x804, 0x840, 0x844, 0x848, 0x84c, 0x850, 0x854, 0x858,
            0x85c, 0x8c0, 0x8c4, 0x8c8, 0x8cc,
        ];
        for m in run31 {
            assert!(
                matches!(gr_method(GrClass::TwoD, m), Disposition::Allowed(_)),
                "{m:#x}"
            );
        }
        let admitted = TWOD_ALLOWED.len();
        assert_eq!(admitted, run31.len());
        // Nothing else in the 2D method space is admitted.
        let others = (0x100..0x4000u32)
            .step_by(4)
            .filter(|m| !run31.contains(m))
            .filter(|&m| matches!(gr_method(GrClass::TwoD, m), Disposition::Allowed(_)))
            .count();
        assert_eq!(others, 0);
        for m in (0x100..0x4000u32).step_by(4) {
            assert!(matches!(
                gr_method(GrClass::InlineToMemory, m),
                Disposition::Refused(_)
            ));
        }
    }

    #[test]
    fn privileged_and_address_methods_are_refused_by_their_names() {
        for (m, frag) in [
            (0x118, "LOAD_MME"),
            (0x140, "PM_TRIGGER"),
            (0x224, "SET_DST_OFFSET"),
            (0x254, "SET_SRC_OFFSET"),
            (0x8dc, "blit trigger"),
            (0x860, "inline data"),
            (0x900, "FALCON"),
            (0x3808, "CALL_MME"),
        ] {
            match gr_method(GrClass::TwoD, m) {
                Disposition::Refused(n) => assert!(n.contains(frag), "{m:#x}: {n}"),
                Disposition::Allowed(r) => panic!("{m:#x} admitted as {}", r.name),
            }
        }
    }

    #[test]
    fn fields_are_reauthored_or_refused() {
        let clip = TWOD_ALLOWED[0];
        assert_eq!(reauthor(&clip, 1), Ok(1));
        assert!(reauthor(&clip, 3).is_err());
        let op = TWOD_ALLOWED[2];
        assert_eq!(reauthor(&op, 3), Ok(3));
        assert!(reauthor(&op, 7).is_err());
        let fmt = TWOD_ALLOWED[3];
        assert_eq!(reauthor(&fmt, 0xcf), Ok(0xcf));
        assert!(reauthor(&fmt, 0x1cf).is_err());
        assert!(reauthor(&fmt, 0xc0).is_err());
        let scale = TWOD_ALLOWED[4];
        assert_eq!(reauthor(&scale, 0xdead_beef), Ok(0xdead_beef));
    }

    #[test]
    fn the_inert_rule_knows_engine_classes() {
        assert!(!is_known_class(1)); // NV01_ROOT_NON_PRIV, Windows' sub-5 value
        assert!(is_known_class(0xc7b5));
        assert!(is_known_class(FERMI_TWOD_A));
        assert!(is_known_class(KEPLER_INLINE_TO_MEMORY_B));
    }
}
