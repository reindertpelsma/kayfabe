//! The real served chain, one arm per child process (no unsafe set_var): the Windows profile's
//! software-runlist metadata is keyed on the declared identity.
#[path = "support/ga106.rs"]
mod ga106;
#[path = "support/windows_fn1.rs"]
mod support;

use kf_arch::{
    ClientKind,
    ids::{ClassId, HClient, HObject},
};
use kf_gsp::{RpcCommand, RpcFunction};
use kf_rm::rmgraph::{AllocFacts, RmEvent};

/// The software-runlist allocation
/// metadata is served to a guest that declared the measured Windows build with `windows-support` on, and
/// to NO other: a Linux declaration, no declaration, and `windows-support=off` with a Windows declaration
/// take the default path, exactly as with no probe installed. Each arm runs in a child process so the
/// diagnostic's stderr lines can be read.
#[test]
fn full_chain_in_separate_environment() {
    for (arm, served) in [
        ("windows", true),
        ("linux", false),
        ("undeclared", false),
        ("windows-support-off", false),
    ] {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child_case", "--nocapture"])
            .env("KF_RUNLIST_CHAIN_ARM", arm)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "child {arm}: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let log = String::from_utf8_lossy(&result.stderr);
        assert_eq!(
            log.contains("diagnostic control=0x20801111 params_bytes=40"),
            served,
            "{arm}"
        );
        assert_eq!(
            log.contains("ALLOC_MEMORY payload_bytes=56 prefix_only=true"),
            served,
            "{arm}"
        );
    }
}

#[test]
fn child_case() {
    let Ok(arm) = std::env::var("KF_RUNLIST_CHAIN_ARM") else {
        return;
    };
    let enabled = if arm == "windows" { "1" } else { "0" };
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
        windows: if arm == "windows-support-off" {
            kf_rm::winprofile::WindowsProfile::off()
        } else {
            kf_rm::winprofile::WindowsProfile::on()
        },
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
    // The guest's fn 1 for this arm: the Windows 580.88 build, the Linux build of the same wire, or
    // nothing.
    let declaration = match arm.as_str() {
        "windows" | "windows-support-off" => Some(support::windows(&driver)),
        "linux" => Some(support::linux(&driver)),
        _ => None,
    };
    if let Some(fn1) = declaration {
        assert_eq!(policy.respond(&fn1).unwrap().rpc_result, 0);
    }
    let mut alloc = vec![0; 44];
    for (off, value) in [(0, 1u32), (4, 3), (8, 4), (12, 0xb297), (20, 12), (32, 1)] {
        alloc[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }
    let reply = policy
        .respond(&command(RpcFunction::RmAlloc, alloc))
        .unwrap();
    assert_eq!(reply.rpc_result == 0, enabled == "1");
    assert!(
        policy
            .respond(&command(RpcFunction::Other(4), vec![0; 56]))
            .is_none_or(|reply| reply.rpc_result != 0)
    );
    for (control, bytes) in [(0x20801110u32, 8u32), (0x20801111, 40)] {
        let wire = driver.rm_control_wire();
        let mut payload = vec![0; wire.params_off + bytes as usize];
        for (off, value) in [(0, 1), (4, 3), (8, control), (wire.params_size_off, bytes)] {
            payload[off..off + 4].copy_from_slice(&value.to_le_bytes());
        }
        payload[wire.params_off..wire.params_off + 4].copy_from_slice(&4u32.to_le_bytes());
        // A declined complete chain is NV_ERR_NOT_SUPPORTED at GspFsm::answer
        // (kf-gsp/src/boot.rs); no link may claim these with NV_OK. ★ 2026-11-15: except the one the
        // Windows profile owns — the software-runlist submit `0x20801111` (host-owned scheduling, a
        // fabricated NV_OK with the request echoed), and only for a declared Windows guest.
        let answered = policy
            .respond(&command(RpcFunction::RmControl, payload))
            .is_some_and(|reply| reply.rpc_result == 0);
        assert_eq!(
            answered,
            arm == "windows" && control == 0x2080_1111,
            "{arm}: control {control:#x}"
        );
    }
}
