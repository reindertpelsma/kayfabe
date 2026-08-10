//! The two controls the **host↔guest ioctl differential** named, at the reply-plane boundary.
//!
//! ## What the differential measured, and what it got wrong
//!
//! `[measured 2026-08-10, nvdiff]` a real GA106 running a staged CUDA program issues exactly
//! **one** non-`NV_OK` status in 613 ioctls (`0x2080012f GPU_QUERY_ECC_STATUS`, inside
//! `cuInit`). Our Mode-2 guest running the *same instrument and the same workload* issued
//! **three**, so the worklist was two ids:
//!
//! | id | guest | hardware | verdict |
//! |---|---|---|---|
//! | `0x20810108` (`NV2081_BINAPI`) | `0x56` | `NV_OK` | reaches this port — served here |
//! | `0x2080200a` `PERF_BOOST` | `0x56` | `NV_OK` | ⊘ **never reaches this port** |
//!
//! ⊘⊘ **The second row is the finding.** `0x2080200a` has a kernel-side implementation
//! (`subdeviceCtrlCmdKPerfBoost_IMPL`); the guest's own RM answers it and re-packages its two
//! fields under `NV2080_CTRL_CMD_INTERNAL_PERF_BOOST_SET_2X` (`0x20800a9a`) for physical RM.
//! `[measured 2026-08-10]` `0x2080200a` appears in **zero** committed device logs and
//! `0x20800a9a` in **38**. An arm for the userspace id would have compiled and unit-tested
//! green while serving nothing — which is why this file asserts the **routing** as well as the
//! replies. See [`kayfabe_abi::perfboost`].
//!
//! ★ This is the boundary `bus_get_c2c_info.rs` exists at, for its reason: a correct encoder
//! reached with the wrong argument — or keyed on an id the wire never carries — is
//! indistinguishable from an unimplemented one at every gate except a live boot.

use kayfabe_abi::binapictl::{BINAPI_CTRL_0X0108, BINAPI_CTRL_0X0108_PARAMS_SIZE};
use kayfabe_abi::perfboost::{INTERNAL_PERF_BOOST_SET_2X, INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::inittables::{InitTablePolicy, WantedTable};
use kayfabe_device::{ChipProfile, chip_for_device_id};
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

const PARAMS_AT: usize = 40;
const CONTROL_STATUS_OFF: usize = 12;
const CONTROL_PARAMS_SIZE_OFF: usize = 16;

fn chip() -> &'static ChipProfile {
    chip_for_device_id(0x2504).expect("GA106 is in the table")
}

fn command(cmd: u32, params: &[u8]) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + params.len()];
    payload[0..4].copy_from_slice(&0xc1d0_000cu32.to_le_bytes());
    payload[4..8].copy_from_slice(&0x5c00_0004u32.to_le_bytes());
    payload[8..12].copy_from_slice(&cmd.to_le_bytes());
    payload[CONTROL_PARAMS_SIZE_OFF..CONTROL_PARAMS_SIZE_OFF + 4]
        .copy_from_slice(&u32::try_from(params.len()).expect("fits").to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(params);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x23,
        sequence: 41,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// The status and the reply params, or `(status, None)` for an envelope refusal.
fn served(cmd: u32, params: &[u8]) -> (u32, Option<Vec<u8>>) {
    let mut p = InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"));
    let reply = p
        .respond(&command(cmd, params))
        .expect("the policy claims this control");
    if reply.body.is_empty() {
        assert_ne!(reply.rpc_result, 0, "an empty body must never carry NV_OK");
        return (reply.rpc_result, None);
    }
    let status = u32::from_le_bytes(
        reply.body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4]
            .try_into()
            .expect("4 bytes"),
    );
    (
        status,
        Some(reply.body[PARAMS_AT..PARAMS_AT + params.len()].to_vec()),
    )
}

/// Whether the chain claims a control at all — the difference between *refused* and
/// *nobody answered*, which is the distinction the unserviced ledger exists to make.
fn claimed(cmd: u32, params_size: usize) -> bool {
    let mut p = InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"));
    p.respond(&command(cmd, &vec![0u8; params_size])).is_some()
}

// ── `0x20810108` — the binary-API control ──────────────────────────────────────────────

/// ★★★ The divergence, closed: hardware answers `NV_OK` and so must we.
#[test]
fn the_binapi_control_is_answered_nv_ok() {
    let (status, params) = served(BINAPI_CTRL_0X0108, &[0u8; BINAPI_CTRL_0X0108_PARAMS_SIZE]);
    assert_eq!(
        status, 0,
        "`[measured 2026-08-10, real GA106]` record 77 of the nvdiff host reference is NV_OK"
    );
    assert_eq!(
        params.expect("a body").len(),
        BINAPI_CTRL_0X0108_PARAMS_SIZE
    );
}

