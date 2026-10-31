//! `NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS` (`0xa06c010a`) at the
//! **reply-plane boundary** — §14.43.
//!
//! ## ⊘⊘ Why this file exists on day one rather than after a boot found the defect
//!
//! `bus_get_c2c_info.rs` is the precedent and it is a confession: §14.37 shipped a serve arm
//! with the chip flag **negated**, `kayfabe_abi::c2cinfo`'s own unit tests stayed green
//! throughout — they cover both arms of the callee faithfully — and only a boot caught it,
//! two rungs later. `a_signature_is_not_the_dispatch`: a correct function reached with the
//! wrong argument is indistinguishable from an unimplemented one at every boundary except
//! the wire.
//!
//! `kayfabe_abi::fmbpromote`'s tests are exactly that kind of callee test. This file asserts
//! the same things one layer out, through `InitTablePolicy::respond`, where the `params_at`
//! arithmetic, the declared-size check and the dispatch all actually happen.
//!
//! ## What is being pinned
//!
//! The control is **all-`[input]`** (`ogkm-580: ctrl/ctrla06c.h:329-352`), so the only
//! correct reply is `NV_OK` carrying the guest's own facts back. That makes the two failure
//! directions sharp and both are asserted here: an answer that is **refused** rebuilds
//! §14.43's wall (`kchangrpapiConstruct_IMPL`'s hard `goto failed`), and an answer that
//! is **served but altered** would be C defect **D7** — a reply overwriting a caller's
//! params with bytes it did not send.

