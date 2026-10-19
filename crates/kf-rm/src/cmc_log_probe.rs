//! Diagnostic-only Blackwell Windows B1 rank2 probe, default off.
//!
//! Public580.65.06 ctrl2080internal.h declares a89's three fields pure [In].
//! This probe tests refusal sensitivity; it does not implement a firmware logger.
//! Guest buffer addresses are never mapped, dereferenced, retained or forwarded.
//! Compiler check of public tag580.65.06 (307159f2623d): command0x20800a89,
//! sizeof24, PublicId@0/logBufferSize@8/logBufferAddr@16. Only that ABI is admitted.
//! This is an emulated diagnostic registration: no physical CMC work or GPU completion
//! is promised. A responsive guest does not prove that its closed driver never polls
//! the buffer; that remains a separate observation, never inferred from this echo.
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// Host-side opt-in; absent from the production flag set.
pub const FLAG: &str = "KF3_CMC_LOG_BUFFER_PROBE";
/// Whether this diagnostic was explicitly requested.
#[must_use]
pub fn enabled() -> bool {
    std::env::var(FLAG).as_deref() == Ok("1")
}

/// A probe for one public guest ABI.
#[derive(Debug, Clone)]
pub struct CmcLogProbe {
    driver: kf_abi::versions::DriverAbiTable,
}
impl CmcLogProbe {
    /// No guest addresses or variable-size state are retained.
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> Self {
        Self { driver }
    }
}
impl CommandPolicy for CmcLogProbe {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        // Layout verified with the public580.65.06 C header: size24, fields0/8/16.
        // No branch/family wildcard: unverified public ABIs remain inert.
        if self.driver.driver_version()
            != (kf_abi::DriverVersion {
                major: 580,
                minor: 65,
                patch: 6,
            })
            || cmd.function != RpcFunction::RmControl
        {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        if req.cmd != 0x2080_0a89
            || req.params_size != 24
            || kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags)
        {
            return None;
        }
        let end = req.params_at.checked_add(24)?;
        // Refuse oversized or truncated payloads before any allocation. A successful
        // reply copies exactly the public header plus24 input bytes, never guest sizes.
        if cmd.payload.len() != end {
            return None;
        }
        let mut body = cmd.payload.clone();
        let status = self.driver.rm_control_wire().status_off;
        body.get_mut(status..status.checked_add(4)?)?.fill(0);
        Some(Reply {
            rpc_result_private: None,
            rpc_result: 0,
            body,
        })
    }
}
