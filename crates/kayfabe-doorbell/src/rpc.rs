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

/// The RM control commands §2.3 enumerates, as the nested ids that ride inside `GSP_RM_CONTROL`.
///
/// ⚠ §1.3: `GSP_RM_ALLOC` and `GSP_RM_CONTROL` are **MIXED, default REFUSE** — serving the
/// envelope does not mean serving everything inside it. This is the inner allowlist.
pub const CONTROLS: [u32; 33] = [
    0x00801813, 0x00e00102, 0x00f10003, 0x20800122, 0x2080012b, 0x20800177, 0x20800a36,
    0x20800a40, 0x20800a41, 0x20800a4c, 0x20800a59, 0x20800a61, 0x20800a9f, 0x20800aac,
    0x20800af3, 0x20801210, 0x20801702, 0x20802209, 0x20802a08, 0x20803083, 0x20808159,
    0x20808162, 0x20809001, 0x2080a026, 0x83de0307, 0x906f0106, 0x90f10106, 0xa06c0101,
    0xa06c0105, 0xa06f0103, 0xa06f0104, 0xb0cc0105, 0xb0cc010a,
];

/// ⊘ Default REFUSE, by name. The envelope is served; its contents are an allowlist.
pub fn control_is_served(cmd: u32) -> bool {
    CONTROLS.contains(&cmd)
}
