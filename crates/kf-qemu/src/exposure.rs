// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **STATUS: LIVE, 2026-10-10** (`docs/design/V3_WINDOW_EXPOSURE_REVIEW.md`; owner rulings
//! `OWNER_RULINGS.md` §AB) — the owner's invariants as checks that can fail:
//!
//! > (2) In a PASSTHROUGH space nothing kayfabe- or host-owned may be mapped. Every VA mapping in
//! > a passthrough space is told by the guest's page-table leaves, and only that.
//! > (3) Only Translated channels may have host-owned mappings (windows, rings), in a T-space no
//! > Passthrough channel can ever use. (4) Only privileged channels may have all guest RAM mapped.
//!
//! A Passthrough channel runs the guest's own push buffers natively on the host GPU, so any VA in
//! its space is reachable by guest-chosen GPU work (guest userspace included). This module names
//! kayfabe's OWN placements in a space — from the space's record or from the host verbs a fake
//! host saw, never from a predicate — and reports each one that the space's kind does not allow.
//!
//! ⊘ **Corrected 2026-10-10, above the RESEARCH version it supersedes:** the research version
//! judged a mirror by whether a Passthrough birth was admissible (`KF3_TSPACE=0` admitted one
//! everywhere). With the T-space hardwired, EVERY mirror is a guest-leaf-only space (rule 2 and
//! rule 3: host-owned mappings live only in the T-space), so any kayfabe placement in any mirror
//! is a violation, whatever its twin state.
//!
//! Scope: kayfabe's placements (a store window, a guest-RAM window, the ring region). Guest rows
//! ([`crate::mem::PlacedRows`]) and SKED rows are guest-leaf placements by construction (a VA and
//! permissions from a walked guest leaf, placed only through `GpuMirror`'s `MapTarget`). ⊘ Host
//! RM's OWN placements in the space (its context buffers, its split-VAS server range) are outside
//! what kayfabe records; the review names them (rows 9-10).

use crate::mem::{Mirror, RING_REGION_BASE};

/// Where a mapping came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// A walked guest page-table leaf (a row) — the only origin a mirror may hold.
    GuestLeaf,
    /// kayfabe's window over the store (guest VRAM).
    StoreWindow,
    /// kayfabe's window over all of guest RAM.
    RamWindow,
    /// kayfabe's ring region `[RING_REGION_BASE, 2^40)` (Translated rings live in it).
    RingRegion,
    /// A VMM range the record declares that is none of the above.
    Unnamed,
}

/// One mapping in a space: `[lo, hi)` and its origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Placement {
    /// First byte.
    pub lo: u64,
    /// One past the last byte.
    pub hi: u64,
    /// What it is.
    pub origin: Origin,
}

/// What kind of host space a placement is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceKind {
    /// A mirrored guest VA space (a twin, a prewarmed or retired spare): Passthrough channels run
    /// here; only guest leaves may be mapped (§AB rule 2).
    Mirror,
    /// The T-space, reachable only with a [`crate::tspace::Privileged`] witness: its windows and
    /// rings are allowed (§AB rules 3-4).
    PrivilegedTSpace,
}

/// kayfabe's placements in a space whose record carries the store window `fb` (`(base, len)`,
/// `len == 0` for none), the guest-RAM window `ram`, and the VMM ranges `reserved` (the ranges a
/// guest leaf must avoid — [`crate::mem::GpuMirror::reserved`]). Each range once, sorted.
#[must_use]
pub fn kayfabe_placements(
    fb: (u64, u64),
    ram: Option<(u64, u64)>,
    reserved: &[(u64, u64)],
) -> Vec<Placement> {
    let fb = (fb.1 > 0).then(|| (fb.0, fb.0.saturating_add(fb.1)));
    let ram = ram.map(|(b, l)| (b, b.saturating_add(l)));
    let ring = (RING_REGION_BASE, kf_chan::host::RING_VA_LIMIT);
    let name = |r: (u64, u64)| {
        if Some(r) == fb {
            Origin::StoreWindow
        } else if Some(r) == ram {
            Origin::RamWindow
        } else if r == ring {
            Origin::RingRegion
        } else {
            Origin::Unnamed
        }
    };
    let mut set = std::collections::BTreeSet::new();
    for r in fb.into_iter().chain(ram).chain(reserved.iter().copied()) {
        set.insert(Placement {
            lo: r.0,
            hi: r.1,
            origin: name(r),
        });
    }
    set.into_iter().collect()
}

/// [`kayfabe_placements`] of a [`Mirror`] record and its VMM ranges. A mirror can record only
/// the positive control's store window ([`Mirror::negctl_window`]); every other kayfabe placement
/// would have to be in `reserved`.
#[must_use]
pub fn mirror_placements(m: &Mirror, reserved: &[(u64, u64)]) -> Vec<Placement> {
    kayfabe_placements(m.negctl_window.unwrap_or((0, 0)), None, reserved)
}

/// ★ The owner invariants for one space: every placement its kind does not allow. In a
/// [`SpaceKind::Mirror`] only [`Origin::GuestLeaf`] is allowed; in the
/// [`SpaceKind::PrivilegedTSpace`] everything kayfabe places is allowed. Empty = the invariants
/// hold for this space.
#[must_use]
pub fn violations(kind: SpaceKind, placements: &[Placement]) -> Vec<Placement> {
    match kind {
        SpaceKind::Mirror => placements
            .iter()
            .copied()
            .filter(|p| p.origin != Origin::GuestLeaf)
            .collect(),
        SpaceKind::PrivilegedTSpace => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    #[test]
    fn each_placement_is_named_once() {
        let fb = (0x1_fffe_0000_0000, 8 * GB);
        let ram = (0x1_fffc_0000_0000, 2 * GB);
        let ring = (RING_REGION_BASE, kf_chan::host::RING_VA_LIMIT);
        let reserved = [ring, (fb.0, fb.0 + fb.1), (ram.0, ram.0 + ram.1)];
        let p = kayfabe_placements(fb, Some(ram), &reserved);
        let origins: Vec<Origin> = p.iter().map(|p| p.origin).collect();
        assert_eq!(
            origins,
            vec![Origin::RingRegion, Origin::RamWindow, Origin::StoreWindow]
        );
        assert_eq!(p[0].lo, RING_REGION_BASE);
    }

    #[test]
    fn an_unrecognised_vmm_range_is_still_a_placement() {
        let p = kayfabe_placements((0, 0), None, &[(0x1000, 0x2000)]);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].origin, Origin::Unnamed);
    }

    /// The check can fail: a kayfabe placement in a mirror is reported, a guest leaf is not, and
    /// the same placements in the privileged T-space are allowed.
    #[test]
    fn a_placement_is_a_violation_only_in_a_mirror() {
        let mut p = kayfabe_placements((0x1_0000_0000, GB), Some((0x2_0000_0000, GB)), &[]);
        assert_eq!(violations(SpaceKind::Mirror, &p), p);
        assert!(violations(SpaceKind::PrivilegedTSpace, &p).is_empty());
        let leaf = Placement {
            lo: 0x1000,
            hi: 0x2000,
            origin: Origin::GuestLeaf,
        };
        assert!(violations(SpaceKind::Mirror, &[leaf]).is_empty());
        p.push(leaf);
        assert_eq!(violations(SpaceKind::Mirror, &p).len(), 2);
    }
}
