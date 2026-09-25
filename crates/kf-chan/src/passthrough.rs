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
    /// ★ P5c: the host error context (an `NV01_CONTEXT_DMA` over the GUEST's own notifier record,
    /// `HostRm::alloc_context_dma`), or 0 — the twin's RC record then lands where the guest reads.
    pub err_ctx: u32,
}

/// A copy engine: `NV2080_ENGINE_TYPE_COPY0..9` (`0x09..=0x12`) or `COPY10..19` (`0x34..=0x3d`)
/// — `kf_abi::submit::copy_index_of_engine_type`, the header's own inverse.
#[must_use]
pub fn is_copy_engine(engine_type: u32) -> bool {
    kf_abi::submit::copy_index_of_engine_type(engine_type).is_some()
}

/// `NV2080_ENGINE_TYPE_COPY(i)` for BOTH blocks (`kf_abi::submit::engine_type_copy` stops at 9).
#[must_use]
pub fn copy_engine_type(i: u32) -> Option<u32> {
    match i {
        0..=9 => kf_abi::submit::engine_type_copy(i),
        10..=19 => Some(kf_abi::submit::ENGINE_TYPE_COPY10 + i - 10),
        _ => None,
    }
}

/// ★ Birth the host twin of a guest channel in `space` (the host VA space mirroring the guest's),
/// over the guest's own ring and USERD — the channel alone: TSG, channel, `BIND`, token. Engine
/// objects follow the guest's own allocs ([`engine_object`]), scheduling the guest's own
/// `GPFIFO_SCHEDULE` (P5b). Returns the host channel; its `token` is what the trap rings inline.
///
/// ★ The host engine IS the guest's `engineType`: the device's engine list is the host's
/// (`kf_rm::hostfacts` `engines`), so the guest's COPY`n` names host COPY`n` — including a GRCE,
/// whose subchannel routing the guest's own pushbuffer already honours, exactly as on bare metal.
/// Only a copy engine or GR0 is expressible here; anything else is refused by name.
///
/// # Errors
/// The host's refusal, by name.
pub fn birth_twin(rm: &HostRm, space: VaSpace, g: GuestChannel) -> Result<Channel, String> {
    if !is_copy_engine(g.engine) && g.engine != ENGINE_TYPE_GRAPHICS {
        return Err(format!("engine type {:#x}: only a copy engine or GR0 has a passthrough twin", g.engine));
    }
    let (userd_memory, userd_offset) = match g.userd {
        UserdAt::Store { store, off } => (store, off),
        UserdAt::Ram { ram, off } => (ram, off),
    };
    rm.birth_channel(space, g.engine, RingSpec {
        gp_fifo_va: g.gpfifo_va,
        gp_fifo_entries: g.entries,
        userd_memory,
        userd_offset,
        err_notifier: g.err_ctx,
    })
    .map_err(|e| format!("birth: {e:?}"))
}

/// ★ The engine object the guest allocated on its channel, allocated on the twin with the guest's
/// CLASS (its pushbuffer `SET_OBJECT`s that id) and params WE author. `class_kind` is the class's
/// kind in the HOST family's generated set — the caller's check; a kind that does not match the
/// twin's engine is refused here. ⊘ GR's context is built by host RM, in the host VA space, at
/// RM-chosen VAs — which is where a collision with the guest's own VAs would surface.
///
/// ★ w827 — **a COPY class on a GR twin** is what CUDA's `cuCtxCreate` allocates: its GR channel
/// carries the copy class too, declared (`declared_copy`) on a GRCE, which shares GR's runlist
/// and takes the channel's copy subchannel exactly as on bare metal. The declared engine is passed
/// to host RM as the engine WE name in params WE author; host RM refuses by name
/// (`chandesConstruct`: *"incompatible runlist"*) any copy engine that is not on this twin's
/// runlist, so nothing beyond a copy-engine ordinal is taken from the guest. `[measured vh w827,
/// b45f202a]` refusing it failed `cuCtxCreate` with 999 in every CUDA rung.
///
/// # Errors
/// A kind/engine mismatch or the host's refusal, by name.
pub fn engine_object(
    rm: &HostRm,
    chan: Channel,
    engine: u32,
    class: u32,
    class_kind: kf_chip::classes::Kind,
    declared_copy: Option<u32>,
) -> Result<u32, String> {
    use kf_chip::classes::Kind;
    let copy = match class_kind {
        Kind::DmaCopy if is_copy_engine(engine) => Some(engine),
        Kind::DmaCopy if engine == ENGINE_TYPE_GRAPHICS => match declared_copy {
            Some(ce) if is_copy_engine(ce) => Some(ce),
            _ => return Err(format!("class {class:#x} (DmaCopy) on a GR twin declares no copy engine ({declared_copy:?})")),
        },
        Kind::Compute | Kind::ThreeD if engine == ENGINE_TYPE_GRAPHICS => None,
        k => return Err(format!("class {class:#x} ({k:?}) on a twin of engine {engine:#x}")),
    };
    rm.alloc_engine_object(chan, class, copy).map_err(|e| format!("engine object {class:#x}: {e:?}"))
}

/// ★ Birth the host twin of a guest channel in `space`, give it the engine object its engine
/// needs (this host's CE class / compute class), and schedule it — the one-call shape gates 5/6
/// prove (`kf-harness`). The device takes the three steps separately, each at the guest's own
/// statement ([`birth_twin`], [`engine_object`], `HostRm::schedule_enable`).
///
/// # Errors
/// The host's refusal, by name.
pub fn birth(rm: &HostRm, space: VaSpace, g: GuestChannel) -> Result<Channel, String> {
    let chan = birth_twin(rm, space, g)?;
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
