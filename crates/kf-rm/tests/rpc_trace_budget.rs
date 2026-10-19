//! Exercise the production diagnostic in isolated processes: no unsafe env mutation.
use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};
use kf_rm::census::{ControlCensus, ControlCensusLog};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
const LIMIT: usize = 16_384;

#[derive(Debug)]
struct FixedReply(Arc<AtomicUsize>);
impl CommandPolicy for FixedReply {
    fn respond(&mut self, _: &RpcCommand) -> Option<Reply> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Some(Reply::refused(0x56))
    }
}

#[test]
fn child_metadata_trace() {
    if std::env::var_os("KF_TEST_RPC_TRACE_BUDGET_CHILD").is_none() {
        return;
    }
    let driver = *table_for(BENCH_DRIVER).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let log = ControlCensusLog::new();
    let mut policy = ControlCensus::new(driver, log.clone(), FixedReply(calls.clone()));
    for i in 0..(2 * LIMIT + 1) {
        // A fresh wrapper models guest-triggered ABI re-selection/reset attempts.
        if i % 1024 == 0 {
            policy = ControlCensus::new(driver, log.clone(), FixedReply(calls.clone()));
        }
        let cmd = RpcCommand {
            function: RpcFunction::Other(4),
            code: 4,
            sequence: i as u32,
            payload: vec![],
            elements: 1,
            delivered: vec![],
        };
        assert_eq!(policy.respond(&cmd), Some(Reply::refused(0x56)));
    }
    assert_eq!(calls.load(Ordering::Relaxed), 2 * LIMIT + 1);
    println!("INNER_CALLS={} ALL_REPLIES_UNCHANGED=1", 2 * LIMIT + 1);
}

#[test]
fn actual_metadata_output_is_bounded_across_policy_recreation() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_metadata_trace", "--nocapture"])
        .env("KF_TEST_RPC_TRACE_BUDGET_CHILD", "1")
        .env("KF3_RPC_TRACE", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        text.lines()
            .filter(|l| l.starts_with("kf-rm: rpc-trace fn="))
            .count(),
        LIMIT
    );
    assert_eq!(text.matches("rpc-trace lifetime limit reached").count(), 1);
    assert!(String::from_utf8_lossy(&out.stdout).contains("ALL_REPLIES_UNCHANGED=1"));
}

#[test]
fn metadata_trace_remains_off_by_default() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_metadata_trace", "--nocapture"])
        .env("KF_TEST_RPC_TRACE_BUDGET_CHILD", "1")
        .env_remove("KF3_RPC_TRACE")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("rpc-trace"));
}
