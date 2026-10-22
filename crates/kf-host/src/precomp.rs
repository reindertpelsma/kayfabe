//! Startup-only, host-authored PRECOMP reads. No guest control is forwarded here.
//! The stock driver's generic regops endpoint is unprivileged, but its per-register
//! GSP allowlist may refuse display addresses. A refusal is a complete fallback.

/// Public capability layouts have at most32 windows and six PRECOMP word types.
pub const MAX_WORDS: usize = 32 * 6;

/// Read a bounded plan while the caller has no effective CAP_SYS_ADMIN.
/// The callback is the actual host ioctl or a GPU-free fake; both buffers remain
/// live for that synchronous call. Only the authored READ32 operation is permitted.
///
/// # Errors
/// Unknown ABI, privileged caller, invalid plan, ioctl refusal or malformed reply.
pub fn read_with(
    _version: &str,
    _unprivileged: bool,
    _addresses: &[u32],
    _control: impl FnMut(u32, &mut [u8], &mut [u8]) -> Result<(), String>,
) -> Result<Vec<u32>, String> {
    Err("PRECOMP host query is not implemented".into())
}
