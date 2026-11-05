//! `NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP` (0x20800a41): the register
//! permission policy this port serves, and the one encoding of it that means the opposite.
//!
//! ## ★★ Where the oracle bytes come from
//!
//! `C: src/qemu/mode2_initctrl_ga106.h:3363` declares `ctl_20800a41[]`, registered at
//! `:6249` as `{0x20800a41u, 0x0u, 8204u, 8200u, ctl_20800a41}` — `psize` 8204 with 8200
//! captured, the four trailing zeros trimmed by the capture's own dedup. Those bytes
//! reconstruct **verbatim** out of `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6`:
//! records **142000-142002**, three consecutive 4096-byte `GUEST_WR`s starting at gpa
//! `0x1_2764_7000`, the params at payload offset **120** of the first, under an RPC envelope
//! reading `header_version=0x03000000 signature="VRPC" length=8276 function=76
//! rpc_result=NV_OK sequence=0` and a control header carrying `hClient=0xc2000006
//! hObject=0xabcd2080 cmd=0x20800a41 status=NV_OK paramsSize=8204 rmapiRpcFlags=0`.
//! `8276 = 32 + 40 + 8204` — envelope, `RpcControlReq::HEADER`, params.
//!
//! A `GUEST_WR` is the device writing into the guest's status queue, so this is the C's
//! *reply*. ★ It is also the first reply that spans more than one queue element, which is
//! the same fact the C's own `nvkvm_gpu_emul.c:3545-3554` had to learn, naming this command.
//!
//! [`ORACLE_PREFIX`] carries the first **1277** bytes — the 8-byte header and the whole
//! 1269-byte gzip stream. `[measured]`: every one of the remaining 6927 bytes of the
//! reconstructed 8204 is zero, which is what [`oracle_params`] rebuilds.
//!
//! ## ★★ What this file settles, and what it cannot
//!
//! It settles the **layout** — `compressedData` at 8, no padding, the struct 8204 long —
//! and it settles it against silicon rather than against a header read.
//!
//! ⊘ It cannot settle the two `profilingRanges*` offsets on bytes alone, because the
//! oracle's board published **zero** profiling ranges and every byte from 1277 on is zero:
//! an encoder that placed `profilingRangesSize` anywhere in that run would match the capture
//! exactly. [`the_tail_layout_is_settled_by_arithmetic_not_by_the_capture`] closes that with
//! the only argument available — the two array bounds and the captured `paramsSize` leave
//! exactly one place to put them.
//!
//! ⊘ And it settles **nothing about which bits to set**. The oracle's board permitted
//! 6809 ranges; this device publishes no map at all, for the reason
//! [`kayfabe_device::ga10x::GA106_USER_REGISTER_ACCESS_MAP`] states.

