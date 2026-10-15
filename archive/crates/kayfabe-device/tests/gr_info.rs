//! `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO` (`0x20800a2a`) — the control that ended
//! the `fmb1` boot, and the third one this port answers with bytes **measured on real
//! silicon**.
//!
//! ## What these tests are for
//!
//! Unlike `0x20802a08`, this reply is not one opaque number: 3712 bytes of it are on disk at
//! `traces/real_ga106/rpc_bodies_real_ga106.txt`, and `kayfabe_abi::tests::real_ga106_bodies`
//! already compares the encoder against them byte for byte. What is left for *this* file is
//! everything between the encoder and the guest:
//!
//! 1. the reply reaches the wire as the bytes the guest reads back, over poison rather than
//!    over zeros, so an echo is distinguishable from an answer;
//! 2. **a zero `MAX_SUBCONTEXT_COUNT` is never served** — a served zero is not a weaker
//!    answer, it is `RmInitAdapter failed! (0x25:0x40:1249)` wearing an answer's clothes,
//!    because `kfifoGetMaxSubcontextFromGr_KERNEL` returns whatever is there
//!    (`ogkm-580: kernel_fifo.c:2792`) and `kchangrpapiSetLegacyMode` rejects the zero
//!    (`kernel_channel_group_api.c:913`); and
//! 3. **two descriptions of one chip may not disagree** — six of the 58 entries restate the
//!    geometry `0x20800a1f`/`0x20800a26`/`0x20800a22` publish, and RM reads both.
//!
//! ⊘ Every observation below is of the **raw envelope**. Asking a decoder whether a
//! servable answer came back is the broken instrument that let a planted mutation survive
//! at the `fmb` rung; see `tests/ce_fault_method_buffer_size.rs` for that write-up.

