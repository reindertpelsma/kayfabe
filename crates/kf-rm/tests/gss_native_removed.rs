//! Security review 1, F-06 / X-01: the removed `KF3_GSS_NATIVE` experiment carried a guest's
//! GSS-legacy control parameters UNCHANGED to the host RM (a guest-authored host action, owner
//! principle A.2). The experiment and its compile shell (`kf_rm::gssnative`) are deleted
//! (`docs/design/V3_GSS_NATIVE.md`, SUPERSEDED).
//!
//! What this file pins: every control the experiment used to forward is left unanswered by the
//! served chain (the FSM posts `NV_ERR_NOT_SUPPORTED`) and no reply is held on a host act. That
//! no product crate can send guest bytes through the opaque host control is pinned by
//! `crates/kf-abi/tests/control_partition.rs` (`harness_only_methods_have_no_product_caller`).

#[path = "support/ga106.rs"]
mod ga106;
#[path = "support/rpcwire.rs"]
mod rpcwire;

use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};

const PARAMS_AT: usize = 40;
const NV_OK: u32 = 0;
/// The removed experiment's parameter cap (an edge of its old rule).
const MAX_PARAMS: usize = 64 * 1024;
const CLIENT: u32 = 0xc1d0_0002;
const DEVICE: u32 = 0xff01_0000;
const SUBDEVICE: u32 = 0xff03_0000;

fn driver() -> kf_abi::versions::DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

fn rpc(function: RpcFunction, code: u32, payload: Vec<u8>) -> RpcCommand {
    RpcCommand {
        function,
        code,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn control(object: u32, cmd: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&CLIENT.to_le_bytes());
    payload[4..8].copy_from_slice(&object.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    rpc(RpcFunction::RmControl, 0x4c, payload)
}

fn alloc(parent: u32, handle: u32, class: u32, params: &[u8]) -> RpcCommand {
    let mut payload: Vec<u8> = [CLIENT, parent, handle, class, 0, params.len() as u32, 0, 0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    payload.extend_from_slice(params);
    rpc(RpcFunction::RmAlloc, 0x67, payload)
}

/// The served chain with the object seat, a client, a device and a subdevice.
fn rig() -> Box<dyn CommandPolicy> {
    let objects = kf_rm::rmrpc::ObjectPolicy::over(
        &driver(),
        kf_abi::GuestOs::Linux,
        Box::new(kf_rm::rmrpc::GraphObjects::new(kf_chip::Family::Ampere)),
        Default::default(),
    );
    let mut chain = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver(),
        kf_rm::ChainLogs::default(),
        kf_rm::census::ControlCensusLog::new(),
        kf_rm::ObjectLinks {
            objects: Some(objects),
            memory: None,
            channels: None,
            display: None,
            console: None,
            windows: Default::default(),
            utilization: None,
        },
    );
    let root = rpc(
        RpcFunction::RmAlloc,
        0x67,
        rpcwire::client_root_alloc_body(0x41, CLIENT, 1234),
    );
    assert_eq!(chain.respond(&root).expect("client").rpc_result, NV_OK);
    let dev = rpcwire::device_params(0, 0, 0);
    assert_eq!(
        chain
            .respond(&alloc(CLIENT, DEVICE, 0x80, &dev))
            .expect("device")
            .rpc_result,
        NV_OK
    );
    assert_eq!(
        chain
            .respond(&alloc(DEVICE, SUBDEVICE, 0x2080, &[0; 4]))
            .expect("subdevice")
            .rpc_result,
        NV_OK
    );
    chain
}

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

/// Every id the experiment forwarded (`docs/design/V3_REFUSAL_AUDIT.md`), plus the edges of its old
/// rule (zero-length, exactly the 64 KiB cap).
fn formerly_forwarded() -> Vec<(String, RpcCommand)> {
    let mut v: Vec<(String, RpcCommand)> = [
        (0x2080_9004u32, 1544usize),
        (0x2080_b201, 2188),
        (0x2080_852e, 528),
        (0x2080_852f, 776),
        (0x2080_a0d1, 2024),
        (0x2080_a0a8, 64),
        (0x2080_9037, 64),
        (0x2080_8539, 64),
        (0x2080_9038, 64),
        (0x2080_a080, 64),
        (0x2080_a097, 64),
        (0x2080_8163, 4),
        (0x2080_a084, 0),
        (0x2080_a0d1, MAX_PARAMS),
    ]
    .into_iter()
    .map(|(cmd, n)| {
        (
            format!("{cmd:#010x}/{n}"),
            control(SUBDEVICE, cmd, &pattern(n, 3)),
        )
    })
    .collect();
    v.push((
        "a GSS-legacy id on a non-subdevice object".into(),
        control(DEVICE, 0x2080_9004, &pattern(16, 1)),
    ));
    v
}

#[test]
fn every_formerly_forwarded_control_is_left_unanswered() {
    for (name, c) in formerly_forwarded() {
        let mut chain = rig();
        assert_eq!(
            chain.respond(&c),
            None,
            "{name}: unanswered, so the FSM refuses it (0x56)"
        );
        assert!(
            chain.defers(&c).is_none(),
            "{name}: no reply may be held on a host act"
        );
    }
}
