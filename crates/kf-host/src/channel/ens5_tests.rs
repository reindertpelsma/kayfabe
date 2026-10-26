// Native-verb fake drives the same reservation/map/rollback helper as HostRm::map_kind.
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
    failure: Option<RmError>,
    uncertain_effect: bool,
    rollback_refused: bool,
    next: Cell<u32>,
}
impl DmaVerbs for HostFake {
    fn map_one(&self, _: u32, p: MapPiece) -> Result<u64, RmError> {
        let n = self.calls.get() + 1;
        self.calls.set(n);
        if n == self.fail.get() {
            if self.uncertain_effect {
                self.maps.borrow_mut().insert(p.at, p.len);
            }
            return Err(self.failure.unwrap_or(RmError::NoMemory));
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

// Same exclusive-placement helper as HostRm; native alloc and act actors take its read side.
fn concurrent_placement(refused: bool) {
    use std::sync::{
        Arc, Barrier, RwLock,
        atomic::{AtomicU32, Ordering},
    };
    let gate = Arc::new(RwLock::new(()));
    let phase = Arc::new(AtomicU32::new(0));
    let ready = Arc::new(Barrier::new(2));
    let (done, wait_done) = std::sync::mpsc::channel();
    let (resume, wait_resume) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let g = gate.clone();
        let p = phase.clone();
        let r = ready.clone();
        let va = scope.spawn(move || {
            super::sparse::with_placement(&g, || {
                p.store(1, Ordering::Release);
                r.wait();
                wait_resume
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                p.store(2, Ordering::Release);
                if refused {
                    Err(RmError::NoMemory)
                } else {
                    Ok(())
                }
            })
        });
        ready.wait();
        assert!(
            gate.try_read().is_err(),
            "act allocation cannot enter a geometry replacement"
        );
        assert!(
            gate.try_write().is_err(),
            "a peer VA refresh cannot enter replacement"
        );
        let g = gate.clone();
        let p = phase.clone();
        let act = scope.spawn(move || {
            let _guard = g.read().unwrap();
            assert_eq!(p.load(Ordering::Acquire), 2);
        });
        // A published-completion actor needs no CPU placement lock. This models the
        // source-level exclusion boundary, not RM/GSP completion timing.
        scope.spawn(move || {
            done.send(()).unwrap();
        });
        wait_done
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        resume.send(()).unwrap();
        assert_eq!(va.join().unwrap().is_err(), refused);
        act.join().unwrap();
    });
    assert!(gate.try_write().is_ok());
}
#[test]
fn ens5_va_act_and_completion_actors_obey_the_production_placement_boundary() {
    concurrent_placement(false);
}
#[test]
fn ens5_native_refusal_releases_the_placement_boundary_after_repair() {
    concurrent_placement(true);
}
#[test]
fn ens5_failed_first_piece_keeps_a_prior_live_second_piece_pinned() {
    let mut t = tree();
    t.mark(LO + P, P, true);
    let f = HostFake {
        fail: Cell::new(1),
        ..Default::default()
    };
    f.maps.borrow_mut().insert(LO + P, P);
    attempt(&mut t, &f, 2 * P);
    assert_eq!(f.maps.borrow().len(), 1);
    assert!(t.clear(&f).is_err());
}
#[test]
fn ens5_rollback_refusal_of_a_later_piece_also_keeps_native_pages_pinned() {
    let mut t = crate::sparse::Reservations::new(
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
            GuestVaRange {
                handle: 3,
                lo: LO + 2 * P,
                hi: LO + 3 * P,
            },
        ],
        100,
    );
    let f = HostFake {
        fail: Cell::new(3),
        rollback_refused: true,
        ..Default::default()
    };
    attempt(&mut t, &f, 3 * P);
    assert_eq!(f.maps.borrow().len(), 2);
    assert!(t.clear(&f).is_err());
}

#[test]
fn ens5_uncertain_cancel_keeps_the_possibly_installed_native_map_pinned() {
    let mut t = tree();
    let f = HostFake {
        fail: Cell::new(1),
        failure: Some(RmError::Interrupted),
        uncertain_effect: true,
        ..Default::default()
    };
    attempt(&mut t, &f, P);
    assert_eq!(f.maps.borrow().len(), 1);
    assert!(t.clear(&f).is_err());
}
#[test]
fn ens5_uncertain_transport_after_a_partial_map_keeps_the_failed_piece_pinned() {
    let mut t = tree();
    let f = HostFake {
        fail: Cell::new(2),
        failure: Some(RmError::Other(0x8000_0005)),
        uncertain_effect: true,
        ..Default::default()
    };
    attempt(&mut t, &f, 2 * P);
    assert_eq!(f.maps.borrow().len(), 1);
    assert!(t.clear(&f).is_err());
}
