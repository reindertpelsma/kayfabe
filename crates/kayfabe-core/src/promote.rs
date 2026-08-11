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
//! RM sets them from two different objects and does not require them to be equal, so
//! refusing a mismatch would refuse a stream the guest's own driver emits. What is
//! refused is the thing that actually matters and that the C could not even see: a
//! promotion whose **acting** client is neither in the component that owns the address
//! space it is writing into **nor kernel-privileged**
//! ([`PromoteFault::ForeignContextObject`]). Attribution is checked against resolution
//! instead of being discarded.
//!
//! ★★★★ ⚠ **The citation for "not required to be equal" was for YEARS the WRONG SITE,
//! and the wrong site says the opposite.** This paragraph cited
//! `ogkm-580: kernel_graphics_object.c:130-135` — which is real and does set
//! `params.hChanClient = RES_GET_CLIENT_HANDLE(pChannelDescendant)` at `:131` and issue
//! the control with `RES_GET_CLIENT_HANDLE(pSubdevice)` at `:136`. But **on that path they
//! are always equal**: `:74-79` obtains the subdevice with
//! `subdeviceGetByDeviceAndGpu(RES_GET_CLIENT(pKernelGraphicsObject), …)`, i.e. in the
//! graphics object's own client. The site where they genuinely differ — and the one
//! `[measured 2026-08-09, boot s38_411d280_route]` — is
//! `nvGpuOpsBindChannelResources` (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:10870`
//! and `:10891-10893`), where the envelope is `retainedChannel->session->handle` (UVM's)
//! and `hChanClient` is the user's.
//!
//! ⊘ So the doc's *claim* was true, its *citation* was to a site that establishes the
//! opposite, and the check written beside it took the site's behaviour rather than the
//! claim. `a_correct_citation_narrowed_by_the_reading` — except here reading the cited
//! lines is what catches it, and reading only the sentence above them is what does not.
//!
//! # Two passes, because R3 says so
//!
//! [`route_promote_ctx`] is a pure read of the [`Spine`](crate::gpu::Spine)'s
//! projection-derived index (rank 0, no proc touched); [`apply_promote_ctx`] takes the
//! **owning** proc alone (rank 1). They are separate for the same structural reason the
//! page-table-write latch is: the proc that *issues* the control and the proc that *owns*
//! the address space are not required to be the same one, and holding two rank-1 locks is
//! what R3 forbids.

use std::collections::{BTreeMap, BTreeSet};

use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb};

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
/// Reached two ways, and ⊘ **never from zero**: either one wire entry carried both halves
/// (the both-preparers-ran state), or [`apply_promote_ctx`]'s §16.48 join completed a
/// [`PromoteHalf`] from its parked partner. Binding a half against a field its producer
/// never wrote would be manufacturing an address; binding it against the *other control's*
/// measured value is the join this type exists to receive.
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

/// ★★★★★ **One HALF of a two-phase promotion** — §16.48.
///
/// An entry that declares a physical buffer with no VA, or a VA with no physical, is not
/// a malformed entry and not a declined one: it is **one phase of a promotion RM
/// deliberately splits in two** for an externally-owned (UVM) VA space. See
/// [`crate::gpu::Vas::promote_halves`] for the two emitters and the `ogkm` citations.
///
/// ⊘ These were previously *counted and dropped* ([`PromoteDeclined`]). Counting them was
/// right — it is what made `bound=0` legible at `s40` — but dropping them is why eleven
/// `NV_OK`s bound nothing. They are now carried so the join can complete them, and
/// [`PromoteDeclined`] keeps its original meaning: **what the wire declared**, unchanged,
/// so the two numbers can be compared rather than one silently replacing the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromoteHalf {
    /// **Phase 1** — `gpuPhysAddr`/`size`/`physAttr` set, `bNonmapped = 1`, no VA.
    Physical {
        /// `gpuPhysAddr`, in [`Self::Physical::aperture`].
        phys: u64,
        /// `size`. ⊘ **May be zero, and a zero is neither refused nor parked** — it is
        /// counted into [`PromoteJoin::half_unusable`] and dropped. See the arm in
        /// [`apply_promote_ctx`]: the ABI classifier's last rule sends any all-zero entry
        /// here, such entries already reach us today, and refusing them would make a pure
        /// join change refuse traffic the guest sends now.
        len: u64,
        /// Which aperture `phys` is in.
        aperture: Aperture,
        /// The join key.
        buffer_id: u16,
    },
    /// **Phase 2** — `gpuVirtAddr` set, `gpuPhysAddr`/`size` never written.
    Virtual {
        /// `gpuVirtAddr`.
        va: GpuVa,
        /// The join key.
        buffer_id: u16,
    },
}

impl PromoteHalf {
    /// The `buffer_id` this half joins on, whichever phase it is.
    #[must_use]
    pub const fn buffer_id(self) -> u16 {
        match self {
            Self::Physical { buffer_id, .. } | Self::Virtual { buffer_id, .. } => buffer_id,
        }
    }
}

