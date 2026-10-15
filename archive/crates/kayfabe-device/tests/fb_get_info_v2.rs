//! `NV2080_CTRL_CMD_FB_GET_INFO_V2` (`0x20801303`) at the **reply-plane boundary**.
//!
//! ## ⊘ What this file is for, beyond "the control is served"
//!
//! Two things the ABI layer's own unit tests structurally cannot say:
//!
//! 1. That `InitTablePolicy` hands `answer_fb_get_info_v2` the **right slice** of the
//!    payload and puts the answer back at the right offset, inside the right envelope, with
//!    the right inner status. `kayfabe-abi` sees a bare params buffer and could not tell.
//! 2. ★★ That the four served words really are the **same values** the port already serves
//!    to `NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG` (`0x20800a1c`). This is the
//!    whole design claim of the rung — *"this control states no new number"* — and it is a
//!    claim about **two controls agreeing**, so no single-control test can hold it. It is
//!    checked here by driving both through one policy and comparing the bytes.
//!
//! ⚠ Written **with** the row rather than after it: `cap1b_differential.rs` names this file
//! as what stands in for the differential coverage `cap1b` cannot give, and §14.30's
//! `BusGetInfoV2` landed with that admission true only in retrospect.

use kayfabe_abi::fbinfo::{
    self, FB_GET_INFO_V2_PARAMS_SIZE, FB_INFO_INDEX_BUS_WIDTH, FB_INFO_INDEX_FB_IS_BROKEN,
    FB_INFO_INDEX_FBP_COUNT, FB_INFO_INDEX_FBP_MASK, FB_INFO_INDEX_L2CACHE_SIZE,
    FB_INFO_INDEX_LTC_COUNT, FB_INFO_INDEX_LTS_COUNT, FB_INFO_INDEX_MAX,
    FB_INFO_INDEX_RAM_LOCATION, FB_INFO_INDEX_RAM_TYPE, FB_INFO_INDEX_TOTAL_RAM_SIZE,
    FB_INFO_MAX_LIST_SIZE, NV2080_CTRL_CMD_FB_GET_INFO_V2,
};
use kayfabe_abi::memsysconfig;
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

/// A `GSP_RM_CONTROL` carrying an arbitrary control id with an arbitrary params body.
fn command(cmd_id: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0xCDu8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&0xc1e0_0004u32.to_le_bytes()); // hClient
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject
    payload[8..12].copy_from_slice(&cmd_id.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&(params.len() as u32).to_le_bytes());
    payload[20..24].copy_from_slice(&0u32.to_le_bytes()); // rmapiRpcFlags: flat, not FINN
    payload[24..40].fill(0);
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

/// An `FB_GET_INFO_V2` request whose declared entries carry `index` and a zero `data`, with
/// the whole tail past them seeded `0xAA`.
///
/// ★ The seed is an assertion, not decoration: a `data` word this port writes must come from
/// the derivation and not from whatever happened to be in the buffer, and the tail must come
/// back exactly as it arrived.
fn fb_command(indices: &[u32]) -> RpcCommand {
    let mut params = vec![0xAAu8; FB_GET_INFO_V2_PARAMS_SIZE];
    params[0..4].copy_from_slice(&(indices.len() as u32).to_le_bytes());
    for (i, &idx) in indices.iter().enumerate() {
        let at = 4 + 8 * i;
        params[at..at + 4].copy_from_slice(&idx.to_le_bytes());
        params[at + 4..at + 8].copy_from_slice(&0u32.to_le_bytes());
    }
    command(NV2080_CTRL_CMD_FB_GET_INFO_V2, &params)
}

/// The reply's `(inner control status, params)`.
///
/// ⊘ A refusal is `Reply { rpc_result: NV_ERR_NOT_SUPPORTED, body: [] }` — the envelope
/// carries the refusal and there is no control header at all — so a refusal is reported as
/// its `rpc_result` with no params. Reading `body[12..16]` unconditionally is what a reader
/// who assumed a served shape would do, and it panics rather than reporting `0`.
fn reply_params(cmd: &RpcCommand) -> Option<(u32, Vec<u8>)> {
    let reply = policy().respond(cmd)?;
    if reply.body.is_empty() {
        assert_ne!(
            reply.rpc_result, 0,
            "an empty body must never travel with NV_OK"
        );
        return Some((reply.rpc_result, Vec::new()));
    }
    let status = u32::from_le_bytes(
        reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
            .try_into()
            .expect("4 bytes"),
    );
    Some((
        status,
        reply.body[PARAMS_AT..PARAMS_AT + FB_GET_INFO_V2_PARAMS_SIZE].to_vec(),
    ))
}

