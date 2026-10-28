//! Owner G has two disjoint scopes and one matched target-apply delta window.
const RAW: &str = include_str!("../../kf-linux-raw/src/mapping_unsafe.rs");
const SPARSE: &str = include_str!("../../kf-host/src/sparse.rs");
const VA: &str = include_str!("../../kf-mem/src/vasmgr.rs");
#[test]
fn g_whole_reservations_mark_includes_reindex() {
    let body = SPARSE
        .split("pub(crate) fn mark(")
        .nth(1)
        .unwrap()
        .split("    /// Turn queued")
        .next()
        .unwrap();
    assert!(body.find("Cost::Mark").unwrap() < body.find("let end").unwrap());
    assert!(body.contains("self.reindex(va, end)"));
}
#[test]
fn g_stitch_scope_includes_nested_mmaps_without_an_anywhere_counter() {
    let stitch = RAW.split("pub fn stitch(").nth(1).unwrap();
    assert!(stitch.find("Cost::Stitch").unwrap() < stitch.find("Mapping::anywhere").unwrap());
    let anywhere = RAW
        .split("fn anywhere(")
        .nth(1)
        .unwrap()
        .split("impl Drop for Mapping")
        .next()
        .unwrap();
    assert!(!anywhere.contains("Cost::"));
}
#[test]
fn g_delta_brackets_actual_apply_and_summary_has_matching_denominator() {
    let before = VA.find("let g_before =").unwrap();
    let apply = VA[before..].find("let a = apply_entry").unwrap() + before;
    let after = VA[apply..].find("let g_costs =").unwrap() + apply;
    assert!(before < apply && apply < after);
    assert!(VA.contains("target_apply_ns="));
    assert!(VA.contains("g_ns > 500_000_000"));
    assert!(VA.contains("mark_calls="));
    assert!(VA.contains("stitch_calls="));
}