use kayfabe_abi::chipinfo::reg_base;
use kayfabe_abi::regaccessmap::{
    self, GZIP_HEADER_SKIP, MAX_COMPRESSED_SIZE, MAX_PROFILING_RANGES,
    NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP, ProfilingRange,
    RegisterAccessMapError, RegisterAccessMapRow, USER_REGISTER_ACCESS_MAP_PARAMS_SIZE,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::ChipProfile;
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// The first 1277 bytes an RTX 3060's GSP answered: `userRegisterAccessMapSize`,
/// `compressedSize`, and the whole gzip member. Everything after is zero.
const ORACLE_PREFIX: &str = concat!(
    "00000800f50400001f8b08003ae2bb6502ffeddd4b6ed3401c07e071923e4054",
    "356a2b58201138011b5456249760cd459070978803b0850de760db1547e00a15",
    "ebc290b46e5af26893da491cfbfba43c067782677ef377dc86e210e69286a56a",
    "97f64a49a0ccf94a6a3edf4d1f7fd53d36c71baf6f0a00002ae6fadbfbb86336",
    "fe73f62599f6fdc7dfe7a606000000000000000080f5f1cfd8000080312f4c01",
    "75b46b0a000000000000000000000000008086f99184d0320dccd699e31e96bd",
    "026f939a2400000000008010bac3dbf6ec5b13f4abbe83533efb4a4e86f76969",
    "6de3dfb0f16763e329d8367ee3377ee3377ee3377ee3377e000000006025a6fc",
    "fcefdba7e17d1ab2bc5de7cf7f7bbd187fc5787af4447b196df9cb5ffef297bf",
    "fce52f7ff9cb5ffef2af9c43eda5b6e52f7ff9cb5ffef297bffce52f7f000058",
    "870a5c6321910200000000000014e6f76be42f7ff9cb5ffef297bffce52fff06",
    "f2fb35f297bffce52f7ff9cb5ffe000000b5f27af42c5d49bb69e3dfb4fc9b66",
    "2f5beceb8f3ec781f7efe27ee6e0410d0c3f4ffd737ad49eb7bdeafdb97dfbf1",
    "d8f1ebb8e1c7b3cdcf7f31f297bffce52ffffba9cff536d23bdaabd91ff5affe",
    "e5bfbefc018039dfff93f31be7afdae5b6e52f7ff9cb5ffef297bffce52f7ff9",
    "cb1f000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000803224a6a0defad9ec4d955e9707178f9d89",
    "759acef7026fd7bbff69c1fe9da23b905520c44ee14550e6f1ea9583010075b7",
    "75cbb6767ebbf966daca9b2ff3f6c9e004a6953e9b388b68e58fbff72fdfdeaf",
    "5e62ab1b63efc65bfe83c223e85efd55213e8cadedfca4aaf5f53084832cece6",
    "dbf6ae4f355af1c3e06bf35b785ac2f9c74e81bdbfb8df1d9d8cddd59ed23fed",
    "2ed07e33f17ad75b87f3777552dacae69c3ffd0bf60700000000000000000000",
    "0068961877f2c7a1f694ed977f3e6bbbf92b367ffa5b7f000000000000000000",
    "000000000040f94617d69c72fdcf6edecec262d7ffec8c3d9670fdcfd1b37b5d",
    "7fb161d7ff9c7cbd82f3a77fc1fe75a8ff4741fdab7ffdd5bffa57fffaab7ff5",
    "affef557ffea5ffdebaffed5bffad75ffdab7ff5afff66d43f00000000000000",
    "00c02ab9fefa7ae74f7feb4ffdab7ffdad3ff5affef5b7fed4bffad7dffa53ff",
    "ea5f7feb4ffdab7ffdad3ff5affef50700000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "00000000000000000000000000000000000000000000000000000080f5f91e07",
    "7a8327fd701e7fc68f751c638c9737000000a8b48e2900f55fa6ccbcca1f0000",
    "00000000000000000000000000000000000000a0b11253207fe48ffc913ff207",
    "0000000000000000000000000000000060e9f2ffffa56d260000000000000000",
    "0000000000d824ed35f707d43fa0fe01f50fa87f000000000000000000000000",
    "000000000000000000000000000000000000000000000000000028cd598cfb66",
    "0100000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "00000000000000000000000000000000000098d33fd5c8167600000800",
);

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "hex must be whole bytes");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

/// The oracle's whole 8204-byte reply: the captured prefix, zero-extended.
fn oracle_params() -> Vec<u8> {
    let mut b = unhex(ORACLE_PREFIX);
    assert_eq!(b.len(), 1277, "the captured non-zero prefix");
    b.resize(8204, 0); // literal — the capture's own `paramsSize`
    b
}

/// The deepest byte of the C capture this file's argument rests on: `profilingRangesSize`,
/// at 4104, whose value this file quotes as zero.
///
/// ★ `0x20800a41`'s row is TRUNCATED — `dlen` 8200 of `psize` 8204 — and the four bytes it
/// does not carry are `profilingRanges[4092..4096]`, which a `profilingRangesSize` of zero
/// puts out of reach of any reader. Stated here so it is checked rather than believed.
const ORACLE_DEEPEST_BYTE: usize = 4104 + 4;

