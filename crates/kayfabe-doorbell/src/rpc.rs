//! The GSP RPC plane — §1 of `THE_SURFACE_v3.md`, as an exhaustive dispatch.
//!
//! ## ★★★ The safe default is a NAMED REFUSAL, never an echo
//!
//! §1.3: *"whatever no link answers falls through to a refusal … An `EchoOk` policy exists but is
//! a **differential-test fixture** and is **not installed in the production chain**. This matters
//! because `0x56` is a status the guest driver **forgives** — so a generic-ack fallback would let
//! a wrong configuration run on, undetected, which is precisely the failure this project has
//! measured repeatedly."*
//!
//! ⇒ [`Disposition::Refuse`] is what ~210 ids get, and it is a **decision**, not a gap.
//!
//! ## ⊘ IGNORE is not REFUSE, and confusing them desyncs the guest
//!
//! Two functions get **no reply at all** (`GSP_SET_SYSTEM_INFO`, `SET_REGISTRY`): §1.3 —
//! *"⊘ no reply at all; **echoing would desync the guest's sequence counter**"*. A refusal is a
//! reply. ⇒ The two are different variants here, deliberately.

/// What we do with one RPC function id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Answer it from local state. ⊘ §1.4: *"No RPC is a verbatim forward."*
    Serve,
    /// ⊘ No reply at all — a reply would advance the guest's sequence counter.
    Ignore,
    /// Reply `NV_ERR_NOT_SUPPORTED` (`0x56`) with a zeroed body, **by name**.
    Refuse,
    /// We send it; the guest never sends it to us.
    OutboundOnly,
    /// ⊘ Not a function at all — an overflow fragment, folded into reassembly.
    Fragment,
}

/// Every id §1.3 enumerates. ⊘ The list is the contract: a gate asserts it is complete, so
/// "all RPCs are implemented" is a checkable fact rather than a claim.
pub const SURFACE: [(u32, &str, Disposition); 18] = [
    (1, "SET_GUEST_SYSTEM_INFO", Disposition::Serve),
    (64, "SET_GUEST_SYSTEM_INFO_EXT", Disposition::Serve),
    (65, "GET_GSP_STATIC_INFO", Disposition::Serve),
    (70, "UPDATE_BAR_PDE", Disposition::Serve),
    (72, "GSP_SET_SYSTEM_INFO", Disposition::Ignore),
    (73, "SET_REGISTRY", Disposition::Ignore),
    (10, "FREE", Disposition::Serve),
    (21, "DUP_OBJECT", Disposition::Serve),
    (103, "GSP_RM_ALLOC", Disposition::Serve),
    (76, "GSP_RM_CONTROL", Disposition::Serve),
    (71, "CONTINUATION_RECORD", Disposition::Fragment),
    (47, "UNLOADING_GUEST_DRIVER", Disposition::Serve),
    (202, "ECC_NOTIFIER_WRITE_ACK", Disposition::Ignore),
    (228, "INIT_GSP_TRACE_CRASH_BUFFER", Disposition::Serve),
    (0x1001, "GSP_INIT_DONE", Disposition::OutboundOnly),
    (0x1003, "POST_EVENT", Disposition::OutboundOnly),
    (0x1004, "RC_TRIGGERED", Disposition::OutboundOnly),
    // ⊘ A sentinel so the table's own shape is asserted rather than assumed.
    (u32::MAX, "__END", Disposition::Refuse),
];

/// ★ Dispatch. Total over `u32` by construction — there is no "unhandled" arm.
pub fn classify(function: u32) -> Disposition {
    match SURFACE.iter().find(|(f, _, _)| *f == function && *f != u32::MAX) {
        Some((_, _, d)) => *d,
        // ⊘ ~210 ids, and this is the decision for all of them.
        None => Disposition::Refuse,
    }
}

pub fn name_of(function: u32) -> Option<&'static str> {
    SURFACE.iter().find(|(f, _, _)| *f == function && *f != u32::MAX).map(|(_, n, _)| *n)
}

