//! Admission boundary for the GB202 per-die caps word that public class headers do not name.
//! Oracle: own GB202 (10de:2b85) root read-only BAR0 read under the stock Linux 580.159.04 open
//! driver, rental 56909569, 2026-10-20 (traces/blackwell_resume_20261020/caps); not a neighbour's value.
use kf_disp::capsderive::{Die, shipped_measured};

const GB202: Die = Die {
    architecture: 0x1b0,
    implementation: 2,
};

#[test]
fn gb202_branch580_has_its_own_measured_caps_word() {
    let rows: Vec<_> = shipped_measured()
        .iter()
        .filter(|m| {
            m.die == GB202 && m.branch == "580" && m.caps_class == 0xca73 && m.offset == 0x00c
        })
        .collect();
    assert_eq!(rows.len(), 1, "GB202 580 must not borrow GB205/GB206");
    assert_eq!(rows[0].value, 0x102);
    assert!(rows[0].source.starts_with("measured:GB202"));
}

#[test]
fn gb202_measurement_is_not_compressed_to_other_branches() {
    assert!(
        !shipped_measured()
            .iter()
            .any(|m| m.die == GB202 && (m.branch == "*" || m.branch == "590" || m.branch == "610"))
    );
}
