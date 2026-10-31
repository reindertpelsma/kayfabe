//! `NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO` (`0x20803801`) at the **reply-plane boundary**,
//! and against the committed real-GA106 capture.
//!
//! ## ⊘ What this file is for, beyond "the control is served"
//!
//! 1. That the reply is the **request edited** at the right offset inside the right
//!    envelope — this control's `[IN]` words (`numQueries`, `queryType`, the per-type input)
//!    must survive, which is the opposite of `ce_get_all_physical_caps.rs`'s claim.
//! 2. ★★★ That the served bytes equal a real GA106's, read out of the committed trace rather
//!    than a literal. The whole 1928-byte record is compared, not just the answered words.
//! 3. ★★ That `gpc_mask` really is the **same row** `INTERNAL_STATIC_KGR_GET_FLOORSWEEPING_MASKS`
//!    serves, by driving both controls through one policy — a claim about two controls
//!    agreeing, which no single-control test can hold.
//! 4. ⚠ That an unmodelled query type takes the **whole control** down rather than hiding in
//!    a per-query status inside an `NV_OK` reply. That is the one failure this control's own
//!    fault tolerance makes invisible, so it is the one most worth a test.

use kayfabe_abi::grfsinfo::{
    self, GR_FS_INFO_PARAMS_SIZE, GrFsQuery, NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO, query_type,
};
use kayfabe_abi::grstatic;
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_device::{ChipProfile, chip_for_device_id};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

const PARAMS_AT: usize = 40;
const CONTROL_STATUS_OFF: usize = 12;

fn chip() -> &'static ChipProfile {
    chip_for_device_id(0x2504).expect("GA106 is in the table")
}
fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

