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
    fail_from: Cell<usize>,
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
        let from = self.fail_from.get();
        if n == self.fail.get() || (from != 0 && n >= from) {
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
    assert_eq!(t.admit(&r, va, SPARSE_GRAIN), Ok(false));
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
    assert_eq!(
        t.stats.overflow, 0,
        "a plain refinement decline is not an overflow"
    );
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

fn late_admission_refusal(failure: usize) {
    let (mut t, r) = (tree(100), Rm::new());
    r.fail.set(failure);
    assert!(t.admit(&r, LO, 2 * SPARSE_GRAIN).is_err());
    assert!(!t.poisoned);
    for p in (LO..LO + 2 * SPARSE_GRAIN).step_by(PAGE as usize) {
        assert_eq!(
            r.state(p),
            None,
            "failed admission leaves no new sparse coverage"
        );
    }
    for b in t.blocks.values() {
        assert!(b.wanted.iter().all(|&v| v == 0));
    }
}
#[test]
fn a_late_sparse_admission_refusal_withdraws_the_first_new_tile() {
    late_admission_refusal(5);
}
#[test]
fn a_refused_sparse_suffix_withdraws_all_new_intent() {
    late_admission_refusal(6);
}

fn full_replacement(va: u64) {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.admit(&r, va, SPARSE_GRAIN).unwrap());
    // Replacing the entire sparse intent must leave sparse PTEs until the map.
    r.guard.borrow_mut().insert(va, Some(0));
    assert_eq!(t.retire(&r, va, SPARSE_GRAIN, true).unwrap(), 0);
    r.guard.borrow_mut().clear();
    populate(&mut t, &r, va, SPARSE_GRAIN);
    assert_eq!(r.state(va), Some(PAGE));
}
#[test]
fn a_full_sparse_to_vidmem_replacement_has_no_invalid_gap() {
    full_replacement(LO);
}
#[test]
fn a_full_sparse_to_dma_replacement_has_no_invalid_gap() {
    full_replacement(LO + NORMAL_GRAIN);
}
#[test]
fn retirement_of_two_pages_preserves_the_other_sparse_intents() {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.admit(&r, LO, SPARSE_GRAIN).unwrap());
    r.guard.borrow_mut().insert(LO + 3 * PAGE, Some(0));
    assert_eq!(t.retire(&r, LO, 2 * PAGE, false).unwrap(), 2 * PAGE);
    assert_eq!(t.stats.retained_bytes, 2 * PAGE);
}
#[test]
fn sparse_retirement_preserves_a_backing_at_the_far_edge() {
    let (mut t, r) = (tree(100), Rm::new());
    let va = LO + NORMAL_GRAIN;
    assert!(t.admit(&r, va, SPARSE_GRAIN).unwrap());
    populate(&mut t, &r, va + SPARSE_GRAIN - PAGE, PAGE);
    r.guard
        .borrow_mut()
        .insert(va + SPARSE_GRAIN - PAGE, Some(PAGE));
    assert_eq!(t.retire(&r, va, SPARSE_GRAIN, false).unwrap(), SPARSE_GRAIN);
}
#[test]
fn held_intent_in_a_different_tile_retries_after_dma_is_removed() {
    let (mut t, r) = (tree(100), Rm::new());
    let va = LO + 2 * NORMAL_GRAIN;
    populate(&mut t, &r, va, 2 * PAGE);
    assert!(!t.admit(&r, va, SPARSE_GRAIN).unwrap());
    r.unmap(va, 2 * PAGE);
    t.mark(va, 2 * PAGE, false);
    assert_eq!(r.state(va), None);
    t.retry(&r).unwrap();
    assert_eq!(r.state(va), Some(0));
}
#[test]
fn a_cancelled_held_suffix_is_not_sparsified_on_retry() {
    let (mut t, r) = (tree(100), Rm::new());
    let va = LO + NORMAL_GRAIN;
    populate(&mut t, &r, va, PAGE);
    assert!(!t.admit(&r, va, 2 * SPARSE_GRAIN).unwrap());
    t.retire(&r, va, 2 * SPARSE_GRAIN, false).unwrap();
    r.unmap(va, PAGE);
    t.mark(va, PAGE, false);
    t.retry(&r).unwrap();
    assert_eq!(r.state(va + SPARSE_GRAIN), None);
}

