//! EXPECTED RED: naive deferred writes cross host latches. The subsequent research
//! implementation must make these tests pass without changing CpuIntr semantics.
use kf_chip::Family;
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
    assert_eq!(naive(&[(Reg::Leaf(0), 1)], 0).0, 1);
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
    assert_eq!(naive(&writes, 0), (1, 2));
}
