//! ★★★ **The hardware reference table — per DIE GROUP, from ogkm's own headers.**
//! (`docs/design/V3_HW_BOUNDARY_INVENTORY.md`.)
//!
//! The hardware-side counterpart of the ioctl ABI table: every register offset, bit field,
//! aperture, in-memory structure field, class-method layout and USERD/notifier struct offset on
//! kayfabe's hardware boundary, **as the C compiler evaluates it** from ogkm's published headers.
//!
//! Two layers, deliberately separate:
//!
//! 1. **Extraction** — `tools/derive_hwref.sh` compiles every chip directory's headers and prints one
//!    row per `(directory, macro)`; the rows are committed as [`TABLE`] (`data/hwref-<ver>.tsv`) and a
//!    test re-derives them whenever ogkm is present. Nothing in the extraction knows about families.
//! 2. **Resolution** (this module) — which directory a die group's driver actually compiles against.
//!    ogkm publishes only the headers each chip's *own* code needs; a Hopper HAL that reuses an Ampere
//!    body is compiled against the Ampere header, and `NV_RAMUSERD_GP_GET` exists only under
//!    `maxwell/gm107`, `pascal/gp100` and `ampere/ga100`. So a die group has a **lineage**: tiers of
//!    directories, its own first ([`DieGroup::tiers`]). A name resolves in the **nearest tier that
//!    defines it**; if the directories of that tier disagree, it is [`Resolved::Ambiguous`] — the
//!    lineage cannot say which HAL is bound — until a [`PINS`] row names the directory and the HAL.
//!
//! ⊘ The lineage is the ONE hand-maintained mapping here, and it is per die group (§50 level 6), never
//! per register: a register a newer die moved shows up as an `Own` row of that die, and a register
//! two candidate ancestors define differently shows up as `Ambiguous` — it cannot silently take the
//! wrong one. ★ `[measured]` between ogkm 580.159.04 and 610.43.02, **0** of the 7 811 `(directory,
//! name)` rows both versions publish changed value: these are hardware facts, not driver-version ABI.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::{Family, FamilyRefusal, arch};

/// The generated rows (`tools/derive_hwref.sh`, ogkm 580.159.04 — the version the bench runs).
pub const TABLE: &str = include_str!("../data/hwref-580.159.04.tsv");

/// ★ A die group: the unit a hardware fact varies by. A [`Family`] is ogkm's architecture axis; two
/// families hold two die groups each whose hardware differs (GA100 vs GA10x, GB10x vs GB20x).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DieGroup {
    /// Turing TU10x/TU11x.
    Tu10x,
    /// Ampere GA100 (datacenter).
    Ga100,
    /// Ampere GA102…GA107.
    Ga10x,
    /// Ada AD102…AD107.
    Ad10x,
    /// Hopper GH100.
    Gh100,
    /// Blackwell datacenter GB100/GB102/GB110/GB112.
    Gb10x,
    /// Blackwell consumer GB202…GB207.
    Gb20x,
}

