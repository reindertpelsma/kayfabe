//! # kayfabe-abi — Axis A: the declarative ABI
//!
//! `mode2_abi_agnostic_layer.md` §2/§4.1. This crate holds the **generated,
//! committed** wire layouts produced by `kayfabe-abi-gen` from the vendored open
//! kernel modules, the driver-version table that selects between them, and the
//! version-independent views the logic crates consume.
//!
//! **The quarantine rule (decision #2):** `#[repr(C)]` NVIDIA wire structs exist
//! ONLY here. The logic crates speak abstract domain types (`kayfabe-arch::ids`,
//! `kayfabe-core::rmgraph::RmEvent`); this crate's decoders translate wire ↔ core
//! at the boundary. A concrete driver version (`V580`, …) or `#[repr(C)]` layout
//! in any logic crate fails review + the grep gate (testing strategy §7 Tier 2).
//!
//! ---
//!
//! # 1. The generation strategy, and the three it is not
//!
//! **Chosen: an offline generator whose output is committed and reviewed.**
//! `crates/kayfabe-abi/gen/` is a standalone cargo project *outside* the
//! workspace, with zero third-party dependencies, run by hand against a vendored
//! open-gpu-kernel-modules checkout:
//!
//! ```text
//! cargo run --manifest-path crates/kayfabe-abi/gen/Cargo.toml -- \
//!     <ogkm-root> crates/kayfabe-abi/src/generated
//! ```
//!
//! | | build.rs generator | bindgen | offline + committed |
//! |---|---|---|---|
//! | ogkm needed to build? | **yes** | **yes**, + a system C front end | **no** |
//! | diff a human can review? | no (build artifact) | no (mechanical blob) | **yes** |
//! | layout tied to build host? | some | **yes** (host target rules) | **no** |
//! | can express version skew? | awkward | **no** | **yes** |
//! | bad parse caught…| at a user's build | at a user's build | **in review** |
//!
//! The decisive constraint is the first row: the open kernel modules must not be
//! a build dependency of anything downstream. That alone rules out `build.rs`
//! and bindgen, and `mode2_abi_agnostic_layer.md` §2.3 already recorded the same
//! ruling ("the generator is a dev tool, not a build dependency, so a bad header
//! parse is caught in review, not at a customer's boot").
//!
//! The second row is why the generator is hand-rolled rather than bindgen-backed
//! even offline. Reviewability is a correctness property here, not a courtesy:
//! the entire L11 incident list is *layout* bugs, and a layout you cannot read
//! is a layout nobody checked. So the emitted code carries a `header:line`
//! provenance comment per field, a `LAYOUT` table, and a `const` block that
//! fails the **build** if the generator's layout disagrees with rustc's.
//!
//! ## Why arm64 and x86_64 need no separate generation
//!
//! Derived, not assumed — see `gen/src/ctype.rs`'s module doc. Every SDK field
//! is a fixed-width NVIDIA typedef; `NvP64` is 8 bytes on both arms of its own
//! `#if`; the `NV_ALIGN_BYTES(8)` attributes exist to fix up **ILP32** and are
//! no-ops on any LP64 target. The generator *enforces* the derivation: an
//! alignment attribute that would raise a field above its natural alignment is a
//! hard error rather than a silently-wrong `#[repr(C)]`.
//!
//! # 2. How version skew is represented
//!
//! [`versions::TABLES`] — nvproxy's inherit-then-mutate registry
//! (`gvisor/pkg/sentry/devices/nvproxy/version.go:142-162`), keyed on the **full**
//! `major.minor.patch` because the real boundaries are mid-major (`NVOS46` grew
//! at 580.65.06, `NVOS47` at 550.54.04). Selection is "newest entry ≤ requested";
//! below the oldest entry is [`wire::AbiError::NoTableForVersion`], never a
//! nearest-neighbour fallback.
//!
//! # 3. Safety
//!
//! `#![forbid(unsafe_code)]` holds. The `#[repr(C)]` mirrors are a layout
//! *oracle* (`size_of` / `offset_of!`), never a cast target: decoding is
//! `from_le_bytes` out of a byte slice. There is no `transmute` and no
//! `bytemuck` anywhere in this crate or its generator.
//!
//! # 4. What is deliberately NOT here
//!
//! - **The wire → `RmEvent` mapping.** This crate does not depend on
//!   `kayfabe-core`. [`view`]'s field names mirror `RmEvent`'s payloads so the
//!   mapping is a rename, but it belongs on the core's side of the seam and the
//!   core is being reshaped concurrently.
//! - **The per-class alloc-param table** beyond `NV01_DEVICE_0`, the GSP-RPC
//!   *payload* structs (213 of them), the UVM ioctls, and the capability
//!   allowlist. Each needs a consumer first; a broad table with one wrong entry
//!   is invisible until a guest trips it.

/// The generated wire layouts. Produced by `kayfabe-abi-gen`; do not edit.
pub mod generated;
pub mod transcribed;
pub mod versions;
pub mod view;
pub mod wire;

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
