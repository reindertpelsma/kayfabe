//! `NV2080_CTRL_CMD_GSP_GET_FEATURES` (`0x20803601`) at the **reply-plane boundary**, and
//! against the committed real-GA106 capture.
//!
//! ## ⊘ What this file is for, beyond "the control is served"
//!
//! 1. That the served bytes equal a real GA106's, read out of the committed trace rather
//!    than a literal — all 72 of them, in one envelope, at the right params offset.
//! 2. ★★★ That the reply's `firmwareVersion` comes from the **guest's own fn 1** and from
//!    nowhere else. This is the claim that makes the rung correct, and the only way to test
//!    it is to drive fn 1 with a *different* string and watch the served reply follow — a
//!    fixed-value test would pass identically against the two wrong constant sources.
//! 3. ★★ That the cross-link seat holds: `InitTablePolicy` **reads** fn 1 without answering
//!    it. If that decline ever became an answer, `GuestSystemInfoPolicy` would stop seeing
//!    the version handshake and no test of this control would notice.
//! 4. ⚠ That an unlatched or unrepeatable guest string produces a **refusal**, not a
//!    default — the direction that costs a boot instead of inventing a value.

use kayfabe_abi::gspfeatures::{
    self, FirmwareVersion, GSP_GET_FEATURES_PARAMS_SIZE, GspFeatures,
    NV2080_CTRL_CMD_GSP_GET_FEATURES,
};
use kayfabe_abi::guestsysinfo::{
    GUEST_DRIVER_VERSION_OFF, SET_GUEST_SYSTEM_INFO_SIZE, VGX_MAJOR_OFF, VGX_MINOR_OFF,
};
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

/// A `SET_GUEST_SYSTEM_INFO` (fn 1) carrying `version` where RM puts `NV_VERSION_STRING`.
fn fn1(version: &str) -> RpcCommand {
    let mut payload = vec![0u8; SET_GUEST_SYSTEM_INFO_SIZE];
    // The two version words are what `GuestSystemInfoPolicy` reads; irrelevant here, but a
    // message with a plausible head is a better fixture than a zeroed one.
    payload[VGX_MAJOR_OFF..VGX_MAJOR_OFF + 4].copy_from_slice(&0x1fu32.to_le_bytes());
    payload[VGX_MINOR_OFF..VGX_MINOR_OFF + 4].copy_from_slice(&0x01u32.to_le_bytes());
    let v = version.as_bytes();
    payload[GUEST_DRIVER_VERSION_OFF..GUEST_DRIVER_VERSION_OFF + v.len()].copy_from_slice(v);
    RpcCommand {
        function: RpcFunction::SetGuestSystemInfo,
        code: 0x01,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn command(params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&0xc1d0_00bdu32.to_le_bytes());
    payload[4..8].copy_from_slice(&0x5c00_0003u32.to_le_bytes());
    payload[8..12].copy_from_slice(&NV2080_CTRL_CMD_GSP_GET_FEATURES.to_le_bytes());
    payload[16..20].copy_from_slice(&u32::try_from(params.len()).expect("fits").to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x23,
        sequence: 73,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// Drive fn 1 then the control through **one** policy, which is the ordering a boot has.
fn served(version: Option<&str>, params: &[u8]) -> Option<(u32, Vec<u8>)> {
    let mut p = policy();
    if let Some(v) = version {
        let handshake = fn1(v);
        // ★★ Claim 3, executed rather than commented: this link must DECLINE fn 1.
        assert!(
            p.respond(&handshake).is_none(),
            "InitTablePolicy must read the version handshake without answering it — an \
             answer here would starve GuestSystemInfoPolicy of the message it owns"
        );
    }
    let reply = p.respond(&command(params))?;
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
        reply.body[PARAMS_AT..PARAMS_AT + GSP_GET_FEATURES_PARAMS_SIZE].to_vec(),
    ))
}

/// The real GA106's `in=` / `out=` pair for this control, parsed out of the committed trace.
///
/// ⊘ Every step of the parse is asserted — a `find` that matches nothing returns nothing,
/// and a test comparing zero bytes to zero bytes is the `gate_read_through_grep_cannot_fail`
/// shape.
fn real_ga106() -> (Vec<u8>, Vec<u8>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is a committed artifact: {e}", path.display()));
    let needle = format!("cmd={NV2080_CTRL_CMD_GSP_GET_FEATURES:#010x}");
    let line = text
        .lines()
        .find(|l| l.contains(&needle) && l.contains("out="))
        .unwrap_or_else(|| panic!("no record for {needle} in {}", path.display()));
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
            GSP_GET_FEATURES_PARAMS_SIZE * 2,
            "{k} captured {} bytes, the struct is {GSP_GET_FEATURES_PARAMS_SIZE}",
            hex.len() / 2
        );
        (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("hex"))
            .collect()
    };
    (field("in="), field("out="))
}