#[test]
fn a_hardware_sized_64k_leaf_run_is_honoured_exactly() {
    let (mut t, r) = (tree(100), Rm::new());
    r.guard.borrow_mut().insert(LO, None);
    r.guard.borrow_mut().insert(LO + 131072, None);
    assert!(t.admit(&r, LO + 65536, 65536).unwrap());
    assert_eq!(r.state(LO + 65536), Some(0));
}

#[test]
fn recycling_a_sparse_only_space_restores_the_plain_invalid_control() {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.admit(&r, LO, SPARSE_GRAIN).unwrap());
    t.clear(&r).unwrap();
    assert_eq!(
        r.state(LO),
        None,
        "a new guest space owns no old sparse leaves"
    );
}
#[test]
fn recycling_several_sparse_blocks_restores_every_invalid_control() {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.admit(&r, LO + NORMAL_GRAIN, 2 * SPARSE_GRAIN).unwrap());
    t.clear(&r).unwrap();
    assert_eq!(r.state(LO + NORMAL_GRAIN), None);
    assert_eq!(r.state(LO + NORMAL_GRAIN + SPARSE_GRAIN), None);
}
#[test]
fn recycled_normal_blocks_never_retry_a_previous_guests_held_intent() {
    let (mut t, r) = (tree(100), Rm::new());
    populate(&mut t, &r, LO, PAGE);
    assert!(!t.admit(&r, LO, SPARSE_GRAIN).unwrap());
    r.unmap(LO, PAGE);
    t.mark(LO, PAGE, false);
    t.clear(&r).unwrap();
    t.retry(&r).unwrap();
    assert_eq!(r.state(LO), None);
}
fn live_recycle(va: u64) {
    let (mut t, r) = (tree(100), Rm::new());
    assert!(t.admit(&r, va, SPARSE_GRAIN).unwrap());
    populate(&mut t, &r, va, PAGE);
    r.guard.borrow_mut().insert(va, Some(PAGE));
    assert!(t.clear(&r).is_err());
    assert_eq!(t.stats.refused, 1);
    assert_eq!(r.state(va), Some(PAGE));
}
#[test]
fn an_emptying_space_with_live_vidmem_cannot_be_recycled() {
    live_recycle(LO);
}
#[test]
fn an_emptying_space_with_live_dma_cannot_be_recycled() {
    live_recycle(LO + NORMAL_GRAIN);
}

#[test]
fn recycling_discards_a_previous_guests_held_sparse_suffix() {
    let (mut t, r) = (tree(100), Rm::new());
    let va = LO + NORMAL_GRAIN;
    populate(&mut t, &r, va, PAGE);
    assert!(!t.admit(&r, va, 2 * SPARSE_GRAIN).unwrap());
    r.unmap(va, PAGE);
    t.mark(va, PAGE, false);
    t.clear(&r).unwrap();
    t.retry(&r).unwrap();
    assert_eq!(r.state(va + SPARSE_GRAIN), None);
}

// ── Plain maps are never refused or changed by the sparse twin (owner ruling 2026-10-26) ──

const RANGE: u32 = 0x9000;
const WIDE: u64 = 64;

fn wide(limit: usize) -> (Reservations, Rm) {
    let r = Rm::new();
    r.blocks
        .borrow_mut()
        .insert(1, (LO, LO + WIDE * NORMAL_GRAIN, false));
    let t = Reservations::new(
        &[GuestVaRange {
            handle: 1,
            lo: LO,
            hi: LO + WIDE * NORMAL_GRAIN,
        }],
        limit,
    );
    (t, r)
}

/// What the host would hold after mapping `off` at `va`: page -> backing offset. Every piece must
/// name a live reservation that contains it (or the range object for an unreserved hole).
fn host_pages(
    r: &Rm,
    pieces: &[(u32, u64, u64)],
    va: u64,
    len: u64,
    off: u64,
) -> BTreeMap<u64, u64> {
    let mut out = BTreeMap::new();
    let mut next = va;
    for &(h, pva, plen) in pieces {
        assert_eq!(pva, next, "pieces tile the row without a gap");
        if h != RANGE {
            let (lo, hi, sparse) = r.blocks.borrow()[&h];
            assert!(
                !sparse && lo <= pva && pva + plen <= hi,
                "piece inside its own block"
            );
        }
        for p in (pva..pva + plen).step_by(PAGE as usize) {
            out.insert(p, off + (p - va));
        }
        next = pva + plen;
    }
    assert_eq!(next, va + len);
    out
}

/// The product with the twin OFF: ONE piece through the static coarse reservation.
fn off_pages(va: u64, len: u64, off: u64) -> BTreeMap<u64, u64> {
    (va..va + len)
        .step_by(PAGE as usize)
        .map(|p| (p, off + (p - va)))
        .collect()
}

