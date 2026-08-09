//! ★★★ **Context promotion — the address-plane join for `GPU_PROMOTE_CTX`.**
//!
//! # What this is, and the thing it is routinely mistaken for
//!
//! `GPU_PROMOTE_CTX` is how the guest tells the GPU where a graphics/compute context's
//! buffers live. It is the **only address-populating RPC** a GSP client emits — and that
//! sentence has been over-read into *"it is the gap between here and first compute"*,
//! which it is not. The compute working set's leaf PTEs arrive through the observed
//! copy-engine page-table writes, not through this control; and the host owns and
//! self-maps the very ranges a promote entry describes, because the Case-1 engine-object
//! forward makes the **host** kernel-RM allocate its own context buffers and issue its
//! own promotion in-kernel.
//!
//! So what this join is worth is exactly this: under MISS = FAULT, resolving a GR
//! context-buffer VA with no binding **faults**. This is the gap-filler that stops that,
//! and nothing more. It is necessary, narrow, and nowhere near sufficient.
//!
//! # ★★ Keyed on the ADDRESS SPACE, never on the RM client
//!
//! The C artifact keyed its table on the `hChanClient` it read out of the params
//! (`nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2447`, `:2419`) and matched on it
//! at resolve time. That is the anti-pattern #12 was rooted in, and hardware has since
//! measured *why*: two concurrent CUDA processes **share one duplicated client**, and
//! UVM's gpu-ops client is global, so a client handle does not identify a process. Here
//! `hObject` is resolved to a live resource, the resource names a `(GpuId, Pdb)`, and the
//! `(GpuId, Pdb)` names the [`Vas`](crate::gpu::Vas) — the memory boundary itself.
//!
//! # The two clients do two different jobs
//!
//! | job | source |
//! |---|---|
//! | **namespace attribution** — which client is acting | the RPC **envelope**'s `hClient` ([`CtxPromotion::client`]) |
//! | **object resolution** — whose handle table `hObject` is a handle in | `hChanClient` ([`CtxPromotion::chan_client`]) |
//!
//! RM sets them from two different objects and does not require them to be equal
//! (`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics_object.c:130-135`), so
//! refusing a mismatch would refuse a stream the guest's own driver emits. What is
//! refused is the thing that actually matters and that the C could not even see: a
//! promotion whose **acting** client is not in the component that owns the address space
//! it is writing into ([`PromoteFault::ForeignContextObject`]). Attribution is checked
//! against resolution instead of being discarded.
//!
//! # Two passes, because R3 says so
//!
//! [`route_promote_ctx`] is a pure read of the [`Spine`](crate::gpu::Spine)'s
//! projection-derived index (rank 0, no proc touched); [`apply_promote_ctx`] takes the
//! **owning** proc alone (rank 1). They are separate for the same structural reason the
//! page-table-write latch is: the proc that *issues* the control and the proc that *owns*
//! the address space are not required to be the same one, and holding two rank-1 locks is
//! what R3 forbids.

use std::collections::BTreeSet;

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb};
use kayfabe_mmu::Binding;

use crate::ProcId;
use crate::gpu::{Proc, Spine};
use crate::rmgraph::NodeKey;

/// ★ **This port's own bound on how many ranges one promotion may declare.**
///
/// It is deliberately *not* an import of NVIDIA's
/// `NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES`: the ABI layer bounds the wire, this
/// bounds the core, and the two are independent numbers that happen to agree today. If
/// they ever stop agreeing, [`PromoteFault::TooManyRanges`] is loud about it — which is
/// the opposite of the C artifact, where three numbers (a comment's `20`, a clamp's `64`,
/// the header's `16`) disagreed silently and the largest one won.
pub const MAX_PROMOTED_RANGES: usize = 16;

