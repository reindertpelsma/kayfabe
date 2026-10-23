//! SECURITY AUDIT 2026-10-21, finding SA-02 (docs/audits/2026-10-21-security-audit.md): EXPECTED RED.
//!
//! `kf_rm::chanlink::is_rm_internal_client` decides, from the client HANDLE VALUE alone, that a channel belongs to
//! the guest KERNEL's RM. That decision (via `kernel_channel`) routes the channel Translated and is the input of
//! `kf_qemu::tspace::Privileged::of`, the only key to the T-space's whole-guest-RAM and whole-store windows.
//!
//! Two defects, both visible without a GPU:
//!  1. The test is `h & 0xC1E0_0000 == 0xC1E0_0000`, a six-bit mask. The documented internal range is
//!     `0xC1E0_0000..=0xC1EF_FFFF` (`RS_CLIENT_HANDLE_MAX`), but `0xFFFF_FFFF`, `0xC1F0_0000`, `0xE1E0_0000` ... all
//!     satisfy it. (The guest driver's own `serverIsClientInternal` has the same mask, so this mirrors the driver,
//!     which is fine for the driver and wrong for a trust decision.)
//!  2. The doc comment says a client that asks for a fixed handle is re-encoded onto the user base, "so no guest
//!     process can hold one in this range". In the shipped open driver that re-encode is compiled out
//!     (`RS_COMPATABILITY_MODE=1`, ogkm-580 `src/nvidia/Makefile:130`; `rs_server.c:3268`), and `serverAllocClient`
//!     takes `hClient = pParams->hClient` (`rs_server.c:612`): the user chooses its client handle. So a guest
//!     userspace process can allocate its root with hClient = 0xC1E0_0100 and be classed a kernel client.
//!
//! This test pins defect 1 only (the exact-range bound), because defect 2 needs a dynamic check on a Linux guest
//! and a design decision (what else identifies RM's own internal clients, e.g. the `internalFlags` stamp).
use kf_rm::chanlink::is_rm_internal_client;

#[test]
fn only_the_documented_internal_handle_range_is_internal() {
    assert!(
        is_rm_internal_client(0xC1E0_0006),
        "the PMA scrubber's client"
    );
    assert!(
        !is_rm_internal_client(0xC1D0_0001),
        "an ordinary user client"
    );
    assert!(
        !is_rm_internal_client(0xC1F0_0000),
        "0xC1F00000 is past RS_CLIENT_HANDLE_MAX (0x100000) above the internal base"
    );
    assert!(
        !is_rm_internal_client(0xFFFF_FFFF),
        "all ones is not a client the guest RM minted"
    );
}
