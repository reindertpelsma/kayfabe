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