/// `(directory …)` tiers — a name resolves in the first tier that defines it.
type Tiers = &'static [&'static [&'static str]];

// The shared tail of every lineage: pre-Turing ancestors whose headers newer families still compile
// against (the method header, USERD, the instance block), then `nv_ref.h`. UVM's own copies sit in
// the tier of the chip they copy, so a disagreement between the two is an ambiguity, not a choice.
const T_TU: &[&str] = &["turing/tu102", "turing/tu104", "uvm/turing/tu102"];
const T_GV: &[&str] = &["volta/gv100", "uvm/volta/gv100"];
const T_GP102: &[&str] = &["pascal/gp102"];
const T_GP100: &[&str] = &["pascal/gp100", "uvm/pascal/gp100"];
const T_GM200: &[&str] = &["maxwell/gm200"];
const T_GM107: &[&str] = &["maxwell/gm107", "uvm/maxwell/gm107"];
const T_GK: &[&str] = &["kepler/gk104"];
const T_REF: &[&str] = &["published"];
const T_GA100: &[&str] = &["ampere/ga100", "uvm/ampere/ga100"];
const T_GA102: &[&str] = &["ampere/ga102"];
const T_AD: &[&str] = &["ada/ad102"];
const T_GH: &[&str] = &["hopper/gh100", "uvm/hopper/gh100"];
const T_GB10X: &[&str] = &[
    "blackwell/gb100",
    "blackwell/gb102",
    "blackwell/gb110",
    "blackwell/gb112",
    "uvm/blackwell/gb100",
];
// A Hopper HAL reuses GA100 bodies AND some GA102 ones (`kmemsysReadUsableFbSize_GA102` is bound for
// GH100, `g_kern_mem_sys_nvoc.c:353-357`), so both are ONE tier for GH100 and later: a name they
// define differently is ambiguous there, never silently the nearer one.
const T_GA10X_AND_GA100: &[&str] = &["ampere/ga102", "ampere/ga100", "uvm/ampere/ga100"];
// Consumer Blackwell binds `_GB100`, `_GH100` and some consumer (`_AD102`/`_GA102`) bodies.
const T_GB10X_AND_GH: &[&str] = &[
    "blackwell/gb100",
    "blackwell/gb102",
    "blackwell/gb110",
    "blackwell/gb112",
    "uvm/blackwell/gb100",
    "hopper/gh100",
    "uvm/hopper/gh100",
];
const T_AD_GA10X_GA100: &[&str] = &[
    "ada/ad102",
    "ampere/ga102",
    "ampere/ga100",
    "uvm/ampere/ga100",
];

impl DieGroup {
    /// Every die group.
    pub const ALL: [DieGroup; 7] = [
        DieGroup::Tu10x,
        DieGroup::Ga100,
        DieGroup::Ga10x,
        DieGroup::Ad10x,
        DieGroup::Gh100,
        DieGroup::Gb10x,
        DieGroup::Gb20x,
    ];

    /// The family this die group belongs to.
    #[must_use]
    pub const fn family(self) -> Family {
        match self {
            DieGroup::Tu10x => Family::Turing,
            DieGroup::Ga100 | DieGroup::Ga10x => Family::Ampere,
            DieGroup::Ad10x => Family::Ada,
            DieGroup::Gh100 => Family::Hopper,
            DieGroup::Gb10x | DieGroup::Gb20x => Family::Blackwell,
        }
    }

    /// The die group for what the host reported (`MC_GET_ARCH_INFO`), with the same refusals as
    /// [`Family::from_arch`].
    ///
    /// # Errors
    /// [`FamilyRefusal`], by name.
    pub fn from_arch(architecture: u32, implementation: u32) -> Result<DieGroup, FamilyRefusal> {
        Ok(match Family::from_arch(architecture, implementation)? {
            Family::Turing => DieGroup::Tu10x,
            Family::Ampere if implementation == arch::IMPL_GA100 => DieGroup::Ga100,
            Family::Ampere => DieGroup::Ga10x,
            Family::Ada => DieGroup::Ad10x,
            Family::Hopper => DieGroup::Gh100,
            Family::Blackwell if architecture == arch::GB100 => DieGroup::Gb10x,
            Family::Blackwell => DieGroup::Gb20x,
        })
    }

    /// ★ The lineage: tiers of chip directories, this die group's own first. A name resolves in the
    /// nearest tier that defines it.
    #[must_use]
    pub const fn tiers(self) -> Tiers {
        match self {
            DieGroup::Tu10x => &[T_TU, T_GV, T_GP102, T_GP100, T_GM200, T_GM107, T_GK, T_REF],
            DieGroup::Ga100 => &[
                T_GA100, T_TU, T_GV, T_GP102, T_GP100, T_GM200, T_GM107, T_GK, T_REF,
            ],
            DieGroup::Ga10x => &[
                T_GA102, T_GA100, T_TU, T_GV, T_GP102, T_GP100, T_GM200, T_GM107, T_GK, T_REF,
            ],
            DieGroup::Ad10x => &[
                T_AD, T_GA102, T_GA100, T_TU, T_GV, T_GP102, T_GP100, T_GM200, T_GM107, T_GK, T_REF,
            ],
            DieGroup::Gh100 => &[
                T_GH,
                T_GA10X_AND_GA100,
                T_TU,
                T_GV,
                T_GP102,
                T_GP100,
                T_GM200,
                T_GM107,
                T_GK,
                T_REF,
            ],
            DieGroup::Gb10x => &[
                T_GB10X,
                T_GH,
                T_GA10X_AND_GA100,
                T_TU,
                T_GV,
                T_GP102,
                T_GP100,
                T_GM200,
                T_GM107,
                T_GK,
                T_REF,
            ],
            DieGroup::Gb20x => &[
                &["blackwell/gb202"],
                T_GB10X_AND_GH,
                T_AD_GA10X_GA100,
                T_TU,
                T_GV,
                T_GP102,
                T_GP100,
                T_GM200,
                T_GM107,
                T_GK,
                T_REF,
            ],
        }
    }
}

/// ★ A name the lineage leaves [`Resolved::Ambiguous`] for a die group, settled by naming the HAL
/// that die group binds and the directory that HAL's source includes. `(die group, name, directory,
/// why)`. ⊘ Only names kayfabe uses get one: for every other ambiguous name the resolver refuses
/// rather than guesses (`cargo run -p kf-chip --example hwref -- --differs` lists them).
pub const PINS: &[(DieGroup, &str, &str, &str)] = &[
    // GH100 and GB10x publish a `dev_riscv_pri.h` WITHOUT the IRQ pair, and GA100 (0x2b4/0x2b8) and
    // GA102 (0x528/0x52c) disagree. The reader is `kflcnRiscvReadIntrStatus`, whose default arm — every
    // chip past TU1xx/GA100, incl. GH100 and GB1xx — is `_GA102` (`ogkm-580: g_kernel_falcon_nvoc.c:
    // 628-635`), compiled against `published/ampere/ga102/dev_riscv_pri.h` (`kernel_falcon_ga102.c:35`).
    (
        DieGroup::Gh100,
        "NV_PRISCV_RISCV_IRQMASK",
        "ampere/ga102",
        RISCV_IRQ_GA102,
    ),
    (
        DieGroup::Gh100,
        "NV_PRISCV_RISCV_IRQDEST",
        "ampere/ga102",
        RISCV_IRQ_GA102,
    ),
    (
        DieGroup::Gb10x,
        "NV_PRISCV_RISCV_IRQMASK",
        "ampere/ga102",
        RISCV_IRQ_GA102,
    ),
    (
        DieGroup::Gb10x,
        "NV_PRISCV_RISCV_IRQDEST",
        "ampere/ga102",
        RISCV_IRQ_GA102,
    ),
];
const RISCV_IRQ_GA102: &str = "kflcnRiscvReadIntrStatus_GA102 (default arm past GA100, g_kernel_falcon_nvoc.c:628-635; \
                               kernel_falcon_ga102.c:35 includes ampere/ga102/dev_riscv_pri.h)";

/// One evaluated value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwValue {
    /// A plain value: an offset, an enum value, a size.
    Val(u64),
    /// A `hi:lo` range: a register bit field, an aperture (`0x3FFFF:0x30000`), or a structure bit
    /// range (`(34*32+31):(34*32+0)` = bits `1119:1088`).
    Range {
        /// `DRF_EXTENT`.
        hi: u64,
        /// `DRF_BASE`.
        lo: u64,
    },
    /// A multi-word `MW(hi:lo)` field — bit positions across a structure of 32-bit words.
    MultiWord {
        /// High bit, counted from bit 0 of word 0.
        hi: u64,
        /// Low bit.
        lo: u64,
    },
    /// `offsetof` of a struct member.
    Offset(u64),
    /// `sizeof` of a struct.
    Size(u64),
    /// A struct member the header does not declare (`Nvc96fControl.GPGet`).
    Absent,
}

