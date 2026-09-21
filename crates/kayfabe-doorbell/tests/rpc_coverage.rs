//! ★★★ "ALL RPCs AND NEEDED RMs ARE IMPLEMENTED" — as a CHECKABLE FACT, not a claim.
//!
//! `[owner, 2026-09-21]` *"ensure all rpcs and needed rms are implemented."*
//!
//! ⊘ The only honest way to answer that is a manifest the build checks. This file holds the
//! required surface **independently of the implementation** — transcribed from
//! `docs/design/THE_SURFACE_v3.md` §1.3 and §2.3 — and asserts the implementation covers it.
//! ⚠ If the two agreed by construction the test would be worthless, so the list is written out
//! here rather than imported.

use kayfabe_doorbell::rpc::{self, Disposition};

/// §1.3's table, transcribed. `(id, name)`.
const REQUIRED_RPCS: [(u32, &str); 17] = [
    (1, "SET_GUEST_SYSTEM_INFO"),
    (64, "SET_GUEST_SYSTEM_INFO_EXT"),
    (65, "GET_GSP_STATIC_INFO"),
    (70, "UPDATE_BAR_PDE"),
    (72, "GSP_SET_SYSTEM_INFO"),
    (73, "SET_REGISTRY"),
    (10, "FREE"),
    (21, "DUP_OBJECT"),
    (103, "GSP_RM_ALLOC"),
    (76, "GSP_RM_CONTROL"),
    (71, "CONTINUATION_RECORD"),
    (47, "UNLOADING_GUEST_DRIVER"),
    (202, "ECC_NOTIFIER_WRITE_ACK"),
    (228, "INIT_GSP_TRACE_CRASH_BUFFER"),
    (0x1001, "GSP_INIT_DONE"),
    (0x1003, "POST_EVENT"),
    (0x1004, "RC_TRIGGERED"),
];

#[test]
fn every_required_rpc_has_a_disposition() {
    let mut missing = Vec::new();
    for (id, name) in REQUIRED_RPCS {
        match rpc::name_of(id) {
            Some(n) if n == name => {}
            Some(n) => missing.push(format!("{id:#x}: implemented as {n}, required {name}")),
            None => missing.push(format!("{id:#x} {name}: NOT IMPLEMENTED — falls through to Refuse")),
        }
    }
    assert!(missing.is_empty(), "⊘ RPC SURFACE INCOMPLETE:\n  {}", missing.join("\n  "));
}

#[test]
fn the_two_no_reply_functions_are_IGNORE_not_REFUSE() {
    // ⊘ §1.3: "no reply at all; ECHOING WOULD DESYNC THE GUEST'S SEQUENCE COUNTER." A refusal IS
    // a reply, so classifying these as Refuse would desync the guest just as an echo would.
    assert_eq!(rpc::classify(72), Disposition::Ignore, "GSP_SET_SYSTEM_INFO");
    assert_eq!(rpc::classify(73), Disposition::Ignore, "SET_REGISTRY");
    assert_eq!(rpc::classify(202), Disposition::Ignore, "ECC_NOTIFIER_WRITE_ACK");
}

#[test]
fn an_unknown_function_is_refused_by_name_never_echoed() {
    // ★★★ §1.3: "The safe default is a NAMED REFUSAL, never an echo ... 0x56 is a status the
    // guest driver FORGIVES — so a generic-ack fallback would let a wrong configuration run on,
    // undetected, which is precisely the failure this project has measured repeatedly."
    for id in [2u32, 99, 150, 0x2000, 0xffff] {
        assert_eq!(rpc::classify(id), Disposition::Refuse, "{id:#x} must be refused");
        assert_eq!(rpc::name_of(id), None);
    }
}

#[test]
fn outbound_events_are_never_served_as_requests() {
    // ⊘ The guest never sends these to us; treating one as a request would answer a message that
    // was never asked.
    for id in [0x1001u32, 0x1003, 0x1004] {
        assert_eq!(rpc::classify(id), Disposition::OutboundOnly);
    }
}

/// §2.3's control ids, transcribed independently.
const REQUIRED_CONTROLS: [u32; 33] = [
    0x00801813, 0x00e00102, 0x00f10003, 0x20800122, 0x2080012b, 0x20800177, 0x20800a36,
    0x20800a40, 0x20800a41, 0x20800a4c, 0x20800a59, 0x20800a61, 0x20800a9f, 0x20800aac,
    0x20800af3, 0x20801210, 0x20801702, 0x20802209, 0x20802a08, 0x20803083, 0x20808159,
    0x20808162, 0x20809001, 0x2080a026, 0x83de0307, 0x906f0106, 0x90f10106, 0xa06c0101,
    0xa06c0105, 0xa06f0103, 0xa06f0104, 0xb0cc0105, 0xb0cc010a,
];

#[test]
fn every_required_rm_control_is_served() {
    let missing: Vec<String> = REQUIRED_CONTROLS
        .iter()
        .filter(|c| !rpc::control_is_served(**c))
        .map(|c| format!("{c:#010x}"))
        .collect();
    assert!(missing.is_empty(), "⊘ RM CONTROL SURFACE INCOMPLETE: {}", missing.join(" "));
}

#[test]
fn an_unlisted_control_is_refused_even_though_the_envelope_is_served() {
    // ⚠ §1.3: GSP_RM_ALLOC and GSP_RM_CONTROL are "MIXED, default REFUSE". Serving the ENVELOPE
    // does not mean serving everything inside it -- the inner allowlist is separate.
    assert_eq!(rpc::classify(76), Disposition::Serve, "the envelope is served");
    assert!(!rpc::control_is_served(0x2080ffff), "but an unlisted control inside it is not");
    assert!(!rpc::control_is_served(0xdeadbeef));
}