use kayfabe_abi::fmbpromote::{
    ADDR_FBMEM, ADDR_SYSMEM, MAX_RUNQUEUES, MEMDESC_INFO_SIZE, NUM_VALID_ENTRIES_OFF,
    NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS,
    PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE,
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

/// An independently-written params builder — deliberately **not** reusing
/// `kayfabe_abi::fmbpromote`'s encoder, so a wrong offset there cannot make this file agree
/// with it.
fn params(entries: &[(u64, u64, u32, u64)], declared: u32) -> Vec<u8> {
    let mut p = vec![0u8; PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE];
    for (i, (base, size, aspace, bar2)) in entries.iter().enumerate() {
        let at = i * MEMDESC_INFO_SIZE;
        p[at..at + 8].copy_from_slice(&base.to_le_bytes());
        p[at + 8..at + 16].copy_from_slice(&size.to_le_bytes());
        p[at + 16..at + 24].copy_from_slice(&1u64.to_le_bytes()); // alignment, as RM sets it
        p[at + 24..at + 28].copy_from_slice(&aspace.to_le_bytes());
        p[at + 28..at + 32].copy_from_slice(&1u32.to_le_bytes()); // NV_MEMORY_CACHED
        let b2 = MEMDESC_INFO_SIZE * MAX_RUNQUEUES + i * 8;
        p[b2..b2 + 8].copy_from_slice(&bar2.to_le_bytes());
    }
    p[NUM_VALID_ENTRIES_OFF..NUM_VALID_ENTRIES_OFF + 4].copy_from_slice(&declared.to_le_bytes());
    p
}

/// What a GA106 guest actually sends: two runqueues, `20480` bytes each — the size
/// `kayfabe_abi::fmbsize` answers this same guest — in `ADDR_SYSMEM`, `bar2Addr = 0`.
fn realistic() -> Vec<u8> {
    params(
        &[
            (0x1_2340_0000, 20480, ADDR_SYSMEM, 0),
            (0x1_2341_0000, 20480, ADDR_SYSMEM, 0),
        ],
        2,
    )
}

fn command_with_declared_size(body: &[u8], declared_size: u32) -> RpcCommand {
    let mut payload = vec![0u8; PARAMS_AT + body.len()];
    payload[0..4].copy_from_slice(&0xc1d0_0013u32.to_le_bytes());
    payload[4..8].copy_from_slice(&0xcaf0_0002u32.to_le_bytes());
    payload[8..12]
        .copy_from_slice(&NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS.to_le_bytes());
    payload[16..20].copy_from_slice(&declared_size.to_le_bytes());
    payload[PARAMS_AT..].copy_from_slice(body);
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x23,
        sequence: 1,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn command(body: &[u8]) -> RpcCommand {
    command_with_declared_size(body, u32::try_from(body.len()).expect("fits"))
}

/// `(status, reply params)`. A refusal comes back as an empty body, which must never carry
/// `NV_OK`.
fn served(cmd: &RpcCommand) -> (u32, Vec<u8>) {
    let mut p = InitTablePolicy::new(chip(), *table_for(BENCH_DRIVER).expect("bench ABI"));
    let reply = p.respond(cmd).expect("the policy claims this control");
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
        reply.body[PARAMS_AT..PARAMS_AT + PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE].to_vec(),
    )
}

/// ★★★ The rung itself: the promotion a GA106 guest sends must be **answered**, and answered
/// with its own bytes.
///
/// A refusal here is §14.43's wall rebuilt — `kchangrpapiConstruct_IMPL` turns any non-`NV_OK`
/// into a hard `goto failed` and the channel group never exists.
#[test]
fn a_realistic_ga106_promotion_is_answered_nv_ok_with_its_own_bytes() {
    let sent = realistic();
    let (status, got) = served(&command(&sent));
    assert_eq!(
        status, 0,
        "every field of this control is [input]; refusing it rebuilds the channel-group wall"
    );
    assert_eq!(
        got, sent,
        "D7's shape: a reply may never carry a byte the caller did not send"
    );
}

/// The policy must *claim* the id — a control that classifies to nothing falls through to the
/// FSM and lands in the unserviced ledger, which is exactly where the boot found it.
#[test]
fn the_policy_claims_the_id_and_states_the_size() {
    assert_eq!(
        WantedTable::from_cmd(NVA06C_CTRL_CMD_INTERNAL_PROMOTE_FAULT_METHOD_BUFFERS),
        Some(WantedTable::PromoteFaultMethodBuffers)
    );
    assert_eq!(
        WantedTable::PromoteFaultMethodBuffers.params_size(),
        PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE
    );
    assert_eq!(WantedTable::PromoteFaultMethodBuffers.cmd_id(), 0xa06c_010a);
}

/// ★ `ADDR_FBMEM` is the `retryInFB` fallback in
/// `kchangrpAllocFaultMethodBuffers_GV100` and is just as legal as sysmem — a serve arm that
/// only accepted sysmem would refuse a guest whose sysmem allocation failed.
#[test]
fn a_framebuffer_resident_method_buffer_is_answered_too() {
    let sent = params(&[(0x8000_0000, 20480, ADDR_FBMEM, 0)], 1);
    let (status, got) = served(&command(&sent));
    assert_eq!(status, 0);
    assert_eq!(got, sent);
}

/// ⊘ The **destroy** form — `size == 0` — is protocol-legal (`ogkm-580:
/// ctrl/ctrla06c.h:337-339`) and must be answered, not refused. A port that treated it as
/// malformed would reject legitimate guest traffic on the channel-group free path.
#[test]
fn the_destroy_form_is_answered_rather_than_refused() {
    let sent = params(&[(0, 0, ADDR_SYSMEM, 0), (0, 0, ADDR_SYSMEM, 0)], 2);
    let (status, got) = served(&command(&sent));
    assert_eq!(status, 0);
    assert_eq!(got, sent);
}

/// ★★★ D1's shape at the boundary: a runqueue count past the two-element array is refused by
/// name, never clamped into two accepted records.
#[test]
fn a_runqueue_count_past_the_array_is_refused_at_the_boundary() {
    for declared in [3u32, 64, u32::MAX] {
        let sent = params(
            &[
                (0x1000, 20480, ADDR_SYSMEM, 0),
                (0x2000, 20480, ADDR_SYSMEM, 0),
            ],
            declared,
        );
        let (status, got) = served(&command(&sent));
        assert_ne!(status, 0, "{declared} runqueues must not be answered NV_OK");
        assert!(got.is_empty());
    }
}

/// An aperture this port cannot name is refused rather than folded into sysmem — the ruling
/// `gpu_promote_ctx.md` §1.4 makes for `physAttr[1:0] == 3`, at this control's boundary.
#[test]
fn an_unnameable_aperture_is_refused_at_the_boundary() {
    for aspace in [0u32, 3, 4, 6, 7, 8] {
        let sent = params(&[(0x1000, 20480, aspace, 0)], 1);
        let (status, _) = served(&command(&sent));
        assert_ne!(
            status, 0,
            "address space {aspace} must not be answered NV_OK"
        );
    }
}

/// The guest's **declared** `paramsSize` must be the size this port encodes — checked
/// exactly, per `gsp_core_bridge.md` §4.3, not as a lower bound.
#[test]
fn a_declared_size_that_is_not_the_struct_is_refused() {
    let sent = realistic();
    for declared in [0u32, 84, 87, 89, 560] {
        let (status, _) = served(&command_with_declared_size(&sent, declared));
        assert_ne!(
            status, 0,
            "a declared paramsSize of {declared} is not this struct"
        );
    }
    // …and the exact one is not refused, so the sweep above cannot pass vacuously.
    let (status, _) = served(&command_with_declared_size(
        &sent,
        u32::try_from(PROMOTE_FAULT_METHOD_BUFFERS_PARAMS_SIZE).expect("fits"),
    ));
    assert_eq!(status, 0);
}

/// ⊘ Slots past `numValidEntries` are not repeated back. This port did not read them, so
/// echoing them would be repeating bytes it never validated — the one place the reply is
/// deliberately not a memcpy of the request.
#[test]
fn slots_past_the_declared_count_are_not_echoed_at_the_boundary() {
    let mut sent = params(&[(0x1000, 20480, ADDR_SYSMEM, 0)], 1);
    sent[MEMDESC_INFO_SIZE..MEMDESC_INFO_SIZE * 2].copy_from_slice(&[0xcd; MEMDESC_INFO_SIZE]);
    let (status, got) = served(&command(&sent));
    assert_eq!(status, 0);
    assert_eq!(&got[MEMDESC_INFO_SIZE..MEMDESC_INFO_SIZE * 2], &[0u8; 32]);
    assert_ne!(got, sent);
}
