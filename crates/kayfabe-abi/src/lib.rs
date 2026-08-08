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
//! ## 2.1 ★ The guest OS is a SECOND key, and it lives beside this one
//!
//! [`GuestOs`] ([`guest_os`]) — `four_axes_of_variation.md` §1's fourth axis, given a
//! home on 2026-07-29. It is deliberately **not** a field of a
//! [`versions::DriverAbiTable`]: the same doc's *"do not collapse guest OS into the
//! version key"* is the C's major-only version-key mistake one level up, a single key
//! silently spanning two independent dimensions. Every rule that is conditioned on which
//! OS built the guest driver hangs off that type, and there is exactly one such rule
//! today — [`GuestOs::client_kind_from_process_id`], whose sentinel is gated on
//! `RMCFG_FEATURE_PLATFORM_UNIX` in the driver.
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
pub mod bifstatic;
pub mod bringup;
pub mod businfo;
pub mod capability;
pub mod chipinfo;
pub mod confcompute;
pub mod deviceinfo;
pub mod eventnotify;
pub mod falconinfo;
pub mod faultbuffer;
pub mod fifochannels;
pub mod fmbsize;
pub mod generated;
pub mod gmmustatic;
pub mod gpuatomics;
pub mod gpuinfo;
pub mod grinfo;
pub mod grstatic;
pub mod gspstaticinfo;
pub mod guest_os;
pub mod guestsysinfo;
pub mod gvaspacepdes;
pub mod host_driver;
pub mod inittables;
// ★ #156 — the ⊘ half of the host-class seam: the three classes that do NOT vary,
// named by role so a name-based gate can tell them from the three that do.
pub mod invariant_classes;
pub mod l2evict;
pub mod memsysconfig;
pub mod notifier;
pub mod oracle;
pub mod pcibars;
pub mod rc;
pub mod regaccessmap;
pub mod smcmode;
pub mod submit;
pub mod transcribed;
pub mod vbios;
pub mod versions;
pub mod view;
pub mod wire;

pub use guest_os::{
    ClientKindRule, ClientKindRuleUnknown, GUEST_OS_CONFIG_NAMES, GuestOs, UnknownGuestOsName,
};

use kayfabe_arch::ids::ClassId;

/// `RMAPI_RPC_FLAGS_SERIALIZED` = `NVBIT(1)`
/// (`ogkm-580: src/nvidia/inc/kernel/rmapi/rmapi.h:163` /
/// `ogkm-610: src/nvidia/inc/kernel/rmapi/rmapi.h:163` — same line at both).
///
/// An NVIDIA wire constant, so per the quarantine rule (decision #2) it exists **only in
/// this crate**; the crates above it speak [`rpc_params_are_serialized`].
const RMAPI_RPC_FLAGS_SERIALIZED: u32 = 1 << 1;

/// Does this RPC's `flags` word declare that `params[]` is **FINN-serialized**?
///
/// `[src]` `rpcRmApiAlloc_GSP` sets the bit when `serverSerializeAllocDown` reports a
/// serialized payload (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11212-11216` /
/// `ogkm-610: src/nvidia/src/kernel/vgpu/rpc.c:11018-11022` — same code, moved); the control
/// path has the same bit on `rmapiRpcFlags` (`ogkm-580: rpc.c:11000-11001` /
/// `ogkm-610: rpc.c:10805-10806`).
///
/// ★ **Why any caller must ask.** When the bit is set, `params[]` is *not* the flat
/// `#[repr(C)]` struct, and every per-class offset a decoder would use is wrong. So this
/// is the predicate a refusal fires on — a **declared bit**, never a length heuristic
/// (`docs/design/gsp_core_bridge.md` §2.2c). Which classes actually set it is
/// `[unverified]` (§7 item 3): refusing them by name is safe, and the day a boot-path
/// class turns out to be serialized, this predicate is where that is discovered.
///
/// One total function of one declared field, exactly like [`GuestOs::client_kind_from_process_id`].
#[must_use]
pub fn rpc_params_are_serialized(flags: u32) -> bool {
    flags & RMAPI_RPC_FLAGS_SERIALIZED != 0
}

/// The bottom of the **VMIOP/RPC** result range — `NV_VGPU_MSG_RESULT__VMIOP` is
/// `0xFF00000a:0xFF000000` (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc_headers.h:121` /
/// `ogkm-610: src/nvidia/inc/kernel/vgpu/rpc_headers.h:122`), and the header states the rule
/// directly at `ogkm-580: rpc_headers.h:66` / `ogkm-610: rpc_headers.h:66`: *"codes below
/// 0xFF000000 must match exactly the NV_STATUS codes in nvos.h"*.
///
/// ★★ **This is the constraint every reply status we invent must satisfy**, and it is a
/// behaviour of the guest, not a convention. `_issueRpcAndWait` reads our `rpc_result` and
/// maps it (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:1993-2008` /
/// `ogkm-610: src/nvidia/src/kernel/vgpu/rpc.c:2011-2026` — same five arms; 610 spells the
/// header access `pVgpuRpcHeader->` where 580 uses the `vgpu_rpc_message_header_v` macro):
///
/// ```text
/// rpc_result == 0                        -> NV_OK
/// rpc_result == 0xFF100009               -> returned VERBATIM (the one special case)
/// rpc_result <  NV_VGPU_MSG_RESULT_VMIOP_BASE -> returned VERBATIM as the NV_STATUS
/// otherwise                              -> COLLAPSED to NV_ERR_GENERIC
/// ```
///
/// So a status at or above this base is a status the guest **cannot see**: every distinct
/// value in that range arrives at the RM caller as one indistinguishable `NV_ERR_GENERIC`.
/// A refusal that wants to say anything at all must pick a value below it.
pub const NV_VGPU_MSG_RESULT_VMIOP_BASE: u32 = 0xFF00_0000;

