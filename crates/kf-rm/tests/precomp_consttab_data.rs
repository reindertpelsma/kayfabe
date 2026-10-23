// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! 2026-10-23: tier-3 rows must match the committed public-interface measurements.
use kf_chip::DieId;
use kf_rm::consttab::{self, BranchScope, DieScope, SourceTag};

const GB202: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../traces/blackwell_windows_resume_20261023/lease_metadata/precomp_tier3.txt"
));
const GB206: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../traces/blackwell_provider_validation_20261023/lease_metadata/precomp_tier3.txt"
));

fn samples(evidence: &str) -> Vec<(u32, u32)> {
    let table = kf_disp::class::for_version("595.91.07").unwrap();
    let row = &kf_chip::display::BLACKWELL_GB20X;
    let base = kf_disp::regs::Regs::for_ip(&table.version, row.ip_version)
        .unwrap()
        .f("NV_PDISP_FE_SW")
        .unwrap()
        .1 as u32;
    let out: Vec<_> = evidence
        .lines()
        .filter(|line| line.contains("\tword\t"))
        .map(|line| {
            let get = |name: &str| {
                let value = line
                    .split('\t')
                    .find_map(|part| part.strip_prefix(name))
                    .unwrap();
                u32::from_str_radix(value.trim_start_matches("0x"), 16).unwrap()
            };
            (get("address=") - base, get("value="))
        })
        .collect();
    assert_eq!(
        out.len(),
        48,
        "the measurement must contain all six types/eight windows"
    );
    out
}

fn key(offset: u32) -> String {
    format!("display.precomp.0xca73.{offset:#06x}")
}

fn values(name: &str, evidence: &str) {
    let die = DieId::from_name(name).unwrap();
    for (offset, value) in samples(evidence) {
        let row = consttab::lookup(&key(offset), die, 595).unwrap();
        assert_eq!(row.size, 4);
        assert!(row.inputs.is_empty());
        assert_eq!(row.words, [value], "{name} {offset:#x}");
        assert_eq!(row.body(), value.to_le_bytes());
    }
}

fn provenance(name: &str, evidence: &str, path: &str) {
    for (offset, _) in samples(evidence) {
        let row = consttab::lookup(&key(offset), DieId::from_name(name).unwrap(), 595).unwrap();
        assert_eq!(row.die, DieScope::Die(DieId::from_name(name).unwrap()));
        assert_eq!(row.branch, BranchScope::Major(595));
        assert_eq!(
            row.source,
            SourceTag::Measured {
                die: name.into(),
                driver: "595.91.07".into(),
                date: "2026-10-23".into(),
            }
        );
        assert_eq!(row.evidence, path);
    }
}

#[test]
fn gb202_every_word_matches_its_measurement() {
    values("GB202", GB202);
}

#[test]
fn gb206_every_word_matches_its_measurement() {
    values("GB206", GB206);
}

#[test]
fn gb202_every_word_retains_its_measured_source_and_evidence() {
    provenance(
        "GB202",
        GB202,
        "traces/blackwell_windows_resume_20261023/lease_metadata/precomp_tier3.txt",
    );
}

#[test]
fn gb206_every_word_retains_its_measured_source_and_evidence() {
    provenance(
        "GB206",
        GB206,
        "traces/blackwell_provider_validation_20261023/lease_metadata/precomp_tier3.txt",
    );
}

#[test]
fn physical_rows_are_scoped_to_the_measured_host_branch() {
    for name in ["GB202", "GB206"] {
        for branch in [0, 580, 590, 596] {
            assert!(
                consttab::lookup(&key(0x780), DieId::from_name(name).unwrap(), branch).is_err()
            );
        }
    }
}

#[test]
fn the_generic_table_never_silently_borrows_a_measured_die() {
    for implementation in [3, 5, 7, 8] {
        assert!(consttab::lookup(&key(0x780), DieId::new(0x1b0, implementation), 595).is_err());
    }
}
