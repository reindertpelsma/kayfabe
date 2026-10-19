//! Diagnostic contract tests; no VM or GPU. API intentionally absent at EXPECTED RED.
use kf_chip::Family;
use kf_trap::cpuintr::{CpuIntr, Raise, Reg};
use kf_trap::cpuintr_audit::{Audit, Event};

fn tree() -> CpuIntr {
    CpuIntr::new(Family::Ada, 0xbb0000).unwrap()
}

#[test]
fn audit_is_off_until_explicitly_armed() {
    let a = Audit::<2>::new();
    assert!(a.begin(&tree(), 8, Event::Latch(0)).is_none());
    assert_eq!(a.used(), 0);
}
#[test]
fn unarmed_audit_cannot_record_a_guest_write() {
    let a = Audit::<2>::new();
    assert!(
        a.begin(
            &tree(),
            8,
            Event::Write {
                off: 0xb81608,
                value: 1
            }
        )
        .is_none()
    );
    assert!(a.read(0).is_none());
}
#[test]
fn latch_captures_pending_mask_and_raise_without_conflating_subscription() {
    let t = tree();
    let a = Audit::<2>::new();
    a.arm();
    let ticket = a.begin(&t, 8, Event::Latch(0)).unwrap();
    let raise = t.latch(0);
    a.finish(ticket, &t, raise);
    let r = a.read(0).unwrap();
    assert_eq!(r.before[0], 0);
    assert_eq!(r.after[0], 1);
    assert_eq!(r.before[16], 0);
    assert_eq!(r.after[16], 0);
    assert_eq!(r.after[32], 0);
    assert_eq!(r.raise, Raise::None);
}
#[test]
fn enable_of_pending_bit_is_observed_with_the_message_result() {
    let t = tree();
    let a = Audit::<2>::new();
    t.latch(0);
    t.write(Reg::LeafEnSet(0), 1);
    a.arm();
    let ticket = a
        .begin(
            &t,
            8,
            Event::Write {
                off: 0xb81608,
                value: 1,
            },
        )
        .unwrap();
    let raise = t.write(Reg::TopEnSet, 1);
    a.finish(ticket, &t, raise);
    let r = a.read(0).unwrap();
    assert_eq!(r.before[0], 1);
    assert_eq!(r.before[16], 1);
    assert_eq!(r.before[32], 0);
    assert_eq!(r.after[32], 1);
    assert_eq!(r.raise, Raise::Message);
}
#[test]
fn audit_cap_never_overwrites_a_committed_record() {
    let t = tree();
    let a = Audit::<2>::new();
    a.arm();
    for vector in 0..2 {
        let ticket = a.begin(&t, 8, Event::Latch(vector)).unwrap();
        a.finish(ticket, &t, Raise::None);
    }
    for _ in 0..20 {
        assert!(a.begin(&t, 8, Event::Latch(3)).is_none());
    }
    assert_eq!(a.used(), 2);
    assert_eq!(a.full(), 20);
    assert_eq!(a.read(0).unwrap().event, Event::Latch(0));
    assert_eq!(a.read(1).unwrap().event, Event::Latch(1));
}
#[test]
fn audit_does_not_publish_an_incomplete_record() {
    let t = tree();
    let a = Audit::<2>::new();
    a.arm();
    let ticket = a.begin(&t, 8, Event::Latch(1)).unwrap();
    assert!(a.read(0).is_none());
    a.finish(ticket, &t, Raise::None);
    assert!(a.read(0).is_some());
}

#[test]
fn separate_reservations_cannot_publish_each_others_payload() {
    let t = tree();
    let a = Audit::<2>::new();
    a.arm();
    let first = a.begin(&t, 8, Event::Latch(7)).unwrap();
    let second = a.begin(&t, 8, Event::Latch(8)).unwrap();
    a.finish(second, &t, Raise::Message);
    assert!(a.read(0).is_none());
    assert_eq!(a.read(1).unwrap().event, Event::Latch(8));
    assert_eq!(a.read(1).unwrap().raise, Raise::Message);
    a.finish(first, &t, Raise::None);
    assert_eq!(a.read(0).unwrap().event, Event::Latch(7));
    assert_eq!(a.read(0).unwrap().end, 1);
    assert_eq!(a.read(1).unwrap().end, 0);
}

#[test]
fn zero_capacity_audit_always_drops_without_indexing_storage() {
    let t = tree();
    let a = Audit::<0>::new();
    a.arm();
    assert!(a.begin(&t, 8, Event::Latch(0)).is_none());
    assert_eq!(a.used(), 0);
    assert_eq!(a.full(), 1);
    assert!(a.read(0).is_none());
}

#[test]
fn stopping_capture_preserves_existing_tickets_and_refuses_new_ones() {
    let t = tree();
    let a = Audit::<2>::new();
    a.arm();
    let ticket = a.begin(&t, 8, Event::Latch(0)).unwrap();
    a.stop();
    assert!(a.begin(&t, 8, Event::Latch(1)).is_none());
    a.finish(ticket, &t, Raise::None);
    assert_eq!(a.read(0).unwrap().event, Event::Latch(0));
}

#[test]
fn stopping_empty_capture_remains_empty() {
    let t = tree();
    let a = Audit::<2>::new();
    a.arm();
    a.stop();
    assert!(a.begin(&t, 8, Event::Latch(0)).is_none());
    assert_eq!(a.used(), 0);
}

#[test]
fn contending_producers_account_for_every_bounded_attempt() {
    let a = std::sync::Arc::new(Audit::<128>::new());
    a.arm();
    let threads: Vec<_> = (0..4)
        .map(|id| {
            let a = std::sync::Arc::clone(&a);
            std::thread::spawn(move || {
                let t = tree();
                for n in 0..100 {
                    if let Some(ticket) = a.begin(&t, 8, Event::Latch(id * 100 + n)) {
                        a.finish(ticket, &t, Raise::None);
                    }
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert!(a.used() <= 128);
    assert_eq!(a.used() as u64 + a.full() + a.contended(), 400);
    for i in 0..a.used() {
        assert!(a.read(i).is_some());
    }
}
