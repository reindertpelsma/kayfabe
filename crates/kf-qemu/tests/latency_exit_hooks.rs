//! Normal QEMU exit bypasses unrealize; latency must not depend on the gate-log flag.
const FFI: &str = include_str!("../src/ffi_unsafe.rs");
fn registration() -> &'static str {
    FFI.split("// ENS12: latency exit registration")
        .nth(1)
        .expect("independent latency exit registration")
        .split("if !out.is_null()")
        .next()
        .unwrap()
}
fn handler() -> &'static str {
    FFI.split("extern \"C\" fn latency_atexit()")
        .nth(1)
        .expect("latency exit callback")
        .split("\n}\n")
        .next()
        .unwrap()
}
#[test]
fn ens12_normal_qemu_exit_registers_latency_without_gatelog() {
    let r = registration();
    assert!(r.contains("kf_host::latency::now().is_some()"));
    assert!(!r.contains("gatelog::enabled"));
    assert!(r.contains("atexit(latency_atexit)"));
}
#[test]
fn ens12_live_device_registers_one_latency_callback_after_threads_start() {
    let r = registration();
    assert!(r.contains("Once") && r.contains("call_once"));
    assert!(r.contains("atexit(latency_atexit)"));
    assert!(
        FFI.find(".spawn(move || d.va_loop())").unwrap()
            < FFI.find("// ENS12: latency exit registration").unwrap()
    );
}
#[test]
fn ens12_exit_callback_flushes_latency_and_contains_stderr_panics() {
    let h = handler();
    assert!(h.contains("catch_unwind"));
    assert!(h.contains("kf_host::latency::dump_shutdown"));
}
#[test]
fn ens12_exit_callback_uses_the_idempotent_dump_without_changing_guest_state() {
    let h = handler();
    assert!(h.contains("catch_unwind(kf_host::latency::dump_shutdown)"));
    assert!(!h.contains("gatelog"));
    assert!(!h.contains("reset(") && !h.contains("init(") && !h.contains("notification("));
}
