//! Composition guards: diagnostics must surround the real operations, not test-only models.
const VA: &str = include_str!("../../kf-mem/src/vasmgr.rs");
const PUMP: &str = include_str!("../src/userpump.rs");
const HOST: &str = include_str!("../../kf-host/src/lib.rs");
const SPARSE: &str = include_str!("../../kf-host/src/sparse.rs");
const CHAN: &str = include_str!("../src/chan.rs");

#[test]
fn every_real_apply_is_bracketed_for_ens3() {
    assert!(VA.contains(".ens3_begin("));
    assert!(VA.contains(".ens3_end("));
    assert!(VA.contains("kf_host::diag::snapshot()"));
}
#[test]
fn apply_cost_is_finished_after_apply_and_before_vahist_printing() {
    let begin = VA.find(".ens3_begin(").unwrap();
    let apply = VA[begin..].find("let a = apply_entry(").unwrap() + begin;
    let end = VA[apply..].find(".ens3_end(").unwrap() + apply;
    let print = VA[apply..].find("if crate::maplog::on()").unwrap() + apply;
    assert!(begin < apply && apply < end && end < print);
}
#[test]
fn rm_ioctl_and_invalidate_category_are_both_measured() {
    assert!(HOST.contains("diag::rm(false, ||"));
    assert!(HOST.contains("cmd == NV2080_CTRL_CMD_DMA_INVALIDATE_TLB"));
}
#[test]
fn sparse_reservation_alloc_free_map_and_unmap_reach_timed_ioctls() {
    for name in [
        "fn raw_alloc_exact_layout(",
        "fn raw_map_dma_slice(",
        "fn raw_unmap_dma_range(",
        "pub fn free(",
    ] {
        let body = HOST.split(name).nth(1).unwrap();
        assert!(
            body.split(".map_err(|e| ioctl_error(&e))?")
                .next()
                .unwrap()
                .contains("diag::rm("),
            "{name}"
        );
    }
    assert!(HOST.contains("cmd == NV2080_CTRL_CMD_DMA_INVALIDATE_TLB"));
}
#[test]
fn sparse_admission_clone_and_restore_scan_have_separate_costs() {
    for phase in ["Phase::Admission", "Phase::Clone", "Phase::Restore"] {
        assert!(SPARSE.contains(phase), "{phase}");
    }
}
#[test]
fn cost_work_counts_actual_clone_blocks_and_restore_page_visits() {
    assert!(SPARSE.contains("Phase::Clone, self.blocks.len() as u64"));
    assert!(SPARSE.contains("Phase::Restore"));
    assert!(SPARSE.contains("diag_scan_pages"));
}
#[test]
fn real_private_put_and_existing_hw_words_feed_snapshot() {
    assert!(PUMP.contains("ens3_progress"));
    assert!(PUMP.contains("publish_ns:"));
    assert!(PUMP.contains("observe_ns:"));
}
#[test]
fn published_sequence_and_marker_come_from_real_fetch_and_marks() {
    assert!(PUMP.contains("ens3_progress"));
    assert!(PUMP.contains("put: self.core.fetch.issued()"));
    assert!(PUMP.contains("marker: self.core.marks.issued()"));
}
#[test]
fn first153_observer_is_started_separately_from_va_apply() {
    assert!(CHAN.contains("self.start_ens3_diag()"));
    assert!(CHAN.contains("mod ens3;"));
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ens3.rs");
    let s = std::fs::read_to_string(p).unwrap_or_default();
    assert!(s.contains("poller.poll("));
    assert!(s.contains("snapshot_153("));
}
#[test]
fn snapshot_thread_is_off_by_default_and_collects_nsi_counters() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ens3.rs");
    let s = std::fs::read_to_string(p).unwrap_or_default();
    assert!(s.contains("kf_host::diag::enabled()"));
    assert!(s.contains("self.pt_summary()"));
    assert!(s.contains("snapshot_153("));
}
