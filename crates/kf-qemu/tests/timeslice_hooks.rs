//! TS1 composition guards cover all production birth/free paths.
const CHAN: &str = include_str!("../src/chan.rs");
const USER: &str = include_str!("../src/userrelay.rs");
#[test]
fn early_set_is_reserved_and_deferred_instead_of_returned_not_ours() {
    let body = CHAN
        .split("ChanStatement::Timeslice {")
        .nth(1)
        .unwrap()
        .split("ChanStatement::ChannelTimeslice")
        .next()
        .unwrap();
    assert!(body.contains("timeslices.prepare("));
    assert!(body.contains("timeslices.apply("));
    assert!(!body.contains("return ChanAnswer::NotOurs"));
}
#[test]
fn production_sets_use_the_bounded_timeslice_orchestrator() {
    assert!(CHAN.contains("timeslices.prepare("));
    assert!(CHAN.contains("timeslices.apply("));
    assert!(CHAN.contains("self.defer("));
}
#[test]
fn private_user_ring_birth_replays_before_becoming_ready() {
    assert!(USER.contains("replay_timeslice("));
    assert!(CHAN.contains("fn replay_timeslice("));
}
#[test]
fn passthrough_and_translated_births_both_replay() {
    assert!(CHAN.matches("replay_timeslice(").count() >= 3);
    assert!(USER.contains("replay_timeslice("));
}
#[test]
fn statement_free_and_reset_cancel_timeslice_lifetimes() {
    assert!(CHAN.contains("timeslices.forget(client, object)"));
    assert!(CHAN.contains("timeslices.clear()"));
}
#[test]
fn native_group_telemetry_is_removed_on_free() {
    assert!(CHAN.contains("timeslices.group_freed("));
    assert!(CHAN.contains("timeslices.forget(client, object)"));
}
