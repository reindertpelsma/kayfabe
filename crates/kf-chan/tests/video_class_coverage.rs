// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! STATUS: LIVE 2026-11-15. GPU-free coverage audit of the VIDEO tier's derived method tables.
//!
//! A guest-KERNEL channel on a video engine (NVDEC, NVENC, OFA) is born Translated on a host ring
//! whose owned object has a video class, and `tmode::video_write` decodes the ring's subchannel 0-4
//! methods against `VideoClass::of_class(class)`. A class with no table makes the decoder refuse
//! `ForeignClass` on the ring's FIRST method and the channel dies. This file enumerates every class
//! such a ring can hold and fails unless each has a table entry or is listed in [`KNOWN_GAPS`].
//!
//! The candidate list is DERIVED, never copied: the engine types come from
//! `kf_chan::host::video_kind_of_engine` (the function `HostRing::new` routes with) and the classes
//! from `kf_chip` for every family (`HostRing` picks the newest class the host supports, so any
//! family's class may be the one held).
//!
//! `KNOWN_GAPS` is an exact-match ratchet. A listed class that gains a table entry must be REMOVED
//! from the list (the test fails if a listed class is covered); a new uncovered class fails the
//! test. This file records the gap; it does NOT fix it. The public headers define no OFA method
//! layout, so no OFA table entry may be added or aliased without an owner ruling.
use kf_abi::submit::method_header_inc;
use kf_chan::host::video_kind_of_engine;
use kf_chan::tmode::{GrConfig, TState, decode};
use kf_chan::translated::Refusal;
use kf_chan::vtables::VideoClass;
use kf_chip::Family;
use kf_chip::classes::Kind;
use std::collections::BTreeSet;

/// Where the gap was investigated.
const REPORT: &str = "/root/night/WUKONG_RELAUNCH_TDR_20261115.md section A (H-A1)";

/// Video classes a guest-kernel ring can hold that have NO derived method table today, each with
/// a one-line reason and the report. Exact match: see the module comment.
const KNOWN_GAPS: &[(u32, &str)] = &[
    (
        0xb4b7,
        "NVENC: class id from ogkm but no open-gpu-doc header in classes/video, so no derived table (not observed on the route; see WUKONG_RELAUNCH_TDR_20261115.md section A)",
    ),
    (
        0xb8b0,
        "NVDEC: class id from ogkm but no open-gpu-doc header in classes/video, so no derived table (not observed on the route; see WUKONG_RELAUNCH_TDR_20261115.md section A)",
    ),
    (
        0xb8fa,
        "OFA: no public header defines an OFA method layout, a table needs an owner ruling (see WUKONG_RELAUNCH_TDR_20261115.md section A: the guest kernel OFA ring dies on its first method, class 0xC9FA measured)",
    ),
    (
        0xc4b0,
        "NVDEC: class id from ogkm but no open-gpu-doc header in classes/video, so no derived table (not observed on the route; see WUKONG_RELAUNCH_TDR_20261115.md section A)",
    ),
    (
        0xc4b7,
        "NVENC: class id from ogkm but no open-gpu-doc header in classes/video, so no derived table (not observed on the route; see WUKONG_RELAUNCH_TDR_20261115.md section A)",
    ),
    (
        0xc6fa,
        "OFA: no public header defines an OFA method layout, a table needs an owner ruling (see WUKONG_RELAUNCH_TDR_20261115.md section A: the guest kernel OFA ring dies on its first method, class 0xC9FA measured)",
    ),
    (
        0xc7fa,
        "OFA: no public header defines an OFA method layout, a table needs an owner ruling (see WUKONG_RELAUNCH_TDR_20261115.md section A: the guest kernel OFA ring dies on its first method, class 0xC9FA measured)",
    ),
    (
        0xc9fa,
        "OFA: no public header defines an OFA method layout, a table needs an owner ruling (see WUKONG_RELAUNCH_TDR_20261115.md section A: the guest kernel OFA ring dies on its first method, class 0xC9FA measured)",
    ),
    (
        0xcdb0,
        "NVDEC: class id from ogkm but no open-gpu-doc header in classes/video, so no derived table (not observed on the route; see WUKONG_RELAUNCH_TDR_20261115.md section A)",
    ),
    (
        0xcdfa,
        "OFA: no public header defines an OFA method layout, a table needs an owner ruling (see WUKONG_RELAUNCH_TDR_20261115.md section A: the guest kernel OFA ring dies on its first method, class 0xC9FA measured)",
    ),
    (
        0xceb0,
        "NVDEC: class id from ogkm but no open-gpu-doc header in classes/video, so no derived table (not observed on the route; see WUKONG_RELAUNCH_TDR_20261115.md section A)",
    ),
    (
        0xceb7,
        "NVENC: class id from ogkm but no open-gpu-doc header in classes/video, so no derived table (not observed on the route; see WUKONG_RELAUNCH_TDR_20261115.md section A)",
    ),
    (
        0xcefa,
        "OFA: no public header defines an OFA method layout, a table needs an owner ruling (see WUKONG_RELAUNCH_TDR_20261115.md section A: the guest kernel OFA ring dies on its first method, class 0xC9FA measured)",
    ),
    (
        0xcfb0,
        "NVDEC: class id from ogkm but no open-gpu-doc header in classes/video, so no derived table (not observed on the route; see WUKONG_RELAUNCH_TDR_20261115.md section A)",
    ),
    (
        0xcfb7,
        "NVENC: class id from ogkm but no open-gpu-doc header in classes/video, so no derived table (not observed on the route; see WUKONG_RELAUNCH_TDR_20261115.md section A)",
    ),
    (
        0xcffa,
        "OFA: no public header defines an OFA method layout, a table needs an owner ruling (see WUKONG_RELAUNCH_TDR_20261115.md section A: the guest kernel OFA ring dies on its first method, class 0xC9FA measured)",
    ),
    (
        0xd1b0,
        "NVDEC: class id from ogkm but no open-gpu-doc header in classes/video, so no derived table (not observed on the route; see WUKONG_RELAUNCH_TDR_20261115.md section A)",
    ),
    (
        0xd1b7,
        "NVENC: class id from ogkm but no open-gpu-doc header in classes/video, so no derived table (not observed on the route; see WUKONG_RELAUNCH_TDR_20261115.md section A)",
    ),
    (
        0xd1fa,
        "OFA: no public header defines an OFA method layout, a table needs an owner ruling (see WUKONG_RELAUNCH_TDR_20261115.md section A: the guest kernel OFA ring dies on its first method, class 0xC9FA measured)",
    ),
    (
        0xd2b0,
        "NVDEC: class id from ogkm but no open-gpu-doc header in classes/video, so no derived table (not observed on the route; see WUKONG_RELAUNCH_TDR_20261115.md section A)",
    ),
    (
        0xd2fa,
        "OFA: no public header defines an OFA method layout, a table needs an owner ruling (see WUKONG_RELAUNCH_TDR_20261115.md section A: the guest kernel OFA ring dies on its first method, class 0xC9FA measured)",
    ),
];

