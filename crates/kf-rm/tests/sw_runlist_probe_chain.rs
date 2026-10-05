//! The real served chain, with opt-in isolated in child processes (no unsafe set_var).
#[path = "support/ga106.rs"]
mod ga106;

use kf_arch::{
    ClientKind,
    ids::{ClassId, HClient, HObject},
};
use kf_gsp::{RpcCommand, RpcFunction};
use kf_rm::rmgraph::{AllocFacts, RmEvent};

#[test]
fn full_chain_in_separate_environment() {
    for enabled in ["0", "1"] {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child_case", "--nocapture"])
            .env("KF_RUNLIST_CHAIN_TEST", enabled)
            .env("KF3_SW_RUNLIST_PROBE", enabled)
            .env_remove("KF3_GFX_POOL_PROBE")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "child {enabled}: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn child_case() {
    let Ok(enabled) = std::env::var("KF_RUNLIST_CHAIN_TEST") else {
        return;
    };
    let driver = *kf_abi::versions::table_for(kf_abi::DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    })
    .unwrap();
    let mut objects = kf_rm::rmrpc::GraphObjects::new(kf_chip::Family::Ampere);
    for (parent, handle, class, facts) in [
        (
            1,
            1,
            0x41,
            AllocFacts {
                client_kind: Some(ClientKind::User { pid: 100 }),
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
    let links = kf_rm::ObjectLinks {
        objects: Some(kf_rm::rmrpc::ObjectPolicy::over(
            &driver,
            kf_abi::GuestOs::Linux,
            Box::new(objects),
            Default::default(),
        )),
        // Any channel action from this metadata/control test is a hard failure.
        channels: Some(std::sync::Arc::new(|statement| {
            panic!("unexpected channel action: {statement:?}")
        })),
        ..Default::default()
    };
    let mut policy = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver,
        Default::default(),
        Default::default(),
        links,
    );
    let command = |function, payload| RpcCommand {
        function,
        code: 0,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    };
    use kf_abi::guestsysinfo::*;
    let twin = kf_abi::generated::windows_twins::WINDOWS_TWINS
        .iter()
        .find(|t| t.win_name == "580.88")
        .unwrap();
    let mut identity = vec![0; SET_GUEST_SYSTEM_INFO_SIZE];
    let vgx = driver.vgx_version().unwrap();
    for (off, value) in [
        (VGX_MAJOR_OFF, vgx.major),
        (VGX_MINOR_OFF, vgx.minor),
        (GUEST_CL_NUM_OFF, twin.win_cl),
    ] {
        identity[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (off, value) in [
        (GUEST_DRIVER_VERSION_OFF, twin.win_name),
        (GUEST_VERSION_OFF, twin.win_branch),
    ] {
        identity[off..off + value.len()].copy_from_slice(value.as_bytes());
    }
    assert_eq!(
        policy
            .respond(&command(RpcFunction::SetGuestSystemInfo, identity))
            .unwrap()
            .rpc_result,
        0
    );
    let mut alloc = vec![0; 44];
    for (off, value) in [(0, 1u32), (4, 3), (8, 4), (12, 0xb297), (20, 12), (32, 1)] {
        alloc[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }
    let reply = policy
        .respond(&command(RpcFunction::RmAlloc, alloc))
        .unwrap();
    assert_eq!(reply.rpc_result == 0, enabled == "1");
    for (control, bytes) in [(0x20801110u32, 8u32), (0x20801111, 40)] {
        let wire = driver.rm_control_wire();
        let mut payload = vec![0; wire.params_off + bytes as usize];
        for (off, value) in [(0, 1), (4, 3), (8, control), (wire.params_size_off, bytes)] {
            payload[off..off + 4].copy_from_slice(&value.to_le_bytes());
        }
        payload[wire.params_off..wire.params_off + 4].copy_from_slice(&4u32.to_le_bytes());
        // A declined complete chain is NV_ERR_NOT_SUPPORTED at GspFsm::answer
        // (kf-gsp/src/boot.rs); no link may claim these with NV_OK.
        assert!(
            policy
                .respond(&command(RpcFunction::RmControl, payload))
                .is_none_or(|reply| reply.rpc_result != 0),
            "control {control:#x} must still refuse"
        );
    }
}
