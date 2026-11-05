//! The command policy that answers the **version handshake** — `SET_GUEST_SYSTEM_INFO`
//! (fn 1) and its tail call `SET_GUEST_SYSTEM_INFO_EXT` (fn 64) — from the driver row.
//!
//! ## ★★ Why this one is keyed on the DRIVER and not on the chip
//!
//! Every other policy in this crate joins a chip row to a wire layout. This one has no
//! chip content at all: the vGPU RPC version is an Axis-A fact about the *guest driver*,
//! and it lives on [`kayfabe_abi::versions::DriverAbiTable`]. It is here because this is
//! where policies live, and it takes only a `DriverAbiTable` — which is exactly what a
//! reader should be able to see at a glance.
//!
//! ## ★★★ This is the control the #127 refusal-default surfaced, and it surfaced ALONE
//!
//! `[measured]` run `t127a` — a stock 580.159.04 guest against `f870288`, driven with
//! `nvidia-smi`, the first boot after the emulated GSP's default became a named refusal.
//! The host-side ledger ([`crate::unserviced`]) reported the whole list in one boot:
//!
//! ```text
//! nvkvm: commands: 3 decoded, 1 UNSERVICED (…), 1 distinct
//! nvkvm:   unserviced fn 1
//! ```
//!
//! and the guest named the same thing from its side:
//!
//! ```text
//! NVRM: kgspInitRm_IMPL: SET_GUEST_SYSTEM_INFO failed: 0x56
//! NVRM: RmInitAdapter: Cannot initialize GSP firmware RM
//! NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x62:0x56:2028)
//! ```
//!
//! ★ Both halves agreeing is the point. `0x56` is our own `NV_ERR_NOT_SUPPORTED` arriving
//! at the guest verbatim, so the envelope really does reach RM; and the count says the list
//! is **one**, which no amount of reading the guest's log could have established.
//!
//! ## ★ Fn 64 is answered here too, and refusing it would have cost a second boot
//!
//! `RmRpcSetGuestSystemInfo` tail-calls `NV_RM_RPC_SET_GUEST_SYSTEM_INFO_EXT` and returns
//! **its** status (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:8825-8832`). The guest reads
//! nothing out of that reply — the function's whole contract is the status — so the answer
//! is a bare `NV_OK` over a zeroed body, and `[inferred]` from that source rather than
//! measured, because a run that refuses fn 1 never gets far enough to send it.
//!
//! ## ★★ Both of those are ACCEPTED answers, so: can the guest keep them forever?
//!
//! No, and the argument is structural rather than a property of these two replies. The
//! guest's control cache is populated from an RPC reply at exactly two call sites in the
//! whole driver — `ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11097` and `:11102`
//! (`ogkm-610: :10902`, `:10907`) — and both are inside `rpcRmApiControl_GSP`, i.e. both
//! are reachable only for `NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL`. This policy answers fn 1
//! and fn 64 and returns `None` for everything else, so no reply of its can reach either.
//! [`crate::sticky`] §1a derives that call-site universe by grep rather than by reading,
//! and `tests/tests/sticky_answer.rs` executes the claim against this type.

use kayfabe_abi::NV_ERR_NOT_SUPPORTED;
use kayfabe_abi::guestsysinfo::{
    self, GuestSystemInfoError, VgxVersion, encode_set_guest_system_info_reply,
};
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// `NV_OK`.
const NV_OK: u32 = 0;

/// Answers the version handshake from the driver row's own `VGX_*_VERSION_NUMBER`.
#[derive(Debug, Clone, Copy)]
pub struct GuestSystemInfoPolicy {
    driver: DriverAbiTable,
}

impl GuestSystemInfoPolicy {
    /// Bind the policy to a guest driver version.
    #[must_use]
    pub fn new(driver: DriverAbiTable) -> GuestSystemInfoPolicy {
        GuestSystemInfoPolicy { driver }
    }

    /// The version this port would answer with, or the reason it will not answer at all.
    ///
    /// Exposed so a test can ask the handshake question without building a wire message,
    /// and so each refusal has a name rather than being "some `Reply` with a non-zero
    /// status".
    ///
    /// # Errors
    ///
    /// [`GuestSystemInfoError::Truncated`] for a payload that cannot hold the struct,
    /// [`GuestSystemInfoError::NoVersionForDriver`] where this port has no citation, and
    /// [`GuestSystemInfoError::VersionMismatch`] where the guest speaks something else.
    pub fn agreed_version(&self, payload: &[u8]) -> Result<VgxVersion, GuestSystemInfoError> {
        let guest = guestsysinfo::decode_declared_vgx(payload)?;
        let ours = self
            .driver
            .vgx_version()
            .ok_or(GuestSystemInfoError::NoVersionForDriver)?;
        if guest != ours {
            return Err(GuestSystemInfoError::VersionMismatch { guest, ours });
        }
        Ok(ours)
    }
}

/// A reply that carries no body and a non-zero envelope result — the short-circuit.
fn refuse() -> Option<Reply> {
    Some(Reply {
        rpc_result: NV_ERR_NOT_SUPPORTED,
        body: Vec::new(),
    })
}

impl CommandPolicy for GuestSystemInfoPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        match cmd.function {
            RpcFunction::SetGuestSystemInfo => match self.agreed_version(&cmd.payload) {
                Ok(ours) => Some(Reply {
                    rpc_result: NV_OK,
                    body: encode_set_guest_system_info_reply(ours),
                }),
                Err(_) => refuse(),
            },
            // ⊘ No body of our own: the guest reads only the status off this one. The
            // request's three 256-byte `[IN]` strings are NOT reflected — `Reply` with an
            // empty body zero-fills to the request's own length.
            RpcFunction::SetGuestSystemInfoExt => Some(Reply {
                rpc_result: NV_OK,
                body: Vec::new(),
            }),
            _ => None,
        }
    }
}

kayfabe_util::assert_send_sync!(GuestSystemInfoPolicy);