#[test]
fn the_control_is_classified_and_sized_as_the_sdk_declares_it() {
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_FB_GET_INFO_V2),
        Some(WantedTable::FbGetInfoV2),
    );
    assert_eq!(WantedTable::FbGetInfoV2.cmd_id(), 0x2080_1303);
    assert_eq!(
        WantedTable::FbGetInfoV2.params_size(),
        FB_GET_INFO_V2_PARAMS_SIZE,
    );
    assert_eq!(
        FB_GET_INFO_V2_PARAMS_SIZE, 1028,
        "the size on the wire, `[measured]` on five real-GA106 ioctls and four of our own"
    );
}

/// ★★★ **The wall itself.** `_kmemsysGetFbInfos` compacts libcuda's seven indices down to
/// the four it cannot answer; this is that request, and these are the words a real GA106
/// puts back (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:50`, re-derived from the
/// raw `out=` hex).
#[test]
fn the_forwarded_four_are_answered_with_real_hardwares_own_words() {
    let cmd = fb_command(&[
        FB_INFO_INDEX_BUS_WIDTH,
        FB_INFO_INDEX_FBP_COUNT,
        FB_INFO_INDEX_L2CACHE_SIZE,
        FB_INFO_INDEX_RAM_TYPE,
    ]);
    let reply = policy().respond(&cmd).expect("this port serves 0x20801303");
    assert_eq!(reply.rpc_result, 0, "the envelope says NV_OK");
    let (status, params) = reply_params(&cmd).expect("served");
    assert_eq!(status, 0, "and so does the inner control status");
    assert_eq!(
        fbinfo::decode_fb_info_pairs(&params),
        Ok(vec![
            (FB_INFO_INDEX_BUS_WIDTH, 0x0000_00c0),
            (FB_INFO_INDEX_FBP_COUNT, 0x0000_0003),
            (FB_INFO_INDEX_L2CACHE_SIZE, 0x0024_0000),
            (FB_INFO_INDEX_RAM_TYPE, 0x0000_0011),
        ]),
    );
    // ⊘ The tail past the four declared entries arrives back untouched — the `0xAA` seed
    // survives, so nothing here writes outside the declared list.
    assert!(
        params[4 + 8 * 4..].iter().all(|&b| b == 0xAA),
        "the tail past the declared entries must be returned untouched"
    );
}

/// ★★★ **The design claim, executed rather than asserted in prose: the two controls that
/// state this silicon agree, because there is only one statement of it.**
#[test]
fn the_words_are_the_same_bytes_the_memsys_static_config_serves() {
    let mut p = policy();
    // What `0x20800a1c` puts on the wire, read back through its own decoder's offsets.
    let memsys = p
        .respond(&command(
            memsysconfig::NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG,
            &[0u8; memsysconfig::MEMSYS_STATIC_CONFIG_PARAMS_SIZE],
        ))
        .expect("this port serves 0x20800a1c");
    let mp = &memsys.body[PARAMS_AT..PARAMS_AT + memsysconfig::MEMSYS_STATIC_CONFIG_PARAMS_SIZE];
    let get32 = |off: usize| u32::from_le_bytes(mp[off..off + 4].try_into().expect("4 bytes"));
    let get64 = |off: usize| u64::from_le_bytes(mp[off..off + 8].try_into().expect("8 bytes"));
    let l2 = get64(memsysconfig::L2_CACHE_SIZE_OFF);
    let ram_type = get32(memsysconfig::RAM_TYPE_OFF);
    let ltc_count = get32(memsysconfig::LTC_COUNT_OFF);

    let (_, fb) = reply_params(&fb_command(&[
        FB_INFO_INDEX_L2CACHE_SIZE,
        FB_INFO_INDEX_RAM_TYPE,
        FB_INFO_INDEX_BUS_WIDTH,
        FB_INFO_INDEX_FBP_COUNT,
    ]))
    .expect("served");
    let pairs = fbinfo::decode_fb_info_pairs(&fb).expect("decodes");
    let data_of = |idx: u32| pairs.iter().find(|&&(i, _)| i == idx).expect("present").1;

    assert_eq!(
        u64::from(data_of(FB_INFO_INDEX_L2CACHE_SIZE)),
        l2,
        "0x20801303's L2 size IS 0x20800a1c's, or this device tells RM two different \
         things about one cache"
    );
    assert_eq!(data_of(FB_INFO_INDEX_RAM_TYPE), ram_type);
    assert_eq!(data_of(FB_INFO_INDEX_BUS_WIDTH), ltc_count * 32);
    assert_eq!(data_of(FB_INFO_INDEX_FBP_COUNT), ltc_count / 2);

    // ⊘⊘ And the derivation that would have been WRONG, pinned against the same reply: a
    // real GA106 answers `LTS_COUNT = 18` while `ltcCount * ltsPerLtcCount` from this very
    // buffer is 24. `0x23` is therefore refused rather than projected — see
    // `kayfabe_abi::fbinfo`.
    let lts_per_ltc = get32(memsysconfig::LTS_PER_LTC_COUNT_OFF);
    assert_eq!(ltc_count * lts_per_ltc, 24);
    assert_ne!(
        ltc_count * lts_per_ltc,
        18,
        "the obvious projection of LTS_COUNT contradicts real hardware, which is why the \
         index is refused"
    );
}