impl HwValue {
    /// Parse the table's value column (our own generated format).
    fn parse(s: &str) -> Option<HwValue> {
        fn num(t: &str) -> Option<u64> {
            match t.strip_prefix("0x") {
                Some(h) => u64::from_str_radix(h, 16).ok(),
                None => t.parse().ok(),
            }
        }
        if s == "absent" {
            return Some(HwValue::Absent);
        }
        if let Some(o) = s.strip_prefix("off=") {
            return num(o).map(HwValue::Offset);
        }
        if let Some(o) = s.strip_prefix("size=") {
            return num(o).map(HwValue::Size);
        }
        if let Some(r) = s.strip_prefix("mw:") {
            let (h, l) = r.split_once(':')?;
            return Some(HwValue::MultiWord {
                hi: num(h)?,
                lo: num(l)?,
            });
        }
        if let Some((h, l)) = s.split_once(':') {
            return Some(HwValue::Range {
                hi: num(h)?,
                lo: num(l)?,
            });
        }
        num(s).map(HwValue::Val)
    }
}

/// How a name resolved for a die group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// Defined in the die group's own tier.
    Own {
        /// The value.
        value: HwValue,
        /// The directory that defines it (the first, when several agree).
        dir: &'static str,
    },
    /// Not in the own tier; the nearest ancestor tier that defines it agrees on one value.
    Inherited {
        /// The value.
        value: HwValue,
        /// The ancestor directory.
        dir: &'static str,
    },
    /// Settled by a [`PINS`] row.
    Pinned {
        /// The value.
        value: HwValue,
        /// The pinned directory.
        dir: &'static str,
        /// The HAL binding that justifies the pin.
        why: &'static str,
    },
    /// ⊘ The nearest tier's directories disagree: which one applies depends on the bound HAL.
    Ambiguous {
        /// Every `(directory, value)` of that tier.
        candidates: Vec<(&'static str, HwValue)>,
    },
    /// No directory of the lineage defines it.
    Absent,
}

