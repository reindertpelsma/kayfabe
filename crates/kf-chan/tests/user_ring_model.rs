//! P2 model only: no hardware adapter or native migration cursor assumptions.
use kf_chan::user_ring::{Ledger, Refusal, Retirement};

fn receipt(generation: u64, guest_get: u32) -> Retirement {
    Retirement {
        generation,
        guest_get,
    }
}

#[test]
fn capacity_is_bounded_and_keeps_one_slot_empty() {
    for entries in [0, 1, 3, 5, u32::MAX] {
        assert!(Ledger::new(entries, 16).is_err());
    }
    assert_eq!(Ledger::new(32, 16).unwrap_err(), Refusal::Capacity);
    let mut q = Ledger::new(4, 4).unwrap();
    for n in 0..3 {
        let p = q.publish(receipt(1, n + 1)).unwrap();
        assert_eq!((p.slot, p.put, p.sequence), (n, n + 1, u64::from(n + 1)));
    }
    assert_eq!(q.available(), 0);
    assert_eq!(q.publish(receipt(1, 4)), Err(Refusal::Full));
    assert_eq!(q.pending(), 3);
}

#[test]
fn gp_get_alone_never_retires_or_grants_staging_reuse() {
    let mut q = Ledger::new(4, 4).unwrap();
    q.publish(receipt(7, 24)).unwrap();
    assert_eq!(q.observe_get(1), Ok(()));
    assert_eq!(q.pending(), 1);
    assert_eq!(q.available(), 2);
    assert_eq!(q.completed(), 0);
    assert_eq!(q.complete(1).unwrap(), [receipt(7, 24)]);
    assert_eq!(q.pending(), 0);
}

#[test]
fn hostile_get_and_future_completion_leave_state_unchanged() {
    let mut q = Ledger::new(8, 8).unwrap();
    q.publish(receipt(1, 1)).unwrap();
    for get in [2, 7, 8, u32::MAX] {
        assert_eq!(q.observe_get(get), Err(Refusal::Fetch));
        assert_eq!(q.pending(), 1);
        assert_eq!(q.completed(), 0);
    }
    assert_eq!(q.complete(2), Err(Refusal::Completion));
    assert_eq!(q.pending(), 1);
    assert_eq!(q.complete(1).unwrap(), [receipt(1, 1)]);
    assert_eq!(q.complete(1).unwrap(), []);
    assert_eq!(q.complete(0).unwrap(), []);
}

#[test]
fn wrapping_host_slots_preserve_distinct_guest_generations_and_indices() {
    let mut q = Ledger::new(4, 4).unwrap();
    let progress = [
        receipt(8, 32767),
        receipt(8, 0),
        receipt(9, 0),
        receipt(9, 65535),
        receipt(9, 0),
    ];
    for (i, &r) in progress.iter().enumerate() {
        let p = q.publish(r).unwrap();
        assert_eq!(p.slot, i as u32 % 4);
        assert_eq!(p.put, (i as u32 + 1) % 4);
        q.observe_get(p.put).unwrap();
        assert_eq!(q.complete(p.sequence).unwrap(), [r]);
    }
    assert_eq!(q.completed(), 5);
    // The model preserves supplied epochs/cursors; it does NOT decide native
    // migration's reset/preserve rule or inspect/follow newly freed mappings.
}

#[test]
fn completion_can_retire_a_batch_but_a_fetch_cannot() {
    let mut q = Ledger::new(8, 8).unwrap();
    for n in 0..7 {
        q.publish(receipt(2, n + 1)).unwrap();
    }
    q.observe_get(7).unwrap();
    assert_eq!(q.available(), 0);
    assert_eq!(
        q.complete(4).unwrap(),
        (1..=4).map(|n| receipt(2, n)).collect::<Vec<_>>()
    );
    assert_eq!(q.pending(), 3);
    assert_eq!(q.available(), 4);
    assert_eq!(
        q.complete(7).unwrap(),
        (5..=7).map(|n| receipt(2, n)).collect::<Vec<_>>()
    );
}

#[test]
fn exhaustive_small_ring_interleavings_do_not_overwrite_or_retire_early() {
    // Every publish/complete schedule to depth12 for both minimal and wrapped
    // rings; completion uses the simulated engine's last executed sequence.
    fn visit(q: Ledger, depth: u32, submitted: Vec<Retirement>, retired: Vec<Retirement>) {
        assert_eq!(q.pending(), submitted.len() - retired.len());
        assert_eq!(
            q.available() as usize + q.pending(),
            q.entries() as usize - 1
        );
        assert_eq!(&submitted[..retired.len()], retired.as_slice());
        if depth == 0 {
            return;
        }
        let mut pushed = q.clone();
        let r = receipt(u64::from(depth % 3), depth);
        if pushed.publish(r).is_ok() {
            let mut s = submitted.clone();
            s.push(r);
            visit(pushed, depth - 1, s, retired.clone());
        } else {
            assert_eq!(q.available(), 0);
        }
        if q.pending() != 0 {
            let mut completed = q.clone();
            let one = completed.complete(q.completed() + 1).unwrap();
            let mut r = retired;
            r.extend(one);
            visit(completed, depth - 1, submitted, r);
        }
    }
    for entries in [2, 4, 8] {
        visit(Ledger::new(entries, 8).unwrap(), 12, vec![], vec![]);
    }
}