/// ★★★ Nothing this file reads of the oracle's reply is missing from the capture.
#[test]
fn every_oracle_byte_this_file_reads_is_inside_what_the_recorder_kept() {
    let cmd = NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP;
    let row = kayfabe_abi::oracle::truncated_row(cmd).expect("0x20800a41 is a truncated row");
    let r = kayfabe_abi::oracle::capture_reliance(cmd).expect("and it carries a reliance");
    assert_eq!(
        r.read_end, ORACLE_DEEPEST_BYTE,
        "this file and kayfabe_abi::oracle must agree on how deep the argument reaches"
    );
    assert!(
        kayfabe_abi::oracle::field_is_captured(0, ORACLE_DEEPEST_BYTE, row.kept),
        "reads [0,{ORACLE_DEEPEST_BYTE}) of a capture that kept {} of {}",
        row.kept,
        row.psize
    );
    assert!(unhex(ORACLE_PREFIX).len() <= ORACLE_DEEPEST_BYTE);
    // ⊘ The zero-extension in `oracle_params` is THIS FILE's, not the recorder's.
    assert!(!kayfabe_abi::oracle::field_is_captured(
        0, row.psize, row.kept
    ));
}

/// The oracle's gzip stream on its own — `compressedData[..compressedSize]`.
///
/// ★ Sliced at LITERAL 8 and 1269, not at the constants under test.
fn oracle_bitmap() -> &'static [u8] {
    Box::leak(oracle_params()[8..8 + 1269].to_vec().into_boxed_slice())
}

/// A row that publishes exactly what the oracle's board published.
///
/// ⊘ Not a row this port would ever ship — see the module docs and
/// [`kayfabe_device::ga10x::GA106_USER_REGISTER_ACCESS_MAP`]. It exists so the encoder can
/// be driven over the *whole* struct, including the fields the served row leaves zero, and
/// checked against silicon.
fn oracle_row() -> RegisterAccessMapRow {
    RegisterAccessMapRow {
        map_size_bytes: 0x0008_0000,
        compressed: oracle_bitmap(),
        profiling_ranges: &[],
    }
}

fn chip() -> &'static ChipProfile {
    kayfabe_device::chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// `RpcControlReq::HEADER`, as the capture's own arithmetic gives it: the reply's declared
/// `length` was 8276 and `8276 - 32 - 8204 = 40`.
const PARAMS_AT: usize = 40;

/// A `GSP_RM_CONTROL` whose header asks for `cmd` with `params_size` bytes of params.
///
/// ★★ The request body is 0xAA, not zeros. That matters more here than anywhere: the
/// caller `portMemSet`s its params to zero before sending (`ogkm-580:
/// gpu_register_access_map.c:242`), so a reply that reflected the request would be
/// indistinguishable from a correct one on the bench and wrong on any guest that did not.
fn map_command(cmd: u32, params_size: u32, params_at: usize) -> RpcCommand {
    let mut payload = vec![0xAAu8; params_at + params_size as usize];
    payload[0..4].copy_from_slice(&0xc200_0006u32.to_le_bytes()); // hClient, as captured
    payload[4..8].copy_from_slice(&0xabcd_2080u32.to_le_bytes()); // hObject, as captured
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // status
    payload[16..20].copy_from_slice(&params_size.to_le_bytes());
    payload[20..24].copy_from_slice(&0u32.to_le_bytes()); // rmapiRpcFlags: flat, not FINN
    payload[24..40].fill(0);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 0,
        payload,
        elements: 3,
        delivered: Vec::new(),
    }
}

// ── The layout, against silicon ────────────────────────────────────────────────────

#[test]
fn the_encoder_reproduces_the_oracles_layout_byte_for_byte() {
    let got = regaccessmap::encode_user_register_access_map(&oracle_row())
        .expect("a row that publishes the oracle's own map encodes");

    // ★★★ LITERALS. Deriving these from `MAP_SIZE_OFF`/`COMPRESSED_DATA_OFF` would move
    // the check and the encoder together — the defect that made #125's and #126's own
    // tests pass while measuring nothing.
    assert_eq!(got.len(), 8204, "sizeof the params struct");
    assert_eq!(
        &got[0..4],
        &0x0008_0000u32.to_le_bytes()[..],
        "userRegisterAccessMapSize — 512 KiB, one bit per dword of a 16 MiB BAR0"
    );
    assert_eq!(
        &got[4..8],
        &1269u32.to_le_bytes()[..],
        "compressedSize — the gzip member's length"
    );
    assert_eq!(
        &got[8..11],
        &[0x1f, 0x8b, 0x08],
        "compressedData starts immediately at 8, with no padding, and is gzip-framed"
    );
    assert_eq!(
        &got[4104..4108],
        &0u32.to_le_bytes()[..],
        "profilingRangesSize — the oracle's board published none"
    );

    assert_eq!(got, oracle_params(), "vs cap1b records 142000-142002");
}

