//! Source tags for the hand-maintained Windows-cell tables ([`crate::hoststub`],
//! [`crate::sw_runlist`]).
//!
//! STATUS: LIVE, 2026-10-31. Rules: `docs/design/V3_PROVENANCE.md` sections 1 and 2.
//!
//! Every datum in those tables carries a [`Row`]: the item's name, a **source tag** and, for a
//! measurement, the in-tree trace lines that show it. The tag grammar is the one of
//! `kf_rm::consttab` (the published constant table), restricted to the three kinds that may
//! stand behind a shipped value:
//!
//! - `ogkm-<version>:<ref>` - read from the open kernel modules (a path and lines);
//! - `measured:<DIE>,<driver>,<YYYY-MM-DD>` - observed on a real GPU and shown by a trace
//!   committed in this tree (the [`Evidence`] lines);
//! - `host-derived:<ref>` - asked from the host's own driver at runtime.
//!
//! Anything else is refused, in particular a tag that names a closed binary, a disassembly, a
//! reverse-engineering result or a `blob`: such a value may stand in a diagnostic, never in a
//! shipped table (`V3_PROVENANCE.md` section 2). The refusal is [`classify`]; the tests of the two
//! table modules and `tests/windows_cell_provenance.rs` run it over every row, so a row with a
//! missing or forbidden tag is a red build.

/// The kinds of source that may stand behind a shipped value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Read from the open kernel modules.
    Ogkm,
    /// Seen on a real GPU and shown by a committed trace (a `measured:` tag, 2026-10-31 grammar).
    Measured,
    /// Asked from the host's own driver at runtime.
    HostDerived,
}

/// One trace line that shows a measurement: `at` is `<path from the repository root>:<line>` and
/// the line must contain `needle` (checked by `tests/windows_cell_provenance.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Evidence {
    /// `path:line`, the path relative to the repository root.
    pub at: &'static str,
    /// A substring that line must contain.
    pub needle: &'static str,
}

/// The provenance of one table item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The item this row covers (a field name or a group of field names).
    pub item: &'static str,
    /// The source tag; see the module documentation.
    pub source: &'static str,
    /// The trace lines that show the value. A row of the trace-backed kind needs at least one.
    pub evidence: &'static [Evidence],
}

/// Words that mark a closed-binary or reverse-engineered origin. A tag containing one is refused
/// whatever else it says.
const FORBIDDEN: &[&str] = &[
    "binary",
    "blob",
    "retail",
    "nvlddmkm",
    "disassembl",
    "decompil",
    "reverse",
    "export row",
];

/// Whole words that mark such an origin (`re`, a relative virtual address).
const FORBIDDEN_WORDS: &[&str] = &["re", "rva"];

fn valid_date(d: &str) -> bool {
    let b = d.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |s: &str| s.parse::<u32>().ok();
    match (num(&d[0..4]), num(&d[5..7]), num(&d[8..10])) {
        (Some(y), Some(m), Some(day)) => {
            (2020..=2100).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&day)
        }
        _ => false,
    }
}

fn numeric_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    (2..=3).contains(&parts.len()) && parts.iter().all(|p| p.parse::<u16>().is_ok())
}

/// Classify a source tag, or say why it is refused.
///
/// # Errors
///
/// An empty tag, a tag that names a closed binary / `re` / `blob` origin, and any tag that is not
/// one of the three accepted forms (including a tag of the trace-backed form whose die, driver
/// or date is malformed).
pub fn classify(tag: &str) -> Result<Kind, String> {
    let t = tag.trim();
    if t.is_empty() {
        return Err("empty source tag".into());
    }
    let lower = t.to_ascii_lowercase();
    if let Some(w) = FORBIDDEN.iter().find(|w| lower.contains(*w)) {
        return Err(format!("source tag {tag:?} names a forbidden origin ({w})"));
    }
    if lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| FORBIDDEN_WORDS.contains(&w))
    {
        return Err(format!(
            "source tag {tag:?} names a reverse-engineering origin"
        ));
    }
    if let Some(m) = t.strip_prefix("measured:") {
        let parts: Vec<&str> = m.split(',').collect();
        let [die, driver, date] = parts.as_slice() else {
            return Err(format!("measured tag {tag:?} is not <DIE>,<driver>,<date>"));
        };
        if die.trim().is_empty() || !numeric_version(driver) || !valid_date(date) {
            return Err(format!(
                "measured tag {tag:?}: bad die, driver (x.y or x.y.z) or date (YYYY-MM-DD)"
            ));
        }
        return Ok(Kind::Measured);
    }
    if let Some(m) = t.strip_prefix("ogkm-") {
        let (version, reference) = m
            .split_once(':')
            .ok_or_else(|| format!("ogkm tag {tag:?} needs a version and a reference"))?;
        if !numeric_version(version) || reference.trim().is_empty() {
            return Err(format!(
                "ogkm tag {tag:?} needs a numeric version and a reference"
            ));
        }
        return Ok(Kind::Ogkm);
    }
    if let Some(m) = t.strip_prefix("host-derived:") {
        if m.trim().is_empty() {
            return Err(format!("host-derived tag {tag:?} needs a reference"));
        }
        return Ok(Kind::HostDerived);
    }
    Err(format!("untagged or unknown source {tag:?}"))
}