impl Resolved {
    /// The value, when the resolution is decided (own, inherited or pinned).
    #[must_use]
    pub fn value(&self) -> Option<HwValue> {
        match self {
            Resolved::Own { value, .. }
            | Resolved::Inherited { value, .. }
            | Resolved::Pinned { value, .. } => Some(*value),
            Resolved::Ambiguous { .. } | Resolved::Absent => None,
        }
    }
}

/// The parsed table.
#[derive(Debug)]
pub struct HwRef {
    /// `name → [(directory, value)]`.
    by_name: HashMap<&'static str, Vec<(&'static str, HwValue)>>,
}

/// ★ The parsed table (parsed once).
///
/// # Panics
/// If [`TABLE`] holds a malformed row — the generator's output is the only input, so that is a
/// broken regeneration, caught by this crate's tests.
#[must_use]
pub fn table() -> &'static HwRef {
    static T: OnceLock<HwRef> = OnceLock::new();
    T.get_or_init(|| {
        let mut by_name: HashMap<&'static str, Vec<(&'static str, HwValue)>> = HashMap::new();
        for line in TABLE
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let mut f = line.split('\t');
            let (Some(dir), Some(_hdr), Some(name), Some(v), None) =
                (f.next(), f.next(), f.next(), f.next(), f.next())
            else {
                panic!("hwref: malformed row {line:?}");
            };
            let value =
                HwValue::parse(v).unwrap_or_else(|| panic!("hwref: unparseable value in {line:?}"));
            by_name.entry(name).or_default().push((dir, value));
        }
        HwRef { by_name }
    })
}

