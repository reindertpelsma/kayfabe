//! Unconnected three-mode doorbell dispatch prototype, 2026-10-12.
//!
//! A companion atomic u64 preserves the existing token word's full host token,
//! ordering stamp and Translated/Emulated completion distinction. No VM caller.
//! All callback methods have a fixed number of atomics, no loops, locks,
//! allocation, OS calls or pushbuffer reads. Pre-mapped relay rows live outside
//! this word and may be replaced only after retirement and native cleanup.
//!
//! CPU exclusion is NOT native stream handoff. Switching a previously published
//! opaque stream sets HandoffPending; this prototype cannot clear that gate.
//! A never-published channel can clear it after its last inline claimant exits.
//! Worker wake requests must use the existing token bitmap/wake protocol; a
//! callback cannot treat a failed claim as completed work.

use core::sync::atomic::Ordering;

/// Dispatch policy; channel kind/permissions remain in authoritative metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    /// Unallocated or retired; absorb doorbells.
    Dead = 0,
    /// Forward the full opaque token through an already mapped doorbell.
    Direct = 1,
    /// Copy only a small, validated GP delta/USERD through pre-created views.
    CopyRing = 2,
    /// Publish pending work to the existing worker path.
    Worker = 3,
}

const MODE: u64 = 3;
const BUSY: u64 = 1 << 2;
const RETIRING: u64 = 1 << 3;
const PUBLISHED: u64 = 1 << 4;
const HANDOFF: u64 = 1 << 5;
const PENDING: u64 = 1 << 6;
const HOST_SHIFT: u32 = 7;
const GENERATION_SHIFT: u32 = 39;
const MAX_GENERATION: u64 = (1 << 25) - 1;

fn mode(word: u64) -> Mode {
    match word & MODE {
        1 => Mode::Direct,
        2 => Mode::CopyRing,
        3 => Mode::Worker,
        _ => Mode::Dead,
    }
}
fn generation(word: u64) -> u32 {
    (word >> GENERATION_SHIFT) as u32
}

/// One controller step; Retry means reschedule, never spin on a vCPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Requested state is established.
    Done,
    /// An inline/worker claim still owns the row.
    Busy,
    /// Opaque native work was published; its stream state is not established.
    NativeHandoffPending,
    /// Wrong lifetime, retired row, or otherwise invalid transition.
    Stale,
    /// Another atomic operation won; controller may retry off the vCPU.
    Retry,
}

/// Allocation failure, before an entry can authorize native publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// Previous lifetime has not completed native cleanup.
    NotClean,
    /// Generation exhaustion is refused; never wrap into an old identity.
    GenerationExhausted,
    /// Dead is not an allocatable dispatch policy.
    DeadMode,
    /// A concurrent atomic changed the row; retry off the vCPU.
    Retry,
}

/// Trusted hot-path budget; neither a GPU fact nor a guest-selectable cap.
pub const MAX_INLINE_GP: u32 = 4;

/// Bound the source delta BEFORE reading any descriptor. Private-ring credits
/// and full source-span/lifetime admission are separate required checks.
pub fn inline_delta(get: u32, put: u32, entries: u32, credits: u32) -> Option<u32> {
    if entries < 2 || !entries.is_power_of_two() || get >= entries || put >= entries {
        return None;
    }
    let delta = put.wrapping_sub(get) & (entries - 1);
    (delta <= MAX_INLINE_GP && delta <= credits).then_some(delta)
}

mod native {
    use super::*;
    use core::sync::atomic::AtomicU64;
    include!("relay_dispatch_impl.rs");
}
pub use native::{Begin, DispatchWord, Ticket};

#[cfg(test)]
mod modeled {
    use super::*;
    use loom::sync::atomic::AtomicU64;
    // The actual implementation, compiled against Loom atomics, not a second
    // state-machine implementation that could omit a load/CAS race.
    include!("relay_dispatch_impl.rs");
}

#[cfg(test)]
#[path = "relay_dispatch_tests.rs"]
mod tests;
