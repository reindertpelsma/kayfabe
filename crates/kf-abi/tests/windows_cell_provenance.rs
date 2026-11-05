//! ★ 2026-10-31 (provenance fix): the Windows-cell tables (`kf_abi::hoststub`,
//! `kf_abi::sw_runlist`) may carry only values with a public source. This test is the CI gate:
//! every row has a source tag of an accepted kind (`ogkm-<version>:<ref>`, `measured:...`,
//! `host-derived:...`; a `binary`/`re`/`blob` origin is refused), and every trace line a
//! `measured:` row cites exists in this tree and shows what the row says it shows.
//! Rules: `docs/design/V3_PROVENANCE.md` sections 1 and 2.

use kf_abi::provenance::{Evidence, Kind, Row, check_row, check_rows};
use kf_abi::{DriverVersion, hoststub, sw_runlist};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn windows_wire() -> DriverVersion {
    DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    }
}

fn all_rows() -> Vec<(String, &'static [Row])> {
    let mut out = Vec::new();
    let h = hoststub::cell(windows_wire()).expect("hoststub cell");
    for c in h.controls {
        out.push((format!("hoststub {:#x}", c.cmd), c.rows));
    }
    let s = sw_runlist::cell(windows_wire()).expect("sw_runlist cell");
    out.push(("sw_runlist".to_string(), s.rows));
    for (name, rows) in sw_runlist::native_cell_rows() {
        out.push((format!("sw_runlist native {name}"), rows));
    }
    out
}

#[test]
fn every_row_of_every_windows_cell_has_an_accepted_source_tag() {
    let h = hoststub::cell(windows_wire()).unwrap();
    for c in h.controls {
        check_rows(c.rows, hoststub::CONTROL_ITEMS).unwrap_or_else(|e| panic!("{:#x}: {e}", c.cmd));
    }
    let s = sw_runlist::cell(windows_wire()).unwrap();
    check_rows(s.rows, sw_runlist::ITEMS).unwrap_or_else(|e| panic!("sw_runlist: {e}"));
    // 2026-11-01: the measured allocation-only cell merged from the Blackwell branch follows the same rule.
    let native: Vec<_> = sw_runlist::native_cell_rows().collect();
    assert!(
        !native.is_empty(),
        "the measured allocation-only cell is gone"
    );
    for (name, rows) in native {
        check_rows(rows, sw_runlist::NATIVE_ITEMS)
            .unwrap_or_else(|e| panic!("sw_runlist native {name}: {e}"));
    }
    let rows = all_rows();
    assert!(
        rows.iter().map(|(_, r)| r.len()).sum::<usize>() >= 8,
        "the tables lost rows"
    );
    for (name, rs) in rows {
        for r in rs {
            let kind = check_row(r).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(matches!(
                kind,
                Kind::Ogkm | Kind::Measured | Kind::HostDerived
            ));
        }
    }
}

fn line_of(e: &Evidence) -> String {
    let (path, line) = e.at.rsplit_once(':').expect("path:line");
    let line: usize = line.parse().expect("line number");
    let text = std::fs::read(root().join(path)).unwrap_or_else(|err| panic!("{}: {err}", e.at));
    let text = String::from_utf8_lossy(&text);
    text.lines()
        .nth(line - 1)
        .unwrap_or_else(|| panic!("{}: the file has fewer lines", e.at))
        .to_string()
}

#[test]
fn every_cited_trace_line_exists_and_shows_what_the_row_says() {
    let mut cited = 0;
    for (name, rs) in all_rows() {
        for r in rs {
            for e in r.evidence {
                let l = line_of(e);
                assert!(
                    l.contains(e.needle),
                    "{name} / {}: {} does not contain {:?}: {l:?}",
                    r.item,
                    e.at,
                    e.needle
                );
                cited += 1;
            }
        }
    }
    assert!(cited >= 8, "only {cited} trace lines are cited");
}

/// The evidence must not itself be a closed-binary reading: a cited line may not carry an export
/// flag word, a file offset or the like. (The earlier tables cited such values; the traces that
/// still contain them are not cited.)
#[test]
fn no_cited_line_carries_a_closed_binary_reading() {
    for (name, rs) in all_rows() {
        for r in rs {
            for e in r.evidence {
                let l = line_of(e).to_ascii_lowercase();
                let word = |w: &str| {
                    l.split(|c: char| !c.is_ascii_alphanumeric())
                        .any(|x| x == w)
                };
                assert!(
                    !l.contains("retail") && !l.contains("export row") && !word("rva"),
                    "{name} / {}: {} carries a closed-binary reading",
                    r.item,
                    e.at
                );
            }
        }
    }
}