/// A catia-like churn: more distinct 2 MiB regions than the budget can hold, with row shapes of a
/// few pages, a whole tile, and several tiles.
fn churn() -> Vec<(u64, u64, u64)> {
    let mut rows = Vec::new();
    for i in 0..WIDE {
        let base = LO + i * NORMAL_GRAIN;
        let (a, n) = match i % 3 {
            0 => (base + 5 * PAGE, 3 * PAGE),
            1 => (base, NORMAL_GRAIN),
            _ => (base + PAGE, 7 * PAGE),
        };
        rows.push((a, n, 0x1000_0000 + i * 0x20_0000));
    }
    rows
}

#[test]
fn plain_maps_across_an_exhausted_budget_are_never_refused() {
    let (mut t, r) = wide(10);
    for (va, len, off) in churn() {
        let pieces = t
            .route_map(&r, RANGE, va, len, true)
            .expect("a plain map is never refused");
        r.backed(va, len);
        assert_eq!(
            host_pages(&r, &pieces, va, len, off),
            off_pages(va, len, off)
        );
    }
    assert!(t.blocks.len() <= 10, "the block bound still holds");
}
#[test]
fn plain_maps_are_byte_identical_to_the_twin_off_through_map_and_unmap() {
    let (mut t, r) = wide(10);
    let (mut on, mut off) = (BTreeMap::new(), BTreeMap::new());
    let rows = churn();
    for &(va, len, o) in &rows {
        let pieces = t.route_map(&r, RANGE, va, len, true).expect("plain map");
        r.backed(va, len);
        on.extend(host_pages(&r, &pieces, va, len, o));
        off.extend(off_pages(va, len, o));
        assert_eq!(on, off);
    }
    for &(va, len, _) in rows.iter().rev() {
        let pieces = t
            .pieces(RANGE, va, len)
            .expect("an unmap routes through a handle");
        for &(_, pva, plen) in &pieces {
            t.mark(pva, plen, false);
            for p in (pva..pva + plen).step_by(PAGE as usize) {
                on.remove(&p);
            }
        }
        r.unmap(va, len);
        for p in (va..va + len).step_by(PAGE as usize) {
            off.remove(&p);
        }
        assert_eq!(on, off);
    }
}
#[test]
fn a_plain_map_into_a_populated_coarse_block_is_accepted() {
    let (mut t, r) = wide(3);
    let a = LO + 4 * NORMAL_GRAIN;
    for va in [
        a,
        a + 8 * NORMAL_GRAIN,
        a + 9 * NORMAL_GRAIN,
        a + 9 * NORMAL_GRAIN + PAGE,
    ] {
        let pieces = t.route_map(&r, RANGE, va, PAGE, true).expect("plain map");
        r.backed(va, PAGE);
        assert_eq!(
            host_pages(&r, &pieces, va, PAGE, 7 * PAGE),
            off_pages(va, PAGE, 7 * PAGE)
        );
    }
}
#[test]
fn a_refused_split_leaves_the_plain_map_in_its_original_block() {
    let (mut t, r) = wide(100);
    r.fail.set(2);
    let va = LO + 3 * NORMAL_GRAIN;
    let pieces = t.route_map(&r, RANGE, va, PAGE, true).expect("plain map");
    r.backed(va, PAGE);
    assert_eq!(host_pages(&r, &pieces, va, PAGE, 0), off_pages(va, PAGE, 0));
    assert_eq!(t.stats.refused, 1, "the RM refusal is counted");
}
#[test]
fn a_block_lost_to_a_failed_repair_is_mapped_through_the_range_object() {
    let (mut t, r) = wide(100);
    r.fail_from.set(1);
    let va = LO + 3 * NORMAL_GRAIN;
    let pieces = t.route_map(&r, RANGE, va, PAGE, true).expect("plain map");
    assert!(pieces.iter().all(|&(h, _, _)| h == RANGE || h != 0));
    assert!(t.poisoned);
    assert_eq!(pieces.iter().map(|p| p.2).sum::<u64>(), PAGE);
    r.fail_from.set(0);
}
#[test]
fn an_unmap_of_a_block_lost_to_a_failed_repair_routes_through_the_range_object() {
    let (mut t, r) = wide(100);
    r.fail_from.set(1);
    let va = LO + 3 * NORMAL_GRAIN;
    let _ = t.route_map(&r, RANGE, va, PAGE, true);
    let pieces = t
        .pieces(RANGE, va, PAGE)
        .expect("an unmap routes through a handle");
    assert!(pieces.iter().all(|&(h, _, _)| h != 0));
}
#[test]
fn declined_plain_maps_are_counted_as_coarse_maps_not_as_overflow() {
    let (mut t, r) = wide(4);
    let mut declined = 0;
    for (va, len, _) in churn() {
        let before = t.blocks.len();
        t.route_map(&r, RANGE, va, len, true).expect("plain map");
        r.backed(va, len);
        declined += u64::from(t.blocks.len() == before);
    }
    assert!(declined > 0);
    assert_eq!(
        t.stats.overflow, 0,
        "overflow is for sparse admissions only"
    );
    assert!(t.stats.coarse_maps > 0 && t.stats.coarse_maps <= WIDE);
}
#[test]
fn a_map_that_refines_normally_is_not_a_coarse_map() {
    let (mut t, r) = wide(100);
    t.route_map(&r, RANGE, LO + NORMAL_GRAIN, PAGE, true)
        .expect("plain map");
    assert_eq!(t.stats.coarse_maps, 0);
    assert_eq!(t.stats.overflow, 0);
}
#[test]
fn a_sparse_run_over_the_block_budget_is_unhonoured_not_refused() {
    let (mut t, r) = (tree(1), Rm::new());
    assert_eq!(t.admit(&r, LO, SPARSE_GRAIN), Ok(false));
    assert_eq!(r.calls.get(), 0);
    assert_eq!(r.state(LO), None);
}