#[test]
fn the_tail_layout_is_settled_by_arithmetic_not_by_the_capture() {
    // ⊘ The capture cannot adjudicate where `profilingRangesSize` sits: it is zero there
    // and so is every byte around it. What does adjudicate it is the pair of array bounds
    // in `ogkm-580: ctrl2080internal.h:774-775` together with the captured `paramsSize`.
    //
    // Two `NvU32`s, then `NvU8[4096]`, then an `NvU32`, then `NvU8[4096]`; all the
    // alignments are already satisfied, so there is exactly one packing — and it lands on
    // the number the capture DID state.
    assert_eq!(MAX_COMPRESSED_SIZE, 4096);
    assert_eq!(MAX_PROFILING_RANGES, 4096);
    assert_eq!(4 + 4 + 4096 + 4 + 4096, 8204, "the captured paramsSize");
    assert_eq!(USER_REGISTER_ACCESS_MAP_PARAMS_SIZE, 8204);

    // And the encoder really writes the ranges where that arithmetic puts them: a row with
    // one range must place its two words at 4108 and 4112, which is a position the oracle
    // could never have shown.
    static ONE: &[ProfilingRange] = &[ProfilingRange {
        offset: 0x0000_1000,
        size: 0x0000_0040,
    }];
    let got = regaccessmap::encode_user_register_access_map(&RegisterAccessMapRow {
        map_size_bytes: 0x0008_0000,
        compressed: oracle_bitmap(),
        profiling_ranges: ONE,
    })
    .expect("one in-range, aligned range encodes");
    assert_eq!(
        &got[4104..4108],
        &8u32.to_le_bytes()[..],
        "profilingRangesSize is in BYTES of the flat NvU32 array (`ogkm-580: :106-110`)"
    );
    assert_eq!(&got[4108..4112], &0x0000_1000u32.to_le_bytes()[..]);
    assert_eq!(&got[4112..4116], &0x0000_0040u32.to_le_bytes()[..]);
}

// ── What this device actually serves ───────────────────────────────────────────────

#[test]
fn this_device_publishes_no_map_and_that_is_rms_own_unsupported() {
    let got = regaccessmap::encode_user_register_access_map(&chip().user_register_access_map)
        .expect("the GA106 row encodes");

    assert_eq!(got.len(), 8204);
    // ★★★ `userRegisterAccessMapSize == 0` is the whole statement: RM logs "User Register
    // Access Map unsupported for this chip", returns NV_OK, and leaves
    // `pUserRegisterAccessMap` NULL so every non-admin regop is refused
    // (`ogkm-580: gpu_register_access_map.c:141-152, 261-267`).
    assert_eq!(
        &got[0..4],
        &0u32.to_le_bytes()[..],
        "userRegisterAccessMapSize"
    );
    assert_eq!(&got[4..8], &0u32.to_le_bytes()[..], "compressedSize");
    assert_eq!(
        &got[4104..4108],
        &0u32.to_le_bytes()[..],
        "profilingRangesSize"
    );
    assert!(
        got.iter().all(|&b| b == 0),
        "the reply is zeros end to end — and it is a claim, not an omission"
    );
    assert!(!chip().user_register_access_map.publishes_map());
}

