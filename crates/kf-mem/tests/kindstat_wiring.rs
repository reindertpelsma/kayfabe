//! KINDSTAT must observe applied GPU rows and remain optional.

#[test]
fn successful_gpu_rows_have_a_kind_observer() {
    let source = include_str!("../src/apply.rs");
    assert!(
        source.contains("apply_entry_observed"),
        "missing mapping census"
    );
    assert!(source.contains("record_mapping"));
}

#[test]
fn status_and_shutdown_have_default_off_kindstat_hooks() {
    let source = include_str!("../../kf-qemu/src/device.rs");
    assert!(
        source.contains("KF3_DIAG_KINDSTAT"),
        "missing runtime opt-in"
    );
    assert!(source.contains("KINDSTAT SUMMARY"));
}

#[test]
fn normal_process_exit_registers_the_kindstat_summary() {
    let c = include_str!("../../../qemu/hw/misc/kf3/kf3.c");
    let ffi = include_str!("../../kf-qemu/src/ffi_unsafe.rs");
    assert!(c.contains("s->kindstat_exit.notify = kf3_kindstat_exit;"));
    assert!(c.contains("qemu_add_exit_notifier(&s->kindstat_exit);"));
    assert!(c.contains("kf3_kindstat_finish(s->h);"));
    assert!(ffi.contains("d.kindstat_finish();"));
}

#[test]
fn device_unplug_and_process_exit_share_the_once_guard() {
    let c = include_str!("../../../qemu/hw/misc/kf3/kf3.c");
    let ffi = include_str!("../../kf-qemu/src/ffi_unsafe.rs");
    let device = include_str!("../../kf-qemu/src/device.rs");
    assert!(c.contains("qemu_add_exit_notifier(&s->kindstat_exit);"));
    assert!(c.contains("qemu_remove_exit_notifier(&s->kindstat_exit);"));
    assert!(c.contains("kf3_kindstat_finish(s->h);"));
    assert!(ffi.contains("d.kindstat_finish();"));
    assert!(device.contains("self.kindstat_finish();"));
    assert!(device.contains("diag.shutdown()"));
}
