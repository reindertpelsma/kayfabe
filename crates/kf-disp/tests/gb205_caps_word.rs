//! Admission boundary for the per-die word that public class headers do not name.
//! Oracle: own GB205/root Linux595 read; not a neighboring GPU's value.
use kf_disp::capsderive::{Die, shipped_measured};

#[test]
fn gb205_linux595_has_its_own_measured_caps_word() {
    let rows: Vec<_> = shipped_measured()
        .iter()
        .filter(|m| {
            m.die
                == Die {
                    architecture: 0x1b0,
                    implementation: 5,
                }
                && m.branch == "595"
                && m.caps_class == 0xca73
                && m.offset == 0x00c
        })
        .collect();
    assert_eq!(rows.len(), 1, "GB205595 must not silently borrow GB206580");
    assert_eq!(rows[0].value, 0x102);
    assert!(rows[0].source.starts_with("measured:GB205"));
}

#[test]
fn gb205_measurement_is_not_compressed_to_other_dies_or_branches() {
    assert!(!shipped_measured().iter().any(|m| {
        m.die
            == Die {
                architecture: 0x1b0,
                implementation: 5,
            }
            && (m.branch == "*" || m.branch == "590" || m.branch == "610")
    }));
}
