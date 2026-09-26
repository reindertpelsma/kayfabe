//! `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS` (`0x20801702`) — ★★★ v3-mapfix: SERVED, because its
//! refusal forged completions (`kf_abi::mcintr` carries the measurement and the argument).
//!
//! What this file pins: the control is in the served universe; the reply is `NV_OK` in the
//! envelope AND in the control header; `engines` comes back exactly as sent (the guest re-reads
//! it after the RPC to choose what it services itself); a params image that is not the
//! four-byte struct is still refused by name.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::mcintr::{
    MC_ENGINE_ID_ALL, MC_ENGINE_ID_GRAPHICS, MC_SERVICE_INTERRUPTS_PARAMS_SIZE,
    NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS,
};
use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::inittables::{InitTablePolicy, WantedTable};

/// `RpcControlReq::HEADER`.
const PARAMS_AT: usize = 40;
/// Byte offset of `status` in the control header.
const CONTROL_STATUS_OFF: usize = 12;

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(ga106::board(), ga106::host(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// A `GSP_RM_CONTROL` carrying `MC_SERVICE_INTERRUPTS` over a `0xAA` fill (so an unwritten byte
/// is visible), `params_size` as declared and `engines` at the head of the params.
fn service_command(engines: u32, params_size: u32) -> RpcCommand {
    let mut payload = vec![0xAAu8; PARAMS_AT + params_size as usize];
    payload[0..4].copy_from_slice(&0xc1d0_0017u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0x5c00_0003u32.to_le_bytes()); // hObject (a subdevice)
    payload[8..12].copy_from_slice(&NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    payload[20..24].copy_from_slice(&0u32.to_le_bytes()); // rmapiRpcFlags: flat
    payload[24..40].fill(0);
    if params_size >= 4 {
        payload[PARAMS_AT..PARAMS_AT + 4].copy_from_slice(&engines.to_le_bytes());
    }
    RpcCommand { function: RpcFunction::RmControl, code: 0x4c, sequence: 329, payload, elements: 1, delivered: Vec::new() }
}

#[test]
fn the_control_is_in_the_served_universe() {
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS),
        Some(WantedTable::McServiceInterrupts)
    );
    assert!(WantedTable::ALL.contains(&WantedTable::McServiceInterrupts));
    assert_eq!(WantedTable::McServiceInterrupts.params_size(), MC_SERVICE_INTERRUPTS_PARAMS_SIZE);
}

#[test]
fn what_libcuda_sends_is_answered_ok_with_engines_echoed() {
    // `[measured]` libcuda and the NVIDIA OpenCL runtime send `engines = ALL`.
    for engines in [MC_ENGINE_ID_ALL, MC_ENGINE_ID_GRAPHICS] {
        let cmd = service_command(engines, MC_SERVICE_INTERRUPTS_PARAMS_SIZE as u32);
        let reply = policy().respond(&cmd).expect("0x20801702 is served");
        assert_eq!(reply.rpc_result, 0, "the envelope says NV_OK");
        let status = u32::from_le_bytes(reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4].try_into().unwrap());
        assert_eq!(status, 0, "the control header's own status says NV_OK");
        let back = u32::from_le_bytes(reply.body[PARAMS_AT..PARAMS_AT + 4].try_into().unwrap());
        assert_eq!(
            back, engines,
            "engines must come back as sent: the guest re-reads it after the RPC to pick what it \
             services itself (intr.c:228-278), and a zero would silently cancel that"
        );
        assert_eq!(reply.body.len(), cmd.payload.len());
    }
}

#[test]
fn a_params_image_that_is_not_the_struct_is_refused() {
    for size in [0u32, 2, 8] {
        let cmd = service_command(MC_ENGINE_ID_ALL, size);
        let reply = policy().respond(&cmd).expect("a WantedTable row answers, refusing by name");
        assert_ne!(reply.rpc_result, 0, "params size {size} is not NV2080_CTRL_MC_SERVICE_INTERRUPTS_PARAMS");
    }
}
