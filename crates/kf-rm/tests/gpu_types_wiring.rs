//! ★★ GPU types, 2026-10-15 - the published constant table is WIRED (`docs/design/V3_GPU_TYPES_20261015.md`).
//!
//! One rule, tested at the responder (`InitTablePolicy`) and at the table:
//! - a (die, driver branch) HIT serves the measured value;
//! - a MISS serves an authored default only where the matrix says AUTHORED (labelled and counted) and otherwise refuses by name;
//! - a wrong branch is a miss; another die's row is never served (property test over every die the table names);
//! - a board-specific (key, die) (two boards of one die disagree) is never served from the table at all;
//! - a key whose rows exist but whose serving is an owner decision (`Withheld`) is counted and not served;
//! - every `rm.*` key of the table has a wiring decision, and the design doc's table says the same.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::DriverVersion;
use kf_abi::versions::table_for;
use kf_chip::die::DIE_NAMES;
use kf_chip::{DieId, arch};
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};
use kf_rm::consttab::{self, DieScope, Refusal};
use kf_rm::inittables::InitTablePolicy;
use kf_rm::rowserve::{self, Origin, Resolution, Use};

const PARAMS_AT: usize = 40;
const GA106: DieId = DieId::new(arch::GA100, 6);
const GA104: DieId = DieId::new(arch::GA100, 4);
const AD104: DieId = DieId::new(arch::AD100, 4);

const V580: DriverVersion = DriverVersion { major: 580, minor: 159, patch: 4 };
const V575: DriverVersion = DriverVersion { major: 575, minor: 57, patch: 8 };
const V595: DriverVersion = DriverVersion { major: 595, minor: 84, patch: 0 };

/// A host whose AUTHORED GMMU / CE numbers are not the GA106 ones, so a measured hit and an authored default are told apart by bytes.
const ODD_GMMU: (u32, u32) = (0x2000, 0x4000);
const ODD_CE: u32 = 0x9000;

fn policy(die: DieId, v: DriverVersion) -> InitTablePolicy {
    let mut host = ga106::host_facts();
    host.die = die;
    host.gmmu_static = kf_abi::gmmustatic::GmmuStaticRow {
        replayable_size: ODD_GMMU.0,
        replayable_shadow_metadata_size: 0,
        non_replayable_size: ODD_GMMU.1,
        non_replayable_shadow_metadata_size: 0,
    };
    host.ce_fault_method_buffer_size = ODD_CE;
    InitTablePolicy::new(
        ga106::board(),
        std::sync::Arc::new(host),
        *table_for(v).expect("a measured driver"),
    )
}

fn control(cmd: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&0xc1d0_0245u32.to_le_bytes());
    payload[4..8].copy_from_slice(&0x5c00_0003u32.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 7,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn w(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap())
}

fn ok(r: &Option<Reply>) -> bool {
    r.as_ref().is_some_and(|r| r.rpc_result == 0)
}

fn gmmu(p: &mut InitTablePolicy) -> Option<(u32, u32)> {
    let r = p.respond(&control(0x2080_0a59, &[0u8; 16]));
    ok(&r).then(|| {
        let body = r.unwrap().body;
        // the params are the tail of the reply at every guest layout (their offset differs per branch)
        let b = &body[body.len() - 16..];
        (w(b, 0), w(b, 2))
    })
}

fn ce(p: &mut InitTablePolicy) -> Option<u32> {
    let r = p.respond(&control(0x2080_2a08, &[0u8; 4]));
    ok(&r).then(|| {
        let body = r.unwrap().body;
        w(&body[body.len() - 4..], 0)
    })
}

// ---------------------------------------------------------------------------------------------------------------- hit / miss

#[test]
fn a_hit_serves_the_measured_value_not_the_hosts_authored_one() {
    let mut p = policy(GA106, V580);
    assert_eq!(gmmu(&mut p), Some((0x31000, 0x120c20)), "GA106 at 580: the Windows-captured row");
    assert_eq!(ce(&mut p), Some(0x5000));
    // GA104 has rows at 595 only
    let mut p = policy(GA104, V595);
    assert_eq!(gmmu(&mut p), Some((0x31000, 0x120c20)));
}

#[test]
fn a_miss_serves_the_authored_value_labelled_and_counted() {
    let before = rowserve::authored_default_count();
    let mut p = policy(AD104, V580);
    assert_eq!(gmmu(&mut p), Some(ODD_GMMU), "AD104 has no row: the host's authored value, never GA106's");
    assert_eq!(ce(&mut p), Some(ODD_CE));
    assert!(rowserve::authored_default_count() >= before + 2, "each authored default is counted");
}

