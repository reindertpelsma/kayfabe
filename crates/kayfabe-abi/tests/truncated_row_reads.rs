//! ★★★ The half of the truncated-capture question that a census cannot answer: **not which
//! rows are short, but whether anything in this tree READS the part that is short.**
//!
//! # Why this file exists
//!
//! `kayfabe_abi::oracle` established that 16 of the C artifact's 56 captured control rows
//! keep only a prefix, and `captured_row_evidence` refuses the whole class. That is a
//! statement about **the capture**. It is silent on the thing that actually cost four rungs
//! on `0x20802a08`: a consumer reading a field the capture does not contain, getting a zero
//! for it, and no test being able to notice — because the argument that the read was inside
//! the prefix existed only as prose, and *prose cannot fail a build*.
//!
//! `oracle::CAPTURE_RELIANCE` is that argument as data: one row per truncated control this
//! tree references, stating the deepest byte of the capture the argument rests on. This file
//! checks it, and — the part that matters — **derives the set of controls that need a row**
//! instead of trusting the list's own length.
//!
//! # ⊘ What this file does NOT establish
//!
//! - ⊘ It does not say the captured bytes are **correct**. `field_is_captured` returning
//!   `true` is the narrower claim that a byte came off the recorder rather than out of
//!   zero-fill.
//! - ⊘ It does not say the uncaptured tails are zeros. No hardware body has been taken for
//!   any of the sixteen; the tails remain unmeasured, which is exactly why reading them
//!   would be fabrication.
//! - ⊘ The reference scan is **lexical**. It finds a control that is named; it cannot find
//!   one reached only through a computed id. That limit is stated rather than papered over,
//!   and it is why `read_end` carries a `why` a human wrote.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use kayfabe_abi::oracle::{CAPTURE_RELIANCE, TRUNCATED_ROWS, capture_reliance, field_is_captured};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Every file with `ext` under `roots`, as (repo-relative path, contents).
///
/// ⚠ A walk, not a list. A hand-written set of files is a smaller universe that shrinks
/// silently — the failure `gates_quantified_over_a_list` records — and the point of this
/// file is the universe.
fn walk(roots: &[&str], ext: &str) -> Vec<(String, String)> {
    let root = repo_root();
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = roots.iter().map(|r| root.join(r)).collect();
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                // ⊘ Build output only. Everything else is in scope, including a directory
                // added tomorrow, so nothing escapes the gate by being new.
                if name != "target" && !name.starts_with('.') {
                    stack.push(p);
                }
            } else if name.ends_with(ext) {
                let rel = p
                    .strip_prefix(&root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .to_string();
                out.push((rel, fs::read_to_string(&p).unwrap_or_default()));
            }
        }
    }
    out
}

fn rust_sources() -> Vec<(String, String)> {
    let out = walk(&["crates", "tests"], ".rs");
    assert!(
        out.len() > 100,
        "the source walk found only {} files — it is not walking the tree, and a gate that \
         quantifies over nothing passes vacuously",
        out.len()
    );
    out
}

/// The two files that describe this defect class rather than consume the capture.
///
/// Both name truncated controls in their prose — including one that exists in this tree
/// **only** as an example of a miscitation — so counting them as consumers would make the
/// gate assert against its own documentation. ⊘ Two named files, not a pattern that can
/// grow.
fn is_self_referential(path: &str) -> bool {
    path.ends_with("crates/kayfabe-abi/src/oracle.rs")
        || path.ends_with("crates/kayfabe-abi/tests/truncated_row_reads.rs")
}

/// The forms a control id is written in here: `0x2080_0a40`, `0x20800a40`, `ctl_20800a40`,
/// and a citation to the row's own line in the C header.
fn mentions(text: &str, cmd: u32, c_line: u32) -> bool {
    let hi = cmd >> 16;
    let lo = cmd & 0xffff;
    text.contains(&format!("0x{hi:04x}_{lo:04x}"))
        || text.contains(&format!("0x{cmd:08x}"))
        || text.contains(&format!("ctl_{cmd:08x}"))
        || text.contains(&format!("ga106.h:{c_line}"))
}