/// Every class a Translated guest-kernel video ring can hold: for each NV2080 engine type that
/// `video_kind_of_engine` routes, the classes of that kind on every family.
fn candidates() -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    // The engine-type space is small (NV2080 types are below 0x100); scanning past it proves the
    // router returns `None` there rather than assuming it.
    for engine in 0..0x200u32 {
        if let Some((_, kind)) = video_kind_of_engine(engine) {
            for f in Family::ALL {
                out.extend(f.classes().of_kind(kind).iter().copied());
            }
        }
    }
    out
}

fn covered(class: u32) -> bool {
    VideoClass::of_class(class).is_some()
}

/// The ratchet. Findings are empty when `gaps` is exactly the uncovered part of `cands`, and every
/// entry is well-formed.
fn audit(
    cands: &BTreeSet<u32>,
    is_covered: &dyn Fn(u32) -> bool,
    gaps: &[(u32, &str)],
) -> Vec<String> {
    let mut f = Vec::new();
    let mut seen = BTreeSet::new();
    for &(class, reason) in gaps {
        if !seen.insert(class) {
            f.push(format!("{class:#x}: listed twice"));
        }
        if reason.trim().is_empty() || reason.contains('\n') || reason.contains('\r') {
            f.push(format!("{class:#x}: the reason must be one non-empty line"));
        }
        if !cands.contains(&class) {
            f.push(format!(
                "{class:#x}: listed but no Translated video ring can hold it (stray entry)"
            ));
        } else if is_covered(class) {
            f.push(format!(
                "{class:#x}: listed but now has a table entry: REMOVE it from KNOWN_GAPS"
            ));
        }
    }
    for &class in cands {
        if !is_covered(class) && !seen.contains(&class) {
            f.push(format!("{class:#x}: a video ring can hold it, it has no method table and is not in KNOWN_GAPS"));
        }
    }
    f
}

#[test]
fn every_kernel_video_class_has_a_table_entry_or_a_known_gap() {
    let f = audit(&candidates(), &covered, KNOWN_GAPS);
    assert!(f.is_empty(), "video class coverage ratchet: {f:#?}");
}

#[test]
fn known_gaps_is_exactly_the_uncovered_set() {
    let uncovered: BTreeSet<u32> = candidates().into_iter().filter(|c| !covered(*c)).collect();
    let listed: BTreeSet<u32> = KNOWN_GAPS.iter().map(|g| g.0).collect();
    assert_eq!(
        listed, uncovered,
        "KNOWN_GAPS must equal the classes with no table (left: listed, right: uncovered)"
    );
    assert_eq!(listed.len(), KNOWN_GAPS.len(), "a class is listed twice");
}

