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

/// ★ 2026-10-16 (PHASE 2 protocol, model only: NO production caller yet; brief appendix C). Why an inline
/// copy stopped short of the delta it was asked to cover. Anything but [`InlineStop::None`] means the
/// worker must take over the rest (the caller leaves PENDING and requests a non-blocking worker wake).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineStop {
    /// Everything up to `GP_PUT` was copied.
    None,
    /// The entry at offset `at` carries SYNC_WAIT (bit 63): the copy stops BEFORE it. The inline path never
    /// forwards it, nor anything after it; the worker owns the channel until it has drained to `GP_PUT`.
    SyncWait {
        /// Offset of the SYNC_WAIT entry in the delta.
        at: u32,
    },
    /// The delta exceeds [`MAX_INLINE_GP`]: nothing is copied inline.
    Budget,
    /// The private ring has fewer credits than the delta: nothing is copied inline.
    Credits,
}

/// A refusal before a single descriptor is loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineRefusal {
    /// The ring is not a power of two of at least two entries.
    Geometry,
    /// `GP_PUT` is outside `[0, entries)` (hostile): counted, nothing copied.
    PutBound,
    /// The private cursor is outside `[0, entries)`.
    GetBound,
}

/// What an inline copy does with ONE load of `GP_PUT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlinePlan {
    /// How many leading entries of the delta the caller may store in the private ring.
    pub copy: u32,
    /// The entries, loaded once each and in order (`entries[..copy]`): the caller stores THESE values.
    pub entries: [u64; MAX_INLINE_GP as usize],
    /// Why the copy is shorter than the delta, if it is.
    pub stop: InlineStop,
}

/// GP_ENTRY1 SYNC_WAIT as a bit of the 64-bit entry.
const GP_SYNC_WAIT: u64 = 1 << 63;

/// Plan the bounded inline copy of the GP entries in `[get, put)` of a guest ring of `entries` slots, with
/// `credits` free private slots. `put` is the caller's single load of `GP_PUT` (the delta is computed once and
/// a `GP_PUT` that moves during the copy is the next call's business). `load(slot)` reads one 8-byte entry and
/// is called AT MOST once per slot, in ring order, at most [`MAX_INLINE_GP`] times, and never when a refusal
/// or a budget / credit escalation is returned. No lock, no allocation, no OS call, no pushbuffer read.
///
/// # Errors
/// See [`InlineRefusal`].
pub fn inline_plan(
    get: u32,
    put: u32,
    entries: u32,
    credits: u32,
    mut load: impl FnMut(u32) -> u64,
) -> Result<InlinePlan, InlineRefusal> {
    if entries < 2 || !entries.is_power_of_two() {
        return Err(InlineRefusal::Geometry);
    }
    if get >= entries {
        return Err(InlineRefusal::GetBound);
    }
    if put >= entries {
        return Err(InlineRefusal::PutBound);
    }
    let mask = entries - 1;
    let delta = put.wrapping_sub(get) & mask;
    let mut plan = InlinePlan {
        copy: 0,
        entries: [0; MAX_INLINE_GP as usize],
        stop: InlineStop::None,
    };
    if delta == 0 {
        return Ok(plan);
    }
    if delta > MAX_INLINE_GP {
        plan.stop = InlineStop::Budget;
        return Ok(plan);
    }
    if delta > credits {
        plan.stop = InlineStop::Credits;
        return Ok(plan);
    }
    for i in 0..delta {
        let raw = load(get.wrapping_add(i) & mask);
        if raw & GP_SYNC_WAIT != 0 {
            plan.stop = InlineStop::SyncWait { at: i };
            return Ok(plan);
        }
        plan.entries[i as usize] = raw;
        plan.copy = i + 1;
    }
    Ok(plan)
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
