//! ★★★ The generated driver matrix IS the committed measurement (`V3_DRIVER_MATRIX.md` §3–§4).
//!
//! `crates/kf-abi/src/generated/matrix.rs` is emitted from `traces/driver_matrix/ranges.tsv` by
//! `tools/drivermatrix/emit_rust.py`. This test re-reads the TSV and checks every row against
//! the Rust tables, so a hand edit to either side — a "fixed" offset, a dropped run, a tag added
//! to one and not the other — is a red test, not a silently different answer.

use kf_abi::DriverVersion;
use kf_abi::generated::matrix::{ALL_STRUCTS, ALL_VALUES, MEASURED};

const RANGES: &str = include_str!("../../../traces/driver_matrix/ranges.tsv");

fn parse_tag(t: &str) -> DriverVersion {
    DriverVersion::parse(t).unwrap_or_else(|| panic!("ranges.tsv names a non-version {t:?}"))
}

fn tsv_tags() -> Vec<DriverVersion> {
    let line = RANGES
        .lines()
        .find(|l| l.starts_with("# tags\t"))
        .expect("ranges.tsv has a '# tags' line");
    let mut v: Vec<_> = line
        .split('\t')
        .nth(1)
        .expect("tags")
        .split_whitespace()
        .map(parse_tag)
        .collect();
    v.sort();
    v
}

#[test]
fn the_measured_tag_list_is_the_tsvs() {
    assert_eq!(
        MEASURED,
        tsv_tags().as_slice(),
        "MEASURED and ranges.tsv disagree on the tag set"
    );
}

#[test]
fn every_tsv_row_is_what_the_rust_tables_answer() {
    let tags = tsv_tags();
    let mut checked = 0usize;
    for line in RANGES
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 5, "malformed row {line:?}");
        let (kind, item, first, last, value) = (
            cols[0],
            cols[1],
            parse_tag(cols[2]),
            parse_tag(cols[3]),
            cols[4],
        );
        let span: Vec<DriverVersion> = tags
            .iter()
            .copied()
            .filter(|t| first <= *t && *t <= last)
            .collect();
        assert!(!span.is_empty(), "row {line:?} covers no measured tag");
        match kind {
            "value" => {
                let t = ALL_VALUES
                    .iter()
                    .find(|v| v.name == item)
                    .unwrap_or_else(|| panic!("no Rust table for {item}"));
                let want = if value == "ABSENT" {
                    None
                } else {
                    Some(value.parse::<u64>().expect("integer"))
                };
                for v in &span {
                    assert_eq!(t.at(*v).expect("measured"), want, "{item} at {v}");
                    checked += 1;
                }
            }
            "sizeof" | "field" => {
                let (s, path) = match item.split_once('.') {
                    Some((s, p)) if kind == "field" => (s, Some(p)),
                    _ => (item, None),
                };
                let t = ALL_STRUCTS
                    .iter()
                    .find(|x| x.name == s)
                    .unwrap_or_else(|| panic!("no Rust table for {s}"));
                for v in &span {
                    let lay = t.at(*v).expect("measured");
                    match (path, value) {
                        (None, "ABSENT") => assert!(lay.is_none(), "{s} should be absent at {v}"),
                        (None, sz) => {
                            let size: u32 =
                                sz.split('+').nth(1).expect("0+SIZE").parse().expect("int");
                            assert_eq!(lay.expect("present").size, size, "sizeof {s} at {v}");
                        }
                        (Some(p), "ABSENT") => {
                            assert!(
                                lay.is_none_or(|l| l.field(p).is_none()),
                                "{s}.{p} should be absent at {v}"
                            );
                        }
                        (Some(p), at) => {
                            let (off, size) = at.split_once('+').expect("OFF+SIZE");
                            let f = lay
                                .and_then(|l| l.field(p))
                                .unwrap_or_else(|| panic!("{s}.{p} missing at {v}"));
                            assert_eq!(
                                (f.off, f.size),
                                (off.parse().expect("off"), size.parse().expect("size")),
                                "{s}.{p} at {v}"
                            );
                        }
                    }
                    checked += 1;
                }
            }
            other => panic!("unknown row kind {other:?}"),
        }
    }
    assert!(
        checked > 1000,
        "the TSV was too small to be the committed matrix ({checked} cells)"
    );
}

/// ★ The bench driver is measured — the default guest AND host version must resolve.
#[test]
fn the_bench_driver_is_measured() {
    assert!(kf_abi::matrix::is_measured(kf_abi::versions::BENCH_DRIVER));
}
