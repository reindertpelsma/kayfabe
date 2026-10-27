//! Source composition supplements the actual host-verb and memory-apply model tests.
const MEM: &str = include_str!("../src/mem.rs");
const APPLY: &str = include_str!("../../kf-mem/src/apply.rs");
const GATE: &str = include_str!("../../kf-host/src/channel/sparse.rs");
const HOST: &str = include_str!("../../kf-host/src/channel.rs");
fn compact(s: &str) -> String {
    s.split_whitespace().collect()
}
#[test]
fn actual_gpu_end_refresh_retries_accepted_sparse_intent() {
    let hook = MEM
        .split("fn end_refresh(")
        .nth(1)
        .unwrap()
        .split("fn invalidate(")
        .next()
        .unwrap();
    assert!(hook.contains("retry_sparse(self.bv.vas.space)"));
    assert!(hook.contains("map_err"));
}
#[test]
fn gpu_retry_is_wired_at_both_refresh_boundaries() {
    assert_eq!(MEM.matches("fn end_refresh(").count(), 2);
    assert_eq!(MEM.matches("retry_sparse(self.bv.vas.space)").count(), 2);
}
#[test]
fn wrapping_target_forwards_the_same_apply_hook() {
    let hook = MEM
        .split("fn end_refresh(")
        .nth(2)
        .unwrap()
        .split("fn begin_refresh(")
        .next()
        .unwrap();
    assert!(compact(hook).contains("Target::Gpu(g)=>g.end_refresh()"));
    assert!(hook.contains("Target::Window(w)"));
}
#[test]
fn every_target_variant_keeps_its_end_hook() {
    assert_eq!(MEM.matches("g.end_refresh()").count(), 1);
    assert_eq!(MEM.matches("w.end_refresh()").count(), 1);
    assert_eq!(MEM.matches("b.win.end_refresh()").count(), 1);
}
#[test]
fn sparse_reconsideration_precedes_the_final_invalidate() {
    let hook = APPLY.find("target.end_refresh()").unwrap();
    let invalidate = APPLY[hook..].find("match target.invalidate()").unwrap() + hook;
    assert!(hook < invalidate);
}
#[test]
fn authored_native_mapping_uses_the_tested_rollback_helper() {
    assert!(compact(HOST).contains("map_tree(self,tree,ops,space.range,MapPiece{"));
    assert!(HOST.contains("map_piece_list_checked(v, &pieces, row, rollback_flags)"));
}
#[test]
fn actual_sparse_callbacks_use_the_placement_exclusion_helper() {
    assert!(compact(GATE).contains("Some(with_placement(&self.placement_gate,||{"));
    assert!(GATE.contains("f(&mut tree, &SpaceOps { rm: self, space })"));
}
#[test]
fn placement_guard_surrounds_the_native_callback() {
    let start = GATE.find("Some(with_placement(").unwrap();
    let call = GATE.find("let result = f(").unwrap();
    assert!(start < call);
    assert!(GATE.contains("let _placement = gate.write()"));
}
