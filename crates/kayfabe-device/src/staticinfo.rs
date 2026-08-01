//! The command policy that answers `GET_GSP_STATIC_INFO` (fn 65) from a chip row.
//!
//! ## ★★ Why this is here and not in a logic crate
//!
//! Same join as [`crate::inittables`]: the *rows* are a fact about a board
//! ([`crate::ChipProfile::fb_regions`], [`crate::ChipProfile::fb_length`]) and the
//! *layout* is the Axis-A quarantine (`kayfabe_abi::gspstaticinfo`). This crate is where
//! a concrete chip's facts are allowed to meet a wire. Nothing below names a generation,
//! a driver version or an address — a second chip is a second row.
//!
//! ## ★★★ Fn 65 has nowhere to put a refusal, so the refusal is the envelope
//!
//! A `GSP_RM_CONTROL` carries a `status` field, so a policy that cannot serve a command
//! can say so *inside* the reply. Fn 65 carries no such field: the guest copies the body
//! into `pGpu->pGspStaticInfo` and reads it (`ogkm-580:
//! src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:4232-4236`). The only place left to refuse
//! is the RPC envelope's `rpc_result`, which the guest checks first and which makes
//! `NV_RM_RPC_GET_GSP_STATIC_INFO` fail with a line that names itself:
//!
//! ```text
//! NVRM: GET_GSP_STATIC_INFO failed: 0x<status>
//! ```
//!
//! That is strictly better than a well-formed body that is wrong, and it is what an
//! unencodable row gets. ⊘ It is *not* what an unknown function gets — those fall through
//! to the chain, which is a different statement.
//!
//! ## ★★ Fn 65's accepted reply is 1792 bytes the guest keeps — but not in the CACHE
//!
//! The guest copies this body into `pGpu->pGspStaticInfo` and reads it for the rest of the
//! boot, which is a form of stickiness this module already argues about above. It is **not**
//! the `rmapiControlCache` kind, and the two must not be conflated: the cache is populated
//! from an RPC reply at exactly two call sites in the whole driver —
//! `ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11097` and `:11102` (`ogkm-610: :10902`,
//! `:10907`) — both inside `rpcRmApiControl_GSP`, reachable only for
//! `NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL`. Fn 65 is not that function. ⇒ a wrong static-info
//! reply dies with the driver life that read it, where a cached control answer would not
//! even be re-asked. [`crate::sticky`] §1a derives the call-site universe by grep;
//! `tests/tests/sticky_answer.rs` executes the claim against this type.

use kayfabe_abi::NV_ERR_NOT_SUPPORTED;
use kayfabe_abi::gspstaticinfo::{
    GSP_STATIC_CONFIG_INFO_SIZE, GspStaticInfo, encode_gsp_static_info,
};
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

use crate::ChipProfile;

/// `NV_OK`.
const NV_OK: u32 = 0;

/// Answers `GET_GSP_STATIC_INFO` with the static facts a chip row states.
///
/// Every other command gets `None`, i.e. the next link in the chain, or the FSM's own
/// acknowledgement.
#[derive(Debug, Clone, Copy)]
pub struct StaticInfoPolicy {
    chip: &'static ChipProfile,
    driver: DriverAbiTable,
}

impl StaticInfoPolicy {
    /// Bind the policy to a chip row and a guest driver version.
    #[must_use]
    pub fn new(chip: &'static ChipProfile, driver: DriverAbiTable) -> StaticInfoPolicy {
        StaticInfoPolicy { chip, driver }
    }

    /// The body this policy would post, or the reason it cannot.
    ///
    /// Exposed so a test can ask the encoding question without building a wire message —
    /// and so the refusal path has a name that is not "some `Reply` with a non-zero
    /// status".
    ///
    /// # Errors
    ///
    /// Whatever `encode_gsp_static_info` refuses: a region table that contradicts itself
    /// or the chip's own `fb_length`, or a driver version whose struct shape this port
    /// does not encode.
    pub fn body(&self) -> Result<Vec<u8>, kayfabe_abi::gspstaticinfo::GspStaticInfoError> {
        encode_gsp_static_info(
            &GspStaticInfo {
                fb_regions: self.chip.fb_regions,
                fb_length: self.chip.fb_length,
            },
            self.driver.gsp_static_info_wire(),
        )
    }
}

impl CommandPolicy for StaticInfoPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::GetGspStaticInfo {
            return None;
        }
        // ★★★ **The size check, and the reason it is not optional.**
        //
        // Every `InitTablePolicy` control refuses on `req.params_size != want.params_size()`
        // — the guest's own declared size against ours. Fn 65 carries no `paramsSize` field
        // to compare, so the equivalent statement is the *body length* the guest's envelope
        // declares, and this policy compared nothing at all.
        //
        // ⚠ That was silent in the worst possible way. `RpcCommand::reply` clamps the body
        // to whatever the request declared, so a disagreement produces neither a fault nor a
        // counter nor a refusal — only a `GspStaticConfigInfo` that is **truncated** (guest
        // struct smaller) or **zero-padded** (guest struct larger), copied wholesale into
        // `pGpu->pGspStaticInfo` and read from there for the rest of the boot
        // (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:4232-4236`). Nothing logs.
        //
        // ★★ And there is **zero margin**: `1824 - 32 == 1792` exactly. This port's
        // `GSP_STATIC_CONFIG_INFO_SIZE` and the guest's `sizeof(GspStaticConfigInfo)` are
        // *the same fact stated twice* — by two teams, from two sources, keyed on a driver
        // version. When they disagree the whole reply is wrong, and the disagreement is
        // exactly what a differently-built guest brings. So the refusal goes in the
        // envelope, where the guest's own `NV_RM_RPC_GET_GSP_STATIC_INFO` fails loudly with
        // a line that names itself, rather than into a body that cannot carry one.
        if cmd.payload.len() != GSP_STATIC_CONFIG_INFO_SIZE {
            return Some(Reply {
                rpc_result: NV_ERR_NOT_SUPPORTED,
                body: Vec::new(),
            });
        }
        match self.body() {
            Ok(body) => Some(Reply {
                rpc_result: NV_OK,
                body,
            }),
            Err(_) => Some(Reply {
                rpc_result: NV_ERR_NOT_SUPPORTED,
                body: Vec::new(),
            }),
        }
    }
}

kayfabe_util::assert_send_sync!(StaticInfoPolicy);
