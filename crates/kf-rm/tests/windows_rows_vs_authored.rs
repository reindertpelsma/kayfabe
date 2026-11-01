//! Lane C 2026-10-14 (resumed): the Windows-capture rows of the published table against what kayfabe AUTHORS (matrix D-1).
//! The real Windows driver's GSP, on a GA106 under six GSP branches (570..610) and on a GA104 at 595, states the same GMMU fault-buffer
//! sizes and CE fault-method buffer size that `authored.rs` states "equal to a stock GA106 GSP". This pins that equality (it is a
//! measurement of two Ampere dies, NOT of every die: the AD104 states 0x54000/0x220c20) so a change to either side is seen.

use kf_chip::DieId;
use kf_rm::{authored, consttab};

fn word(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().expect("word"))
}

#[test]
fn the_ampere_windows_rows_equal_the_authored_values_on_every_measured_branch() {
    let ga106 = DieId::from_name("GA106").expect("GA106");
    let ga104 = DieId::from_name("GA104").expect("GA104");
    let g = authored::GMMU_STATIC;
    let mut seen = 0;
    for (die, branches) in [
        (ga106, &[570u16, 575, 580, 590, 595, 610][..]),
        (ga104, &[595u16][..]),
    ] {
        for &b in branches {
            let gm = consttab::answer("rm.0x20800a59", die, b, &[0u8; 16]).expect("gmmu row");
            assert_eq!(word(&gm, 0), g.replayable_size, "{die} {b}");
            assert_eq!(word(&gm, 2), g.non_replayable_size, "{die} {b}");
            let ce = consttab::answer("rm.0x20802a08", die, b, &[0u8; 4]).expect("ce row");
            assert_eq!(
                word(&ce, 0),
                authored::CE_FAULT_METHOD_BUFFER_SIZE,
                "{die} {b}"
            );
            seen += 1;
        }
    }
    assert_eq!(seen, 7);
}

#[test]
fn an_unmeasured_die_or_branch_is_refused_not_borrowed() {
    let ad104 = DieId::from_name("AD104").expect("AD104");
    assert!(consttab::answer("rm.0x20800a59", ad104, 595, &[0u8; 16]).is_err());
    let ga106 = DieId::from_name("GA106").expect("GA106");
    assert!(consttab::answer("rm.0x20800a59", ga106, 560, &[0u8; 16]).is_err());
}
