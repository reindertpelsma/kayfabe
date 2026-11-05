//! `NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE` (`0x20800a4c`) — ★★★★ **the control that
//! decided `cuInit`**, named by an in-guest bisect on 2026-08-08 (`execution_plane_
//! increments.md` §14.29, boot `gis1_e6ed6bc`).
//!
//! ## ⊘ What these tests are NOT
//!
//! They are not proof that `cuInit` succeeds. Only a boot is (`only_live_boots_are_proof`).
//! What they pin is the pair of facts that would make a correct value useless anyway:
//!
//! 1. the reply body is the four bytes `getGpuInfos` reads back on the next line
//!    (`data = params.smcMode;`, `ogkm-580: subdevice_ctrl_gpu_kernel.c:265`, over the struct
//!    `rpc.c:11085-11090` copies out), and
//! 2. **the answer is not the poison and not an echo** — because on this part the correct
//!    answer is four ZERO bytes, which is also what a buffer nobody wrote looks like.
//!
//! ★ (2) is the whole reason this file is careful. For `kayfabe_abi::fmbsize` the wrong
//! answer was zero and the right answer was not, so *"non-zero"* was a usable proxy. Here
//! the polarity is inverted: zero is the measured, named, correct answer
//! (`NV2080_CTRL_GPU_INFO_GPU_SMC_MODE_UNSUPPORTED`), so nothing about the *value* can
//! distinguish "served correctly" from "never written". Only the poison fill can.

