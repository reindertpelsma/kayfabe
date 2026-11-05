//! `NV2080_CTRL_CMD_BUS_GET_C2C_INFO` (`0x2080182b`) at the **reply-plane boundary**.
//!
//! ## ⊘⊘ Why this file exists, and it is not "for completeness"
//!
//! §14.37 shipped this control's serve arm with the chip flag **negated** —
//! `c2c_absent(!self.chip.has_c2c)` — so a GA106 (`has_c2c: false`) asked the encoder about a
//! part that *has* C2C and got the refusal that arm exists to produce. `kayfabe_abi::c2cinfo`'s
//! unit tests were **green throughout**: they cover both arms of `c2c_absent` faithfully, and
//! the defect was in the argument the call site passed, which no test of the callee can see.
//!
//! `[measured 2026-08-09, boot `gf1437` at `e7bb8c6`]` the boot caught it — row 86 of the guest's
//! `cuInit` trace still answered `0x56` while every accounting gate agreed the control was
//! served. ★ That is `only_live_boots_are_proof` earning its place, and the cheap fix is a test
//! at the boundary the boot exercises rather than at the one the encoder lives in.

use kayfabe_abi::c2cinfo::{C2C_INFO_PARAMS_SIZE, NV2080_CTRL_CMD_BUS_GET_C2C_INFO};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_device::{ChipProfile, chip_for_device_id};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

const PARAMS_AT: usize = 40;
const CONTROL_STATUS_OFF: usize = 12;

fn chip() -> &'static ChipProfile {
    chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn command(params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&0xc1d0_000cu32.to_le_bytes());
    payload[4..8].copy_from_slice(&0x5c00_0003u32.to_le_bytes());
    payload[8..12].copy_from_slice(&NV2080_CTRL_CMD_BUS_GET_C2C_INFO.to_le_bytes());
    payload[16..20].copy_from_slice(&u32::try_from(params.len()).expect("fits").to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x23,
        sequence: 86,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn served(params: &[u8]) -> (u32, Vec<u8>) {
    let mut p = InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"));
    let reply = p
        .respond(&command(params))
        .expect("the policy claims this control");
    if reply.body.is_empty() {
        assert_ne!(reply.rpc_result, 0, "an empty body must never carry NV_OK");
        return (reply.rpc_result, Vec::new());
    }
    let status = u32::from_le_bytes(
        reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
            .try_into()
            .expect("4 bytes"),
    );
    (
        status,
        reply.body[PARAMS_AT..PARAMS_AT + C2C_INFO_PARAMS_SIZE].to_vec(),
    )
}

/// ★★★ The regression the boot found: a GA106 must be **answered**, not refused.
#[test]
fn a_ga106_is_answered_nv_ok_and_not_refused() {
    let (status, params) = served(&[0u8; C2C_INFO_PARAMS_SIZE]);
    assert_eq!(
        status, 0,
        "a real GA106 answers NV_OK; a refusal here is the negated-flag defect returning"
    );
    assert_eq!(params.len(), C2C_INFO_PARAMS_SIZE);
    assert!(
        params.iter().all(|&b| b == 0),
        "no C2C links, so every field is zero"
    );
}

/// ⊘ The falsifier: assert the chip row this port ships really is the no-C2C one, so the test
/// above cannot pass by the flag having drifted to `true` and the encoder's other arm firing.
#[test]
fn the_shipped_chip_row_is_the_one_the_answer_is_argued_from() {
    assert!(
        !chip().has_c2c,
        "GA106 is a consumer GeForce die; the all-zero reply is only correct because of this"
    );
}

/// The id classifies to the variant, and the variant states this struct's size.
#[test]
fn the_id_and_the_size_are_one_statement() {
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_BUS_GET_C2C_INFO),
        Some(WantedTable::C2cInfo)
    );
    assert_eq!(WantedTable::C2cInfo.params_size(), C2C_INFO_PARAMS_SIZE);
}

/// ⊘ A size the guest declares wrong is refused rather than answered.
#[test]
fn a_size_the_guest_declares_wrong_is_refused() {
    for wrong in [0usize, 27, 29] {
        let (status, _) = served(&vec![0u8; wrong]);
        assert_ne!(status, 0, "a {wrong}-byte params_size is not this struct");
    }
}
