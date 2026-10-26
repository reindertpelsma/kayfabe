use super::*;
use std::cell::{Cell, RefCell};

const LO: u64 = 1 << 40;
const HI: u64 = LO + 8 * NORMAL_GRAIN;

#[derive(Default)]
struct Rm {
    blocks: RefCell<BTreeMap<u32, (u64, u64, bool)>>,
    maps: RefCell<BTreeMap<u64, u64>>,
    next: Cell<u32>,
    calls: Cell<usize>,
    fail: Cell<usize>,
    guard: RefCell<BTreeMap<u64, Option<u64>>>,
}

impl Rm {
    fn new() -> Self {
        let r = Self::default();
        r.blocks.borrow_mut().insert(1, (LO, HI, false));
        r.next.set(2);
        r
    }
    // Some(0) is sparse; backed offsets start at PAGE. None is plain invalid.
    fn state(&self, va: u64) -> Option<u64> {
        self.maps.borrow().get(&va).copied().or_else(|| {
            self.blocks
                .borrow()
                .values()
                .any(|&(lo, hi, sparse)| sparse && lo <= va && va < hi)
                .then_some(0)
        })
    }
    fn check(&self) {
        for (&va, &state) in self.guard.borrow().iter() {
            assert_eq!(self.state(va), state, "unchanged {va:#x}");
        }
    }
    fn backed(&self, va: u64, len: u64) {
        for p in (va..va + len).step_by(PAGE as usize) {
            self.maps.borrow_mut().insert(p, PAGE + p - va);
        }
        self.check();
    }
    fn unmap(&self, va: u64, len: u64) {
        self.maps
            .borrow_mut()
            .retain(|&p, _| p < va || p >= va + len);
        self.check();
    }
}
impl Ops for Rm {
    fn reserve(&self, lo: u64, len: u64, sparse: bool) -> Result<u32, RmError> {
        let n = self.calls.get() + 1;
        self.calls.set(n);
        if n == self.fail.get() {
            return Err(RmError::NoMemory);
        }
        assert!(len > 0 && lo.checked_add(len).is_some());
        assert!(
            !self
                .blocks
                .borrow()
                .values()
                .any(|&(a, b, _)| lo < b && a < lo + len),
            "RM rejects reservations inside a live reservation"
        );
        if sparse {
            assert_eq!(len, SPARSE_GRAIN);
            assert_eq!(lo % SPARSE_GRAIN, 0);
        }
        let h = self.next.get();
        self.next.set(h + 1);
        self.blocks.borrow_mut().insert(h, (lo, lo + len, sparse));
        self.check();
        Ok(h)
    }
    fn release(&self, h: u32) -> Result<(), RmError> {
        let (lo, hi, _) = self
            .blocks
            .borrow_mut()
            .remove(&h)
            .expect("own reservation");
        assert!(
            !self.maps.borrow().keys().any(|&va| lo <= va && va < hi),
            "a populated block must never be freed"
        );
        self.check();
        Ok(())
    }
}
fn tree(limit: usize) -> Reservations {
    Reservations::new(
        &[GuestVaRange {
            handle: 1,
            lo: LO,
            hi: HI,
        }],
        limit,
    )
}
fn populate(t: &mut Reservations, r: &Rm, va: u64, len: u64) {
    t.normal(r, va, len).unwrap();
    t.mark(va, len, true);
    r.backed(va, len);
}
fn block(t: &Reservations, va: u64) -> &Block {
    t.blocks.range(..=va).next_back().unwrap().1
}

#[test]
fn first_vidmem_population_is_a_two_mib_tile() {
    let (mut t, r) = (tree(100), Rm::new());
    populate(&mut t, &r, LO + NORMAL_GRAIN, PAGE);
    assert_eq!(
        block(&t, LO + NORMAL_GRAIN).hi - block(&t, LO + NORMAL_GRAIN).lo,
        NORMAL_GRAIN
    );
    assert!(r.calls.get() >= 2);
}
#[test]
fn first_dma_population_at_the_far_edge_is_a_two_mib_tile() {
    let (mut t, r) = (tree(100), Rm::new());
    populate(&mut t, &r, HI - PAGE, PAGE);
    assert_eq!(
        block(&t, HI - PAGE).hi - block(&t, HI - PAGE).lo,
        NORMAL_GRAIN
    );
}
fn populated_fallback(va: u64) {
    let (mut t, r) = (tree(100), Rm::new());
    populate(&mut t, &r, va, PAGE);
    r.guard.borrow_mut().insert(va, Some(PAGE));
    assert!(!t.admit(&r, va, SPARSE_GRAIN).unwrap());
    assert_eq!(r.state(va + PAGE), None);
    assert_eq!(t.stats.unhonoured_bytes, SPARSE_GRAIN);
    assert!(!block(&t, va).sparse);
}
#[test]
fn populated_normal_vidmem_stays_invalid_around_the_backing() {
    populated_fallback(LO);
}
#[test]
fn populated_normal_dma_stays_invalid_at_another_tile() {
    populated_fallback(LO + 3 * NORMAL_GRAIN);
}

