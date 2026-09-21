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
fn the_two_no_reply_functions_are_ignore_not_refuse() {
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

/// ⊘⊘⊘ §2.3's control ids **with the dispositions the doc states**. The first version of this
/// file listed only the ids and asserted every one was SERVED — which asserted the bug
/// (`GPU_EXEC_REG_OPS`, arbitrary register peek/poke, marked served). ⚠ And the "transcribed
/// independently" claim was false: it was a byte-identical copy of `rpc::CONTROLS`, so the two
/// agreed by construction. This list carries the VERDICTS, which is where the disagreement lives.
const MUST_BE_REFUSED: [(u32, &str); 9] = [
    (0x20800122, "GPU_EXEC_REG_OPS"),
    (0xb0cc010a, "perf EXEC_REG_OPS"),
    (0xb0cc0105, "ALLOC_PMA_STREAM"),
    (0x83de0307, "DEBUG_SET_MODE_MMU_DEBUG"),
    (0x20800177, "GPU_REPORT_NON_REPLAYABLE_FAULT"),
    (0x00e00102, "fabric"),
    (0x00f10003, "fabric"),
    (0x20803083, "fabric"),
    (0x20801702, "MC_SERVICE_INTERRUPTS"),
];

#[test]
fn the_nine_refused_by_name_controls_are_not_served() {
    // ★★★ The one that would have shipped a hole: a handler written against the old flat list
    // would have admitted REGISTER PEEK/POKE from the guest.
    let mut wrong = Vec::new();
    for (id, name) in MUST_BE_REFUSED {
        if rpc::control_is_served(id) {
            wrong.push(format!("{id:#010x} {name} is SERVED but §2.3 refuses it by name"));
        }
        match rpc::control_disposition(id) {
            rpc::ControlDisposition::RefusedByName(_) => {}
            other => wrong.push(format!("{id:#010x} {name}: {other:?}")),
        }
    }
    assert!(wrong.is_empty(), "⊘ REFUSAL LIST VIOLATED:\n  {}", wrong.join("\n  "));
}

#[test]
fn admitted_is_not_served() {
    // ⚠ §2.3: ~135 commands pass the allowlist with NO handler and fall to the unserviced ledger.
    // Collapsing that into "served" is how "the allowlist admits it" becomes "we handle it".
    assert_eq!(rpc::control_disposition(0x2080a026), rpc::ControlDisposition::AdmittedUndispatched);
    assert!(!rpc::control_is_served(0x2080a026), "admitted-undispatched is NOT served");
}

#[test]
fn an_unlisted_control_is_refused_even_though_the_envelope_is_served() {
    assert_eq!(rpc::classify(76), Disposition::Serve, "the envelope is served");
    assert!(!rpc::control_is_served(0x2080ffff));
    assert!(matches!(
        rpc::control_disposition(0xdeadbeef),
        rpc::ControlDisposition::RefusedByName(_)
    ));
}