#[test]
fn the_enumeration_reaches_all_three_video_kinds() {
    let mut kinds = BTreeSet::new();
    for engine in 0..0x200u32 {
        if let Some((_, k)) = video_kind_of_engine(engine) {
            kinds.insert(k);
        }
    }
    assert_eq!(
        kinds,
        BTreeSet::from([Kind::VideoDecoder, Kind::VideoEncoder, Kind::OpticalFlow])
    );
    let c = candidates();
    assert!(!c.is_empty());
    // Anchors: a decoder, an encoder and the OFA class are all derived, not hand-listed.
    for kind in [Kind::VideoDecoder, Kind::VideoEncoder, Kind::OpticalFlow] {
        assert!(
            Family::ALL
                .iter()
                .any(|f| f.classes().of_kind(kind).iter().any(|k| c.contains(k))),
            "no candidate of kind {kind:?}"
        );
    }
}

#[test]
fn known_gap_reasons_are_one_line_and_point_at_the_report() {
    assert!(REPORT.contains("WUKONG_RELAUNCH_TDR_20261115"));
    for &(class, reason) in KNOWN_GAPS {
        assert!(
            !reason.trim().is_empty() && !reason.contains('\n'),
            "{class:#x}"
        );
        assert!(
            reason.contains("WUKONG_RELAUNCH_TDR_20261115"),
            "{class:#x}: no pointer to the report"
        );
    }
}

/// The predicate this audit uses is the one the real decoder uses: a video ring of an uncovered
/// class refuses `ForeignClass` on its first subchannel-4 method, a covered class never does.
#[test]
fn the_decoder_agrees_with_the_table_predicate() {
    for class in candidates() {
        let mut pb = vec![method_header_inc(4, 0x240, 1).unwrap()];
        pb.push(1);
        let mut st = TState::default();
        st.gr = GrConfig {
            video: class,
            ..GrConfig::default()
        };
        let r = decode(&pb, |_| false, &mut st, None);
        let foreign = matches!(r, Err(Refusal::ForeignClass { class: c, .. }) if c == class);
        assert_eq!(foreign, !covered(class), "class {class:#x}: {r:?}");
    }
}

// ---- the ratchet's own tests (hostile arguments) ----------------------------------------------

fn set(v: &[u32]) -> BTreeSet<u32> {
    v.iter().copied().collect()
}

#[test]
fn a_new_uncovered_class_fails() {
    let f = audit(&set(&[1, 2]), &|c| c == 1, &[]);
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].contains("0x2") && f[0].contains("not in KNOWN_GAPS"));
}

#[test]
fn a_listed_gap_that_gains_a_table_entry_fails() {
    let f = audit(&set(&[7]), &|_| true, &[(7, "r")]);
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].contains("REMOVE"));
}

#[test]
fn a_stray_allow_list_entry_fails() {
    // Not a candidate, whether covered or not; and an entry the table also lacks.
    assert_eq!(audit(&set(&[1]), &|_| true, &[(9, "r")]).len(), 1);
    assert_eq!(
        audit(&set(&[1]), &|_| false, &[(1, "r"), (9, "r")]).len(),
        1
    );
}

#[test]
fn the_exact_match_passes() {
    assert!(audit(&set(&[1, 2, 3]), &|c| c != 2, &[(2, "r")]).is_empty());
    assert!(audit(&set(&[]), &|_| false, &[]).is_empty());
}

#[test]
fn hostile_entries_are_rejected() {
    let c = set(&[0, u32::MAX]);
    let none = |_: u32| false;
    // Empty, blank and multi-line reasons.
    for bad in ["", "   ", "a\nb", "a\rb", "\n"] {
        let f = audit(&c, &none, &[(0, bad), (u32::MAX, "r")]);
        assert_eq!(f.len(), 1, "{bad:?}: {f:?}");
    }
    // A duplicate entry is not accepted twice.
    assert_eq!(
        audit(&c, &none, &[(0, "r"), (0, "r"), (u32::MAX, "r")]).len(),
        1
    );
    // Class 0 and u32::MAX are ordinary values: uncovered, they need an entry like any other.
    assert_eq!(audit(&c, &none, &[]).len(), 2);
    // A gap for an empty candidate set is a stray.
    assert_eq!(audit(&set(&[]), &none, &[(0, "r")]).len(), 1);
}

#[test]
fn a_duplicate_entry_fails() {
    let f = audit(&set(&[5]), &|_| false, &[(5, "r"), (5, "r")]);
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].contains("twice"));
}

#[test]
fn a_covered_class_among_many_gaps_is_found() {
    let c = set(&[1, 2, 3, 4]);
    let f = audit(&c, &|c| c == 3, &[(1, "r"), (2, "r"), (3, "r"), (4, "r")]);
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].starts_with("0x3:"));
}

#[test]
fn a_covered_class_listed_with_a_stray_reports_both() {
    let f = audit(&set(&[1]), &|_| true, &[(1, "r"), (99, "r")]);
    assert_eq!(f.len(), 2, "{f:?}");
}
