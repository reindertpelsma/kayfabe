//! EXPECTED RED frontiers and passive runtime invariant monitor.
#[path = "../src/gatelog/ens4_packet.rs"]
mod packet;
use packet::{Packet, Watch};
#[test]
fn held_announcement_is_class_i() {
    assert_eq!(Packet::default().class(true), "i_unpublished");
}
#[test]
fn newer_held_packet_is_not_the_prior_completed_packet() {
    let mut p = Packet::default();
    p.published();
    p.marker(1);
    p.seen(1);
    assert_eq!(p.class(true), "i_unpublished");
}
#[test]
fn published_without_marker_coverage_stays_unknown() {
    let mut p = Packet::default();
    p.published();
    assert_eq!(p.class(false), "unknown_no_covering_marker");
}
#[test]
fn old_marker_does_not_cover_later_publication() {
    let mut p = Packet::default();
    p.published();
    p.marker(1);
    p.seen(1);
    p.published();
    assert_eq!(p.class(false), "unknown_no_covering_marker");
}
#[test]
fn covered_pending_host_completion_is_class_iii_observation() {
    let mut p = Packet::default();
    p.published();
    p.marker(1);
    assert_eq!(p.class(false), "iii_host_completion_unobserved");
}
#[test]
fn newer_covering_marker_needs_its_own_observation() {
    let mut p = Packet::default();
    p.published();
    p.marker(1);
    p.seen(1);
    p.published();
    p.marker(2);
    assert_eq!(p.class(false), "iii_host_completion_unobserved");
}
#[test]
fn completed_marker_without_guest_evidence_is_only_ii_candidate() {
    let mut p = Packet::default();
    p.published();
    p.marker(1);
    p.seen(1);
    assert_eq!(p.class(false), "ii_candidate_guest_delivery_unknown");
}
#[test]
fn future_reference_cannot_establish_completion() {
    let mut p = Packet::default();
    p.published();
    p.marker(1);
    p.seen(2);
    assert_eq!(p.class(false), "iii_host_completion_unobserved");
}
#[test]
fn unissued_seen_has_named_bit() {
    assert_eq!(Watch::default().check(1, 2), 1);
}
#[test]
fn unissued_seen_after_normal_turn_is_detected() {
    let mut w = Watch::default();
    assert_eq!(w.check(2, 1), 0);
    assert_eq!(w.check(2, 3), 1);
}
#[test]
fn seen_regression_has_named_bit() {
    let mut w = Watch::default();
    w.check(3, 2);
    assert_eq!(w.check(3, 1), 2);
}
#[test]
fn publication_regression_has_named_bit() {
    let mut w = Watch::default();
    w.check(3, 1);
    assert_eq!(w.check(2, 1), 4);
}
#[test]
fn reset_means_new_monitor_not_forgiven_regression() {
    let mut w = Watch::default();
    w.check(5, 4);
    assert_eq!(w.check(0, 0), 6);
    assert_eq!(Watch::default().check(0, 0), 0);
}
#[test]
fn another_completed_cohort_still_cannot_prove_guest_delivery() {
    let mut p = Packet::default();
    p.published();
    p.marker(7);
    p.seen(7);
    assert_eq!(p.class(false), "ii_candidate_guest_delivery_unknown");
}
#[test]
fn unissued_reference_after_valid_progress_does_not_complete_new_marker() {
    let mut p = Packet::default();
    p.published();
    p.marker(1);
    p.seen(1);
    p.published();
    p.marker(2);
    p.seen(3);
    assert_eq!(p.class(false), "iii_host_completion_unobserved");
}
#[test]
fn repeated_marker_does_not_gain_coverage_of_new_work() {
    let mut p = Packet::default();
    p.published();
    p.marker(1);
    p.seen(1);
    p.published();
    p.marker(1);
    assert_eq!(p.class(false), "unknown_no_covering_marker");
}
#[test]
fn stale_marker_does_not_gain_coverage_of_new_work() {
    let mut p = Packet::default();
    p.published();
    p.marker(2);
    p.seen(2);
    p.published();
    p.marker(1);
    assert_eq!(p.class(false), "unknown_no_covering_marker");
}
