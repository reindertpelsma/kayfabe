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
//!   bare metal (ruling 3): the 17 2D rows by `kf-gr-tier` at 01870988
//!   (`traces/windows_code43_walls_20261007/gr-tier-native-run32.log`), the 3D/compute notifier rows
//!   by the batch-2 oracle log named in that README. A method no row admits is refused BY NAME and kills only that channel:
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

// ⊘ Audit fix 2026-10-07 (BLOCKER 2): no class id is written here. Every id comes from kf-chip's
// generated per-family sets (`kf_chip::classes`): `twod`, `inline_to_memory` (Blackwell's is its
// own, 0xCD40), `threed`, `compute`. The 2D method table below is `cl902d.h`'s layout; a test pins
// that every family's generated 2D set is exactly the class that header defines.

/// A graphics class the GR tier knows a table for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GrClass {
    /// `FERMI_TWOD_A`.
    TwoD,
    /// `KEPLER_INLINE_TO_MEMORY_B`.
    InlineToMemory,
    /// ★ Batch 2 (run32): a family's 3D class (`ADA_A` 0xC997 on Ada), any family kf-chip lists.
    ThreeD,
    /// ★ Batch 2 (run32): a family's compute class (`ADA_COMPUTE_A` 0xC9C0 on Ada).
    Compute,
}

impl GrClass {
    /// The kinds a GR-tier ring holds a host object of, allocated at admission, each on its own.
    /// (Compute is the ring's own GR-context object.)
    pub const ALLOCATED: [GrClass; 3] = [GrClass::TwoD, GrClass::InlineToMemory, GrClass::ThreeD];

    /// The tier's class for `class` — its kind in ANY family's generated set — or `None`.
    #[must_use]
    pub fn of_class(class: u32) -> Option<GrClass> {
        kf_chip::Family::ALL.iter().find_map(|f| {
            match kf_chip::classes::classes_for(*f).kind_of(class)? {
                kf_chip::classes::Kind::TwoD => Some(GrClass::TwoD),
                kf_chip::classes::Kind::InlineToMemory => Some(GrClass::InlineToMemory),
                kf_chip::classes::Kind::ThreeD => Some(GrClass::ThreeD),
                kf_chip::classes::Kind::Compute => Some(GrClass::Compute),
                _ => None,
            }
        })
    }

    /// The kf-chip kind.
    #[must_use]
    pub const fn kind(self) -> kf_chip::classes::Kind {
        match self {
            GrClass::TwoD => kf_chip::classes::Kind::TwoD,
            GrClass::InlineToMemory => kf_chip::classes::Kind::InlineToMemory,
            GrClass::ThreeD => kf_chip::classes::Kind::ThreeD,
            GrClass::Compute => kf_chip::classes::Kind::Compute,
        }
    }

    /// What the kind is called in a refusal.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            GrClass::TwoD => "the 2D class",
            GrClass::InlineToMemory => "the inline-to-memory class",
            GrClass::ThreeD => "the 3D class",
            GrClass::Compute => "the compute class",
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
    /// ★ An ADDRESS register's upper bits (`ADDRESS_UPPER` 7:0) — held, never emitted as written.
    AddressUpper8,
    /// ★ The same address's lower word (`ADDRESS_LOWER` 31:0): the decoder pairs it with the upper
    /// bits into a guest VA of `bytes` written bytes, which the binder resolves through the
    /// placement rows into a window address and the address perimeter emits.
    AddressLower32 {
        /// The upper register's method.
        upper: u32,
        /// The bytes the engine may write there.
        bytes: u64,
    },
    /// ★ Batch 3 (run44): `SET_REPORT_SEMAPHORE_D`, the trigger. Admitted only as a one-word
    /// RELEASE of the payload with no report, reduction, trap or awaken — see
    /// [`reauthor_report_semaphore_d`].
    ReportSemaphoreD,
}

/// `NV*97/C0_SET_NOTIFY_A/B` and `NOTIFY` (`ogkm-580.65.06: clc997.h:41-50`; the compute layout
/// from `clc7c0.h:41-50`, because OGKM's `clc9c0.h` names only the class id, line 27 — INFERRED to
/// be unchanged for `ADA_COMPUTE_A`, as it is for every compute class header OGKM does carry).
/// A `NOTIFY` writes one 16-byte `NvNotification` there; NOTIFY itself is not admitted.
const NOTIFY_BYTES: u64 = 16;

/// ★ Batch 2 (run32): the 3D and compute classes, admitted: exactly the two methods Windows'
/// kernel GR channel sent on each — the notifier address, validated and re-authored.
pub const NOTIFY_ALLOWED: [Row; 2] = [
    row(0x0104, "SET_NOTIFY_A", Field::AddressUpper8),
    row(
        0x0108,
        "SET_NOTIFY_B",
        Field::AddressLower32 {
            upper: 0x0104,
            bytes: NOTIFY_BYTES,
        },
    ),
];

