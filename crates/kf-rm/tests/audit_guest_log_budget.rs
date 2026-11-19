//! SA-07. FALSIFIER: ordinary guest calls can produce stderr above the shared
//! per-second budget; both floods run through the real policy entry points.
#[path = "support/ga106.rs"]
mod ga106;
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use std::process::Command;
use std::time::Instant;

fn rpc(function: RpcFunction, payload: Vec<u8>) -> RpcCommand {
    RpcCommand {
        function,
        code: 103,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}
fn flood(kind: &str) {
    let abi = *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).unwrap();
    if kind == "gss" {
        let wire = abi.rm_control_wire();
        let mut policy =
            kf_rm::inittables::InitTablePolicy::new(ga106::board(), ga106::host(), abi);
        for i in 0..100_000u32 {
            let cmd = 0xa000_8000 | ((i >> 15) << 16) | (i & 0x7fff);
            let mut payload = vec![0; wire.params_off];
            payload[0..4].copy_from_slice(&0xc1d0_0100u32.to_le_bytes());
            payload[4..8].copy_from_slice(&0x2080u32.to_le_bytes());
            payload[8..12].copy_from_slice(&cmd.to_le_bytes());
            assert!(
                policy
                    .respond(&rpc(RpcFunction::RmControl, payload))
                    .is_none()
            );
        }
    } else {
        use kf_rm::chanlink::{ChanAnswer, ChannelPolicy};
        let mut policy = ChannelPolicy::new(
            abi,
            kf_abi::GuestOs::Linux,
            std::sync::Arc::new(|_| ChanAnswer::Done),
        );
        for i in 0..100_000u32 {
            let client = 0xc1d0_0000 + i;
            let mut payload: Vec<u8> = [client, 0, 0, 0, 0, 8, 0, 0, client, 1234]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect();
            // Explicitly retain the caller's PID as a userspace declaration.
            payload[36..40].copy_from_slice(&1234u32.to_le_bytes());
            assert!(
                policy
                    .respond(&rpc(RpcFunction::RmAlloc, payload))
                    .is_none()
            );
        }
    }
}
fn bounded(name: &str, kind: &str) {
    if std::env::var("KF_F1_LOG_CHILD").ok().as_deref() == Some(kind) {
        flood(kind);
        return;
    }
    let start = Instant::now();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("KF_F1_LOG_CHILD", kind)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "policy flood failed: {}",
        String::from_utf8_lossy(&child.stderr)
    );
    let lines = child.stderr.iter().filter(|&&b| b == b'\n').count() as u64;
    // At most 32 payloads and 32 summaries per second, including edge buckets.
    let bound = 64 * (start.elapsed().as_secs() + 2);
    eprintln!("SA07_FLOOD kind={kind} lines={lines} bound={bound}");
    assert!(
        lines <= bound,
        "{kind}: {lines} lines for 100000 guest calls, bound {bound}"
    );
}
#[test]
fn unknown_gss_controls_have_bounded_stderr() {
    bounded("unknown_gss_controls_have_bounded_stderr", "gss");
}
#[test]
fn guest_chosen_root_clients_have_bounded_stderr() {
    bounded("guest_chosen_root_clients_have_bounded_stderr", "root");
}