/// ★★★ The rung: libcuda's own request, a `580.159.04` guest, the real GA106's own reply.
#[test]
fn libcudas_own_request_gets_the_real_ga106s_own_reply() {
    let (input, expected) = real_ga106();
    // ⊘ Non-vacuity: the captured request really is all-zero, so "the reply is constructed,
    // not edited" is a statement about this record and not an assumption.
    assert!(input.iter().all(|&b| b == 0), "the request is all-zero");
    let (status, params) = served(Some("580.159.04"), &input).expect("served");
    assert_eq!(status, 0, "a real GA106 answers NV_OK");
    assert_eq!(params, expected, "all {GSP_GET_FEATURES_PARAMS_SIZE} bytes");
}

/// ★★★ The claim the whole rung rests on: the version is the GUEST'S, latched off fn 1.
///
/// ⊘ This is the test the two wrong constant sources would fail, and the reason it drives a
/// version that is **not** the bench's: a fixture pinned to `580.159.04` passes identically
/// whether the value came off the wire, out of `host_driver`, or out of
/// `DriverAbiTable::version()` — which is exactly how a wrong source survives.
#[test]
fn the_firmware_version_follows_the_guest_and_not_any_constant() {
    for guest in ["580.159.04", "570.86.15", "610.43.02", "999.1.2"] {
        let (status, params) =
            served(Some(guest), &[0u8; GSP_GET_FEATURES_PARAMS_SIZE]).expect("served");
        assert_eq!(status, 0);
        let back = gspfeatures::decode_gsp_get_features(&params).expect("decodes");
        assert_eq!(
            back.firmware.as_str(),
            guest,
            "the reply must repeat the guest's own NV_VERSION_STRING"
        );
        // ...and the other three fields do NOT follow the guest — they are ours to state.
        assert_eq!(back.features, GspFeatures::GA106);
        assert!(back.valid);
        assert!(back.default_gsp_rm_gpu);
    }
    // ⊘ And the specific wrong constant, named: this policy's own ABI-table row.
    use kayfabe_abi::DriverAbi;
    let row = table_for(BENCH_DRIVER).expect("bench").version();
    let row_string = format!("{}.{}.{:02}", row.major, row.minor, row.patch);
    let (_, params) =
        served(Some("580.159.04"), &[0u8; GSP_GET_FEATURES_PARAMS_SIZE]).expect("served");
    let back = gspfeatures::decode_gsp_get_features(&params).expect("decodes");
    assert_ne!(
        back.firmware.as_str(),
        row_string,
        "serving DriverAbiTable::version() would answer {row_string} where hardware says \
         580.159.04"
    );
}

/// ⚠ No fn 1 yet ⇒ a refusal, never a default.
#[test]
fn an_unlatched_version_refuses_rather_than_inventing_one() {
    let (status, params) = served(None, &[0u8; GSP_GET_FEATURES_PARAMS_SIZE]).expect("answered");
    assert_ne!(status, 0, "NV_ERR_NOT_SUPPORTED, not a made-up version");
    assert!(params.is_empty(), "a refusal carries no body");
}

/// ⚠ A guest string this port will not repeat leaves the latch empty — so the control
/// refuses rather than serving an unvalidated buffer that `nvidia-smi` would print.
#[test]
fn a_hostile_guest_string_costs_a_refusal_and_not_a_pass_through() {
    for hostile in ["", "580\u{1}159", &"5".repeat(200)] {
        let (status, params) =
            served(Some(hostile), &[0u8; GSP_GET_FEATURES_PARAMS_SIZE]).expect("answered");
        assert_ne!(status, 0, "{hostile:?} must not reach the reply");
        assert!(params.is_empty());
    }
    // ★ And the falsifier for that claim: a *valid* unusual string IS repeated, so the
    // refusals above are the validator biting and not the latch being broken.
    let (status, params) =
        served(Some("1.2.3"), &[0u8; GSP_GET_FEATURES_PARAMS_SIZE]).expect("served");
    assert_eq!(status, 0);
    let back = gspfeatures::decode_gsp_get_features(&params).expect("decodes");
    assert_eq!(back.firmware.as_str(), "1.2.3");
}