/// ★ Batch 3 (run44, 2026-10-07): the 3D class's report-semaphore release, exactly as Windows'
/// kernel GR channel sent it (`traces/windows_code43_walls_20261007/README.md`, "Run44 result":
/// `200406c0 00000001 20286060 <payload> 1000f010`, a 4-word incrementing write on the 3D
/// subchannel). `NVC997_SET_REPORT_SEMAPHORE_A/B/C/D` (`ogkm-580.65.06:
/// src/common/sdk/nvidia/inc/class/clc997.h:3913-3922`): `A` `OFFSET_UPPER` 7:0, `B` `OFFSET_LOWER`
/// 31:0, `C` `PAYLOAD` 31:0, `D` the trigger. The engine writes the payload (one word: `D` must say
/// `STRUCTURE_SIZE_ONE_WORD`) at `A:B`, which is resolved through the placement rows like a
/// notifier. It is the completion of Windows' GR work, written by the real engine (§S item 7).
///
/// ⊘ A family row, by hand (allowed: `AGENTS.md`, *Derive, never capture*): the method offsets and
/// every `D` field are identical in the public `TURING_A`, `AMPERE_A`, `AMPERE_B`, `ADA_A` and
/// `HOPPER_A` headers (`clc597.h`, `clc697.h`, `clc797.h`, `clc997.h`, `clcb97.h`, compared field by
/// field 2026-10-07); `BLACKWELL_A/B`'s (`clcd97.h`, `clce97.h`) name only the class id, so the rows
/// are not admitted there.
pub const REPORT_SEMAPHORE_FAMILIES: [kf_chip::Family; 4] = [
    kf_chip::Family::Turing,
    kf_chip::Family::Ampere,
    kf_chip::Family::Ada,
    kf_chip::Family::Hopper,
];

/// The payload word the engine writes for a `STRUCTURE_SIZE_ONE_WORD` report.
const REPORT_SEMAPHORE_BYTES: u64 = 4;

/// Batch 3's rows (3D class only; see [`REPORT_SEMAPHORE_FAMILIES`]).
pub const REPORT_SEMAPHORE_ALLOWED: [Row; 4] = [
    row(0x1B00, "SET_REPORT_SEMAPHORE_A", Field::AddressUpper8),
    row(
        0x1B04,
        "SET_REPORT_SEMAPHORE_B",
        Field::AddressLower32 {
            upper: 0x1B00,
            bytes: REPORT_SEMAPHORE_BYTES,
        },
    ),
    row(0x1B08, "SET_REPORT_SEMAPHORE_C", Field::Word),
    row(0x1B0C, "SET_REPORT_SEMAPHORE_D", Field::ReportSemaphoreD),
];

/// `SET_REPORT_SEMAPHORE_D` fields (`clc997.h:3923-4010`).
mod semd {
    /// `RELEASE` 4:4: after all preceding reads (0) or writes (1) complete — both admitted.
    pub const RELEASE: u32 = 1 << 4;
    /// `PIPELINE_LOCATION` 15:12.
    pub const PIPELINE_SHIFT: u32 = 12;
    /// Its mask.
    pub const PIPELINE_MASK: u32 = 0xF << PIPELINE_SHIFT;
    /// The named `PIPELINE_LOCATION` values (`:3935-3946`): NONE, DATA_ASSEMBLER, VERTEX_SHADER,
    /// VPC, STREAMING_OUTPUT, GEOMETRY_SHADER, ZCULL, TESSELATION_INIT_SHADER, TESSELATION_SHADER,
    /// PIXEL_SHADER, DEPTH_TEST, ALL.
    pub const PIPELINE_NAMED: [u32; 12] =
        [0x0, 0x1, 0x2, 0x4, 0x5, 0x6, 0x7, 0x8, 0x9, 0xA, 0xC, 0xF];
    /// `STRUCTURE_SIZE` 28:28 = `ONE_WORD` (`:3985-3987`).
    pub const ONE_WORD: u32 = 1 << 28;
}

