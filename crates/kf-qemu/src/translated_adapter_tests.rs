//! Structural checks of the drainer/act split, supplementing real cell tests.
//! Source pins do not simulate a GPU or prove host ioctl latency.

fn diagnostic_drainer() -> &'static str {
    include_str!("chan.rs")
        .split("    pub fn bar0trace_dump(")
        .nth(1)
        .expect("diagnostic entry")
        .split("    fn bar0trace_dump_act(")
        .next()
        .expect("diagnostic drainer half")
}

#[test]
fn translated_diagnostic_drainer_never_takes_the_pump_lock() {
    assert!(
        !diagnostic_drainer().contains("slot.lock()"),
        "default-off diagnostic still blocks the drainer behind a pump"
    );
}

#[test]
fn translated_diagnostic_drainer_queues_host_views_to_the_act_thread() {
    let drainer = diagnostic_drainer();
    assert!(drainer.contains("self.defer("), "diagnostic must run as an act");
    assert!(!drainer.contains("arm_cpu_view"), "host ioctl on the drainer");
    assert!(!drainer.contains("release_cpu_view"), "host ioctl on the drainer");
}