/// `NV_ERR_NOT_SUPPORTED` — `NV_STATUS` `0x56`
/// (`ogkm-580: src/common/sdk/nvidia/inc/nvstatuscodes.h:115` /
/// `ogkm-610: src/common/sdk/nvidia/inc/nvstatuscodes.h:115` — same line at both).
///
/// ★ **Not cosmetic.** RM sets `RMAPI_PARAM_COPY_FLAGS_SKIP_COPYOUT` on this status, i.e.
/// the value changes what the *guest* does with its own params buffer — which the C
/// artifact learned expensively: faking `NV_OK` for a control the real driver answers
/// `0x56` copied 1464 bytes of garbage over the CUDA user library's ECC buffer and
/// produced the `rbp=0` SIGSEGV
/// (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2883-2894`). It is also the only
/// NV_STATUS the C ever deliberately fails with.
///
/// # ★★ B4 settled `gsp_core_bridge.md` §4.2's `[open]`: this value, and one table
///
/// The `[open]` asked which `NV_STATUS` a refusal should carry, and treated "the C used
/// it" as the only argument. There are now three real ones, all `[src]`:
///
/// 1. **It reaches the guest at all.** `0x56 < NV_VGPU_MSG_RESULT_VMIOP_BASE`, so
///    `_issueRpcAndWait` returns it verbatim rather than collapsing it to
///    `NV_ERR_GENERIC` (`ogkm-580: rpc.c:2004-2005` / `ogkm-610: rpc.c:2023-2024`).
///    See that constant.
/// 2. **NVIDIA expects it from a GSP control.** `rpcRmApiControl_GSP`'s own error path
///    lists `NV_ERR_NOT_SUPPORTED` (with `NV_ERR_OBJECT_NOT_FOUND`) as the statuses to log
///    *quietly* (`ogkm-580: rpc.c:11109-11115` / `ogkm-610: rpc.c:10914-10920`) — a refusal
///    carrying it is an outcome the driver already treats as ordinary, not an anomaly.
/// 3. **The tempting alternative is wrong on this path.**
///    `NV_VGPU_MSG_RESULT_RPC_API_CONTROL_NOT_SUPPORTED` (`0xFF100009`) is NVIDIA's own
///    "recognised but not supported" control status and survives the collapse by explicit
///    special case (`ogkm-580: rpc.c:1996-1997` / `ogkm-610: rpc.c:2014-2015`) — but the
///    translation back to a real `NV_STATUS` lives **only** in `rpcRmApiControl_wrapper`
///    (`ogkm-580: rpc.c:5425-5430` / `ogkm-610: rpc.c:5432-5437`), which is the vGPU
///    `RM_API_CONTROL` path.
///    `rpcRmApiControl_GSP` — fn 76, the one a GSP-client guest actually sends — has no
///    such arm, so `0xFF100009` would propagate to the RM caller *as* an `NV_STATUS`,
///    which it is not.
///
/// It therefore stays **one named value** rather than becoming a per-refusal table: the
/// three facts above constrain the choice, and nothing observed constrains a *split*. A
/// table of 200 status codes with no consumer is the breadth this crate's §4 refuses.
pub const NV_ERR_NOT_SUPPORTED: u32 = 0x0000_0056;

/// A guest driver version, as detected/advertised at device realize.
/// (Values are data, not code: one generated module per version.)
///
/// ★ **Guest.** The host's driver version is a *different* number about a different
/// machine, and the product property is that the two need not agree
/// (`host_driver_version_pin.md`). It has its own type,
/// [`host_driver::HostDriverVersion`], so neither can be passed where the other is meant.
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

    // ★ `only_the_kernel_sentinel_decodes_to_a_kernel_client` MOVED on 2026-07-29, to
    // `guest_os.rs`'s own test module, together with the function it is about. It now
    // reads `GuestOs::Linux.client_kind_from_process_id`, and it has a sibling asserting
    // that the same inputs REFUSE under a profile whose rule we do not have.

    /// `RMAPI_RPC_FLAGS_SERIALIZED` is **bit 1**, so the neighbouring bit must not fire
    /// the predicate and the bit must be found under any amount of surrounding noise.
    /// Kills `1<<1`→`1<<0`, `&`→`|`, `!=0`→`==0` and the whole-function stubs.
    #[test]
    fn only_bit_one_declares_serialized_params() {
        // RMAPI_RPC_FLAGS_NONE (0) and COPYOUT_ON_ERROR (NVBIT(0)) are the two other
        // values `rpc.c` writes into this field — neither means "serialized".
        assert!(!rpc_params_are_serialized(0x0000_0000));
        assert!(!rpc_params_are_serialized(0x0000_0001));
        assert!(!rpc_params_are_serialized(0xFFFF_FFFD));
        assert!(rpc_params_are_serialized(0x0000_0002));
        assert!(rpc_params_are_serialized(0x0000_0003));
        assert!(rpc_params_are_serialized(0xFFFF_FFFF));
    }
}
