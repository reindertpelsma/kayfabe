//! `NV2080_CTRL_CMD_BUS_GET_INFO_V2` (`0x20801823`) at the **reply-plane boundary**.
//!
//! ## ⊘ Why this file exists, and what it says about the rung that landed the control
//!
//! §14.30 served this control with unit tests over `kayfabe_abi::businfo::
//! answer_bus_get_info_v2` and **nothing at the `InitTablePolicy::respond` boundary** — so
//! nothing checked that the answer reaches the guest in the right envelope, with the right
//! inner status, at the right offset in the reply body. `cap1b_differential.rs`'s closure
//! assertion is the gate that would have said so, and `[measured 2026-08-08]` it has been
//! **red since §14.29** for exactly this reason, on two controls.
//!
//! ⚠ A served control with no reply-plane test is the `a_flag_is_not_progress` shape at one
//! remove: the ABI function is correct and well tested, and the thing that carries it to the
//! guest is untested. The ABI layer cannot tell you that the policy passed it the wrong
//! slice of the payload.

use kayfabe_abi::businfo::{
    self, BUS_GET_INFO_V2_PARAMS_SIZE, BUS_INFO_INDEX_PCIE_GEN_INFO,
    NV2080_CTRL_CMD_BUS_GET_INFO_V2, PcieGenInfo,
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

/// A `GSP_RM_CONTROL` carrying `0x20801823` with the given `(index, data)` entries.
///
/// ★ The params tail past the declared entries is seeded `0xCD`, because `[measured
/// 2026-08-08, real GA106, R22]` real RM returns it **untouched** — so the seed surviving is
/// an assertion, not decoration.
fn bus_command(entries: &[(u32, u32)], params_size: u32) -> RpcCommand {
    let mut payload = vec![0xCDu8; PARAMS_AT + params_size as usize];
    payload[0..4].copy_from_slice(&0xc1e0_0004u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject
    payload[8..12].copy_from_slice(&NV2080_CTRL_CMD_BUS_GET_INFO_V2.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    payload[20..24].copy_from_slice(&0u32.to_le_bytes()); // rmapiRpcFlags: flat, not FINN
    payload[24..40].fill(0);
    if payload.len() >= PARAMS_AT + 4 {
        payload[PARAMS_AT..PARAMS_AT + 4].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        for (i, &(idx, data)) in entries.iter().enumerate() {
            let at = PARAMS_AT + 4 + 8 * i;
            if at + 8 <= payload.len() {
                payload[at..at + 4].copy_from_slice(&idx.to_le_bytes());
                payload[at + 4..at + 8].copy_from_slice(&data.to_le_bytes());
            }
        }
    }
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x23,
        sequence: 32,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

#[test]
fn the_control_is_classified_and_sized_as_the_sdk_declares_it() {
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_BUS_GET_INFO_V2),
        Some(WantedTable::BusGetInfoV2),
    );
    assert_eq!(WantedTable::BusGetInfoV2.cmd_id(), 0x2080_1823);
    assert_eq!(
        WantedTable::BusGetInfoV2.params_size(),
        BUS_GET_INFO_V2_PARAMS_SIZE,
    );
    assert_eq!(BUS_GET_INFO_V2_PARAMS_SIZE, 420, "size on the wire");
}

#[test]
fn the_forwarded_index_is_answered_in_the_envelope_and_the_tail_is_left_alone() {
    let cmd = bus_command(
        &[(BUS_INFO_INDEX_PCIE_GEN_INFO, 0)],
        BUS_GET_INFO_V2_PARAMS_SIZE as u32,
    );
    let reply = policy().respond(&cmd).expect("this port serves 0x20801823");
    assert_eq!(reply.rpc_result, 0, "the envelope says NV_OK");
    let status = u32::from_le_bytes(
        reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
            .try_into()
            .expect("4 bytes"),
    );
    assert_eq!(status, 0, "and so does the inner control status");

    let params = &reply.body[PARAMS_AT..PARAMS_AT + BUS_GET_INFO_V2_PARAMS_SIZE];
    assert_eq!(
        businfo::decode_bus_info_pairs(params),
        Ok(vec![(
            BUS_INFO_INDEX_PCIE_GEN_INFO,
            PcieGenInfo::fully_trained(chip().pcie_max_gen).encode()
        )]),
        "the served word is DERIVED from the chip's die generation, never transcribed"
    );
    // ⊘ And it is not either measured word: `0x00302000` (idle) and `0x00322000` (loaded)
    // are one rented slot at one instant. The port presents a link trained at the die's own
    // generation.
    let word = u32::from_le_bytes([params[8], params[9], params[10], params[11]]);
    assert_ne!(word, 0x0030_2000);
    assert_ne!(word, 0x0032_2000);
    let decoded = PcieGenInfo::decode(word).expect("the served word decodes");
    assert_eq!(decoded.gpu_gen, decoded.negotiated_gen);
    assert_eq!(decoded.gpu_gen, decoded.current_gen);

    // ★ `[measured]` R22: all 52 indices came back with the tail past the declared entry
    // untouched. The `0xCD` seed surviving is that measurement.
    assert!(
        params[12..].iter().all(|&b| b == 0xCD),
        "the tail past the declared entry is not this port's to write"
    );
}

#[test]
fn an_index_with_no_derivation_refuses_the_whole_call_by_name() {
    // ⊘ Quantified over a list, and it includes indices the guest's own kernel normally
    // answers — `0x0f` BUS_NUMBER, `0x10` DEVICE_NUMBER, `0x2c` DOMAIN_NUMBER, `0x03`
    // PCIE_GPU_LINK_CAPS, `0x06` PCIE_DOWNSTREAM_LINK_CAPS. If one of those ever ARRIVES
    // here it means the guest could not compute it, and answering zero would be the
    // positive claim `PCIE_LINK_CAP_GEN_GEN1` rather than "unknown".
    for index in [0x00u32, 0x02, 0x03, 0x06, 0x0b, 0x0f, 0x10, 0x2c, 0x33] {
        let reply = policy().respond(&bus_command(
            &[(index, 0)],
            BUS_GET_INFO_V2_PARAMS_SIZE as u32,
        ));
        match reply {
            None => panic!("index {index:#x} must be refused, not left unclassified"),
            Some(r) => {
                assert_ne!(r.rpc_result, 0, "index {index:#x} must not be served NV_OK");
                assert!(r.body.is_empty(), "index {index:#x} refused with a body");
            }
        }
    }
}

#[test]
fn one_unmeasured_index_alongside_the_served_one_refuses_the_whole_call() {
    // ★★ RM's own shape: `getBusInfos` forwards under `NV_CHECK_OK_OR_RETURN` and returns
    // the first failure for the ENTIRE request (`ogkm-580: kern_bus_ctrl.c:333`). A port
    // that answered the entries it knew and left the rest would return `NV_OK` over a
    // partly-unwritten struct — the worst available outcome, because the guest would read
    // `Gen 1` out of the untouched slot.
    let reply = policy().respond(&bus_command(
        &[(BUS_INFO_INDEX_PCIE_GEN_INFO, 0), (0x0f, 0)],
        BUS_GET_INFO_V2_PARAMS_SIZE as u32,
    ));
    let r = reply.expect("classified");
    assert_ne!(r.rpc_result, 0);
    assert!(r.body.is_empty());
}

#[test]
fn a_wrongly_sized_request_is_refused_rather_than_answered() {
    for bad in [0u32, 4, 12, 419, 421, 112] {
        let reply = policy().respond(&bus_command(&[(BUS_INFO_INDEX_PCIE_GEN_INFO, 0)], bad));
        if let Some(r) = reply {
            assert_ne!(
                r.rpc_result, 0,
                "params_size {bad} must not be served NV_OK"
            );
            assert!(r.body.is_empty(), "params_size {bad} refused with a body");
        }
    }
}

#[test]
fn a_guest_supplied_list_size_is_not_taken_on_trust() {
    // The count is the guest's, and it is a loop bound over a buffer. ⊘ Checked before use,
    // never after: `busInfoListSize = 0` and anything past 52 are both refused.
    for count in [0u32, 53, 0xFFFF_FFFF] {
        let mut cmd = bus_command(
            &[(BUS_INFO_INDEX_PCIE_GEN_INFO, 0)],
            BUS_GET_INFO_V2_PARAMS_SIZE as u32,
        );
        cmd.payload[PARAMS_AT..PARAMS_AT + 4].copy_from_slice(&count.to_le_bytes());
        let r = policy().respond(&cmd).expect("classified");
        assert_ne!(
            r.rpc_result, 0,
            "busInfoListSize {count} must not be served"
        );
        assert!(r.body.is_empty());
    }
}
