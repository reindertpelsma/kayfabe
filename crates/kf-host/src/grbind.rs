//! Existing unsupported USER bind outcome exposed for a tests-first production adapter.

use crate::Channel;
use kf_abi::grbind::Request;

/// Ordinary host effects. Tests use an independent recording fake.
pub trait Host {
    /// Native minimum size/alignment for a generated engine identifier.
    fn properties(&mut self, engine: u32) -> Result<(u64, u64), u32>;
    /// An authored request whose handles have been replaced with owned handles.
    fn bind(&mut self, request: Request) -> Result<(), u32>;
}

/// Unsupported baseline; the RED tests specify the safe implementation before it is added.
pub fn execute(
    _host: &mut impl Host,
    _request: Request,
    _client: u32,
    _channel: Channel,
    _mapped: impl FnMut(u64, u64) -> bool,
) -> Result<u64, u32> {
    Err(0x56)
}