/// ★★★ The anti-shrink gate. The set of truncated rows this tree references is **computed**,
/// and `CAPTURE_RELIANCE` must equal it exactly.
///
/// Deleting a row weakens nothing silently: the derived set still contains its control and
/// this goes red. Adding a consumer for a seventeenth row goes red too, with the control
/// named. ★ Running this derivation for the first time is what found `0x20800b03` in the
/// universe — referenced only by a **miscited line number**, since corrected.
#[test]
fn the_set_of_truncated_rows_this_tree_references_is_exactly_the_set_it_argues_about() {
    let sources = rust_sources();
    let declared: BTreeSet<u32> = CAPTURE_RELIANCE.iter().map(|r| r.cmd).collect();

    let mut referenced: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for row in TRUNCATED_ROWS {
        for (path, text) in &sources {
            if is_self_referential(path) {
                continue;
            }
            if mentions(text, row.cmd, row.c_line) {
                referenced.entry(row.cmd).or_default().push(path.clone());
            }
        }
    }
    let referenced_ids: BTreeSet<u32> = referenced.keys().copied().collect();

    let missing: Vec<String> = referenced_ids
        .difference(&declared)
        .map(|c| format!("{c:#010x} (referenced by {:?})", referenced[c]))
        .collect();
    assert!(
        missing.is_empty(),
        "these truncated rows are referenced but carry NO reliance statement — every one is \
         a place where a read could be landing past what the recorder kept and nothing \
         would say so: {missing:#?}"
    );
    let stale: Vec<String> = declared
        .difference(&referenced_ids)
        .map(|c| format!("{c:#010x}"))
        .collect();
    assert!(
        stale.is_empty(),
        "these reliance statements are about controls nothing references any more — a \
         statement nobody depends on is one nobody will keep true: {stale:#?}"
    );
    // ⊘ …and the derivation must not be vacuous. If the scan matched nothing the two sets
    // would be trivially equal and this file would pass having checked nothing.
    assert!(
        referenced_ids.len() >= 9,
        "the reference scan found only {} truncated rows in use; it found nine when it was \
         written, so a smaller number means the scan broke, not that the tree shrank",
        referenced_ids.len()
    );
}

/// Every reliance statement's `sites` must exist and must actually name the control.
///
/// ⊘ Without this a site is a sentence: a path that no longer exists, or one that never
/// mentioned the control, would keep asserting that somebody had thought about it.
#[test]
fn every_declared_site_exists_and_names_the_control_it_claims_to_argue_about() {
    let root = repo_root();
    for r in CAPTURE_RELIANCE {
        let row = TRUNCATED_ROWS
            .iter()
            .find(|t| t.cmd == r.cmd)
            .expect("a truncated row");
        for site in r.sites {
            let p = root.join(site);
            let text = fs::read_to_string(&p)
                .unwrap_or_else(|e| panic!("{:#010x}: site {site} unreadable: {e}", r.cmd));
            assert!(
                mentions(&text, r.cmd, row.c_line),
                "{:#010x}: site {site} does not name this control",
                r.cmd
            );
        }
    }
}

/// The reliance itself, checked from outside the crate that declares it.
#[test]
fn no_declared_reliance_reaches_past_the_captured_prefix() {
    for r in CAPTURE_RELIANCE {
        let row = TRUNCATED_ROWS
            .iter()
            .find(|t| t.cmd == r.cmd)
            .expect("a truncated row");
        assert!(
            field_is_captured(0, r.read_end, row.kept),
            "{:#010x}: relies on [0,{}) but only {} of {} bytes were captured",
            r.cmd,
            r.read_end,
            row.kept,
            row.psize
        );
    }
    // ★ And the accessor agrees with the table, so a caller reaching for a decision gets one.
    for row in TRUNCATED_ROWS {
        let declared = CAPTURE_RELIANCE.iter().any(|r| r.cmd == row.cmd);
        assert_eq!(
            capture_reliance(row.cmd).is_some(),
            declared,
            "{:#010x}",
            row.cmd
        );
    }
}

// ── The citation half: an address is not a source ──────────────────────────────────────

/// The committed census, keyed by the line each row sits on in the C header.
fn census_by_line() -> BTreeMap<u32, (u32, usize, usize, String)> {
    let p = repo_root().join("traces/c_oracle_census/initctrl_ga106_census.tsv");
    let text = fs::read_to_string(&p).unwrap_or_else(|e| panic!("census {p:?} unreadable: {e}"));
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(f.len(), 9, "census row has 9 fields: {line}");
        let cmd = u32::from_str_radix(f[0].trim_start_matches("0x"), 16).expect("cmd");
        out.insert(
            f[6].parse::<u32>().expect("c_line"),
            (
                cmd,
                f[2].parse().expect("psize"),
                f[3].parse().expect("dlen"),
                f[7].to_string(),
            ),
        );
    }
    out
}