/// One **complete** VA → physical declaration recovered from a context promotion.
///
/// Only the both-preparers-ran state reaches here. A promote-only entry (VA declared,
/// `gpuPhysAddr`/`size` never written) and an initialize-only entry (physical buffer, no
/// VA) are counted in [`PromoteDeclined`] and never become a `PromotedRange` — binding
/// either would mean manufacturing an address out of a field the producer did not write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromotedRange {
    /// The GPU virtual address the context buffer is mapped at.
    pub va: GpuVa,
    /// Its length in bytes. Never zero.
    pub len: u64,
    /// The backing address, in [`Self::aperture`]. For [`Aperture::Vidmem`] this is a
    /// **guest** framebuffer offset — a buffer the host never touches (the host allocated
    /// and self-mapped its own).
    pub phys: u64,
    /// Which aperture `phys` is in.
    pub aperture: Aperture,
    /// `NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_*` — MAIN, PATCH, PRIV_ACCESS_MAP, …
    ///
    /// Carried rather than dropped. The C artifact never stored it, so its table could
    /// not tell one context buffer from another.
    pub buffer_id: u16,
}

/// The non-bindable entries a promotion carried — **named and counted, never silent**.
///
/// This type exists because of C defect D3. The *behaviour* it describes is forced (a
/// promote-only entry has no length, and `AddressTable::bind` refuses a zero length, so
/// it is structurally unbindable) — but a forced outcome that nothing names reads as an
/// intentional decision it is not, and the C's `!sz` arm silently swallowed 4 of the 9
/// entries in the project's own captured blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PromoteDeclined {
    /// Entries declaring a physical buffer and no VA (`bNonmapped`).
    pub initialize_only: u32,
    /// Entries declaring a VA whose physical/length fields the producer never wrote.
    pub promote_only: u32,
}

/// One context promotion, in core vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CtxPromotion {
    /// The **envelope**'s `hClient` — the namespace this message acts in.
    pub client: HClient,
    /// `hChanClient` — the namespace [`Self::object`] is a handle in.
    pub chan_client: HClient,
    /// `hObject` — a channel **or** a channel group (TSG).
    pub object: HObject,
    /// The complete mappings, in wire order.
    pub ranges: Vec<PromotedRange>,
    /// What was dropped, and in which state.
    pub declined: PromoteDeclined,
}

/// Where a promotion resolved to: the address space, and the proc that owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromoteRoute {
    /// The proc owning the target address space — **not** necessarily the proc that
    /// issued the control.
    pub proc: ProcId,
    /// The target GPU.
    pub gpu: GpuId,
    /// The address space's page-directory base.
    pub pdb: Pdb,
}

/// What a promotion did. Every entry is accounted for: `bound + already +
/// declined.initialize_only + declined.promote_only` equals the number the control
/// carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromoteJoin {
    /// The address space that was written.
    pub route: PromoteRoute,
    /// Ranges newly forward-populated into the table.
    pub bound: u32,
    /// Ranges that were **already** bound by a previous promotion, byte-identically —
    /// the idempotent re-promote. Not an error: the same context buffer is promoted
    /// again when a second channel of a TSG comes up.
    pub already: u32,
    /// What the control declared and this join did not bind.
    pub declined: PromoteDeclined,
}

