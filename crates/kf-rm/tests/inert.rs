//! `kf_rm::inert` — the commands this port acknowledges and deliberately does
//! nothing about.
//!
//! ## ★★★ What has to be tested here is the LIST, not the answer
//!
//! The answer is two lines. The risk is that the list grows by drift: task #127 made the
//! FSM's default a **named refusal** precisely so that an unmodelled command cannot pass
//! silently, and a policy that says `NV_OK` to things is the one place that guarantee can
//! be given back. So `the_inert_list_is_exactly_the_two_eligible_entries` pins the
//! membership, and `nothing_of_the_guests_request_comes_back` pins the property that
//! separates an inert acknowledgement from the echo the whole task deleted.

use kf_rm::inert::InertPolicy;
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};

fn command(function: RpcFunction, code: u32, payload: Vec<u8>) -> RpcCommand {
    RpcCommand {
        function,
        code,
        sequence: 7,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// Every function this port's vocabulary names, so the list below is quantified over the
/// real universe rather than over whatever the test author remembered.
const EVERY_FUNCTION: &[RpcFunction] = &[
    RpcFunction::SetGuestSystemInfo,
    RpcFunction::SetGuestSystemInfoExt,
    RpcFunction::Free,
    RpcFunction::DupObject,
    RpcFunction::UnloadingGuestDriver,
    RpcFunction::GetGspStaticInfo,
    RpcFunction::InitGspTraceCrashBuffer,
    RpcFunction::ContinuationRecord,
    RpcFunction::GspSetSystemInfo,
    RpcFunction::SetRegistry,
    RpcFunction::EccNotifierWriteAck,
    RpcFunction::RmControl,
    RpcFunction::RmAlloc,
    RpcFunction::InitDone,
    RpcFunction::PostEvent,
    RpcFunction::RcTriggered,
    RpcFunction::Other(0),
    RpcFunction::Other(228),
    RpcFunction::Other(u32::MAX),
];

#[test]
fn the_inert_list_is_exactly_the_two_eligible_entries() {
    let inert: Vec<RpcFunction> = EVERY_FUNCTION
        .iter()
        .copied()
        .filter(|f| InertPolicy::is_inert(*f))
        .collect();
    assert_eq!(
        inert,
        vec![
            RpcFunction::UnloadingGuestDriver,
            RpcFunction::InitGspTraceCrashBuffer
        ],
        "adding an entry is a deliberate edit to this assertion; see the module's docs \
         for what makes a command eligible",
    );
    // ★ And `Other(228)` is NOT inert: the wire id is only inert once the ABI table has
    // CLASSIFIED it, so a build whose function table forgot the id refuses rather than
    // accepting it by number.
    assert!(!InertPolicy::is_inert(RpcFunction::Other(228)));
}

#[test]
fn the_inert_command_is_acknowledged_with_nv_ok() {
    let mut p = InertPolicy::new();
    let reply = p
        .respond(&command(
            RpcFunction::InitGspTraceCrashBuffer,
            0xE4,
            vec![0u8; 12],
        ))
        .expect("fn 228 is acknowledged");
    assert_eq!(reply.rpc_result, 0);
}

#[test]
fn nothing_of_the_guests_request_comes_back() {
    let mut p = InertPolicy::new();
    // A plausible `{pa, size}`: a guest-physical address and a length. If either came
    // back, this policy would be an echo with a shorter list.
    let mut payload = vec![0u8; 12];
    payload[0..8].copy_from_slice(&0x1_2345_6000u64.to_le_bytes());
    payload[8..12].copy_from_slice(&0x4000u32.to_le_bytes());
    let reply = p
        .respond(&command(
            RpcFunction::InitGspTraceCrashBuffer,
            0xE4,
            payload.clone(),
        ))
        .expect("acknowledged");
    assert!(
        reply.body.is_empty(),
        "an empty body, which `RpcCommand::reply` zero-fills to the request's length",
    );
    assert_ne!(reply.body, payload);
}

#[test]
fn the_teardown_rpc_is_acknowledged_rather_than_refused() {
    // ★★★ PC-D5. Fn 47's refusal does not stay inside the reply: `kgspUnloadRm_IMPL`
    // stashes the RPC's status (`ogkm-580: kernel_gsp.c:4301`), runs the ENTIRE unload, and
    // then returns that stashed status in preference to the teardown's own
    // (`:4341-4343`). So refusing it hands `rmmod` a failure for an unload that succeeded.
    //
    // It is eligible by this module's rule and not by convenience:
    // `rpcUnloadingGuestDriver_v1F_07` reads back only `_issueRpcAndWait`'s status
    // (`ogkm-580: rpc.c:9168-9192`) — there is no `[OUT]` field to get wrong.
    let mut p = InertPolicy::new();
    // A real fn-47 body: `{bInPMTransition, bGc6Entering, newLevel}`.
    let mut payload = vec![0u8; 12];
    payload[0..4].copy_from_slice(&1u32.to_le_bytes());
    payload[8..12].copy_from_slice(&3u32.to_le_bytes());
    let reply = p
        .respond(&command(
            RpcFunction::UnloadingGuestDriver,
            47,
            payload.clone(),
        ))
        .expect("fn 47 is acknowledged, not left to the ledger's refusal");
    assert_eq!(
        reply.rpc_result, 0,
        "NV_OK, or rmmod reports a failed unload"
    );
    // ⊘ And still nothing of the guest's comes back: an inert acknowledgement is not an
    // echo with a shorter list.
    assert!(reply.body.is_empty());
    assert_ne!(reply.body, payload);
}

#[test]
fn every_other_function_falls_through_to_the_chain() {
    let mut p = InertPolicy::new();
    for f in EVERY_FUNCTION.iter().copied() {
        if InertPolicy::is_inert(f) {
            continue;
        }
        assert!(
            p.respond(&command(f, 1, vec![0u8; 8])).is_none(),
            "{f:?} must reach the next link, not be acknowledged here",
        );
    }
}

// ⊘ The served-chain test (the whole CommandPolicy chain) returns with P3 — see V3_P2_PORT_MAP.md.
