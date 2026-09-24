//! ★★★★★ **Passthrough channels — the guest's own ring reaches the engine unparsed.**
//!
//! A guest USER channel (compute, user CE) names only VIRTUAL addresses, and its VA space is
//! mirrored by the walk + reconcile. So the host channel is born **over the guest's own GPFIFO VA
//! and the guest's own USERD**, and the doorbell is one inline store in the vCPU trap
//! (`kf_trap::Action::RingHostInline`). Nothing here reads a pushbuffer, a GP entry or a cursor:
//! the engine fetches from the guest's memory and writes the guest's `GP_GET` itself.
//!
//! ⊘⊘ **ADOPT AT CREATION, NEVER LAZILY.** `[measured w233, GA106/580]` host RM accepts a
//! caller-supplied USERD — and **zeroes all 512 bytes of it**, returning `NV_OK`. A channel born at
//! the guest's first doorbell would wipe the very `GP_PUT` that rang it. In v3 the guest's channel
//! allocation reaches us as an RPC we answer, so [`birth`] runs INSIDE that answer, before the guest
//! can have written anything to its USERD.

use kf_abi::submit::{ENGINE_TYPE_COPY0, ENGINE_TYPE_GRAPHICS};
use kf_host::{Channel, HostRm, RingSpec, VaSpace};

/// Where the guest put its USERD: a slice of one of the two ground truths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserdAt {
    /// At GPGA offset `off` in the store object `store`.
    Store {
        /// The store handle.
        store: u32,
        /// GPGA offset.
        off: u64,
    },
    /// At memfd offset `off` in the guest-RAM descriptor `ram`.
    Ram {
        /// The guest-RAM `OS_DESCRIPTOR` handle.
        ram: u32,
        /// memfd offset.
        off: u64,
    },
}

/// The guest channel as its allocation described it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestChannel {
    /// The GPFIFO's VA in the guest's (mirrored) VA space.
    pub gpfifo_va: u64,
    /// GPFIFO entries.
    pub entries: u32,
    /// The guest's USERD.
    pub userd: UserdAt,
    /// The engine it was allocated on (an `NV2080_ENGINE_TYPE`; COPY0 for a user CE).
    pub engine: u32,
}

/// ★ Birth the host twin of a guest channel in `space` (the host VA space mirroring the guest's),
/// over the guest's own ring and USERD. Returns the host channel; its `token` is what the trap
/// rings inline.
///
/// # Errors
/// The host's refusal, by name.
pub fn birth(rm: &HostRm, space: VaSpace, g: GuestChannel) -> Result<Channel, String> {
    let (userd_memory, userd_offset) = match g.userd {
        UserdAt::Store { store, off } => (store, off),
        UserdAt::Ram { ram, off } => (ram, off),
    };
    let chan = rm
        .birth_channel(space, g.engine, RingSpec {
            gp_fifo_va: g.gpfifo_va,
            gp_fifo_entries: g.entries,
            userd_memory,
            userd_offset,
            err_notifier: 0,
        })
        .map_err(|e| format!("birth: {e:?}"))?;
    // The engine object the guest's own allocation named. ⊘ GR's context is built here, by host
    // RM, in the host VA space — RM places its context buffers at RM-chosen VAs, which is where a
    // collision with the guest's own VAs would surface (named `0x51` at reconcile, never silent).
    match g.engine {
        ENGINE_TYPE_COPY0 => {
            rm.alloc_ce_object(chan, g.engine).map_err(|e| format!("ce object: {e:?}"))?;
        }
        ENGINE_TYPE_GRAPHICS => {
            rm.alloc_compute_object(chan).map_err(|e| format!("compute object: {e:?}"))?;
        }
        other => return Err(format!("no engine object for engine type {other:#x}")),
    }
    rm.schedule(chan).map_err(|e| format!("schedule: {e:?}"))?;
    Ok(chan)
}