use kayfabe_abi::grinfo::{
    self, GA106_GR_INFO, GR_INFO_ENTRY_SIZE, GrInfoError, IDX_LITTER_NUM_GPCS,
    IDX_MAX_SUBCONTEXT_COUNT, IDX_RT_CORE_COUNT, KGR_GET_INFO_PARAMS_SIZE,
    NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::ChipProfile;
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `RpcControlReq::HEADER` — `cap1b`'s own arithmetic: `paylen 44 - 4 = 40`.
const PARAMS_AT: usize = 40;
/// Byte offset of `status` in the reply's control header.
const CONTROL_STATUS_OFF: usize = 12;

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy_for(c: &'static ChipProfile) -> InitTablePolicy {
    InitTablePolicy::new(c, *table_for(BENCH_DRIVER).expect("bench ABI"))
}

fn policy() -> InitTablePolicy {
    policy_for(chip())
}

/// A `GSP_RM_CONTROL` carrying the guest's `0x20800a2a`, over a `0xAA` fill.
///
/// ★★ The poison is what makes "answered" distinguishable from "echoed". RM sends this
/// struct as a `portMemAllocNonPaged` buffer it does not initialise before the RPC
/// (`ogkm-580: kernel_graphics.c:1228-1234`), so the poison models nothing about the guest —
/// it is an instrument.
fn gr_info_command(params_size: u32) -> RpcCommand {
    let mut payload = vec![0xAAu8; PARAMS_AT + params_size as usize];
    payload[0..4].copy_from_slice(&0xc1e0_0004u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject
    payload[8..12].copy_from_slice(&NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    payload[20..24].copy_from_slice(&0u32.to_le_bytes()); // rmapiRpcFlags: flat, not FINN
    payload[24..40].fill(0);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 50,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// Did the policy produce an **accepted** answer? `rpc_result == 0` and inner `status == 0`,
/// whatever bytes follow — the raw envelope, and nothing that interprets it.
fn accepted(p: &mut InitTablePolicy, cmd: &RpcCommand) -> bool {
    p.respond(cmd).is_some_and(|reply| {
        let Some(st) = reply.body.get(CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4) else {
            return false;
        };
        reply.rpc_result == 0 && u32::from_le_bytes(st.try_into().expect("4 bytes")) == 0
    })
}

#[test]
fn the_control_is_classified_and_sized_as_the_sdk_declares_it() {
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO),
        Some(WantedTable::GrInfo),
    );
    assert_eq!(
        WantedTable::GrInfo.cmd_id(),
        0x2080_0a2a,
        "the id behind the pGrInfo == NULL assertion that ended boot fmb1"
    );
    assert_eq!(WantedTable::GrInfo.params_size(), KGR_GET_INFO_PARAMS_SIZE);
    assert_eq!(
        WantedTable::GrInfo.params_size(),
        3712,
        "the psize a real GA106 answered — traces/real_ga106/rpc_bodies_real_ga106.txt"
    );
}

#[test]
fn the_reply_carries_the_bytes_the_real_ga106_put_on_the_wire() {
    let cmd = gr_info_command(KGR_GET_INFO_PARAMS_SIZE as u32);
    let reply = policy()
        .respond(&cmd)
        .expect("this port serves 0x20800a2a since the GR-info rung");
    assert_eq!(reply.rpc_result, 0, "the envelope says NV_OK");
    let status = u32::from_le_bytes(
        reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
            .try_into()
            .expect("4 bytes"),
    );
    assert_eq!(status, 0, "and so does the inner control status");
    let params = &reply.body[PARAMS_AT..PARAMS_AT + KGR_GET_INFO_PARAMS_SIZE];
    assert_eq!(
        params,
        GA106_GR_INFO.encode().expect("the GA106 row encodes"),
        "the reply is the chip row's encoding, unmodified in transit"
    );
    // ⊘ And not the poison. An echo would bring `0xAA` back, and `0xAAAAAAAA` at
    // `infoList[0x2c]` is ALSO non-zero — it would clear `numMax != 0` and then size a
    // golden context buffer at `RM_PAGE_SIZE * 0xAAAAAAAA`. Passing that assert is not the
    // bar.
    assert!(
        !params.contains(&0xAA),
        "the reply is an answer, not an echo"
    );
    let at = IDX_MAX_SUBCONTEXT_COUNT * GR_INFO_ENTRY_SIZE;
    assert_eq!(
        u32::from_le_bytes(params[at + 4..at + 8].try_into().expect("4 bytes")),
        64,
        "MAX_SUBCONTEXT_COUNT, read back off the wire"
    );
}

#[test]
fn the_answer_comes_from_the_chip_row_and_not_from_this_crate() {
    // ⊘ The property that keeps the measurement attributable: a second generation that has
    // never been asked cannot be answered with GA106's 58 numbers by accident.
    assert_eq!(chip().gr_info.data, GA106_GR_INFO.data);
}

#[test]
fn a_zero_max_subcontext_count_is_refused_rather_than_served() {
    // ★★★ The falsifier for the whole rung, observed on the RAW ENVELOPE.
    assert!(
        GA106_GR_INFO.encode().is_ok(),
        "the real row still encodes, so the check below is not passing for the wrong reason"
    );
    let mut bad = *chip();
    bad.gr_info.data[IDX_MAX_SUBCONTEXT_COUNT] = 0;
    let leaked: &'static ChipProfile = Box::leak(Box::new(bad));
    let cmd = gr_info_command(KGR_GET_INFO_PARAMS_SIZE as u32);
    assert!(
        !accepted(&mut policy_for(leaked), &cmd),
        "a chip whose GR info says zero maximum subcontexts must not produce an accepted \
         reply — that zero IS RmInitAdapter 0x25:0x40:1249"
    );
    // …and the real row does, so the assertion above is discriminating.
    assert!(accepted(&mut policy(), &cmd));
}

#[test]
fn the_other_two_load_bearing_zeros_are_refused_on_the_wire_too() {
    let cmd = gr_info_command(KGR_GET_INFO_PARAMS_SIZE as u32);
    for (index, name) in [
        (IDX_LITTER_NUM_GPCS, "LITTER_NUM_GPCS"),
        (
            grinfo::IDX_LITTER_MIN_SUBCTX_PER_SMC_ENG,
            "LITTER_MIN_SUBCTX_PER_SMC_ENG",
        ),
    ] {
        let mut bad = *chip();
        bad.gr_info.data[index] = 0;
        let leaked: &'static ChipProfile = Box::leak(Box::new(bad));
        assert!(
            !accepted(&mut policy_for(leaked), &cmd),
            "a zero {name} must not be served"
        );
    }
}

#[test]
fn a_chip_whose_two_gr_descriptions_disagree_is_refused_on_the_wire() {
    // ★★ The cross-check, at the serve site rather than only in a unit test. RM reads the
    // RT-core count out of this control and the SM geometry out of `0x20800a22`; a device
    // that published 27 RT cores and 28 SMs would be believed twice and contradicted never.
    let mut bad = *chip();
    bad.gr_info.data[IDX_RT_CORE_COUNT] = 27;
    let leaked: &'static ChipProfile = Box::leak(Box::new(bad));
    let cmd = gr_info_command(KGR_GET_INFO_PARAMS_SIZE as u32);
    assert!(
        !accepted(&mut policy_for(leaked), &cmd),
        "two descriptions of one chip disagree and the port answered anyway"
    );
    // The disagreement is named, not merely detected.
    assert_eq!(
        bad.gr_info.validate_against(&bad.gr_static),
        Err(GrInfoError::DisagreesWithGrStatic {
            index: IDX_RT_CORE_COUNT,
            info: 27,
            derived: 28,
        })
    );
}