/// Check one row: a non-empty item, an accepted tag, and for a trace-backed row at least one
/// well-formed evidence line.
///
/// # Errors
///
/// The reason the row is refused.
pub fn check_row(row: &Row) -> Result<Kind, String> {
    if row.item.trim().is_empty() {
        return Err("row without an item name".into());
    }
    let kind = classify(row.source).map_err(|e| format!("{}: {e}", row.item))?;
    if kind == Kind::Measured && row.evidence.is_empty() {
        return Err(format!("{}: a measured row names no trace line", row.item));
    }
    for e in row.evidence {
        let ok =
            e.at.rsplit_once(':')
                .is_some_and(|(p, l)| !p.is_empty() && l.parse::<usize>().is_ok_and(|n| n > 0))
                && !e.needle.is_empty();
        if !ok {
            return Err(format!(
                "{}: evidence {:?} is not <path>:<line> with a needle",
                row.item, e.at
            ));
        }
    }
    Ok(kind)
}

/// Check that `rows` covers exactly the `items` (each once) and that every row passes
/// [`check_row`].
///
/// # Errors
///
/// The first problem found.
pub fn check_rows(rows: &[Row], items: &[&str]) -> Result<(), String> {
    for r in rows {
        check_row(r)?;
    }
    for item in items {
        let n = rows.iter().filter(|r| r.item == *item).count();
        if n != 1 {
            return Err(format!(
                "item {item:?} has {n} provenance rows, need exactly one"
            ));
        }
    }
    if let Some(extra) = rows.iter().find(|r| !items.contains(&r.item)) {
        return Err(format!("provenance row for unknown item {:?}", extra.item));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_accepted_forms_classify() {
        assert_eq!(
            classify("ogkm-580.65.06:src/common/sdk/nvidia/inc/nvos.h:2924-2929"),
            Ok(Kind::Ogkm)
        );
        assert_eq!(
            classify("measured:AD104,580.88,2026-10-07"),
            Ok(Kind::Measured)
        );
        assert_eq!(
            classify("measured:GA106,580.159.04,2026-08-20"),
            Ok(Kind::Measured)
        );
        assert_eq!(
            classify("host-derived:NV2080_CTRL_CMD_GPU_GET_INFO_V2"),
            Ok(Kind::HostDerived)
        );
    }

    #[test]
    fn empty_binary_re_and_blob_origins_are_refused() {
        for bad in [
            "",
            "   ",
            "binary",
            "binary:nvlddmkm.sys",
            "re",
            "re:export-row",
            "RE: notes",
            "blob:somewhere",
            "retail-pe:0x1135a40",
            "measured:AD104,580.88,2026-10-07 (from the retail binary)",
            "ogkm-580.65.06:disassembly of the driver",
            "from a capture",
            concat!("ogkm", ":nvos.h"),
            "measured:AD104,580.88",
            "measured:AD104,580.88,yesterday",
            "measured:,580.88,2026-10-07",
            "ogkm-580.65.06:",
            "ogkm-x.y:nvos.h",
            "host-derived:",
        ] {
            assert!(classify(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn a_measured_row_without_a_trace_line_is_refused() {
        let row = Row {
            item: "x",
            source: "measured:AD104,580.88,2026-10-07",
            evidence: &[],
        };
        assert!(check_row(&row).unwrap_err().contains("no trace line"));
        let ok = Row {
            evidence: &[Evidence {
                at: "traces/a.log:3",
                needle: "n",
            }],
            ..row
        };
        assert_eq!(check_row(&ok), Ok(Kind::Measured));
        let bad_line = Row {
            evidence: &[Evidence {
                at: "traces/a.log:0",
                needle: "n",
            }],
            ..row
        };
        assert!(check_row(&bad_line).is_err());
    }

    #[test]
    fn rows_must_cover_each_item_exactly_once() {
        let r = |item| Row {
            item,
            source: "ogkm-580.65.06:a.h:1",
            evidence: &[],
        };
        assert!(check_rows(&[r("a"), r("b")], &["a", "b"]).is_ok());
        assert!(check_rows(&[r("a")], &["a", "b"]).is_err());
        assert!(check_rows(&[r("a"), r("a")], &["a"]).is_err());
        assert!(check_rows(&[r("a"), r("c")], &["a"]).is_err());
    }
}