#[test]
fn a_wrong_branch_is_a_miss_not_a_neighbour_branchs_row() {
    // GA106 has rows at 570..610 only; 565 (a measured guest driver) and 560 have none. (At the table: the guest layouts of
    // those older branches differ, so the responder-level bytes are checked on 580 below.)
    for b in [535u16, 550, 560, 565] {
        assert!(consttab::answer("rm.0x20800a59", GA106, b, &[0u8; 16]).is_err(), "branch {b}");
        match rowserve::resolve("rm.0x20800a59", GA106, b, &[0u8; 16], Some(&[7u8; 16])) {
            Resolution::Serve(bytes, Origin::Authored(_)) => assert_eq!(bytes, vec![7u8; 16]),
            other => panic!("branch {b}: {other:?}"),
        }
    }
    // GA104 at 580 (rows at 595 only)
    let mut p = policy(GA104, V580);
    assert_eq!(gmmu(&mut p), Some(ODD_GMMU));
}

// ---------------------------------------------------------------------------------------------------------------- P/K/A group

#[test]
fn the_group_p_scalars_are_served_on_a_hit_and_refused_on_a_miss() {
    let req = |v: u32| v.to_le_bytes().to_vec();
    let mut p = policy(GA106, V580);
    let r = p.respond(&control(0x2080_2806, &req(3)));
    assert!(ok(&r), "2806 hit");
    assert_eq!(w(&r.unwrap().body[PARAMS_AT..], 0), 0);
    let r = p.respond(&control(0x2080_2801, &req(3)));
    assert!(ok(&r), "2801 hit");
    assert_eq!(w(&r.unwrap().body[PARAMS_AT..], 0), 2);
    // a different question is refused
    assert!(!ok(&p.respond(&control(0x2080_2806, &req(4)))));
    // another die, another branch: refused (never GA106's value)
    for (die, v) in [(AD104, V580), (GA104, V580), (GA106, V595), (GA106, V575)] {
        let mut p = policy(die, v);
        let r = p.respond(&control(0x2080_2801, &req(3)));
        let expect_hit = (die, v.major) == (GA106, 575) || (die, v.major) == (GA106, 595);
        assert_eq!(ok(&r), expect_hit, "{die} {}", v.major);
    }
}

#[test]
fn the_withheld_keys_are_counted_and_never_served_even_on_a_row() {
    let before = rowserve::withheld_count();
    let mut p = policy(GA106, V580);
    let mut pool = vec![0u8; 40];
    pool[0..4].copy_from_slice(&4u32.to_le_bytes());
    assert!(!ok(&p.respond(&control(0x2080_121f, &pool))), "gfx pool is an owner decision");
    let caps = vec![0u8; 8];
    assert!(!ok(&p.respond(&control(0x5070_0709, &caps))), "NV5070 caps is an owner decision");
    assert!(rowserve::withheld_count() >= before + 2);
}

// ---------------------------------------------------------------------------------------------------------------- board-specific (F-6)

#[test]
fn a_board_specific_pair_is_refused_everywhere_and_has_no_table_row() {
    let reg = consttab::board_specific().expect("registry validates against the table");
    assert_eq!(reg.len(), 3, "GA102 8162, AD104 9001, AD104 a001");
    for b in reg {
        for branch in [0u16, 535, 570, 575, 580, 590, 595, 610, 615] {
            match consttab::lookup(&b.key, b.die, branch) {
                Err(Refusal::BoardSpecific { .. }) => {}
                other => panic!("{} {} {branch}: {other:?}", b.key, b.die),
            }
        }
        assert!(
            consttab::table().unwrap().iter().all(|r| !(r.key == b.key && r.die == DieScope::Die(b.die))),
            "{} {}: a board-specific value is never tabled",
            b.key,
            b.die
        );
    }
    // and not through the responder: AD104 at 595, no host answer -> the cudart gate request is refused
    let mut p = policy(AD104, V595);
    assert!(!ok(&p.respond(&control(0x2080_9001, &[0u8; 8]))));
    assert!(!ok(&p.respond(&control(0x2080_a001, &[0u8; 16]))));
}