/// Re-author `SET_REPORT_SEMAPHORE_D`: admitted only as `OPERATION_RELEASE` (1:0 = 0) of one word
/// (`STRUCTURE_SIZE_ONE_WORD`), `REPORT_NONE` (27:23), with `FLUSH_DISABLE`, `REDUCTION_ENABLE`,
/// `SUB_REPORT`, `ACQUIRE`, `REDUCTION_OP`, `COMPARISON`, `REDUCTION_FORMAT`, `CONDITIONAL_TRAP`,
/// `AWAKEN_ENABLE`, `REPORT_DWORD_NUMBER` and every unnamed bit zero; `RELEASE` (4:4) and a named
/// `PIPELINE_LOCATION` (15:12) are carried. The word is rebuilt from those two fields.
///
/// # Errors
/// Any other operation, size, report, reduction, trap, awaken, or an unnamed bit.
pub fn reauthor_report_semaphore_d(v: u32) -> Result<u32, &'static str> {
    if v & semd::ONE_WORD == 0 {
        return Err("SET_REPORT_SEMAPHORE_D: only STRUCTURE_SIZE_ONE_WORD is admitted");
    }
    if v & !(semd::RELEASE | semd::PIPELINE_MASK | semd::ONE_WORD) != 0 {
        return Err(
            "SET_REPORT_SEMAPHORE_D: only a plain one-word RELEASE (no report, reduction, trap, awaken, flush-disable) is admitted",
        );
    }
    let pipe = (v & semd::PIPELINE_MASK) >> semd::PIPELINE_SHIFT;
    if !semd::PIPELINE_NAMED.contains(&pipe) {
        return Err("SET_REPORT_SEMAPHORE_D: a PIPELINE_LOCATION the class header does not name");
    }
    Ok((v & semd::RELEASE) | (pipe << semd::PIPELINE_SHIFT) | semd::ONE_WORD)
}

/// ★ Batch 3: the disposition of a report-semaphore method on graphics class `class`, or `None` when
/// `method` is not one of [`REPORT_SEMAPHORE_ALLOWED`] or `class` is not a 3D class (the caller then
/// asks [`gr_method`]).
#[must_use]
pub fn report_semaphore(class: u32, method: u32) -> Option<Disposition> {
    let row = REPORT_SEMAPHORE_ALLOWED
        .iter()
        .find(|r| r.method == method)?;
    let family = kf_chip::Family::ALL
        .iter()
        .copied()
        .find(|f| kf_chip::classes::classes_for(*f).threed.contains(&class))?;
    Some(if REPORT_SEMAPHORE_FAMILIES.contains(&family) {
        Disposition::Allowed(*row)
    } else {
        Disposition::Refused(
            "SET_REPORT_SEMAPHORE_* (this family's public 3D header does not define it)",
        )
    })
}

