// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The DERIVED display capabilities page (`NV_PDISP_FE_SW`, BAR0 `0x640000`), by tier —
//! `docs/design/V3_DISPLAY_CAPS_DERIVATION_20261014.md`.
//!
//! 2026-10-23: this module builds the lower-tier page and legacy unnamed rows.
//! Production GB20x construction then applies the approved complete PRECOMP table
//! through `kf_rm::precomp`; see V3_BLACKWELL_PRECOMP_CONSTTAB_20261023.md.
//!
//! The page a die presents is built from three kinds of source, and every word of it records which:
//!
//! - **T1, a rule** over the family's class header (`data/classes-*.tsv`, generated from ogkm) and the
//!   family row: the authored base page (`caps::sdr_page` / `caps::page`: heads, SORs, windows, TMDS
//!   clocks, ILUT/OLUT/TMO sizes, CSC presence) plus the **window-layer model** in `data/caps_rows.tsv`
//!   (the only hand-kept part: family rows). A window is layer `window % layers_per_head` of head
//!   `window / layers_per_head` (NVKMS: windows `2N`/`2N+1` belong to head `N`, ogkm
//!   `nvkms-rm.c ProbeHeadCountAndWindowAssignment`); layer 0 is the main layer, the others are overlay
//!   layers, and the two have different pipelines.
//! - **T2, a same-die startup snapshot** from host-authored READ32 regops. The
//!   display-capabilities object is privileged; the generic regops endpoint is
//!   NON_PRIVILEGED but each address still requires the host's GSP allowlist.
//!   [`crate::hostcaps`] binds physical observations to die/class/windows/layout.
//!   Field-level virtual support has priority: no residual host-backed guest field
//!   is approved today. Refusal keeps the same derived virtual page.
//!   No physical-page access permission is assumed from the endpoint's flags.
//! - **T3, a privileged per-die measurement** (`data/caps_measured.tsv`, `tools/read_display_caps_page.py`):
//!   only for words no class header names and the Windows display driver needs (bisected on hardware).
//!   A die with no row gets the clean authored page and [`Derived::unmeasured`] says so — never a
//!   neighbouring die's row (OWNER_RULINGS §AH).
//!
//! Nothing here is served from a captured page; the captured AD104 page is a test oracle only
//! (`tests/fixtures/`).

use crate::caps::{CapsPage, Missing, page, sdr_page};
use crate::class::{ClassTable, put};
use crate::regs::Regs;
use std::collections::BTreeMap;

/// Which tier produced a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// T1: a rule over the class header and the family row.
    Rule,
    /// T2: the host's own unprivileged control reply.
    Host,
    /// T3: a privileged per-die measurement row.
    Measured,
}

/// A die, as the RM reports it (`NV2080_CTRL_CMD_MC_GET_ARCH_INFO`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Die {
    /// `NV2080_CTRL_MC_ARCH_INFO_ARCHITECTURE_*`.
    pub architecture: u32,
    /// `NV2080_CTRL_MC_ARCH_INFO_IMPLEMENTATION_*`.
    pub implementation: u32,
}

/// One measured word of one die (tier 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasuredWord {
    /// The die it was read on.
    pub die: Die,
    /// The guest-driver branch the row was measured for (`580`), or `*` for a row the driver does not matter to.
    pub branch: String,
    /// The caps class of the page it was read from.
    pub caps_class: u32,
    /// Byte offset of the word in the page.
    pub offset: u32,
    /// The value.
    pub value: u32,
    /// Where it came from (`measured:<die>,<how>,<date>`); a row without one is refused.
    pub source: String,
}

/// Facts obtained from the host's own unprivileged controls at realize (tier2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostFacts {
    /// A complete same-die PRECOMP snapshot, or the unchanged derived fallback.
    pub precomp: Option<crate::hostcaps::HostPrecomp>,
}

/// The measured words shipped with kayfabe (`data/caps_measured.tsv`).
///
/// # Panics
/// Never at run time: the committed file is parsed by a test.
#[must_use]
pub fn shipped_measured() -> &'static [MeasuredWord] {
    static ROWS: std::sync::OnceLock<Vec<MeasuredWord>> = std::sync::OnceLock::new();
    ROWS.get_or_init(|| {
        parse_measured(include_str!("../data/caps_measured.tsv")).unwrap_or_default()
    })
}

fn num(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()
}

