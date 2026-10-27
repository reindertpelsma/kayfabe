//! Bind bounded diagnostic metadata to the actual native operations.
const MEM: &str = include_str!("../src/mem.rs");
const READ: &str = include_str!("../src/userread.rs");
const DEVICE: &str = include_str!("../src/device.rs");
const RUNTIME: &str = include_str!("../../kf-host/src/latency/runtime.rs");
#[test]
fn row_writer_records_its_registered_thread_and_named_activity() {
    let guard: String = MEM
        .split("fn write_rows_releasing")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .collect();
    assert!(guard.contains("writer.store(kf_host::latency::thread_id()"));
    assert!(guard.contains("Activity::HeldRows"));
}
#[test]
fn row_writer_clears_ownership_before_unlocking_and_waking() {
    let drop = MEM.split("impl Drop for RowsWriteGuard").nth(1).unwrap();
    assert!(drop.contains("writer.store(0"));
    let wrapper = drop.split("fn write_rows_releasing").next().unwrap();
    assert!(!wrapper.contains("hub.released()"));
    let fields = MEM
        .split("struct RowsWriteGuard")
        .nth(1)
        .unwrap()
        .split("struct RowReleased")
        .next()
        .unwrap();
    assert!(fields.find("guard:").unwrap() < fields.find("_after_unlock:").unwrap());
}
#[test]
fn user_source_snapshots_keep_the_same_row_owner_metadata_across_revalidation() {
    assert_eq!(READ.matches("writer: log.writer.clone()").count(), 2);
    assert!(READ.contains("try_rows(&rows, &self.writer)"));
    assert!(READ.contains("Cause::Rows"));
}
#[test]
fn normal_and_storm_irq_outcomes_have_distinct_recording_entries() {
    let storm = DEVICE.split("fn irq_storm_tick(").nth(1).unwrap();
    assert!(storm.contains("latency::notification_storm(mask, sent)"));
    assert!(DEVICE.contains("latency::notification(mask, sent.is_ok())"));
}
#[test]
fn packet_and_split_context_both_prioritize_named_blockers() {
    assert_eq!(RUNTIME.matches("sort_by_key(|w| context_key(").count(), 2);
}

#[test]
fn reset_cancels_queued_split_latency_without_inventing_a_completion() {
    let p = MEM
        .split("pub fn reset_life(&self) -> usize")
        .nth(1)
        .unwrap()
        .split("if let Ok(mut m) = self.split_tokens")
        .next()
        .unwrap();
    assert!(p.contains("latency::split_canceled"));
}
#[test]
fn reset_cancels_running_and_released_split_latency() {
    let p = MEM
        .split("if let Ok(mut m) = self.split_tokens.lock()")
        .nth(1)
        .unwrap()
        .split("if let Ok(mut m) = self.gates")
        .next()
        .unwrap();
    assert_eq!(p.matches("latency::split_canceled").count(), 2);
}
