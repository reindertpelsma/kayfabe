//! The `gpu-uuid` / `vm-id` device properties: parsed at [`crate::device::Config::check`] (before
//! anything is opened, so a hostile string costs nothing) and resolved at realize into the ONE
//! guest-visible GPU UUID. The mode set, the exact `auto` construction and the rationale are in
//! [`kf_rm::gpuuid`]; this file is only the VMM-side glue, kept GPU-free so it is testable.

use crate::device::Config;
use kf_abi::gspstaticinfo::GpuGid;
use kf_rm::gpuuid::{GpuUuidMode, Inputs, Resolved, parse_vm_id, resolve};

/// The highest PCI `devfn` (8 bits).
const DEVFN_MAX: u32 = 0xff;

/// Parse the properties without resolving them: a malformed `gpu-uuid=`, `vm-id=` or `devfn` is
/// refused here, by name.
///
/// # Errors
/// The refusal.
pub fn check(cfg: &Config) -> Result<(), String> {
    GpuUuidMode::parse(cfg.gpu_uuid.as_deref()).map_err(|e| e.to_string())?;
    parse_vm_id(cfg.vm_id.as_deref()).map_err(|e| e.to_string())?;
    if cfg.pci_devfn > DEVFN_MAX {
        return Err(format!(
            "the device's PCI devfn {:#x} does not fit 8 bits",
            cfg.pci_devfn
        ));
    }
    Ok(())
}

/// Resolve the device's properties and the host GPU's UUID to the guest-visible UUID.
///
/// # Errors
/// The refusal, as text (the realize error).
pub fn resolve_for(
    cfg: &Config,
    host_gid: Option<GpuGid>,
    entropy: &mut dyn FnMut(&mut [u8; 16]) -> Result<(), String>,
) -> Result<Resolved, String> {
    check(cfg)?;
    let inputs = Inputs {
        mode: GpuUuidMode::parse(cfg.gpu_uuid.as_deref()).map_err(|e| e.to_string())?,
        vm_id: parse_vm_id(cfg.vm_id.as_deref()).map_err(|e| e.to_string())?,
        slot: cfg.pci_devfn as u8,
        host_gid,
    };
    resolve(&inputs, entropy).map_err(|e| e.to_string())
}