/// ⊘ **The three indices the guest kernel answers itself must be REFUSED.** Serving `0x08`
/// would overwrite the guest's own correct `TOTAL_RAM_SIZE` — which `[measured]` our boot
/// already answers byte-identically to a real GA106 with no help from this port.
#[test]
fn the_guest_kernels_own_indices_are_refused_not_overwritten() {
    for idx in [
        FB_INFO_INDEX_TOTAL_RAM_SIZE,
        FB_INFO_INDEX_RAM_LOCATION,
        FB_INFO_INDEX_FB_IS_BROKEN,
    ] {
        let (status, _) = reply_params(&fb_command(&[idx])).expect("claimed, then refused");
        assert_ne!(status, 0, "index {idx:#x} must be refused, not answered");
    }
}

/// ★★★ §14.34's three, at the policy boundary and against the real GA106's own reply
/// (`traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:66` answers `{0x07, 6, 18}`).
///
/// ⊘ `LTS_COUNT` is the one to watch: it is a **projection of `l2_cache_size`**
/// (`l2 / 128 KiB`), not a stated literal, so this test and the `0x1b` one below cannot
/// disagree about the same silicon. The product `ltc x ltsPerLtc = 24` is pinned separately
/// as the value it must NOT be.
#[test]
fn the_second_walls_three_indices_are_answered_from_the_same_l2_row() {
    let want = [
        (FB_INFO_INDEX_FBP_MASK, 0x07u32),
        (FB_INFO_INDEX_LTC_COUNT, 6),
        (FB_INFO_INDEX_LTS_COUNT, 18),
    ];
    for (idx, value) in want {
        let (status, params) = reply_params(&fb_command(&[idx])).expect("served");
        assert_eq!(status, 0, "index {idx:#x}");
        assert_eq!(
            fbinfo::decode_fb_info_pairs(&params).unwrap()[0],
            (idx, value),
            "index {idx:#x}"
        );
    }
    // ★ And all three in one request, which is the shape libcuda actually sends.
    let (status, params) =
        reply_params(&fb_command(&[0x1a, 0x22, 0x23])).expect("served as a batch");
    assert_eq!(status, 0);
    assert_eq!(
        fbinfo::decode_fb_info_pairs(&params).unwrap(),
        want.to_vec()
    );
}

/// ⚠ **One unmeasured entry refuses the WHOLE call.** That is RM's shape — a single status
/// covers the request — and it is the property that makes a partial answer impossible.
#[test]
fn one_unmeasured_entry_refuses_the_whole_request() {
    // ⚠ `0x24` `L2CACHE_ONLY_MODE`, not `LTS_COUNT`: §14.34 serves that one, and a test
    // whose "unmeasured" index quietly became measured would keep passing while proving
    // nothing. The index used here must be one no rung has served.
    let (status, _) = reply_params(&fb_command(&[
        FB_INFO_INDEX_BUS_WIDTH,
        0x24,
        FB_INFO_INDEX_L2CACHE_SIZE,
    ]))
    .expect("claimed, then refused");
    assert_ne!(status, 0);
}

/// ⊘ **Guest-supplied counts and indices are bounds-checked at the policy boundary**, not
/// only in the unit tests — including the one that separates this control from
/// `BUS_GET_INFO_V2`: the legal index bound is `INDEX_MAX` (`0x3b`), which is 68 lower than
/// the `0x80` array length.
#[test]
fn a_malformed_request_is_refused_at_the_policy_boundary() {
    // `fbInfoListSize` past the array.
    let mut cmd = fb_command(&[FB_INFO_INDEX_BUS_WIDTH]);
    cmd.payload[PARAMS_AT..PARAMS_AT + 4]
        .copy_from_slice(&(FB_INFO_MAX_LIST_SIZE as u32 + 1).to_le_bytes());
    assert_ne!(reply_params(&cmd).expect("claimed").0, 0);

    // `fbInfoListSize == 0`.
    let mut cmd = fb_command(&[FB_INFO_INDEX_BUS_WIDTH]);
    cmd.payload[PARAMS_AT..PARAMS_AT + 4].copy_from_slice(&0u32.to_le_bytes());
    assert_ne!(reply_params(&cmd).expect("claimed").0, 0);

    // An index inside the array but past `INDEX_MAX`.
    assert_ne!(
        reply_params(&fb_command(&[FB_INFO_INDEX_MAX + 1]))
            .expect("claimed")
            .0,
        0
    );

    // A params body too short to hold the struct.
    let short = command(NV2080_CTRL_CMD_FB_GET_INFO_V2, &[0u8; 64]);
    assert!(
        reply_params(&short).is_none_or(|(status, _)| status != 0),
        "a short params body must never be answered NV_OK"
    );
}
