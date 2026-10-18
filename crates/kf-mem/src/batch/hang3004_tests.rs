// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Diagnostic controls for the run-3004 fixed VA warning. These exercise production
//! BatchedVas against the existing open-RM model; no GPU or production change.

use super::*;
use crate::sim::{Owner, P, Sim, space};
use std::sync::atomic::Ordering::Relaxed;

fn low_row(off: u64) -> Desired {
    Desired {
        va: 0xb_0000,
        len: 0x8_0000,
        off,
        ram: false,
        kind: 0,
        perm: kf_host::MapPerm::READ_WRITE,
        leaf: 0x1_0000,
    }
}

#[test]
fn a_fresh_low_reservation_collision_still_maps_every_requested_page() {
    let sim = Sim::new(space(0x1_0000, 0x10_0000));
    let bv = BatchedVas::with_low_reserve(&sim, true);
    let d = low_row(0x20_0000);
    assert_eq!(bv.map(&d, true), Ok(Mapped::Placed));
    assert_eq!(bv.micro_refused.load(Relaxed), 1);
    assert_eq!(bv.micro_made.load(Relaxed), 0);
    assert_eq!(bv.held_ours.load(Relaxed), 0);
    let rm = sim.0.borrow();
    assert_eq!(
        rm.n.rm_reserve, 1,
        "a fresh space already refuses this reserve"
    );
    for delta in (0..d.len).step_by(P as usize) {
        assert_eq!(
            rm.translate(d.va + delta, Owner::Mirror),
            Some((false, d.off + delta)),
            "the fallback must place both sides of the permanent reservation edge"
        );
    }
    assert!(rm.broken_mirror().is_empty());
    assert!(rm.violations.is_empty());
}

#[test]
fn repeated_low_collision_map_and_retire_cycles_leave_no_dynamic_resources() {
    let sim = Sim::new(space(0x1_0000, 0x10_0000));
    for cycle in 0..50 {
        let bv = BatchedVas::with_low_reserve(&sim, true);
        let d = low_row(0x20_0000 + cycle * 0x10_0000);
        assert_eq!(bv.map(&d, true), Ok(Mapped::Placed));
        assert_eq!(bv.micro_refused.load(Relaxed), 1);
        assert_eq!(bv.unmap_range(d.va, d.len, true), Ok(()));
        let _ = bv.drain();
        assert_eq!(bv.leftovers(), 0, "cycle {cycle}");
        let rm = sim.0.borrow();
        assert!(rm.maps.is_empty(), "cycle {cycle}: mappings");
        assert!(rm.blocks.is_empty(), "cycle {cycle}: dynamic VA blocks");
        assert!(rm.resv.is_empty(), "cycle {cycle}: micro reservations");
        assert!(rm.objs.is_empty(), "cycle {cycle}: batch objects");
        assert_eq!(rm.asserts, 0);
        assert!(rm.violations.is_empty());
    }
    assert_eq!(sim.0.borrow().n.rm_reserve, 50);
}
