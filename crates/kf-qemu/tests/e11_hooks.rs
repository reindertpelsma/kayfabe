//! E11 hooks stay on the VA path and precede Rust child creation.
const DEVICE: &str = include_str!("../src/device.rs");
const VAS: &str = include_str!("../../kf-mem/src/vasmgr.rs");
#[test]
fn e11_installs_before_realization_and_unmasks_at_va_entry() {
    let realize = DEVICE.split("pub fn realize(cfg:").nth(1).unwrap();
    assert!(realize.find("refresh_stack_install()").unwrap() < realize.find("cfg.check()").unwrap());
    let va = DEVICE.split("pub fn va_loop(&self)").nth(1).unwrap();
    assert!(va.find("refresh_stack_va()").unwrap() < va.find("if self.latency_active").unwrap());
}
#[test]
fn e11_async_batch_owns_timer_until_apply_finishes_and_before_next_pump() {
    assert!(VAS.contains("debug_stack: Option<kf_host::diag::RefreshStack>"));
    let submit = VAS.split("fn submit_batch(").nth(1).unwrap();
    assert!(submit.find("refresh_stack_arm()").unwrap() < submit.find("self.walker.submit").unwrap());
    let ready = VAS.split("pub fn on_walk_ready(").nth(1).unwrap();
    assert!(ready.contains("batch.debug_stack.take()"));
}
