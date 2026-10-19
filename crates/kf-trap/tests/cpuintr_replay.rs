//! Ordered serial replay gate. The naive fixture remains the documented negative control.
//! This does not prove concurrent publication, untrapped read coherence or autonomous progress.
use kf_chip::Family;
use kf_trap::cpuintr;
#[path = "../src/cpuintr_replay.rs"]
mod cpuintr_replay;
use cpuintr_replay::{CAPACITY, Owed, Replay, Write};
use kf_trap::cpuintr::{CpuIntr, Raise, Reg};

// Deliberately naive test fixture, not device code. Writes are queued, host
// latches apply immediately; all queued writes are replayed on the final drain.
fn naive(writes: &[(Reg, u32)], latch: u32) -> (u32, usize) {
    let t = CpuIntr::new(Family::Ada, 0xbb0000).unwrap();
    let mut messages = usize::from(t.latch(latch) == Raise::Message);
    for &(r, v) in writes {
        messages += usize::from(t.write(r, v) == Raise::Message);
    }
    (t.read(Reg::Leaf(0)), messages)
}

#[test]
fn w1c_before_a_host_latch_cannot_erase_the_new_pending_bit() {
    // Synchronous W1C(1), latch(0): pending=1. Deferred W1C runs after
    // latch and incorrectly acknowledges work that arrived after the write.
    let t = tree();
    let mut replay = Replay::new(&t);
    assert_eq!(replay.write(w(Reg::Leaf(0), 1)), Owed::default());
    assert_eq!(replay.latch(0), Owed::default());
    assert_eq!(t.read(Reg::Leaf(0)), 1);
    assert_eq!(
        naive(&[(Reg::Leaf(0), 1)], 0).0,
        0,
        "negative control still loses the latch"
    );
}

#[test]
fn trigger_before_a_host_latch_preserves_the_pre_latch_message() {
    // Synchronous enable, trigger, W1C, latch has TWO messages and pending=1.
    // Replaying these writes after the host latch loses its pending bit.
    let writes = [
        (Reg::TopEnSet, 1),
        (Reg::LeafEnSet(0), 1),
        (Reg::Trigger, 0),
        (Reg::Leaf(0), 1),
    ];
    let t = tree();
    let mut replay = Replay::new(&t);
    let mut messages = 0;
    for (reg, value) in writes {
        messages += replay.write(w(reg, value)).messages;
    }
    messages += replay.latch(0).messages;
    assert_eq!((t.read(Reg::Leaf(0)), messages), (1, 2));
    assert_eq!(
        naive(&writes, 0),
        (0, 2),
        "negative control loses pending state"
    );
}

fn tree() -> CpuIntr {
    CpuIntr::new(Family::Ada, 0xbb0000).unwrap()
}
fn w(reg: Reg, value: u32) -> Write {
    Write { reg, value }
}
fn equal(a: &CpuIntr, b: &CpuIntr, leaves: usize) {
    for i in 0..leaves {
        for r in [Reg::Leaf(i), Reg::LeafEnSet(i), Reg::LeafEnClear(i)] {
            assert_eq!(a.read(r), b.read(r), "{r:?}");
        }
    }
    for r in [Reg::Top, Reg::TopEnSet, Reg::TopEnClear, Reg::Trigger] {
        assert_eq!(a.read(r), b.read(r), "{r:?}");
    }
}
fn random_run(mut seed: u64) {
    for family in [
        Family::Turing,
        Family::Ampere,
        Family::Ada,
        Family::Hopper,
        Family::Blackwell,
    ] {
        let n = if matches!(family, Family::Hopper | Family::Blackwell) {
            16
        } else {
            8
        };
        let a = CpuIntr::new(family, 0xbb0000).unwrap();
        let b = CpuIntr::new(family, 0xbb0000).unwrap();
        let mut r = Replay::new(&b);
        let (mut synchronous, mut replayed) = (0u64, 0u64);
        let (mut sync_invalid, mut replay_invalid) = (0u64, 0u64);
        for step in 0..15_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let i = (seed as usize / 16) % n;
            let value = if step % 2 == 0 {
                1 << ((seed >> 32) % 32)
            } else {
                (seed >> 32) as u32
            };
            let (got, expected, barrier) = match seed % 11 {
                0 => {
                    let vector = ((seed >> 24) % 530) as u32;
                    (r.latch(vector), a.latch(vector), true)
                }
                1 => {
                    a.reset();
                    (r.reset(), Raise::None, true)
                }
                2 => (r.drain(), Raise::None, true),
                k => {
                    let reg = match k {
                        3 => Reg::LeafEnSet(i),
                        4 => Reg::LeafEnClear(i),
                        5 => Reg::TopEnSet,
                        6 => Reg::TopEnClear,
                        7 => Reg::Leaf(i),
                        8 => Reg::Trigger,
                        9 => Reg::Top,
                        _ => Reg::LeafEnSet(i),
                    };
                    (
                        r.write(w(reg, value)),
                        a.write(reg, value),
                        !cpuintr_replay::recordable(reg),
                    )
                }
            };
            synchronous += u64::from(expected == Raise::Message);
            sync_invalid += u64::from(expected == Raise::OutOfRange);
            replayed += u64::from(got.messages);
            replay_invalid += u64::from(got.out_of_range);
            if barrier {
                assert_eq!(
                    (replayed, replay_invalid),
                    (synchronous, sync_invalid),
                    "{family:?} seed{seed} step{step}; every synchronous message is due by this barrier"
                );
                equal(&a, &b, n);
            }
        }
        let tail = r.drain();
        replayed += u64::from(tail.messages);
        replay_invalid += u64::from(tail.out_of_range);
        equal(&a, &b, n);
        assert_eq!((replayed, replay_invalid), (synchronous, sync_invalid));
        eprintln!(
            "{family:?}: sync={synchronous} replay={replayed} extra={} invalid={sync_invalid}",
            replayed - synchronous
        );
    }
}
#[test]
fn random_replay_equivalence_seed_one() {
    random_run(0x12fedc098760);
}
#[test]
fn random_replay_equivalence_seed_two() {
    random_run(0xacd99875421);
}

