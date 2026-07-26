//! # kayfabe-abi — Axis A: the declarative ABI (SKELETON this milestone)
//!
//! `mode2_abi_agnostic_layer.md` §2/§4.1. This crate will hold the **generated,
//! committed** per-driver-version modules produced by `kayfabe-abi-gen` from the
//! vendored open kernel modules (structs + sizes, class/ctrl/ioctl/RPC IDs,
//! alloc-param sizes, GMMU format constants, register offsets), plus the runtime
//! [`DriverAbi`] dispatch selected from the detected guest driver version.
//!
//! **The quarantine rule (decision #2):** `#[repr(C)]` NVIDIA wire structs exist
//! ONLY here. The logic crates speak abstract domain types (`kayfabe-arch::ids`,
//! `kayfabe-core::rmgraph::RmEvent`); this crate's decoders translate wire ↔ core at
//! the boundary. A concrete driver version (`V580`, …) or `#[repr(C)]` layout in
//! any logic crate fails review + the grep gate (testing strategy §7 Tier 2).
//!
//! Not implemented this milestone by design: the first milestone is the pure-logic
//! core, which must be provably independent of everything that will live here.

use kayfabe_arch::ClientKind;
use kayfabe_arch::ids::ClassId;

/// `KERNEL_PID` — the reserved `processID` RM stamps on a **kernel-privileged** client's
/// `NV01_ROOT` alloc params (`ogkm: src/nvidia/inc/kernel/vgpu/rpc.h:67-77`:
/// `privLevel >= RS_PRIV_LEVEL_KERNEL → processID = KERNEL_PID`, else the client's own
/// `ProcID`).
///
/// This is an NVIDIA wire constant, so per the quarantine rule (decision #2) it exists
/// **only in this crate**; the logic crates speak [`ClientKind`].
const KERNEL_PID: u32 = 0xFFFF_FFFF;

/// Decode a declared `processID` from `NV0000_ALLOC_PARAMETERS` into the abstract
/// [`ClientKind`] the core groups on (`l1_concurrency.md` §12.27).
///
/// This is the whole wire→domain translation for decision #14's grouping rule, and it is
/// deliberately one total function of one declared field: no handle-range test, no
/// `processName` sniffing, no dup-graph inference. Measured shape (RTX 3060 / 580.159.04):
/// the two concurrent CUDA processes' clients declared their own pids, while the single
/// UVM session client and every other RM-internal client declared [`KERNEL_PID`].
#[must_use]
pub fn client_kind_from_process_id(process_id: u32) -> ClientKind {
    if process_id == KERNEL_PID {
        ClientKind::Kernel
    } else {
        ClientKind::User { pid: process_id }
    }
}

/// A guest driver version, as detected/advertised at device realize.
/// (Values are data, not code: one generated module per version.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DriverVersion {
    /// Major (e.g. 580).
    pub major: u16,
    /// Minor.
    pub minor: u16,
    /// Patch.
    pub patch: u16,
}

/// # Axis-A runtime dispatch (shape only this milestone)
///
/// One impl per generated driver version (nvproxy-style versioned tables with
/// inherit-then-mutate deltas). The four branch points mirror nvproxy's proven
/// decomposition: frontend ioctls, UVM ioctls, control commands, alloc classes —
/// each handler carrying its generated param size + capability allowlist entry
/// (closing the default-allow gap, `nvproxy_gap_analysis`).
///
/// `Send + Sync` supertrait (decision #17): like `Arch`, a driver-version table set
/// is immutable shared data, selected once at realize and stored core-side.
pub trait DriverAbi: Send + Sync {
    /// The version this table set was generated from.
    fn version(&self) -> DriverVersion;

    /// Generated alloc-param size for `class` (the L11 bug class — cuCtxCreate-401
    /// and three Vulkan gaps were all missing entries). `None` = class not in this
    /// version's table (loud refusal upstream, never a guessed size).
    fn alloc_param_size(&self, class: ClassId) -> Option<usize>;
}

// The concurrency contract, compile-time-asserted (decision #17).
kayfabe_util::assert_send_sync!(DriverVersion, dyn DriverAbi);

#[cfg(test)]
mod tests {
    use super::*;

    /// The `KERNEL_PID` sentinel is the ONLY thing that makes a client kernel-privileged,
    /// and every other value — including the numerically adjacent ones and the handle-like
    /// values that seeded the old mis-reading — is a user pid. Kills the `==`→`!=`,
    /// `==`→`>=` and constant mutants in one predicate.
    #[test]
    fn only_the_kernel_sentinel_decodes_to_a_kernel_client() {
        assert_eq!(client_kind_from_process_id(0xFFFF_FFFF), ClientKind::Kernel);
        for pid in [
            0u32,
            1,
            0x0000_dd13, // process A, measured
            0x0000_dd14, // process B, measured
            0xFFFF_FFFE, // adjacent below the sentinel
            0x7FFF_FFFF, // sign-bit boundary
            0xc1d0_0069, // ★ the UVM session's HANDLE — not its processID
        ] {
            assert_eq!(
                client_kind_from_process_id(pid),
                ClientKind::User { pid },
                "processID {pid:#x} declares a user client",
            );
        }
    }
}