#[test]
fn no_reply_this_port_sends_advertises_the_timer_block_it_does_not_serve() {
    // ★★ The cross-check between the two rungs. `GA106_REG_BASES` refuses
    // `NV_REG_BASE_TIMER` because its readers hand clients `0x9400`, which this device does
    // not serve. `[measured]` — the oracle's own bitmap permits `0x9400`-`0x9404` outright:
    // inflating the 1269-byte gzip member of `ORACLE_PREFIX` (itself reconstructed from
    // `traces/mode2_c_reference/cap1b_coldboot_hermetic_d6` records 142000-142002, see this
    // file's docs) yields 524 288 bytes whose set bits are 6809 ranges over 37 743
    // registers, and `0x9400/4` is one of them. So publishing it here would re-grant by
    // regop exactly what
    // the reg-base table declined — one layer lower, and with no per-entry refusal
    // vocabulary to decline it in.
    assert!(
        !chip()
            .chip_info
            .reg_bases
            .iter()
            .any(|b| b.index == reg_base::TIMER),
        "the chip-info table declines NV_REG_BASE_TIMER"
    );
    assert!(
        !chip().user_register_access_map.publishes_map(),
        "and the access map grants nothing, so no path advertises 0x9400"
    );
}

// ── The policy ─────────────────────────────────────────────────────────────────────

#[test]
fn the_policy_answers_the_control_without_reflecting_one_byte_of_the_request() {
    let mut p = policy();
    let cmd = map_command(
        NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP,
        8204,
        PARAMS_AT,
    );
    let reply = p.respond(&cmd).expect("the policy answers 0x20800a41");

    assert_eq!(reply.rpc_result, 0, "NV_OK in the envelope");
    // Literal offsets into the control header: `status` at 12, `paramsSize` at 16.
    assert_eq!(&reply.body[12..16], &0u32.to_le_bytes()[..], "status");
    assert_eq!(
        &reply.body[16..20],
        &8204u32.to_le_bytes()[..],
        "paramsSize"
    );
    assert_eq!(&reply.body[8..12], &0x2080_0a41u32.to_le_bytes()[..], "cmd");

    // ★★★ THE BITE. A surviving 0xAA in the first word would be
    // `userRegisterAccessMapSize = 0xAAAAAAAA` with `compressedSize = 0xAAAAAAAA` behind
    // it — a 2.7 GiB paged allocation, and, if it succeeded, an inflate over 4096 bytes of
    // request garbage. A surviving 0xAA with a *zeroed* size word would be 6927 bytes of
    // profiling ranges RM would then fail the whole control on.
    let params = &reply.body[PARAMS_AT..PARAMS_AT + 8204];
    assert!(
        !params.contains(&0xAA),
        "the reply carries bytes from the guest's request"
    );
    assert!(params.iter().all(|&b| b == 0));
}

