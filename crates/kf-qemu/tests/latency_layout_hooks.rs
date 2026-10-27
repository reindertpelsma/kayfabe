//! Native composition checks supplement the reservation and relay behavioral models.
const MEM: &str = include_str!("../src/mem.rs");
const DEVICE: &str = include_str!("../src/device.rs");
const MANAGER: &str = include_str!("../../kf-mem/src/vasmgr.rs");
fn body<'a>(s: &'a str, start: &str, end: &str) -> &'a str {
    s.split(start).nth(1).unwrap().split(end).next().unwrap()
}
#[test]
fn deferred_layout_is_run_only_after_split_receipts_and_scope_release() {
    let step = DEVICE
        .find("m.retry_deferred_layout()")
        .expect("idle layout retry");
    assert!(DEVICE[..step].rfind("finish_split(ticket, res)").is_some());
    assert!(DEVICE[..step].rfind("finish_gate_loop").is_some());
}
#[test]
fn deferred_layout_rearms_owed_work_without_a_timer() {
    let idle = body(
        DEVICE,
        "// Existing invalidates/reconciliation",
        "if self.latency_active",
    );
    assert!(idle.contains("allow_deferred_layout()"));
    assert!(idle.contains("m.retry_deferred_layout()"));
    assert!(idle.contains("has_deferred_layout()"));
    assert!(idle.contains("inbox.wake.signal()"));
}
#[test]
fn every_split_request_interrupts_optional_layout_under_the_queue_lock() {
    for (start, end) in [
        ("pub fn request_split(", "pub(crate) fn request_user_split"),
        (
            "pub(crate) fn request_user_split(",
            "pub fn request_gated_split",
        ),
        ("pub fn request_gated_split(", "pub fn take_split_requests"),
    ] {
        let p = body(MEM, start, end);
        let push = p.find("q.push(").unwrap();
        assert!(p[push..].contains("interrupt_layout()"));
    }
}
#[test]
fn optional_layout_continuations_are_bounded_by_live_target_identity() {
    assert!(MANAGER.contains("deferred_layout: BTreeSet<VasKey>"));
    let remove = body(
        MANAGER,
        "pub fn remove(&mut self, key: VasKey) -> Option<T> {",
        "fn flush_released",
    );
    assert!(remove.contains("deferred_layout.remove(&key)"));
    assert!(MANAGER.contains("deferred_layout.clear()"));
}
#[test]
fn resolved_pdb_scope_is_published_before_its_global_hold_is_narrowed() {
    let resolved = DEVICE
        .find("mark_split_scopes_resolved(&admitted_splits)")
        .unwrap();
    assert!(
        DEVICE[..resolved]
            .rfind("set_walk_scope(keys, global")
            .is_some()
    );
    assert!(
        DEVICE[resolved..]
            .find("m.on_walk_ready(trigger)")
            .is_some()
    );
}
#[test]
fn manager_critical_scope_covers_all_applies_in_a_mixed_batch() {
    let scope = MANAGER.find("let _split_critical").unwrap();
    let apply = MANAGER[scope..].find("let a = apply_entry").unwrap();
    assert!(apply > 0);
    assert!(MANAGER[scope..scope + apply].contains("Want::Split"));
}
#[test]
fn row_writers_use_scope_release_on_all_mapping_paths() {
    let gpu = body(MEM, "impl GpuMirror", "pub struct Mirror");
    assert!(!gpu.contains("self.rows.write()"));
    assert!(
        gpu.matches("write_rows_releasing(&self.rows, &self.log)")
            .count()
            >= 6
    );
}
#[test]
fn row_unlock_happens_before_the_handoff_callback() {
    let drop = body(
        MEM,
        "impl Drop for RowsWriteGuard",
        "fn write_rows_releasing",
    );
    assert!(drop.find("drop(self.guard.take())").unwrap() < drop.find("hub.released()").unwrap());
}
