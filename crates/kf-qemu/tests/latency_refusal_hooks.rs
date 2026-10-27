//! Preserve mandatory sparse refusal checks while removing optional retry work.
const MEM: &str = include_str!("../src/mem.rs");
const HOST: &str = include_str!("../../kf-host/src/channel/sparse.rs");
#[test]
fn split_critical_empty_diff_still_checks_sparse_readiness_refusals() {
    let p = MEM
        .split("fn end_refresh(&self)")
        .nth(1)
        .unwrap()
        .split("fn invalidate")
        .next()
        .unwrap();
    assert!(p.contains("sparse_refresh_ready"));
    assert!(p.contains("map_err"));
    assert!(!p.contains("return Ok(())"));
}
#[test]
fn sparse_readiness_keeps_the_existing_poison_and_lock_guard_without_retry() {
    let p = HOST.split("pub fn sparse_refresh_ready(").nth(1);
    assert!(p.is_some(), "same guard as ordinary sparse bookkeeping");
    let p = p.unwrap().split("pub fn reset_sparse").next().unwrap();
    assert!(p.contains("self.with_sparse(space.space"));
    assert!(!p.contains("tree.retry("));
}