/// Every way a promotion is refused, by name. There is no catch-all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromoteFault {
    /// `hObject` does not resolve in `hChanClient`'s live handle table. MISS = FAULT: a
    /// promotion naming an object we cannot see is refused, never guessed at.
    UnknownContextObject {
        /// `hChanClient`.
        client: HClient,
        /// `hObject`.
        object: HObject,
    },
    /// `hObject` resolves, but to something that is neither a channel nor a channel
    /// group. Typed resolution (decision #18C): a hostile guest naming a memory object
    /// here must not have its promotion land in whatever address space that object's
    /// neighbour happens to use.
    NotAContextObject {
        /// `hChanClient`.
        client: HClient,
        /// `hObject`.
        object: HObject,
    },
    /// The channel/TSG exists but names no routable address space — its VASpace has not
    /// declared a page-directory base, its `Device` target has not resolved, or the VAS
    /// has since died. **DEFER at derivation, FAULT here**: the guest asked us to write a
    /// table that does not exist.
    ///
    /// ★★★ **This variant names hop 2 of [`route_promote_ctx`] ONLY** — the
    /// [`Spine::ctx_vas`] miss. It used to name hop 3 as well, and that was a refusal
    /// whose name could not be true of both things it was raised for:
    ///
    /// | hop | miss means | who is at fault |
    /// |---|---|---|
    /// | 2 — `ctx_vas` | the channel/TSG resolved, but **no `(gpu, pdb)` was ever derived for it** — its VA space declared no page-directory base | the guest has not published a root, or we did not route the publication |
    /// | 3 — `by_pdb` | a `(gpu, pdb)` WAS derived, and **no proc owns it** | our own projection disagrees with itself |
    ///
    /// ⊘ Those are opposite diagnoses — *"the root never arrived"* versus *"the root
    /// arrived and the owner index lost it"* — and a census counting one tag could not
    /// tell a reader which had happened. `[measured 2026-08-09, boot `s35_03a7e10_dup`]`
    /// printed `PromoteFault::ContextVasUndeclared x1` and **three separate rungs read it
    /// as hop 2** because that is the reading the doc comment invited; nothing in the
    /// capture could have refuted hop 3. Hop 3 is now
    /// [`PromoteFault::ContextVasNoOwner`], and the two are counted apart.
    ///
    /// ★ This is `refuse_by_name_means_the_name_is_true` applied to a variant that already
    /// existed: the fix is not a new check, it is one name per thing being refused.
    ContextVasUndeclared {
        /// `hChanClient`.
        client: HClient,
        /// `hObject`.
        object: HObject,
    },
    /// ★★★ Hop 3 of [`route_promote_ctx`]: the channel/TSG resolved to a real
    /// `(GpuId, Pdb)` and **[`Spine::by_pdb`] names no owning proc for it**.
    ///
    /// ⊘ Split out of [`PromoteFault::ContextVasUndeclared`] because the two are opposite
    /// diagnoses — see that variant's table. This one is the **louder** of the pair: both
    /// indices are derived from the same projection in the same pass
    /// (`project.rs:1179` populates `by_pdb`, `:1238`/`:1254` populate `ctx_vas`), so a
    /// `(gpu, pdb)` present in one and absent from the other is an internal disagreement
    /// rather than a fact about the guest. Its sibling [`PromoteFault::UnknownVas`] makes
    /// the same argument one level further in.
    ContextVasNoOwner {
        /// `hChanClient`.
        client: HClient,
        /// `hObject`.
        object: HObject,
        /// The address space the context object DID resolve to — the fact that makes this
        /// different from its sibling, and the key that was looked up and missed.
        pdb: Pdb,
    },
    /// The address space resolved but its owning proc has retired between the route and
    /// the apply. Skipped rather than re-attached: re-attaching a promotion to whoever
    /// inherits a retired proc's id is the C's never-pruned-table aliasing class.
    RetiredProc(
        /// The proc that owned the address space when the route was taken.
        ProcId,
    ),
    /// The owning proc holds no `Vas` at the routed `(gpu, pdb)`. The route was derived
    /// from the same projection the `Vas` is, so this is an internal disagreement rather
    /// than guest input — and it is loud for that reason.
    UnknownVas {
        /// The target GPU.
        gpu: GpuId,
        /// The address space.
        pdb: Pdb,
    },
    /// ★★★ **The acting client is not in the component that owns the address space it
    /// is promoting into.**
    ///
    /// `hChanClient`/`hObject` name *someone's* channel; the envelope's `hClient` names
    /// who is asking. Without this check, a client may declare bindings in a **victim's**
    /// address space by naming the victim's client and channel — the params-field
    /// injection the C could not even detect, because it never read the envelope.
    ///
    /// The check is component-scoped, not handle-scoped, precisely because two concurrent
    /// CUDA processes share a duplicated client: the question is *"is the acting
    /// namespace part of this process?"*, which is what [`Proc`] is.
    ForeignContextObject {
        /// The envelope's `hClient`.
        client: HClient,
        /// `hObject`.
        object: HObject,
        /// The proc that owns the address space.
        owner: ProcId,
    },
    /// More ranges than [`MAX_PROMOTED_RANGES`]. See that constant.
    TooManyRanges {
        /// How many the caller supplied.
        declared: usize,
        /// [`MAX_PROMOTED_RANGES`].
        max: usize,
    },
    /// A range with zero length, or whose `va + len` wraps `u64`. Guest-influenced
    /// arithmetic refuses; it never panics and it never clips (boundary-1 posture).
    Malformed {
        /// Start of the offending range.
        va: GpuVa,
        /// Its declared length.
        len: u64,
    },
    /// Two ranges **inside one promotion** overlap each other. Caught before anything is
    /// bound, so the control is refused whole rather than half-applied.
    SelfOverlap {
        /// The earlier range's start.
        a: GpuVa,
        /// The later range's start.
        b: GpuVa,
    },
    /// The range overlaps a binding this promotion does not own — either a range from
    /// another populate source, or a previous promotion that declared *different*
    /// contents at the same address. Unmap is eager in this port: the guest must have
    /// unbound first, and silently replacing is how the #14 collision class hides.
    Collides {
        /// Start of the range being promoted.
        va: GpuVa,
        /// Its length.
        len: u64,
    },
}

