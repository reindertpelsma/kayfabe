//! TS1 composition guards cover all production birth/free paths.
const CHAN: &str = include_str!("../src/chan.rs");
const USER: &str = include_str!("../src/userrelay.rs");
fn has(text: &str, needle: &str) -> bool {
    let compact = |s: &str| s.split_whitespace().collect::<String>();
    compact(text).contains(&compact(needle))
}
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
    assert!(has(body, "timeslices.apply("));
    assert!(!body.contains("return ChanAnswer::NotOurs"));
}
#[test]
fn production_sets_use_the_bounded_timeslice_orchestrator() {
    assert!(CHAN.contains("timeslices.prepare("));
    assert!(has(CHAN, "timeslices.apply("));
    assert!(CHAN.contains("self.defer("));
}
#[test]
fn private_user_ring_birth_replays_before_becoming_ready() {
    assert!(USER.contains("replay_timeslice("));
    assert!(CHAN.contains("fn replay_timeslice("));
}
#[test]
fn passthrough_and_translated_births_both_replay() {
    assert!(CHAN.matches("replay_timeslice(").count() == 5);
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
#[test]
fn every_timeslice_call_uses_the_fact_wrapped_owned_setter() {
    assert!(has(CHAN, ".apply(&queued, &twins, &me.timeslice_host())"));
    assert!(has(
        CHAN,
        ".birth((a.client, tsg), chan, group_new, &self.timeslice_host())"
    ));
}
#[test]
fn birth_set_and_bind_facts_are_wired_into_the_existing_adapter() {
    for needle in [
        "self.tsg_facts.birth(",
        "\"ctxsw-set\"",
        "\"zcull-bind\"",
        "self.tsg_facts.forget(client, object)",
        "self.tsg_facts.clear()",
    ] {
        assert!(has(CHAN, needle), "missing {needle}");
    }
}

#[test]
fn each_passthrough_birth_kind_replays_after_mirror_acceptance() {
    let body = CHAN
        .split("fn birth_in_group(")
        .nth(1)
        .unwrap()
        .split("fn relay_userd(")
        .next()
        .unwrap();
    assert_eq!(body.matches("self.replay_timeslice(").count(), 3);
}
#[test]
fn translated_birth_replays_and_cleans_up_before_publishing_a_slot() {
    let body = CHAN
        .split("\"birth translated\"")
        .nth(1)
        .unwrap()
        .split("let slot = Slot")
        .next()
        .unwrap();
    assert!(body.contains("me.replay_timeslice("));
    assert!(body.contains("me.free_timesliced_channel("));
}
#[test]
fn native_cleanup_is_separate_from_guest_lifetime_invalidation() {
    let body = CHAN
        .split("fn free_timesliced_channel(")
        .nth(1)
        .unwrap()
        .split("fn release_twin(")
        .next()
        .unwrap();
    assert!(body.contains("self.timeslices.group_freed(chan.tsg)"));
    assert!(!body.contains("timeslices.forget("));
    assert!(CHAN.contains("self.timeslices.clear()"));
}
#[test]
fn rejected_queue_admission_cancels_the_exact_reserved_ticket() {
    let body = CHAN
        .split("ChanStatement::Timeslice {")
        .nth(1)
        .unwrap()
        .split("ChanStatement::ChannelTimeslice")
        .next()
        .unwrap();
    assert!(has(body, "if matches!(answer, ChanAnswer::Refused { .. })"));
    assert!(body.contains("self.timeslices.cancel(&ticket)"));
}
#[test]
fn reservation_has_a_cancel_path_as_well_as_an_act_consumer() {
    assert!(has(CHAN, "timeslices.apply("));
    assert!(CHAN.contains("timeslices.cancel(&ticket)"));
    assert!(CHAN.contains("let queued = ticket.clone()"));
}
