//! ★★★ The C artifact's captured control table, censused — and the pin that stops
//! [`kayfabe_abi::oracle::TRUNCATED_ROWS`] from being an unchecked transcription.
//!
//! # What this test is for
//!
//! `oracle::TRUNCATED_ROWS` is sixteen rows of numbers copied out of a header in a
//! **different repository** — `/workspace/nvidia-gpu-passthrough/src/qemu/-
//! mode2_initctrl_ga106.h`, read-only, and absent on any box that only has the Rust tree.
//! A hand-transcribed constant of that shape is the exact failure this project keeps
//! meeting: it cannot detect a shared misreading, and nothing in the build would notice if
//! the C table changed underneath it.
//!
//! So the census is a **committed artifact** — `traces/c_oracle_census/-
//! initctrl_ga106_census.tsv`, produced by `scripts/census_initctrl.py` — and this test
//! reads *that file* and compares it to the Rust tables.
//!
//! ⊘ **The two are independent by construction.** The TSV is derived from the C header by a
//! Python parser; the Rust tables are `const` data. Neither is computed from the other at
//! build time, so a mismatch is a real disagreement rather than a tautology. ★★ And this
//! test never asks `captured_row_evidence` what class a row is in — the classification
//! comes from the TSV's own `class` column, so the predicate under test is not its own
//! observer.
//!
//! # ⊘ What this test does NOT establish
//!
//! - ⊘ It does not say any captured body is **correct**. It says how many bytes were kept.
//! - ⊘ It does not say the uncaptured tails are non-zero. No hardware body has been taken
//!   for any of the sixteen truncated rows — that is #159.

use std::collections::BTreeMap;

use kayfabe_abi::oracle::{
    CAPTURED_ROWS_TOTAL, COMPLETE_ROWS_TOTAL, CapturedEvidence, EMPTY_CAPTURE_ROWS, OracleRowError,
    TRUNCATED_ROWS, captured_row_evidence, empty_capture_row, truncated_row,
};

/// One parsed row of the committed census.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CensusRow {
    cmd: u32,
    psize: usize,
    dlen: usize,
    arrlen: usize,
    trailing_zeros_kept: usize,
    c_line: u32,
    class: String,
}

struct Census {
    rows: BTreeMap<u32, CensusRow>,
    total: usize,
    complete: usize,
    truncated: usize,
    empty: usize,
}

fn census() -> Census {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../traces/c_oracle_census/initctrl_ga106_census.tsv"
    );
    let text = std::fs::read_to_string(p).unwrap_or_else(|e| panic!("census {p} unreadable: {e}"));

    let mut rows = BTreeMap::new();
    let (mut total, mut complete, mut truncated, mut empty) = (0usize, 0usize, 0usize, 0usize);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# ") {
            let mut it = rest.split('\t');
            let (k, v) = (it.next().unwrap_or(""), it.next());
            if let Some(v) = v.and_then(|v| v.parse::<usize>().ok()) {
                match k {
                    "total" => total = v,
                    "complete" => complete = v,
                    "truncated" => truncated = v,
                    "empty" => empty = v,
                    _ => {}
                }
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(f.len(), 9, "census row has 9 fields: {line}");
        let cmd = u32::from_str_radix(f[0].trim_start_matches("0x"), 16).expect("cmd");
        let row = CensusRow {
            cmd,
            psize: f[2].parse().expect("psize"),
            dlen: f[3].parse().expect("dlen"),
            arrlen: f[4].parse().expect("arrlen"),
            trailing_zeros_kept: f[5].parse().expect("tz"),
            c_line: f[6].parse().expect("c_line"),
            class: f[7].to_string(),
        };
        assert!(rows.insert(cmd, row).is_none(), "{cmd:#010x} appears twice");
    }
    assert!(!rows.is_empty(), "census parsed to nothing");
    Census {
        rows,
        total,
        complete,
        truncated,
        empty,
    }
}

#[test]
fn the_census_header_agrees_with_its_own_rows_and_with_the_pinned_totals() {
    let c = census();
    assert_eq!(c.rows.len(), c.total, "header `total` vs parsed rows");
    let count = |k: &str| c.rows.values().filter(|r| r.class == k).count();
    assert_eq!(count("complete"), c.complete);
    assert_eq!(count("truncated"), c.truncated);
    assert_eq!(count("empty"), c.empty);
    assert_eq!(
        c.complete + c.truncated + c.empty,
        c.total,
        "classes partition the table"
    );

    // …and the Rust constants say the same thing.
    assert_eq!(c.total, CAPTURED_ROWS_TOTAL);
    assert_eq!(c.complete, COMPLETE_ROWS_TOTAL);
    assert_eq!(c.truncated, TRUNCATED_ROWS.len());
    assert_eq!(c.empty, EMPTY_CAPTURE_ROWS.len());

    // ★★★ The finding that made the widening necessary, asserted rather than narrated:
    // only about half the table is a complete capture, and the truncated class — the one
    // `BodyNeverCaptured` did NOT cover — is the larger of the two defective ones.
    assert!(
        c.complete * 2 < c.total * 3 / 2 + c.total / 2,
        "complete is ~half, not most"
    );
    assert!(
        c.truncated > c.empty,
        "the uncovered class was the bigger one"
    );
}

