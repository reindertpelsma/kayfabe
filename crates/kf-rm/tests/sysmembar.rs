//! `NV2080_CTRL_CMD_INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR` (`0x20800a70`) — ★★★ v3-refusals: SERVED as
//! the host GPU's own sysmembar, because its refusal was a flush the guest believed had happened
//! (`kf_rm::sysmembar` carries the measurement and the argument; `docs/design/
//! V3_REFUSAL_AUDIT.md`).
//!
//! What this file pins:
//! - with the memory plane seated, the control is answered `NV_OK` (envelope AND control header),
//!   carried to the plane as `MemStatement::Sysmembar`, HELD until the plane settles it (the host
//!   verb has returned), and never reaches the unserviced ledger;
//! - without the memory plane (register-only), nothing can perform it, so it is still refused —
//!   the answer is never an unbacked `NV_OK`;
//! - the link claims nothing else and holds nothing else.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::barpde::MemStatement;
use kf_rm::sysmembar::{NV2080_CTRL_CMD_INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR, SysmembarPolicy};

/// `RpcControlReq::HEADER`.
const PARAMS_AT: usize = 40;
/// Byte offset of `status` in the control header.
const CONTROL_STATUS_OFF: usize = 12;

fn driver() -> kf_abi::versions::DriverAbiTable {
    *table_for(BENCH_DRIVER).expect("the bench driver has a wire table")
}

/// A `GSP_RM_CONTROL` as `kbusSendSysmembarSingle_KERNEL` sends it: internal client/subdevice,
/// `NULL, 0` params (`ogkm-580: kern_bus.c:428-430`).
fn control(cmd: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&0xc1e0_0002u32.to_le_bytes());
    payload[4..8].copy_from_slice(&0x5c00_0003u32.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    RpcCommand { function: RpcFunction::RmControl, code: 0x4c, sequence: 57, payload, elements: 1, delivered: Vec::new() }
}

type Seen = std::sync::Arc<std::sync::Mutex<Vec<MemStatement>>>;

fn chain(memory: bool, log: &kf_rm::unserviced::UnservicedLog) -> (Box<dyn CommandPolicy>, Seen) {
    let seen: Seen = std::sync::Arc::default();
    let s = seen.clone();
    let links = kf_rm::ObjectLinks {
        objects: None,
        memory: memory.then(|| kf_rm::MemoryLink {
            sink: std::sync::Arc::new(move |st| s.lock().unwrap().push(st)),
            guest_os: kf_abi::GuestOs::Linux,
        }),
        channels: None,
    };
    let c = kf_rm::served_policy(
        ga106::board(),
        ga106::host(),
        driver(),
        kf_rm::ChainLogs { unserviced: log.clone(), ..Default::default() },
        kf_rm::census::ControlCensusLog::new(),
        links,
    );
    (c, seen)
}

#[test]
fn with_the_memory_plane_the_sysmembar_is_carried_held_and_answered_ok() {
    let log = kf_rm::unserviced::UnservicedLog::new();
    let (mut c, seen) = chain(true, &log);
    let cmd = control(NV2080_CTRL_CMD_INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR, &[]);
    let r = c.respond(&cmd).expect("answered");
    assert_eq!(r.rpc_result, 0, "the envelope says NV_OK");
    let st = u32::from_le_bytes(r.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4].try_into().unwrap());
    assert_eq!(st, 0, "the control header's own status says NV_OK");
    assert!(
        c.holds_for_refresh(&cmd),
        "the NV_OK must wait for the plane to settle it — i.e. for the host sysmembar to return"
    );
    assert_eq!(seen.lock().unwrap().as_slice(), &[MemStatement::Sysmembar], "carried to the plane, once");
    assert_eq!(log.total(), 0, "nothing reached the ledger");
}

#[test]
fn without_the_memory_plane_it_is_still_refused_never_an_unbacked_ok() {
    let log = kf_rm::unserviced::UnservicedLog::new();
    let (mut c, seen) = chain(false, &log);
    let cmd = control(NV2080_CTRL_CMD_INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR, &[]);
    let r = c.respond(&cmd);
    assert!(r.is_none_or(|r| r.rpc_result != 0), "nothing can perform it, so nothing may say it was done");
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(log.total(), 1, "the ledger names it");
}

#[test]
fn the_link_claims_only_its_control_and_holds_nothing_else() {
    let seen: Seen = std::sync::Arc::default();
    let s = seen.clone();
    let mut p = SysmembarPolicy::new(driver(), std::sync::Arc::new(move |st| s.lock().unwrap().push(st)));
    for other in [0x2080_0a6c_u32, 0x2080_1702, 0x0080_1813] {
        let cmd = control(other, &[0u8; 8]);
        assert!(p.respond(&cmd).is_none(), "{other:#x} is not the sysmembar");
        assert!(!p.holds_for_refresh(&cmd), "{other:#x} must hold nothing");
    }
    let alloc = RpcCommand { function: RpcFunction::RmAlloc, code: 0x67, sequence: 1, payload: vec![0; 64], elements: 1, delivered: Vec::new() };
    assert!(p.respond(&alloc).is_none());
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(p.carried, 0);
    let cmd = control(NV2080_CTRL_CMD_INTERNAL_BUS_FLUSH_WITH_SYSMEMBAR, &[]);
    assert_eq!(p.respond(&cmd).map(|r| r.rpc_result), Some(0));
    assert_eq!(p.carried, 1);
}
