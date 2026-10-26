//! Scope guards supplement the virtual-time and actual Inbox tests.
const MANAGER: &str = include_str!("../../kf-mem/src/vasmgr.rs");
const CHAN: &str = include_str!("../src/chan.rs");
#[test]
fn delay_is_between_begin_and_apply_with_end_recording_afterwards() {
    let begin = MANAGER.find(".ens3_begin(").unwrap();
    let request = MANAGER[begin..].find(".ens4_delay_requested(").unwrap() + begin;
    let inject = MANAGER[request..]
        .find("crate::applydelay::inject(e.runs.len())")
        .unwrap()
        + request;
    let apply = MANAGER[inject..].find("apply_entry(&space.target").unwrap() + inject;
    let end = MANAGER[apply..].find(".ens3_end(").unwrap() + apply;
    assert!(begin < request && request < inject && inject < apply && apply < end);
}
#[test]
fn injection_is_local_to_va_apply_and_shown_in_status() {
    assert!(MANAGER.contains("crate::applydelay::inject(e.runs.len())"));
    assert!(CHAN.contains("kf_mem::applydelay::summary()"));
    assert!(!CHAN.contains("applydelay::inject("));
}
#[test]
fn linux_large_apply_coverage_is_logged_even_when_the_dose_is_zero() {
    assert!(MANAGER.contains("kf3: ENS4 APPLY space="));
    assert!(MANAGER.contains("e.runs.len() >= 1000"));
}

const PUMP: &str = include_str!("../src/userpump.rs");
#[test]
fn all_actual_marker_publish_paths_record_successful_counts() {
    assert_eq!(PUMP.matches("ens4_actual_marker =").count(), 3);
    assert!(PUMP.contains("self.core.marks.issued() > marker_before"));
}
#[test]
fn gate_and_batch_markers_record_counts_only_after_publication_returns_success() {
    let compact = PUMP.split_whitespace().collect::<String>();
    assert!(compact.contains(")?;self.aux.ens4_actual_marker=count;"));
    assert!(compact.contains(")?;self.aux.ens4_actual_marker=marks.issued();"));
}
#[test]
fn marker_seen_and_the_used_observation_timestamp_feed_the_endpoint() {
    let body = PUMP
        .split("rec.ens4_turn(")
        .nth(1)
        .unwrap()
        .split(");")
        .next()
        .unwrap();
    assert!(body.contains("self.core.marks.seen()"));
    assert!(body.contains("self.aux.ens4_observed_ns"));
    assert!(!body.contains("get"));
}
#[test]
fn completion_comes_from_validated_marker_progress_not_raw_get_or_irq() {
    assert!(PUMP.contains("rec.ens4_turn("));
    assert!(PUMP.contains("self.core.marks.seen()"));
    assert!(CHAN.contains("gatelog::global().ens4_summary()"));
    assert!(CHAN.contains("ens4_worst_gate_apply_complete_ns=NA ens4_metric=off"));
}
#[test]
fn staged_witnesses_register_actual_marker_success_too() {
    let body = PUMP
        .split("let marker_before = self.core.marks.issued()")
        .nth(1)
        .unwrap()
        .split("fn segment_turn(")
        .next()
        .unwrap();
    assert!(body.contains("result.is_ok() && self.core.marks.issued() > marker_before"));
    assert!(body.contains("self.aux.ens4_actual_marker = self.core.marks.issued()"));
}
#[test]
fn turn_endpoint_has_exact_actual_marker_and_validated_seen_arguments() {
    let compact = PUMP.split_whitespace().collect::<String>();
    assert!(compact.contains(
        "rec.ens4_turn(chan.token,self.aux.ens4_actual_marker,\
        self.core.marks.seen(),self.aux.ens4_observed_ns,rec.now(),)"
    ));
}
#[test]
fn every_direct_private_put_updates_the_existing_ens3_progress_too() {
    assert_eq!(PUMP.matches("self.ens3_published(").count(), 3);
}
#[test]
fn pointer_gate_and_batch_publications_have_progress_hooks_after_success() {
    for (first, next) in [
        ("fn publish_entry(", "fn publish_gate("),
        ("fn publish_gate(", "fn publish_marker("),
        ("fn publish_marker(", "fn event("),
    ] {
        let body = PUMP
            .split(first)
            .nth(1)
            .unwrap()
            .split(next)
            .next()
            .unwrap();
        assert!(body.contains("self.ens3_published("));
    }
}
#[test]
fn receipt_invariants_are_checked_unconditionally_before_optional_diagnostics() {
    let turn = PUMP
        .split("fn turn(&mut self, me: &ChanPlane")
        .nth(1)
        .unwrap();
    let check = turn.find(".ens4_watch").unwrap();
    let optional = turn[check..].find("if kf_host::diag::enabled()").unwrap() + check;
    assert!(check < optional);
    assert!(turn[check..optional].contains("ens4_unissued_seen"));
    assert!(turn[check..optional].contains("ens4_sequence_regressed"));
}
#[test]
fn invariant_bits_have_status_counters_and_never_change_the_turn_result() {
    let turn = PUMP.split("let receipt_bits =").nth(1).unwrap();
    let check = turn.split("// Only validated").next().unwrap();
    assert!(check.contains("ens4_unissued_seen"));
    assert!(check.contains("ens4_sequence_regressed"));
    assert!(!check.contains("return"));
    assert!(!check.contains("panic!"));
}
