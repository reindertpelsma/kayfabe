//! ★ F-1 (matrix 5.3.5): the ZBC default entries are MEASURED PER DIE, not one GA102 capture served to every die.
//! `ZbcPolicy::new(.., die)`: a measured die gets its own defaults (Turing: formats 1/2/4 and a non-zero colorDS; Ampere/Ada/Blackwell:
//! the GA102 values), an unmeasured die gets an EMPTY table (valid = 0) with a counted, logged refusal - never a neighbour's entries.

use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_abi::zbc::{self as z, TableType};
use kf_chip::DieId;
use kf_chip::die::DIE_NAMES;
use kf_rm::consttab;
use kf_rm::zbc::{ZBC_DEFAULTS_REFUSED, ZbcPolicy};
use std::sync::atomic::Ordering;

fn policy(die: DieId) -> ZbcPolicy {
    let abi = *table_for(BENCH_DRIVER).expect("bench");
    ZbcPolicy::new(abi, Some([(1, 30), (1, 15), (1, 15)]), die)
}

fn entry(p: &mut ZbcPolicy, t: TableType, idx: u32) -> (bool, [u32; 14]) {
    let mut q = vec![0u8; z::GET_ENTRY_PARAMS_SIZE];
    z::put(&mut q, 11, idx);
    z::put(&mut q, 13, t.wire());
    let r = p.answer(z::GET_ZBC_CLEAR_TABLE_ENTRY, &q).expect("entry");
    let w: [u32; 14] = std::array::from_fn(|i| z::word(&r, i).unwrap());
    (w[12] != 0, w)
}

fn die(n: &str) -> DieId {
    DieId::from_name(n).expect("die")
}

#[test]
fn turing_gets_turings_defaults_not_the_ga102_capture() {
    for n in ["TU102", "TU106", "TU116"] {
        let mut p = policy(die(n));
        let (v2, w2) = entry(&mut p, TableType::Color, 2);
        assert!(v2);
        assert_eq!(w2[0..4], [0xffff_ffff; 4], "{n}");
        assert_eq!(
            w2[4..8],
            [0x3f80_0000; 4],
            "{n}: colorDS is non-zero on Turing"
        );
        assert_eq!(w2[10], 2, "{n}: format 2");
        assert_eq!(entry(&mut p, TableType::Color, 1).1[10], 1, "{n}: format 1");
        assert_eq!(entry(&mut p, TableType::Color, 3).1[10], 4, "{n}: format 4");
    }
}

#[test]
fn ampere_ada_blackwell_dies_get_the_measured_defaults() {
    for n in [
        "GA102", "GA104", "GA106", "AD102", "AD103", "AD104", "AD106", "GB202", "GB203", "GB206",
    ] {
        let mut p = policy(die(n));
        assert_eq!(
            entry(&mut p, TableType::Color, 2).1[0..4],
            [0xffff_ffff; 4],
            "{n}"
        );
        assert_eq!(entry(&mut p, TableType::Color, 2).1[4..8], [0; 4], "{n}");
        assert_eq!(entry(&mut p, TableType::Color, 3).1[10], 0, "{n}");
        assert_eq!(entry(&mut p, TableType::Depth, 2).1[8], 0x3f80_0000, "{n}");
        assert_eq!(entry(&mut p, TableType::Stencil, 3).1[9], 0xff, "{n}");
        assert!(
            !entry(&mut p, TableType::Color, 4).0,
            "{n}: only the defaults, never a previous renter's entry"
        );
    }
}

#[test]
fn an_unmeasured_die_gets_an_empty_table_and_a_counted_refusal() {
    for n in [
        "GA100", "GA107", "AD107", "TU104", "GB205", "GB207", "GH100",
    ] {
        let before = ZBC_DEFAULTS_REFUSED.load(Ordering::Relaxed);
        let mut p = policy(die(n));
        assert!(
            ZBC_DEFAULTS_REFUSED.load(Ordering::Relaxed) >= before + 3,
            "{n}: three tables refused, counted"
        );
        for (t, i) in [
            (TableType::Color, 1),
            (TableType::Color, 2),
            (TableType::Depth, 1),
            (TableType::Stencil, 1),
        ] {
            assert!(!entry(&mut p, t, i).0, "{n}: nothing is claimed");
        }
        // a guest SET still works on the empty table
        let mut s = vec![0u8; z::SET_COLOR_PARAMS_SIZE];
        z::put(&mut s, 0, 0xff00_0000);
        assert!(p.answer(z::SET_ZBC_COLOR_CLEAR, &s).is_ok(), "{n}");
    }
}

/// ★ Property: for every die ogkm names, the defaults served are exactly that die's own row (or none) - never another die's.
#[test]
fn no_die_is_served_another_dies_defaults() {
    let rows = consttab::table().unwrap();
    let mut measured = 0;
    for &(a, i, name) in DIE_NAMES {
        let d = DieId::new(a, i);
        let has = rows
            .iter()
            .any(|r| r.key == "zbc.color" && r.die == consttab::DieScope::Die(d));
        let mut p = policy(d);
        assert_eq!(
            entry(&mut p, TableType::Color, 2).0,
            has,
            "{name}: served iff the die has its own row"
        );
        measured += usize::from(has);
    }
    assert_eq!(measured, 13, "the 13 dies of the idle-rental sweep");
}

#[test]
fn a_zbc_row_is_per_die_with_a_wildcard_branch_only_in_the_host_state_namespace() {
    let hdr = "key\tdie\tbranch\tsize\tinputs\twords\tsource\tevidence\n";
    // CODE READ: historical text at commit 007ba1eb0:121;
    // BW9 quotes this statement without independently validating its evidence.
    // allowed: zbc.* measured per die, branch *
    let ok = format!(
        "{hdr}zbc.color\tTU116\t*\t44\t-\t0 0 0 0 0 0 0 0 0 0 1\tmeasured:TU116,580.178.04,2026-10-14\tx\n"
    );
    assert!(consttab::parse(&ok).is_ok());
    // CODE READ: historical text at commit 007ba1eb0:126;
    // BW9 quotes this statement without independently validating its evidence.
    // a measured wildcard BRANCH anywhere else is still refused
    let bad = format!(
        "{hdr}rm.0x20802a08\tGA106\t*\t4\t-\t0x5000\tmeasured:GA106,580.105.07,2026-10-14\tx\n"
    );
    assert!(consttab::parse(&bad).unwrap_err().contains("mechanism"));
    // and a wildcard DIE never is
    let bad = format!(
        "{hdr}zbc.color\t*\t*\t44\t-\t0 0 0 0 0 0 0 0 0 0 1\tmeasured:TU116,580.178.04,2026-10-14\tx\n"
    );
    assert!(consttab::parse(&bad).is_err());
}