/// Parse `data/caps_measured.tsv`: `die_arch  die_impl  branch  caps_class  offset  value  source`, hex but the branch and source.
///
/// # Errors
/// By line, for a malformed row or one with no source tag.
pub fn parse_measured(tsv: &str) -> Result<Vec<MeasuredWord>, String> {
    let mut out = Vec::new();
    for (n, line) in tsv.lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let bad = || format!("caps_measured.tsv line {}: malformed row", n + 1);
        let [a, i, br, c, o, v, src] = f.as_slice() else {
            return Err(bad());
        };
        if !src.starts_with("measured:") {
            return Err(format!(
                "caps_measured.tsv line {}: the source must start with `measured:` (a blob-derived row is refused)",
                n + 1
            ));
        }
        out.push(MeasuredWord {
            die: Die {
                architecture: num(a).ok_or_else(bad)?,
                implementation: num(i).ok_or_else(bad)?,
            },
            branch: (*br).to_string(),
            caps_class: num(c).ok_or_else(bad)?,
            offset: num(o).ok_or_else(bad)?,
            value: num(v).ok_or_else(bad)?,
            source: (*src).to_string(),
        });
    }
    Ok(out)
}

/// One family row of the window-layer model (`data/caps_rows.tsv`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerRow {
    /// `main` (layer 0) or `overlay` (every other layer).
    pub overlay: bool,
    /// The caps register, header name without the class prefix.
    pub register: String,
    /// The field, header name.
    pub field: String,
    /// `TRUE`/`FALSE` (resolved through the header's `<field>_TRUE`) or a number.
    pub value: String,
    /// The row is skipped when the family's class has no such field (Blackwell has no CSC LUT).
    pub optional: bool,
    /// Why — the source of the row.
    pub basis: String,
}

/// Parse `data/caps_rows.tsv`: `kind  layer  register  field  value  required|optional  basis`.
///
/// # Errors
/// By line, for a malformed row.
pub fn parse_rows(tsv: &str) -> Result<Vec<LayerRow>, String> {
    let mut out = Vec::new();
    for (n, line) in tsv.lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let bad = || format!("caps_rows.tsv line {}: malformed row", n + 1);
        let ["LAYER", layer, register, field, value, req, basis] = f.as_slice() else {
            return Err(bad());
        };
        out.push(LayerRow {
            overlay: match *layer {
                "main" => false,
                "overlay" => true,
                _ => return Err(bad()),
            },
            register: (*register).to_string(),
            field: (*field).to_string(),
            value: (*value).to_string(),
            optional: match *req {
                "required" => false,
                "optional" => true,
                _ => return Err(bad()),
            },
            basis: (*basis).to_string(),
        });
    }
    Ok(out)
}

/// The window-layer family rows shipped with kayfabe (`data/caps_rows.tsv`).
#[must_use]
pub fn shipped_rows() -> &'static [LayerRow] {
    static ROWS: std::sync::OnceLock<Vec<LayerRow>> = std::sync::OnceLock::new();
    ROWS.get_or_init(|| parse_rows(include_str!("../data/caps_rows.tsv")).unwrap_or_default())
}

/// One word of the derived page and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// Byte offset.
    pub offset: u32,
    /// The tier that last wrote it.
    pub tier: Tier,
    /// What wrote it (a rule's basis, or the measurement's source).
    pub why: String,
}

/// The derived page, with its provenance and what it could not derive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    /// The page.
    pub page: CapsPage,
    /// Per word: tier and reason (every word of the page, in offset order).
    pub provenance: Vec<Provenance>,
    /// Words the Windows display driver needs that no header names and that have no measured row for
    /// this die: `(offset, why it is needed)`. Empty when the die is measured.
    pub unmeasured: Vec<(u32, &'static str)>,
}

/// The unnamed words the Windows display driver was bisected to NEED (`V3_DISPLAY_CAPS_DERIVATION_20261014.md`
/// §3: with the window-layer pipelines, `0x00c` is the one that moves the driver from "initialises all
/// eight windows and resets" / "hangs" to "initialises the main-layer windows and scans one").
pub const REQUIRED_UNNAMED: [(u32, &str); 1] = [(
    0x00c,
    "no class header names it; needed by the Windows display driver together with the window-layer pipelines",
)];

/// A resolved row: word offset, field `(hi, lo)`, value.
type ResolvedRow = (u32, (u8, u8), u32);

