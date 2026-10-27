// Source-composition tests bind recorder contracts to actual native boundaries.
const PUMP: &str = include_str!("../src/userpump.rs");
const DEVICE: &str = include_str!("../src/device.rs");
const HOST: &str = include_str!("../../kf-chan/src/host.rs");
const WORKER: &str = include_str!("../../kf-chan/src/worker.rs");
#[test]
fn ens11_user_pointer_publication_records_successful_native_doorbell() {
    let p = PUMP
        .split("fn publish_entry(")
        .nth(1)
        .unwrap()
        .split("fn publish_gate(")
        .next()
        .unwrap();
    assert!(p.contains("latency.published"));
    assert!(p.find("publish_guarded(").unwrap() < p.find("latency.published").unwrap());
}
#[test]
fn ens11_scanned_publication_records_after_the_guarded_doorbell() {
    let p = PUMP
        .split("    fn publish(\n")
        .nth(1)
        .unwrap()
        .split("fn segment_turn(")
        .next()
        .unwrap();
    assert!(p.contains("latency.published"));
    assert!(p.contains("result.is_ok()"));
}
#[test]
fn ens11_user_reference_observation_uses_the_first_word_read_time() {
    let p = PUMP
        .split("fn hw_words(")
        .nth(1)
        .unwrap()
        .split("fn write_guest_get(")
        .next()
        .unwrap();
    assert!(p.contains("latency_word"));
    assert!(PUMP.contains("latency.marker"));
}
#[test]
fn ens11_peek_observation_is_also_recorded_before_pumping() {
    let p = PUMP.split("fn user_ring_peek_one(").nth(1).unwrap();
    assert!(p.contains("latency.marker"));
    assert!(p.contains("marks.peek(reference)"));
}
#[test]
fn ens11_private_fence_is_recorded_only_after_successful_doorbell() {
    let p = HOST
        .split("pub fn fence_releasing(")
        .nth(1)
        .unwrap()
        .split("pub const GATE_OFF")
        .next()
        .unwrap();
    assert!(p.contains("latency.published"));
    assert!(p.find("rm.doorbell(").unwrap() < p.find("latency.published").unwrap());
}
#[test]
fn ens11_private_fence_observation_is_recorded_at_the_native_load() {
    let p = HOST
        .split("pub fn completed(")
        .nth(1)
        .unwrap()
        .split("pub fn push(")
        .next()
        .unwrap();
    assert!(p.contains("latency.marker"));
    assert!(p.contains("load_u32"));
}
#[test]
fn ens11_notification_records_success_not_just_a_logical_latch() {
    let p = DEVICE
        .split("    fn deliver(")
        .nth(1)
        .unwrap()
        .split("pub fn latch_and_deliver")
        .next()
        .unwrap();
    assert!(p.contains("latency::notification"));
    assert!(p.contains("is_ok()"));
}
#[test]
fn ens11_notification_path_has_no_new_packet_recorder_lock() {
    let p = DEVICE
        .split("    fn deliver(")
        .nth(1)
        .unwrap()
        .split("/// ★ v3-initrace")
        .next()
        .unwrap();
    assert!(p.contains("latency::notification"));
    assert!(!p.contains(".lock()"));
}
#[test]
fn ens11_worker_activity_and_loop_progress_are_native_hooks() {
    assert!(WORKER.contains("latency::worker_loop"));
    assert!(WORKER.contains("latency::activity"));
}
#[test]
fn ens11_real_rm_verbs_capture_the_calling_threads_operation() {
    let src = include_str!("../../kf-host/src/diag.rs");
    assert!(src.contains("latency::activity"));
    assert!(src.contains("Activity::Rm"));
}