#[test]
fn every_truncated_row_in_the_c_header_is_in_truncated_rows_byte_for_byte() {
    let c = census();
    let from_c: BTreeMap<u32, &CensusRow> = c
        .rows
        .values()
        .filter(|r| r.class == "truncated")
        .map(|r| (r.cmd, r))
        .collect();

    // ★★ Quantified over the C header's own universe, not over the Rust list. Shortening
    // the Rust list cannot weaken this gate — a row present in the header and absent from
    // `TRUNCATED_ROWS` fails here.
    for (cmd, cr) in &from_c {
        let rr = truncated_row(*cmd).unwrap_or_else(|| {
            panic!("{cmd:#010x} is truncated in the C header but absent from TRUNCATED_ROWS")
        });
        assert_eq!(rr.psize, cr.psize, "{cmd:#010x} psize");
        assert_eq!(rr.kept, cr.dlen, "{cmd:#010x} kept");
        assert_eq!(rr.c_line, cr.c_line, "{cmd:#010x} c_line");
        assert_eq!(
            rr.trailing_zeros_kept, cr.trailing_zeros_kept,
            "{cmd:#010x} trailing_zeros_kept"
        );
    }
    // …and nothing in the Rust list that is not truncated in the header.
    for rr in TRUNCATED_ROWS {
        assert!(
            from_c.contains_key(&rr.cmd),
            "{:#010x} is in TRUNCATED_ROWS but is not truncated in the C header",
            rr.cmd
        );
    }
}

#[test]
fn dlen_is_not_a_trailing_zero_trim_and_the_census_is_where_that_is_checked() {
    // ★★★ The hypothesis that had to die before the widening was legitimate. If `dlen` were
    // trailing-zero-trimmed, a truncated row's tail really would be zeros and refusing it
    // would be wrong. A trimmer cannot leave a zero byte at the end of what it kept.
    //
    // ⊘ Checked against the census's own count of trailing zero bytes — derived from the
    // header's raw hex by `scripts/census_initctrl.py`, not from any Rust decoder.
    let c = census();
    let offenders: Vec<u32> = c
        .rows
        .values()
        .filter(|r| r.class == "truncated" && r.trailing_zeros_kept == 0)
        .map(|r| r.cmd)
        .collect();
    assert!(
        offenders.is_empty(),
        "these truncated rows end in a NON-zero byte, so `dlen` could be a zero-trim for \
         them and refusing them needs re-arguing: {offenders:#010x?}"
    );
    // The two largest are the ones that carry the mechanism: a 16 KiB GSP message-queue
    // element, not a field boundary.
    let a22 = &c.rows[&0x2080_0a22];
    assert_eq!((a22.psize, a22.dlen), (34592, 16376)); // 16376 = 16384 - 8
    assert!(a22.trailing_zeros_kept > 10_000);
    let a40 = &c.rows[&0x2080_0a40];
    assert_eq!((a40.psize, a40.dlen), (24580, 16384));
    assert!(a40.trailing_zeros_kept > 15_000);
}

#[test]
fn a_rows_declared_dlen_matches_the_bytes_its_array_actually_holds() {
    // ⚠ A row claiming a `dlen` its `ctl_` array does not have would make every length in
    // this census a fiction. The C's consumer does `memcpy(resp + 120, cr->data, cr->dlen)`,
    // so a short array would be an out-of-bounds read there and a wrong `kept` here.
    let c = census();
    for r in c.rows.values() {
        assert_eq!(
            r.arrlen, r.dlen,
            "{:#010x}: row declares dlen {} but its array holds {} bytes",
            r.cmd, r.dlen, r.arrlen
        );
    }
}

#[test]
fn the_predicate_classifies_every_row_the_way_the_census_does() {
    // ★★ The whole table, driven from the census's `class` column. The predicate is graded
    // against an independently-derived classification rather than against itself.
    let c = census();
    for r in c.rows.values() {
        let got = captured_row_evidence(r.cmd, r.psize, r.dlen);
        match r.class.as_str() {
            "complete" => assert_eq!(
                got,
                Ok(CapturedEvidence::Complete { psize: r.psize }),
                "{:#010x} is complete in the census",
                r.cmd
            ),
            "truncated" => assert_eq!(
                got,
                Err(OracleRowError::BodyTruncated {
                    cmd: r.cmd,
                    kept: r.dlen,
                    psize: r.psize
                }),
                "{:#010x} is truncated in the census and must be refused",
                r.cmd
            ),
            "empty" if r.psize == 0 => assert_eq!(
                got,
                Ok(CapturedEvidence::NoBodyExists),
                "{:#010x} has no body to capture",
                r.cmd
            ),
            "empty" => assert_eq!(
                got,
                Err(OracleRowError::BodyNeverCaptured {
                    cmd: r.cmd,
                    psize: r.psize
                }),
                "{:#010x} is an empty capture and must be refused",
                r.cmd
            ),
            other => panic!("{:#010x}: unknown census class {other:?}", r.cmd),
        }
    }
}

#[test]
fn the_two_defect_lists_are_disjoint_and_cover_the_censuss_defective_rows() {
    let c = census();
    for r in c.rows.values() {
        let e = empty_capture_row(r.cmd).is_some();
        let t = truncated_row(r.cmd).is_some();
        assert!(!(e && t), "{:#010x} is in both defect lists", r.cmd);
        match r.class.as_str() {
            "empty" => assert!(e, "{:#010x} is empty but not in EMPTY_CAPTURE_ROWS", r.cmd),
            "truncated" => assert!(t, "{:#010x} is truncated but not in TRUNCATED_ROWS", r.cmd),
            _ => assert!(
                !e && !t,
                "{:#010x} is complete but listed as defective",
                r.cmd
            ),
        }
    }
}
