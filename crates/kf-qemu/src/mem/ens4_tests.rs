//! The delay remains inside the real Inbox global-gate predicate; no GPU or wall sleep.
use super::Inbox;
use kf_mem::{applydelay::Config, latency_gate::Bound};
#[test]
fn every_large_apply_sleep_still_holds_the_actual_user_gate() {
    let inbox = Inbox::new().unwrap();
    inbox.set_walk_busy(true);
    let mut ns = 100_000u64;
    Config::parse(Some("500"), None).unwrap().run(4174, |ms| {
        assert!(inbox.walk_busy());
        ns += ms * 1_000_000;
    });
    assert!(
        inbox.walk_busy(),
        "the diagnostic must not release the production gate"
    );
    inbox.set_walk_busy(false);
    let bound = Bound::calibrate(&vec![100_000; 10000]).unwrap();
    assert!(!bound.passes(Some(ns)));
}
#[test]
fn a_pending_global_statement_remains_gated_during_injected_apply() {
    let inbox = Inbox::new().unwrap();
    let _ticket = inbox.request_split(7, None);
    let mut ms_slept = 0;
    Config::parse(Some("3000"), None).unwrap().run(4174, |ms| {
        assert!(inbox.walk_busy());
        ms_slept += ms;
    });
    assert_eq!(ms_slept, 3000);
    assert!(inbox.walk_busy());
    assert!(
        !Bound::calibrate(&vec![100_000; 10000])
            .unwrap()
            .passes(Some(ms_slept * 1_000_000))
    );
}