/// ⊘ The guest's own declared size must be the one we encode.
#[test]
fn a_size_the_guest_declares_wrong_is_refused() {
    let mut p = policy();
    assert!(p.respond(&fn1("580.159.04")).is_none());
    for wrong in [0usize, 8, GSP_GET_FEATURES_PARAMS_SIZE - 1] {
        let cmd = command(&vec![0u8; wrong]);
        let reply = p.respond(&cmd).expect("answered");
        assert_ne!(
            reply.rpc_result, 0,
            "a {wrong}-byte params_size is not this struct"
        );
    }
}

/// The id classifies to the variant, and the variant states this struct's size.
#[test]
fn the_id_and_the_size_are_one_statement() {
    assert_eq!(
        WantedTable::from_cmd(NV2080_CTRL_CMD_GSP_GET_FEATURES),
        Some(WantedTable::GspGetFeatures)
    );
    assert_eq!(
        WantedTable::GspGetFeatures.params_size(),
        GSP_GET_FEATURES_PARAMS_SIZE
    );
    // ⚠ 72, and the trace declares 72 — the two are checked against each other rather than
    // both against a literal.
    let (input, _) = real_ga106();
    assert_eq!(input.len(), WantedTable::GspGetFeatures.params_size());
}

/// ★★ The latch is observable, so "not yet seen" and "seen and refused" are distinguishable
/// from outside — the distinction the reply plane collapses into one refusal.
#[test]
fn the_latch_names_what_it_observed() {
    let mut p = policy();
    assert_eq!(p.guest_firmware(), None, "nothing observed before fn 1");
    assert!(p.respond(&fn1("580.159.04")).is_none());
    assert_eq!(
        p.guest_firmware(),
        Some(FirmwareVersion::parse("580.159.04").expect("parses"))
    );
    // ⊘ Non-vacuity for the fixture itself: `"bad\0…"` is a NUL-terminated C string whose
    // value is `"bad"`, so it LATCHES. Asserting it were rejected would have been a test
    // passing on a wrong premise — the field is `char[0x100]`, and a NUL is its terminator
    // rather than a hostile byte.
    assert!(p.respond(&fn1("bad\u{0}rest")).is_none());
    assert_eq!(
        p.guest_firmware(),
        Some(FirmwareVersion::parse("bad").expect("parses"))
    );
}

/// ★★★ The most recent handshake wins, **including when it fails** — the defect my own
/// first draft had.
///
/// ⊘ That draft skipped the write when the message did not decode and cleared the latch
/// when the string did not parse, so two failure modes of one message behaved differently
/// and a stale version could have been reported after the guest replaced it. Both now land
/// on `None`, which makes the reply plane refuse rather than repeat something the guest is
/// no longer saying.
#[test]
fn the_latch_always_reflects_the_most_recent_handshake() {
    let good = FirmwareVersion::parse("580.159.04").expect("parses");
    // Failure mode 1: a string that decodes and will not be repeated (empty).
    let mut p = policy();
    assert!(p.respond(&fn1("580.159.04")).is_none());
    assert_eq!(p.guest_firmware(), Some(good));
    assert!(p.respond(&fn1("")).is_none());
    assert_eq!(
        p.guest_firmware(),
        None,
        "an unrepeatable string must clear the latch, not leave a stale one"
    );

    // Failure mode 2: a message that does not decode at all (too short for the struct).
    let mut p = policy();
    assert!(p.respond(&fn1("580.159.04")).is_none());
    assert_eq!(p.guest_firmware(), Some(good));
    let mut truncated = fn1("580.159.04");
    truncated.payload.truncate(8);
    assert!(p.respond(&truncated).is_none());
    assert_eq!(
        p.guest_firmware(),
        None,
        "an undecodable handshake must clear the latch too — the two failure modes of one \
         message must not behave differently"
    );

    // ...and the control refuses in both cases rather than serving the stale value.
    let reply = p
        .respond(&command(&[0u8; GSP_GET_FEATURES_PARAMS_SIZE]))
        .expect("answered");
    assert_ne!(reply.rpc_result, 0);
}
