//! Admission boundary for the GB203 per-die caps word that public class headers do not name.
//! Oracle: own GB203 (10de:2c05, RTX 5070 Ti) root read-only BAR0 reads under the stock Linux
//! 580.159.04 and 595.91.07 open drivers, Vast lease 58957600, 2026-11-03
//! (traces/blackwell_hw_20261103/gb203_caps); not a neighbour's value. Before these rows the GB203
//! Windows guest never allocated its display core channel (the gate run bwhw1 at 993b844a).
use kf_disp::capsderive::{Die, shipped_measured};

const GB203: Die = Die {
    architecture: 0x1b0,
    implementation: 3,
};

#[test]
fn gb203_has_its_own_measured_caps_word_on_both_measured_branches() {
    for branch in ["580", "595"] {
        let rows: Vec<_> = shipped_measured()
            .iter()
            .filter(|m| {
                m.die == GB203 && m.branch == branch && m.caps_class == 0xca73 && m.offset == 0x00c
            })
            .collect();
        assert_eq!(
            rows.len(),
            1,
            "GB203 {branch} must not borrow another die's row"
        );
        assert_eq!(rows[0].value, 0x102);
        assert!(rows[0].source.starts_with("measured:GB203 10de:2c05"));
        assert!(
            rows[0]
                .source
                .contains("traces/blackwell_hw_20261103/gb203_caps/")
        );
    }
}

#[test]
fn gb203_measurement_is_not_compressed_to_other_branches() {
    assert!(
        !shipped_measured()
            .iter()
            .any(|m| m.die == GB203 && !(m.branch == "580" || m.branch == "595"))
    );
}