/// ★★★★★ **Where the PHYSICAL half of a context buffer lives** — §16.50.
///
/// # The measurement this exists to answer
///
/// `s41b` bound nothing with `orphans(awaiting_va=0, awaiting_phys=10)`: cup2's address
/// space held ten VA halves and not one physical. The physicals arrived — the cumulative
/// tally shows `phys>0` for four ids — they arrived **in a different address space**.
///
/// # ⊘ NOT derived from which ids orphaned in that boot
///
/// A list fitted to one boot is a list that will be wrong on the next one. This is read
/// off the **arms of RM's own phase-1 emitter**, `kgrctxPrepareInitializeCtxBuffer_IMPL`
/// (`ogkm-580: kernel_graphics_context.c:1710-1807`), which is the only function that can
/// ever produce a `PromoteHalf::Physical`. Every wire id reaches exactly one arm, and the
/// arm says where the memory descriptor it publishes came from:
///
/// | wire id | arm | descriptor source | scope |
/// |---|---|---|---|
/// | `0x0` MAIN | `:1713-1731` | `ppEngCtxDesc[subdev]` of the channel's own group | [`Self::PerContext`] |
/// | `0x1` PM | `:1732-1740` | `pKernelGraphicsContextUnicast->pmCtxswBuffer` | [`Self::PerContext`] |
/// | `0x2` PATCH | `:1741-1747` | `pKernelGraphicsContextUnicast->ctxPatchBuffer` | [`Self::PerContext`] |
/// | `0x3` BUFFER_BUNDLE_CB | `:1748` ↘ | — | [`Self::Never`] |
/// | `0x4` PAGEPOOL | `:1750` ↘ | — | [`Self::Never`] |
/// | `0x5` ATTRIBUTE_CB | `:1752` ↘ | — | [`Self::Never`] |
/// | `0x6` RTV_CB_GLOBAL | `:1754` ↘ | — | [`Self::Never`] |
/// | `0x7` GFXP_POOL | `:1756-1758` `// No initialization from kernel RM; return NV_OK` | — | [`Self::Never`] |
/// | `0x8` GFXP_CTRL_BLK | `:1759-1782` | `localCtxBuffer` **if `bAllocated`**, else `kgraphicsGetGlobalCtxBuffers` | [`Self::PerContext`] ⚠ |
/// | `0x9` FECS_EVENT | `:1783` ↘ | `kgraphicsGetGlobalCtxBuffers(pGpu, pKernelGraphics, gfid)` | [`Self::PerGpu`] |
/// | `0xa` PRIV_ACCESS_MAP | `:1784` ↘ | idem | [`Self::PerGpu`] |
/// | `0xb` UNRESTRICTED_PRIV_ACCESS_MAP | `:1785-1801` | idem | [`Self::PerGpu`] |
/// | `0xc` GLOBAL_PRIV_ACCESS_MAP | `:1803-1805` `// No initialization from kernel RM` | — | [`Self::Never`] |
///
/// ⚠ **`0x8` is deliberately [`Self::PerContext`] and that is the one judgement call
/// here.** Its arm reads a *per-context* `localCtxBuffer` first and only falls back to the
/// GPU-wide pool, so its physical half **can** be a private per-context buffer. Publishing
/// that GPU-wide would let one context's VA join against another context's private
/// physical — a wrong binding, which is worse than an orphaned half. ⊘ The conservative
/// answer is taken because the aggressive one is unrecoverable, not because `0x8` was
/// unobserved; it is absent from `s41b`'s tally, and an id we have never seen is the last
/// one that should get GPU-wide scope on a guess.
///
/// # ★ Why the "global vs per-context" enum is NOT the predicate on its own
///
/// `kgrctxGetGlobalContextBufferInternalId_IMPL`
/// (`ogkm-580: kernel_graphics_context.c:201-250`) is RM's own membership oracle: it
/// refuses `0x0`/`0x1`/`0x2` with `NV_ERR_INVALID_ARGUMENT` (`:214-219`) and maps
/// `0x3`–`0xc` onto the ten-entry `GR_GLOBALCTX_BUFFER` enum
/// (`kernel_graphics_context_buffers.h:186-196`). So *global* = `0x3..=0xc`, exactly.
/// But **global is not the same question as where the physical lives**: six of those ten
/// ids never emit a physical at all, and `0x8` may emit a private one. Membership comes
/// from the enum; the *scope* comes from the emitter's arms. Using membership alone would
/// have GPU-scoped `0x8` and silently claimed six ids were fixed when nothing ever
/// publishes them.
///
/// # ⊘ MISS = FAULT for an id off the end
///
/// An id RM does not recognise hits its `default:` arm and is refused
/// (`:1806-1807`, *"Unrecognized promote ctx enum"*). We cannot refuse — the entry may be
/// a complete range and complete ranges are none of this classifier's business — so an
/// unknown id is classified [`Self::PerContext`]: the *narrowest* scope, never GPU-wide.
/// Nothing gains reach by being unrecognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysHalfScope {
    /// The physical half describes a buffer private to one context. It parks in the
    /// [`crate::gpu::Vas`] and joins only VA halves from that same address space.
    PerContext,
    /// ★★★★★ The physical half describes a buffer RM allocates **once per GPU** and every
    /// context maps at its own VA. It is published GPU-wide ([`GlobalCtxPhys`]) and joins
    /// VA halves from **any** address space — which is the whole of §16.50's fix, because
    /// `s41b` measured the two halves arriving under two different procs.
    PerGpu,
    /// ⊘ Nothing in kernel RM ever emits a phase-1 entry for this id. Its arm returns
    /// `NV_OK` with `*pbAddEntry` left `NV_FALSE`.
    ///
    /// ★ Carried as a *named* third value rather than folded into [`Self::PerContext`],
    /// because it is the answer to a different question: a `PerContext` id whose physical
    /// never shows up is a bug in our routing, and a `Never` id whose physical never shows
    /// up is RM behaving exactly as written. A two-valued classifier would report both as
    /// "orphaned" and send the next rung looking for a phase 1 that cannot exist
    /// (`falsifier_blocker_vs_only_blocker`). Its backing has to be recovered at
    /// **allocation** time from `kgraphicsGetGlobalCtxBuffers`, which this port has not
    /// done and which no join can substitute for.
    Never,
}

