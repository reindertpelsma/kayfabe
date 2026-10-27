//! ENS6 composition guards: the new counters and the layout slice reach the real hooks.
const DEVICE: &str = include_str!("../src/device.rs");
const MEM: &str = include_str!("../src/mem.rs");
const HOST: &str = include_str!("../../kf-host/src/lib.rs");
const GATE: &str = include_str!("../../kf-host/src/channel/sparse.rs");
fn compact(s: &str) -> String {
    s.split_whitespace().collect()
}

#[test]
fn heartbeat_prints_every_rm_verb_class() {
    assert!(DEVICE.contains("kf_host::diag::verb_totals()"));
    for name in [
        " rmverbs[",
        "reserve=",
        "alloc_other=",
        "free=",
        "map=",
        "unmap=",
        "control=",
        "invalidate=",
    ] {
        assert!(DEVICE.contains(name), "{name}");
    }
}

#[test]
fn heartbeat_prints_the_fallback_and_slice_counters() {
    for name in [
        "rm_declined={}",
        "backoff_skips={}",
        "slice_deferred={}",
        "speculative_skipped={}",
        "window_hits={}",
        "layout_refresh_peak_us={}",
        "layout_refresh_peak_verbs={}",
    ] {
        assert!(DEVICE.contains(name), "{name}");
    }
}

#[test]
fn every_timed_host_ioctl_names_its_verb() {
    assert!(
        !HOST.contains("diag::rm(false, ||"),
        "an unclassified RM verb remains"
    );
    for (name, verb) in [
        ("fn raw_alloc_exact_layout(", "diag::alloc_verb(class)"),
        ("fn raw_alloc_via_exact(", "diag::alloc_verb(class)"),
        ("pub fn raw_alloc_nested(", "diag::alloc_verb(class)"),
        ("fn raw_control_status(", "diag::control_verb(cmd)"),
        ("fn raw_map_dma_slice(", "diag::Verb::Map"),
        ("pub fn raw_unmap_dma_range(", "diag::Verb::Unmap"),
        ("pub fn free(", "diag::Verb::Free"),
        ("pub fn arm_cpu_view_on(", "diag::Verb::Other"),
        ("pub fn release_cpu_view(", "diag::Verb::Other"),
    ] {
        let body = HOST.split(name).nth(1).unwrap_or_else(|| panic!("{name}"));
        let body = body
            .split("diag::rm_verb(")
            .nth(1)
            .unwrap_or_else(|| panic!("{name}"));
        assert!(body.trim_start().starts_with(verb), "{name}: {verb}");
    }
}

#[test]
fn a_new_refresh_renews_the_layout_slice_before_the_first_retry() {
    let hook = MEM
        .split("fn begin_refresh(")
        .nth(1)
        .unwrap()
        .split("self.bv.begin_refresh();")
        .nth(1)
        .unwrap();
    let renew = hook.find("begin_layout_slice(").expect("slice renewed");
    let retry = hook.find("retry_sparse(").expect("retry");
    assert!(renew < retry);
}

#[test]
fn sparse_callbacks_run_on_the_sliced_ops() {
    let gate = compact(GATE);
    assert!(
        gate.contains("slice::Sliced{"),
        "the tree's ops charge the slice"
    );
    assert!(GATE.contains("fn begin_layout_slice("));
    assert!(GATE.contains("self.slice.renew()"));
}