/// Resolve one row for window `index`: `(word offset, field, value)`, or `None` when the family's class
/// has no such field.
fn resolve_row(
    t: &ClassTable,
    caps: u32,
    lr: &LayerRow,
    index: u32,
) -> Result<Option<ResolvedRow>, Missing> {
    let name = format!("{}_{}", lr.register, lr.field);
    let Some(f) = t.f(caps, &name) else {
        return Ok(None);
    };
    let v = match lr.value.as_str() {
        "TRUE" | "FALSE" => t
            .v(caps, &format!("{name}_{}", lr.value))
            .ok_or_else(|| Missing(format!("NV{caps:04X}_{name}_{}", lr.value)))?,
        n => n
            .parse::<u32>()
            .map_err(|_| Missing(format!("row value {n} of {name}")))?,
    };
    if u64::from(v) >= (1_u64 << (f.0 - f.1 + 1)) {
        return Err(Missing(format!(
            "NV{caps:04X}_{name} value {v} does not fit {}:{}",
            f.0, f.1
        )));
    }
    let off = t
        .a(caps, &lr.register, index)
        .ok_or_else(|| Missing(format!("NV{caps:04X}_{}({index})", lr.register)))?;
    Ok(Some((off, f, v)))
}

/// ★ Derive the page one die presents.
///
/// `sdr` selects the SDR-colour base page (the production profile); `rows` are the window-layer family
/// rows; `measured` the T3 rows (only this die's, this class's and this driver `branch`'s rows are used);
/// `host` the T2 facts.
///
/// # Errors
/// [`Missing`] for a name the class table lacks, a row that does not fit its field, or a window count
/// that is not a multiple of the head count.
#[allow(clippy::too_many_arguments)]
pub fn derive(
    t: &ClassTable,
    r: &Regs,
    row: &kf_chip::display::DisplayRow,
    die: Die,
    sdr: bool,
    rows: &[LayerRow],
    measured: &[MeasuredWord],
    branch: &str,
    _host: &HostFacts,
) -> Result<Derived, Missing> {
    let c = &row.classes;
    let base = if sdr {
        sdr_page(t, r, c.caps, c.core, row.heads, row.windows)?
    } else {
        page(t, r, c.caps, row.heads, row.windows)?
    };
    let mut words: BTreeMap<u32, u32> = base.words.iter().copied().collect();
    let mut why: BTreeMap<u32, (Tier, String)> = base
        .words
        .iter()
        .map(|(o, _)| {
            (
                *o,
                (
                    Tier::Rule,
                    "authored base page (class header + family row)".into(),
                ),
            )
        })
        .collect();
    if row.heads == 0 || !row.windows.is_multiple_of(row.heads) {
        return Err(Missing(format!(
            "{} windows over {} heads is not a whole number of layers per head",
            row.windows, row.heads
        )));
    }
    let layers_per_head = row.windows / row.heads;
    // T1: the window-layer pipelines
    for win in 0..row.windows {
        let overlay = win % layers_per_head != 0;
        for lr in rows.iter().filter(|x| x.overlay == overlay) {
            let Some((off, f, v)) = resolve_row(t, c.caps, lr, win)? else {
                if lr.optional {
                    continue;
                }
                return Err(Missing(format!(
                    "NV{:04X}_{}_{}",
                    c.caps, lr.register, lr.field
                )));
            };
            let e = words.entry(off).or_insert(0);
            *e = put(*e, f, v);
            why.insert(
                off,
                (
                    Tier::Rule,
                    format!(
                        "window-layer model, {} layer: {}",
                        if overlay { "overlay" } else { "main" },
                        lr.basis
                    ),
                ),
            );
        }
    }
    // Tier1 derives each supported named field. Host facts remain physical
    // observations: no residual field is eligible to alter this virtual page.
    crate::precomp_fields::apply(t, row, die, &mut words)?;
    // T3: this die's measured words
    let mut unmeasured: Vec<(u32, &'static str)> = Vec::new();
    for (off, because) in REQUIRED_UNNAMED {
        let hit = measured.iter().find(|m| {
            m.die == die
                && m.caps_class == c.caps
                && m.offset == off
                && (m.branch == "*" || m.branch == branch)
        });
        match hit {
            Some(m) => {
                words.insert(off, m.value);
                why.insert(off, (Tier::Measured, m.source.clone()));
            }
            None => unmeasured.push((off, because)),
        }
    }
    words.retain(|_, v| *v != 0);
    why.retain(|o, _| words.contains_key(o));
    Ok(Derived {
        provenance: words
            .keys()
            .map(|o| {
                let (tier, w) = why.get(o).cloned().unwrap_or((Tier::Rule, String::new()));
                Provenance {
                    offset: *o,
                    tier,
                    why: w,
                }
            })
            .collect(),
        page: CapsPage {
            base: base.base,
            words: words.into_iter().collect(),
        },
        unmeasured,
    })
}
