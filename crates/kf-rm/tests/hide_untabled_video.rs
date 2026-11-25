// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! STATUS: LIVE 2026-11-15. GPU-free tests of `hostquery::hide_untabled_video`, the filter that serves
//! a guest no video engine whose classes have no derived method table
//! (`kf_chip::video_gaps::UNTABLED_VIDEO_CLASSES`; run profile `KF3_WIN_HIDE_UNTABLED_VIDEO=1`).
//!
//! The whole-query before/after on a captured GA106 is in `host_facts_query_ga106.rs`; this file is the
//! rule over every family and over hostile engine lists. The rule is a filter: it never adds an engine,
//! never reorders one, and touches nothing that is not an NVDEC, NVENC or OFA.

use kf_chip::Family;
use kf_chip::classes::Kind;
use kf_chip::video_gaps::{UNTABLED_VIDEO_CLASSES, is_untabled, kernel_ring_servable};
use kf_rm::authored::EngineKind as E;
use kf_rm::hostquery::hide_untabled_video;

fn base() -> Vec<E> {
    vec![
        E::Graphics(0),
        E::Copy(0),
        E::Copy(1),
        E::VideoDecode(0),
        E::VideoEncode(0),
        E::OpticalFlow(0),
        E::Software,
    ]
}

fn kind_of(k: E) -> Option<Kind> {
    match k {
        E::VideoDecode(_) => Some(Kind::VideoDecoder),
        E::VideoEncode(_) => Some(Kind::VideoEncoder),
        E::OpticalFlow(_) => Some(Kind::OpticalFlow),
        _ => None,
    }
}

/// Ampere and Ada: OFA has no table (every OFA class is listed), NVDEC and NVENC do.
#[test]
fn ampere_and_ada_lose_only_ofa() {
    for f in [Family::Ampere, Family::Ada] {
        assert_eq!(
            hide_untabled_video(f, base()),
            [
                E::Graphics(0),
                E::Copy(0),
                E::Copy(1),
                E::VideoDecode(0),
                E::VideoEncode(0),
                E::Software
            ],
            "{f:?}"
        );
    }
}

/// Turing (no OFA class; its NVDEC/NVENC classes have no table), Hopper (no NVENC class; NVDEC and
/// OFA untabled) and Blackwell (all untabled) serve no video engine through a guest-kernel ring.
#[test]
fn turing_hopper_and_blackwell_serve_no_video_engine() {
    for f in [Family::Turing, Family::Hopper, Family::Blackwell] {
        assert_eq!(
            hide_untabled_video(f, base()),
            [E::Graphics(0), E::Copy(0), E::Copy(1), E::Software],
            "{f:?}"
        );
    }
}

/// The verdict is the list's, family by family and kind by kind: kept means no class of the kind is
/// listed and the family has one; dropped means a listed class or none at all.
#[test]
fn every_verdict_is_derived_from_the_family_class_sets_and_the_list() {
    for f in Family::ALL {
        for k in [Kind::VideoDecoder, Kind::VideoEncoder, Kind::OpticalFlow] {
            let engine = match k {
                Kind::VideoDecoder => E::VideoDecode(0),
                Kind::VideoEncoder => E::VideoEncode(0),
                _ => E::OpticalFlow(0),
            };
            let kept = hide_untabled_video(f, vec![engine]) == [engine];
            let classes = f.classes().of_kind(k);
            let derived = !classes.is_empty() && classes.iter().all(|c| !is_untabled(*c));
            assert_eq!(kept, derived, "{f:?} {k:?} {classes:x?}");
            assert_eq!(kept, kernel_ring_servable(f, k), "{f:?} {k:?}");
        }
    }
}

/// Hostile lists: every instance of a hidden kind goes (including instances no host has), duplicates
/// go together, an empty list stays empty, and a list with no video engine is untouched.
#[test]
fn hostile_engine_lists() {
    let f = Family::Ampere;
    assert!(hide_untabled_video(f, vec![]).is_empty());
    assert_eq!(
        hide_untabled_video(f, vec![E::OpticalFlow(0), E::OpticalFlow(0)]),
        []
    );
    assert_eq!(
        hide_untabled_video(
            f,
            vec![
                E::OpticalFlow(1),
                E::OpticalFlow(u32::MAX),
                E::Copy(u32::MAX),
                E::Graphics(u32::MAX),
                E::VideoDecode(u32::MAX),
                E::VideoEncode(7),
            ]
        ),
        [
            E::Copy(u32::MAX),
            E::Graphics(u32::MAX),
            E::VideoDecode(u32::MAX),
            E::VideoEncode(7)
        ],
        "only the kind is judged, never the instance number"
    );
    let plain = vec![E::Graphics(0), E::Copy(0), E::Software];
    assert_eq!(hide_untabled_video(f, plain.clone()), plain);
    assert_eq!(hide_untabled_video(f, vec![E::Software]), [E::Software]);
}

/// A filter, not a source: the output is an in-order subsequence of the input on every family, only
/// video kinds are ever removed, and applying it twice changes nothing.
#[test]
fn it_only_removes_in_order_video_engines_and_is_idempotent() {
    let mut input = base();
    input.extend([E::VideoDecode(1), E::VideoEncode(1), E::OpticalFlow(1)]);
    input.reverse();
    for f in Family::ALL {
        let out = hide_untabled_video(f, input.clone());
        let mut it = input.iter();
        for o in &out {
            assert!(
                it.any(|i| i == o),
                "{f:?}: {o:?} is not in order in the input"
            );
        }
        for i in &input {
            if !out.contains(i) {
                assert!(kind_of(*i).is_some(), "{f:?}: removed a non-video {i:?}");
            }
        }
        assert_eq!(
            hide_untabled_video(f, out.clone()),
            out,
            "{f:?} not idempotent"
        );
    }
}

/// TODAY's data (a deliberate edit of this test is the owner-ruled table arriving): the list is not
/// empty, it holds every OFA class of every family, and the classes the decoder has tables for are
/// not on it.
#[test]
fn today_every_ofa_class_of_every_family_is_listed_and_no_tabled_class_is() {
    assert!(!UNTABLED_VIDEO_CLASSES.is_empty());
    for f in Family::ALL {
        for c in f.classes().of_kind(Kind::OpticalFlow) {
            assert!(is_untabled(*c), "{f:?} {c:#x}");
        }
    }
    for c in [0xc6b0u32, 0xc7b0, 0xc7b7, 0xc9b0, 0xc9b7, 0xc5b0, 0xc5b7] {
        assert!(!is_untabled(c), "{c:#x} has a derived table");
    }
}
