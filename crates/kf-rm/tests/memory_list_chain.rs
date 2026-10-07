//! Real served-chain opt-in, tested in isolated process environments.
#[path = "support/ga106.rs"]
mod ga106;
use kf_arch::{
    ClientKind,
    ids::{ClassId, HClient, HObject},
};
use kf_gsp::{RpcCommand, RpcFunction};
use kf_rm::{
    memory_list::{GuestRamAuthority, RamSpan},
    rmgraph::{AllocFacts, RmEvent},
};
#[derive(Debug)]
struct Ram;
impl GuestRamAuthority for Ram {
    fn validate(&self, s: RamSpan) -> Option<u64> {
        (s.base == 0x2000 && s.length == 0x7000).then_some(1)
    }
    fn read(&self, _: RamSpan, _: u64, _: u64, _: &mut [u8]) -> bool {
        panic!("no registration read")
    }
    fn write(&self, _: RamSpan, _: u64, _: u64, _: &[u8]) -> bool {
        panic!("no registration write")
    }
}
#[test]
fn memory_list_full_chain_independent_optin() {
    for enabled in ["0", "1"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "memory_list_child", "--nocapture"])
            .env("KF_MEMORY_LIST_CHAIN_TEST", enabled)
            .env("KF3_MEMORY_LIST_PROBE", enabled)
            .env_remove("KF3_SW_RUNLIST_PROBE")
            .env_remove("KF3_GFX_POOL_PROBE")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
#[test]
fn memory_list_child() {
    let Ok(enabled) = std::env::var("KF_MEMORY_LIST_CHAIN_TEST") else {
        return;
    };
    let driver =
        *kf_abi::versions::table_for(kf_abi::DriverVersion::parse("580.65.06").unwrap()).unwrap();
    let mut objects = kf_rm::rmrpc::GraphObjects::new(kf_chip::Family::Ampere)
        .with_guest_ram(std::sync::Arc::new(Ram));
    for (parent, handle, class, facts) in [
        (
            1,
            1,
            0x41,
            AllocFacts {
                client_kind: Some(ClientKind::User { pid: 123 }),
                ..Default::default()
            },
        ),
        (
            1,
            2,
            0x80,
            AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        ),
        (2, 3, 0x2080, AllocFacts::default()),
    ] {
        objects
            .graph
            .apply(RmEvent::Alloc {
                client: HClient(1),
                parent: HObject(parent),
                handle: HObject(handle),
                class: ClassId(class),
                facts,
            })
            .unwrap();
    }
    let mut policy = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver,
        Default::default(),
        Default::default(),
        kf_rm::ObjectLinks {
            objects: Some(kf_rm::rmrpc::ObjectPolicy::over(
                &driver,
                kf_abi::GuestOs::Linux,
                Box::new(objects),
                Default::default(),
            )),
            channels: Some(std::sync::Arc::new(|s| {
                panic!("unexpected channel action {s:?}")
            })),
            ..Default::default()
        },
    );
    let command = |function, payload| RpcCommand {
        function,
        code: 0,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    };
    let mut payload = vec![0; 64];
    for (off, v) in [
        (0, 1u32),
        (4, 2),
        (8, 4),
        (12, 0x81),
        (16, 0x48002000),
        (40, 1),
        (48, 0x10000),
    ] {
        payload[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    payload[32..40].copy_from_slice(&0x7000u64.to_le_bytes());
    payload[56..64].copy_from_slice(&2u64.to_le_bytes());
    let cmd = command(RpcFunction::Other(4), payload.clone());
    let reply = policy.respond(&cmd);
    assert_eq!(
        reply.as_ref().is_some_and(|r| r.rpc_result == 0),
        enabled == "1"
    );
    if enabled == "1" {
        assert_eq!(reply.unwrap().body, payload);
    }
    let mut short = cmd.clone();
    short.delivered = short.payload.clone();
    short.payload.pop();
    assert!(
        policy.respond(&short).is_none_or(|r| r.rpc_result != 0),
        "must never parse delivered tail"
    );
    let mut ordinary = vec![0; 32];
    for (off, v) in [(0, 1u32), (4, 2), (8, 5), (12, 0x81)] {
        ordinary[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    assert!(
        policy
            .respond(&command(RpcFunction::RmAlloc, ordinary))
            .is_none_or(|r| r.rpc_result != 0),
        "no fn103 widening"
    );
    for (control, bytes) in [(0x20801110u32, 8u32), (0x20801111, 40)] {
        let w = driver.rm_control_wire();
        let mut payload = vec![0; w.params_off + bytes as usize];
        for (off, v) in [(0, 1), (4, 3), (8, control), (w.params_size_off, bytes)] {
            payload[off..off + 4].copy_from_slice(&v.to_le_bytes());
        }
        assert!(
            policy
                .respond(&command(RpcFunction::RmControl, payload))
                .is_none_or(|r| r.rpc_result != 0)
        );
    }
}