use kayfabe_abi::smcmode::{
    self, GA106_SMC_MODE, INTERNAL_GPU_GET_SMC_MODE_PARAMS_SIZE,
    NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE, SmcMode,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_device::{ChipProfile, chip_for_device_id};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER`.
const PARAMS_AT: usize = 40;

/// Byte offset of `status` in the reply's control header.
const CONTROL_STATUS_OFF: usize = 12;

fn chip() -> &'static ChipProfile {
    chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// A `GSP_RM_CONTROL` carrying the guest's `0x20800a4c`.
///
/// ★★ Over a `0xAA` fill. The guest actually sends this struct **zeroed**
/// (`portMemSet(&params, 0x0, sizeof(params))`, `ogkm-580:
/// subdevice_ctrl_gpu_kernel.c:259`) — the poison is not a model of the guest, it is the
/// only instrument that can tell a served zero from an unwritten one on this control.
fn smc_command(params_size: u32) -> RpcCommand {
    let mut payload = vec![0xAAu8; PARAMS_AT + params_size as usize];
    payload[0..4].copy_from_slice(&0xc1e0_0004u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject
    payload[8..12].copy_from_slice(&NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    payload[20..24].copy_from_slice(&0u32.to_le_bytes()); // rmapiRpcFlags: flat, not FINN
    payload[24..40].fill(0);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 30,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

#[test]
fn the_control_is_classified_and_sized_as_the_sdk_declares_it() {
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_INTERNAL_GPU_GET_SMC_MODE),
        Some(WantedTable::InternalGpuGetSmcMode),
    );
    assert_eq!(
        WantedTable::InternalGpuGetSmcMode.cmd_id(),
        0x2080_0a4c,
        "the id in the `unserviced fn 76` line of eight committed bench boots"
    );
    assert_eq!(
        WantedTable::InternalGpuGetSmcMode.params_size(),
        INTERNAL_GPU_GET_SMC_MODE_PARAMS_SIZE,
    );
}

#[test]
fn the_reply_carries_the_bytes_the_real_ga106_put_on_the_wire() {
    // ★★★ Transcribed from the RAW RPC reply on a real part —
    // `traces/real_ga106/rpc_bodies_real_ga106.txt:617-619`,
    // `cmd=0x20800a4c psize=4 gspst=0x0` / `+00000 00 00 00 00`.
    let cmd = smc_command(INTERNAL_GPU_GET_SMC_MODE_PARAMS_SIZE as u32);
    let reply = policy()
        .respond(&cmd)
        .expect("this port serves 0x20800a4c since the §14.29 rung");
    assert_eq!(reply.rpc_result, 0, "the envelope says NV_OK");
    let status = u32::from_le_bytes(
        reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
            .try_into()
            .expect("4 bytes"),
    );
    assert_eq!(status, 0, "and so does the inner control status");
    let params = &reply.body[PARAMS_AT..PARAMS_AT + INTERNAL_GPU_GET_SMC_MODE_PARAMS_SIZE];
    assert_eq!(
        params,
        &[0x00, 0x00, 0x00, 0x00],
        "what a real GA106 returned"
    );

    // ⊘⊘ THE ASSERTION THAT CARRIES THE FILE. The four bytes above are also what an
    // untouched buffer, a dropped reply, or a body this policy never wrote would produce —
    // on this control, and on no other one this port serves. The poison is the only witness
    // that the arm RAN: `0xAA` survives everything except an arm that overwrote it.
    assert!(
        !params.contains(&0xAA),
        "the params were never written — a zero here is indistinguishable from the right \
         answer, which is exactly why the request is poison-filled"
    );
    assert_eq!(
        smcmode::decode_smc_mode(params),
        Ok(SmcMode::Unsupported),
        "and it decodes to the NAMED meaning, not to a bare zero"
    );
    assert_eq!(smcmode::decode_smc_mode(params), Ok(GA106_SMC_MODE));
}

#[test]
fn a_wrongly_sized_request_is_refused_rather_than_answered() {
    // RM allocates exactly `sizeof(NV2080_CTRL_INTERNAL_GPU_GET_SMC_MODE_PARAMS)`; a request
    // claiming any other size is not this control and must not be served as though it were.
    // ⊘ Serving a short one would write four bytes past a buffer the guest sized.
    //
    // ⚠ The refusal is signalled in the **RPC envelope** (`rpc_result`) over an *empty*
    // body, not in an inner control status (`inittables.rs:855-860`). An earlier draft of
    // this test read `body[12..16]` and panicked on the empty vector — the instrument, not
    // the port (`suspect_the_instrument_first`). Asserting on the envelope is also the
    // stronger check: a refusal with a body would be a different bug and fails here too.
    for bad in [0u32, 2, 3, 5, 8, 564] {
        let reply = policy().respond(&smc_command(bad));
        match reply {
            None => {}
            Some(r) => {
                assert_ne!(
                    r.rpc_result, 0,
                    "params_size {bad} must not be served as NV_OK"
                );
                assert!(
                    r.body.is_empty(),
                    "params_size {bad} was refused but still carried a {}-byte body",
                    r.body.len()
                );
            }
        }
    }
}

#[test]
fn the_chip_row_states_a_mode_and_it_is_the_measured_one() {
    // ⚠ Not a tautology, and the shape is the point: `smc_mode` is an ENUM, so "the row
    // forgot to state it" cannot compile — unlike `ce_fault_method_buffer_size`, where zero
    // is the sentinel for unstated and realize has to catch it. On a control whose correct
    // answer IS zero, a numeric sentinel is unavailable by construction.
    assert_eq!(chip().smc_mode, SmcMode::Unsupported);
    assert_eq!(chip().smc_mode, GA106_SMC_MODE);
    // ★ GeForce silicon has no MIG. `Disabled` would claim a MIG-capable part with MIG off,
    // which is a different statement about the machine and is what a guess would have
    // produced from the name alone.
    assert_ne!(chip().smc_mode, SmcMode::Disabled);
}

#[test]
fn every_declared_mode_round_trips_through_the_served_reply_shape() {
    // ★ Quantified over `SmcMode::ALL` rather than over the one value this chip uses, so a
    // sixth code point added to the enum without an encoding is caught here
    // (`gates_quantified_over_a_list`).
    for mode in SmcMode::ALL {
        let body = smcmode::encode_smc_mode(mode);
        assert_eq!(body.len(), INTERNAL_GPU_GET_SMC_MODE_PARAMS_SIZE);
        assert_eq!(smcmode::decode_smc_mode(&body), Ok(mode));
    }
}