#[test]
fn fifo_set_then_clear_is_not_a_final_value_store() {
    let t = tree();
    let mut r = Replay::new(&t);
    let _ = r.write(w(Reg::LeafEnSet(0), 1));
    let _ = r.write(w(Reg::LeafEnClear(0), 1));
    assert_eq!(r.drain(), Owed::default());
    assert_eq!(t.read(Reg::LeafEnSet(0)), 0);
}

#[test]
fn pure_replay_refuses_oversized_input_before_any_apply() {
    let t = tree();
    let writes = [w(Reg::TopEnSet, 1); CAPACITY + 1];
    assert_eq!(cpuintr_replay::replay(&t, &writes), None);
    assert_eq!(t.read(Reg::TopEnSet), 0);
    assert_eq!(
        cpuintr_replay::replay(&t, &writes[..CAPACITY]),
        Some(Owed::default())
    );
    assert_eq!(t.read(Reg::TopEnSet), 1);
}

#[test]
fn pure_replay_rejection_cannot_acknowledge_a_pending_edge() {
    let t = tree();
    t.latch(0);
    let writes = [w(Reg::Leaf(0), 1); CAPACITY + 1];
    assert_eq!(cpuintr_replay::replay(&t, &writes), None);
    assert_eq!(t.read(Reg::Leaf(0)), 1);
}
#[test]
fn owed_pending_edge_is_returned_at_explicit_drain() {
    let t = tree();
    let mut r = Replay::new(&t);
    assert_eq!(r.latch(0).messages, 0);
    let _ = r.write(w(Reg::LeafEnSet(0), 1));
    let _ = r.write(w(Reg::TopEnSet, 1));
    assert_eq!(r.drain().messages, 1);
    assert_eq!(
        r.drain().messages,
        0,
        "no second delivery of the same owed result"
    );
}
#[test]
fn owed_pending_edge_is_returned_before_w1c_barrier() {
    let t = tree();
    let mut r = Replay::new(&t);
    let _ = r.latch(0);
    let _ = r.write(w(Reg::TopEnSet, 1));
    let _ = r.write(w(Reg::LeafEnSet(0), 1));
    assert_eq!(r.write(w(Reg::Leaf(0), 1)).messages, 1);
    assert_eq!(t.read(Reg::Leaf(0)), 0);
}
#[test]
fn capacity_flush_never_drops_a_write_or_message() {
    let t = tree();
    let mut r = Replay::new(&t);
    let _ = r.latch(0);
    let _ = r.write(w(Reg::TopEnSet, 1));
    let _ = r.write(w(Reg::LeafEnSet(0), 1));
    for _ in 2..CAPACITY {
        let _ = r.write(w(Reg::LeafEnSet(0), 0));
    }
    assert_eq!(r.write(w(Reg::LeafEnClear(0), 1)).messages, 1);
    let _ = r.drain();
    assert_eq!(t.read(Reg::LeafEnSet(0)), 0);
}
#[test]
fn deferred_mask_reads_are_stale_even_when_eventual_irq_replay_is_correct() {
    let t = tree();
    let mut r = Replay::new(&t);
    let _ = r.write(w(Reg::LeafEnSet(0), 1));
    assert_eq!(
        t.read(Reg::LeafEnSet(0)),
        0,
        "counterexample: a guest read is not a drain"
    );
    let _ = r.drain();
    assert_eq!(t.read(Reg::LeafEnClear(0)), 1);
}
#[test]
fn unmask_without_a_later_event_has_no_autonomous_delivery() {
    let t = tree();
    let mut r = Replay::new(&t);
    let _ = r.latch(0);
    let _ = r.write(w(Reg::LeafEnSet(0), 1));
    assert_eq!(r.write(w(Reg::TopEnSet, 1)).messages, 0);
    assert_eq!(
        t.read(Reg::TopEnSet),
        0,
        "nothing wakes a worker in this pure model"
    );
    assert_eq!(
        r.drain().messages,
        1,
        "requires an actual future drain, not elapsed time"
    );
}