impl HwRef {
    /// Every `(directory, value)` that defines `name`.
    #[must_use]
    pub fn rows(&self, name: &str) -> &[(&'static str, HwValue)] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }

    /// The value `dir` defines for `name`.
    #[must_use]
    pub fn in_dir(&self, dir: &str, name: &str) -> Option<HwValue> {
        self.rows(name)
            .iter()
            .find(|(d, _)| *d == dir)
            .map(|(_, v)| *v)
    }

    /// A class-header or struct row (`dir == "class"`).
    #[must_use]
    pub fn class(&self, name: &str) -> Option<HwValue> {
        self.in_dir("class", name)
    }

    /// ★ Resolve `name` for `group`: the nearest tier that defines it, a pin, or a named refusal.
    #[must_use]
    pub fn resolve(&self, group: DieGroup, name: &str) -> Resolved {
        if let Some(&(_, _, dir, why)) = PINS.iter().find(|(g, n, _, _)| *g == group && *n == name)
        {
            return match self.in_dir(dir, name) {
                Some(value) => Resolved::Pinned { value, dir, why },
                None => Resolved::Absent,
            };
        }
        let rows = self.rows(name);
        for (i, tier) in group.tiers().iter().enumerate() {
            let hits: Vec<(&'static str, HwValue)> = tier
                .iter()
                .filter_map(|d| rows.iter().find(|(rd, _)| rd == d).copied())
                .collect();
            let Some(&(dir, value)) = hits.first() else {
                continue;
            };
            if hits.iter().any(|(_, v)| *v != value) {
                return Resolved::Ambiguous { candidates: hits };
            }
            return if i == 0 {
                Resolved::Own { value, dir }
            } else {
                Resolved::Inherited { value, dir }
            };
        }
        Resolved::Absent
    }

    /// The resolved plain value of `name` for `group`, or why there is none.
    ///
    /// # Errors
    /// The [`Resolved`] that is not a plain value.
    pub fn value(&self, group: DieGroup, name: &str) -> Result<u64, Resolved> {
        match self.resolve(group, name) {
            r @ (Resolved::Own { .. } | Resolved::Inherited { .. } | Resolved::Pinned { .. }) => {
                match r.value() {
                    Some(HwValue::Val(v)) => Ok(v),
                    _ => Err(r),
                }
            }
            r => Err(r),
        }
    }

    /// The resolved `hi:lo` range of `name` for `group`, or why there is none.
    ///
    /// # Errors
    /// The [`Resolved`] that is not a range.
    pub fn range(&self, group: DieGroup, name: &str) -> Result<(u64, u64), Resolved> {
        let r = self.resolve(group, name);
        match r.value() {
            Some(HwValue::Range { hi, lo }) => Ok((hi, lo)),
            _ => Err(r),
        }
    }
}

/// ★ Resolve-or-panic helpers for the `#[test]`s that hold a hand-written constant to the header of
/// every die group it serves — the panic message carries the resolution, so a failure says WHICH
/// directory disagreed (or that the lineage could not decide).
pub mod expect {
    use super::{DieGroup, HwValue, Resolved, table};

    fn decided(g: DieGroup, name: &str) -> HwValue {
        match table().resolve(g, name) {
            r @ (Resolved::Own { .. } | Resolved::Inherited { .. } | Resolved::Pinned { .. }) => {
                r.value().unwrap_or(HwValue::Absent)
            }
            r => panic!("{g:?} {name}: no decided value in ogkm ({r:?})"),
        }
    }

    /// A plain value (an offset, an enum value).
    ///
    /// # Panics
    /// When the name does not resolve to a plain value for `g`.
    #[must_use]
    pub fn val(g: DieGroup, name: &str) -> u64 {
        match decided(g, name) {
            HwValue::Val(v) => v,
            v => panic!("{g:?} {name}: {v:?} is not a plain value"),
        }
    }

    /// A `hi:lo` range.
    ///
    /// # Panics
    /// When the name does not resolve to a range for `g`.
    #[must_use]
    pub fn range(g: DieGroup, name: &str) -> (u64, u64) {
        match decided(g, name) {
            HwValue::Range { hi, lo } => (hi, lo),
            v => panic!("{g:?} {name}: {v:?} is not a range"),
        }
    }

    /// The base of an aperture / the low bit of a field (`DRF_BASE`).
    ///
    /// # Panics
    /// As [`range`].
    #[must_use]
    pub fn base(g: DieGroup, name: &str) -> u64 {
        range(g, name).1
    }

    /// The length of an aperture (`DRF_SIZE`), i.e. `hi - lo + 1`.
    ///
    /// # Panics
    /// As [`range`].
    #[must_use]
    pub fn len(g: DieGroup, name: &str) -> u64 {
        let (hi, lo) = range(g, name);
        hi - lo + 1
    }

    /// `1 << lo` of a ONE-bit field.
    ///
    /// # Panics
    /// As [`range`], or when the field is wider than one bit.
    #[must_use]
    pub fn bit(g: DieGroup, name: &str) -> u64 {
        let (hi, lo) = range(g, name);
        assert_eq!(hi, lo, "{g:?} {name}: {hi}:{lo} is not one bit");
        1 << lo
    }

    /// The unshifted mask of a field (`DRF_MASK`).
    ///
    /// # Panics
    /// As [`range`].
    #[must_use]
    pub fn mask(g: DieGroup, name: &str) -> u64 {
        let (hi, lo) = range(g, name);
        if hi - lo + 1 >= 64 {
            u64::MAX
        } else {
            (1u64 << (hi - lo + 1)) - 1
        }
    }

    /// A class-header value (`NVC86F_SEM_EXECUTE`, …).
    ///
    /// # Panics
    /// When the class headers define no such plain value.
    #[must_use]
    pub fn class_val(name: &str) -> u64 {
        match table().class(name) {
            Some(HwValue::Val(v)) => v,
            v => panic!("class {name}: {v:?}"),
        }
    }

    /// A class-header `hi:lo` field.
    ///
    /// # Panics
    /// When the class headers define no such range.
    #[must_use]
    pub fn class_range(name: &str) -> (u64, u64) {
        match table().class(name) {
            Some(HwValue::Range { hi, lo }) => (hi, lo),
            v => panic!("class {name}: {v:?}"),
        }
    }

    /// A struct member's offset (`Nvc86fControl.GPGet`), `None` when the struct does not declare it.
    ///
    /// # Panics
    /// When the table has no row for it at all.
    #[must_use]
    pub fn class_offset(name: &str) -> Option<u64> {
        match table().class(name) {
            Some(HwValue::Offset(o)) => Some(o),
            Some(HwValue::Absent) => None,
            v => panic!("struct {name}: {v:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_parses_and_every_directory_is_placed_or_deliberately_outside_every_lineage() {
        let t = table();
        assert!(
            t.by_name.len() > 3000,
            "the table is the whole vocabulary, not a sample"
        );
        // A directory the generator emits must sit in some lineage, or be one of the named
        // non-discrete trees — so a new chip directory in a future ogkm cannot be silently unread.
        const OUTSIDE: &[&str] = &[
            "class",
            "ampere/ga10b",
            "blackwell/gb10b",
            "blackwell/gb20b",
        ];
        let placed: std::collections::BTreeSet<&str> = DieGroup::ALL
            .iter()
            .flat_map(|g| g.tiers().iter().flat_map(|t| t.iter().copied()))
            .collect();
        for rows in t.by_name.values() {
            for (d, _) in rows {
                assert!(
                    placed.contains(d) || OUTSIDE.contains(d),
                    "directory {d} is in no lineage"
                );
            }
        }
    }

    #[test]
    fn a_register_a_newer_die_moved_resolves_per_die_group() {
        // ★ planted positive for "Own": the RISC-V IRQ mask moved between the TU102 and GA102 blocks.
        let t = table();
        assert_eq!(
            t.value(DieGroup::Tu10x, "NV_PRISCV_RISCV_IRQMASK"),
            Ok(0x2b4)
        );
        assert_eq!(
            t.value(DieGroup::Ga100, "NV_PRISCV_RISCV_IRQMASK"),
            Ok(0x2b4)
        );
        assert_eq!(
            t.value(DieGroup::Ga10x, "NV_PRISCV_RISCV_IRQMASK"),
            Ok(0x528)
        );
        // Ada publishes no dev_riscv_pri.h: it inherits GA102's.
        assert!(matches!(
            t.resolve(DieGroup::Ad10x, "NV_PRISCV_RISCV_IRQMASK"),
            Resolved::Inherited {
                value: HwValue::Val(0x528),
                dir: "ampere/ga102"
            }
        ));
    }

    #[test]
    fn a_structure_field_only_an_ancestor_publishes_is_inherited_from_the_nearest_one() {
        // `NV_RAMUSERD_GP_GET` exists only in gm107/gp100/ga100 — word 34 = byte 0x88.
        let t = table();
        for g in DieGroup::ALL {
            assert_eq!(
                t.range(g, "NV_RAMUSERD_GP_GET"),
                Ok((34 * 32 + 31, 34 * 32)),
                "{g:?}"
            );
        }
        // ★ And the nearest ancestor wins over a farther one that differs: the PDB's high bits are
        // `7:0` of word 129 on gm107 but `31:0` from gp100 on.
        assert_eq!(
            t.range(DieGroup::Tu10x, "NV_RAMIN_PAGE_DIR_BASE_HI"),
            Ok((129 * 32 + 31, 129 * 32))
        );
    }

    #[test]
    fn two_candidate_ancestors_that_disagree_are_ambiguous_never_the_nearer_one() {
        // ★ planted positive for "Ambiguous": GB202 publishes `NV_XAL_EP_BAR0_WINDOW` but not its
        // `_BASE` field, and the next tier disagrees — GB100 says 22:0, GH100 says 21:0. Which one a
        // GB20x RM uses depends on the bound `kbusSetBAR0WindowVidOffset` HAL; the table must not
        // pick the nearer directory silently.
        let t = table();
        assert_eq!(
            t.resolve(DieGroup::Gb20x, "NV_XAL_EP_BAR0_WINDOW").value(),
            Some(HwValue::Val(0x10_fd40))
        );
        match t.resolve(DieGroup::Gb20x, "NV_XAL_EP_BAR0_WINDOW_BASE") {
            Resolved::Ambiguous { candidates } => {
                assert!(
                    candidates.contains(&("blackwell/gb100", HwValue::Range { hi: 22, lo: 0 }))
                );
                assert!(candidates.contains(&("hopper/gh100", HwValue::Range { hi: 21, lo: 0 })));
            }
            r => panic!("expected an ambiguity, got {r:?}"),
        }
        // Each die group whose own directory defines it is decided.
        assert_eq!(
            t.range(DieGroup::Gh100, "NV_XAL_EP_BAR0_WINDOW_BASE"),
            Ok((21, 0))
        );
        assert_eq!(
            t.range(DieGroup::Gb10x, "NV_XAL_EP_BAR0_WINDOW_BASE"),
            Ok((22, 0))
        );
    }

    #[test]
    fn a_pin_names_a_directory_that_defines_the_name_and_settles_a_real_ambiguity() {
        // ⊘ A pin is a claim about a HAL binding; one that points at a directory without the name,
        // or that settles nothing, is a stale row.
        let t = table();
        for (g, n, dir, _) in PINS {
            assert!(
                t.in_dir(dir, n).is_some(),
                "{g:?} {n}: pinned to {dir}, which does not define it"
            );
            assert!(matches!(t.resolve(*g, n), Resolved::Pinned { .. }));
            // Without the pin the lineage would have to guess.
            let rows = t.rows(n);
            let tier = g
                .tiers()
                .iter()
                .find(|tier| tier.iter().any(|d| rows.iter().any(|(rd, _)| rd == d)));
            let vals: Vec<HwValue> = tier
                .into_iter()
                .flat_map(|tier| tier.iter())
                .filter_map(|d| t.in_dir(d, n))
                .collect();
            assert!(
                vals.windows(2).any(|w| w[0] != w[1]),
                "{g:?} {n}: the pin settles no ambiguity"
            );
        }
    }

    #[test]
    fn die_groups_follow_the_hosts_arch_info() {
        assert_eq!(
            DieGroup::from_arch(arch::GA100, arch::IMPL_GA100),
            Ok(DieGroup::Ga100)
        );
        assert_eq!(DieGroup::from_arch(arch::GA100, 6), Ok(DieGroup::Ga10x));
        assert_eq!(DieGroup::from_arch(arch::GB100, 0), Ok(DieGroup::Gb10x));
        assert_eq!(DieGroup::from_arch(arch::GB200, 3), Ok(DieGroup::Gb20x));
        assert_eq!(DieGroup::from_arch(arch::AD100, 6), Ok(DieGroup::Ad10x));
        for g in DieGroup::ALL {
            assert_eq!(
                g.tiers().last(),
                Some(&T_REF),
                "{g:?}: nv_ref.h is every lineage's root"
            );
        }
    }
}
