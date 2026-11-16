// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! STATUS: LIVE 2026-11-15. The video classes that have NO derived method table, and the rule that
//! follows from the list: **an engine whose classes have no derived method table is not offered to a
//! guest that builds kernel rings on it.**
//!
//! # Why this list exists
//!
//! A guest-KERNEL channel on a video engine (NVDEC, NVENC, OFA) is born Translated on a host ring
//! whose owned object has the family's video class, and `kf_chan::tmode::video_write` decodes the
//! ring's subchannel 0-4 methods against `kf_chan::vtables::VideoClass::of_class(class)`. A class with
//! no table makes the decoder refuse `ForeignClass` on the ring's FIRST method; the channel dies
//! and the guest's watchdog fires (measured: Windows, OFA class `0xC9FA`, ~9 s later, every
//! frame-generation boot; `docs/design/V3_VIDEO_ENGINES.md`, 2026-11-15 amendment). The tables are
//! DERIVED from NVIDIA's public class headers (`tools/derive_video_methods.py`); the public headers
//! define no OFA method layout, and no OFA table entry may be added, captured or aliased without an
//! owner ruling (`docs/OWNER_RULINGS.md`: derive, never capture; no forged completion; no undecoded
//! pass-through of guest-built ring contents).
//!
//! # One list, two readers (the two cannot drift)
//!
//! [`UNTABLED_VIDEO_CLASSES`] is read by
//!
//! 1. `kf_rm::hostquery` through [`kernel_ring_servable`], when the device is realized with the
//!    `KF3_WIN_HIDE_UNTABLED_VIDEO=1` profile (`kf_rm::hostquery::HostFactsPolicy`): an engine kind
//!    that fails it is left out of the engine list the guest is served, so every statement the guest
//!    derives its engine set from (FIFO rows, `engineCaps`, falcon table, interrupt table, PRI bases)
//!    omits it together, exactly as for a host that lacks the engine; and
//! 2. `crates/kf-chan/tests/video_class_coverage.rs`, which fails unless this list is EXACTLY the
//!    set of video classes a guest-kernel ring can hold that `VideoClass::of_class` has no table for
//!    (a class that gains a table must LEAVE the list; a new uncovered class must JOIN it). So the
//!    offer filter can never be wider or narrower than the decoder's real coverage.
//!
//! ⊘ This is a filter over a coverage list, not a table: it holds no method, no offset and no
//! disposition, and adding a class here never makes the decoder accept anything.
//!
//! # What the rule does NOT say
//!
//! It is keyed to the guest-KERNEL ring route only. A guest USER channel on a video engine is a
//! passthrough twin (no method table is read; measured on a Linux guest, `V3_GFX_TESTSET.md` 4.1),
//! so on a guest that creates no kernel ring on the engine, hiding it would only cost the guest a
//! working feature (`VK_NV_optical_flow`). That is why the filter is a declared profile
//! (`KF3_WIN_HIDE_UNTABLED_VIDEO`, off by default, set by the Windows run profile) and not the
//! default: the guest OS is never detected from the wire (`kf_abi::guest_os`, section 2).

use crate::Family;
use crate::classes::Kind;

/// Video classes a guest-kernel ring can hold that have NO derived method table today, each with a
/// one-line reason that names the report. Exact match with the decoder's coverage: see the module
/// comment. Sorted by class id.
pub const UNTABLED_VIDEO_CLASSES: &[(u32, &str)] = &[
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

/// Whether `class` is a video class with no derived method table.
#[must_use]
pub fn is_untabled(class: u32) -> bool {
    UNTABLED_VIDEO_CLASSES.iter().any(|&(c, _)| c == class)
}

/// Whether a guest-kernel ring on a video engine of `kind` can be served on `family` from a DERIVED
/// method table: the family lists at least one class of the kind, and EVERY class it lists is
/// tabled. (A host ring holds the newest class of the kind that the host supports, so a kind with
/// any untabled class could land on it; the conservative answer is "not servable". No family has a
/// mixed kind today, the test `no_family_mixes_tabled_and_untabled_classes_of_a_kind` pins that.)
///
/// A kind that is not a video kind is not governed by this rule and answers `true`.
#[must_use]
pub fn kernel_ring_servable(family: Family, kind: Kind) -> bool {
    if !matches!(
        kind,
        Kind::VideoDecoder | Kind::VideoEncoder | Kind::OpticalFlow
    ) {
        return true;
    }
    let classes = family.classes().of_kind(kind);
    !classes.is_empty() && classes.iter().all(|c| !is_untabled(*c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_sorted_unique_and_one_line() {
        let mut prev = None;
        for &(class, reason) in UNTABLED_VIDEO_CLASSES {
            if let Some(p) = prev {
                assert!(p < class, "{p:#x} then {class:#x}: sorted, no duplicates");
            }
            prev = Some(class);
            assert!(!reason.trim().is_empty() && !reason.contains(['\n', '\r']));
        }
    }

    #[test]
    fn every_listed_class_is_a_video_class_of_some_family() {
        for &(class, _) in UNTABLED_VIDEO_CLASSES {
            assert!(
                Family::ALL.iter().any(|f| {
                    [Kind::VideoDecoder, Kind::VideoEncoder, Kind::OpticalFlow]
                        .iter()
                        .any(|k| f.classes().of_kind(*k).contains(&class))
                }),
                "{class:#x} is no family's video class"
            );
        }
    }

    #[test]
    fn a_non_video_kind_is_never_governed() {
        for f in Family::ALL {
            for k in Kind::ALL {
                if !matches!(
                    k,
                    Kind::VideoDecoder | Kind::VideoEncoder | Kind::OpticalFlow
                ) {
                    assert!(kernel_ring_servable(f, k), "{f:?} {k:?}");
                }
            }
        }
    }

    #[test]
    fn no_family_mixes_tabled_and_untabled_classes_of_a_kind() {
        for f in Family::ALL {
            for k in [Kind::VideoDecoder, Kind::VideoEncoder, Kind::OpticalFlow] {
                let cs = f.classes().of_kind(k);
                let n = cs.iter().filter(|c| is_untabled(**c)).count();
                assert!(n == 0 || n == cs.len(), "{f:?} {k:?}: {cs:x?}");
            }
        }
    }

    #[test]
    fn an_empty_class_list_is_not_servable() {
        // Turing lists no OFA class; Hopper lists no NVENC class.
        assert!(Family::Turing.classes().optical_flow.is_empty());
        assert!(!kernel_ring_servable(Family::Turing, Kind::OpticalFlow));
        assert!(Family::Hopper.classes().video_encoder.is_empty());
        assert!(!kernel_ring_servable(Family::Hopper, Kind::VideoEncoder));
    }
}
