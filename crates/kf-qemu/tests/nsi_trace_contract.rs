//! P1 diagnostic wiring gates. These pin the absence of production measurement work and
//! the placement of observations on workers, before delivery. Runtime trace tests live beside
//! the recorder; these checks cover wiring that a recorder-only test cannot exercise.

fn body<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    source
        .split_once(start)
        .unwrap()
        .1
        .split_once(end)
        .unwrap()
        .0
}

#[test]
fn nsi_measurement_is_compile_time_excluded_from_production() {
    let manifest = include_str!("../Cargo.toml");
    assert!(
        manifest.contains("diag-nsi = []"),
        "missing opt-in diagnostic feature"
    );
    assert!(
        manifest.contains("default = []"),
        "production must not enable diagnostics"
    );
    let lib = include_str!("../src/lib.rs");
    assert!(lib.contains("#[cfg(feature = \"diag-nsi\")]\npub mod nsitrace;"));
}

#[test]
fn fifo_and_engine_decisions_are_observed_before_any_delivery() {
    let chan = include_str!("../src/chan.rs");
    for (start, end) in [
        ("pub fn nsi_fifo_edge(", "pub fn nsi_engine_edge("),
        ("pub fn nsi_engine_edge(", "pub fn nsi_tick("),
    ] {
        let edge = body(chan, start, end);
        let note = edge
            .find("self.nsi_trace.note(")
            .expect("unobserved host edge");
        assert!(edge[..note].contains("#[cfg(feature = \"diag-nsi\")]"));
        assert!(note < edge.find("deliver(v)").unwrap());
        assert!(edge[..note].contains("let armed ="));
        assert!(edge[..note].contains("let verdict ="));
    }
}

#[test]
fn paced_trailing_raise_has_its_own_observation() {
    let chan = include_str!("../src/chan.rs");
    let tick = body(chan, "pub fn nsi_tick(", "pub fn pt_summary(");
    assert!(
        tick.contains("self.nsi_trace.note("),
        "trailing raise is unobservable"
    );
    assert!(tick.contains("Source::Trailing"));
    assert!(tick.find("self.nsi_trace.note(").unwrap() < tick.find("deliver(v)").unwrap());
}

#[test]
fn no_recorder_hook_is_added_to_the_vcpu_interrupt_or_mmio_paths() {
    let device = include_str!("../src/device.rs");
    for (start, end) in [
        ("fn bar0_write_inner(", "pub fn bar1_usermode_write("),
        ("fn deliver(", "pub fn probe_loop("),
        ("pub fn latch_and_deliver(", "pub fn irq_fd("),
    ] {
        assert!(!body(device, start, end).contains("nsi_trace"));
    }
}

#[test]
fn live_counter_requests_run_only_on_the_existing_drainer_heartbeat() {
    let device = include_str!("../src/device.rs");
    let drainer = body(device, "pub fn drainer_loop(", "pub fn enable_doorbell_fast_path(");
    assert!(drainer.contains("nsi_trace.poll_stats("), "no live counter observation");
    assert!(drainer.contains("self.chans.relay_stats.summary()"));
    assert!(drainer.contains("self.chans.pt_summary()"));
}

#[test]
fn a_counter_request_is_idempotent_and_independent_of_trace_freezing() {
    let trace = include_str!("../src/nsitrace.rs");
    let method = trace.split_once("pub fn poll_stats(").expect("missing request method").1;
    assert!(method.contains("self.stats_seen"));
    assert!(!method.contains("self.finish()"), "stats must leave tracing active");
    assert!(method.contains("report("));
}
