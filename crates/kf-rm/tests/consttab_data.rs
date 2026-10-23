//! CODE READ: CW1 data extraction contract, before the table exists on master.
//! Source: codex/sweep-dies-20261022 at bc3a967e0, measured_constants.tsv.

use std::path::PathBuf;

fn data() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/measured_constants.tsv");
    std::fs::read_to_string(path).expect("published per-die measurement table must exist")
}

fn records(text: &str) -> Vec<Vec<&str>> {
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .skip(1)
        .map(|line| line.split('\t').collect())
        .collect()
}

#[test]
fn every_observation_has_exact_die_driver_date_and_evidence() {
    let text = data();
    let rows = records(&text);
    assert_eq!(
        rows.len(),
        95,
        "95 observations; software-state rule is separate"
    );
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for row in rows {
        assert_eq!(row.len(), 8);
        assert_ne!(row[1], "*", "a measurement never borrows another die");
        let tag = row[6].strip_prefix("measured:").expect("measurement tag");
        let tag: Vec<_> = tag.split(',').collect();
        assert_eq!(tag.len(), 3);
        assert_eq!(tag[0], row[1]);
        assert!((2..=3).contains(&tag[1].split('.').count()));
        assert_eq!(tag[2].len(), 10);
        let path = row[7].split_whitespace().next().expect("evidence");
        let path = path.split(':').next().expect("path before optional line");
        assert!(repo.join(path).exists(), "missing evidence: {path}");
    }
}

#[test]
fn measured_scope_keeps_the_documented_die_and_board_gaps() {
    let text = data();
    let rows = records(&text);
    for missing in [
        "TU104", "TU117", "GA103", "GA107", "AD107", "GB205", "GB207",
    ] {
        assert!(
            !rows.iter().any(|r| r[1] == missing),
            "unmeasured {missing}"
        );
    }
    for (key, die) in [
        ("gss.0x20808162", "GA102"),
        ("cudart.0x20809001", "AD104"),
        ("cudart.0x2080a001", "AD104"),
    ] {
        assert!(!rows.iter().any(|r| r[0] == key && r[1] == die));
    }
    assert_eq!(rows.iter().filter(|r| r[0].starts_with("zbc.")).count(), 39);
    assert!(!rows.iter().any(|r| r[0] == "cudart.0x20802209"));
}

#[test]
fn ga106_and_ad104_observations_match_existing_oracles() {
    let text = data();
    let rows = records(&text);
    for &(cmd, size, expected) in kf_abi::cudartinit::SERVED {
        if cmd == 0x2080_2209 {
            continue;
        }
        let key = format!("cudart.{cmd:#010x}");
        let row = rows
            .iter()
            .find(|r| r[0] == key && r[1] == "GA106" && r[2] == "580")
            .expect("GA106 captured observation");
        assert_eq!(row[3].parse::<usize>().unwrap(), size);
        let words: Vec<u32> = row[5]
            .split_whitespace()
            .map(|w| u32::from_str_radix(w.strip_prefix("0x").unwrap(), 16).unwrap())
            .collect();
        assert_eq!(words, expected);
    }
    for (die, branch, byte) in [("GA106", "580", "0x1"), ("AD104", "595", "0x0")] {
        let row = rows
            .iter()
            .find(|r| r[0] == "gss.0x20808162" && r[1] == die && r[2] == branch)
            .unwrap();
        assert_eq!(row[5], byte);
    }
}
