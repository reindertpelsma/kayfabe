// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **STATUS: RESEARCH, 2026-10-10** (`docs/design/V3_WINDOW_EXPOSURE_REVIEW.md`) — the owner's
//! invariant of 2026-10-10 as a check that can fail:
//!
//! > In a PASSTHROUGH space nothing kayfabe- or host-owned may be mapped. Every VA mapping in a
//! > passthrough space is told by the guest's page-table leaves, and only that.
//!
//! A Passthrough channel runs the guest's own push buffers natively on the host GPU, so any VA in
//! its space is reachable by guest-chosen GPU work (guest userspace included). This module names
//! kayfabe's OWN placements in a mirrored space — from the space's record, never from a predicate
//! — and reports each one in a space a Passthrough channel can be born in.
//!
//! Scope: kayfabe's placements (the store window, the guest-RAM window, the ring region). Guest
//! rows ([`crate::mem::PlacedRows`]) and SKED rows are guest-leaf placements by construction (a VA
//! and permissions from a walked guest leaf). ⊘ Host RM's OWN placements in the space (its context
//! buffers, its split-VAS server range) are outside what kayfabe records; the review names them.

use crate::mem::{Mirror, RING_REGION_BASE};

/// Where a mapping in a mirrored space came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// kayfabe's identity window over the store (guest VRAM, plus the firmware carve-out on the
    /// default path).
    StoreWindow,
    /// kayfabe's window over all of guest RAM.
    RamWindow,
    /// kayfabe's ring region `[RING_REGION_BASE, 2^40)`: Translated rings (push buffer, GPFIFO,
    /// fence, USERD) are placed in it.
    RingRegion,
    /// A VMM range the record declares that is none of the above.
    Unnamed,
}

/// One of kayfabe's own placements in a space: `[lo, hi)` and its origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Placement {
    /// First byte.
    pub lo: u64,
    /// One past the last byte.
    pub hi: u64,
    /// What it is.
    pub origin: Origin,
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

/// [`kayfabe_placements`] of a [`Mirror`] record and its VMM ranges.
#[must_use]
pub fn mirror_placements(m: &Mirror, reserved: &[(u64, u64)]) -> Vec<Placement> {
    kayfabe_placements((m.fb_base, m.fb_len), m.ram, reserved)
}

/// Whether a Passthrough channel can be born in a space with twin state `twin` — the gate the
/// channel plane applies at a passthrough birth (`crate::chan`, the `birth passthrough` arm): in
/// T-mode (`KF3_TSPACE=1`) a birth in a guest-KERNEL space is refused by name
/// (`TwinState::try_user`); on the default path no gate applies, so EVERY mirror admits one.
#[must_use]
pub fn passthrough_admissible(tmode: bool, twin: &crate::twin::TwinState) -> bool {
    !(tmode && twin.is_kernel())
}

/// ★ The owner invariant: every placement of kayfabe's in a space a Passthrough channel can be
/// born in is a violation. Empty = the invariant holds for this space.
#[must_use]
pub fn violations(passthrough: bool, placements: &[Placement]) -> Vec<Placement> {
    if passthrough {
        placements.to_vec()
    } else {
        Vec::new()
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
        let reserved = crate::mem::vmm_ranges(Some(fb), Some(ram));
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

    /// The check can fail: a placement in a passthrough-admissible space is reported, and the
    /// same placement in a space no passthrough channel can be born in is not.
    #[test]
    fn a_placement_is_a_violation_only_where_passthrough_can_run() {
        let p = kayfabe_placements((0x1_0000_0000, GB), None, &[]);
        assert_eq!(violations(true, &p), p);
        assert!(violations(false, &p).is_empty());
        assert!(violations(true, &[]).is_empty());
    }

    #[test]
    fn only_a_tmode_kernel_space_refuses_passthrough() {
        let user = crate::twin::TwinState::for_kernel(false);
        let kernel = crate::twin::TwinState::for_kernel(true);
        assert!(passthrough_admissible(true, &user));
        assert!(!passthrough_admissible(true, &kernel));
        // ⊘ The default path gates nothing: a guest-KERNEL space admits a passthrough birth.
        assert!(passthrough_admissible(false, &user));
        assert!(passthrough_admissible(false, &kernel));
        // ★ Against the real gate: in T-mode `try_user` refuses exactly where this says false.
        assert!(user.try_user().is_ok());
        assert!(kernel.try_user().is_err());
    }
}