/// ★ **ROUTE (rank 0).** Resolve `hObject` in `hChanClient`'s namespace to the address
/// space its context buffers live in, and to the proc that owns it. A pure read of the
/// spine's projection-derived indices; it touches no [`Proc`].
///
/// The resolution is three hops and every one is a *forward* lookup:
///
/// 1. `(hChanClient, hObject)` → the **live** resource at that handle
///    ([`crate::rmgraph::RmGraph::node`]). A handle value is recyclable by design, so it
///    is resolved through the live table at the moment of use and then immediately
///    replaced by the resource's stable identity.
/// 2. resource identity → `(GpuId, Pdb)` ([`Spine::ctx_vas`], derived from the projection
///    beside `by_pdb`, never accreted).
/// 3. `(GpuId, Pdb)` → owning proc ([`Spine::by_pdb`]).
///
/// There is no reverse resolution anywhere in it, and no step has a fallback.
///
/// # Errors
///
/// [`PromoteFault::UnknownContextObject`], [`PromoteFault::NotAContextObject`],
/// [`PromoteFault::ContextVasUndeclared`] (hop 2), [`PromoteFault::ContextVasNoOwner`]
/// (hop 3) — ★ one name per hop, so a census row names which lookup missed.
pub fn route_promote_ctx(
    spine: &Spine,
    chan_client: HClient,
    object: HObject,
) -> Result<PromoteRoute, PromoteFault> {
    let node = spine
        .rmgraph
        .node(NodeKey::new(chan_client, object))
        .ok_or(PromoteFault::UnknownContextObject {
            client: chan_client,
            object,
        })?;
    if !matches!(
        node.kind,
        kayfabe_arch::ObjectKind::Channel { .. } | kayfabe_arch::ObjectKind::Tsg
    ) {
        return Err(PromoteFault::NotAContextObject {
            client: chan_client,
            object,
        });
    }
    let &(gpu, pdb) = spine
        .ctx_vas
        .get(&node.id())
        .ok_or(PromoteFault::ContextVasUndeclared {
            client: chan_client,
            object,
        })?;
    // ★★★ Hop 3, and it refuses under its OWN name. `ctx_vas` answered — a `(gpu, pdb)`
    // exists for this context object — so a miss here is the owner index disagreeing with
    // the index that sits beside it, not the guest failing to publish a root.
    let proc = *spine
        .by_pdb
        .get(&(gpu, pdb))
        .ok_or(PromoteFault::ContextVasNoOwner {
            client: chan_client,
            object,
            pdb,
        })?;
    Ok(PromoteRoute { proc, gpu, pdb })
}

