//! Diagnostic-only Blackwell Windows B1 rank2 probe, default off.
//!
//! Public580.65.06 ctrl2080internal.h declares a89's three fields pure [In].
//! This probe tests refusal sensitivity; it does not implement a firmware logger.
//! Guest buffer addresses are never mapped, dereferenced, retained or forwarded.
use kf_gsp::{CommandPolicy, Reply, RpcCommand};

/// Host-side opt-in; absent from the production flag set.
pub const FLAG: &str = "KF3_CMC_LOG_BUFFER_PROBE";
/// Whether this diagnostic was explicitly requested.
#[must_use]
pub fn enabled() -> bool { std::env::var(FLAG).as_deref() == Ok("1") }

/// A probe for one public guest ABI.
#[derive(Debug, Clone)]
pub struct CmcLogProbe { driver: kf_abi::versions::DriverAbiTable }
impl CmcLogProbe {
    /// No guest addresses or variable-size state are retained.
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> Self { Self { driver } }
}
impl CommandPolicy for CmcLogProbe {
    fn respond(&mut self, _cmd: &RpcCommand) -> Option<Reply> {
        let _ = self.driver;
        None
    }
}
