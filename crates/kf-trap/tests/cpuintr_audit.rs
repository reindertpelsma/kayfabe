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
