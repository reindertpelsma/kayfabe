//! Reply-boundary tests for the explicitly enabled diagnostic link.
use kf_abi::{DriverVersion, versions::table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::cmc_log_probe::CmcLogProbe;

fn driver() -> kf_abi::versions::DriverAbiTable {
    *table_for(DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    })
    .unwrap()
}
fn request() -> RpcCommand {
    let d = driver();
    let w = d.rm_control_wire();
    let mut payload = vec![0; w.params_off + 24];
    payload[8..8 + 4].copy_from_slice(&0x2080_0a89u32.to_le_bytes());
    payload[w.params_size_off..w.params_size_off + 4].copy_from_slice(&24u32.to_le_bytes());
    payload[w.status_off..w.status_off + 4].copy_from_slice(&0xdead_beefu32.to_le_bytes());
    // Nonzero padding is input too; hostile physical address and size are inert bytes.
    payload[w.params_off..].fill(0xa5);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 76,
        sequence: 9,
        payload,
        elements: 1,
        delivered: vec![],
    }
}
#[test]
fn pure_in_bytes_and_padding_are_unchanged_both_statuses_are_ok() {
    let d = driver();
    let w = d.rm_control_wire();
    let mut probe = CmcLogProbe::new(d);
    let cmd = request();
    let reply = probe.respond(&cmd).expect("a89 diagnostic must answer");
    let mut expected = cmd.payload.clone();
    expected[w.status_off..w.status_off + 4].fill(0);
    assert_eq!(reply.rpc_result, 0);
    assert_eq!(reply.body, expected);
    assert_eq!(reply.body.len(), w.params_off + 24);
}
#[test]
fn repeated_registration_and_extreme_guest_addresses_need_no_host_state() {
    let mut probe = CmcLogProbe::new(driver());
    let mut cmd = request();
    for byte in [0, 0xff, 0x55] {
        cmd.payload[driver().rm_control_wire().params_off..].fill(byte);
        let reply = probe.respond(&cmd).expect("bounded stateless echo");
        assert_eq!(
            &reply.body[driver().rm_control_wire().params_off..],
            &cmd.payload[driver().rm_control_wire().params_off..]
        );
    }
}
#[test]
fn malformed_serialized_and_other_controls_are_declined() {
    let d = driver();
    let w = d.rm_control_wire();
    let good = request();
    let mut probe = CmcLogProbe::new(d);
    for length in [
        0,
        w.params_off - 1,
        w.params_off,
        w.params_off + 23,
        w.params_off + 25,
        1024 * 1024,
    ] {
        let mut c = good.clone();
        c.payload.resize(length, 0);
        assert!(probe.respond(&c).is_none(), "length{length}");
    }
    for size in [0, 23, 25, u32::MAX] {
        let mut c = good.clone();
        c.payload[w.params_size_off..w.params_size_off + 4].copy_from_slice(&size.to_le_bytes());
        assert!(probe.respond(&c).is_none());
    }
    let mut c = good.clone();
    c.payload[w.rpc_flags_off..w.rpc_flags_off + 4].copy_from_slice(&2u32.to_le_bytes());
    assert!(probe.respond(&c).is_none());
    let mut c = good.clone();
    c.function = RpcFunction::Free;
    assert!(probe.respond(&c).is_none());
    let mut c = good;
    c.payload[8..8 + 4].copy_from_slice(&0x2080_0ab2u32.to_le_bytes());
    assert!(probe.respond(&c).is_none());
}
#[test]
fn unverified_guest_abi_is_inert() {
    let d = *table_for(DriverVersion {
        major: 580,
        minor: 159,
        patch: 4,
    })
    .unwrap();
    assert!(CmcLogProbe::new(d).respond(&request()).is_none());
}