/// Where the physical half of `external_id` lives — see [`PhysHalfScope`] for the full
/// derivation and the `ogkm` line for every arm.
#[must_use]
pub const fn phys_half_scope(external_id: u16) -> PhysHalfScope {
    match external_id {
        // `:1713-1747` — three per-context memory descriptors.
        0..=2 => PhysHalfScope::PerContext,
        // `:1748-1758` — one fall-through arm, five ids, "No initialization from kernel RM".
        3..=7 => PhysHalfScope::Never,
        // `:1759-1782` — reads a per-context `localCtxBuffer` when one is allocated. ⚠
        8 => PhysHalfScope::PerContext,
        // `:1783-1801` — `kgraphicsGetGlobalCtxBuffers(pGpu, …, gfid)`, unconditionally.
        9..=11 => PhysHalfScope::PerGpu,
        // `:1803-1805` — "No initialization from kernel RM".
        12 => PhysHalfScope::Never,
        // `:1806-1807` — RM's `default:` refuses. We take the narrowest scope instead.
        _ => PhysHalfScope::PerContext,
    }
}

/// Is `external_id` a **global** context buffer — one RM shares across every context —
/// rather than a per-context one?
///
/// ★ This is RM's own membership rule and nothing else:
/// `kgrctxGetGlobalContextBufferInternalId_IMPL` maps `0x3`–`0xc` and returns
/// `NV_ERR_INVALID_ARGUMENT` for MAIN/PM/PATCH
/// (`ogkm-580: kernel_graphics_context.c:214-219`).
///
/// ⊘ **It is NOT the scoping predicate** — see [`PhysHalfScope`] for why six global ids
/// publish no physical at all and one may publish a private one. Exposed because it is the
/// answer to *"is this buffer shared?"*, which is a real and different question, and
/// because stating it separately is what keeps the two from being conflated again.
#[must_use]
pub const fn is_global_ctx_buffer(external_id: u16) -> bool {
    matches!(external_id, 3..=12)
}

/// A physical half published **GPU-wide**: `buffer_id` → the buffer RM allocated once for
/// the whole GPU. See [`PhysHalfScope::PerGpu`].
///
/// ⊘ **Entries are never consumed by a join.** One global buffer is mapped by every
/// context that needs it, so removing it when the first VA space joins against it would
/// re-orphan every later one — the exact failure this map exists to end, reintroduced one
/// layer down.
pub type GlobalCtxPhys = BTreeMap<u16, GlobalPhysHalf>;

/// The published physical half of one global context buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalPhysHalf {
    /// `gpuPhysAddr`, in [`Self::aperture`].
    pub phys: u64,
    /// `size`. Never zero — a zero-length half is [`PromoteJoin::half_unusable`] and is
    /// dropped before it can reach here.
    pub len: u64,
    /// Which aperture [`Self::phys`] is in.
    pub aperture: Aperture,
}

