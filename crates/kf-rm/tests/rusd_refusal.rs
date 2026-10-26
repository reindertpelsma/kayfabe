//! CODE READ: BW9 protects the uniform refusal until a real RUSD publisher exists.
//! The GA106 fixture supplies unrelated host facts; die and family select the tested path.
#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::{DriverVersion, versions::table_for};
use kf_chip::DieId;
use kf_gsp::{RpcCommand, RpcFunction};
use kf_rm::{ChainLogs, ObjectLinks, census::ControlCensusLog};
use std::sync::Arc;

fn must_decline(die: &str, selector: u32, initialize: bool) {
    let driver = *table_for(DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    })
    .unwrap();
    let wire = driver.rm_control_wire();
    let mut host = ga106::host_facts();
    host.die = DieId::from_name(die).unwrap();
    host.family = if die.starts_with("GB") {
        kf_chip::Family::Blackwell
    } else {
        kf_chip::Family::Ampere
    };
    let logs = ChainLogs::default();
    let unserviced = logs.unserviced.clone();
    let mut policy = kf_rm::served_policy(
        ga106::board(),
        Arc::new(host),
        driver,
        logs,
        ControlCensusLog::new(),
        ObjectLinks::default(),
    );
    let mut payload = vec![0; wire.params_off + 16];
    payload[8..12].copy_from_slice(&selector.to_le_bytes());
    payload[wire.params_size_off..wire.params_size_off + 4].copy_from_slice(&16u32.to_le_bytes());
    if initialize {
        payload[wire.params_off] = 1;
        payload[wire.params_off + 8..].copy_from_slice(&0x4000u64.to_le_bytes());
    } else {
        payload[wire.params_off..wire.params_off + 8].copy_from_slice(&2u64.to_le_bytes());
    }
    let cmd = RpcCommand {
        function: RpcFunction::RmControl,
        code: 76,
        sequence: 28,
        payload,
        elements: 1,
        delivered: Vec::new(),
    };
    assert!(
        policy.respond(&cmd).is_none(),
        "{die}: success needs a real publisher"
    );
    assert_eq!(
        unserviced.total(),
        1,
        "decline must reach the terminal ledger"
    );
}

#[test]
fn gb202_init_has_no_success_without_publisher() {
    must_decline("GB202", 0x2080_0afe, true);
}
#[test]
fn gb205_init_has_no_success_without_publisher() {
    must_decline("GB205", 0x2080_0afe, true);
}
#[test]
fn ga104_init_has_no_success_without_publisher() {
    must_decline("GA104", 0x2080_0afe, true);
}
#[test]
fn gb202_poll_has_no_success_without_publisher() {
    must_decline("GB202", 0x2080_0aff, false);
}
#[test]
fn gb205_poll_has_no_success_without_publisher() {
    must_decline("GB205", 0x2080_0aff, false);
}
#[test]
fn ga104_poll_has_no_success_without_publisher() {
    must_decline("GA104", 0x2080_0aff, false);
}
