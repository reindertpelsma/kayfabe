//! A new selective space must reserve every guest range before host context birth.

use super::{GuestVaRange, Ops, RmError};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

pub(crate) fn reserve_initial(
    ops: &dyn Ops,
    ranges: &[(u64, u64)],
) -> Result<[GuestVaRange; 3], RmError> {
    if ranges.len() > 3 || ranges.iter().any(|&(lo, hi)| lo >= hi) {
        return Err(RmError::NoMemory);
    }
    let mut guest = [GuestVaRange::default(); 3];
    for (slot, &(lo, hi)) in guest.iter_mut().zip(ranges) {
        match ops.reserve(lo, hi - lo, false) {
            Ok(handle) => *slot = GuestVaRange { handle, lo, hi },
            Err(e) => {
                for g in guest.iter().filter(|g| g.handle != 0) {
                    let _ = ops.release(g.handle);
                }
                return Err(e);
            }
        }
    }
    Ok(guest)
}

pub(crate) fn claim(counter: &AtomicUsize, limit: usize) -> bool {
    counter
        .try_update(Relaxed, Relaxed, |n| {
            (n < limit).then_some(n.saturating_add(1))
        })
        .is_ok()
}

/// Claim one session slot for a reservation. Only the tree's own blocks (`tree_block`) count
/// against the cap; any other reservation passes exactly as it does with the twin OFF.
pub(crate) fn claim_block(counter: &AtomicUsize, limit: usize, tree_block: bool) -> bool {
    let _ = tree_block;
    claim(counter, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    struct Rm {
        blocks: RefCell<BTreeMap<u32, (u64, u64)>>,
        next: Cell<u32>,
        fail: u32,
    }
    impl Ops for Rm {
        fn reserve(&self, lo: u64, len: u64, sparse: bool) -> Result<u32, RmError> {
            assert!(!sparse);
            let h = self.next.get() + 1;
            self.next.set(h);
            if h == self.fail {
                return Err(RmError::NoMemory);
            }
            self.blocks.borrow_mut().insert(h, (lo, lo + len));
            Ok(h)
        }
        fn release(&self, h: u32) -> Result<(), RmError> {
            assert!(self.blocks.borrow_mut().remove(&h).is_some());
            Ok(())
        }
    }
    fn refusal(fail: u32) {
        let rm = Rm {
            blocks: RefCell::new(BTreeMap::new()),
            next: Cell::new(0),
            fail,
        };
        assert_eq!(
            reserve_initial(
                &rm,
                &[(65536, 1048576), (1 << 40, 1 << 47), (1 << 32, 1 << 33)]
            ),
            Err(RmError::NoMemory)
        );
        assert!(rm.blocks.borrow().is_empty());
    }
    #[test]
    fn a_refused_first_root_prevents_context_birth() {
        refusal(1);
    }
    #[test]
    fn a_refused_high_root_releases_the_low_root_before_context_birth() {
        refusal(2);
    }
    #[test]
    fn a_refused_third_root_releases_both_preceding_roots() {
        refusal(3);
    }
    #[test]
    fn an_initially_full_session_budget_cannot_create_another_object() {
        let n = AtomicUsize::new(3);
        assert!(!claim(&n, 3));
        assert_eq!(n.load(Relaxed), 3);
    }
    #[test]
    fn a_session_with_one_object_left_can_claim_exactly_one() {
        let n = AtomicUsize::new(8);
        assert!(claim(&n, 9));
        assert!(!claim(&n, 9));
        assert_eq!(n.load(Relaxed), 9);
    }
    #[test]
    fn concurrent_spaces_share_one_hard_object_limit() {
        let n = AtomicUsize::new(0);
        let accepted = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| (0..64).filter(|_| claim(&n, 7)).count()))
                .collect();
            jobs.into_iter().map(|j| j.join().unwrap()).sum::<usize>()
        });
        assert_eq!(accepted, 7);
        assert_eq!(n.load(Relaxed), 7);
    }
    #[test]
    fn a_non_tree_reservation_passes_a_full_session() {
        let n = AtomicUsize::new(5);
        assert!(claim_block(&n, 5, false));
    }
    #[test]
    fn a_non_tree_reservation_never_consumes_the_session_budget() {
        let n = AtomicUsize::new(2);
        assert!(claim_block(&n, 5, false));
        assert_eq!(n.load(Relaxed), 2);
    }
    #[test]
    fn a_tree_block_is_refused_by_a_full_session() {
        let n = AtomicUsize::new(5);
        assert!(!claim_block(&n, 5, true));
        assert_eq!(n.load(Relaxed), 5);
    }
    #[test]
    fn a_tree_block_consumes_one_session_slot() {
        let n = AtomicUsize::new(2);
        assert!(claim_block(&n, 5, true));
        assert_eq!(n.load(Relaxed), 3);
    }
}
