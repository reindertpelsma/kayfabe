//! CODE READ: data-only loader contract for CW1; source bc3a967e0 consttab.rs.

use kf_chip::{DieId, arch};
use kf_rm::consttab::{self, Refusal, SourceTag};
use std::path::PathBuf;

const HEADER: &str = "key\tdie\tbranch\tsize\tinputs\twords\tsource\tevidence\n";
const GA106: DieId = DieId::new(arch::GA100, 6);
const AD104: DieId = DieId::new(arch::AD100, 4);
const ROW: &str =
    "rm.test\tGA106\t580\t4\t0x0=0x1\t0x2\tmeasured:GA106,580.159.04,2026-10-14\ttraces/test.txt\n";

fn parse_row(row: &str) -> Result<Vec<consttab::Row>, String> {
    consttab::parse(&format!("{HEADER}{row}"))
}

#[test]
fn published_rows_validate_with_existing_evidence() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rows = consttab::parse(consttab::TABLE_TSV).expect("observation table");
    assert_eq!(rows.len(), 95);
    consttab::validate_evidence(&rows, |path| repo.join(path).exists())
        .expect("every citation exists");
    for row in rows {
        assert!(matches!(row.source, SourceTag::Measured { .. }));
    }
}

#[test]
fn missing_evidence_is_rejected_by_validator() {
    let rows = parse_row(ROW).unwrap();
    assert!(
        consttab::validate_evidence(&rows, |_| false)
            .unwrap_err()
            .contains("evidence")
    );
}

#[test]
fn evidence_checker_receives_only_repo_path_not_annotation() {
    let rows = parse_row(&ROW.replace("traces/test.txt", "traces/test.txt:85 (capture)")).unwrap();
    assert!(consttab::validate_evidence(&rows, |path| path == "traces/test.txt").is_ok());
    assert!(consttab::validate_evidence(&rows, |path| path == "different.txt").is_err());
}

#[test]
fn empty_evidence_is_rejected() {
    assert!(
        parse_row(&ROW.replace("traces/test.txt", ""))
            .unwrap_err()
            .contains("evidence")
    );
}

#[test]
fn whitespace_evidence_is_rejected() {
    assert!(
        parse_row(&ROW.replace("traces/test.txt", "   "))
            .unwrap_err()
            .contains("evidence")
    );
}

#[test]
fn absolute_evidence_path_is_rejected() {
    assert!(parse_row(&ROW.replace("traces/test.txt", "/tmp/test.txt")).is_err());
}

#[test]
fn parent_traversal_evidence_is_rejected() {
    assert!(parse_row(&ROW.replace("traces/test.txt", "traces/../../test.txt")).is_err());
}

#[test]
fn invalid_month_is_rejected() {
    assert!(
        parse_row(&ROW.replace("2026-10-14", "2026-13-14"))
            .unwrap_err()
            .contains("date")
    );
}

#[test]
fn invalid_day_is_rejected() {
    assert!(
        parse_row(&ROW.replace("2026-10-14", "2026-02-30"))
            .unwrap_err()
            .contains("date")
    );
    assert!(parse_row(&ROW.replace("2026-10-14", "2025-02-29")).is_err());
    assert!(parse_row(&ROW.replace("2026-10-14", "2024-02-29")).is_ok());
}

#[test]
fn tag_must_name_the_row_die() {
    assert!(parse_row(&ROW.replace("measured:GA106", "measured:AD104")).is_err());
}

#[test]
fn tag_must_name_the_row_driver_branch() {
    assert!(parse_row(&ROW.replace(",580.159.04,", ",595.91.07,")).is_err());
}

#[test]
fn measured_die_wildcard_is_rejected() {
    assert!(parse_row(&ROW.replace("\tGA106\t", "\t*\t")).is_err());
}

#[test]
fn measured_rm_branch_wildcard_is_rejected() {
    assert!(parse_row(&ROW.replace("\t580\t", "\t*\t")).is_err());
}

#[test]
fn zbc_host_state_keeps_exact_die_and_host_driver_tag() {
    let rows = parse_row(
        &ROW.replace("rm.test", "zbc.color")
            .replace("\t580\t", "\t*\t"),
    )
    .unwrap();
    assert!(consttab::lookup_in(&rows, "zbc.color", GA106, 595).is_ok());
    assert!(consttab::lookup_in(&rows, "zbc.color", AD104, 595).is_err());
}