/// ★ **ACT (rank 1, one proc).** Forward-populate `p`'s complete ranges into the routed
/// address space's table.
///
/// # It is all-or-nothing, and that is the point
///
/// Every range is validated against the table, against the promotion's own other ranges,
/// and against this VAS's promote idempotence set **before a single bind happens**. A
/// promotion that would half-apply is refused whole. The alternative — bind until
/// something collides — leaves the guest holding a `NV_ERR` for a control that
/// nonetheless changed its address space, which is a state neither side can reason about.
///
/// # ★★ The idempotence set is this source's OWN, and reusing `rpc_bound` would be a bug
///
/// [`crate::gpu::Vas::promote_bound`] mirrors `rpc_bound`'s role for the RPC map source
/// and must stay separate from it. `Spine::sync_rpc_mappings` builds its desired set
/// **exclusively** from `RmGraph::mappings()` and unbinds every `rpc_bound` VA not in it;
/// a promotion is not a `MapMemoryDma` and never will be, so a promote binding filed under
/// `rpc_bound` would be reaped on the very next `Spine::apply` — a table correct
/// immediately after the control and empty a moment later, which reads as a race.
///
/// # Errors
///
/// [`PromoteFault::RetiredProc`], [`PromoteFault::UnknownVas`],
/// [`PromoteFault::ForeignContextObject`], [`PromoteFault::TooManyRanges`],
/// [`PromoteFault::Malformed`], [`PromoteFault::SelfOverlap`], [`PromoteFault::Collides`].
pub fn apply_promote_ctx(
    proc: &mut Proc,
    route: &PromoteRoute,
    p: &CtxPromotion,
) -> Result<PromoteJoin, PromoteFault> {
    if proc.id != route.proc {
        return Err(PromoteFault::RetiredProc(route.proc));
    }
    // ★★★ Attribution checked AGAINST resolution. See `ForeignContextObject`.
    if !proc.client_values().contains(&p.client) {
        return Err(PromoteFault::ForeignContextObject {
            client: p.client,
            object: p.object,
            owner: route.proc,
        });
    }
    if p.ranges.len() > MAX_PROMOTED_RANGES {
        return Err(PromoteFault::TooManyRanges {
            declared: p.ranges.len(),
            max: MAX_PROMOTED_RANGES,
        });
    }
    let vas = proc
        .vases
        .get_mut(&(route.gpu, route.pdb))
        .ok_or(PromoteFault::UnknownVas {
            gpu: route.gpu,
            pdb: route.pdb,
        })?;

    // ── PASS 1: validate everything, bind nothing. ───────────────────────────────────
    //
    // ★ Re-validated here even though the ABI decoder already refused a zero length:
    // this function's argument is a core value that any caller can build, so the law has
    // to hold at the site that depends on it (R5's revalidate rule), not only at the site
    // that happened to produce it today.
    for r in &p.ranges {
        if r.len == 0 || r.va.0.checked_add(r.len).is_none() {
            return Err(PromoteFault::Malformed {
                va: r.va,
                len: r.len,
            });
        }
    }
    for (i, a) in p.ranges.iter().enumerate() {
        for b in p.ranges.iter().skip(i + 1) {
            let overlap = a.va.0 < b.va.0.saturating_add(b.len) && b.va.0 < a.va.0 + a.len;
            if overlap {
                return Err(PromoteFault::SelfOverlap { a: a.va, b: b.va });
            }
        }
    }
    // Which ranges are already there, byte-identically, from a previous promotion.
    let mut already: BTreeSet<u64> = BTreeSet::new();
    for r in &p.ranges {
        let mut covered = false;
        for (start, _len, binding) in vas.table.spans(r.va, r.len) {
            // The span's offset into the binding is unread here: this asks whether an
            // IDENTICAL range is already bound, and an identical range starts at the
            // binding's base by definition (`start == r.va.0` below).
            let Some((b, _within)) = binding else {
                continue;
            };
            covered = true;
            // An identical re-promote is the ONE overlap that is not a conflict: same
            // start, same length, same contents, and previously bound by this source.
            let identical = start == r.va.0
                && vas.promote_bound.contains(&r.va.0)
                && b.phys == r.phys
                && b.aperture == r.aperture
                && b.host.is_none()
                && vas
                    .table
                    .iter()
                    .any(|(va, len, _)| va == r.va.0 && len == r.len);
            if !identical {
                return Err(PromoteFault::Collides {
                    va: r.va,
                    len: r.len,
                });
            }
        }
        if covered {
            already.insert(r.va.0);
        }
    }

    // ── PASS 2: apply. Nothing below can fail. ───────────────────────────────────────
    let mut bound = 0u32;
    for r in &p.ranges {
        if already.contains(&r.va.0) {
            continue;
        }
        vas.table
            .bind(
                route.pdb,
                r.va,
                r.len,
                Binding {
                    phys: r.phys,
                    aperture: r.aperture,
                    // ★ `None`, and its own doc says why: *declared by the RPC/CE-capture
                    // source only — nothing host-side exists yet, and nothing host-side
                    // needs reclaiming*. Which is exactly the status of a GR context
                    // buffer the HOST allocated and mapped for itself.
                    host: None,
                },
            )
            .map_err(|_| PromoteFault::Collides {
                va: r.va,
                len: r.len,
            })?;
        vas.promote_bound.insert(r.va.0);
        bound += 1;
    }
    Ok(PromoteJoin {
        route: *route,
        bound,
        already: u32::try_from(already.len()).unwrap_or(u32::MAX),
        declined: p.declined,
    })
}
