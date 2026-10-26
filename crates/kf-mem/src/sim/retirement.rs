//! Free/invalidate ordering, with a warmed stale GPU translation and hostile RAM discard.

use super::*;
use std::os::fd::AsFd;

const VA: u64 = 0x100_0000;

fn batch(m: &SimMirror<'_>, low: bool) {
    let rows: Vec<Desired> = (0..8)
        .map(|i| Desired {
            va: VA + i * P,
            len: P,
            off: (100 + 3 * i) * P,
            ram: true,
            kind: 0,
            perm: MapPerm::READ_WRITE,
            leaf: P,
        })
        .collect();
    m.map_batch(&rows, true).unwrap();
    m.invalidate().unwrap();
    let mut rm = m.sim().0.borrow_mut();
    assert_eq!(rm.objs.len(), 1);
    assert_eq!(rm.resv.len(), usize::from(low));
    rm.warm_tlb(VA);
    rm.reuse_on_free = true;
}

#[test]
fn deferred_range_keeps_batch_pinned_until_invalidate_completes() {
    let sim = Sim::new(space(VA, VA + 16 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, false);
    m.unmap_range(VA, 8 * P, true).unwrap();
    assert_eq!(sim.0.borrow().objs.len(), 1, "batch pin outlives stale TLB");
    assert_eq!(sim.0.borrow().n.rm_free, 0);
    m.invalidate().unwrap();
    let rm = sim.0.borrow();
    assert!(
        rm.objs.is_empty(),
        "successful invalidate releases the batch"
    );
    assert!(rm.premature_frees.is_empty());
    assert_eq!(rm.gpu_access(VA), Ok(None));
}

#[test]
fn hostile_discard_cannot_reuse_a_page_reachable_by_a_stale_translation() {
    let sim = Sim::new(space(VA, VA + 16 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, false);
    m.unmap_range(VA, 8 * P, true).unwrap();
    assert_eq!(sim.0.borrow().gpu_access(VA), Ok(Some((true, 100 * P))));
    m.invalidate().unwrap();
    assert_eq!(sim.0.borrow().gpu_access(VA), Ok(None));
    assert!(sim.0.borrow().reused.contains(&(true, 100 * P)));
}

#[test]
fn refused_invalidate_keeps_batch_pins_for_the_retry() {
    let sim = Sim::new(space(VA, VA + 16 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, false);
    m.unmap_range(VA, 8 * P, true).unwrap();
    sim.0.borrow_mut().refuse_invalidate = true;
    assert!(m.invalidate().is_err());
    assert_eq!(sim.0.borrow().objs.len(), 1);
    assert_eq!(sim.0.borrow().gpu_access(VA), Ok(Some((true, 100 * P))));
    sim.0.borrow_mut().refuse_invalidate = false;
    m.invalidate().unwrap();
    assert!(sim.0.borrow().objs.is_empty());
}

#[test]
fn repeated_refused_invalidates_never_allow_hostile_page_reuse() {
    let sim = Sim::new(space(VA, VA + 16 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, false);
    m.unmap_range(VA, 8 * P, true).unwrap();
    sim.0.borrow_mut().refuse_invalidate = true;
    for _ in 0..3 {
        assert!(m.invalidate().is_err());
        assert!(sim.0.borrow().reused.is_empty());
        assert_eq!(sim.0.borrow().gpu_access(VA), Ok(Some((true, 100 * P))));
    }
    sim.0.borrow_mut().refuse_invalidate = false;
    m.invalidate().unwrap();
    assert!(sim.0.borrow().premature_frees.is_empty());
    assert!(
        sim.0.borrow().objs.is_empty(),
        "retry must finish the retained retirements"
    );
}

#[test]
fn deferred_unmap_keeps_the_empty_micro_reservation_until_completion() {
    let sim = Sim::new(space(VA + 32 * P, VA + 64 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, true);
    m.unmap_range(VA, 8 * P, true).unwrap();
    assert_eq!(sim.0.borrow().resv.len(), 1);
    m.invalidate().unwrap();
    assert!(sim.0.borrow().resv.is_empty());
    assert!(sim.0.borrow().premature_frees.is_empty());
}

#[test]
fn per_run_unmap_does_not_free_the_micro_before_stale_translation_is_gone() {
    let sim = Sim::new(space(VA + 32 * P, VA + 64 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, true);
    for i in 0..8 {
        m.unmap(VA + i * P, true).unwrap();
    }
    assert!(sim.0.borrow().premature_frees.is_empty());
    assert_eq!(sim.0.borrow().resv.len(), 1);
    m.invalidate().unwrap();
    assert!(sim.0.borrow().objs.is_empty());
    assert!(sim.0.borrow().resv.is_empty());
}

#[test]
fn stale_translation_model_detects_early_unpin_and_reuse() {
    let sim = Sim::new(space(VA, VA + 16 * P));
    let rows = [Desired {
        va: VA,
        len: P,
        off: 100 * P,
        ram: true,
        kind: 0,
        perm: MapPerm::READ_WRITE,
        leaf: P,
    }];
    let h = SpaceVerbs::map_scattered(&&sim, std::io::stdin().as_fd(), &rows, true).unwrap();
    let mut rm = sim.0.borrow_mut();
    rm.warm_tlb(VA);
    rm.reuse_on_free = true;
    rm.unmap_range_in(RESV, VA, VA + P, true);
    assert_eq!(rm.gpu_access(VA), Ok(Some((true, 100 * P))));
    rm.free(h).unwrap();
    assert_eq!(rm.premature_frees, vec![h]);
    assert!(
        rm.gpu_access(VA).is_err(),
        "negative control catches reused memory"
    );
    rm.invalidate().unwrap();
    assert_eq!(rm.gpu_access(VA), Ok(None));
}

#[test]
fn an_empty_retry_refresh_finishes_pending_batch_retirement() {
    let sim = Sim::new(space(VA, VA + 16 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, false);
    m.unmap_range(VA, 8 * P, true).unwrap();
    let ram = |off, _len| Some(off);
    let out = apply_entry(&m, &[], &cfg(&ram));
    assert!(
        out.invalidated,
        "pending pins require an invalidate without new rows"
    );
    assert!(sim.0.borrow().objs.is_empty());
    assert!(sim.0.borrow().premature_frees.is_empty());
}

#[test]
fn an_empty_retry_refresh_finishes_pending_micro_retirement() {
    let sim = Sim::new(space(VA + 32 * P, VA + 64 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, true);
    m.unmap_range(VA, 8 * P, true).unwrap();
    let ram = |off, _len| Some(off);
    let out = apply_entry(&m, &[], &cfg(&ram));
    assert!(out.invalidated);
    assert!(sim.0.borrow().resv.is_empty());
    assert!(sim.0.borrow().premature_frees.is_empty());
}

#[test]
fn retiring_a_space_with_a_refused_invalidate_preserves_its_batch_pin() {
    let sim = Sim::new(space(VA, VA + 16 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, false);
    sim.0.borrow_mut().refuse_invalidate = true;
    m.retire();
    assert_eq!(sim.0.borrow().objs.len(), 1);
    assert!(
        m.bv.leftovers() > 0,
        "a failed retire cannot recycle the space"
    );
    assert_eq!(sim.0.borrow().gpu_access(VA), Ok(Some((true, 100 * P))));
    sim.0.borrow_mut().refuse_invalidate = false;
    m.retire();
    assert!(sim.0.borrow().objs.is_empty());
    assert_eq!(m.bv.leftovers(), 0);
}

#[test]
fn retiring_a_space_with_a_refused_invalidate_preserves_its_micro_and_backing() {
    let sim = Sim::new(space(VA + 32 * P, VA + 64 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, true);
    sim.0.borrow_mut().refuse_invalidate = true;
    m.retire();
    let rm = sim.0.borrow();
    assert_eq!(rm.resv.len(), 1);
    assert_eq!(rm.objs.len(), 1);
    assert!(rm.premature_frees.is_empty());
    assert_eq!(rm.gpu_access(VA), Ok(Some((true, 100 * P))));
    drop(rm);
    sim.0.borrow_mut().refuse_invalidate = false;
    m.retire();
    assert!(sim.0.borrow().resv.is_empty());
    assert_eq!(m.bv.leftovers(), 0);
}

#[test]
fn repeated_partial_unmaps_do_not_accumulate_duplicate_micro_pins() {
    let sim = Sim::new(space(VA + 32 * P, VA + 64 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, true);
    for i in 0..4 {
        m.unmap_range(VA + i * P, P, true).unwrap();
        assert_eq!(m.bv.micro.lock().unwrap().values().next().unwrap().pins, 1);
    }
    m.invalidate().unwrap();
    assert_eq!(m.bv.micro.lock().unwrap().values().next().unwrap().pins, 0);
}

#[test]
fn refused_invalidates_do_not_accumulate_duplicate_micro_pins() {
    let sim = Sim::new(space(VA + 32 * P, VA + 64 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, true);
    sim.0.borrow_mut().refuse_invalidate = true;
    for i in 0..4 {
        m.unmap_range(VA + i * P, P, true).unwrap();
        assert!(m.invalidate().is_err());
        assert_eq!(m.bv.micro.lock().unwrap().values().next().unwrap().pins, 1);
        assert!(sim.0.borrow().reused.is_empty());
    }
    sim.0.borrow_mut().refuse_invalidate = false;
    m.invalidate().unwrap();
    assert_eq!(m.bv.micro.lock().unwrap().values().next().unwrap().pins, 0);
}

#[test]
fn a_failed_unmap_that_acted_still_requires_invalidate_before_drain_frees_backing() {
    let sim = Sim::new(space(VA, VA + 16 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, false);
    sim.0.borrow_mut().fail_unmaps = Some(UnmapFault {
        every: 1,
        after: true,
        seen: 0,
    });
    assert!(m.unmap_range(VA, 8 * P, true).is_err());
    assert!(sim.0.borrow().maps.is_empty());
    assert_eq!(sim.0.borrow().objs.len(), 1);
    m.retire();
    assert!(sim.0.borrow().premature_frees.is_empty());
    assert_eq!(sim.0.borrow().gpu_access(VA), Ok(None));
    assert!(sim.0.borrow().objs.is_empty());
}

#[test]
fn a_failed_unmap_and_refused_invalidate_leave_the_booked_batch_pinned_at_retire() {
    let sim = Sim::new(space(VA, VA + 16 * P));
    let m = SimMirror::new(&sim, true, true);
    batch(&m, false);
    sim.0.borrow_mut().fail_unmaps = Some(UnmapFault {
        every: 1,
        after: true,
        seen: 0,
    });
    assert!(m.unmap_range(VA, 8 * P, true).is_err());
    sim.0.borrow_mut().refuse_invalidate = true;
    m.retire();
    assert_eq!(sim.0.borrow().objs.len(), 1);
    assert_eq!(sim.0.borrow().gpu_access(VA), Ok(Some((true, 100 * P))));
    assert!(m.bv.leftovers() > 0);
    assert!(sim.0.borrow().premature_frees.is_empty());
}

#[test]
fn stale_translation_model_detects_early_unpin_after_a_refused_micro_invalidate() {
    let sim = Sim::new(space(VA + 32 * P, VA + 64 * P));
    let resv = SpaceVerbs::reserve(&&sim, VA, 16 * P).unwrap();
    let rows = [Desired {
        va: VA,
        len: P,
        off: 200 * P,
        ram: true,
        kind: 0,
        perm: MapPerm::READ_WRITE,
        leaf: P,
    }];
    let obj =
        SpaceVerbs::map_scattered_in(&&sim, resv, std::io::stdin().as_fd(), &rows, true).unwrap();
    let mut rm = sim.0.borrow_mut();
    rm.warm_tlb(VA);
    rm.reuse_on_free = true;
    rm.unmap_range_in(resv, VA, VA + P, true);
    rm.refuse_invalidate = true;
    assert!(rm.invalidate().is_err());
    assert_eq!(rm.gpu_access(VA), Ok(Some((true, 200 * P))));
    rm.free(obj).unwrap();
    assert!(rm.premature_frees.contains(&obj));
    assert!(
        rm.gpu_access(VA).is_err(),
        "refusal cannot make early unpin safe"
    );
    rm.refuse_invalidate = false;
    rm.invalidate().unwrap();
    assert_eq!(rm.gpu_access(VA), Ok(None));
}
