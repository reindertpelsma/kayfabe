//! ★★★★★ **v3 `kf-core` — the plane, composed.** (`THE_ARCHITECTURE_v3.md` §1-§3, §5, §7, §8)
//!
//! The vCPU trap (`kf_trap::TrapPath`) on one side, the host behind [`plane::HostOps`] on the
//! other: workers (`Plane::worker_pass`) serve rung Translated/Emulated tokens, ONE register drainer
//! (`Plane::drainer_pass`) applies privileged writes in order, and completions follow §8
//! (`completion::Completion::for_route`: Passthrough and Translated are NEVER forged — the GPU writes
//! them; only Emulated work that reached no GPU is).
//!
//! Copied from the old tree's `kayfabe-doorbell` (v3-native, w823-824: 48 tests, an exhaustive
//! interleaving model) — only these modules; the trap path is `kf-trap`. ⊘ This is the ONE worker
//! loop: `kf-chan`'s runners are what `HostOps` calls, never a second loop.

pub mod caps;
pub mod channel;
pub mod completion;
pub mod leaf;
pub mod lifetime;
pub mod plane;

pub use caps::{Aperture, Backing, Refusal, Twin, VmCaps};
pub use channel::{Birth, Decoded, Disposition, Owner, Submission, size_is_total};
pub use completion::{Completion, EngineIrq, Raise};
pub use leaf::{GuestRamBlock, GuestRamLayout, HostSlice, LeafRefusal};
pub use lifetime::{Step, Teardown, TeardownError, WalkerState};
pub use plane::{HostOps, Plane, Translatable, Vmm};

#[cfg(test)]
mod tests;