/// ★★★ **Citing the oracle is not the oracle being right — and here it is the address that
/// is wrong.**
///
/// The memory of that name records a gate satisfied by a row that cited an *empty* body as
/// corroboration. `[measured]` 2026-08-02, the same failure arrives from the other side:
/// three claims in this tree cited a `mode2_initctrl_ga106.h` line belonging to a
/// **different control**, and two of those lines were truncated rows. Every **value** was
/// correct, every **address** wrong, and the `C:`-citation gate satisfied throughout. This
/// test resolves the address instead of counting it.
///
/// ★★★ **WIDENED 2026-08-06 to EVERY census row, and the silent skip is GONE (Q20 = (a)).**
///
/// It used to judge only citations to a **truncated** row, and only when the citing line named
/// a control — `if named.is_empty() { continue; }`. That skip is the whole problem: an
/// unattributable citation *looked* like a pass. `[measured]` at the time of widening there
/// were **69** `ga106.h:<line>` citations in the tree, **37** naming no control, and **0** of
/// those 37 cited a truncated row — so the old gate's scope contained **nothing it skipped**,
/// and every unattributable citation in the repo sat entirely outside it.
///
/// ★ All **23** whose cited line is a real census row were adjudicated by hand before the
/// widening and **none was a miscitation** — every claim was about the control at the line it
/// cited; the subject simply sat on an *adjacent* line (a `cmd:` field, a preceding sentence)
/// rather than the citing one. The control id was then written onto each citing line, which
/// records an adjudication that was actually performed. ⊘ Stamping the census value onto a
/// citation *without* reading it would launder a miscitation into compliance — it would make
/// this gate green **by construction**, which is the failure mode the gate exists to catch.
///
/// ⊘ Still out of scope, and stated rather than fixed: **14** citations name a line that is
/// not a census row at all — the header banner (`:1-2`) or a body array (`:3346`, `:3878`).
/// Those are legitimate citations the census cannot resolve, because it indexes registration
/// lines. Widening to them needs a body-line index, not a rule change.
#[test]
fn every_citation_to_a_census_rows_line_names_the_control_that_line_carries() {
    let census = census_by_line();
    let truncated_lines: BTreeMap<u32, u32> = census
        .iter()
        .filter(|(_, v)| v.3 == "truncated")
        .map(|(line, v)| (*line, v.0))
        .collect();
    assert_eq!(
        truncated_lines.len(),
        TRUNCATED_ROWS.len(),
        "the census and the Rust table must agree on how many rows are truncated"
    );
    // ★ THE WIDENING: every census row, not just the truncated ones.
    let cited_lines: BTreeMap<u32, u32> = census.iter().map(|(l, v)| (*l, v.0)).collect();

    let mut sources = rust_sources();
    sources.extend(walk(&["docs", "notes"], ".md"));

    let mut checked = 0usize;
    let mut wrong: Vec<String> = Vec::new();
    for (path, text) in &sources {
        if is_self_referential(path) {
            continue;
        }
        for line in text.lines() {
            for (cited_line, owner) in &cited_lines {
                if !line.contains(&format!("ga106.h:{cited_line}")) {
                    continue;
                }
                // Attribution: the citing line must name a control. If it names none, the
                // citation is unattributable and this test cannot judge it.
                let named: Vec<u32> = census
                    .values()
                    .map(|v| v.0)
                    .filter(|c| {
                        let (hi, lo) = (c >> 16, c & 0xffff);
                        line.contains(&format!("0x{hi:04x}_{lo:04x}"))
                            || line.contains(&format!("0x{c:08x}"))
                            || line.contains(&format!("ctl_{c:08x}"))
                    })
                    .collect();
                if named.is_empty() {
                    // ⊘ NOT a skip. An unattributable citation is exactly what Q20 decided
                    // against: the reader cannot tell which control the claim is about, so
                    // the gate cannot tell either, and "cannot tell" must never read as
                    // "checked". Name the control on the citing line — after confirming the
                    // claim really is about the row at that line.
                    wrong.push(format!(
                        "{path}: cites ga106.h:{cited_line} ({owner:#010x}) but names NO                          control on the citing line — unattributable. Add the control id to                          this line once you have CONFIRMED the claim is about that row"
                    ));
                    continue;
                }
                checked += 1;
                if !named.contains(owner) {
                    wrong.push(format!(
                        "{path}: cites ga106.h:{cited_line}, which is {owner:#010x}, but the \
                         line names {named:#010x?} — an address that is not the source the \
                         claim means"
                    ));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "miscited oracle rows:\n{}",
        wrong.join("\n")
    );
    // ⊘ A floor, because a gate that checks nothing reports clean. `[measured]` 2026-08-06,
    // widened to every census row: **42** attributable citations resolve.
    //
    // ⚠ The first version of this line said 55, and 55 was a GUESS I wrote into an assertion
    // message as though it were a measurement. The gate caught it by failing. A number in a
    // failure message is a claim like any other — this one is now the count the run produced,
    // and it is exact rather than padded, matching the `>= 5` the truncated-only floor used.
    assert!(
        checked >= 42,
        "only {checked} attributable citations to a census row were resolved — the widened \
         scan found 42, so a smaller number is the instrument breaking rather than the tree \
         improving. Raise this floor when citations are ADDED; never lower it to go green"
    );
}
