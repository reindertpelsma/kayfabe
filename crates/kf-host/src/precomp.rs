//! Startup-only, host-authored PRECOMP reads. No guest control is forwarded here.
//! The stock driver's generic regops endpoint is unprivileged, but its per-register
//! GSP allowlist may refuse display addresses. A refusal is a complete fallback.
//!
//! This compound pointer ABI is encoded directly in the exact HOST layout measured
//! by compiling the public SDK, including its pointed-to rows. It does not use the
//! bench-struct carry of ordinary scalar HostRm controls. Unsupported versions are
//! refused before the ioctl; there is no opaque/nearest-version bypass.

#[path = "precomp_layout.rs"]
mod layout;

/// Public capability layouts have at most32 windows and six PRECOMP word types.
pub const MAX_WORDS: usize = 32 * 6;
const BATCH_WORDS: usize = 32;

/// Read a bounded plan while the caller has no effective CAP_SYS_ADMIN.
/// The callback is the actual host ioctl or a GPU-free fake; both buffers remain
/// live for that synchronous call. Only the authored READ32 operation is permitted.
///
/// # Errors
/// Unknown ABI, privileged caller, invalid plan, ioctl refusal or malformed reply.
pub fn read_with(
    version: &str,
    unprivileged: bool,
    addresses: &[u32],
    control: impl FnMut(u32, &mut [u8], &mut [u8]) -> Result<(), String>,
) -> Result<Vec<u32>, String> {
    if !unprivileged {
        return Err("PRECOMP host query requires CAP_SYS_ADMIN clear".into());
    }
    read_checked(version, addresses, control)
}

/// Privileged, read-only sweep measurement, separate from the runtime tier-2 API.
/// No product caller uses this entry point and a refusal is never retried with writes.
///
/// # Errors
/// Missing effective CAP_SYS_ADMIN, unknown ABI, invalid plan or any host refusal.
pub fn measure_with(
    version: &str,
    privileged: bool,
    addresses: &[u32],
    control: impl FnMut(u32, &mut [u8], &mut [u8]) -> Result<(), String>,
) -> Result<Vec<u32>, String> {
    if !privileged {
        return Err("PRECOMP measurement requires effective CAP_SYS_ADMIN".into());
    }
    read_checked(version, addresses, control)
}

fn read_checked(
    version: &str,
    addresses: &[u32],
    mut control: impl FnMut(u32, &mut [u8], &mut [u8]) -> Result<(), String>,
) -> Result<Vec<u32>, String> {
    let l = layout::for_version(version)
        .ok_or_else(|| format!("PRECOMP host query has no compiled ABI for {version}"))?;
    if addresses.is_empty()
        || addresses.len() > MAX_WORDS
        || addresses.iter().any(|a| !a.is_multiple_of(4))
    {
        return Err("PRECOMP host plan is empty, oversized or unaligned".into());
    }
    let mut result = Vec::with_capacity(addresses.len());
    for batch in addresses.chunks(BATCH_WORDS) {
        let mut ops = vec![0; l.op_size * batch.len()];
        for (op, address) in ops.chunks_exact_mut(l.op_size).zip(batch) {
            op[l.operation] = l.read32;
            op[l.kind] = l.global;
            op[l.status] = l.no_access; // A host that never filled a row cannot fabricate success.
            op[l.offset..l.offset + 4].copy_from_slice(&address.to_le_bytes());
        }
        let requested_ops = ops.clone();
        let mut params = vec![0; l.param_size]; // No target channel, write mask, route or privilege flags.
        let count = u32::try_from(batch.len()).map_err(|_| "PRECOMP count overflow")?;
        params[l.count..l.count + 4].copy_from_slice(&count.to_le_bytes());
        params[l.pointer..l.pointer + 8].copy_from_slice(&(ops.as_mut_ptr() as u64).to_le_bytes());
        let requested_params = params.clone();
        control(l.cmd, &mut params, &mut ops)?;
        if params != requested_params {
            return Err("PRECOMP host changed request metadata".into());
        }
        for (op, requested) in ops
            .chunks_exact(l.op_size)
            .zip(requested_ops.chunks_exact(l.op_size))
        {
            if op[l.status] != l.success {
                return Err(format!(
                    "PRECOMP host register refusal: status {}",
                    op[l.status]
                ));
            }
            if op
                .iter()
                .zip(requested)
                .enumerate()
                .any(|(i, (got, want))| {
                    i != l.status && !(l.value..l.value + 4).contains(&i) && got != want
                })
            {
                return Err("PRECOMP host changed register metadata".into());
            }
            let bytes: [u8; 4] = op[l.value..l.value + 4]
                .try_into()
                .map_err(|_| "PRECOMP short register reply")?;
            result.push(u32::from_le_bytes(bytes));
        }
    }
    Ok(result)
}

impl crate::HostRm {
    /// Read the own die's public capability plan for a privileged sweep only.
    /// This is measurement evidence, not a runtime fallback or guest control path.
    ///
    /// # Errors
    /// Missing admin capability, unsupported ABI, invalid plan or host refusal.
    pub fn measure_precomp_capabilities(&self, addresses: &[u32]) -> Result<Vec<u32>, String> {
        let caps = kf_linux_raw::capability::current_thread_caps()
            .map_err(|e| format!("PRECOMP measurement capget refused: {e}"))?;
        measure_with(
            self.driver_version(),
            caps.holds_effective(kf_linux_raw::capability::CAP_SYS_ADMIN),
            addresses,
            |cmd, params, _ops| {
                self.raw_control_exact(self.subdevice(), cmd, params)
                    .map_err(|e| format!("PRECOMP measured own-subdevice control: {e:?}"))
            },
        )
    }

    /// Ask only at realize, using a plan authored from public display headers on
    /// this session's own die. No guest-provided offset, flags or channel handle.
    ///
    /// # Errors
    /// Capability inspection, unknown ABI, host/permission refusal or malformed reply.
    pub fn read_precomp_capabilities(&self, addresses: &[u32]) -> Result<Vec<u32>, String> {
        let caps = kf_linux_raw::capability::current_thread_caps()
            .map_err(|e| format!("PRECOMP capget refused: {e}"))?;
        read_with(
            self.driver_version(),
            !caps.holds_effective(kf_linux_raw::capability::CAP_SYS_ADMIN),
            addresses,
            |cmd, params, _ops| {
                self.raw_control_exact(self.subdevice(), cmd, params)
                    .map_err(|e| format!("PRECOMP own-subdevice control: {e:?}"))
            },
        )
    }
}
