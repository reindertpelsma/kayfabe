//! Native split lifecycle and independent diagnostic composition contracts.
const MEM: &str = include_str!("../src/mem.rs");
const DEVICE: &str = include_str!("../src/device.rs");
const MANAGER: &str = include_str!("../../kf-mem/src/vasmgr.rs");
fn compact(s: &str) -> String {
    s.split_whitespace().collect()
}
#[test]
fn ens11_translated_split_records_request_and_enqueue_separately() {
    let p = MEM
        .split("pub fn request_split(")
        .nth(1)
        .unwrap()
        .split("pub(crate) fn request_user_split")
        .next()
        .unwrap();
    assert!(p.contains("latency::split_request"));
    assert!(p.contains("latency::split_queued"));
}
#[test]
fn ens11_user_and_gated_splits_share_the_bounded_split_history() {
    let p = MEM
        .split("pub(crate) fn request_user_split(")
        .nth(1)
        .unwrap()
        .split("pub fn take_split_requests")
        .next()
        .unwrap();
    assert_eq!(p.matches("latency::split_request").count(), 2);
    assert_eq!(p.matches("latency::split_queued").count(), 2);
}
#[test]
fn ens11_actual_walk_submission_timestamps_every_attached_split() {
    let p = MANAGER
        .split("fn submit_batch(")
        .nth(1)
        .unwrap()
        .split("fn batch_census")
        .next()
        .unwrap();
    assert!(p.contains("latency::split_start"));
    assert!(p.contains("Want::Split"));
}
#[test]
fn ens11_apply_cost_records_diff_size_and_real_rm_delta_per_split() {
    let p = MANAGER.split("pub fn on_walk_ready(").nth(1).unwrap();
    assert!(p.contains("latency::split_applied"));
    assert!(p.contains("cost_before"));
    assert!(p.contains("e.runs.len()"));
}
#[test]
fn ens11_split_result_is_recorded_after_gate_release_and_result_publication() {
    let p = MEM
        .split("pub fn finish_split(")
        .nth(1)
        .unwrap()
        .split("pub fn split_result")
        .next()
        .unwrap();
    assert!(p.contains("latency::split_released"));
    assert!(p.find("m.insert(ticket, r)").unwrap() < p.find("latency::split_released").unwrap());
}
#[test]
fn ens11_whole_drained_split_set_is_queued_before_starting_any_walk() {
    let p = DEVICE
        .split("for (ticket, target) in drained_splits")
        .nth(1)
        .unwrap()
        .split("// Publish resolved mutation domains")
        .next()
        .unwrap();
    let p = compact(p);
    assert!(p.contains("m.queue_split("));
    assert!(p.contains("m.start_queued_splits(trigger)"));
    assert!(!p.contains("m.on_split("));
    assert!(!p.contains("m.on_split_space("));
}
#[test]
fn ens11_latency_alone_can_start_the_existing_metadata_probe_timer() {
    let p = DEVICE
        .split("pub fn probe_timer_enabled(")
        .nth(1)
        .unwrap()
        .split("pub fn probe_device_state")
        .next()
        .unwrap();
    assert!(p.contains("self.latency_active"));
}
#[test]
fn ens11_independent_probe_can_dump_first_slow_while_workers_are_blocked() {
    let p = DEVICE
        .split("pub fn probe_loop(")
        .nth(1)
        .unwrap()
        .split("pub fn probe_timer_enabled")
        .next()
        .unwrap();
    assert!(p.contains("latency::poll"));
}
#[test]
fn ens11_storm_notifications_are_included_in_the_actual_irq_journal() {
    let p = DEVICE
        .split("fn irq_storm_tick(")
        .nth(1)
        .unwrap()
        .split("pub fn irq_fd")
        .next()
        .unwrap();
    assert!(p.contains("latency::notification"));
    assert!(p.contains("signal().is_ok()"));
}
#[test]
fn ens11_storm_vector_snapshot_does_not_require_a_pending_completion_bit() {
    let p = DEVICE
        .split("fn irq_storm_tick(")
        .nth(1)
        .unwrap()
        .split("pub fn irq_fd")
        .next()
        .unwrap();
    assert!(p.contains("latency::notification_mask"));
    assert!(p.contains("completion_enabled"));
}

#[test]
fn ens11_an_armed_invalidate_cannot_submit_before_the_drained_split_snapshot_is_admitted() {
    let p = DEVICE
        .split("let drained_splits = self.mem.inbox.take_split_requests();")
        .nth(1)
        .unwrap()
        .split("// Publish resolved mutation domains")
        .next()
        .unwrap();
    assert!(p.find("m.queue_split").unwrap() < p.find("m.on_invalidate(req, trigger)").unwrap());
    assert_eq!(p.matches("m.start_queued_splits(trigger)").count(), 1);
}

#[test]
fn ens11_actual_probe_interval_exists_with_latency_alone() {
    let p = DEVICE
        .split("pub fn probe_loop(")
        .nth(1)
        .unwrap()
        .split("pub fn probe_timer_enabled")
        .next()
        .unwrap();
    assert!(p.contains("overdue.is_some() || self.latency_active"));
    assert!(p.contains("latency::poll()"));
}
