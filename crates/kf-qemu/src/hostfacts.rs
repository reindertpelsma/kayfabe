//! ★ Per-die facts READ FROM THE HOST — the PCI identity the guest binds on and the PCIe link the
//! device presents. Never a table row: `derive_per_die_maintain_per_family` (owner, 2026-09-13).
//!
//! Source: the NVIDIA driver's own procfs (`/proc/driver/nvidia/gpus/<bdf>/information`, which names
//! the device minor) to find the BDF of the GPU our session opened, then that function's sysfs.

use std::path::{Path, PathBuf};

/// The host GPU's PCI identity and link, as the guest will see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPci {
    /// Vendor id.
    pub vendor: u16,
    /// Device id.
    pub device: u16,
    /// Subsystem vendor id.
    pub subsystem_vendor: u16,
    /// Subsystem id.
    pub subsystem: u16,
    /// Class code (24 bits).
    pub class: u32,
    /// Revision id.
    pub revision: u8,
    /// BAR0 size in bytes (`resource0`).
    pub bar0_bytes: u64,
    /// ★ The host's BAR1 aperture in bytes (sysfs `resource` line 1) — the pool every kf3 device
    /// on this card takes views from ([`crate::cardbudget`]). Queried, never a literal (ReBAR and
    /// datacenter parts differ).
    pub bar1_bytes: u64,
    /// The PCIe generation of the link's maximum speed.
    pub max_gen: kf_abi::businfo::PcieGen,
    /// ★ The link's maximum width in lanes (sysfs `max_link_width`) — ⊘ not a presented ×16
    /// (an AD106 is ×8; RM derives the guest's UVM link bandwidth from it).
    pub max_width: u32,
}

fn read_hex(p: &Path) -> Result<u64, String> {
    let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    u64::from_str_radix(s.trim().trim_start_matches("0x"), 16).map_err(|e| format!("{}: {e}", p.display()))
}

/// The sysfs directory of the GPU at device minor `minor`.
///
/// # Errors
/// No GPU with that minor in `/proc/driver/nvidia/gpus`.
pub fn sysfs_dir_for_minor(minor: u32) -> Result<PathBuf, String> {
    let root = Path::new("/proc/driver/nvidia/gpus");
    let entries = std::fs::read_dir(root).map_err(|e| format!("{}: {e}", root.display()))?;
    for e in entries.flatten() {
        let info = std::fs::read_to_string(e.path().join("information")).unwrap_or_default();
        let this = info
            .lines()
            .find_map(|l| l.strip_prefix("Device Minor:"))
            .and_then(|v| v.trim().parse::<u32>().ok());
        if this == Some(minor) {
            return Ok(Path::new("/sys/bus/pci/devices").join(e.file_name()));
        }
    }
    Err(format!("no GPU with device minor {minor} under {}", root.display()))
}

/// Read the host GPU's identity from `dir` (a `/sys/bus/pci/devices/<bdf>`).
///
/// # Errors
/// A missing or unparsable sysfs attribute, by name.
pub fn read_host_pci(dir: &Path) -> Result<HostPci, String> {
    let h = |n: &str| read_hex(&dir.join(n));
    let resource = std::fs::read_to_string(dir.join("resource")).map_err(|e| format!("resource: {e}"))?;
    let span = |line: Option<&str>| -> u64 {
        let mut f = line
            .unwrap_or("")
            .split_whitespace()
            .map(|x| u64::from_str_radix(x.trim_start_matches("0x"), 16).unwrap_or(0));
        let (start, end) = (f.next().unwrap_or(0), f.next().unwrap_or(0));
        if end > start { end - start + 1 } else { 0 }
    };
    let mut lines = resource.lines();
    let bar0 = lines.next().ok_or("resource: empty")?;
    let bar1_bytes = span(lines.next());
    if bar1_bytes == 0 {
        return Err("resource: the host GPU reports no BAR1 aperture".to_string());
    }
    let speed = std::fs::read_to_string(dir.join("max_link_speed")).unwrap_or_default();
    // "16.0 GT/s PCIe" → Gen4. The encoder rejects nothing; an unknown speed is refused here.
    let max_gen = match speed.split_whitespace().next().unwrap_or("") {
        "2.5" => kf_abi::businfo::PcieGen::Gen1,
        "5.0" => kf_abi::businfo::PcieGen::Gen2,
        "8.0" => kf_abi::businfo::PcieGen::Gen3,
        "16.0" => kf_abi::businfo::PcieGen::Gen4,
        "32.0" => kf_abi::businfo::PcieGen::Gen5,
        other => return Err(format!("max_link_speed {other:?} is not a PCIe generation this tree encodes")),
    };
    // "8" → ×8. Decimal in sysfs (`max_link_width`); a width PCIe does not define is refused
    // where the link word is built (`kf_chip::bar0::pcie_link_caps`).
    let width = std::fs::read_to_string(dir.join("max_link_width")).map_err(|e| format!("max_link_width: {e}"))?;
    let max_width =
        width.trim().parse::<u32>().map_err(|e| format!("max_link_width {:?}: {e}", width.trim()))?;
    Ok(HostPci {
        vendor: h("vendor")? as u16,
        device: h("device")? as u16,
        subsystem_vendor: h("subsystem_vendor")? as u16,
        subsystem: h("subsystem_device")? as u16,
        class: h("class")? as u32,
        revision: h("revision")? as u8,
        bar0_bytes: span(Some(bar0)),
        bar1_bytes,
        max_gen,
        max_width,
    })
}
