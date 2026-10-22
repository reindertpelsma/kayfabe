//! SA-04: FALSIFIER: freeing leaves or the last aliases repeatedly scans unrelated
//! policy state. Child processes discard allocation diagnostics before timing frees.
use kf_abi::generated::classes as nv;
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::barpde::{MemStatement, PageDirPolicy};
use kf_rm::chanlink::{ChanAnswer, ChannelPolicy};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

fn rpc(function: RpcFunction, words: &[u32]) -> RpcCommand {
    RpcCommand {
        function,
        code: 103,
        sequence: 1,
        payload: words.iter().flat_map(|x| x.to_le_bytes()).collect(),
        elements: 1,
        delivered: Vec::new(),
    }
}
fn workload(kind: &str, n: u32) -> (Duration, usize) {
    let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).unwrap();
    let retired = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&retired);
    let mut policy: Box<dyn CommandPolicy> = if kind == "channel" {
        Box::new(ChannelPolicy::new(
            abi,
            kf_abi::GuestOs::Linux,
            Arc::new(move |_| {
                seen.fetch_add(1, Ordering::Relaxed);
                ChanAnswer::Done
            }),
        ))
    } else {
        Box::new(PageDirPolicy::new(
            abi,
            kf_abi::GuestOs::Linux,
            Arc::new(move |st| {
                if matches!(st, MemStatement::Retire { .. }) {
                    seen.fetch_add(1, Ordering::Relaxed);
                }
            }),
        ))
    };
    let a = 0xc1d0_0100;
    let b = 0xc1d0_0200;
    let handle = |i| 0x1000_0000 + i;
    for i in 0..n {
        assert!(
            policy
                .respond(&rpc(
                    RpcFunction::RmAlloc,
                    &[a, 0x10, handle(i), nv::FERMI_VASPACE_A, 0, 8, 0, 0, 0, 0]
                ))
                .is_none()
        );
        if kind == "aliases" {
            assert!(
                policy
                    .respond(&rpc(
                        RpcFunction::DupObject,
                        &[b, 0x20, handle(i), a, handle(i), 0, 0]
                    ))
                    .is_none()
            );
        }
    }
    let start = Instant::now();
    for i in 0..n {
        assert!(
            policy
                .respond(&rpc(RpcFunction::Free, &[a, 0, handle(i), 0]))
                .is_none()
        );
    }
    if kind == "aliases" {
        assert_eq!(
            retired.load(Ordering::Relaxed),
            0,
            "a dup keeps its original alive"
        );
        for i in 0..n {
            assert!(
                policy
                    .respond(&rpc(RpcFunction::Free, &[b, 0, handle(i), 0]))
                    .is_none()
            );
        }
    }
    let elapsed = start.elapsed();
    (elapsed, retired.load(Ordering::Relaxed))
}
fn bounded(name: &str, kind: &str, n: u32) {
    if std::env::var("KF_F1_FREE_CHILD").ok().as_deref() == Some(name) {
        let (elapsed, retired) = workload(kind, n);
        assert_eq!(retired, n as usize);
        println!("SA04_PERF {}", elapsed.as_nanos());
        return;
    }
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("KF_F1_FREE_CHILD", name)
        .stderr(Stdio::null())
        .output()
        .unwrap();
    let output = String::from_utf8(child.stdout).unwrap();
    assert!(child.status.success(), "policy workload failed: {output}");
    let nanos: u128 = output
        .lines()
        .find_map(|s| s.strip_prefix("SA04_PERF "))
        .expect("measured frees")
        .parse()
        .unwrap();
    eprintln!("SA04_POLICY kind={kind} n={n} free_ns={nanos}");
    assert!(
        nanos < Duration::from_millis(500).as_nanos(),
        "{kind} {n} frees took {nanos}ns; indexed work must fit 500ms"
    );
}
#[test]
fn page_directory_leaves_30000() {
    bounded("page_directory_leaves_30000", "page", 30_000);
}
#[test]
fn page_directory_leaves_45000() {
    bounded("page_directory_leaves_45000", "page", 45_000);
}
#[test]
fn channel_leaves_30000() {
    bounded("channel_leaves_30000", "channel", 30_000);
}
#[test]
fn channel_leaves_45000() {
    bounded("channel_leaves_45000", "channel", 45_000);
}
#[test]
fn orphan_aliases_1600() {
    bounded("orphan_aliases_1600", "aliases", 1600);
}
#[test]
fn orphan_aliases_2400() {
    bounded("orphan_aliases_2400", "aliases", 2400);
}
