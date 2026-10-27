//! Bind bounded diagnostic metadata to the actual native operations.
const MEM: &str = include_str!("../src/mem.rs");
const READ: &str = include_str!("../src/userread.rs");
const DEVICE: &str = include_str!("../src/device.rs");
const RUNTIME: &str = include_str!("../../kf-host/src/latency/runtime.rs");
#[test]
fn row_writer_records_its_registered_thread_and_named_activity() {
    let guard = MEM.split("fn write_rows_releasing").nth(1).unwrap();
    assert!(guard.contains("writer.store(kf_host::latency::thread_id()"));
    assert!(guard.contains("Activity::HeldRows"));
}
#[test]
fn row_writer_clears_ownership_before_unlocking_and_waking() {
    let drop = MEM.split("impl Drop for RowsWriteGuard").nth(1).unwrap();
    let clear = drop.find("writer.store(0").unwrap();
    assert!(clear < drop.find("drop(self.guard.take())").unwrap());
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
    assert!(DEVICE.contains("latency::notification(mask, sent)"));
}
#[test]
fn packet_and_split_context_both_prioritize_named_blockers() {
    assert_eq!(RUNTIME.matches("sort_by_key(|w| context_key(").count(), 2);
}
