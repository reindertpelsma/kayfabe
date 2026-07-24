//! # nvkvm-abi — Axis A: the declarative ABI (SKELETON this milestone)
//!
//! `mode2_abi_agnostic_layer.md` §2/§4.1. This crate will hold the **generated,
//! committed** per-driver-version modules produced by `nvkvm-abi-gen` from the
//! vendored open kernel modules (structs + sizes, class/ctrl/ioctl/RPC IDs,
//! alloc-param sizes, GMMU format constants, register offsets), plus the runtime
//! [`DriverAbi`] dispatch selected from the detected guest driver version.
//!
//! **The quarantine rule (decision #2):** `#[repr(C)]` NVIDIA wire structs exist
//! ONLY here. The logic crates speak abstract domain types (`nvkvm-arch::ids`,
//! `nvkvm-core::rmgraph::RmEvent`); this crate's decoders translate wire ↔ core at
//! the boundary. A concrete driver version (`V580`, …) or `#[repr(C)]` layout in
//! any logic crate fails review + the grep gate (testing strategy §7 Tier 2).
//!
//! Not implemented this milestone by design: the first milestone is the pure-logic
//! core, which must be provably independent of everything that will live here.

use nvkvm_arch::ids::ClassId;

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
pub trait DriverAbi {
    /// The version this table set was generated from.
    fn version(&self) -> DriverVersion;

    /// Generated alloc-param size for `class` (the L11 bug class — cuCtxCreate-401
    /// and three Vulkan gaps were all missing entries). `None` = class not in this
    /// version's table (loud refusal upstream, never a guessed size).
    fn alloc_param_size(&self, class: ClassId) -> Option<usize>;
}
