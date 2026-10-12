//! Command-byte consumption only; never an engine completion model.
use kf_chan::user_consumption::{ConsumedSource, Consumption, Refusal};
fn source(n: u32) -> ConsumedSource {
    ConsumedSource {
        generation: 7,
        guest_get: n,
    }
}
#[test]
fn neither_command_nor_barrier_commit_releases_pb_bytes() {
    let mut q = Consumption::new(8, 91).unwrap();
    let p = q.publish(source(1)).unwrap();
    assert_eq!(
        (
            p.command_slot,
            p.barrier_slot,
            p.witness_slot,
            p.put,
            p.witness_sequence
        ),
        (0, 1, 2, 3, 3)
    );
    assert_eq!(q.available_gp(), 4);
    assert!(q.observe_get(91, 1).unwrap().is_empty());
    assert_eq!(q.available_gp(), 5);
    assert!(q.observe_get(91, 2).unwrap().is_empty());
    assert_eq!(q.pending_sources(), 1);
    assert_eq!(q.observe_get(91, 3).unwrap(), [source(1)]);
    assert_eq!(q.pending_sources(), 0);
    assert!(q.observe_get(91, 3).unwrap().is_empty());
}
#[test]
fn wrapped_absolute_witnesses_retire_only_their_own_preceding_commands() {
    let mut q = Consumption::new(8, 91).unwrap();
    q.publish(source(1)).unwrap();
    q.publish(source(2)).unwrap();
    assert_eq!(q.publish(source(3)), Err(Refusal::Full));
    assert_eq!(q.observe_get(91, 5).unwrap(), [source(1)]);
    let p = q.publish(source(3)).unwrap();
    assert_eq!(
        (
            p.command_slot,
            p.barrier_slot,
            p.witness_slot,
            p.put,
            p.witness_sequence
        ),
        (6, 7, 0, 1, 9)
    );
    assert_eq!(q.observe_get(91, 0).unwrap(), [source(2)]);
    assert_eq!(q.pending_sources(), 1);
    assert_eq!(q.observe_get(91, 1).unwrap(), [source(3)]);
}
#[test]
fn unissued_reset_or_wrong_lifetime_grants_no_credit() {
    let mut q = Consumption::new(8, 91).unwrap();
    q.publish(source(1)).unwrap();
    for (lifetime, index, refusal) in [
        (92, 3, Refusal::Lifetime),
        (91, 8, Refusal::Fetch),
        (91, 4, Refusal::Fetch),
    ] {
        assert_eq!(q.observe_get(lifetime, index), Err(refusal));
        assert_eq!(q.pending_sources(), 1);
        assert_eq!(q.available_gp(), 4);
    }
    assert!(q.observe_get(91, 1).unwrap().is_empty());
    assert_eq!(q.observe_get(91, 0), Err(Refusal::Fetch));
    assert_eq!(q.pending_sources(), 1);
    assert_eq!(q.observe_get(91, 3).unwrap(), [source(1)]);
}
#[test]
fn repeated_wraps_keep_the_last_witness_pending_until_actual_progress() {
    for entries in [4, 8, 16, 32, 64, 128, 256, 512] {
        let mut q = Consumption::new(entries, 91).unwrap();
        for n in 1..1000 {
            let p = q.publish(source(n)).unwrap();
            assert!(
                q.observe_get(91, (p.barrier_slot + 1) % entries)
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(q.pending_sources(), 1);
            assert_eq!(q.observe_get(91, p.put).unwrap(), [source(n)]);
        }
    }
}
#[test]
fn early_descriptor_credit_cannot_overflow_source_metadata() {
    let mut q = Consumption::new(8, 91).unwrap();
    q.publish(source(1)).unwrap();
    q.observe_get(91, 2).unwrap();
    q.publish(source(2)).unwrap();
    q.publish(source(3)).unwrap();
    assert_eq!(q.pending_sources(), 3);
    assert_eq!(q.available_gp(), 0);
    assert_eq!(
        q.observe_get(91, 1).unwrap(),
        [source(1), source(2), source(3)]
    );
}
#[test]
fn capacity_is_trusted_bounded_and_keeps_one_empty_slot() {
    for entries in [0, 1, 2, 3, 7, 513, 1024, u32::MAX] {
        assert!(matches!(
            Consumption::new(entries, 91),
            Err(Refusal::Capacity)
        ));
    }
    assert!(matches!(Consumption::new(8, 0), Err(Refusal::Lifetime)));
    let mut q = Consumption::new(4, 91).unwrap();
    q.publish(source(1)).unwrap();
    assert_eq!(q.available_gp(), 0);
    assert_eq!(q.publish(source(2)), Err(Refusal::Full));
}

#[test]
fn source_generations_survive_a_shared_native_ring_without_cross_writeback() {
    let mut q = Consumption::new(8, 91).unwrap();
    let old = ConsumedSource {
        generation: 23,
        guest_get: 123,
    };
    let new = ConsumedSource {
        generation: 24,
        guest_get: 0,
    };
    q.publish(old).unwrap();
    q.publish(new).unwrap();
    assert_eq!(q.observe_get(91, 3).unwrap(), [old]);
    assert_eq!(q.observe_get(91, 6).unwrap(), [new]);
}