#[test]
fn ens3_admission_cost_probe_counts_clone_and_restore_without_rm_calls() {
    let (mut t, r) = (tree(BLOCK_LIMIT), Rm::new());
    t.blocks = (0..BLOCK_LIMIT)
        .map(|i| {
            let lo = LO + i as u64 * NORMAL_GRAIN;
            (
                lo,
                Block {
                    lo,
                    hi: lo + NORMAL_GRAIN,
                    handle: i as u32 + 1,
                    sparse: false,
                    live: [0; 8],
                    wanted: [0; 8],
                },
            )
        })
        .collect();
    let before = crate::diag::snapshot();
    assert_eq!(
        t.admit(&r, LO, (BLOCK_LIMIT as u64 + 1) * SPARSE_GRAIN),
        Ok(false)
    );
    let delta = crate::diag::snapshot().since(before);
    assert_eq!(r.calls.get(), 0);
    if crate::diag::enabled() {
        assert_eq!(delta.costs[2].calls, 1);
        assert_eq!(delta.costs[3].work, BLOCK_LIMIT as u64);
        assert_eq!(delta.costs[4].work, BLOCK_LIMIT as u64 * 512);
        eprintln!(
            "ENS3_ADMISSION_PROBE blocks={} block_bytes={} clone_ns={} \
            restore_pages={} restore_ns={} admission_ns={} rm_calls=0",
            BLOCK_LIMIT,
            std::mem::size_of::<Block>(),
            delta.costs[3].ns,
            delta.costs[4].work,
            delta.costs[4].ns,
            delta.costs[2].ns
        );
    } else {
        assert_eq!(delta, crate::diag::Snapshot::default());
    }
}
#[test]
fn each_declined_sparse_admission_is_one_overflow_and_its_bytes_are_unhonoured() {
    let (mut t, r) = (tree(1), Rm::new());
    for n in 1..=3u64 {
        assert_eq!(t.admit(&r, LO, SPARSE_GRAIN), Ok(false));
        assert_eq!(t.stats.overflow, n);
        assert_eq!(t.stats.unhonoured_bytes, n * SPARSE_GRAIN);
    }
    assert_eq!(t.stats.refused, 0);
}
#[test]
fn a_sparse_run_declined_midway_withdraws_every_new_tile() {
    let (mut t, r) = (tree(3), Rm::new());
    assert_eq!(t.admit(&r, LO, 2 * SPARSE_GRAIN), Ok(false));
    assert!(
        !r.blocks.borrow().values().any(|b| b.2),
        "no sparse block remains"
    );
    assert_eq!(r.state(LO), None);
    assert_eq!(r.state(LO + SPARSE_GRAIN), None);
    assert_eq!(t.stats.overflow, 1);
}