fn exact_sparse(va: u64) {
    let (mut t, r) = (tree(100), Rm::new());
    r.guard.borrow_mut().insert(va - PAGE, None);
    r.guard.borrow_mut().insert(va + SPARSE_GRAIN, None);
    assert!(t.admit(&r, va, SPARSE_GRAIN).unwrap());
    assert_eq!(r.state(va), Some(0));
    assert!(block(&t, va).sparse);
    assert_eq!(block(&t, va).hi - block(&t, va).lo, SPARSE_GRAIN);
}
#[test]
fn one_sparse_tile_never_absorbs_a_plain_invalid_neighbour() {
    exact_sparse(LO + SPARSE_GRAIN);
}
#[test]
fn a_sparse_tile_at_a_normal_boundary_is_exact() {
    exact_sparse(LO + NORMAL_GRAIN);
}
fn partial_sparse(va: u64) {
    let (mut t, r) = (tree(100), Rm::new());
    r.guard.borrow_mut().insert(va - PAGE, None);
    r.guard.borrow_mut().insert(va + PAGE, None);
    assert!(!t.admit(&r, va, PAGE).unwrap());
    assert_eq!(r.state(va), None);
    assert_eq!(t.stats.unhonoured_bytes, PAGE);
}
#[test]
fn an_isolated_small_sparse_leaf_stays_invalid() {
    partial_sparse(LO + PAGE);
}
#[test]
fn rounding_an_edge_sparse_leaf_would_absorb_a_negative_control() {
    partial_sparse(LO + SPARSE_GRAIN - PAGE);
}
#[test]
fn sixteen_derived_small_leaves_can_fill_one_exact_sparse_tile() {
    let (mut t, r) = (tree(100), Rm::new());
    for i in 0..16 {
        assert_eq!(t.admit(&r, LO + i * PAGE, PAGE).unwrap(), i == 15);
    }
    assert_eq!(r.state(LO), Some(0));
    assert_eq!(r.state(LO + SPARSE_GRAIN), None);
}
fn transitions(va: u64) {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.admit(&r, va, SPARSE_GRAIN).unwrap());
    r.guard
        .borrow_mut()
        .insert(va + SPARSE_GRAIN - PAGE, Some(0));
    t.retire(&r, va, PAGE, true).unwrap();
    populate(&mut t, &r, va, PAGE);
    assert_eq!(r.state(va), Some(PAGE));
    assert!(t.admit(&r, va, PAGE).unwrap());
    r.unmap(va, PAGE);
    t.mark(va, PAGE, false);
    assert_eq!(r.state(va), Some(0));
}
#[test]
fn sparse_vidmem_sparse_is_two_direct_pte_transitions() {
    transitions(LO);
}
#[test]
fn sparse_dma_sparse_keeps_other_sparse_pages_accessible() {
    transitions(LO + 2 * NORMAL_GRAIN);
}
#[test]
fn all_sparse_intent_removed_frees_only_the_empty_block() {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.admit(&r, LO, SPARSE_GRAIN).unwrap());
    assert_eq!(t.retire(&r, LO, SPARSE_GRAIN, false).unwrap(), 0);
    assert_eq!(r.state(LO), None);
    assert!(!block(&t, LO).sparse);
}
#[test]
fn partial_retirement_preserves_unchanged_sparse_intent() {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.admit(&r, LO, SPARSE_GRAIN).unwrap());
    r.guard.borrow_mut().insert(LO + PAGE, Some(0));
    assert_eq!(t.retire(&r, LO, PAGE, false).unwrap(), PAGE);
    assert_eq!(r.state(LO), Some(0));
    assert_eq!(t.stats.retained_bytes, PAGE);
}
#[test]
fn full_sparse_retirement_preserves_a_backed_neighbour() {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.admit(&r, LO, SPARSE_GRAIN).unwrap());
    populate(&mut t, &r, LO + PAGE, PAGE);
    r.guard.borrow_mut().insert(LO + PAGE, Some(PAGE));
    assert_eq!(t.retire(&r, LO, SPARSE_GRAIN, false).unwrap(), SPARSE_GRAIN);
    assert_eq!(r.state(LO), Some(0));
}
fn budget(va: u64) {
    let (mut t, r) = (tree(1), Rm::new());
    assert!(t.admit(&r, va, SPARSE_GRAIN).is_err());
    assert_eq!(r.calls.get(), 0);
    assert_eq!(t.blocks.len(), 1);
    assert_eq!(t.stats.overflow, 1);
    assert_eq!(r.state(va), None);
}
#[test]
fn sparse_object_exhaustion_is_refused_before_free() {
    budget(LO);
}
#[test]
fn sparse_object_exhaustion_in_the_middle_preserves_the_original_reservation() {
    budget(LO + NORMAL_GRAIN);
}
#[test]
fn normal_object_exhaustion_never_places_a_backing_in_a_coarse_block() {
    let (mut t, r) = (tree(1), Rm::new());
    assert!(t.normal(&r, LO + NORMAL_GRAIN, PAGE).is_err());
    assert_eq!(r.calls.get(), 0);
    assert_eq!(t.stats.overflow, 1);
}
fn restore(failure: usize) {
    let (mut t, r) = (tree(100), Rm::new());
    r.fail.set(failure);
    assert!(t.admit(&r, LO + NORMAL_GRAIN, SPARSE_GRAIN).is_err());
    assert!(!t.poisoned);
    assert_eq!(t.stats.refused, 1);
    for va in (LO..HI).step_by(SPARSE_GRAIN as usize) {
        assert!(
            r.blocks
                .borrow()
                .values()
                .any(|&(a, b, _)| a <= va && va < b)
        );
        assert_eq!(r.state(va), None);
    }
}
#[test]
fn first_rm_reservation_refusal_restores_empty_normal_coverage() {
    restore(1);
}
#[test]
fn a_later_rm_refusal_restores_all_coarse_siblings() {
    restore(2);
}
#[test]
fn held_intent_is_retried_only_after_the_normal_block_empties() {
    let (mut t, r) = (tree(100), Rm::new());
    populate(&mut t, &r, LO, PAGE);
    assert!(!t.admit(&r, LO, SPARSE_GRAIN).unwrap());
    r.unmap(LO, PAGE);
    t.mark(LO, PAGE, false);
    assert_eq!(r.state(LO), None);
    t.retry(&r).unwrap();
    assert_eq!(r.state(LO), Some(0));
}
#[test]
fn cancelled_held_intent_never_appears_on_a_later_refresh() {
    let (mut t, r) = (tree(100), Rm::new());
    populate(&mut t, &r, LO, PAGE);
    assert!(!t.admit(&r, LO, SPARSE_GRAIN).unwrap());
    t.retire(&r, LO, SPARSE_GRAIN, false).unwrap();
    r.unmap(LO, PAGE);
    t.mark(LO, PAGE, false);
    t.retry(&r).unwrap();
    assert_eq!(r.state(LO), None);
}
#[test]
fn pieces_route_through_the_new_sparse_handle() {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.admit(&r, LO + SPARSE_GRAIN, SPARSE_GRAIN).unwrap());
    let p = t.pieces(99, LO, 3 * SPARSE_GRAIN).unwrap();
    assert_eq!(p.len(), 3);
    assert_eq!(p.iter().map(|x| x.2).sum::<u64>(), 3 * SPARSE_GRAIN);
    assert_ne!(p[0].0, p[1].0);
    assert_eq!(p[1].0, block(&t, LO + SPARSE_GRAIN).handle);
}
#[test]
fn pieces_route_a_backed_row_across_two_normal_tiles() {
    let (mut t, r) = (tree(100), Rm::new());
    t.normal(&r, LO + NORMAL_GRAIN - PAGE, 2 * PAGE).unwrap();
    let p = t.pieces(99, LO + NORMAL_GRAIN - PAGE, 2 * PAGE).unwrap();
    assert_eq!(p.len(), 2);
    assert_ne!(p[0].0, p[1].0);
}
#[test]
fn sparse_wrapping_range_is_refused() {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.admit(&r, u64::MAX - PAGE + 1, 2 * PAGE).is_err());
    assert_eq!(r.calls.get(), 0);
}
#[test]
fn normal_wrapping_range_is_refused() {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.normal(&r, u64::MAX - PAGE + 1, 2 * PAGE).is_err());
    assert_eq!(r.calls.get(), 0);
}
#[test]
fn sparse_null_page_is_refused() {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.admit(&r, 0, SPARSE_GRAIN).is_err());
}
#[test]
fn normal_null_page_is_refused() {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.normal(&r, 0, PAGE).is_err());
}
#[test]
fn switch_is_default_off_and_requires_exact_one() {
    for v in [None, Some(""), Some("true"), Some("01"), Some("0")] {
        assert!(!enabled_from(v));
    }
    assert!(enabled_from(Some("1")));
}