/// ★ 3D/compute methods refused under their names (`clc997.h`, `clc7c0.h`, line of the define).
pub const NOTIFY_CLASS_REFUSED: [(u32, u32, &str); 5] = [
    (0x010C, 0x010C, "NOTIFY (the notifier write trigger)"), // :47
    (0x0114, 0x0120, "LOAD_MME_* (macro engine programming)"), // :55
    (0x0124, 0x0124, "SET_MME_SHADOW_RAM_CONTROL"),
    (0x0140, 0x0140, "PM_TRIGGER"),
    (0x3800, 0x3FFC, "CALL_MME_MACRO / CALL_MME_DATA"),
];

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
            Disposition::Refused("not in the 2D (cl902d.h) allowlist")
        }
        GrClass::InlineToMemory => {
            Disposition::Refused("not in the inline-to-memory allowlist (none admitted)")
        }
        GrClass::ThreeD | GrClass::Compute => {
            if let Some(r) = NOTIFY_ALLOWED.iter().find(|r| r.method == method) {
                return Disposition::Allowed(*r);
            }
            if let Some(&(_, _, name)) = NOTIFY_CLASS_REFUSED
                .iter()
                .find(|&&(lo, hi, _)| (lo..=hi).contains(&method))
            {
                return Disposition::Refused(name);
            }
            Disposition::Refused("not in the 3D/compute allowlist")
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
        Field::AddressUpper8 => {
            if v & !0xFF != 0 {
                return Err("ADDRESS_UPPER beyond 7:0");
            }
            Ok(v)
        }
        Field::AddressLower32 { .. } => {
            if v & 3 != 0 {
                return Err("ADDRESS_LOWER below 4-byte alignment");
            }
            Ok(v)
        }
        Field::ReportSemaphoreD => reauthor_report_semaphore_d(v),
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

    /// ★ Batch 3 (run44): exactly the observed `D` form and its named variants pass; everything
    /// that is not a plain one-word release is refused.
    #[test]
    fn report_semaphore_d_is_a_plain_one_word_release_only() {
        assert_eq!(reauthor_report_semaphore_d(0x1000_f010), Ok(0x1000_f010));
        assert_eq!(reauthor_report_semaphore_d(0x1000_0000), Ok(0x1000_0000));
        for bad in [
            0x1000_f011u32, // OPERATION_ACQUIRE
            0x1000_f013,    // OPERATION_TRAP
            0x0000_f010,    // STRUCTURE_SIZE_FOUR_WORDS
            0x1080_f010,    // REPORT 27:23 != NONE
            0x1010_f010,    // AWAKEN_ENABLE
            0x1008_f010,    // CONDITIONAL_TRAP
            0x1000_f018,    // REDUCTION_ENABLE
            0x1000_f014,    // FLUSH_DISABLE
            0x1040_f010,    // bit 22, unnamed
            0x3000_f010,    // bit 29, unnamed
            0x1000_3010,    // PIPELINE_LOCATION 3, unnamed
        ] {
            assert!(reauthor_report_semaphore_d(bad).is_err(), "{bad:#x}");
        }
    }

    #[test]
    fn report_semaphore_rows_are_the_3d_class_of_the_header_families_only() {
        for f in kf_chip::Family::ALL {
            let set = kf_chip::classes::classes_for(f);
            for &c in set.threed {
                let d = report_semaphore(c, 0x1B0C);
                if REPORT_SEMAPHORE_FAMILIES.contains(&f) {
                    assert!(matches!(d, Some(Disposition::Allowed(_))), "{c:#x}");
                } else {
                    assert!(matches!(d, Some(Disposition::Refused(_))), "{c:#x}");
                }
            }
            // Not a 3D class: the per-kind tables decide (no report-semaphore row).
            for &c in set.compute.iter().chain(set.twod) {
                assert!(report_semaphore(c, 0x1B00).is_none(), "{c:#x}");
            }
        }
        // Neighbours of the rows are not admitted by them.
        let ada = kf_chip::classes::classes_for(kf_chip::Family::Ada).threed[0];
        assert!(report_semaphore(ada, 0x1B10).is_none());
        assert!(report_semaphore(ada, 0x1AFC).is_none());
        assert!(matches!(
            gr_method(GrClass::ThreeD, 0x1B10),
            Disposition::Refused(_)
        ));
        // The address halves carry the one-word footprint.
        assert_eq!(
            REPORT_SEMAPHORE_ALLOWED[1].field,
            Field::AddressLower32 {
                upper: 0x1B00,
                bytes: 4
            }
        );
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
    fn batch2_admits_only_the_notifier_address_on_3d_and_compute() {
        assert_eq!(GrClass::of_class(0xC997), Some(GrClass::ThreeD));
        assert_eq!(GrClass::of_class(0xC9C0), Some(GrClass::Compute));
        assert_eq!(GrClass::of_class(0xC7B5), None);
        for c in [GrClass::ThreeD, GrClass::Compute] {
            for m in (0x100..0x4000u32).step_by(4) {
                let allowed = matches!(gr_method(c, m), Disposition::Allowed(_));
                assert_eq!(allowed, m == 0x104 || m == 0x108, "{m:#x}");
            }
            assert!(matches!(gr_method(c, 0x10c), Disposition::Refused(n) if n.contains("NOTIFY")));
        }
        assert!(reauthor(&NOTIFY_ALLOWED[0], 0x100).is_err());
        assert!(reauthor(&NOTIFY_ALLOWED[1], 0x2023_b3a2).is_err());
        assert_eq!(reauthor(&NOTIFY_ALLOWED[1], 0x2023_b3a0), Ok(0x2023_b3a0));
    }

    #[test]
    fn the_inert_rule_knows_engine_classes() {
        assert!(!is_known_class(1)); // NV01_ROOT_NON_PRIV, Windows' sub-5 value
        assert!(is_known_class(0xc7b5));
        let ada = kf_chip::classes::classes_for(kf_chip::Family::Ada);
        assert!(is_known_class(ada.twod[0]));
        assert!(is_known_class(ada.inline_to_memory[0]));
    }

    /// The 2D table is `cl902d.h`'s layout: every family's generated 2D set must be exactly the
    /// class that header defines (the generated `class_ids:FERMI_TWOD_A` row), and every family's
    /// inline-to-memory class is recognised on its own (Blackwell's included).
    #[test]
    fn class_ids_are_derived_from_the_generated_family_sets() {
        let header: Vec<u32> = kf_abi::generated::matrix::CLASS_IDS_FERMI_TWOD_A
            .runs
            .iter()
            .filter_map(|r| r.value)
            .filter_map(|v| u32::try_from(v).ok())
            .collect();
        assert!(!header.is_empty());
        for f in kf_chip::Family::ALL {
            let set = kf_chip::classes::classes_for(f);
            for &c in set.twod {
                assert!(header.contains(&c), "{f:?} 2D {c:#x}");
                assert_eq!(GrClass::of_class(c), Some(GrClass::TwoD));
            }
            for &c in set.inline_to_memory {
                assert_eq!(GrClass::of_class(c), Some(GrClass::InlineToMemory), "{f:?}");
            }
            for &c in set.threed {
                assert_eq!(GrClass::of_class(c), Some(GrClass::ThreeD), "{f:?}");
            }
        }
    }
}