#[test]
fn a_declared_params_size_that_is_not_ours_is_refused_rather_than_answered() {
    let mut p = policy();
    let reply = p
        .respond(&map_command(
            NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP,
            8203,
            PARAMS_AT,
        ))
        .expect("a refusal is still a reply");
    assert_eq!(reply.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
    assert!(reply.body.is_empty(), "a refusal carries no body");
}

#[test]
fn a_serialized_request_is_refused_rather_than_answered_flat() {
    let mut p = policy();
    let mut cmd = map_command(
        NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP,
        8204,
        PARAMS_AT,
    );
    let flags: u32 = 0x0000_0002; // `RMAPI_RPC_FLAGS_SERIALIZED` = `NVBIT(1)`
    cmd.payload[20..24].copy_from_slice(&flags.to_le_bytes());
    let reply = p.respond(&cmd).expect("a refusal is still a reply");
    assert_eq!(reply.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
}

#[test]
fn the_classifier_names_this_control_and_its_size() {
    assert_eq!(
        WantedTable::from_cmd(0x2080_0a41),
        Some(WantedTable::UserRegisterAccessMap)
    );
    assert_eq!(WantedTable::UserRegisterAccessMap.params_size(), 8204);
    assert_eq!(
        NV2080_CTRL_CMD_INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP,
        0x2080_0a41
    );
}

// ── The encoder's refusals, each induced ───────────────────────────────────────────

/// The one that matters. ★★★ A non-zero size with an empty bitmap is not "no policy" — it
/// sends `gpuConstructUserRegisterAccessMap_IMPL` down `portMemSet(…, 0xFF, …)`
/// (`ogkm-580: gpu_register_access_map.c:286-291`) and **opens all 16 MiB of BAR0 to
/// unprivileged guest userspace**.
///
/// ⊘ Note what the bad row is: [`RegisterAccessMapRow::NOT_PUBLISHED`] with one field
/// changed. That is the whole reason this refusal exists rather than a comment.
#[test]
fn a_map_size_with_no_bitmap_is_refused_because_rm_reads_it_as_allow_everything() {
    let bad = RegisterAccessMapRow {
        map_size_bytes: 0x0008_0000,
        ..RegisterAccessMapRow::NOT_PUBLISHED
    };
    assert_eq!(
        regaccessmap::encode_user_register_access_map(&bad),
        Err(RegisterAccessMapError::MapPublishedWithoutBitmap {
            map_size: 0x0008_0000
        })
    );
    // And the row it was derived from encodes, so the refusal is about the change and not
    // about the row.
    assert!(
        regaccessmap::encode_user_register_access_map(&RegisterAccessMapRow::NOT_PUBLISHED).is_ok()
    );
}

#[test]
fn a_bitmap_with_no_map_size_is_refused_because_rm_never_inflates_it() {
    let bad = RegisterAccessMapRow {
        map_size_bytes: 0,
        compressed: oracle_bitmap(),
        profiling_ranges: &[],
    };
    assert_eq!(
        regaccessmap::encode_user_register_access_map(&bad),
        Err(RegisterAccessMapError::BitmapWithoutMap {
            compressed_len: 1269
        })
    );
}

#[test]
fn a_map_size_rm_would_round_up_is_refused() {
    let bad = RegisterAccessMapRow {
        map_size_bytes: 0x0008_0001,
        compressed: oracle_bitmap(),
        profiling_ranges: &[],
    };
    assert_eq!(
        regaccessmap::encode_user_register_access_map(&bad),
        Err(RegisterAccessMapError::MapSizeNotDwordAligned {
            map_size: 0x0008_0001
        })
    );
}

#[test]
fn a_bitmap_past_the_arrays_end_is_refused() {
    // 4097 bytes: one past `compressedData[]`. Gzip-framed, so this is the size check and
    // not the framing one.
    let mut big = vec![0u8; 4097];
    big[0..3].copy_from_slice(&[0x1f, 0x8b, 0x08]);
    let bad = RegisterAccessMapRow {
        map_size_bytes: 0x0008_0000,
        compressed: Box::leak(big.into_boxed_slice()),
        profiling_ranges: &[],
    };
    assert_eq!(
        regaccessmap::encode_user_register_access_map(&bad),
        Err(RegisterAccessMapError::BitmapTooLarge {
            len: 4097,
            max: 4096
        })
    );
}

#[test]
fn a_bitmap_that_is_not_gzip_framed_is_refused_because_rm_does_not_check() {
    // ★ RM does `pComprData += 10` unconditionally (`ogkm-580: :359-365`). A raw deflate
    // stream is not rejected there — it is inflated ten bytes in, silently.
    let raw: &'static [u8] = Box::leak(
        oracle_params()[8 + GZIP_HEADER_SKIP..8 + 64]
            .to_vec()
            .into_boxed_slice(),
    );
    let bad = RegisterAccessMapRow {
        map_size_bytes: 0x0008_0000,
        compressed: raw,
        profiling_ranges: &[],
    };
    assert_eq!(
        regaccessmap::encode_user_register_access_map(&bad),
        Err(RegisterAccessMapError::BitmapNotGzipFramed)
    );
    // A stream shorter than the framing RM skips is the same refusal, for the same reason.
    let stub: &'static [u8] = &[0x1f, 0x8b, 0x08, 0x00];
    assert_eq!(
        regaccessmap::encode_user_register_access_map(&RegisterAccessMapRow {
            map_size_bytes: 0x0008_0000,
            compressed: stub,
            profiling_ranges: &[],
        }),
        Err(RegisterAccessMapError::BitmapNotGzipFramed)
    );
}

#[test]
fn more_profiling_ranges_than_the_array_holds_are_refused() {
    // 513 pairs = 4104 bytes, one pair past `profilingRanges[]`.
    let many: Vec<ProfilingRange> = (0..513)
        .map(|i| ProfilingRange {
            offset: i * 4,
            size: 4,
        })
        .collect();
    let bad = RegisterAccessMapRow {
        map_size_bytes: 0x0008_0000,
        compressed: oracle_bitmap(),
        profiling_ranges: Box::leak(many.into_boxed_slice()),
    };
    assert_eq!(
        regaccessmap::encode_user_register_access_map(&bad),
        Err(RegisterAccessMapError::ProfilingRangesTooLarge {
            bytes: 4104,
            max: 4096
        })
    );
}

#[test]
fn profiling_ranges_without_a_published_map_are_refused() {
    static ONE: &[ProfilingRange] = &[ProfilingRange {
        offset: 0x9400,
        size: 4,
    }];
    let bad = RegisterAccessMapRow {
        profiling_ranges: ONE,
        ..RegisterAccessMapRow::NOT_PUBLISHED
    };
    assert_eq!(
        regaccessmap::encode_user_register_access_map(&bad),
        Err(RegisterAccessMapError::ProfilingRangeWithoutMap { count: 1 })
    );
}

#[test]
fn a_misaligned_profiling_range_is_refused_because_it_would_end_rminitnvdevice() {
    // ★★ `gpuSetUserRegisterAccessPermissions_IMPL` `NV_ASSERT_OR_RETURN`s both alignments
    // (`ogkm-580: :51-52`); the failure propagates through the bulk call to
    // `gpuConstructUserRegisterAccessMap`'s `done:` with a non-NV_OK status (`:313-319`),
    // which is the same line this whole rung is about.
    static BAD_OFF: &[ProfilingRange] = &[ProfilingRange {
        offset: 0x9402,
        size: 4,
    }];
    assert_eq!(
        regaccessmap::encode_user_register_access_map(&RegisterAccessMapRow {
            map_size_bytes: 0x0008_0000,
            compressed: oracle_bitmap(),
            profiling_ranges: BAD_OFF,
        }),
        Err(RegisterAccessMapError::ProfilingRangeUnaligned {
            offset: 0x9402,
            size: 4
        })
    );
    static BAD_SIZE: &[ProfilingRange] = &[ProfilingRange {
        offset: 0x9400,
        size: 6,
    }];
    assert_eq!(
        regaccessmap::encode_user_register_access_map(&RegisterAccessMapRow {
            map_size_bytes: 0x0008_0000,
            compressed: oracle_bitmap(),
            profiling_ranges: BAD_SIZE,
        }),
        Err(RegisterAccessMapError::ProfilingRangeUnaligned {
            offset: 0x9400,
            size: 6
        })
    );
}

#[test]
fn a_profiling_range_past_the_end_of_the_map_is_refused() {
    // A 512 KiB bitmap covers 512 KiB * 8 bits * 4 bytes = 16 MiB of register space. This
    // range's last dword is one past it, which is `NV_ERR_INVALID_ARGUMENT` at
    // `ogkm-580: :64-65` and fails the whole control.
    static PAST: &[ProfilingRange] = &[ProfilingRange {
        offset: 0x00FF_FFFC,
        size: 8,
    }];
    assert_eq!(
        regaccessmap::encode_user_register_access_map(&RegisterAccessMapRow {
            map_size_bytes: 0x0008_0000,
            compressed: oracle_bitmap(),
            profiling_ranges: PAST,
        }),
        Err(RegisterAccessMapError::ProfilingRangeOutsideMap {
            offset: 0x00FF_FFFC,
            size: 8,
            map_size: 0x0008_0000
        })
    );
    // The same range one dword earlier ends exactly at the boundary, and encodes.
    static EDGE: &[ProfilingRange] = &[ProfilingRange {
        offset: 0x00FF_FFF8,
        size: 8,
    }];
    assert!(
        regaccessmap::encode_user_register_access_map(&RegisterAccessMapRow {
            map_size_bytes: 0x0008_0000,
            compressed: oracle_bitmap(),
            profiling_ranges: EDGE,
        })
        .is_ok()
    );
}
