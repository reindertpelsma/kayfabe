//! `SET_GUEST_SYSTEM_INFO` (fn 1) and `SET_GUEST_SYSTEM_INFO_EXT` (fn 64) — the version
//! handshake, and the one rung the #127 refusal-default surfaced.
//!
//! ## ★★★ What makes this handshake worth a test at all
//!
//! It is the one control where an **echo passes**. The guest writes its own
//! `VGX_*_VERSION_NUMBER` into the request, reads the same two fields back **out of the
//! reply**, and hands them to `rpcSetIpVersion`, which selects the RPC function table every
//! later message is encoded against (`ogkm-580:
//! src/nvidia/src/kernel/vgpu/rpc.c:8760-8828`). A mirror therefore agrees with anything,
//! and a device that speaks no version at all sails through — with the disagreement
//! surfacing hundreds of messages later at the wrong struct offsets.
//!
//! So `a_guest_speaking_another_version_is_refused_rather_than_agreed_with` is the
//! load-bearing test here, not the happy path. It is the one an echo fails.

use kayfabe_abi::DriverVersion;
use kayfabe_abi::guestsysinfo::{
    GuestSystemInfoError, SET_GUEST_SYSTEM_INFO_SIZE, VgxVersion, decode_declared_vgx,
    encode_set_guest_system_info_reply,
};
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_device::guestsysinfo::GuestSystemInfoPolicy;
use kayfabe_gsp::{CommandPolicy, RpcCommand, RpcFunction};

/// `ogkm-580: src/nvidia/inc/kernel/vgpu/vgpu_version.h:33-34`, written as literals so a
/// change to the table is a change this file has to agree to.
const V580_MAJOR: u32 = 0x2B;
const V580_MINOR: u32 = 0x13;
/// `ogkm-610: vgpu_version.h:33-34` — and it moved, which is the whole reason for a row.
const V610_MAJOR: u32 = 0x2E;
const V610_MINOR: u32 = 0x0D;

fn policy_for(v: DriverVersion) -> GuestSystemInfoPolicy {
    GuestSystemInfoPolicy::new(*table_for(v).expect("a table row"))
}

