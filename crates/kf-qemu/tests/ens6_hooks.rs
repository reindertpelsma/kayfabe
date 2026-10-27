//! ENS6 composition guards: the new counters and the layout slice reach the real hooks.
const DEVICE: &str = include_str!("../src/device.rs");
const MEM: &str = include_str!("../src/mem.rs");
const HOST: &str = include_str!("../../kf-host/src/lib.rs");
const GATE: &str = include_str!("../../kf-host/src/channel/sparse.rs");
const DIAG: &str = include_str!("../../kf-host/src/diag.rs");
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
    assert!(
        gate.contains("slice:&self.slice,"),
        "the host session's own slice"
    );
    assert!(GATE.contains("fn begin_layout_slice("));
    assert!(GATE.contains("self.slice.renew()"));
}

#[test]
fn the_heartbeat_reads_the_busiest_refresh_from_the_host_slice() {
    assert!(DEVICE.contains("self.rm.layout_slice_peak()"));
    assert!(GATE.contains("self.slice.peak()"));
}

#[test]
fn the_sliced_ops_are_built_on_the_host_session_slice_and_renewed_by_it() {
    assert!(GATE.contains("slice: &self.slice,"));
    let body = GATE
        .split("fn begin_layout_slice(")
        .nth(1)
        .unwrap()
        .split("pub fn layout_slice_peak(")
        .next()
        .unwrap();
    assert!(body.contains("self.slice.renew()"));
}

#[test]
fn the_slice_is_renewed_once_per_refresh_by_the_gpu_mirror() {
    assert_eq!(MEM.matches("begin_layout_slice()").count(), 1);
}

#[test]
fn heartbeat_verb_indices_follow_the_enum_declaration_order() {
    let names = [
        "ReserveVa",
        "AllocOther",
        "Free",
        "Map",
        "Unmap",
        "Control",
        "Invalidate",
        "Other",
    ];
    let enum_body = DIAG
        .split("pub enum Verb {")
        .nth(1)
        .unwrap()
        .split("\n}")
        .next()
        .unwrap();
    let mut last = 0;
    for n in names {
        let at = enum_body.find(&format!("\n    {n},")).expect(n);
        assert!(at > last, "{n} out of order");
        last = at;
    }
    assert!(compact(DEVICE).contains("(f(0),f(1),f(2),f(3),f(4),f(5),f(6),f(7))"));
    assert!(DEVICE.contains("let f = |i: usize|"));
    assert!(DEVICE.contains("let v = kf_host::diag::verb_totals();"));
}

#[test]
fn the_first_rm_reservation_refusals_are_logged_with_their_arguments() {
    let body = GATE
        .split("fn reserve(&self, lo: u64, len: u64, sparse: bool)")
        .nth(1)
        .unwrap()
        .split("fn release(")
        .next()
        .unwrap();
    assert!(body.contains("RM refused reserve"));
    for arg in ["space", "lo", "len", "sparse"] {
        assert!(body.contains(arg), "{arg}");
    }
    assert!(body.contains("REFUSAL_LOG_CAP"), "bounded");
}