fn mechanism_rows() -> Vec<consttab::Row> {
    parse_row("rule.test\t*\t*\t4\t-\t0x5\trule:unit fixture software state\tcrates/kf-abi/src/cudartinit.rs\n").unwrap()
}

#[test]
fn unknown_implementation_never_gets_wildcard() {
    let rows = mechanism_rows();
    assert!(consttab::lookup_in(&rows, "rule.test", DieId::new(arch::AD100, 1), 580).is_err());
    assert!(consttab::lookup_in(&rows, "rule.test", GA106, 580).is_ok());
}

#[test]
fn unknown_architecture_never_gets_wildcard() {
    let rows = mechanism_rows();
    assert!(consttab::lookup_in(&rows, "rule.test", DieId::new(0xdead, 6), 580).is_err());
}

#[test]
fn named_gap_is_refused_without_neighbor_row() {
    assert!(matches!(
        consttab::lookup("cudart.0x20809064", DieId::new(arch::GA100, 4), 580),
        Err(Refusal::Unmeasured { .. })
    ));
    assert!(consttab::lookup("zbc.color", DieId::new(arch::TU100, 4), 580).is_err());
}

#[test]
fn other_family_never_borrows_a_measured_row() {
    let rows = parse_row(ROW).unwrap();
    assert!(consttab::lookup_in(&rows, "rm.test", AD104, 580).is_err());
    assert!(consttab::lookup_in(&rows, "rm.test", GA106, 580).is_ok());
}

#[test]
fn other_rm_branch_does_not_match() {
    let rows = parse_row(ROW).unwrap();
    assert!(consttab::lookup_in(&rows, "rm.test", GA106, 595).is_err());
    assert!(consttab::lookup("rm.0x20800a59", GA106, 535).is_err());
}

#[test]
fn changed_request_is_refused_and_counted() {
    let before = consttab::refusal_count();
    assert!(matches!(
        consttab::answer("gss.0x20808162", GA106, 580, &[1]),
        Err(Refusal::OtherRequest { .. })
    ));
    assert!(consttab::refusal_count() > before);
    assert_eq!(
        consttab::answer("gss.0x20808162", GA106, 580, &[0]).unwrap(),
        vec![1]
    );
}

#[test]
fn changed_request_size_is_refused() {
    assert!(matches!(
        consttab::answer("gss.0x20808162", AD104, 595, &[0, 0, 0, 0]),
        Err(Refusal::OtherRequest { .. })
    ));
    assert_eq!(
        consttab::answer("gss.0x20808162", AD104, 595, &[0]).unwrap(),
        vec![0]
    );
}

#[test]
fn board_specific_keys_are_named_refusals() {
    for (key, die) in [
        ("gss.0x20808162", DieId::new(arch::GA100, 2)),
        ("cudart.0x20809001", AD104),
        ("cudart.0x2080a001", AD104),
    ] {
        assert!(matches!(
            consttab::lookup(key, die, 595),
            Err(Refusal::BoardSpecific { .. })
        ));
    }
}

#[test]
fn header_column_names_are_exact() {
    assert!(
        consttab::parse(&format!(
            "{}{ROW}",
            HEADER.replace("die\tbranch", "chip\tbranch")
        ))
        .is_err()
    );
}

#[test]
fn header_column_order_is_exact() {
    assert!(
        consttab::parse(&format!(
            "{}{ROW}",
            HEADER.replace("die\tbranch", "branch\tdie")
        ))
        .is_err()
    );
}

#[test]
fn duplicate_scope_is_rejected() {
    assert!(consttab::parse(&format!("{HEADER}{ROW}{ROW}")).is_err());
}

#[test]
fn reply_cannot_overrun_declared_size() {
    assert!(parse_row(&ROW.replace("\t0x2\t", "\t0x2 0x3\t")).is_err());
}

#[test]
fn board_specific_refusal_does_not_depend_on_guest_branch() {
    for branch in [535, 580, 595, 610] {
        assert!(matches!(
            consttab::lookup("cudart.0x20809001", AD104, branch),
            Err(Refusal::BoardSpecific { .. })
        ));
    }
}
