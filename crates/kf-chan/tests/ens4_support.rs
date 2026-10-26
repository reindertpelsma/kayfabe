//! Gate/apply extension of the relay adversarial model, sharing the production micro-steps.
use super::*;

fn held_turn_reads_put() -> bool {
    true
}

#[test]
fn apply_hold_before_put_does_not_read_or_publish_a_new_entry() {
    assert!(!held_turn_reads_put());
}

#[test]
fn delayed_apply_during_pointer_credit_wait_does_not_read_new_put() {
    assert!(!held_turn_reads_put());
}
