//! Composition evidence supplements the runtime policy/reservation tests; no GPU is required.
const DEVICE: &str = include_str!("../src/device.rs");
const CHANNEL: &str = include_str!("../../kf-host/src/channel.rs");
const GATE: &str = include_str!("../../kf-host/src/channel/sparse.rs");
fn compact(s: &str) -> String {
    s.split_whitespace().collect()
}
#[test]
fn framebuffer_budget_is_configured_before_host_fact_queries() {
    assert!(
        DEVICE.find("rm.configure_sparse_budget(").unwrap()
            < DEVICE.find("crate::rmfacts::host_facts(").unwrap()
    );
}
#[test]
fn configured_framebuffer_bytes_reach_the_immutable_policy() {
    assert!(
        compact(DEVICE).contains("rm.configure_sparse_budget(cfg.fb_mb.saturating_mul(1<<20))")
    );
    assert!(GATE.contains("self.sparse_budget.set(wanted)"));
}
#[test]
fn guest_tree_and_common_reserve_guard_use_the_same_policy() {
    assert!(compact(CHANNEL).contains("Reservations::new_guest(&vas.guest,"));
    assert!(CHANNEL.contains("self.sparse_budget().per_space"));
    assert!(CHANNEL.contains("self.sparse_budget().per_vm"));
    assert!(!CHANNEL.contains("const SESSION_BLOCK_LIMIT"));
}
#[test]
fn reservations_are_restricted_to_the_initial_guest_owned_roots() {
    assert!(compact(CHANNEL).contains("Reservations::new_guest(&vas.guest,"));
    assert!(!compact(CHANNEL).contains("Reservations::new(&vas.guest,"));
}
#[test]
fn heartbeat_publishes_live_budgets_and_unique_current_deficit() {
    for field in [
        "budget_space={}",
        "budget_vm={}",
        "current_unhonoured_bytes={}",
        "missing_capped={}",
        "withheld_windows={}",
    ] {
        assert!(DEVICE.contains(field), "{field}");
    }
    assert!(DEVICE.contains("s.current_unhonoured_bytes,"));
    assert!(DEVICE.contains("budget.per_space,"));
    assert!(DEVICE.contains("budget.per_vm,"));
}
#[test]
fn current_deficit_is_distinct_from_the_cumulative_admission_counter() {
    assert!(DEVICE.contains("s.current_unhonoured_bytes,"));
    assert!(GATE.contains("current_unhonoured_bytes: self.sparse_missing.load(Relaxed)"));
    assert!(GATE.contains("unhonoured_bytes: self.sparse_totals[4].load(Relaxed)"));
}
#[test]
fn native_completion_and_space_destruction_adjust_the_live_gauge() {
    assert!(GATE.contains("self.update_sparse_missing(before, after)"));
    assert!(CHANNEL.contains("self.update_sparse_missing(tree.counters(),"));
}
#[test]
fn warnings_are_default_on_bounded_and_include_space_identity() {
    for token in [
        "SPARSE_UNHONOURED space={space:#x}",
        "tree.missing_warning(clock())",
        "self.sparse_warnings.load(Relaxed) < 32",
        "(n < 32).then_some(n + 1)",
    ] {
        assert!(GATE.contains(token), "{token}");
    }
}

#[test]
fn warning_claim_is_atomic_and_capped_even_when_spaces_race() {
    let start = GATE.find("if self.sparse_warnings.load").unwrap();
    let end = GATE[start..].find("SPARSE_UNHONOURED").unwrap() + start;
    let admission = &GATE[start..end];
    assert!(admission.contains("try_update"));
    assert!(admission.contains("(n < 32).then_some(n + 1)"));
}