/// ★ The non-vacuity instrument, and the reason it is not redundant with the abi crate's own:
/// the *captured* body is all zeros on both sides, so a serve arm that returned an empty
/// (transport-zero-filled) reply would pass the test above. This drives a body no capture
/// contains and asserts the guest's own words come back.
///
/// ⊘ `[[an-in-annotation-is-not-a-transport-fact]]`: an empty reply body is a full-length
/// zero-fill, and RM copies it back regardless of direction markings.
#[test]
fn the_binapi_reply_is_the_request_not_a_zero_fill() {
    let mut params = vec![0u8; BINAPI_CTRL_0X0108_PARAMS_SIZE];
    for (i, b) in params.iter_mut().enumerate() {
        *b = u8::try_from(i % 251).expect("a byte");
    }
    let (status, body) = served(BINAPI_CTRL_0X0108, &params);
    assert_eq!(status, 0);
    assert_eq!(
        body.expect("a body"),
        params,
        "a zero-filled reply would overwrite words the caller sent"
    );
}

/// A `paramsSize` neither measured caller declares is refused, not padded.
#[test]
fn the_binapi_control_refuses_an_unmeasured_length() {
    let (status, body) = served(BINAPI_CTRL_0X0108, &[0u8; 64]);
    assert_ne!(status, 0, "64 bytes is not the 992 both callers declare");
    assert!(body.is_none(), "a refusal carries no body");
}

// ── `0x20800a9a` — the internal P-state boost ──────────────────────────────────────────

/// ★★★ The id the worklist did **not** name, served — and the id it **did** name, still not
/// claimed, because the wire never carries it here.
#[test]
fn the_boost_id_this_port_actually_receives_is_the_internal_one() {
    assert!(
        claimed(
            INTERNAL_PERF_BOOST_SET_2X,
            INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE
        ),
        "0x20800a9a is what `kperfBoostSet_IMPL` sends to physical RM"
    );
    assert!(
        !claimed(0x2080_200a, 8),
        "⊘ 0x2080200a is implemented by the GUEST KERNEL and reaches this port ZERO times \
         (`[measured 2026-08-10]`, 0 of the committed device logs name it). Claiming it \
         would be an arm for an id the wire does not carry — dead code with a green test."
    );
}

/// The two requests the differential measured on the ioctl boundary, carried through the
/// translation the guest kernel performs, are acknowledged with their own fields back.
#[test]
fn the_measured_boost_requests_are_acknowledged() {
    for (flags, duration) in [(0x12u8, 0xffff_ffffu32), (0x10, 0)] {
        let mut params = vec![0u8; INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE];
        params[0] = flags;
        params[4..8].copy_from_slice(&duration.to_le_bytes());
        let (status, body) = served(INTERNAL_PERF_BOOST_SET_2X, &params);
        assert_eq!(status, 0, "hardware answers NV_OK for flags {flags:#04x}");
        assert_eq!(body.expect("a body"), params);
    }
}

/// ⚠ The acknowledgement is only honest about a request this port could **read**. A flag bit
/// the SDK header names nothing for is refused, never masked away into an `NV_OK`.
#[test]
fn a_boost_request_this_port_cannot_read_is_refused_not_acknowledged() {
    let mut params = vec![0u8; INTERNAL_PERF_BOOST_SET_2X_PARAMS_SIZE];
    params[0] = 0x80; // bit 7 names nothing in ctrl2080perf.h
    let (status, body) = served(INTERNAL_PERF_BOOST_SET_2X, &params);
    assert_ne!(status, 0);
    assert!(body.is_none());
}

// ── The table's own accounting ─────────────────────────────────────────────────────────

/// ⊘ Both ids are in [`WantedTable::ALL`], which is what makes them *served* rather than
/// merely spelled — `from_cmd` is a lookup through that array, so a variant left out of it is
/// not merely untested, it is not served.
#[test]
fn both_ids_are_in_the_served_universe() {
    for cmd in [BINAPI_CTRL_0X0108, INTERNAL_PERF_BOOST_SET_2X] {
        let w = WantedTable::from_cmd(cmd).expect("in ALL");
        assert_eq!(w.cmd_id(), cmd);
        assert!(
            WantedTable::ALL.contains(&w),
            "from_cmd is a lookup through ALL; this cannot fail without the array shrinking"
        );
    }
    assert!(
        WantedTable::from_cmd(0x2080_200a).is_none(),
        "the userspace boost id is deliberately absent — see this file's module docs"
    );
}