/// A fn-1 request declaring `major`/`minor`, with the three `[IN]` strings filled with a
/// byte that is not zero — so a reply that reflected any of them is visible.
fn request(major: u32, minor: u32) -> RpcCommand {
    let mut payload = vec![0xCDu8; SET_GUEST_SYSTEM_INFO_SIZE];
    payload[0..4].copy_from_slice(&major.to_le_bytes());
    payload[4..8].copy_from_slice(&minor.to_le_bytes());
    RpcCommand {
        function: RpcFunction::SetGuestSystemInfo,
        code: 1,
        sequence: 3,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

#[test]
fn the_struct_is_the_size_the_generated_header_declares() {
    // Six NvU32 and three char[0x100], no alignment hole. 792, as a literal.
    assert_eq!(SET_GUEST_SYSTEM_INFO_SIZE, 792);
    assert_eq!(6 * 4 + 3 * 0x100, 792);
}

#[test]
fn the_bench_driver_speaks_the_version_its_own_header_declares() {
    let t = table_for(BENCH_DRIVER).expect("bench ABI");
    assert_eq!(
        t.vgx_version(),
        Some(VgxVersion {
            major: V580_MAJOR,
            minor: V580_MINOR
        })
    );
}

#[test]
fn the_version_is_a_row_and_it_really_does_move() {
    let t610 = table_for(DriverVersion {
        major: 610,
        minor: 43,
        patch: 2,
    })
    .expect("610 row");
    assert_eq!(
        t610.vgx_version(),
        Some(VgxVersion {
            major: V610_MAJOR,
            minor: V610_MINOR
        })
    );
    // ★ NON-VACUITY: if the two rows agreed, every assertion in this file would hold for a
    // port that hard-coded one constant.
    assert_ne!(
        table_for(BENCH_DRIVER).expect("bench ABI").vgx_version(),
        t610.vgx_version(),
    );
}

#[test]
fn a_driver_with_no_citation_gets_no_version_and_the_handshake_refuses() {
    // 550.54.04 is a real row in the table, and this port has no `vgpu_version.h` for it.
    let old = DriverVersion {
        major: 550,
        minor: 54,
        patch: 4,
    };
    assert_eq!(table_for(old).expect("550 row").vgx_version(), None);

    let p = policy_for(old);
    assert_eq!(
        p.agreed_version(&request(V580_MAJOR, V580_MINOR).payload)
            .expect_err("no citation, no answer"),
        GuestSystemInfoError::NoVersionForDriver,
    );
}

#[test]
fn the_reply_states_our_version_and_reflects_none_of_the_request() {
    let mut p = policy_for(BENCH_DRIVER);
    let req = request(V580_MAJOR, V580_MINOR);
    let reply = p.respond(&req).expect("the policy answers fn 1");

    assert_eq!(reply.rpc_result, 0, "NV_OK in the envelope");
    assert_eq!(reply.body.len(), 792);
    // Literal offsets 0 and 4 — the two words the guest reads back.
    assert_eq!(&reply.body[0..4], &V580_MAJOR.to_le_bytes()[..]);
    assert_eq!(&reply.body[4..8], &V580_MINOR.to_le_bytes()[..]);
    // ★★★ THE BITE. The request's three 256-byte `[IN]` strings were 0xCD. Nothing of the
    // guest's own comes back: the reply is authored, not reflected.
    assert!(
        !reply.body.contains(&0xCD),
        "a byte of the request survived into the reply",
    );
    assert!(
        reply.body[8..].iter().all(|b| *b == 0),
        "everything else is zero"
    );
}

#[test]
fn a_guest_speaking_another_version_is_refused_rather_than_agreed_with() {
    let mut p = policy_for(BENCH_DRIVER);
    // The 610 pair, sent to a port bound to 580. An echoing GSP would agree; RM would then
    // select the 610 function table and encode every later message at the wrong offsets.
    let reply = p
        .respond(&request(V610_MAJOR, V610_MINOR))
        .expect("a refusal is still a reply");
    assert_eq!(reply.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);
    assert!(reply.body.is_empty());

    assert_eq!(
        policy_for(BENCH_DRIVER)
            .agreed_version(&request(V610_MAJOR, V610_MINOR).payload)
            .expect_err("mismatched"),
        GuestSystemInfoError::VersionMismatch {
            guest: VgxVersion {
                major: V610_MAJOR,
                minor: V610_MINOR
            },
            ours: VgxVersion {
                major: V580_MAJOR,
                minor: V580_MINOR
            },
        },
    );
}

#[test]
fn a_payload_too_short_to_hold_the_struct_is_refused_not_padded() {
    let mut p = policy_for(BENCH_DRIVER);
    let mut cmd = request(V580_MAJOR, V580_MINOR);
    cmd.payload.truncate(791);
    let reply = p.respond(&cmd).expect("a refusal is still a reply");
    assert_eq!(reply.rpc_result, kayfabe_abi::NV_ERR_NOT_SUPPORTED);

    assert_eq!(
        decode_declared_vgx(&cmd.payload).expect_err("short"),
        GuestSystemInfoError::Truncated {
            need: 792,
            got: 791
        }
    );
}

#[test]
fn the_tail_call_is_answered_too_because_the_guest_returns_its_status() {
    let mut p = policy_for(BENCH_DRIVER);
    let reply = p
        .respond(&RpcCommand {
            function: RpcFunction::SetGuestSystemInfoExt,
            code: 0x40,
            sequence: 4,
            payload: vec![0xCDu8; 0x108],
            elements: 1,
            delivered: Vec::new(),
        })
        .expect("fn 64 is answered");
    assert_eq!(reply.rpc_result, 0);
    // Empty, which `RpcCommand::reply` zero-fills to the request's own length — so the
    // guest's `guestDriverBranch` string is not handed back either.
    assert!(reply.body.is_empty());
}

#[test]
fn every_other_function_falls_through() {
    let mut p = policy_for(BENCH_DRIVER);
    for f in [
        RpcFunction::RmControl,
        RpcFunction::GetGspStaticInfo,
        RpcFunction::RmAlloc,
        RpcFunction::Other(999),
    ] {
        assert!(
            p.respond(&RpcCommand {
                function: f,
                code: 999,
                sequence: 5,
                payload: vec![0u8; 64],
                elements: 1,
                delivered: Vec::new(),
            })
            .is_none(),
            "{f:?} is not this policy's",
        );
    }
}

#[test]
fn the_encoder_writes_only_the_two_words() {
    let body = encode_set_guest_system_info_reply(VgxVersion {
        major: 0x2B,
        minor: 0x13,
    });
    assert_eq!(body.len(), 792);
    assert_eq!(&body[0..8], &[0x2B, 0, 0, 0, 0x13, 0, 0, 0]);
    assert!(body[8..].iter().all(|b| *b == 0));
}