/// ★★★ A half **parked** in a [`crate::gpu::Vas`], waiting for its partner.
///
/// ★ The variant names state what is MISSING, not what is held, because that is the
/// question a census row is read to answer: `AwaitingVa` is a physical buffer with no
/// address yet, `AwaitingPhysical` is an address with nothing behind it. Naming them after
/// what they hold would make [`crate::gpu::Vas::promote_orphans`]'s two numbers
/// indistinguishable at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkedHalf {
    /// Phase 1 arrived; phase 2 has not. **Parked-awaiting-VA.**
    AwaitingVa {
        /// `gpuPhysAddr`.
        phys: u64,
        /// `size`, never zero.
        len: u64,
        /// Which aperture `phys` is in.
        aperture: Aperture,
    },
    /// Phase 2 arrived; phase 1 has not. **Parked-awaiting-physical.**
    AwaitingPhysical {
        /// `gpuVirtAddr`.
        va: GpuVa,
    },
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
    /// ★★★★★ §16.48 — the **half**-declarations, in wire order: one phase of a two-phase
    /// promotion each, joined on `buffer_id` by [`apply_promote_ctx`].
    ///
    /// ⊘ Not a second copy of [`Self::ranges`] and never overlapping it: the ABI
    /// classifier assigns each wire entry exactly one of three states, so an entry is a
    /// complete range **or** a half, never both.
    pub halves: Vec<PromoteHalf>,
    /// What the wire declared in each incomplete state — ★ **unchanged in meaning by the
    /// join**. These count what arrived; [`PromoteJoin`]'s counters say what became of it.
    /// Keeping both is what lets "ten VA halves arrived" and "ten VA halves joined" be
    /// distinguished, which a single number could not.
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
    /// ★★★ §16.44 — the **acting** (envelope) client's namespace is a live declaration of
    /// [`kayfabe_arch::ClientKind::Kernel`].
    ///
    /// Read here, at rank 0, because that is the only rank that holds the
    /// [`Spine`] — [`apply_promote_ctx`] takes the owning [`Proc`] alone and cannot look
    /// a foreign namespace up. See [`PromoteFault::ForeignContextObject`] for what it
    /// licenses and what it deliberately does not.
    ///
    /// ⊘ **`false` on absence, and that is the MISS = FAULT posture, not a default.** A
    /// client with no live root declaration is not "probably a user client" and is not
    /// "probably kernel" either; it groups with nobody
    /// ([`crate::rmgraph::RmGraph::client_kinds`]), and the promotion is refused under
    /// [`PromoteFault::ForeignContextObject`] unless the *component* test passes on its
    /// own. Nothing is permitted by an absence.
    pub acting_kernel: bool,
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
    /// ★★★★★ §16.48 — halves that **completed a parked partner and bound**. This is the
    /// number the whole rung exists to move off zero.
    ///
    /// ⊘ Counted separately from [`Self::bound`] on purpose: `bound` is "a single entry
    /// carried both halves", `joined` is "two controls were stitched together". They are
    /// different mechanisms and a rung that confused them could not tell a two-phase join
    /// working from the guest simply having sent a complete entry.
    pub joined: u32,
    /// Halves **parked** by this promotion, awaiting a partner. Newly parked only.
    pub parked: u32,
    /// Halves that re-declared an **identical** already-parked half — the idempotent
    /// re-promote, at half granularity. Not an error, not progress.
    pub half_already: u32,
    /// ★★ Physical halves declaring **zero length** — counted and dropped, never parked
    /// and never refused. See the arm in [`apply_promote_ctx`] for why all three of those
    /// choices are deliberate and which one would have broken the boot.
    pub half_unusable: u32,
    /// ★★★ **Orphan reading, taken AFTER this promotion applied**, as
    /// `(awaiting_va, awaiting_physical)` over the whole target VAS — see
    /// [`crate::gpu::Vas::promote_orphans`].
    ///
    /// ⚠ This is a **residual**, not an event: it counts halves not joined *yet*. At the
    /// deepest promotion the guest reaches it is the join's own falsifier — a healthy
    /// `joined=N` beside a large orphan count means the key is wrong, and that is
    /// precisely the reading `bound=N` alone could never give.
    pub orphans: (u32, u32),
    /// ★★★★★ §16.50 — of [`Self::joined`], how many completed against a **GPU-published**
    /// physical half ([`PhysHalfScope::PerGpu`]) rather than one parked in this same
    /// address space.
    ///
    /// ⊘ Counted apart from `joined` for the reason `joined` is counted apart from
    /// `bound`: "the two halves were in one VAS and we stitched them" and "the halves were
    /// in two different address spaces and the GPU-wide scope bridged them" are different
    /// mechanisms, and only the second is what this rung changed. A summed number could
    /// not tell the fix working from the previous rung's join working.
    pub joined_global: u32,
    /// ★★★★★ **The counter built for the case where the fix does nothing.**
    ///
    /// How many global physical halves the GPU-wide map holds **after** this promotion. It
    /// is emitted unconditionally on the success path and it does not ride any refusal or
    /// any join, so it reads the same whether `joined_global` is 10 or 0 — which is
    /// exactly the case this rung is most likely to get.
    ///
    /// ★ It is what makes a `joined_global=0` legible instead of mute, and the three
    /// readings are distinguishable with no other evidence:
    ///
    /// | `globals_known` | `joined_global` | reading |
    /// |---|---|---|
    /// | `0` | `0` | ⊘ no [`PhysHalfScope::PerGpu`] physical was **ever** published. The scoping is irrelevant; phase 1 is not arriving for those ids at all, and the next question is `kgraphicsGetGlobalCtxBuffers`, not the join. |
    /// | `>0` | `0` | the map filled and nothing drew on it — the VA halves are for ids that publish nothing ([`PhysHalfScope::Never`]), or the drain is not reaching them. |
    /// | `>0` | `>0` | ★ the cross-address-space bridge fired. |
    ///
    /// ⊘ An instrument hung off the refusal path has its deletion scheduled by the fix it
    /// guides; this one is hung off the success path, so it survives its own fix. That has
    /// now cost two consecutive rungs and it is not paid for a third time.
    pub globals_known: u32,
    /// Global physical halves **this** promotion published into the GPU-wide map. New
    /// publications only — a byte-identical re-publication is not counted here.
    ///
    /// ★ Distinguishes *"the map is filling"* from *"the map was already full"*, which
    /// [`Self::globals_known`] alone cannot: a steady non-zero `globals_known` with
    /// `globals_added=0` on every row means publication happened before the first row we
    /// can see, and that is a fact about **when** phase 1 runs.
    pub globals_added: u32,
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
    /// ★★★ **A foreign USER client is promoting into an address space it is not part
    /// of.**
    ///
    /// `hChanClient`/`hObject` name *someone's* channel; the envelope's `hClient` names
    /// who is asking. Without this check, a client may declare bindings in a **victim's**
    /// address space by naming the victim's client and channel — the params-field
    /// injection the C could not even detect, because it never read the envelope.
    ///
    /// The check is component-scoped, not handle-scoped, precisely because two concurrent
    /// CUDA processes share a duplicated client: the question is *"is the acting
    /// namespace part of this process?"*, which is what [`Proc`] is.
    ///
    /// # ★★★★ §16.44 — what this variant STOPPED refusing, and why the narrowing is RM's
    ///
    /// It used to fire on *any* acting client outside the owning component, and
    /// `[measured 2026-08-09, boot s38_411d280_route]` that refused a stream RM's own
    /// source emits: envelope `0xc1d0000a` (**UVM's** session client), `hChanClient`
    /// `0xc1d0000c` (`cup2`'s), `hObject` `0x5c000019` (`cup2`'s channel). ⊘ The two are
    /// **not** required to be equal, and the emitting site is
    /// `nvGpuOpsBindChannelResources`, which sets
    /// `pParams->hChanClient = RES_GET_CLIENT_HANDLE(pKernelChannel)` — the *user's*
    /// client — and issues the control with `retainedChannel->session->handle` — **UVM's**
    /// — as the envelope (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:10870`,
    /// `:10891-10893`). The `kernel_graphics_object.c` site cited beside
    /// [`crate::promote`]'s module doc is the *other* one, and there they are always
    /// equal because the subdevice is looked up in the graphics object's own client
    /// (`:74-79`).
    ///
    /// ★ So the widened predicate is not "let UVM into `cup2`'s component" — nothing does
    /// that, and §12.27 deliberately keeps every kernel client in the ONE reserved system
    /// component so a global UVM client cannot merge every CUDA process into one. It is
    /// **"a kernel-privileged client may act on a user client's channel"**, which is
    /// *exactly* RM's own rule on this path: reaching
    /// `nvGpuOpsBindChannelResources` at all requires a live `UVM_CHANNEL_RETAINER`
    /// (`0xc574`), and RM registers that class
    /// **`RS_FLAGS_ALLOC_KERNEL_PRIVILEGED`** with
    /// `Parents = RS_LIST(classId(Device), classId(KernelChannelGroupApi))`
    /// (`ogkm-580: src/nvidia/src/kernel/rmapi/resource_list.h:394-400`). Its constructor
    /// resolves the *named* client with `serverGetClientUnderLock` and applies **no
    /// ownership test beyond that privilege gate**
    /// (`ogkm-580: .../gpu/fifo/uvm_channel_retainer.c`), so RM permits precisely what
    /// this now permits, and no more.
    ///
    /// ⊘ **The injection this variant exists for still refuses**, because the widening is
    /// keyed on [`kayfabe_arch::ClientKind::Kernel`], which is *declared* — it comes from
    /// `NV0000_ALLOC_PARAMETERS.processID == 0xFFFF_FFFF`
    /// ([`kayfabe_abi::GuestOs::client_kind_from_process_id`]), a field the guest's
    /// **kernel** RM writes and a guest userspace process never can. A hostile CUDA
    /// process naming a victim's client and channel is a user client, is outside the
    /// owning component, and lands here exactly as before.
    ForeignContextObject {
        /// The envelope's `hClient` — who is **acting**.
        client: HClient,
        /// ★ `hChanClient` — whose handle table [`Self::ForeignContextObject::object`] was
        /// resolved in.
        ///
        /// Carried because this variant is *about* two clients disagreeing and printed
        /// only one of them. `[measured 2026-08-09]` §16.43 had to **infer** `hChanClient`
        /// from a census exemplar and from the previous boot's differently-shaped fault,
        /// and wrote the inference into the design doc as though it were a quoted field.
        /// ⊘ A refusal that cannot name the thing it refuses over is the defect, not the
        /// evidence — `a_wall_that_can_carry_no_name`.
        chan_client: HClient,
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
    /// ★★★ **The range's aperture names no GPGA kind** —
    /// [`kayfabe_mmu::RegionKindFault::PeerHasNoKind`], i.e. the promotion declared
    /// [`kayfabe_arch::Aperture::Peer`]: a second physical GPU's framebuffer, which this
    /// device does not back.
    ///
    /// ⊘ Before [`kayfabe_mmu::RegionKind`] existed this bound silently and became
    /// `Representability::Fabricated` at classify time — fiction over another GPU's memory,
    /// handed to a CPU executor that then had no plane to put it in. The kind is decided
    /// here now, and *"no kind describes this"* is a refusal rather than a fall-through.
    UndecidableKind {
        /// Start of the offending range.
        va: GpuVa,
        /// Why no kind could be decided.
        fault: kayfabe_mmu::RegionKindFault,
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
    /// ★★★ §16.48 — a half **re-declares a `buffer_id` that is already parked, with
    /// DIFFERENT contents**.
    ///
    /// The idempotent case (byte-identical re-declaration) is [`PromoteJoin::half_already`]
    /// and is not an error — the same context buffer is promoted again when a second
    /// channel of a TSG comes up. This variant is the other case: the same `buffer_id` in
    /// the same address space now names a different physical buffer, or a different VA.
    ///
    /// ⊘ Refused rather than overwritten, mirroring [`PromoteFault::Collides`] one level
    /// down. Silently replacing a parked half would let the *second* declaration join
    /// against the *first*'s partner and bind a mapping neither control ever described —
    /// which is manufacturing an address by a slower route than the one MISS = FAULT
    /// already forbids.
    HalfConflict {
        /// The join key that disagreed.
        buffer_id: u16,
        /// The address space it disagreed in.
        pdb: Pdb,
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
    acting: HClient,
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
    // ★★★ §16.44 — the acting namespace's DECLARED kind, read at the only rank that can
    // see it. A linear scan is deliberate: `client_kinds` is a bulk iterator precisely so
    // callers do not grow the O(clients x clients) call pattern it exists to avoid, and a
    // promotion is a handful of controls per process lifetime, not a data-plane verb.
    let acting_kernel = spine
        .rmgraph
        .client_kinds()
        .any(|(k, kind)| k.client == acting && kind == kayfabe_arch::ClientKind::Kernel);
    Ok(PromoteRoute {
        proc,
        gpu,
        pdb,
        acting_kernel,
    })
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
    globals: &mut GlobalCtxPhys,
) -> Result<PromoteJoin, PromoteFault> {
    if proc.id != route.proc {
        return Err(PromoteFault::RetiredProc(route.proc));
    }
    // ★★★ Attribution checked AGAINST resolution, in TWO arms. See
    // `ForeignContextObject` for the measurement and the `ogkm` citations.
    //
    //   (A) the acting client is in the owning component  — `kgrobjPromoteContext`, where
    //       RM looks the subdevice up in the graphics object's OWN client, so envelope
    //       and `hChanClient` are equal by construction;
    //   (B) the acting client is a declared KERNEL client — `nvGpuOpsBindChannelResources`,
    //       where the envelope is UVM's session and `hChanClient` is the user's. RM gates
    //       that path on `RS_FLAGS_ALLOC_KERNEL_PRIVILEGED`, so (B) is RM's own rule and
    //       not a widening past it.
    //
    // ⊘ (B) is NOT "UVM is part of the process". Nothing puts it there and §12.27 keeps
    // every kernel client in the one reserved system component on purpose. What remains
    // refused is a foreign USER client, which is the injection this guard was written for.
    if !route.acting_kernel && !proc.client_values().contains(&p.client) {
        return Err(PromoteFault::ForeignContextObject {
            client: p.client,
            chan_client: p.chan_client,
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
    // ── PASS 1b: STAGE THE TWO-PHASE JOIN. Mutates nothing. ──────────────────────────
    //
    // ★★★★★ §16.48. Halves are folded into a SCRATCH copy of the parking map and the
    // completions they produce are collected; only if every half validates does any of it
    // reach the `Vas`. The scratch copy is what keeps this all-or-nothing without a second
    // rollback path — and it is affordable for exactly the reason the module doc gives for
    // the linear `client_kinds` scan: a promotion is a handful of controls per process
    // lifetime, not a data-plane verb.
    //
    // ⊘ Sequential, so two halves sharing a `buffer_id` INSIDE one promotion see each
    // other. Handling that by scanning `vas.promote_halves` alone would let the second one
    // park over the first with no name attached to it.
    let mut scratch = vas.promote_halves.clone();
    // ★ The `bool` is *"this completion drew on the GPU-wide map"*. Carried alongside the
    // range rather than counted at staging time so that [`PromoteJoin::joined_global`]
    // stays a strict subset of [`PromoteJoin::joined`]: a completion that turns out to be
    // an identical re-promote is skipped in PASS 2, and a `joined_global` incremented
    // where it was staged would have counted a bind that never happened.
    let mut completed: Vec<(PromotedRange, bool)> = Vec::new();
    // ★★ `parked` is what this promotion LEFT parked, not how many parks it performed.
    // A half that parks and is then completed by its partner LATER IN THE SAME CONTROL
    // parked nothing — counting the gross insert would report `parked=1 joined=1` for a
    // control that ended with an empty map, and a reader would go looking for an orphan
    // that does not exist.
    let mut parked_ids: BTreeSet<u16> = BTreeSet::new();
    let mut half_already = 0u32;
    let mut half_unusable = 0u32;
    let mut joined_global = 0u32;
    let mut globals_added = 0u32;

    // ── PASS 1a: DRAIN this VAS's already-parked VA halves against the GPU-wide map. ──
    //
    // ★★★★★ §16.50, and this pass is the one that makes the fix apply to the state
    // `s41b` actually measured. Those ten `AwaitingPhysical` halves were parked by
    // *earlier* promotions; the physicals they wait on were published under a different
    // proc before cup2 existed. Without a drain the scoping would only ever help halves
    // that arrive from now on, and the boot would report `joined_global=0` for a reason
    // that has nothing to do with whether the scoping is right.
    //
    // ⊘ Only [`PhysHalfScope::PerGpu`] ids are drained. A `PerContext` half parked here
    // is waiting on a physical that belongs to THIS address space, and completing it from
    // a GPU-wide publication would be inventing a binding.
    for (buffer_id, g) in globals.iter() {
        if phys_half_scope(*buffer_id) != PhysHalfScope::PerGpu {
            continue;
        }
        if let Some(ParkedHalf::AwaitingPhysical { va }) = scratch.get(buffer_id) {
            completed.push((
                PromotedRange {
                    va: *va,
                    len: g.len,
                    phys: g.phys,
                    aperture: g.aperture,
                    buffer_id: *buffer_id,
                },
                true,
            ));
            scratch.remove(buffer_id);
        }
    }

    for h in &p.halves {
        match *h {
            PromoteHalf::Physical {
                phys,
                len,
                aperture,
                buffer_id,
            } => {
                // ★★★★ A zero-length physical half is COUNTED AND DROPPED, never refused
                // and never parked.
                //
                // ⊘ Refusing it was this rung's first draft and it would have been a
                // silent behaviour change: the ABI classifier's last arm sends *any*
                // all-zero entry here (`view.rs`, rule 4 — "otherwise ⇒ InitializeOnly"),
                // and such entries already reach us today, where they are counted into
                // `declined.initialize_only` and harmlessly ignored. Turning them into a
                // whole-control refusal would make a change advertised as a pure join
                // refuse traffic the guest sends now — the one outcome that scores this
                // rung as broken rather than as measured.
                //
                // ★ Parking it would be worse than either: it can never produce a bindable
                // range, so it would sit in the map forever and inflate the orphan count
                // with an orphan WE created. `injection_measures_necessity_never_sufficiency`
                // has a sibling here — an instrument must not manufacture its own readings.
                if len == 0 {
                    half_unusable += 1;
                    continue;
                }
                // ★★★★★ §16.50 — a GPU-scoped physical is PUBLISHED, not parked.
                //
                // `s41b` measured this half arriving under one proc and its VA halves
                // under another, so parking it in the emitting proc's `Vas` is what
                // guaranteed `joined=0`: the two halves were correct, keyed correctly, and
                // in two different maps.
                //
                // ⊘ It is published IN ADDITION to falling through to the ordinary
                // per-VAS arm below, never instead of it. The emitting address space is
                // as entitled to join against its own declaration as any other, and
                // routing it away from the local map would trade one orphan class for
                // another.
                if phys_half_scope(buffer_id) == PhysHalfScope::PerGpu {
                    let publish = GlobalPhysHalf {
                        phys,
                        len,
                        aperture,
                    };
                    match globals.get(&buffer_id) {
                        // ★ A DIFFERING re-publication refuses by name rather than
                        // overwriting. Overwriting would silently retarget every context
                        // that already joined against the old value — a wrong table, which
                        // `HalfConflict` exists precisely to prevent expressing.
                        Some(prev) if *prev != publish => {
                            return Err(PromoteFault::HalfConflict {
                                buffer_id,
                                pdb: route.pdb,
                            });
                        }
                        Some(_) => {}
                        None => {
                            globals.insert(buffer_id, publish);
                            globals_added += 1;
                        }
                    }
                }
                match scratch.get(&buffer_id) {
                    Some(ParkedHalf::AwaitingPhysical { va }) => {
                        completed.push((
                            PromotedRange {
                                va: *va,
                                len,
                                phys,
                                aperture,
                                buffer_id,
                            },
                            false,
                        ));
                        scratch.remove(&buffer_id);
                        parked_ids.remove(&buffer_id);
                    }
                    Some(ParkedHalf::AwaitingVa {
                        phys: p0,
                        len: l0,
                        aperture: a0,
                    }) => {
                        if (*p0, *l0, *a0) == (phys, len, aperture) {
                            half_already += 1;
                        } else {
                            return Err(PromoteFault::HalfConflict {
                                buffer_id,
                                pdb: route.pdb,
                            });
                        }
                    }
                    None => {
                        scratch.insert(
                            buffer_id,
                            ParkedHalf::AwaitingVa {
                                phys,
                                len,
                                aperture,
                            },
                        );
                        parked_ids.insert(buffer_id);
                    }
                }
            }
            PromoteHalf::Virtual { va, buffer_id } => match scratch.get(&buffer_id) {
                Some(ParkedHalf::AwaitingVa {
                    phys,
                    len,
                    aperture,
                }) => {
                    completed.push((
                        PromotedRange {
                            va,
                            len: *len,
                            phys: *phys,
                            aperture: *aperture,
                            buffer_id,
                        },
                        false,
                    ));
                    scratch.remove(&buffer_id);
                    parked_ids.remove(&buffer_id);
                }
                Some(ParkedHalf::AwaitingPhysical { va: v0 }) => {
                    if *v0 == va {
                        half_already += 1;
                    } else {
                        return Err(PromoteFault::HalfConflict {
                            buffer_id,
                            pdb: route.pdb,
                        });
                    }
                }
                // ★★★★★ §16.50 — the local map missed, so ask the GPU-wide one before
                // parking. This is the arm that binds a VA declared by cup2's address
                // space against a physical RM published once, long before, under the
                // driver-init proc.
                None => match globals.get(&buffer_id) {
                    Some(g) if phys_half_scope(buffer_id) == PhysHalfScope::PerGpu => {
                        completed.push((
                            PromotedRange {
                                va,
                                len: g.len,
                                phys: g.phys,
                                aperture: g.aperture,
                                buffer_id,
                            },
                            true,
                        ));
                    }
                    // ⊘ Includes the case where a publication EXISTS for an id whose
                    // scope is not `PerGpu`. It cannot be reached today — nothing
                    // publishes such an id — but the scope test is repeated at the point
                    // of USE rather than trusted from the point of insertion, because a
                    // future caller building the map itself would otherwise silently gain
                    // cross-context joins for private buffers.
                    _ => {
                        scratch.insert(buffer_id, ParkedHalf::AwaitingPhysical { va });
                        parked_ids.insert(buffer_id);
                    }
                },
            },
        }
    }

    // ── PASS 1c: the completions are ordinary ranges and face the ORDINARY laws. ──────
    //
    // ⊘ A range assembled from two controls gets no weaker check than one that arrived
    // whole. It is validated against the same wrap/zero rule, against this promotion's own
    // complete ranges, against the other completions, and against the table.
    for (r, _) in &completed {
        if r.len == 0 || r.va.0.checked_add(r.len).is_none() {
            return Err(PromoteFault::Malformed {
                va: r.va,
                len: r.len,
            });
        }
    }
    for (i, (a, _)) in completed.iter().enumerate() {
        for b in completed
            .iter()
            .skip(i + 1)
            .map(|(r, _)| r)
            .chain(p.ranges.iter())
        {
            let overlap = a.va.0 < b.va.0.saturating_add(b.len) && b.va.0 < a.va.0 + a.len;
            if overlap {
                return Err(PromoteFault::SelfOverlap { a: a.va, b: b.va });
            }
        }
    }

    // Which ranges are already there, byte-identically, from a previous promotion.
    let mut already: BTreeSet<u64> = BTreeSet::new();
    for r in p.ranges.iter().chain(completed.iter().map(|(r, _)| r)) {
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
                && b.phys() == r.phys
                && b.aperture() == r.aperture
                && b.host().is_none()
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
    let mut joined = 0u32;
    for (r, from_join, from_global) in p
        .ranges
        .iter()
        .map(|r| (r, false, false))
        .chain(completed.iter().map(|(r, g)| (r, true, *g)))
    {
        if already.contains(&r.va.0) {
            continue;
        }
        // ★★★ THE DECISION, at the bind site. The promotion declares an aperture; for a
        // range with nothing host-side reachable from the table that aperture IS the kind
        // — `Vidmem` → kind 2, sysmem → kind 4 — and `Peer` is refused by name instead of
        // becoming fiction over another GPU's framebuffer.
        //
        // ⚠ **A residual disagreement, named rather than papered over.** This chain's own
        // comment says *the HOST allocated and mapped this GR context buffer for itself*,
        // which is the owner's kind 3 — but no [`kayfabe_mmu::HostBacking`] reaches this
        // site, and kind 3 without one is exactly the state
        // [`kayfabe_mmu::Binding::real_gpu_memory`] refuses to let anyone write. So the
        // truthful declaration here is the guest-declared one, and the *gap* is that the
        // host object this range was allocated from is not carried to the bind. That is a
        // supply-side change (`promote_ctx` would have to receive the backing), not a
        // relabelling, and it is deliberately not made here.
        let binding = kayfabe_mmu::Binding::declared_by_guest(r.phys, r.aperture)
            .map_err(|fault| PromoteFault::UndecidableKind { va: r.va, fault })?;
        vas.table
            .bind(route.pdb, r.va, r.len, binding)
            .map_err(|_| PromoteFault::Collides {
                va: r.va,
                len: r.len,
            })?;
        vas.promote_bound.insert(r.va.0);
        // ★ `bound` and `joined` are counted apart, never summed here: see
        // [`PromoteJoin::joined`]. A caller wanting the total adds them itself and is
        // then visibly making that choice.
        if from_join {
            joined += 1;
            // ★ Strict subset of `joined`, counted HERE and not where the completion was
            // staged — see the `completed` declaration.
            if from_global {
                joined_global += 1;
            }
        } else {
            bound += 1;
        }
    }
    vas.promote_halves = scratch;
    Ok(PromoteJoin {
        route: *route,
        bound,
        already: u32::try_from(already.len()).unwrap_or(u32::MAX),
        declined: p.declined,
        joined,
        parked: u32::try_from(parked_ids.len()).unwrap_or(u32::MAX),
        half_already,
        half_unusable,
        orphans: vas.promote_orphans(),
        joined_global,
        globals_known: u32::try_from(globals.len()).unwrap_or(u32::MAX),
        globals_added,
    })
}