/// The registry is DERIVED from the tier-2 measurements: every (key, die) whose boards disagree there is listed, and nothing else.
#[test]
fn the_board_specific_registry_equals_the_contradictions_in_the_tier2_summary() {
    let tsv = include_str!("../../../traces/constants_measure_20261014b/tier2_summary.tsv");
    let mut lines = tsv.lines();
    let head: Vec<&str> = lines.next().unwrap().split('\t').collect();
    let col = |n: &str| head.iter().position(|h| *h == n).unwrap();
    let (cdie, c8162, ca001, c9001, c9009) = (col("die_by_host"), col("8162"), col("a001"), col("9001"), col("9009"));
    let mut seen: std::collections::BTreeMap<(&str, String), std::collections::BTreeSet<String>> = Default::default();
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() <= c9009 || f[cdie] == "-" {
            continue;
        }
        for (key, c) in [("gss.0x20808162", c8162), ("cudart.0x2080a001", ca001), ("cudart.0x20809001", c9001), ("cudart.0x20809009", c9009)] {
            if f[c] != "-" && !f[c].is_empty() {
                seen.entry((key, f[cdie].to_string())).or_default().insert(f[c].to_string());
            }
        }
    }
    let mined: std::collections::BTreeSet<(String, String)> = seen
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|((k, d), _)| (k.to_string(), d))
        .collect();
    let listed: std::collections::BTreeSet<(String, String)> = consttab::board_specific()
        .unwrap()
        .iter()
        .map(|b| (b.key.clone(), b.die.to_string()))
        .collect();
    assert_eq!(mined, listed);
}

// ---------------------------------------------------------------------------------------------------------------- never across dies