/// RM control commands, **with their dispositions** — §2.3.
///
/// ## ⊘⊘⊘ THE DEFECT THIS REPLACES, because it would have shipped a hole
///
/// `[fable w823, HIGH H1]` the first version was a flat `CONTROLS: [u32; 33]` built by copying
/// §2.3's ids **without their dispositions**, and `control_is_served()` returned `true` for all
/// of them. Nine are **refused by name** in §2.3, including:
///
/// | id | name | why §2.3 refuses it |
/// |---|---|---|
/// | `0x20800122` | `GPU_EXEC_REG_OPS` | ★ **arbitrary register peek/poke** |
/// | `0xb0cc010a` | perf-counter `EXEC_REG_OPS` | arbitrary register peek/poke |
/// | `0xb0cc0105` | `ALLOC_PMA_STREAM` | hardware performance counters |
/// | `0x83de0307` | `DEBUG_SET_MODE_MMU_DEBUG` | SM-debugger |
/// | `0x20800177` | `GPU_REPORT_NON_REPLAYABLE_FAULT` | the fault mechanism is not modelled |
/// | `0x00e00102`, `0x00f10003`, `0x20803083` | fabric / NVLink | |
/// | `0x20801702` | `MC_SERVICE_INTERRUPTS` | ★ **deliberately** refused, to cancel the guest's polling loop |
///
/// ⇒ The first handler written against that "inner allowlist" would have admitted **register
/// peek/poke from the guest**. ⚠ And the coverage test *asserted* it was served — a test that
/// encoded the bug.
///
/// ★ The lesson is narrow and repeatable: **an id list is not an allowlist.** Copying the
/// identifiers out of a table and leaving the verdicts behind inverts the table's meaning while
/// looking like faithful transcription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlDisposition {
    /// Answered from our own model, no host GPU touched.
    ServedLocally,
    /// ⊘ Refused by name, with a reason.
    RefusedByName(&'static str),
    /// Passes the allowlist, no handler ⇒ falls to the unserviced ledger as `NV_ERR_NOT_SUPPORTED`.
    /// ⚠ §2.3: ~135 commands are in this state. Admitted is **not** served.
    AdmittedUndispatched,
}

pub const CONTROLS: [(u32, ControlDisposition); 33] = [
    (0x00801813, ControlDisposition::ServedLocally),
    (0x2080012b, ControlDisposition::ServedLocally),
    (0x20800a36, ControlDisposition::ServedLocally),
    (0x20800a40, ControlDisposition::ServedLocally),
    (0x20800a41, ControlDisposition::ServedLocally),
    (0x20800a4c, ControlDisposition::ServedLocally),
    (0x20800a59, ControlDisposition::ServedLocally),
    (0x20800a61, ControlDisposition::ServedLocally),
    (0x20800a9f, ControlDisposition::ServedLocally),
    (0x20800aac, ControlDisposition::ServedLocally),
    (0x20800af3, ControlDisposition::ServedLocally),
    (0x20801210, ControlDisposition::ServedLocally),
    (0x20802209, ControlDisposition::ServedLocally),
    (0x20802a08, ControlDisposition::ServedLocally),
    (0x20808159, ControlDisposition::ServedLocally),
    (0x20808162, ControlDisposition::ServedLocally),
    (0x20809001, ControlDisposition::ServedLocally),
    (0x90f10106, ControlDisposition::ServedLocally),
    (0xa06c0101, ControlDisposition::ServedLocally),
    (0xa06c0105, ControlDisposition::ServedLocally),
    (0xa06f0103, ControlDisposition::ServedLocally),
    (0xa06f0104, ControlDisposition::ServedLocally),
    (0x906f0106, ControlDisposition::ServedLocally),
    (0x2080a026, ControlDisposition::AdmittedUndispatched),
    // ⊘ The nine §2.3 refuses BY NAME.
    (0x20800122, ControlDisposition::RefusedByName("GPU_EXEC_REG_OPS: arbitrary register peek/poke")),
    (0xb0cc010a, ControlDisposition::RefusedByName("perf EXEC_REG_OPS: arbitrary register peek/poke")),
    (0xb0cc0105, ControlDisposition::RefusedByName("ALLOC_PMA_STREAM: hardware performance counters")),
    (0x83de0307, ControlDisposition::RefusedByName("DEBUG_SET_MODE_MMU_DEBUG: SM debugger")),
    (0x20800177, ControlDisposition::RefusedByName("GPU_REPORT_NON_REPLAYABLE_FAULT: fault mechanism not modelled")),
    (0x00e00102, ControlDisposition::RefusedByName("fabric/NVLink")),
    (0x00f10003, ControlDisposition::RefusedByName("fabric/NVLink")),
    (0x20803083, ControlDisposition::RefusedByName("fabric/NVLink")),
    (0x20801702, ControlDisposition::RefusedByName("MC_SERVICE_INTERRUPTS: deliberately refused to cancel the guest's polling loop")),
];

pub fn control_disposition(cmd: u32) -> ControlDisposition {
    CONTROLS
        .iter()
        .find(|(c, _)| *c == cmd)
        .map(|(_, d)| *d)
        // ⊘ Default deny, by name.
        .unwrap_or(ControlDisposition::RefusedByName("not on the allowlist — default deny"))
}

/// ⊘ *Served* means answered. Admitted-undispatched and refused are both **not served**, and
/// collapsing them is how "the allowlist admits it" became "we handle it".
pub fn control_is_served(cmd: u32) -> bool {
    matches!(control_disposition(cmd), ControlDisposition::ServedLocally)
}
