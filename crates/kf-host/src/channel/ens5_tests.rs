//! Native-verb fake drives the same reservation/map/rollback helper as HostRm::map_kind.
use super::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
const LO: u64 = 1 << 40;
const P: u64 = 4096;
#[derive(Default)]
struct HostFake {
    maps: RefCell<BTreeMap<u64, u64>>,
    calls: Cell<u32>,
    fail: Cell<u32>,
    rollback_refused: bool,
    next: Cell<u32>,
}
impl DmaVerbs for HostFake {
    fn map_one(&self, _: u32, p: MapPiece) -> Result<u64, RmError> {
        let n = self.calls.get() + 1;
        self.calls.set(n);
        if n == self.fail.get() {
            return Err(RmError::NoMemory);
        }
        self.maps.borrow_mut().insert(p.at, p.len);
        Ok(p.at)
    }
    fn unmap_one(&self, _: u32, va: u64, _: u64, _: u32) -> Result<(), RmError> {
        if self.rollback_refused {
            return Err(RmError::NoMemory);
        }
        self.maps.borrow_mut().remove(&va);
        Ok(())
    }
}
impl crate::sparse::Ops for HostFake {
    fn reserve(&self, _: u64, _: u64, _: bool) -> Result<u32, RmError> {
        let next = self.next.get() + 10;
        self.next.set(next);
        Ok(next)
    }
    fn release(&self, _: u32) -> Result<(), RmError> {
        Ok(())
    }
}
fn tree() -> crate::sparse::Reservations {
    crate::sparse::Reservations::new(
        &[
            GuestVaRange {
                handle: 1,
                lo: LO,
                hi: LO + P,
            },
            GuestVaRange {
                handle: 2,
                lo: LO + P,
                hi: LO + 2 * P,
            },
        ],
        100,
    )
}
fn attempt(t: &mut crate::sparse::Reservations, f: &HostFake, len: u64) {
    assert!(
        map_tree(
            f,
            t,
            f,
            9,
            MapPiece {
                memory: 3,
                offset: 0,
                len,
                at: LO,
                extra: 0,
                shared: false,
                kind: 0
            },
            0,
            false
        )
        .is_err()
    );
}
#[test]
fn ens5_failed_first_native_map_does_not_pin_an_empty_reservation() {
    let mut t = tree();
    let f = HostFake {
        fail: Cell::new(1),
        ..Default::default()
    };
    attempt(&mut t, &f, P);
    assert!(f.maps.borrow().is_empty());
    assert!(
        t.clear(&f).is_ok(),
        "failed map must undo provisional occupancy"
    );
}
#[test]
fn ens5_failed_native_suffix_with_successful_rollback_does_not_pin_a_reservation() {
    let mut t = tree();
    let f = HostFake {
        fail: Cell::new(2),
        ..Default::default()
    };
    attempt(&mut t, &f, 2 * P);
    assert!(f.maps.borrow().is_empty());
    assert!(
        t.clear(&f).is_ok(),
        "all rollback receipts permit occupancy undo"
    );
}
#[test]
fn ens5_failed_native_rollback_keeps_every_possibly_live_piece_pinned() {
    let mut t = tree();
    let f = HostFake {
        fail: Cell::new(2),
        rollback_refused: true,
        ..Default::default()
    };
    attempt(&mut t, &f, 2 * P);
    assert_eq!(f.maps.borrow().len(), 1);
    assert!(
        t.clear(&f).is_err(),
        "uncertain native rollback never permits free"
    );
}
#[test]
fn ens5_failed_replacement_restores_prior_live_occupancy() {
    let mut t = tree();
    t.mark(LO, P, true);
    let f = HostFake {
        fail: Cell::new(1),
        ..Default::default()
    };
    f.maps.borrow_mut().insert(LO, P);
    attempt(&mut t, &f, P);
    assert_eq!(f.maps.borrow().len(), 1);
    assert!(
        t.clear(&f).is_err(),
        "an older live mapping must remain pinned"
    );
}