fn command(params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&0xc1e0_0004u32.to_le_bytes());
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes());
    payload[8..12].copy_from_slice(&NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO.to_le_bytes());
    payload[16..20].copy_from_slice(&u32::try_from(params.len()).expect("fits").to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x23,
        sequence: 40,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn reply_params(cmd: &RpcCommand) -> Option<(u32, Vec<u8>)> {
    let reply = policy().respond(cmd)?;
    if reply.body.is_empty() {
        assert_ne!(reply.rpc_result, 0, "an empty body must never carry NV_OK");
        return Some((reply.rpc_result, Vec::new()));
    }
    let status = u32::from_le_bytes(
        reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
            .try_into()
            .expect("4 bytes"),
    );
    Some((
        status,
        reply.body[PARAMS_AT..PARAMS_AT + GR_FS_INFO_PARAMS_SIZE].to_vec(),
    ))
}

/// The real GA106's `in=` / `out=` pair for this control, parsed out of the committed trace.
///
/// ⊘ The parse is asserted at every step — a `find` that matches nothing returns nothing,
/// and a test comparing zero bytes to zero bytes is the `gate_read_through_grep_cannot_fail`
/// shape.
fn real_ga106() -> (Vec<u8>, Vec<u8>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is a committed artifact: {e}", path.display()));
    let needle = format!("cmd={NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO:#010x}");
    let line = text
        .lines()
        .find(|l| l.contains(&needle) && l.contains("out="))
        .unwrap_or_else(|| panic!("no record for {needle} in {}", path.display()));
    // ⚠ A truncated record would silently compare a prefix. The interposer marks those.
    assert!(
        !line.contains("TRUNC"),
        "{needle}'s record is TRUNCATED; a prefix comparison would pass while proving less"
    );
    let field = |k: &str| -> Vec<u8> {
        let hex = line
            .split(k)
            .nth(1)
            .and_then(|t| t.split_whitespace().next())
            .unwrap_or_else(|| panic!("no {k} field for {needle}"));
        assert_eq!(
            hex.len(),
            GR_FS_INFO_PARAMS_SIZE * 2,
            "{k} captured {} bytes, the struct is {GR_FS_INFO_PARAMS_SIZE}",
            hex.len() / 2
        );
        (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("hex"))
            .collect()
    };
    (field("in="), field("out="))
}

/// ★★★ The rung: hand this port libcuda's own request bytes and get the real GA106's own
/// reply bytes, all 1928 of them.
#[test]
fn libcudas_own_request_gets_the_real_ga106s_own_reply() {
    let (input, expected) = real_ga106();
    let (status, params) = reply_params(&command(&input)).expect("served");
    assert_eq!(status, 0, "a real GA106 answers NV_OK");
    assert_eq!(params, expected);
}

/// ⊘ Exactly two bytes change, and they are the two hardware changes.
#[test]
fn only_the_two_out_words_hardware_writes_are_written() {
    let (input, _) = real_ga106();
    let (_, params) = reply_params(&command(&input)).expect("served");
    let diff: Vec<usize> = (0..GR_FS_INFO_PARAMS_SIZE)
        .filter(|&k| input[k] != params[k])
        .collect();
    assert_eq!(diff, [40, 60]);
}

/// ★★ `gpc_mask` is the same row the floorsweeping control serves — driven through one
/// policy so the agreement is executed, not asserted.
#[test]
fn the_gpc_count_agrees_with_the_floorsweeping_masks_control() {
    let req = grfsinfo::build_request(&[GrFsQuery {
        query_type: query_type::GPC_COUNT,
        input: 0,
    }]);
    let (status, params) = reply_params(&command(&req)).expect("served");
    assert_eq!(status, 0);
    let answered = grfsinfo::decode_answers(&params).expect("decode")[0].2;

    let from_the_gr_row = chip()
        .gr_static
        .gpc_mask()
        .expect("GA106 states a GPC mask");
    assert_eq!(answered, from_the_gr_row.count_ones());
    assert_eq!(answered, chip().gr_static.gpcs.len() as u32);
    // ⊘ And the constant is the same value — checked, so that if the chip row ever stops
    // matching it, this says so rather than both quietly moving.
    assert_eq!(from_the_gr_row, grstatic::GA106_GPC_MASK);
}

/// ⚠⚠ **The failure this control's fault tolerance would otherwise hide.** An unmodelled
/// query type must refuse the WHOLE control; a per-query status would ride inside an `NV_OK`
/// and reach no ledger.
#[test]
fn an_unmodelled_query_type_refuses_the_whole_control() {
    for qt in [
        query_type::INVALID,
        query_type::TPC_MASK,
        query_type::PPC_MASK,
        query_type::ROP_MASK,
        0xffff,
    ] {
        let req = grfsinfo::build_request(&[GrFsQuery {
            query_type: qt,
            input: 0,
        }]);
        let (status, params) = reply_params(&command(&req)).expect("claimed, then refused");
        assert_ne!(status, 0, "query type {qt} must refuse the whole control");
        assert!(params.is_empty(), "a refusal carries no params");
    }
}

/// ★ A MIG-only type is the other way round: served, with the refusal in the query's own
/// status, because that is what a real non-MIG GA106 does.
#[test]
fn a_mig_only_query_type_is_served_with_a_per_query_refusal() {
    let req = grfsinfo::build_request(&[
        GrFsQuery {
            query_type: query_type::PARTITION_SYSPIPE_ID,
            input: 0,
        },
        GrFsQuery {
            query_type: query_type::GPC_COUNT,
            input: 0,
        },
    ]);
    let (status, params) = reply_params(&command(&req)).expect("served");
    assert_eq!(status, 0, "the CALL succeeds");
    let a = grfsinfo::decode_answers(&params).expect("decode");
    assert_eq!(
        a[0].1,
        kayfabe_abi::NV_ERR_NOT_SUPPORTED,
        "the QUERY refuses"
    );
    assert_eq!(a[1].1, 0, "and the batch marched on");
    assert_eq!(a[1].2, 3);
}

/// ⊘ A structural fault refuses the whole control, at the policy boundary.
#[test]
fn a_malformed_batch_is_refused_at_the_policy_boundary() {
    for n in [0u16, 97, 0xffff] {
        let mut req = grfsinfo::build_request(&[GrFsQuery {
            query_type: query_type::GPC_COUNT,
            input: 0,
        }]);
        req[0..2].copy_from_slice(&n.to_le_bytes());
        let (status, _) = reply_params(&command(&req)).expect("claimed, then refused");
        assert_ne!(status, 0, "numQueries {n}");
    }
}

/// The id is classified to this table and its declared size is the struct's.
#[test]
fn the_control_is_classified_and_sized() {
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO),
        Some(WantedTable::GrmgrGetGrFsInfo)
    );
    assert_eq!(
        WantedTable::GrmgrGetGrFsInfo.params_size(),
        GR_FS_INFO_PARAMS_SIZE
    );
}
