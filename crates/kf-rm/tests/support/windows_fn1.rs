//! `SET_GUEST_SYSTEM_INFO` fixtures for the Windows-profile tests: what a Windows 580.88 guest
//! declares at fn 1, and what the Linux 580.65.06 build declares. Test-authored bytes only; nothing
//! here is a host-authored act.
#![allow(dead_code)]

use kf_gsp::{RpcCommand, RpcFunction};

/// The wire table of the measured Windows twin's layout (580.65.06).
pub fn abi() -> kf_abi::versions::DriverAbiTable {
    *kf_abi::versions::table_for(kf_abi::DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    })
    .expect("580.65.06 has a wire table")
}

fn fn1(w: &kf_abi::versions::DriverAbiTable, name: &str, branch: &str, cl: u32) -> RpcCommand {
    use kf_abi::guestsysinfo::*;
    let mut p = vec![0; SET_GUEST_SYSTEM_INFO_SIZE];
    let vgx = w.vgx_version().expect("vgx pair");
    p[VGX_MAJOR_OFF..VGX_MAJOR_OFF + 4].copy_from_slice(&vgx.major.to_le_bytes());
    p[VGX_MINOR_OFF..VGX_MINOR_OFF + 4].copy_from_slice(&vgx.minor.to_le_bytes());
    p[GUEST_DRIVER_VERSION_OFF..GUEST_DRIVER_VERSION_OFF + name.len()]
        .copy_from_slice(name.as_bytes());
    p[GUEST_VERSION_OFF..GUEST_VERSION_OFF + branch.len()].copy_from_slice(branch.as_bytes());
    p[GUEST_CL_NUM_OFF..GUEST_CL_NUM_OFF + 4].copy_from_slice(&cl.to_le_bytes());
    RpcCommand {
        function: RpcFunction::SetGuestSystemInfo,
        code: 0,
        sequence: 1,
        payload: p,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// fn 1 of the measured Windows build: it names the Windows driver (`580.88`) and its branch and
/// changelist, which `kf_abi::windows_twin` maps to the 580.65.06 wire layout.
pub fn windows(w: &kf_abi::versions::DriverAbiTable) -> RpcCommand {
    let twin = kf_abi::generated::windows_twins::WINDOWS_TWINS
        .iter()
        .find(|t| t.win_name == "580.88")
        .expect("the measured Windows twin");
    fn1(w, twin.win_name, twin.win_branch, twin.win_cl)
}

/// fn 1 of a Linux guest of the same wire version: it names the Linux driver itself.
pub fn linux(w: &kf_abi::versions::DriverAbiTable) -> RpcCommand {
    fn1(w, &w.driver_version().to_string(), "", 0)
}