#[test]
fn a_failed_repair_stops_sparse_bookkeeping_but_never_plain_routing() {
    assert!(poison_refuses(true, false));
    assert!(!poison_refuses(true, true));
}
#[test]
fn a_healthy_layout_refuses_nothing() {
    assert!(!poison_refuses(false, false));
    assert!(!poison_refuses(false, true));
}
#[test]
fn a_map_with_refinement_off_makes_no_reservation_call_and_is_placed_coarse() {
    let (mut t, r) = wide(100);
    let va = LO + 3 * NORMAL_GRAIN;
    let pieces = t.route_map(&r, RANGE, va, PAGE, false).expect("plain map");
    r.backed(va, PAGE);
    assert_eq!(r.calls.get(), 0);
    assert_eq!(host_pages(&r, &pieces, va, PAGE, 0), off_pages(va, PAGE, 0));
    assert_eq!(t.stats.coarse_maps, 1);
}
#[test]
fn a_plain_map_below_the_twin_floor_is_refused_as_with_the_twin_off() {
    let (mut t, r) = wide(100);
    assert!(t.route_map(&r, RANGE, 0, PAGE, true).is_err());
    assert_eq!(r.calls.get(), 0);
    assert_eq!(t.stats.coarse_maps, 0);
}
#[test]
fn a_wrapping_plain_map_is_refused_as_with_the_twin_off() {
    let (mut t, r) = wide(100);
    assert!(
        t.route_map(&r, RANGE, u64::MAX - PAGE + 1, 2 * PAGE, true)
            .is_err()
    );
    assert_eq!(r.calls.get(), 0);
    assert_eq!(t.stats.coarse_maps, 0);
}
#[test]
fn a_block_lost_to_a_failed_repair_is_never_freed_or_split_again() {
    let (mut t, r) = wide(100);
    r.fail_from.set(1);
    let va = LO + 3 * NORMAL_GRAIN;
    let _ = t.route_map(&r, RANGE, va, PAGE, true);
    r.fail_from.set(0);
    let calls = r.calls.get();
    assert!(t.normal(&r, va, PAGE).is_err());
    assert_eq!(
        r.calls.get(),
        calls,
        "no host call on a block that has no reservation"
    );
}
#[test]
fn a_sparse_run_over_a_block_lost_to_a_failed_repair_makes_no_host_call() {
    let (mut t, r) = wide(100);
    r.fail_from.set(1);
    let _ = t.route_map(&r, RANGE, LO + 3 * NORMAL_GRAIN, PAGE, true);
    r.fail_from.set(0);
    let calls = r.calls.get();
    assert!(t.admit(&r, LO + 3 * NORMAL_GRAIN, SPARSE_GRAIN).is_err());
    assert_eq!(r.calls.get(), calls);
}
#[test]
fn a_routed_map_is_live_so_a_sparse_run_cannot_free_its_tile() {
    let (mut t, r) = wide(100);
    let va = LO + 5 * NORMAL_GRAIN;
    t.route_map(&r, RANGE, va, PAGE, true).expect("plain map");
    r.backed(va, PAGE);
    r.guard.borrow_mut().insert(va, Some(PAGE));
    assert_eq!(t.admit(&r, va, SPARSE_GRAIN), Ok(false));
    assert_eq!(r.state(va), Some(PAGE));
}
#[test]
fn a_routed_map_is_live_so_the_space_cannot_be_recycled() {
    let (mut t, r) = wide(100);
    let va = LO + 5 * NORMAL_GRAIN;
    t.route_map(&r, RANGE, va, PAGE, true).expect("plain map");
    r.backed(va, PAGE);
    assert!(t.clear(&r).is_err());
}
#[test]
fn a_refused_plain_map_does_not_count_as_a_coarse_map() {
    let (mut t, r) = wide(100);
    assert!(t.route_map(&r, RANGE, 0, PAGE, true).is_err());
    assert_eq!(t.stats.coarse_maps, 0);
}
#[test]
fn a_map_with_refinement_off_leaves_the_geometry_untouched() {
    let (mut t, r) = wide(100);
    let before = t.blocks.len();
    t.route_map(&r, RANGE, LO + 7 * NORMAL_GRAIN, PAGE, false)
        .expect("plain map");
    assert_eq!(t.blocks.len(), before);
}
#[test]
fn routing_survives_a_poisoned_layout_for_every_plain_call() {
    assert!(!poison_refuses(true, true));
    assert!(!poison_refuses(false, true));
}
#[test]
fn only_sparse_bookkeeping_stops_on_a_poisoned_layout() {
    let refused: Vec<_> = [(false, false), (false, true), (true, false), (true, true)]
        .into_iter()
        .filter(|&(p, r)| poison_refuses(p, r))
        .collect();
    assert_eq!(refused, [(true, false)]);
}