/// ★ PROPERTY: over every die the table names (and every die ogkm names), no lookup returns a row of another die; a wildcard row is
/// returned only if it states a mechanism. Mutation check recorded in the doc: `lookup_in` matching any die of the family fails this.
#[test]
fn no_row_is_ever_served_across_dies() {
    let rows = consttab::table().unwrap();
    let branches: std::collections::BTreeSet<u16> = [0u16, 535, 560, 570, 575, 580, 590, 595, 610, 615]
        .into_iter()
        .chain(rows.iter().filter_map(|r| match r.branch {
            consttab::BranchScope::Major(m) => Some(m),
            consttab::BranchScope::Any => None,
        }))
        .collect();
    let keys: std::collections::BTreeSet<&str> = rows.iter().map(|r| r.key.as_str()).collect();
    let mut checked = 0u64;
    for &(a, i, name) in DIE_NAMES {
        let die = DieId::new(a, i);
        assert_eq!(die.name(), Some(name));
        for key in &keys {
            for &b in &branches {
                if let Ok(r) = consttab::lookup(key, die, b) {
                    match &r.die {
                        DieScope::Die(d) => assert_eq!(*d, die, "{key} {die} {b}: served {d}'s row"),
                        DieScope::Any => assert!(r.source.is_mechanism(), "{key}: a measured wildcard"),
                    }
                    if let consttab::BranchScope::Major(m) = r.branch {
                        assert_eq!(m, b, "{key} {die}: served branch {m}'s row at {b}");
                    }
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 40, "the walk saw {checked} hits");
}

// ---------------------------------------------------------------------------------------------------------------- nothing is unconsumed

#[test]
fn every_rm_key_of_the_table_has_a_wiring_decision_and_every_decision_has_rows() {
    let rows = consttab::table().unwrap();
    let rm_keys: std::collections::BTreeSet<&str> = rows.iter().map(|r| r.key.as_str()).filter(|k| k.starts_with("rm.")).collect();
    let decided: std::collections::BTreeSet<&str> = rowserve::USES.iter().map(|(k, _)| *k).collect();
    assert_eq!(rm_keys, decided, "a row consumed by no code, or a decision with no row");
    assert_eq!(rm_keys.len(), 6);
    assert_eq!(rows.iter().filter(|r| r.key.starts_with("rm.")).count(), 44, "42 inherited rows plus the two exact AD102/580 questions");
}

#[test]
fn the_design_doc_states_the_same_wiring_as_the_code() {
    let doc = include_str!("../../../docs/design/V3_GPU_TYPES_20261015.md");
    let mut found = 0;
    for l in doc.lines().filter(|l| l.starts_with("| `rm.0x")) {
        let c: Vec<&str> = l.split('|').map(str::trim).collect();
        let key = c[1].trim_matches('`');
        let want = match rowserve::use_of(key).expect("key in the code") {
            Use::Serve(rowserve::Fallback::Authored(_)) => "serve, authored fallback",
            Use::Serve(rowserve::Fallback::Refuse) => "serve, refuse on a miss",
            Use::Withheld(_) => "withheld",
        };
        assert_eq!(c[2], want, "{key}: doc vs code");
        found += 1;
    }
    assert_eq!(found, rowserve::USES.len());
}

#[test]
fn resolve_names_a_withheld_key_and_never_serves_it() {
    match rowserve::resolve("rm.0x2080121f", GA106, 580, &[0u8; 40], None) {
        Resolution::Refuse(t) => assert!(t.contains("W349WITHHELD") && t.contains("rm.0x2080121f"), "{t}"),
        Resolution::Serve(..) => panic!("served"),
    }
    match rowserve::resolve("rm.0x20800a59", GA106, 580, &[0u8; 16], Some(&[1u8; 16])) {
        Resolution::Serve(b, Origin::Measured) => assert_eq!(w(&b, 0), 0x31000),
        other => panic!("{other:?}"),
    }
}

/// The doc's machine-readable lines (`ZBC-MEASURED:`, `BOARD-SPECIFIC:`) equal the data.
#[test]
fn the_design_doc_lists_the_zbc_dies_and_the_board_specific_pairs_the_data_has() {
    let doc = include_str!("../../../docs/design/V3_GPU_TYPES_20261015.md");
    let line = |tag: &str| doc.lines().find_map(|l| l.strip_prefix(tag)).unwrap_or_else(|| panic!("no {tag} line")).to_string();
    let doc_zbc: std::collections::BTreeSet<String> = line("ZBC-MEASURED:").split_whitespace().map(str::to_string).collect();
    let data_zbc: std::collections::BTreeSet<String> = consttab::table()
        .unwrap()
        .iter()
        .filter(|r| r.key == "zbc.color")
        .map(|r| match &r.die {
            DieScope::Die(d) => d.to_string(),
            DieScope::Any => "*".into(),
        })
        .collect();
    assert_eq!(doc_zbc, data_zbc);
    let doc_bs: std::collections::BTreeSet<String> = line("BOARD-SPECIFIC:")
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect();
    let data_bs: std::collections::BTreeSet<String> = consttab::board_specific()
        .unwrap()
        .iter()
        .map(|b| format!("{} {}", b.key, b.die))
        .collect();
    assert_eq!(doc_bs, data_bs);
}

/// Exact questions from the loss-free native AD102/580.88 initialization capture.
#[test]
fn ad102_580_small_controls_answer_only_the_measured_requests() {
    let ad102 = DieId::new(arch::AD100, 2);
    let mut p = policy(ad102, V580);
    for (cmd, input, output) in [(0x2080_2801, 3u32, 2), (0x2080_2806, 0xb, 0)] {
        let r = p.respond(&control(cmd, &input.to_le_bytes()));
        assert!(ok(&r), "AD102/580 {cmd:#x} measured hit");
        assert_eq!(w(&r.unwrap().body[PARAMS_AT..], 0), output);
        assert!(!ok(&p.respond(&control(cmd, &(input ^ 1).to_le_bytes()))));
    }
    assert!(!ok(&p.respond(&control(0x2080_2806, &3u32.to_le_bytes()))),
        "GA106's request is not AD102's measured question");
    let mut wrong_branch = policy(ad102, V595);
    assert!(!ok(&wrong_branch.respond(&control(0x2080_2801, &3u32.to_le_bytes()))));
    let mut neighbour = policy(DieId::new(arch::AD100, 3), V580);
    assert!(!ok(&neighbour.respond(&control(0x2080_2801, &3u32.to_le_bytes()))));
}

#[test]
fn tu106_580_small_controls_answer_only_the_measured_requests() {
    let tu106 = DieId::new(arch::TU100, 6);
    let mut p = policy(tu106, V580);
    for (cmd, output) in [(0x2080_2801, 2), (0x2080_2806, 0)] {
        let r = p.respond(&control(cmd, &3u32.to_le_bytes()));
        assert!(ok(&r), "TU106/580 {cmd:#x} measured hit");
        assert_eq!(w(&r.unwrap().body[PARAMS_AT..], 0), output);
        assert!(!ok(&p.respond(&control(cmd, &2u32.to_le_bytes()))));
        assert!(!ok(&p.respond(&control(cmd, &[]))));
        assert!(!ok(&p.respond(&control(cmd, &[3, 0, 0, 0, 0, 0, 0, 0]))));
    }
    assert!(!ok(&p.respond(&control(0x2080_2806, &0xbu32.to_le_bytes()))));
    let mut wrong_branch = policy(tu106, V595);
    assert!(!ok(&wrong_branch.respond(&control(0x2080_2801, &3u32.to_le_bytes()))));
    let mut neighbour = policy(DieId::new(arch::TU100, 4), V580);
    assert!(!ok(&neighbour.respond(&control(0x2080_2801, &3u32.to_le_bytes()))));
}
