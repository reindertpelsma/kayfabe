//! The runtime ownership spine: [`Gpu`] → [`Proc`] → [`Vas`]/[`Channel`]
//! (arch doc §4.3.1). The single most important design decision of the rewrite:
//! **the per-process container owns all four planes — address, execution,
//! completion, isolate — keyed on PDB + vChid, from the first line of code.**
//!
//! [`Gpu`] holds the [`RmGraph`] as the source of truth and keeps its runtime
//! state **synced to the graph's projections** after every applied event: `Proc`s
//! are created/retired as dup-connected components appear/merge/free; `Vas`es
//! appear when a PDB is declared; `Channel`s when channel nodes appear. Routing
//! maps (`by_pdb`, `by_vchid`) are always rebuilt from the projection — never
//! accreted from event order.
//!
//! Single-process is the N=1 case of the only code path: there is no
//! `multiproc()` gate and no arming window (lesson L9).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kayfabe_arch::Arch;
use kayfabe_arch::fault::ErrorNotifier;
use kayfabe_arch::ids::{ClassId, EngineKind, GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_completion::{CompletionQueue, DeliveryPlane, FenceArms, PostBatch};
use kayfabe_isolate::{
    CancelReason, Cancels, HostHandle, Isolate, IsolateBox, IsolateCensus, IsolateFactory,
    IsolateId, Orphans, Worker,
};
use kayfabe_mmu::{AddressFault, AddressTable};
use kayfabe_util::Instant;

use crate::gpa::{GpaArena, GpaBlock, GpaError, GpaSpace};
use crate::project::{Boundaries, ProcBoundary, ProjectionError, SYSTEM_ANCHOR, project};
use crate::reactor::SourceRegistry;
use crate::rmgraph::{ClientId, ClientKey, NodeKey, ResourceKey, RmEvent, RmGraph, RmGraphError};
use crate::{ChanId, ProcAnchor, ProcId};

/// ★ G10 (`l1_concurrency.md` §12.22) — the largest number of distinct **condemned
/// components** the device carries before it refuses to derive new processes.
///
/// A condemned entry is only dropped when the guest frees its client root, which a guest
/// that just crashed its own worker has no incentive to do; the list was unbounded and
/// was rescanned on **every** apply. Sized far above any real workload (a machine with
/// this many *simultaneously crashed, unreaped* GPU processes has a different problem)
/// and far below `MAX_LIVE_HANDLES`, so it never trips a benign guest.
pub const MAX_CONDEMNED_COMPONENTS: usize = 1024;

/// How many channels [`Gpu::vas_census_string`] names per outcome group before it counts
/// the rest.
///
/// ⊘ The overflow is **reported** (`+N more`), never silently dropped: a census that
/// printed three rows out of forty while looking complete is the shape
/// `unserviced_len`/`bridge_refusal_len` already refuse.
pub const VAS_CENSUS_EXEMPLARS: usize = 3;

/// ★ G10 — the largest number of **retired-but-unreaped** procs the device carries before
/// it refuses to derive new processes. Reaping is the adapter's call (the L10 quiesce
/// edge), so an adapter that never reaches one, or a guest that keeps a proc non-quiesced,
/// would otherwise grow this list without limit — each entry holding an isolate and a GPA
/// arena.
pub const MAX_RETIRED_PROCS: usize = 1024;

/// Which device-global list hit its cap ([`GpuError::SpineCapacity`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpineCapacity {
    /// [`MAX_CONDEMNED_COMPONENTS`].
    CondemnedComponents,
    /// [`MAX_RETIRED_PROCS`].
    RetiredProcs,
}

/// Errors surfaced by [`Gpu::apply`]. All loud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuError {
    /// The graph refused the event (protocol violation).
    Graph(RmGraphError),
    /// The projection found an inconsistency (collision / dangling edge).
    Projection(ProjectionError),
    /// Two `Proc`s merged (a dup edge joined their components) after the absorbed
    /// one had already touched its data plane. The protocol's early-arm point
    /// (the UVM dup precedes any data-plane use — arch doc §4.3.4) makes this
    /// unreachable for a sane guest; a hostile one gets a loud refusal, never a
    /// silent state fold (lesson L9).
    LateMerge {
        /// The surviving proc.
        kept: ProcId,
        /// The proc that could not be absorbed.
        absorbed: ProcId,
    },
    /// GPA window/arena exhaustion.
    Gpa(GpaError),
    /// The address table refused an RPC-map forward-population (overlap/miss).
    Address(AddressFault),
    /// A `MapMemoryDma` named a memory resource with no declared backing — the RPC
    /// populate source cannot resolve `memory → phys`. MISS=FAULT (never guess).
    UnbackedMapping {
        /// The VAS's PDB.
        pdb: Pdb,
        /// The faulting mapping VA.
        va: u64,
    },
    /// ★ MG-6: a GPU target was presented under a **different architecture** than the
    /// device was realized with. V1 multi-GPU is **homogeneous-arch only**
    /// (`multi_gpu_and_mig.md`): a heterogeneous/mixed-arch config is refused loudly at
    /// realize, never silently misbehaved. (Heterogeneous multi-arch is explicitly out
    /// of scope — the seam audit's avoid-list.)
    HeterogeneousArch {
        /// The target whose arch disagreed.
        gpu: GpuId,
        /// The device's realized arch name.
        expected: &'static str,
        /// The arch the target was presented under.
        got: &'static str,
    },
    /// ★ G10 (§12.22) — a device-global, guest-reachable list is at its cap, so the
    /// device refuses to derive a **new** `Proc`.
    ///
    /// **Why the refusal is here and not at the growth site.** The two lists grow at
    /// `Spine::retire_proc` (a worker died) and at the graph-driven retire — and refusing
    /// *there* would be worse than useless. Refusing a condemnation **un-condemns** a
    /// component whose isolate is already dead, turning a bounded resource problem into a
    /// use-after-death and a silent-corruption path (§12.13's whole argument). Refusing a
    /// retirement leaves a proc live whose worker is gone. Dropping corpses instead would
    /// leak the isolates and GPA arenas they hold.
    ///
    /// So the refusal lands on the only guest-reachable action that *consumes* the
    /// resource: deriving a new process. Every existing proc keeps serving, nothing is
    /// un-condemned, nothing is dropped, and the guest recovers exactly as it does from a
    /// single condemnation — by freeing the dead components' client roots, which prunes
    /// the entries and clears the condition. Backpressure, not a brick.
    SpineCapacity {
        /// Which list.
        what: SpineCapacity,
        /// Its cap.
        cap: usize,
    },
}

impl From<RmGraphError> for GpuError {
    fn from(e: RmGraphError) -> Self {
        GpuError::Graph(e)
    }
}
impl From<ProjectionError> for GpuError {
    fn from(e: ProjectionError) -> Self {
        GpuError::Projection(e)
    }
}
impl From<GpaError> for GpuError {
    fn from(e: GpaError) -> Self {
        GpuError::Gpa(e)
    }
}

/// One guest GPU virtual address space — THE memory boundary (decision #14).
///
/// Owns the forward-populated address table for its PDB and its **own host VAS**
/// (materialized lazily by the fwd plane): per-Vas host separation is the proven
/// #14 fix — identical guest VAs in distinct `Vas`es publish into distinct host
/// address spaces and cannot collide. Address ops key HERE, never on [`Proc`].
pub struct Vas {
    /// ★ MG-4: the GPU target this VAS lives on (graph-derived from its `Device`
    /// ancestor, never guessed). `Pdb` is a per-GPU namespace, so a `Vas` is keyed by
    /// `(GpuId, Pdb)` in [`Proc::vases`] and this tag disambiguates identical PDBs on
    /// different GPUs.
    pub gpu: GpuId,
    /// The hardware identity (the GPU's CR3), unique only WITHIN [`Self::gpu`].
    pub pdb: Pdb,
    /// The VASpace origin node in the RM graph.
    pub origin: ResourceKey,
    /// The forward-populated VA→backing table (MISS=FAULT).
    pub table: AddressTable,
    /// This Vas's own host VAS object, once materialized by the fwd plane.
    pub host_vas: Option<HostHandle>,
    /// Captured page-table pages of this VAS (#13's per-PDB `m2_cpt` equivalent;
    /// populated by the CE-PT-write capture feed once the mmu port lands).
    pub pt_pages: BTreeSet<u64>,
    /// ★★★ #102 stage C3 — **the forward-populated page-table metadata chain**: for each
    /// known table page, the level it sits at and the virtual address its entry 0
    /// describes.
    ///
    /// A page of eight-byte words is the same bytes at every level; what it *means*
    /// depends entirely on where in the tree it hangs, so a direct decode of a dirtied
    /// page cannot happen without this. The C carries the same triple in `m2_cpt` and
    /// fills it from a **discovery sweep** that it *"reset + rebuilt on every recorded
    /// walk"* (`C: nvkvm_gpu_emul.c:604`).
    ///
    /// ★ This port does not port that sweep, and does not need to. Level 0 is a
    /// **declared** fact — a PDB *is* its own root page — and every decode then hands each
    /// child its own level and `vabase`, so the chain grows forward from the root, one
    /// observed write at a time. Nothing here is derived backwards from a physical
    /// address, and nothing sweeps a tree looking for what might be a page table.
    ///
    /// Keyed by physical page address; the root is **not** stored, because a fact that is
    /// declared does not need a cache that can disagree with it. Bounded by
    /// `kayfabe_fwd::MAX_PT_META`, which is guest-influenced and therefore a boundary-1
    /// concern rather than a tidiness one.
    pub pt_meta: BTreeMap<u64, kayfabe_mmu::walker::PtPage>,
    /// ★★★ **The reachability shadow** — `resume_from_fault.md` §7 step 4, model in
    /// `reachability_on_transition.md`.
    ///
    /// Answers the question [`Vas::pt_meta`] and [`Vas::table`] together cannot: *is this
    /// leaf reachable from the page-directory root, and did we see the guest write it?*
    /// A leaf binds only if both, so a page-table page filled before anything points at
    /// it waits for its link (hole 1) and a directory entry read out of allocator residue
    /// can make a page reachable but can never bind a mapping out of it (hole 2).
    ///
    /// ★ It lives on the `Vas` and nowhere else, and that is hole 5: a `Vas` is keyed by
    /// `(GpuId, Pdb)`, so a `SET_PAGE_DIRECTORY` rebind — which re-points an entire
    /// address space with **zero** entry writes — mints a different key and therefore a
    /// different shadow. Nothing carries over. `ReachShadow::audit_root` is the standing
    /// check that this held.
    pub reach: kayfabe_mmu::reach::ReachShadow,
    /// ★ G6 (§12.20): the live [`GpaBlock`] behind each **host-published** VA — the
    /// token that lets that GPA range be given BACK to the proc's arena instead of
    /// leaking until the whole proc is reaped. Keyed by VA, exactly like the binding it
    /// accompanies; `Binding` is `Copy` and a free token must not be (that is what makes
    /// the double free unrepresentable), so the two live side by side — the same split
    /// G1 made between the placement and the allocation.
    pub blocks: BTreeMap<u64, GpaBlock>,
    /// ★★★★★ **The guest-RAM pins live in this VAS** — VA → what was built over the
    /// guest's own pages there (`guest_ram_crossing.md` §5.8).
    ///
    /// ## ⊘ It is an IDEMPOTENCE SET before it is a record, and that is not tidiness
    ///
    /// A doorbell fires many times on one channel, and the pin is driven from a doorbell.
    /// Without this map the second doorbell would issue a second `OS_DESCRIPTOR` over the
    /// same pages and a second **fixed** `map_dma` at an address the first one already
    /// occupies — and RM answers that with `0x51 NV_ERR_NO_MEMORY`, which is
    /// indistinguishable from genuine exhaustion. ⇒ The failure this map prevents is not a
    /// leak, it is a **refusal whose cause cannot be read off it**.
    ///
    /// ⊘ Keyed by VA within this `Vas`, exactly like [`Self::blocks`], because a `Vas` is
    /// `(GpuId, Pdb)` and a pin is only meaningful in the address space it was placed in.
    pub guest_ram_pins: BTreeMap<u64, GuestRamPin>,
    /// ★★★★★ **Whole-VAS sweep bookkeeping** — see [`PtSweepState`]. Default is *"never
    /// swept"*, which is a trigger, not a steady state.
    pub sweep: PtSweepState,
    /// VAs currently bound into `table` by the **RPC map source** (`MapMemoryDma`),
    /// so the sync can idempotently add/remove them without disturbing bindings from
    /// other populate sources (`publish_backing`, CE-PT-write capture).
    pub rpc_bound: BTreeSet<u64>,
    /// ★★ VAs currently bound into `table` by the **context-promotion source**
    /// (`GPU_PROMOTE_CTX` — [`crate::promote`]), i.e. that source's own idempotence set.
    ///
    /// Deliberately NOT [`Self::rpc_bound`], and the separation is load-bearing rather
    /// than tidy: [`Spine::sync_rpc_mappings`] builds its desired set exclusively from
    /// `RmGraph::mappings()` and unbinds every `rpc_bound` VA that is not in it. A
    /// promotion is not a `MapMemoryDma` and never will be — no `RmEvent::MapMemoryDma`
    /// has a producer on a GSP client at all — so a promote binding filed under
    /// `rpc_bound` would be silently reaped on the very next [`Spine::apply`]. The
    /// failure mode is a table that is correct immediately after the control and empty a
    /// moment later, which reads as a race and is not one.
    pub promote_bound: BTreeSet<u64>,
    /// ★★★★★ §16.48 — **the two-phase promote join's parked halves**, keyed by
    /// `NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ID_*`.
    ///
    /// For an **externally-owned (UVM) VA space** RM promotes one context buffer in two
    /// separate controls, and neither carries a bindable range on its own:
    ///
    /// | phase | emitter | writes | our decode |
    /// |---|---|---|---|
    /// | 1 — physical | `kgrctxPrepareInitializeCtxBuffer` (`ogkm-580: kernel_graphics_context.c:1843-1849`) | `gpuPhysAddr`, `size`, `physAttr`, `bufferId`, `bNonmapped=1` | [`crate::promote::PromoteHalf::Physical`] |
    /// | 2 — virtual | `nvGpuOpsBindChannelResources` (`ogkm-580: nv_gpu_ops.c:10886-10888`) | `bufferId`, `gpuVirtAddr` — and **nothing else**, the params struct is `portMemSet` to 0 at `:10869` | [`crate::promote::PromoteHalf::Virtual`] |
    ///
    /// RM states the split in a comment on the sibling falcon path
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/falcon/kernel_falcon.c:217`, inside the
    /// `gvaspaceIsExternallyOwned(pGVAS)` branch): *"Promote physical address only. VA
    /// will be promoted later as part of nvgpuBindChannelResources"*. The GR path states
    /// it as code rather than prose — `kgrctxPreparePromoteCtxBuffer_IMPL` opens with
    /// *"RM is not responsible for promoting the buffers when UVM is enabled"* and
    /// returns `*pbAddEntry = NV_FALSE` for an externally-owned VAS
    /// (`ogkm-580: kernel_graphics_context.c:1883-1885`), so phase 1 **cannot** carry a VA
    /// and phase 2 **cannot** carry a physical.
    ///
    /// ⊘ `[measured 2026-08-09, boot s40_4733730_acceptcensus]` **eleven promotions were
    /// answered `NV_OK` and bound zero ranges**, because a `PromotedRange` required both
    /// halves in one entry. This map is the join that fixes it — and ⊘ it joins, it never
    /// **invents**: a half with no partner stays parked and is *countable*
    /// ([`Vas::promote_orphans`]), because a join that silently dropped halves would show
    /// a healthy `bound=N` over a table that is still wrong.
    ///
    /// # ★★ Keyed per-VAS, and that is a claim this rung MEASURES rather than assumes
    ///
    /// The key is `buffer_id` **within this `Vas`**, i.e. `(GpuId, Pdb, buffer_id)`
    /// globally — not `(chan_client, hObject, buffer_id)` as `§16.47.5` proposed. Handles
    /// are recyclable and this port resolves them to stable identities at rank 0 before
    /// anything is keyed on them; both emitters name the **same channel** in the **same**
    /// client namespace (phase 1 uses `RES_GET_PARENT_HANDLE(pChannelDescendant)`, phase 2
    /// `RES_GET_HANDLE(pKernelChannel)`), so both route to the same `(gpu, pdb)` whenever
    /// the channels share a VA space.
    ///
    /// ⚠ **When they do NOT share one, this key orphans the halves rather than joining
    /// them** — phase 1 runs once per GR context (`bKGrMainCtxBufferInitialized`), so a
    /// second VA space's phase 2 finds no physical parked. That is a real limit, it is
    /// deliberately *visible* instead of papered over, and [`Vas::promote_orphans`] is the
    /// number that will say whether it happens. ⊘ Widening the key on a guess is exactly
    /// the `measure_before_reasoning_is_the_order` mistake.
    pub promote_halves: BTreeMap<u16, crate::promote::ParkedHalf>,
}

impl Vas {
    /// ★★★★★ **w318 — EVERYTHING THE PUBLICATION CENSUS READS, AS ONE COMPARABLE VALUE.**
    ///
    /// `(table generation, guest-RAM pin count)`. Two terms because
    /// `SharedDevice::vas_publish_census` reads exactly two mutable things off a `Vas`:
    ///
    /// - `self.table` — every row it buckets, and every `Binding::host` it tests. Covered by
    ///   [`kayfabe_mmu::AddressTable::generation`], which is bumped at the table's only two
    ///   write sites.
    /// - `self.guest_ram_pins` — the `already_pinned` bucket, which is a **containment test
    ///   against this map**. Pins are only ever inserted (`commit_pin_guest_ram`) and dropped
    ///   with the whole `Vas`, so a length is a faithful epoch for it; ⊘ if a *removal* of a
    ///   single pin is ever added, this term must become a generation too, and the type here
    ///   is what a reader will find when they look.
    ///
    /// # ⊘ WHAT THIS DOES NOT COVER, said here rather than discovered by a fault
    ///
    /// It is an epoch of **our record**, not of the host. The publication verb's outcome also
    /// depends on host state this cannot see — a framebuffer range already joined, an isolate
    /// that went away. A gate built on this owes a second term for that; the shim's
    /// `PublishStamp` carries one and names it.
    #[must_use]
    pub fn publish_epoch(&self) -> (u64, usize) {
        (self.table.generation(), self.guest_ram_pins.len())
    }

    /// ★★★ **The orphan count** — parked halves that never found their partner, split by
    /// which half is missing.
    ///
    /// Returns `(awaiting_va, awaiting_physical)`: how many `buffer_id`s hold a phase-1
    /// physical with no phase-2 VA, and how many hold a phase-2 VA with no phase-1
    /// physical.
    ///
    /// ⊘ **An orphan is only knowable as a residual**, never at the moment a half arrives:
    /// "this half's partner is never coming" is a statement about the future. So this is a
    /// *current-state* reading, and it means "not joined **yet**" at the instant it is
    /// taken. Read at the deepest point the guest reached, a non-zero count is the join's
    /// own falsifier — see [`crate::promote::PromoteJoin`].
    #[must_use]
    pub fn promote_orphans(&self) -> (u32, u32) {
        let mut awaiting_va = 0u32;
        let mut awaiting_phys = 0u32;
        for h in self.promote_halves.values() {
            match h {
                crate::promote::ParkedHalf::AwaitingVa { .. } => awaiting_va += 1,
                crate::promote::ParkedHalf::AwaitingPhysical { .. } => awaiting_phys += 1,
            }
        }
        (awaiting_va, awaiting_phys)
    }

    /// ★★★★★ **w310 — THE REMOVAL. `guest_ram_pins` had none, and that was the leak.**
    ///
    /// `docs/audits/w301_cancellation_error_leaks.md` §3.2, verified still true at master
    /// `74200b2b`: [`Vas::guest_ram_pins`] was a `BTreeMap` with `insert`/`get`/`range`/`len`
    /// and **no `remove`, `retain`, `clear` or `drain` anywhere in the tree**. Dropping the
    /// `Vas` dropped the map and **lost the handles**, so the objects became unnameable and
    /// no reclaim could ever be written for them.
    ///
    /// ⊘ **`take`, not `remove`, and the shape is the point.** A per-VA removal would invite
    /// a partial reclaim — and a pin released while its `Vas` is live is exactly the unproven
    /// act [`PinReleaseVerdict::RefusedVasLive`] refuses. The only caller is
    /// [`Spine::stage_dropped_vases`], which owns the whole dying `Vas`; taking the map
    /// **empties the record in the same statement that stages its disposal**, so "recorded
    /// but not staged" and "staged but still recorded" are both unrepresentable.
    pub fn take_guest_ram_pins(&mut self) -> BTreeMap<u64, GuestRamPin> {
        core::mem::take(&mut self.guest_ram_pins)
    }
}

/// ★★★★★ **w310 — MAY THIS GUEST-RAM PIN BE RELEASED? The predicate, as a value.**
///
/// # ⊘ The hazard on both sides, because only naming one of them produces a wrong answer
///
/// **Not releasing** keeps a live, RM-pinned host-GPU translation into guest pages the guest
/// has freed and its kernel has handed to a *different* guest process. The host GPU can still
/// write there, and — measured by w307 — **no fault, no `Xid`, no notifier**: the guest's
/// unmap arrives as page-table writes, and the translation we keep is precisely the one the
/// engine would otherwise have faulted on. A silent cross-process write inside the guest.
///
/// **Releasing too early** causes exactly the fault we are preventing, except ours: the
/// engine walks a translation we removed under it.
///
/// # ⊘ What CANNOT be leaned on
///
/// [`kayfabe_isolate::Isolate::is_quiesced`] is `in_flight() == 0` — *no worker checked out*
/// — and its own doc is titled *"★ This is NOT 'the device is quiescent' — do not conflate
/// them"*. **There is no GPU quiescence fence anywhere in this tree** (w301 §3.3, with the
/// known-positive that found `await_semaphore` and used it nowhere on a teardown path). So
/// the safe release cannot be *"prove the engine is idle"*; nothing here can.
///
/// # ★★★ The predicate that IS provable — the `PREEMPT` shape, one level over
///
/// w303 made `NVA06C_CTRL_CMD_PREEMPT` honest not by fencing anything but by proving the
/// group *"has no host twin, so nothing ever reached the GPU, so it is idle by
/// construction."* The analogue here is not about the engine at all:
///
/// > **A guest-RAM pin's only GPU-visible mapping lives in exactly one host VAS — the
/// > [`Vas::host_vas`] of the `Vas` that records it** ([`kayfabe_isolate::VerbPlan::PinGuestRam`]
/// > maps into `host_vas` and nowhere else; its refusal arms unmap from that one VAS).
/// > **When that `Vas` dies, `stage_dropped_vases` already stages `free(host_vas)` — and
/// > freeing a VAS destroys every mapping in it.** ⇒ Releasing the pin in the same batch
/// > cannot expose the engine to anything the batch already exposes it to. It does not make
/// > the mapping die sooner in any observable way; it makes the **descriptor and the page
/// > pin** die with it instead of outliving it.
///
/// ⚠ Stated as strictly as it deserves: this is **not** a proof that the engine is idle. It
/// is a proof that the release **adds no exposure**, over a teardown that is happening
/// regardless. The residual — a VAS freed under a running engine — is pre-existing, is the
/// same class `stage_dropped_vases` has always had for `Binding::host` rows, and is named in
/// this rung's report rather than silently inherited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinReleaseVerdict {
    /// ★ **Provably safe** — the `Vas` is dying and this is the host VAS the same batch
    /// frees. Unmap from it, free the descriptor, `munmap` the isolate's window.
    Release(HostHandle),
    /// ⊘ **Refused: the pin records no host VAS.** A pin implies one
    /// (`commit_pin_guest_ram` adopts `fresh_vas` into `Vas::host_vas` at the moment it
    /// inserts), but the type does not enforce it, and if it is ever false the mapping
    /// cannot be *named*. Freeing the descriptor anyway would let RM auto-unmap it from a
    /// VAS we cannot see — an unmap whose safety is exactly what this enum exists to decide.
    /// ⇒ Refused by name, counted, and the pin is left to the isolate's death.
    RefusedNoHostVas,
    /// ⊘⊘ **Refused: the `Vas` is LIVE.** This is the release we deliberately do **not**
    /// build. Reclaiming a pin whose `Vas` survives — a housekeeping GC over ranges the
    /// guest has unmapped — needs the one thing the tree does not have: a GPU fence.
    /// Nothing in the batch would be freeing the host VAS, so the argument above evaporates
    /// and the unmap stands alone, under a possibly-running engine.
    ///
    /// ★ **An unpin you cannot justify is worse than the leak**: the leak is silent about
    /// memory the guest is done with; a premature unpin corrupts live work. This variant is
    /// the refusal being a *shippable answer*, and it is exercised by
    /// `tests/tests/guest_ram_pin_release.rs` so the absence is a value rather than prose.
    RefusedVasLive,
}

/// Decide [`PinReleaseVerdict`] for one pin. See that type for the whole argument.
///
/// ⊘ Deliberately total and free of state: the decision is a function of *"is this `Vas`
/// being torn down in this batch"* and *"can its host VAS be named"*, and nothing else. A
/// predicate that consulted live state could be right at the moment it was asked and wrong
/// by the time the verb ran.
#[must_use]
pub fn classify_pin_release(vas_is_dying: bool, host_vas: Option<HostHandle>) -> PinReleaseVerdict {
    match (vas_is_dying, host_vas) {
        (false, _) => PinReleaseVerdict::RefusedVasLive,
        (true, None) => PinReleaseVerdict::RefusedNoHostVas,
        (true, Some(h)) => PinReleaseVerdict::Release(h),
    }
}

/// ★★ **w310 — what one [`Spine::stage_dropped_vases`] did with the pins it found.**
///
/// Exists because *"the release path is wired"* and *"the release path ran"* are different
/// claims, and only the second is about a shipped archive (`a_census_zero_needs_a_known_positive`).
/// `released` is the number a boot log prints and a bench criterion grades as a **floor**.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PinReclaim {
    /// Pins staged for release — one `unmap`, one `free`, one `munmap` each.
    pub released: usize,
    /// Pins refused as [`PinReleaseVerdict::RefusedNoHostVas`].
    pub refused_no_host_vas: usize,
    /// ★★★ Address-table rows **skipped** because their host object was already staged by
    /// its pin. **This is the double-free answer**, and it is counted rather than asserted:
    /// w291's merge writes the pin's `memory` into an exact-extent row *as well*, so a row
    /// walk that did not skip would `free` the same handle a second time — *"a DOUBLE FREE
    /// of a host object, strictly worse than the leak this closes"*
    /// (`kayfabe-fwd/src/lib.rs`, the merge's own doc).
    pub rows_deduped: usize,
}

impl PinReclaim {
    /// Fold another tally in.
    fn absorb(&mut self, o: PinReclaim) {
        self.released += o.released;
        self.refused_no_host_vas += o.refused_no_host_vas;
        self.rows_deduped += o.rows_deduped;
    }
}

/// ★★★ One live pin of **guest** pages into a host VAS — the record
/// [`Vas::guest_ram_pins`] holds.
///
/// Two names rather than one, and it is the same asymmetry `alloc_os_descriptor` warns
/// about: `memory` is the RM object that **pins** the pages for the GPU and is undone by
/// `free`; `mapped` is the isolate's own window onto them and is undone by `munmap`.
/// Releasing either alone leaves the other, so a record that carried one of them could
/// not describe a complete teardown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRamPin {
    /// The host GPU VA the pages were placed at — equal to the guest's own VA, or the
    /// verb that produced this refused (`RmError::PlacementRefused`).
    pub host_va: u64,
    /// The `OS_DESCRIPTOR` object RM built over the pages.
    pub memory: HostHandle,
    /// The isolate's mapping of the same pages.
    pub mapped: kayfabe_isolate::GuestRamMapped,
    /// How many bytes the grant named.
    pub len: u64,
}

impl Vas {
    fn new(gpu: GpuId, pdb: Pdb, origin: ResourceKey) -> Self {
        Vas {
            gpu,
            pdb,
            origin,
            // ★★★★★ CLAIMED, never bare. `AddressTable::owned_by` is what makes the owner's
            // per-VAS guarantee CHECKED rather than structural-by-convention: a caller handed
            // the wrong `Vas` is refused by name at both entrances instead of answering
            // confidently and wrongly. ⊘ `AddressTable::new()` here would be the silent
            // no-op, and `tests/tests/operand_join_is_per_vas.rs` asserts this call site.
            table: AddressTable::owned_by(pdb),
            host_vas: None,
            pt_pages: BTreeSet::new(),
            pt_meta: BTreeMap::new(),
            // Level 0 is a DECLARED fact: a PDB *is* its own root page. The shadow is
            // rooted here and nowhere else, which is what `ReachShadow::audit_root`
            // checks at every commit.
            reach: kayfabe_mmu::reach::ReachShadow::new(pdb.0 & !0xfff),
            blocks: BTreeMap::new(),
            guest_ram_pins: BTreeMap::new(),
            rpc_bound: BTreeSet::new(),
            promote_bound: BTreeSet::new(),
            promote_halves: BTreeMap::new(),
            sweep: PtSweepState::default(),
        }
    }
}

/// ★★★★★ **The C's `m2_gr_vas_dirty` / `m2_gr_pt_trunc`, as state rather than as globals**
/// (`C: nvkvm_gpu_emul.c:583-591`).
///
/// The whole-VAS sweep is HALF of the C's design; this is what makes the other half — the
/// dirty-driven **re**-sweep — expressible. Without it a sweep is one-shot, and a one-shot
/// sweep does not carry the safety argument that lets the sweep exist at all (see
/// [`kayfabe_mmu::reach::ReachShadow::witness_swept`]): the torn-read window is bounded only
/// because a page that was mid-update was, by definition, being written, and therefore comes
/// back as dirty and is re-swept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PtSweepState {
    /// How many whole-VAS sweeps have completed for this address space. `0` is itself a
    /// trigger — a `Vas` that has never been swept is the C's *"`chan_vas_n` grew"* case.
    pub sweeps: u64,
    /// ★★★ The last sweep hit its budget, so its picture of this address space is **partial**.
    ///
    /// The C carries the same bit (`m2_gr_pt_trunc`) for the same reason: truncation must
    /// force another sweep, or a mapping that fell off the end of the walk is missing until
    /// something unrelated happens to dirty a page.
    pub truncated: bool,
    /// A guest write landed on a page this address space's sweep is **tracking**, so the
    /// picture is stale and the next doorbell must re-sweep. Set at the plan, cleared when the
    /// sweep it armed has committed.
    pub dirty: bool,
    /// Pages the last sweep visited — the size of the walk, for the budget to be re-derived
    /// from a measurement instead of from the C's constant.
    pub last_pages: usize,
}

/// One guest channel — THE exec boundary (vChid, experiment E0).
pub struct Channel {
    /// Core-assigned per-proc slot.
    pub id: ChanId,
    /// The channel node in the RM graph.
    pub key: ResourceKey,
    /// ★ MG-4: the GPU target this channel lives on (graph-derived from its `Device`
    /// ancestor). `VChid` is a per-GPU runlist index, so the exec-plane routing map is
    /// keyed `(GpuId, VChid)` and this tag names which GPU the doorbell demuxes on.
    /// Always a RESOLVED target by construction: a channel whose target has not
    /// resolved yet is never materialized (deferred in `Gpu::refresh`, matching the
    /// `Vas` pattern) — never tagged with a default-GPU0 guess.
    pub gpu: GpuId,
    /// The exec-plane identity the doorbell demuxes on (unique only WITHIN [`Self::gpu`]).
    pub vchid: VChid,
    /// ★★★★★ **What this channel IS to the guest** — emulated (the guest's privileged
    /// kernel drives it) or passthrough (unprivileged guest userspace does). The owner's
    /// 2026-08-11 split; see [`crate::channel_kind`] for the whole model, the naming
    /// collisions it had to avoid, and what it refutes about being "two axes".
    ///
    /// # ⊘ DECLARED here, never re-derived by a consumer — and the cost of the
    /// re-derivation is on the record
    ///
    /// Synced from [`crate::project::ProcBoundary::channel_kind`], the one derivation,
    /// on the same pass as every other declared fact below. Before it existed, the axis
    /// was reachable **only** as `proc != Gpu::SYSTEM_PROC` recomputed at whichever
    /// consumer needed it, and its absence from
    /// `kayfabe_qemu_raw::shim::forwarding_plane_owns_ce` cost **12 boots** of
    /// `RmInitAdapter failed! (0x25:0x65:1249)` before `6fcedac`.
    ///
    /// ⚠ **It is invariant for this channel's life in this `Proc`, and that is
    /// structural rather than lucky**: a `Proc` *is* one [`crate::project::ProcBoundary`]
    /// (`sync_proc_to_boundary` copies the boundary's `anchor` onto it on the same
    /// pass), and a component cannot migrate between the reserved system anchor and a
    /// user anchor — `RmGraph::apply` refuses `RESERVED_CLIENT` as guest input. It is
    /// nonetheless re-assigned on every refresh with the other declared facts, for
    /// `vas_origin`'s stated reason: a field refreshed on a different pass from the
    /// resolution that produced it is how two projections of one fact come to disagree.
    pub kind: crate::channel_kind::GuestChannelKind,
    /// The PDB of the VAS this channel is declared against (None = GSP-managed
    /// with no declared VAS — system-routed). Keyed under [`Self::gpu`] in the Vas
    /// map. Named for what it IS — a [`Pdb`] — matching the projection's
    /// [`crate::project::ChannelFacts::vas_pdb`] (one concept, one name).
    pub vas_pdb: Option<Pdb>,
    /// ★★★ **The VASpace RESOURCE this channel resolved to** — graph-synced from
    /// [`crate::project::ChannelFacts::vas_origin`], the answer
    /// `project::resolve_channel_vas` already computed through the declared precedence
    /// (own `hVASpace` → CtxShare's → parent TSG's).
    ///
    /// # ⊘ Why this is carried rather than re-derived by whoever needs it
    ///
    /// `[measured 2026-08-09, boot `msr2_319d29a`]` — because a second derivation
    /// **disagreed with this one, and lost the channel `cuInit` walls on**.
    /// `kayfabe_rt::SharedDevice::ce_channel_facts` reported the VA space as
    /// `node.facts.h_vaspace`, i.e. the channel's OWN declared handle, and the boot printed
    ///
    /// ```text
    /// first doorbell refusal [FwdFault::IsolateRetired] … | c=0xc1d0000a
    ///     vas=NONE-DECLARED ring=0x121010000
    /// ```
    ///
    /// A UVM channel declares no `hVASpace` of its own — it inherits it through its
    /// CtxShare/TSG — so that field is `None` while [`Self::vas_pdb`], derived from the
    /// **resolved** node, is `Some`. The two projections of one fact disagreed, the
    /// ring-reading path took the weaker one, and every UVM doorbell fell through to a plane
    /// that reads no ring at all.
    ///
    /// ⚠ A [`ResourceKey`] and not a handle, for §12.41's reason: a channel legitimately
    /// binds its VA space through a `DUP_OBJECT` alias, and the alias may resolve to a
    /// dup-kept ghost whose origin handle the guest has since re-allocated.
    pub vas_origin: Option<ResourceKey>,
    /// ★★★★ **§16.28 — the `hVASpace` that names this channel's parent DEVICE's default
    /// address space**, graph-synced from
    /// [`crate::project::ChannelFacts::vas_device_default`] on the same pass as
    /// [`Self::vas_origin`], because it is the fourth answer of the one resolution that
    /// produced the other three.
    ///
    /// ⚠ A **name**, not a resource: `Some` here with [`Self::vas_origin`] `None` and
    /// [`Self::vas_pdb`] `None` is the normal, correct shape — RM freed the handle right
    /// after publishing the address space's root under it. See that projection field for
    /// what the name is good for and what it deliberately is not.
    pub vas_device_default: Option<HObject>,
    /// ★★★★ **§16.25 — which route produced [`Self::vas_origin`], and what the routes that
    /// ran actually hit.** Graph-synced from [`crate::project::ChannelFacts::vas_route`],
    /// refreshed on the same pass as [`Self::vas_pdb`] and [`Self::vas_origin`] for the
    /// reason their own comment gives: they are one resolution, and refreshing a report on
    /// a different pass from the decision it describes recreates the disagreement the
    /// carried `vas_origin` exists to end.
    ///
    /// ⊘ **Report-only.** Nothing branches on it. It is here so that a `NoVas` refusal can
    /// name which of the three declared-fact routes was tried — and so that the nine
    /// **served** channels of boot `s23_10a769c_cup2` can be compared against the fifteen
    /// refused ones on the same field. See [`crate::project::VasRoutes`].
    pub vas_route: crate::project::VasRoutes,
    /// ★ The fine [`EngineKind`] of this channel's context (`execution_plane.md`
    /// §2.2 "what the core tracks"): graph-synced from the projection — the channel
    /// class's declared kind, refined by the engine object allocated on it. NVENC
    /// vs GR-compute is distinguishable HERE, so routing and completion-arm
    /// selection key on the channel, not just on a parse.
    pub engine: EngineKind,
    /// Host channel object, once materialized by the fwd plane.
    pub host_channel: Option<HostHandle>,
    /// Host work-submit token, once materialized.
    pub host_token: Option<u64>,
    /// ★★★ **E5 — the engine method state this channel's engine holds between runs**
    /// (`execution_plane_increments.md` §8.2.1).
    ///
    /// It lives HERE, on the channel, because that is where the hardware keeps it: a
    /// copy-engine operation is assembled out of several method runs into the engine's
    /// own registers, and `LAUNCH_DMA` fires what has accumulated. `kayfabe-fwd` hands it
    /// to `PushbufferAbi::decode_run` and reads no field of it.
    ///
    /// ⊘ **This is the one guest-driven state in `Channel`, and it is bounded
    /// structurally rather than by a check** — a fixed
    /// `kayfabe_arch::SUBCHANNELS × kayfabe_arch::METHOD_SLOTS` array, no allocation, no
    /// key the guest supplies. It resets where the engine resets: per-subchannel on
    /// `SET_OBJECT`, and wholesale when this channel dies, because the value dies with it.
    pub method_state: kayfabe_arch::MethodState,
    /// Host engine objects forwarded on this channel, keyed by the guest's declared
    /// engine-object class — the Case-1 forward's **idempotency table**
    /// (`execution_plane.md` §2.2: "the object's Case-1 alloc has been forwarded, so
    /// re-sends are idempotent"). A replayed alloc resolves HERE and never re-allocs
    /// a duplicate host object (the same retried-RPC discipline as the graph's
    /// alloc/DUP replay).
    pub host_engine_objects: BTreeMap<ClassId, HostHandle>,
    /// ★★★ Where this channel asked to be told it died — graph-synced from
    /// [`crate::project::ChannelFacts::error_notifier`].
    ///
    /// `None` is the reason a fault on this channel is **escalated instead of emitted**:
    /// see `crate::fault::NotifierGap`. Kept on the runtime channel, beside `vchid` and
    /// `vas_pdb`, because the emitter runs off a refused doorbell and the channel is the
    /// only thing it holds.
    pub error_notifier: Option<ErrorNotifier>,
}

/// Per-proc execution plane. Nothing scalar, nothing one-shot (the C's
/// `m2_gr_*`/`doorbell_setup` cracks, ⚠4): every channel's scheduling state is
/// its own, per proc.
#[derive(Debug, Default)]
pub struct ExecPlane {
    /// ★★★ Channels the **guest** has asked to be scheduled —
    /// `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` (`0xa06f0103`) with `bEnable = NV_TRUE`, task
    /// #177.
    ///
    /// # Why this is a second set and not a flag on [`Self::scheduled`]
    ///
    /// The two sets answer different questions and are written by different actors:
    ///
    /// | set | written by | means |
    /// |---|---|---|
    /// | `requested` | the **guest's control**, through [`Gpu::schedule_channel`] | "the guest declared this channel runnable" |
    /// | `scheduled` | `kayfabe_fwd::commit_doorbell` | "a host runlist submit has actually happened for it" |
    ///
    /// Collapsing them would make the control's answer unfalsifiable in the exact way
    /// `refusal_invisible_in_the_ledger` describes: if the guest's `NV_OK` and the host's
    /// completed act were one bit, no test could distinguish *"we recorded an intent"*
    /// from *"a host GPU accepted it"*, and the first would read as the second forever.
    ///
    /// ★★ It is `requested` that **gates** submission (`kayfabe_fwd::plan_doorbell`), and
    /// that is what makes serving `0xa06f0103` a performed transition rather than a word:
    /// before the control a doorbell on this channel is refused by name
    /// (`FwdFault::NotScheduled`), after it the doorbell proceeds. ⊘ Break the gate and
    /// the control is once again a fabricated promise — the mutation is in
    /// `scripts/bite_gpfifo_schedule.py`.
    pub requested: BTreeSet<ChanId>,
    /// Channels whose host TSG/channel has been made runnable.
    pub scheduled: BTreeSet<ChanId>,
    /// ★★★ Which engine the guest has **bound** each channel to —
    /// `NVA06F_CTRL_CMD_BIND` (`0xa06f0104`), E9/§13.6.
    ///
    /// The value is in **RM engine space**, not the wire's `NV2080_ENGINE_TYPE` space:
    /// the policy converts with `kayfabe_abi::submit::nv2080_to_rm_engine_type` *before*
    /// routing here, because the two spaces collide above `0x12` (raw `0x13` is `NVDEC0`
    /// in one and `COPY10` in the other) and a raw value stored here would poison every
    /// later comparison against the RM-space engine tables.
    ///
    /// ⊘ This map records the guest's declaration; it does not assert a host runlist
    /// assignment happened — the same declared-versus-performed split as
    /// [`Self::requested`] vs [`Self::scheduled`], for the same
    /// `refusal_invisible_in_the_ledger` reason.
    pub bound: BTreeMap<ChanId, u32>,
    /// ★★★ How many GPFIFO entries of each channel's ring have already been **forwarded**
    /// — the doorbell path's resume point (`kayfabe_fwd::read_gpfifo_ring`).
    ///
    /// # ⊘ Why a cursor is not optional here
    ///
    /// A GPFIFO ring is *append-and-ring*: the guest writes entries and rings once per
    /// batch, and every entry it ever wrote is still sitting in the ring afterwards. A
    /// doorbell path that forwarded everything it could read would re-issue every earlier
    /// copy on every later doorbell — bytes moved twice, on a real engine, with no error
    /// anywhere. That is `#13`'s `CE-DROP` inverted, and it is silent in exactly the same
    /// way.
    ///
    /// # ⚠ Advanced on SUCCESS only, and the reason is measured
    ///
    /// `kayfabe_rt::ceutils::run_submission` takes its cursor **by value** and hands the
    /// advanced one back only in its success value, because *"a cursor advanced through a
    /// refusal would turn one loud failure into a silently dropped copy"* — and
    /// `[measured 2026-08-08, boot run_p2_c89899a]` the guest's own
    /// `channelWaitForFinishPayload` retries once before failing, so the entry it could not
    /// run is re-read rather than skipped. This cursor obeys the same rule for the same
    /// reason.
    ///
    /// ⊘ **`[NOT MEASURED]` — wrap-around.** The count is monotonic and the ring is
    /// circular; nothing here handles a guest that fills its ring and wraps to index 0.
    /// No boot has reached a second batch on this path, so the shape a wrap takes is
    /// unobserved, and inventing one would be a guess with a cursor attached to it.
    pub forwarded: BTreeMap<ChanId, u32>,
}

/// Where a `0xa06f0103` is going — [`route_schedule_channel`]'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleRoute {
    /// The proc that owns the channel.
    pub proc: ProcId,
    /// Its channel slot.
    pub chan: ChanId,
}

/// ★ **ROUTE (rank 0).** Resolve `(client, object)` to the proc and channel slot the
/// guest's `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` names. A pure read of the spine's
/// projection-derived indices; it touches no [`Proc`].
///
/// Two forward hops, no fallback:
///
/// 1. `(client, object)` → the **live** resource at that handle, then immediately its
///    stable [`ResourceKey`] (handle values are recyclable; identities are not).
/// 2. identity → `(ProcId, ChanId)` ([`Spine::by_chan`], built from the projection beside
///    `by_vchid` and never accreted).
///
/// ⚠ It deliberately does **not** route through [`Spine::ctx_vas`] the way
/// [`crate::promote::route_promote_ctx`] does. The first channel this control is ever
/// asked about is RM's global CeUtils scrubber, which is allocated with
/// `hVASpace = NV01_NULL_OBJECT` on purpose (`ogkm-580:
/// src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:86-93`); a VAS-keyed route would
/// refuse it for a reason that has nothing to do with scheduling, and the refusal would
/// have looked like a correct one.
///
/// # Errors
/// [`ScheduleFault`], by variant.
pub fn route_schedule_channel(
    spine: &Spine,
    client: HClient,
    object: HObject,
) -> Result<ScheduleRoute, ScheduleFault> {
    let node = spine
        .rmgraph
        .node(NodeKey::new(client, object))
        .ok_or(ScheduleFault::UnknownChannel { client, object })?;
    if !matches!(node.kind, kayfabe_arch::ObjectKind::Channel { .. }) {
        return Err(ScheduleFault::NotAChannel { client, object });
    }
    let &(proc, chan) = spine
        .by_chan
        .get(&node.id())
        .ok_or(ScheduleFault::ChannelNotMaterialized { client, object })?;
    Ok(ScheduleRoute { proc, chan })
}

/// ★ **ACT (rank 1, one proc).** Record or withdraw the guest's scheduling declaration.
#[must_use]
pub fn apply_schedule_channel(proc: &mut Proc, route: &ScheduleRoute, enable: bool) -> ScheduleAck {
    let changed = if enable {
        proc.exec.requested.insert(route.chan)
    } else {
        proc.exec.requested.remove(&route.chan)
    };
    ScheduleAck {
        proc: route.proc,
        chan: route.chan,
        enabled: enable,
        changed,
    }
}

/// What [`Gpu::schedule_channel`] performed — #177.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleAck {
    /// The proc that owns the channel.
    pub proc: ProcId,
    /// The channel slot the intent was recorded against.
    pub chan: ChanId,
    /// The `bEnable` that was asked for.
    pub enabled: bool,
    /// Whether the set actually moved. ★ `false` is a **legitimate** answer and still
    /// `NV_OK`: RM re-schedules an already-scheduled channel on several paths, and this
    /// control is idempotent by construction. It is reported rather than discarded so a
    /// boot's census can distinguish "the guest asked twice" from "the guest asked once".
    pub changed: bool,
}

/// Why [`Gpu::schedule_channel`] refused — #177.
///
/// ⊘ Every variant means *"this port examined the request and declined"*, which is why
/// they are answered with `kayfabe_abi::submit::GPFIFO_SCHEDULE_REFUSED_STATUS`
/// (`NV_ERR_INVALID_STATE`) and never with `NV_ERR_NOT_SUPPORTED` — the latter is the
/// FSM's signature for *"nobody claimed this command"*, and reusing it would make the two
/// indistinguishable in the guest's own dmesg, which is the only place a reader ever sees
/// this control fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleFault {
    /// No live resource at `(client, object)`.
    UnknownChannel {
        /// The client namespace asked in.
        client: HClient,
        /// The handle asked about.
        object: HObject,
    },
    /// The handle resolves, but not to a channel.
    NotAChannel {
        /// The client namespace asked in.
        client: HClient,
        /// The handle asked about.
        object: HObject,
    },
    /// A live channel resource that no proc has a slot for — it was declared but the
    /// projection has not (or can no longer) place it. ★ Distinct from
    /// [`Self::UnknownChannel`] on purpose: one means the guest named something that does
    /// not exist, the other means **we** cannot place something that does, and only the
    /// second is our defect.
    ChannelNotMaterialized {
        /// The client namespace asked in.
        client: HClient,
        /// The handle asked about.
        object: HObject,
    },
}

impl core::fmt::Display for ScheduleFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ScheduleFault::UnknownChannel { client, object } => {
                write!(f, "no live resource at ({client:?}, {object:?})")
            }
            ScheduleFault::NotAChannel { client, object } => {
                write!(f, "({client:?}, {object:?}) is not a channel")
            }
            ScheduleFault::ChannelNotMaterialized { client, object } => write!(
                f,
                "channel ({client:?}, {object:?}) is declared but has no slot in any proc"
            ),
        }
    }
}

impl core::error::Error for ScheduleFault {}

/// Where a `0xa06c0101` is going — [`route_schedule_group`]'s answer. §16.56.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleGroupRoute {
    /// The proc that owns every channel of the group.
    pub proc: ProcId,
    /// The group's member channels, in the projection's own order.
    pub chans: Vec<ChanId>,
    /// ★ Members that resolved as live channel **resources** but have no slot in any
    /// proc. Reported rather than discarded: a group that is *partly* placed is a
    /// different finding from one that is wholly placed, and answering `NV_OK` while
    /// silently dropping members is the shape [`ExecPlane::requested`]'s doc forbids.
    pub unmaterialized: usize,
}

/// ★★★★ **§16.56 — ROUTE (rank 0) for the TSG form of GPFIFO_SCHEDULE (`0xa06c0101`).**
///
/// Resolve `(client, object)` to the proc and the **set** of channel slots the guest's
/// `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` names.
///
/// # Why a set, and why that is the driver's own semantics rather than our guess
///
/// The TSG form is not a different operation from the channel form — it is the same
/// operation quantified over the group's member list, and RM's own kernel half says so by
/// doing exactly that before it RPCs to us: `kchangrpapiCtrlCmdGpFifoSchedule_IMPL` walks
/// `pKernelChannelGroup->pChanList` twice — once asserting every member is schedulable
/// (`NV_ERR_INVALID_STATE` if not) and once forcing every member onto one runlist — and
/// only then hands the command to the GSP
/// (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel_group_api.c:1102-1170`). Its
/// params are a **typedef** of the channel form's, not a look-alike:
/// `typedef NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS NVA06C_CTRL_GPFIFO_SCHEDULE_PARAMS`
/// (`ogkm-580: ctrl/ctrla06c.h:101`), and the guest's vGPU RPC dispatcher sends both ids
/// down one arm (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:4557-4559`).
///
/// ⇒ Fanning the guest's one control out over the group's members is a **translation of
/// the guest's intent**, in the sense `mode2_forwarding_model.md` requires, not an
/// invention.
///
/// # The hops
///
/// 1. `(client, object)` → the live resource, which must be an [`ObjectKind::Tsg`].
/// 2. the group's **members**: every live channel resource whose declared parent is this
///    group's origin handle, in this group's origin namespace. ⊘ Read from the *graph*,
///    never from a list we accrete — the graph is where a `FREE` of a member is already
///    reflected, and a cached member list is the stale-set bug class this port has paid
///    for twice (`ExecPlane`'s "nothing scalar, nothing one-shot").
/// 3. each member's identity → `(ProcId, ChanId)` via [`Spine::by_chan`], exactly the hop
///    [`route_schedule_channel`] uses and for its reasons.
///
/// ⚠ It does **not** route through [`Spine::ctx_vas`], for the same reason
/// [`route_schedule_channel`] does not — read that function's docs.
///
/// # Errors
/// [`ScheduleGroupFault`], by variant.
pub fn route_schedule_group(
    spine: &Spine,
    client: HClient,
    object: HObject,
) -> Result<ScheduleGroupRoute, ScheduleGroupFault> {
    let group = spine
        .rmgraph
        .node(NodeKey::new(client, object))
        .ok_or(ScheduleGroupFault::UnknownGroup { client, object })?;
    if group.kind != kayfabe_arch::ObjectKind::Tsg {
        return Err(ScheduleGroupFault::NotAGroup { client, object });
    }
    // ★ Members are matched against the group's ORIGIN key, not against the `(client,
    // object)` the guest asked in. A dup alias resolves to the same resource, and the
    // members' declared `parent` handle lives in the allocating namespace — pairing the
    // asked-in client with a member's parent handle would silently find nothing for a
    // group named through an alias, which reads as "the group is empty".
    let mut chans: Vec<ChanId> = Vec::new();
    let mut procs: BTreeSet<ProcId> = BTreeSet::new();
    let mut unmaterialized = 0usize;
    let mut members = 0usize;
    for n in spine.rmgraph.nodes() {
        if !matches!(n.kind, kayfabe_arch::ObjectKind::Channel { .. })
            || n.key.client != group.key.client
            || n.parent != group.key.handle
        {
            continue;
        }
        members += 1;
        match spine.by_chan.get(&n.id()) {
            Some(&(pid, cid)) => {
                procs.insert(pid);
                chans.push(cid);
            }
            None => unmaterialized += 1,
        }
    }
    if members == 0 {
        return Err(ScheduleGroupFault::GroupHasNoChannels { client, object });
    }
    if chans.is_empty() {
        return Err(ScheduleGroupFault::NoMemberMaterialized {
            client,
            object,
            members,
        });
    }
    if procs.len() != 1 {
        return Err(ScheduleGroupFault::GroupSpansProcs {
            client,
            object,
            procs: procs.len(),
        });
    }
    let proc = *procs.iter().next().expect("exactly one proc");
    Ok(ScheduleGroupRoute {
        proc,
        chans,
        unmaterialized,
    })
}

/// ★★★★★ **w303 — how much of a channel group is REACHABLE BY THE HOST GPU.**
///
/// The one fact `NVA06C_CTRL_CMD_PREEMPT` turns on: *is there anything a preemption could
/// preempt?* Work reaches a real GA106 through exactly one door — a **host channel** born
/// for a guest channel ([`Channel::host_channel`]) and rung through
/// `kayfabe_isolate::RmBackend::ring_doorbell`. A group with **no** member holding a host
/// twin has never submitted a byte to hardware, so it is idle **by construction**, and
/// "the preempt completed" is a true statement about it rather than a forged one.
///
/// ⊘ **The converse is deliberately NOT claimed.** `with_host_twin > 0` does not mean work
/// *is* executing — only that we cannot say it is not. That asymmetry is the whole point:
/// one side is provable from state we hold, the other needs a cursor read we do not do on
/// this path, and conflating them is how an ack becomes a lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupHostTwins {
    /// The proc that owns the group.
    pub proc: ProcId,
    /// Member channels that resolved to a slot in that proc.
    pub materialized: usize,
    /// Members that resolved as channel resources but hold no slot
    /// ([`ScheduleGroupRoute::unmaterialized`]). They can hold no host twin either, but
    /// they are reported separately so "the group is empty" and "the group is not placed"
    /// stay distinguishable.
    pub unmaterialized: usize,
    /// ★ How many materialized members have a **live host channel**. This is the number
    /// the answer turns on.
    pub with_host_twin: usize,
}

/// ★ **CENSUS (rank 1, one proc).** Count [`ScheduleGroupRoute`]'s members that hold a
/// host twin. Pure read — no verb, no mutation — so it is legal wherever
/// [`apply_schedule_group`] is.
#[must_use]
pub fn census_group_host_twins(proc: &Proc, route: &ScheduleGroupRoute) -> GroupHostTwins {
    let mut materialized = 0usize;
    let mut with_host_twin = 0usize;
    for &cid in &route.chans {
        let Some(chan) = proc.channels.get(&cid) else {
            // ⊘ Routed but absent from the proc: counted as neither, because it is a
            // *staleness* fact and not a host-reachability one. It cannot hold a twin.
            continue;
        };
        materialized += 1;
        if chan.host_channel.is_some() {
            with_host_twin += 1;
        }
    }
    GroupHostTwins {
        proc: route.proc,
        materialized,
        unmaterialized: route.unmaterialized,
        with_host_twin,
    }
}

/// ★ **ACT (rank 1, one proc).** Record or withdraw the guest's scheduling declaration
/// for every member of the group. §16.56.
#[must_use]
pub fn apply_schedule_group(
    proc: &mut Proc,
    route: &ScheduleGroupRoute,
    enable: bool,
) -> ScheduleGroupAck {
    let mut changed = 0usize;
    for &cid in &route.chans {
        let moved = if enable {
            proc.exec.requested.insert(cid)
        } else {
            proc.exec.requested.remove(&cid)
        };
        if moved {
            changed += 1;
        }
    }
    ScheduleGroupAck {
        proc: route.proc,
        members: route.chans.len(),
        changed,
        unmaterialized: route.unmaterialized,
        enabled: enable,
    }
}

/// What [`Gpu::schedule_group`] performed — §16.56.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleGroupAck {
    /// The proc that owns the group.
    pub proc: ProcId,
    /// How many member channels the intent was recorded against.
    pub members: usize,
    /// How many of them actually moved. ★ `0` is legitimate and still `NV_OK` — this
    /// control is idempotent and RM re-issues it — but it is reported rather than
    /// discarded so a boot's census can tell "the guest asked twice" from "the guest
    /// asked once", exactly as [`ScheduleAck::changed`] does for the channel form.
    pub changed: usize,
    /// Members that had no slot ([`ScheduleGroupRoute::unmaterialized`]).
    pub unmaterialized: usize,
    /// The `bEnable` that was asked for.
    pub enabled: bool,
}

/// What [`Gpu::set_ctxsw_preemption_mode`] verified — §16.59.
///
/// ⊘ **Nothing was programmed, and that is the claim, not a caveat.** This control asks for
/// a postcondition (*"this context switches at X"*). Wait-for-idle is the postcondition this
/// port's execution plane is unconditionally in — it has no preemption machinery of any
/// kind, so *"switch at idle"* is not a mode it fails to program but the only mode it has.
/// ⇒ The honest service of the request is to **verify** it and say so, which is strictly
/// more than the C artifact did (it echoed `NV_OK` to a `COMPUTE_CILP` request —
/// `docs/design/execution_plane_increments.md` §16.59).
///
/// ⚠ **The mode words are NOT classified here**, and the crate boundary is why: which modes
/// a request asks for is a pure ABI question over a wire struct, and `kayfabe-core` does not
/// depend on `kayfabe-abi` (deliberately). The classifier is
/// `kayfabe_abi::submit::CtxswPreemptionRequest::asks_for`, and refusing a preemption mode
/// this port does not implement is `ObjectPolicy::respond_ctxsw_preemption_mode`'s job —
/// exactly as the `NV2080_ENGINE_TYPE` → `RM_ENGINE_TYPE` check for `NVA06F_CTRL_CMD_BIND`
/// is the policy's and not [`Gpu::bind_channel`]'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtxswPreemptionAck {
    /// The proc that owns the context named by `hChannel`.
    pub proc: ProcId,
    /// Whether `hChannel` named a TSG (`true`) or a bare channel (`false`). Reported
    /// because the field's **name** says channel and
    /// `[measured 2026-08-10, boot s46_1a9e93c_abi35 record 331]` the `cuCtxCreate` path
    /// puts a group handle in it.
    pub was_group: bool,
}

/// Why [`Gpu::set_ctxsw_preemption_mode`] refused — §16.59.
///
/// ★★★ Answered with `kayfabe_abi::submit::CTXSW_PREEMPTION_REFUSED_STATUS`
/// (`NV_ERR_NOT_SUPPORTED`), and this is the one control on the served list where that is
/// **the header's own status for this exact condition** rather than a borrowed one:
/// *"A value of `NV_ERR_NOT_SUPPORTED` is returned if the target channel does not support
/// preemption context switch mode changes"* (`ogkm-580: ctrl2080gr.h:791-795`). Read the
/// constant's docs for why the standing "never reuse `0x56`" rule is not being bent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtxswPreemptionFault {
    /// No live resource at `(client, h_channel)`. ⊘ Resolved in the **asked-in client**,
    /// not in the subdevice's namespace: the guest names the context by a handle of its
    /// own, and a port that answered `NV_OK` without finding it would be promising
    /// something about an object it cannot see.
    UnknownContext {
        /// The client namespace asked in.
        client: HClient,
        /// The `hChannel` field's value.
        h_channel: HObject,
    },
    /// `(client, h_channel)` resolves to something that is neither a TSG nor a channel.
    NotAContext {
        /// The client namespace asked in.
        client: HClient,
        /// The `hChannel` field's value.
        h_channel: HObject,
    },
    /// The context resolved but **we** have placed no channel of it in any proc, so there
    /// is no execution plane whose behaviour the answer would be about.
    NoMemberMaterialized {
        /// The client namespace asked in.
        client: HClient,
        /// The `hChannel` field's value.
        h_channel: HObject,
    },
    /// ★★★★ The request asks for a preemption mode that is **not wait-for-idle** — GfxP or
    /// GfxP-pool on the graphics side, CTA or CILP on the compute side.
    ///
    /// ⊘⊘ This is the variant the C artifact does not have, and the reason this arm is
    /// classified at all. `[measured 2026-08-10, cap3 #453716]` the C's guest asked for
    /// `cilpPreemptMode = 2` (`COMPUTE_CILP`) and the C answered `NV_OK` with the bytes
    /// echoed; `[measured 2026-08-10, boot s46 record 331]` **our** guest asks for `0`
    /// (`COMPUTE_WFI`). The two differ in exactly the word that decides whether the ack is
    /// true, so an unconditional echo — the C's behaviour, and what this rung was briefed to
    /// port — is honest on our measured payload and a lie on the C's.
    ///
    /// ⚠ Whether a spinning compute kernel can be preempted **at all** on GA10x consumer is
    /// `[unknown]` from the open tree (`compute_limiting_and_priority.md` §3.3: no `_IMPL`
    /// body, no `bCilpSupported` symbol anywhere). ⇒ We cannot even inherit an answer here;
    /// refusing is the only position that is not a guess.
    PreemptionNotImplemented {
        /// Which engine's mode word asked for it — `"gfxpPreemptMode"` or
        /// `"cilpPreemptMode"`, the C field name.
        field: &'static str,
        /// The mode value as it arrived.
        mode: u32,
    },
}

impl core::fmt::Display for CtxswPreemptionFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CtxswPreemptionFault::UnknownContext { client, h_channel } => {
                write!(
                    f,
                    "no live resource at ({client:?}, hChannel {h_channel:?})"
                )
            }
            CtxswPreemptionFault::NotAContext { client, h_channel } => write!(
                f,
                "({client:?}, hChannel {h_channel:?}) is neither a channel group nor a channel"
            ),
            CtxswPreemptionFault::NoMemberMaterialized { client, h_channel } => write!(
                f,
                "({client:?}, hChannel {h_channel:?}) has no channel with a slot in any proc"
            ),
            CtxswPreemptionFault::PreemptionNotImplemented { field, mode } => write!(
                f,
                "{field}={mode} asks for a preemption mode other than wait-for-idle; this \
                 port never preempts a context"
            ),
        }
    }
}

impl core::error::Error for CtxswPreemptionFault {}

/// ★★★★ **§16.59 — ROUTE (rank 0) for `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE`.**
///
/// Resolve the request's `hChannel` — which `[measured]` carries a **TSG** handle on the
/// `cuCtxCreate` path — to the proc whose execution plane the answer will be about.
///
/// ⊘ The caller must already have established that the request asks for wait-for-idle;
/// see [`CtxswPreemptionAck`] for why that half is not here.
///
/// # Errors
/// [`CtxswPreemptionFault`], by variant.
pub fn route_ctxsw_preemption(
    spine: &Spine,
    client: HClient,
    h_channel: HObject,
) -> Result<CtxswPreemptionAck, CtxswPreemptionFault> {
    let node = spine
        .rmgraph
        .node(NodeKey::new(client, h_channel))
        .ok_or(CtxswPreemptionFault::UnknownContext { client, h_channel })?;
    // ⊘ Both kinds are accepted because RM's own field is polymorphic in practice, not
    // because we could not tell them apart: the group form is what `cuCtxCreate` sends and
    // the channel form is what the header's prose describes.
    let (proc, was_group) = match node.kind {
        kayfabe_arch::ObjectKind::Tsg => {
            let route = route_schedule_group(spine, client, h_channel)
                .map_err(|_| CtxswPreemptionFault::NoMemberMaterialized { client, h_channel })?;
            (route.proc, true)
        }
        kayfabe_arch::ObjectKind::Channel { .. } => {
            let (pid, _cid) = *spine
                .by_chan
                .get(&node.id())
                .ok_or(CtxswPreemptionFault::NoMemberMaterialized { client, h_channel })?;
            (pid, false)
        }
        _ => return Err(CtxswPreemptionFault::NotAContext { client, h_channel }),
    };
    Ok(CtxswPreemptionAck { proc, was_group })
}

/// Why [`Gpu::schedule_group`] refused — §16.56.
///
/// ⊘ Every variant means *"this port examined the request and declined"*, answered with
/// `kayfabe_abi::submit::GPFIFO_SCHEDULE_REFUSED_STATUS` (`NV_ERR_INVALID_STATE`) and
/// never with `NV_ERR_NOT_SUPPORTED` — see [`ScheduleFault`] for the argument, which
/// applies here **doubly**: `0x56` on this exact command is the signature the port wore
/// for the whole of `s43`/`s44`, so reusing it would make a decided refusal
/// indistinguishable from the wall this increment exists to remove.
///
/// ★ `NV_ERR_INVALID_STATE` is in the command's own vocabulary rather than borrowed:
/// `kchangrpapiCtrlCmdGpFifoSchedule_IMPL` returns exactly it when a member is not
/// schedulable (`ogkm-580: kernel_channel_group_api.c:1106-1109`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleGroupFault {
    /// No live resource at `(client, object)`.
    UnknownGroup {
        /// The client namespace asked in.
        client: HClient,
        /// The handle asked about.
        object: HObject,
    },
    /// The handle resolves, but not to a channel group.
    NotAGroup {
        /// The client namespace asked in.
        client: HClient,
        /// The handle asked about.
        object: HObject,
    },
    /// A live TSG with **no channel members at all**. ⊘ Answered as a refusal rather
    /// than as a vacuous `NV_OK`: an `NV_OK` here would be a promise about an empty set,
    /// which is the unfalsifiable-ack shape this port refuses by policy.
    GroupHasNoChannels {
        /// The client namespace asked in.
        client: HClient,
        /// The handle asked about.
        object: HObject,
    },
    /// The group has members, and **we** cannot place any of them in a proc. ★ Distinct
    /// from [`Self::GroupHasNoChannels`] on purpose, and the distinction is the whole
    /// diagnostic: one means the guest scheduled an empty group, the other means our
    /// projection lost every channel the guest built — only the second is our defect.
    NoMemberMaterialized {
        /// The client namespace asked in.
        client: HClient,
        /// The handle asked about.
        object: HObject,
        /// How many live channel resources declared this group as their parent.
        members: usize,
    },
    /// The group's members resolved into **more than one proc**. Not reachable by a
    /// well-formed guest — a TSG's channels are allocated in one namespace under one
    /// anchor — and refused rather than partially applied, because "schedule some of the
    /// group" is not a state RM has and not one this port will invent.
    GroupSpansProcs {
        /// The client namespace asked in.
        client: HClient,
        /// The handle asked about.
        object: HObject,
        /// How many distinct procs the members landed in.
        procs: usize,
    },
}

impl core::fmt::Display for ScheduleGroupFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ScheduleGroupFault::UnknownGroup { client, object } => {
                write!(f, "no live resource at ({client:?}, {object:?})")
            }
            ScheduleGroupFault::NotAGroup { client, object } => {
                write!(f, "({client:?}, {object:?}) is not a channel group")
            }
            ScheduleGroupFault::GroupHasNoChannels { client, object } => write!(
                f,
                "channel group ({client:?}, {object:?}) has no member channels"
            ),
            ScheduleGroupFault::NoMemberMaterialized {
                client,
                object,
                members,
            } => write!(
                f,
                "channel group ({client:?}, {object:?}) has {members} member(s), none with a \
                 slot in any proc"
            ),
            ScheduleGroupFault::GroupSpansProcs {
                client,
                object,
                procs,
            } => write!(
                f,
                "channel group ({client:?}, {object:?}) spans {procs} procs"
            ),
        }
    }
}

impl core::error::Error for ScheduleGroupFault {}

/// Where a `0xa06f0104` is going — [`route_bind_channel`]'s answer. E9/§13.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindRoute {
    /// The proc that owns the channel.
    pub proc: ProcId,
    /// Its channel slot.
    pub chan: ChanId,
}

/// ★ **ROUTE (rank 0).** Resolve `(client, object)` to the proc and channel slot the
/// guest's `NVA06F_CTRL_CMD_BIND` names — [`route_schedule_channel`]'s three forward
/// hops, verbatim and for its reasons (read that function's docs; they are the argument).
///
/// ⚠ Including the non-reason: it does **not** route through [`Spine::ctx_vas`], because
/// the first channel this control is ever asked about — the global CeUtils channel RM
/// allocates at `mem_mgr.c:4155` — is on the same `hVASpace = NV01_NULL_OBJECT` footing
/// as the scrubber, and a VAS-keyed route would refuse it for a reason that has nothing
/// to do with binding.
///
/// ⊘ And it does **not** ask whether the engine is one this device advertised. That check
/// belongs to whoever holds the advertised set (`ChipProfile::engines`), which the core
/// deliberately does not (`execution_plane_increments.md` §13.6): the policy checks the
/// engine *before* routing, so a fault from here is always about the **channel**.
///
/// # Errors
/// [`BindFault`], by variant.
pub fn route_bind_channel(
    spine: &Spine,
    client: HClient,
    object: HObject,
) -> Result<BindRoute, BindFault> {
    let node = spine
        .rmgraph
        .node(NodeKey::new(client, object))
        .ok_or(BindFault::UnknownChannel { client, object })?;
    if !matches!(node.kind, kayfabe_arch::ObjectKind::Channel { .. }) {
        return Err(BindFault::NotAChannel { client, object });
    }
    let &(proc, chan) = spine
        .by_chan
        .get(&node.id())
        .ok_or(BindFault::ChannelNotMaterialized { client, object })?;
    Ok(BindRoute { proc, chan })
}

/// ★ **ACT (rank 1, one proc).** Record which engine the guest bound the channel to.
///
/// `rm_engine_type` is in **RM engine space** — see [`ExecPlane::bound`] for why the
/// conversion must already have happened.
#[must_use]
pub fn apply_bind_channel(proc: &mut Proc, route: &BindRoute, rm_engine_type: u32) -> BindAck {
    let previous = proc.exec.bound.insert(route.chan, rm_engine_type);
    BindAck {
        proc: route.proc,
        chan: route.chan,
        rm_engine_type,
        changed: previous != Some(rm_engine_type),
    }
}

/// What [`Gpu::bind_channel`] performed — E9/§13.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindAck {
    /// The proc that owns the channel.
    pub proc: ProcId,
    /// The channel slot the binding was recorded against.
    pub chan: ChanId,
    /// The engine it was bound to, in **RM engine space** (already converted).
    pub rm_engine_type: u32,
    /// Whether the binding actually moved. ★ `false` is a **legitimate** answer and still
    /// `NV_OK` — re-binding a channel to the engine it is already bound to is idempotent —
    /// reported rather than discarded for [`ScheduleAck::changed`]'s census reason.
    pub changed: bool,
}

/// Why [`Gpu::bind_channel`] refused — E9/§13.6.
///
/// ⊘ Every variant means *"this port examined the request and could not route the
/// CHANNEL"*, which is why the policy answers them all with
/// `kayfabe_abi::submit::BIND_REFUSED_STATUS` (`NV_ERR_INVALID_STATE`). The **engine**
/// refusal is a different answer (`BIND_UNKNOWN_ENGINE_STATUS`, `0x57`) and is decided in
/// the policy, before routing, by whoever holds the advertised engine set — so it has no
/// variant here on purpose. And never `NV_ERR_NOT_SUPPORTED`: `0x56` is the FSM's
/// signature for *"nobody claimed this command"*, the exact number the bench guest
/// printed at `mem_utils.c:1969` while this control was unserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindFault {
    /// No live resource at `(client, object)`.
    UnknownChannel {
        /// The client namespace asked in.
        client: HClient,
        /// The handle asked about.
        object: HObject,
    },
    /// The handle resolves, but not to a channel.
    NotAChannel {
        /// The client namespace asked in.
        client: HClient,
        /// The handle asked about.
        object: HObject,
    },
    /// A live channel resource that no proc has a slot for — [`ScheduleFault`] draws the
    /// same distinction for the same reason: only this variant is **our** defect.
    ChannelNotMaterialized {
        /// The client namespace asked in.
        client: HClient,
        /// The handle asked about.
        object: HObject,
    },
}

impl core::fmt::Display for BindFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BindFault::UnknownChannel { client, object } => {
                write!(f, "no live resource at ({client:?}, {object:?})")
            }
            BindFault::NotAChannel { client, object } => {
                write!(f, "({client:?}, {object:?}) is not a channel")
            }
            BindFault::ChannelNotMaterialized { client, object } => write!(
                f,
                "channel ({client:?}, {object:?}) is declared but has no slot in any proc"
            ),
        }
    }
}

impl core::error::Error for BindFault {}

/// Per-proc poll bookkeeping (the C's `m2_poll_kick`/`m2_last_db_token`
/// singletons, made per-process — crack ⚠7).
#[derive(Debug, Default)]
pub struct PollState {
    /// Virtual time of the proc's last completion-poll RPC.
    pub last_poll: Option<Instant>,
    /// Last doorbell token observed from this proc (for poll-kick replay).
    pub last_token: Option<u64>,
}

/// ★★★ One **deferred isolate materialization**: which proc, which target
/// (`l1_concurrency.md` §3.3 R1).
///
/// The whole payload is a pair of ids and deliberately not a reference or a handle:
/// R5's own rule is *"IDs, never held references"*, because everything this names may be
/// gone by the time the spawn comes back and the install has to be able to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PendingSpawn {
    /// The proc whose isolate is to be materialized.
    pub proc: ProcId,
    /// The target GPU it is to be materialized for (MG-5: one isolate per pair).
    pub gpu: GpuId,
}

/// A batch of [`PendingSpawn`]s, latched under a lock and discharged without one.
///
/// `#[must_use]` for exactly the reason `Cancels` is: the whole point of returning the
/// work instead of doing it is that the caller runs it somewhere the invariant permits,
/// and a batch dropped at the call site is that discipline silently not happening. ⊘ It
/// is *not* an obligation whose loss corrupts state — see [`Spine::pending_spawns`] — but
/// a caller that means to drop it should have to say so.
#[must_use = "a latched spawn must be discharged with no lock held (R1), not dropped"]
#[derive(Debug, Default)]
pub struct PendingSpawns(Vec<PendingSpawn>);

impl PendingSpawns {
    /// Nothing to spawn.
    pub fn new() -> PendingSpawns {
        PendingSpawns::default()
    }

    /// True when there is no deferred spawn in this batch.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How many deferred spawns this batch carries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    fn push(&mut self, proc: ProcId, gpu: GpuId) {
        self.0.push(PendingSpawn { proc, gpu });
    }
}

impl IntoIterator for PendingSpawns {
    type Item = PendingSpawn;
    type IntoIter = std::vec::IntoIter<PendingSpawn>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// The per-process container — the unit of ownership for all four planes.
///
/// Also the unit of **parallelism** (concurrency contract, crate docs): the
/// per-proc entry points (`kayfabe-fwd`'s `publish_backing`, this type's methods)
/// take `&mut Proc`, so two vCPU threads holding disjoint `&mut` borrows out of
/// [`Gpu::procs`] mutate different procs simultaneously with no shared lock —
/// their arenas, host VASes, isolates, and completion queues are disjoint by
/// construction.
pub struct Proc {
    /// Derived identity (grouping label only — address ops key on [`Vas`],
    /// exec ops on [`Channel`]).
    pub id: ProcId,
    /// Deterministic component label (★★★ §12.42 — the smallest client DECLARATION).
    pub anchor: ProcAnchor,
    /// Client **declarations** in this proc's dup-connected component (★ §12.27:
    /// declared **user** clients only, joined by user↔user dups — except on `Gpu::system`,
    /// whose set is every declared **kernel** client, joined by rule rather than by dup).
    ///
    /// ★★★ §12.42 — [`ClientKey`], not `HClient`; see
    /// [`crate::project::ProcBoundary::clients`]. [`Self::client_values`] is the lossy
    /// by-value view.
    pub clients: BTreeSet<ClientKey>,
    /// ★★★ §12.39 Part B — the **identities** of those clients' namespaces
    /// ([`ClientId`], never reused), recorded at the same sync that set
    /// [`Self::clients`].
    ///
    /// The split of labelling from matching, stated once: **[`Self::clients`] LABELS a
    /// component and this MATCHES one.** An `hClient` is a value the guest recycles (see
    /// [`ClientId`]), so matching a boundary to a live `Proc` on values alone let a
    /// re-declared namespace **adopt the previous tenant's proc** — its isolate, its GPA
    /// arena, its host VAS, its `pending_release` queue. `project` already refuses to
    /// attribute the old declaration's *resources* to the new one; without this the
    /// runtime handed over the container anyway, which is the same breach one layer down.
    ///
    /// Two names for one namespace is a real cost (§12.39's D1 was rejected partly for
    /// it), so it is confined: nothing outside the match in [`Spine::plan_refresh`] and
    /// the condemnation key reads this field, and `ProcAnchor` deliberately stays an
    /// `HClient` because it is a deterministic *label* of a live component and never an
    /// identity.
    client_ids: BTreeSet<ClientId>,
    /// ★ The address plane: one [`Vas`] per declared **`(GpuId, Pdb)`** (MG-4). A proc
    /// holds several (several VASpaces, and — spanning GPUs — per target); address ops
    /// key on `(GpuId, Pdb)` because a `Pdb` is a per-GPU namespace (two GPUs legally
    /// present identical PDB values).
    pub vases: BTreeMap<(GpuId, Pdb), Vas>,
    /// The exec plane's channels.
    pub channels: BTreeMap<ChanId, Channel>,
    /// Channel node → slot (stable across graph re-derivations).
    pub chan_ids: BTreeMap<ResourceKey, ChanId>,
    /// Per-proc execution plane state.
    pub exec: ExecPlane,
    /// Per-proc completion queue (§4.3.2 — the starvation fix's per-proc half).
    pub completion: CompletionQueue,
    /// Per-proc mapped-fence completion arms (pattern **e**, `execution_plane.md`
    /// §1.2/§2.4 — the NVENC fence-not-event shape, `nvenc_101`). Distinct from the
    /// event-delivery plane by construction: a fired fence is read by the guest
    /// straight from the mapped page, it never rides the GSP queue.
    pub fences: FenceArms,
    /// ★ MG-5: this proc's per-**target** unprivileged host isolates — one sandboxed
    /// host worker per GPU this proc touches (`session == ProcId`, distinct namespace
    /// per GPU). A bug forwarding this proc's GPU0 traffic cannot reach its GPU1 host
    /// handles: the #14 blast-radius boundary lifted onto the GPU axis. Materialized
    /// lazily by [`Gpu`] as the proc's targets are derived.
    ///
    /// Held in [`IsolateBox`] rather than bare `Box<dyn Isolate>` so that **dropping
    /// one under a lock is loud** (`l1_concurrency.md` §12.16, gap G3b): a real
    /// isolate's `Drop` is a blocking teardown, and reaping a `Proc` is what runs it.
    pub isolates: BTreeMap<GpuId, IsolateBox>,
    /// ★★★ **Targets this proc needs an isolate for and does not yet have one for** —
    /// the durable half of the R1 spawn deferral (`l1_concurrency.md` §3.3).
    ///
    /// Set by [`Spine::defer_isolate`] under the device write lock, cleared by
    /// [`Spine::install_isolate`] when the spawn lands, or by [`Proc::vacate`] when the
    /// proc stops existing. It is what makes the gap between *decided* and *materialized*
    /// a **nameable** state rather than an anonymous absence: `kayfabe_fwd::checkout`
    /// answers [`kayfabe_fwd::FwdFault::NoTarget`] ("an internal inconsistency") when
    /// there is no isolate and none was ever asked for, and
    /// `kayfabe_fwd::FwdFault::IsolatePending` when one is on its way. The second is
    /// converging staleness in §12.9's sense — a *legal* concurrent state that resolves
    /// by materializing and re-planning, never a guest-visible fault.
    ///
    /// ⊘ Private: the only writers are the two spine ops above, and a plane that could be
    /// set from outside them would be a second place that decides an isolate is wanted.
    pending_isolates: BTreeSet<GpuId>,
    /// ★ MG-5: this proc's per-**target** private GPA arenas (disjoint by
    /// construction, per GPU). Recycled per target at the reap quiesce point (#80).
    pub arenas: BTreeMap<GpuId, GpaArena>,
    /// The set of GPU targets this proc spans (has materialized an isolate/arena for).
    /// Drives per-target completion composition (no cross-GPU serialization).
    pub targets: BTreeSet<GpuId>,
    /// Per-proc poll bookkeeping.
    pub poll: PollState,
    /// ★★ **T0's queue** (`l1_os_shell.md` §7.6 T0, gap G2): host objects whose core-side
    /// owner — a [`Vas`] or a [`Channel`] — was dropped by [`Gpu::sync_proc_to_boundary`]'s
    /// `retain` **while this proc is still alive**, keyed by the target GPU whose isolate
    /// namespace they belong to.
    ///
    /// This is the one lifecycle path §7.0's backstop does not cover. Everywhere else,
    /// something dies: a proc retires, a worker HUPs, an isolate is reaped, and the
    /// isolate process boundary frees the whole client tree. Here **nothing dies** — the
    /// guest freed a *subset* of its objects and kept running, which is not an exotic case
    /// but the steady state of a training job. So per-object reclamation is not an
    /// optimisation on this path; it is the only reclamation there is.
    ///
    /// **Why a queue and not a call.** `refresh` runs under the device write lock, where
    /// R1 forbids issuing host verbs. Filling a queue is pure state; draining it is a
    /// plan-and-execute op like any other, run lock-free on a checked-out worker via
    /// [`Orphans::release_plan`] — the same disposal vocabulary and the same worker
    /// discipline the refused-commit path already uses, deliberately *not* a second
    /// reclamation mechanism.
    ///
    /// **Keyed by [`GpuId`] because a handle is only meaningful in its own isolate's
    /// namespace** (MG-5, boundary 2): a proc holds one isolate per target, so releasing
    /// a GPU0 handle on the GPU1 connection would be a cross-namespace free — precisely
    /// what [`kayfabe_mocks::HostLedger::free_of_unknown`] exists to catch.
    ///
    /// Private, with [`Proc::take_pending_release`] as the only way out: [`Orphans`] is
    /// `#[must_use]`, so a queue that leaves this struct cannot be dropped on the floor
    /// without the compiler saying so.
    pending_release: BTreeMap<GpuId, Orphans>,
    /// ★★ **w310 — how many guest-RAM pins this proc's VAS deaths actually reclaimed.**
    ///
    /// Cumulative over the proc's whole life, never reset. Public because it is the number
    /// a boot log prints and a bench criterion grades as a **floor** — *"the release path
    /// ran"* is a different claim from *"the release path is wired"*, and only a counter
    /// that a live boot moves can carry the first.
    pub pin_reclaim: PinReclaim,
    retired: bool,
    next_chan: u32,
}

impl Proc {
    /// ★★★ §12.42 — the `hClient` VALUES this component's declarations were made at.
    /// The lossy by-value view of [`Self::clients`]; see
    /// [`crate::project::ProcBoundary::client_values`].
    #[must_use]
    pub fn client_values(&self) -> BTreeSet<HClient> {
        self.clients.iter().map(|k| k.client).collect()
    }

    fn new(id: ProcId, anchor: ProcAnchor) -> Self {
        Proc {
            id,
            anchor,
            clients: BTreeSet::new(),
            client_ids: BTreeSet::new(),
            vases: BTreeMap::new(),
            channels: BTreeMap::new(),
            chan_ids: BTreeMap::new(),
            exec: ExecPlane::default(),
            completion: CompletionQueue::new(),
            fences: FenceArms::new(),
            isolates: BTreeMap::new(),
            pending_isolates: BTreeSet::new(),
            arenas: BTreeMap::new(),
            targets: BTreeSet::new(),
            poll: PollState::default(),
            pending_release: BTreeMap::new(),
            pin_reclaim: PinReclaim::default(),
            retired: false,
            next_chan: 0,
        }
    }

    /// ★★ **T0's drain, and its precondition, as ONE indivisible act**: check a worker
    /// out of `gpu`'s isolate and take that isolate's pending-release queue **iff the
    /// isolate was otherwise idle** (`l1_os_shell.md` §7.6 T0).
    ///
    /// ## Why the idle test exists — measured, not reasoned
    ///
    /// Reclaiming an object is only safe if nothing is still *using* it, and the thing
    /// that uses host objects is an in-flight verb — which by construction runs with no
    /// lock held, so no lock can exclude it. The first version of T0 drained on every
    /// checkout, and `retry_ledger.rs`'s scripted re-stale wedged immediately with
    /// `RmError::BadHandle`: a publisher was parked *inside its mapping verb*, referring
    /// to a host VAS, when the guest freed that VASpace; the drain then freed the VAS
    /// underneath the parked verb. Our own reclamation became a use-after-free, and it
    /// surfaced to the guest as an anonymous host error rather than as staleness.
    ///
    /// The predicate that excludes it is the one the reap already uses for the identical
    /// reason ([`Isolate::is_quiesced`], `l1_concurrency.md` §12.16 gap G3: *"the reap
    /// would otherwise tear an isolate down under a live connection held by a foreign
    /// thread"*). It is **sufficient**, and that is a property of the plan/execute/commit
    /// shape rather than a hope:
    ///
    /// - a plan is made under the proc lock and **checks its worker out as the last thing
    ///   it does there** (§7.3), so any op that planned *before* the queue was filled
    ///   holds a worker of this isolate;
    /// - every path that gives a worker back — commit, refused commit, mid-chain failure —
    ///   disposes of its own orphans *before* the check-in (`SharedDevice::verb_op`);
    /// - so "no worker is checked out" ⟹ every op that could still name a dropped `Vas`'s
    ///   or `Channel`'s host objects has already finished with them;
    /// - and an op that plans *after* the fill cannot name them at all — `retain` removed
    ///   them from core state, and a plan is derived from core state.
    ///
    /// Getting this gate wrong is asymmetric in exactly the way `is_quiesced`'s own docs
    /// describe: draining too early is a use-after-free, draining too late leaves the
    /// queue for the next op or for the backstop sweep. So the test is conservative and
    /// the queue is simply left in place when it fails.
    ///
    /// Indivisible because it must be: the idle test has to be read **before** this
    /// caller's own checkout makes it false, and both must happen inside the one locked
    /// phase. Splitting it into two public calls is how that ordering would eventually be
    /// got wrong. `Orphans` is `#[must_use]`, so the disposal obligation travels with the
    /// returned value; the caller runs [`Orphans::release_plan`] on the returned worker,
    /// with no lock held.
    pub fn checkout_with_pending_release(&mut self, gpu: GpuId) -> (Option<Worker>, Orphans) {
        let Proc {
            isolates,
            pending_release,
            ..
        } = self;
        let Some(iso) = isolates.get_mut(&gpu) else {
            return (None, Orphans::default());
        };
        let idle = iso.is_quiesced();
        let worker = iso.checkout();
        let orphans = if worker.is_some() && idle {
            pending_release.remove(&gpu).unwrap_or_default()
        } else {
            Orphans::default()
        };
        (worker, orphans)
    }

    /// The targets with something queued for release (§7.6 T0's "backstop for a proc
    /// that goes quiet" — the sweep asks this before it checks a worker out).
    #[must_use]
    pub fn pending_release_targets(&self) -> Vec<GpuId> {
        self.pending_release
            .iter()
            .filter(|(_, o)| !o.is_empty())
            .map(|(&g, _)| g)
            .collect()
    }

    /// ★ **What is queued for release, per target** — the read-only half of
    /// [`Proc::pending_release_targets`] (`l1_concurrency.md` §12.35).
    ///
    /// Exists so that "this host object was *scheduled* for disposal" is an observable
    /// fact and not an inference from the absence of a `Free` verb. That distinction is
    /// the whole of the §12.35 teardown post-condition: an outstanding object whose owner
    /// the core dropped is a **leak** if nothing ever queued it, and the ordinary
    /// deferred T0 disposition if something did. A test cannot tell those apart without
    /// this window, and telling them apart is the point.
    ///
    /// `&Orphans` and never `&mut`: [`Proc::checkout_with_pending_release`] stays the
    /// only way the queue leaves this struct, so the `#[must_use]` disposal obligation
    /// cannot be sidestepped by reaching in here.
    pub fn staged_releases(&self) -> impl Iterator<Item = (GpuId, &Orphans)> {
        self.pending_release.iter().map(|(&g, o)| (g, o))
    }

    /// How many host objects + mappings + guest-RAM windows are queued for release across
    /// every target — diagnostics, and the executable statement that a drain actually
    /// drained.
    ///
    /// ★ **w310** — delegates to [`Orphans::len`] rather than summing two named fields, so a
    /// kind added to `Orphans` cannot go uncounted here. The previous hand-rolled sum would
    /// have silently omitted `guest_ram`.
    #[must_use]
    pub fn pending_release_len(&self) -> usize {
        self.pending_release.values().map(Orphans::len).sum()
    }

    /// ★★★★★ **w317 — is there staged disposal work here that a budgeted drain can
    /// actually issue?**
    ///
    /// Deliberately **not** `!pending_release.is_empty()`. An entry whose target GPU has no
    /// isolate is unreachable for *any* drain — [`Proc::drop`] skips it by the same test
    /// (`isolates.get_mut(&gpu)` ⇒ `continue`), and its disposition of record is §7.0
    /// namespace death. Gating the reap on the wider predicate would defer such a proc at
    /// **every** quiesce point forever, turning a bounded drain into a permanent leak — the
    /// exact "defers indefinitely is a leak with extra steps" failure. The two predicates
    /// must agree, and this is the one both sides use.
    #[must_use]
    pub fn has_drainable_releases(&self) -> bool {
        self.pending_release
            .iter()
            .any(|(gpu, o)| !o.is_empty() && self.isolates.contains_key(gpu))
    }

    /// ★★★★★ **w317 — the budgeted twin of [`Proc::checkout_with_pending_release`]:** check
    /// a worker out of one target's isolate and take **at most `budget`** of that target's
    /// staged disposals, leaving the remainder queued.
    ///
    /// Same indivisibility, same reason: the idle test has to be read *before* this caller's
    /// own checkout makes it false, so both happen inside the one locked phase. What differs
    /// is only that the queue is **split** rather than taken whole
    /// ([`Orphans::split_off_budget`], which preserves the release order across batches).
    ///
    /// Returns `None` when there is nothing drainable, when the chosen isolate is not
    /// quiesced (draining under a live verb is the use-after-free
    /// [`Proc::checkout_with_pending_release`]'s idle test exists to prevent — the budget
    /// does not relax it), or when its pool has no worker free. All three are **skips**, and
    /// the queue is simply left where it is.
    ///
    /// ⚠ **One target per call, by construction.** A second batch would need a second worker
    /// of the same isolate; the caller loops instead, and each loop turn re-reads the idle
    /// test rather than assuming it still holds.
    pub fn checkout_retired_release_budgeted(
        &mut self,
        budget: usize,
    ) -> Option<(GpuId, Worker, Orphans)> {
        if budget == 0 {
            return None;
        }
        let Proc {
            isolates,
            pending_release,
            ..
        } = self;
        let gpu = *pending_release
            .iter()
            .find(|(gpu, o)| !o.is_empty() && isolates.contains_key(gpu))
            .map(|(gpu, _)| gpu)?;
        let iso = isolates.get_mut(&gpu)?;
        if !iso.is_quiesced() {
            return None;
        }
        let worker = iso.checkout()?;
        let q = pending_release.get_mut(&gpu)?;
        let batch = q.split_off_budget(budget);
        if q.is_empty() {
            pending_release.remove(&gpu);
        }
        Some((gpu, worker, batch))
    }

    /// This proc's isolate for GPU `gpu`, if materialized. Address/exec ops route
    /// through the isolate of their **op's target GPU** (MG-5).
    #[must_use]
    pub fn isolate(&self, gpu: GpuId) -> Option<&dyn Isolate> {
        self.isolates.get(&gpu).map(|iso| &**iso)
    }

    /// Mutable access to this proc's isolate for GPU `gpu` (materialized by [`Gpu`]).
    pub fn isolate_mut(&mut self, gpu: GpuId) -> Option<&mut IsolateBox> {
        self.isolates.get_mut(&gpu)
    }

    /// ★ Is every one of this proc's per-target isolates **quiesced** — no worker
    /// checked out anywhere (`l1_concurrency.md` §12.16, gap G3)?
    ///
    /// The reap's precondition, and the reason `Spine::reap_retired` **checks**
    /// instead of trusting the adapter's declared quiesce point. The adapter picks
    /// *when* to try (the GSP re-handshake / idle edge, L10); this picks whether it is
    /// actually safe, per proc, per target. A proc with a verb still in flight on ANY
    /// of its GPUs is not reapable on any of them: its `Proc` is one value and
    /// dropping it drops every isolate it owns.
    ///
    /// Vacuously true for a proc that materialized no target — there is no sandbox to
    /// tear down, so there is nothing to wait for.
    #[must_use]
    pub fn is_quiesced(&self) -> bool {
        self.isolates.values().all(|iso| iso.is_quiesced())
    }

    /// ★ **Stage host objects for release** — the only way into `pending_release` from
    /// outside this module (`l1_os_shell.md` §7.6 T0, and §7.5's wedge).
    ///
    /// Two producers, and the second is why this is public. The first is
    /// [`Spine::stage_dropped_vases`]/`stage_dropped_channels`: the guest freed a subset
    /// while the process kept running. The second is §7.5's **abandoned chain**: a wedged
    /// worker's [`kayfabe_isolate::VerbFailure::orphans`] are host objects that exist,
    /// that the unwind could not run on, and that no `Vas` or `Channel` names — the exact
    /// UNACCOUNTED shape §12.35's audit reports. Staging them is what makes their
    /// disposition (§7.0 namespace death) a *stated* one rather than an unnameable set.
    ///
    /// Pure bookkeeping — no verb — so it is legal under the device write lock, exactly
    /// like the T0 staging it shares a queue with.
    pub fn stage_release(&mut self, gpu: GpuId, orphans: Orphans) {
        if orphans.is_empty() {
            return;
        }
        let q = self.pending_release.entry(gpu).or_default();
        q.unmap.extend(orphans.unmap);
        q.free.extend(orphans.free);
    }

    /// ★★ **VACATE — the clean death** (`l1_concurrency.md` §12.35): this proc's
    /// component vanished (the guest freed its client root, or it was absorbed by a
    /// merge). It refuses every new op from this instant, exactly like [`Proc::retire`],
    /// and it is out of every routing map — but its **isolates stay live**, because its
    /// staged `pending_release` queue still has to be disposed of and a retired isolate
    /// refuses every verb, disposal included.
    ///
    /// The split from [`Proc::retire`] is the whole of §12.35's second half, and it is a
    /// statement about *why* the proc died rather than a convenience:
    ///
    /// | death | isolate | why |
    /// |---|---|---|
    /// | **vacate** — the component vanished cleanly | stays live until the reap | the sandbox is healthy; its handles can and should be freed per object |
    /// | **retire** — worker HUP, condemnation, out-of-band kill | stopped at once | the sandbox is untrustworthy or already dead (§12.17, no resurrect); the disposition of record is the session's own death (§7.0) |
    ///
    /// Both are *removal* from the live set. Only one of them can still clean up, and
    /// pretending otherwise is what made §12.33's residue unreclaimable.
    ///
    /// ★★ **And it CANCELS** (`l1_os_shell.md` §7.6 T2): the returned [`Cancels`] is one
    /// latched break signal per verb this proc still has in flight.
    pub fn vacate(&mut self) -> Cancels {
        self.retired = true;
        // ★ R1's spawn deferral: a proc that has left the live set wants no isolate. The
        // marker is dropped here so the state says so, rather than relying on every reader
        // to remember to check `retired` first — [`Proc::wants_isolate`] checks anyway,
        // because a race can still hand `install_isolate` a sandbox spawned a moment
        // before this ran, and that one is refused on the same predicate.
        self.pending_isolates.clear();
        // ★ §7.6 T2 / §15 amendment 4 — the proc is gone, so every verb still in flight
        // for it is work whose requester no longer exists. Latch a cancel for each; the
        // shell discharges them after the guards drop (R1: firing one is a syscall).
        // Before this, a pending verb ran to completion against a dead proc and only its
        // commit noticed — correct, but it held a pool worker and a host round trip for
        // an answer nobody was waiting for.
        let mut cancels = Cancels::new();
        for iso in self.isolates.values_mut() {
            cancels.absorb(iso.request_cancel_all(CancelReason::ProcExit));
        }
        cancels
    }

    /// Stage 1 of teardown (lesson L10): stop every per-target isolate, mark retired.
    /// Heavy data-plane reap happens at the proven quiesce point, then drop.
    ///
    /// The **violent** death — see [`Proc::vacate`] for the clean one and for why the
    /// two must differ.
    pub fn retire(&mut self) -> Cancels {
        // Order matters and is not stylistic: latch the cancels BEFORE the isolates are
        // retired. A retired isolate refuses every new checkout, but the slots that are
        // already out are exactly the ones that need cancelling — and `vacate` reads them
        // through `checked_out()`, which stays true either way. Doing it after would still
        // work today; doing it first states the dependency.
        //
        // ★ **Only if it has not already vacated.** `Spine::retire_proc` is
        // `vacate`-then-`retire` (§12.35's one removal point), so an unguarded second
        // `vacate()` here latches a SECOND break signal for the same worker on the same
        // txn. It is not harmless: the delivered count doubles, the second delivery
        // re-arms a seam the first one may already have disarmed (§7.2 refinement 4), and
        // any accounting over "how many cancels did this teardown fire" becomes a lie.
        // Caught by `cancellation.rs`'s exact `(fired, delivered, observed)` census,
        // which reported (2,2,1) where the design says (1,1,0).
        let cancels = if self.retired {
            Cancels::new()
        } else {
            self.vacate()
        };
        for iso in self.isolates.values_mut() {
            iso.retire();
        }
        cancels
    }

    /// True once retired (a retired proc must refuse new ops).
    #[must_use]
    pub fn is_retired(&self) -> bool {
        self.retired
    }

    /// ★★★ **Is an isolate for `gpu` DECIDED-BUT-NOT-YET-MATERIALIZED?** (R1's spawn
    /// deferral — see [`Self::pending_isolates`].)
    ///
    /// The predicate `kayfabe_fwd::checkout` reads to tell converging staleness (*"one is
    /// on its way; materialize it and re-plan"*) apart from an internal inconsistency
    /// (*"nothing ever asked for one"*). ⊘ False for a retired proc even while the marker
    /// is set: a retired proc refuses every op by an earlier arm, and answering "pending"
    /// for one would send a caller off to spawn a sandbox for a process that is gone.
    #[must_use]
    pub fn wants_isolate(&self, gpu: GpuId) -> bool {
        !self.retired && self.pending_isolates.contains(&gpu)
    }

    /// True while this proc has touched no data-plane state (merge legality,
    /// lesson L9).
    fn is_untouched(&self) -> bool {
        // "Touched" = host-materialized data-plane state (any target's arena carved,
        // host channel / host VAS allocated, or a binding published into a host VAS).
        // Pure RPC address-table bookkeeping (`Binding::host = None`) is NOT host state
        // — it is re-derivable from the graph, so it never blocks an early merge. (A
        // proc that has materialized no target yet has empty `arenas` → vacuously
        // untouched.)
        // ★ T0: a proc whose subset-frees are still queued HAS host state — it just is
        // not reachable through a `Vas` or a `Channel` any more. Without this clause a
        // proc that published and then freed everything could read as untouched again
        // (G6's intra-arena free rewinds the cursor), which is exactly the early-merge
        // legality question lesson L9 answers "no" to.
        self.pending_release.values().all(Orphans::is_empty)
            && self.arenas.values().all(GpaArena::is_untouched)
            && self.channels.values().all(|c| c.host_channel.is_none())
            && self
                .vases
                .values()
                .all(|v| v.host_vas.is_none() && v.table.iter().all(|(_, _, b)| b.host().is_none()))
    }
}

impl Drop for Proc {
    /// ★★ **THE DRAIN — the last step of `decide → stage → drain → remove`**
    /// (`l1_concurrency.md` §12.35).
    ///
    /// A `Proc` cannot be destroyed without the host cleanup it already staged being
    /// *issued*. That is the property §12.33 found missing, and putting it here rather
    /// than at a call site is the point: "removed before cleaned" stops being
    /// expressible, in the same way `release(arena)`-by-value stopped double-release
    /// from being expressible. Every path that destroys a proc — the reap's
    /// [`Reclaimed`], a test dropping a `Gpu`, the device's own teardown — goes through
    /// this one.
    ///
    /// **Three preconditions, and each is checked rather than assumed.**
    ///
    /// 1. **Lock-free (R1).** A verb may not be issued under a ranked lock. This is
    ///    already law for a `Proc` drop for a *different* reason —
    ///    [`kayfabe_isolate::IsolateBox`]'s own `Drop` asserts it, because a real
    ///    isolate's teardown is a blocking syscall (§12.16 G3b) — and
    ///    [`kayfabe_isolate::Worker::execute`] asserts it again for the verb. So this
    ///    adds no new obligation; it *relies* on one the design already enforces.
    /// 2. **Quiesced (M2-a's rule).** Per-object reclamation must never race an
    ///    in-flight verb, and no lock can exclude one — only the isolate's own quiesce
    ///    predicate can. `checkout()` returning a worker on an idle isolate is that
    ///    predicate, the same one [`Proc::checkout_with_pending_release`] uses.
    /// 3. **Not already panicking.** A panic inside `Drop` during an unwind aborts the
    ///    process. Same guard, same reason, as `IsolateBox`.
    ///
    /// **What happens when it cannot run.** A *retired* isolate refuses the checkout, so
    /// a violently-killed proc's queue is left exactly where it was — and its disposition
    /// of record is the one it always had: the session's death frees the whole client
    /// tree (§7.0, the C's #80 backstop). That is a **different disposition**, not a
    /// failure, and it is why [`Proc::vacate`] exists to keep the clean path's isolates
    /// alive.
    fn drop(&mut self) {
        if std::thread::panicking() || self.pending_release.is_empty() {
            return;
        }
        let Proc {
            isolates,
            pending_release,
            ..
        } = self;
        for (gpu, orphans) in core::mem::take(pending_release) {
            if orphans.is_empty() {
                continue;
            }
            let Some(iso) = isolates.get_mut(&gpu) else {
                continue;
            };
            // The quiesce test and the checkout as one act, for the reason
            // `checkout_with_pending_release` states: splitting them is how the ordering
            // gets got wrong. A retired isolate returns `None` here and keeps its queue's
            // disposition as §7.0 namespace death.
            if !iso.is_quiesced() {
                continue;
            }
            let Some(mut worker) = iso.checkout() else {
                continue;
            };
            // Best effort by construction: a refused release leaves objects that the
            // isolate's imminent death disposes of in bulk anyway. `execute` is what
            // asserts R1.
            let _ = worker.execute(&orphans.release_plan());
            iso.checkin(worker);
        }
    }
}

/// ★ What one [`Spine::reap_retired`] reclaimed — **the corpses, handed to the
/// caller to drop with ZERO locks held** (`l1_concurrency.md` §12.16, gap G3b).
///
/// The reap runs under the device write lock (it is a spine op: it recycles arenas
/// into the shared per-target windows). Dropping a [`Proc`] there would run every one
/// of its [`kayfabe_isolate::IsolateBox`]es' `Drop` — `waitpid`, namespace teardown,
/// fd close — under a rank-0 lock. Returning them instead makes the lock-free drop
/// the *only* thing the caller can do with the value, and
/// [`kayfabe_isolate::IsolateBox`]'s `Drop` panics naming R1 if it is not.
///
/// Deliberately opaque: there is no accessor that hands out a `&Proc`, because a
/// reaped proc is not a thing to consult — the only live questions are "how many"
/// (diagnostics) and "when does it die" (now, here, unlocked).
#[must_use = "the reaped procs must be DROPPED, and dropped with ZERO ranked locks \
              held (R1) — a real isolate's Drop is waitpid + namespace teardown. \
              Bind this value outside every lock scope and let it fall."]
pub struct Reclaimed {
    procs: Vec<Proc>,
    deferred: usize,
    deferred_for_drain: usize,
    orphaned: Vec<(GpuId, core::ops::Range<u64>)>,
}

/// ★★★★★ **w317 — WHAT A REAP DOES WITH A PROC WHOSE STAGED DISPOSAL QUEUE IS NOT EMPTY.**
///
/// # ⊘ An enum and not a `bool`, deliberately
///
/// `same_flag_opposite_polarity` is a paid-for lesson in this tree, and a bare
/// `reap_retired(true)` is exactly its shape: the two arms below are **opposite dispositions
/// of real host objects**, and nothing at a call site would say which one `true` meant. The
/// names carry the meaning to the call site instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapPolicy {
    /// **Reap every quiesced proc**, and let [`Proc`]'s `Drop` issue whatever is still staged
    /// — *all of it*, in one blocking burst, on the calling thread.
    ///
    /// This is what [`Spine::reap_retired`] has always done and what every caller outside the
    /// QEMU register-write path still wants: a test, a `Gpu` teardown or the executor's
    /// quiesce point has no guest to stall, so clause (b) of `INLINE-SAFE` does not bind and
    /// the simplest disposal is the right one.
    Unbudgeted,
    /// **Hold a proc back while it still has drainable staged work**
    /// ([`Proc::has_drainable_releases`]), so the burst above can never happen.
    ///
    /// For the one caller where clause (b) *does* bind: `Regs::write` runs with the QEMU BQL
    /// held, so a blocking disposal there halts **every vCPU and QEMU's main loop**, and
    /// w314 measured that burst at **2.65–3.70 s** against `scrubberDestruct`'s 4 000 ms.
    /// Pair this with `SharedDevice::drain_retired_budgeted`, which is what actually empties
    /// the queue a bounded slice per trap; on its own this arm only defers.
    HoldUndrained,
}

/// ★★★★★ **w317 — ONE TURN of the budgeted retired drain**, planned under the device write
/// lock and executed with none held (`Spine::plan_retired_drain`).
///
/// It carries a checked-out [`Worker`] and an [`Orphans`] batch, which is why it is
/// `#[must_use]` twice over: dropping it on the floor both leaks the host objects the batch
/// names *and* wedges the isolate's pool slot, so the proc never quiesces and is deferred at
/// every quiesce point after — the permanent-defer failure the budget exists to avoid.
#[must_use = "a RetiredDrain holds a checked-out Worker and an undisposed Orphans batch — \
              dispose of `orphans` on `worker`, then return `worker` via \
              `Spine::checkin_retired`"]
pub struct RetiredDrain {
    /// The retired proc this batch came from.
    pub pid: ProcId,
    /// The target whose isolate namespace the handles belong to (MG-5, boundary 2).
    pub gpu: GpuId,
    /// A worker of `gpu`'s isolate, checked out as part of the same locked phase that read
    /// the idle test.
    pub worker: Worker,
    /// At most `budget` disposals, in the release order [`Orphans::split_off_budget`]
    /// preserves across batches.
    pub orphans: Orphans,
}

impl Reclaimed {
    /// How many procs this reap took (and this value will destroy).
    #[must_use]
    pub fn len(&self) -> usize {
        self.procs.len()
    }

    /// True if the reap took nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.procs.is_empty()
    }

    /// How many retired procs the reap **put back** because they were not quiesced
    /// (§12.16, G3) — a verb was still in flight on one of their isolates. They stay
    /// on the retired list for the next quiesce point; nothing is lost, and nothing
    /// was torn down under a live connection.
    #[must_use]
    pub fn deferred(&self) -> usize {
        self.deferred
    }

    /// ★★★★★ **w317 — of [`Reclaimed::deferred`], how many were held back because their
    /// staged disposal queue is not empty yet**, rather than because a verb is in flight.
    ///
    /// The two are opposite facts wearing the same word. *Not quiesced* means something is
    /// still using the proc's host objects and reaping would be a use-after-free; *not
    /// drained* means the proc is idle and we are deliberately spending its teardown over
    /// several traps instead of one. Only the second is expected to be non-zero on a healthy
    /// teardown, and only the second going **monotonically up** is the "deferred work is
    /// piling up" failure this rung pre-registered as outcome (B). A single `deferred` count
    /// cannot tell them apart, and `a_count_cannot_see_a_substitution` is what happens when a
    /// caller tries.
    #[must_use]
    pub fn deferred_for_drain(&self) -> usize {
        self.deferred_for_drain
    }

    /// ★ G7 (§12.19) — GPA ranges the reap **could not route home**: their target no
    /// longer exists, or its window refused them as foreign. Empty on every path the
    /// core can currently reach, and that is the point — this used to be an
    /// `if let Some(t) = …` whose `else` silently dropped the arena, permanently losing
    /// that range from the device's guest-physical space with nothing said anywhere.
    /// A leak the caller can see is a leak somebody can fix.
    #[must_use]
    pub fn orphaned(&self) -> &[(GpuId, core::ops::Range<u64>)] {
        &self.orphaned
    }
}

impl core::fmt::Debug for Reclaimed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Reclaimed")
            .field("reaped", &self.procs.len())
            .field("deferred", &self.deferred)
            .field("deferred_for_drain", &self.deferred_for_drain)
            .field("orphaned", &self.orphaned)
            .finish()
    }
}

/// ★ Mutable access to the per-proc containers, abstracted so the L1 adapter can
/// own each [`Proc`] inside its own lock cell (e.g. `Mutex<Proc>`) while the core's
/// spine ops still drive the whole set under the device write lock
/// (`l1_concurrency.md` §3.4: "the L1 wrapper owning `Proc`s and the core's
/// cross-proc ops taking iterator/visitor arguments").
///
/// Contract (normative):
/// - All access is `&mut`-based — a spine op runs under whole-device exclusivity
///   (L1's device *write* lock), so `Mutex::get_mut`-style lock-free access is
///   sufficient; the trait deliberately has **no shared-read accessor** (one would
///   be unimplementable for a lock cell).
/// - [`ProcSet::iter_mut`] yields procs in **ascending `ProcId` order** — derived
///   state must stay a deterministic function of the graph, never of iteration
///   order (decision #27).
pub trait ProcSet {
    /// The proc with id `id`, if live.
    fn get_mut(&mut self, id: ProcId) -> Option<&mut Proc>;
    /// Insert a newly-derived proc.
    fn insert(&mut self, id: ProcId, proc: Proc);
    /// Remove (and return) a proc — retirement, or the poll split-borrow.
    fn remove(&mut self, id: ProcId) -> Option<Proc>;
    /// All live procs, ascending by `ProcId` (determinism contract above).
    fn iter_mut(&mut self) -> impl Iterator<Item = (ProcId, &mut Proc)>;
}

impl ProcSet for BTreeMap<ProcId, Proc> {
    fn get_mut(&mut self, id: ProcId) -> Option<&mut Proc> {
        BTreeMap::get_mut(self, &id)
    }
    fn insert(&mut self, id: ProcId, proc: Proc) {
        BTreeMap::insert(self, id, proc);
    }
    fn remove(&mut self, id: ProcId) -> Option<Proc> {
        BTreeMap::remove(self, &id)
    }
    fn iter_mut(&mut self) -> impl Iterator<Item = (ProcId, &mut Proc)> {
        BTreeMap::iter_mut(self).map(|(&id, p)| (id, p))
    }
}

/// Device-global ceiling on [`Spine::pt_learned`] — how many *discovered* page-table pages
/// the ownership index will hold across every address space on every target.
///
/// ⚠ **Boundary-1**: the guest chooses how many page-table pages exist, so an unbounded
/// index is a guest-driven allocation. The per-`Vas` chain is already capped by
/// `kayfabe_fwd::MAX_PT_META`; this is the *device* sum, which that per-VAS cap does not
/// bound because the guest also chooses how many address spaces to create.
///
/// ★ The number: 2^17 pages ≈ 4 MiB of index for a guest whose page tables span 512 MiB of
/// framebuffer at 4 KiB granularity. Sized to be reached only by a guest doing something
/// no CUDA workload does, so [`Spine::pt_learned_refused`] staying zero is the ordinary
/// case and a non-zero value is a real signal rather than routine noise.
pub const MAX_PT_LEARNED: usize = 1 << 17;

/// ★ The device-global SPINE (`l1_concurrency.md` §3.4 — the `Gpu` ownership
/// split): everything a per-proc op only *reads* (graph, routing maps, targets)
/// plus the spine-mutating machinery (factory, window geometry, the retired list).
/// Separately borrowable from [`Gpu::procs`]/[`Gpu::system`], so the L1 adapter
/// can put THIS under the device `RwLock` and each [`Proc`] under its own `Mutex`
/// as a **lock swap, not a rewrite**:
///
/// - **per-proc op** = device *read* lock (`&Spine`) + that proc's `Mutex`
///   (`&mut Proc`) — `kayfabe-fwd`'s route/act split and `publish_backing`;
/// - **spine op** = device *write* lock (`&mut Spine` + exclusive access to every
///   proc, expressed as the [`ProcSet`] argument) — [`Spine::apply`], the
///   completion pump/poll/drain, [`Spine::reap_retired`].
///
/// Still pure L0: no lock lives here; the split only shapes ownership so the locks
/// drop in cleanly later (rules R1–R4 of the L1 design).
pub struct Spine {
    /// The Axis-B behavior this device was realized with. The core only ever
    /// calls trait methods on it — never names a generation. **One arch for all
    /// targets** (MG-6: V1 multi-GPU is homogeneous-arch).
    ///
    /// ★ **Private, read-only through [`Spine::arch`]** — and that is what makes MG-6's
    /// homogeneity guard (`ensure_target`, `plan_refresh`) a statement rather than a
    /// branch. Written exactly once, at realize; every [`GpuTarget`] is minted
    /// with `self.arch.name()` and `GpuTarget::arch_name` is private too, so within this
    /// composition `t.arch_name != self.arch.name()` cannot hold. While the field was
    /// `pub`, an out-of-crate assignment was the ONLY thing that could make it hold — so
    /// the guard's unreachability was a convention, not a property. It is a property now.
    arch: Box<dyn Arch>,
    /// ★ Source of truth (decision #14).
    pub rmgraph: RmGraph,
    /// ★ Data-plane routing (derived): `(GpuId, PDB)` → owning proc (MG-3). Keyed on
    /// the target because a `Pdb` is a per-GPU namespace. The `Vas` lives in
    /// `procs[pid].vases[(gpu, pdb)]`.
    pub by_pdb: BTreeMap<(GpuId, Pdb), ProcId>,
    /// ★ Exec-plane routing (derived): `(GpuId, vChid)` → (proc, channel) (MG-3).
    pub by_vchid: BTreeMap<(GpuId, VChid), (ProcId, ChanId)>,
    /// ★★★ **#177** — the same routing fact keyed by the channel's own **stable
    /// identity** instead of by its vChid.
    ///
    /// `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` names its channel by `(hClient, hObject)`, not
    /// by a doorbell token, so [`Spine::by_vchid`] is the wrong key for it and decoding a
    /// vChid just to look one up would be a second description of a fact the projection
    /// already has. Built in the **same loop** as `by_vchid`, from the same
    /// [`crate::project::Boundaries`], so the two can never disagree about which proc owns
    /// a channel.
    ///
    /// ⊘ Deliberately holds only channels the projection could route. A channel whose
    /// vChid was never recovered is absent from both maps: it cannot be rung, so it must
    /// not be schedulable either.
    pub by_chan: BTreeMap<ResourceKey, (ProcId, ChanId)>,
    /// ★★★ **Page-table-page ownership: `(GpuId, physical page)` → the `Vas` whose page
    /// table it is** (`#102`/#13's operand split; the C's `m2_cpt`, `C:
    /// nvkvm_gpu_emul.c:596-609`).
    ///
    /// ## Why this is device-global and not per-`Proc`
    ///
    /// Because **the writer and the owner are different procs.** The guest kernel's
    /// CeUtils channel is what writes a user process's page tables — it identity-maps the
    /// whole framebuffer into its OWN address space and issues the writes as copies into
    /// that alias (`C: :4936-4952`). So when the parser sees that copy, the channel it is
    /// parsing belongs to the system proc while the page being written belongs to a user
    /// proc. A per-proc index cannot answer "whose page table is this?" — which is the
    /// only question the operand split turns on.
    ///
    /// ## Why the physical page and not the VA
    ///
    /// Because the operand form varies and the target does not. The same page-table write
    /// arrives as a *physical* destination from one channel and as a *virtual* one through
    /// the FB alias from another; the C hooks on the **resolved physical** in both cases
    /// (`C: :6353`, `:6437`) and so must we. Gating on `!dst_is_virtual`, as this port did,
    /// excludes exactly the case #13 is about.
    ///
    /// **Derived, never accreted** — rebuilt from the projection in [`Spine::refresh`]
    /// beside [`Spine::by_pdb`], seeded from each live `Vas`'s **root**: a PDB *is* the
    /// physical address of its root page directory, so the root page is a declared fact
    /// and needs no discovery.
    ///
    /// ★ **Deeper levels live in [`Self::pt_learned`]** — forward-populated by the decode
    /// at the guest's commit point, which is E8 and is now built. Both maps are re-derived
    /// from the same projection, which is what makes them prunable: a `Vas` that dies takes
    /// its pages with it, where the C's table was *"never pruned on handle free"*
    /// (`eight_blockers_resolved.md` §2).
    pub pt_roots: BTreeMap<(GpuId, u64), (ProcId, Pdb)>,
    /// ★★★ **The DEEPER levels — the same index, discovered instead of declared** (E8,
    /// `execution_plane_increments.md` §12). [`Self::pt_roots`]'s own doc named this as
    /// *"the next stage"*; this is it.
    ///
    /// ## Why a second map rather than more rows in the first
    ///
    /// Because the two are known in different ways, and collapsing them would hide it. A
    /// root is a **declared** fact — a PDB *is* the physical address of its root page
    /// directory, so it needs no discovery and cannot be wrong. Everything here was
    /// **discovered** by decoding guest bytes, so it is exactly as good as the decode that
    /// produced it. ⊘ A single map would let a diagnostic say "this page is a page table"
    /// without saying which of those two sentences it means.
    ///
    /// ## ★ Derived, never accreted — and that survives the incremental publish
    ///
    /// This is a projection of every live `Vas::pt_meta`, recomputed from scratch in
    /// [`Spine::refresh`] exactly as [`Self::pt_roots`] is. [`Spine::publish_pt_pages`]
    /// applies the *same function* early, at the decode's commit point, because a level
    /// learned in one pass must be recognisable before the next RM graph event — the guest
    /// does not wait for one. ⇒ publishing can never disagree with the projection: it is
    /// the projection, run sooner — asserted by
    /// `tests/tests/pt_index_projection.rs::pt_index_publish_equals_projection`, which
    /// publishes, forces a rebuild, and compares both maps for equality.
    ///
    /// ⚠ **That claim shipped in E8 v1 citing a test that DID NOT EXIST**, and it was
    /// false: publish refused the second claimant while [`Spine::refresh`] silently kept
    /// the lowest `ProcId`, so this map's answer flipped across a rebuild. Both paths now
    /// **decline** a contested page ([`Self::pt_contested`]) — the only rule implementable
    /// on both. ⊘ Grep every test name a doc cites; a citation is not a test.
    ///
    /// Pruning is therefore automatic and is the property the C artifact lacked: a `Vas`
    /// that dies takes its pages with it, where the C's table was *"never pruned on handle
    /// free"* (`eight_blockers_resolved.md` §2).
    ///
    /// ⚠ Bounded by [`MAX_PT_LEARNED`], device-global and guest-influenced — a
    /// boundary-1 concern. Overflow REFUSES the excess and counts it
    /// ([`Spine::pt_learned_refused`]); it never evicts, because evicting a page the guest
    /// is still writing would silently return it to "ordinary data" and unbind its leaves.
    pub pt_learned: BTreeMap<(GpuId, u64), (ProcId, Pdb)>,
    /// ★★★ Pages **more than one** `(proc, pdb)` has claimed — indexed for NOBODY.
    ///
    /// A physical page reachable from two live address spaces' page tables is either guest
    /// aliasing or a wrong decode. Either way we do not know whose it is, so the index
    /// declines to answer: [`Self::pt_page_owner`] returns `None`, `classify_ce` treats a
    /// write to it as ordinary data, and its leaves do not bind.
    ///
    /// ⊘ **Decline, do not pick a winner** — and this is the correction to E8's first cut,
    /// which had `publish_pt_pages` refuse the *second* claimant (first-by-arrival) while
    /// [`Spine::refresh`] silently kept the *lowest `ProcId`* (`entry().or_insert()`). Two
    /// different policies for one question, so `pt_page_owner` answered differently before
    /// and after a refresh — and the "REFUSED, not re-homed" property the design rests on
    /// was defeated by the projection, silently and uncounted. Declining is the only rule
    /// both paths can implement identically, because it does not depend on arrival order or
    /// on iteration order.
    ///
    /// ★ Sticky by construction: once contested, a page stays out even if a later publish
    /// re-offers it, so the incremental path is idempotent. `refresh` re-derives the set
    /// from scratch (a key claimed by ≥2 owners), so it is a projection like the rest.
    ///
    /// ★ **This is ROBUSTNESS, not a security boundary** — corrected 2026-08-05, and the
    /// correction matters because the first draft claimed the opposite.
    ///
    /// ⊘ It said a process could "forge a PDE at another proc's page table". **It cannot.**
    /// Unprivileged guest userspace does not author page-table entries at all; `nvidia.ko`
    /// does, and it already holds access to every address space in the guest — the same
    /// reason a Linux process cannot edit its own PTEs outside `mmap`. Procs A and B are
    /// both inside **one guest VM**, so guest-internal isolation is the guest kernel's job,
    /// exactly as it is on bare metal where a real GPU does not separate one process from
    /// another. This port's boundary is guest → host escape and no step of that scenario
    /// crosses it.
    ///
    /// ⇒ what declining buys is a **loud miss instead of a silent wrong binding** when our
    /// own decode is wrong or a buggy guest kernel does something unexpected. Worth having,
    /// and not a boundary. See `execution_plane_increments.md` §12.7.
    pub pt_contested: BTreeSet<(GpuId, u64)>,
    /// How many page publications [`Self::pt_learned`] has refused for want of room.
    ///
    /// ★ A counter and not a log: the refusal is a capacity fact, and the page that lost
    /// is not more interesting than the one before it. Non-zero means the address plane is
    /// under-provisioned for this guest, which is a sizing decision and not a bug to chase.
    pub pt_learned_refused: u64,
    /// ★★ **Context-object → address space**: a channel or TSG **resource** → the
    /// `(GpuId, Pdb)` whose table its context buffers belong in
    /// ([`crate::promote::route_promote_ctx`]).
    ///
    /// ## Why it is device-global, like [`Self::pt_roots`]
    ///
    /// Because a promotion's *issuer* and its *target* are two different questions, and
    /// the answer to the second is a property of the address space rather than of
    /// whoever asked. Answering it out of the issuing proc's own state would key the
    /// join on the RM client — which is precisely the C artifact's table, and which
    /// hardware has measured cannot identify a process: two concurrent CUDA processes
    /// share one duplicated client.
    ///
    /// ## Why by [`ResourceKey`], not by handle
    ///
    /// RM recycles object-handle values with no quarantine, so `(client, handle)` is a
    /// *label*. The route resolves the guest's handle through the LIVE handle table at
    /// the moment of use and then indexes here by the resulting stable identity, so a
    /// dup-kept ghost and the value's next tenant can never be confused for one another.
    ///
    /// **Derived, never accreted** — rebuilt in [`Spine::refresh`] beside
    /// [`Self::by_pdb`], and filtered through it, so a CONDEMNED component's context
    /// objects do not route: its address space must not attract new bindings.
    pub ctx_vas: BTreeMap<ResourceKey, (GpuId, Pdb)>,
    /// ★★★★★ §16.50 — **the physical halves of GPU-scoped global context buffers**, per
    /// target: `GpuId` → (`buffer_id` → the buffer RM allocated once for the whole GPU).
    ///
    /// # Why this is device-global and not on a [`Vas`]
    ///
    /// `s41b` measured a promotion carrying **ten VA halves and zero physical halves** in
    /// cup2's address space, while the cumulative tally showed the physicals had certainly
    /// arrived. They arrived **under a different proc**: RM publishes a global context
    /// buffer's physical address once, from the driver-init path, and then every later
    /// context maps that one buffer at its own VA and declares only the VA. Parking the
    /// physical in the emitting proc's `Vas` therefore guaranteed `joined=0` — two correct
    /// halves, correctly keyed, in two maps that never meet.
    ///
    /// ⊘ **Membership is derived from RM's arms, not from that boot's orphans** — see
    /// [`crate::promote::PhysHalfScope`]. Only ids whose phase-1 emitter reads
    /// `kgraphicsGetGlobalCtxBuffers` unconditionally are published here; `0x8`
    /// GFXP_CTRL_BLK is excluded because its arm may publish a *private* per-context
    /// buffer, and six global ids publish nothing at all.
    ///
    /// ★ It sits beside [`Self::ctx_vas`] at rank 0, which is what lets the L1 shell read
    /// it before taking the owning proc's lock and merge back after releasing it —
    /// see [`Gpu::promote_ctx`] for the single-owner form and the shell for the sharded
    /// one. ⊘ **Not `refresh`-derived**: unlike `by_pdb`/`ctx_vas` this is not a
    /// projection of the resource graph but an accreted record of what RM *declared*, and
    /// rebuilding it would erase a publication whose emitting client has since been freed
    /// — which is the normal case, since the driver-init client does not outlive boot.
    pub global_ctx_phys: BTreeMap<GpuId, crate::promote::GlobalCtxPhys>,
    /// ★ MG-6: per-target device state — one [`GpuTarget`] (its own guest-physical
    /// window + GSP-queue drain gate) per routable GPU. `GpuId::ZERO` is realized at
    /// [`Gpu::new`]; further targets are minted lazily as their Devices are derived.
    pub targets: BTreeMap<GpuId, GpuTarget>,
    /// ★ The completion-source registry (`l1_concurrency.md` §6, decision #37).
    ///
    /// Device-global, and deliberately **here** rather than on [`Proc`]: dispatch must
    /// resolve a source whose proc may have just retired, so the routing table has to
    /// outlive — and be keyed independently of — the proc it names. It sits at rank 0
    /// of the L1 lock order with the rest of the spine.
    ///
    /// Its retire wiring is in [`Spine::refresh`]: BOTH proc-retirement paths call
    /// [`SourceRegistry::deregister_proc`], which is what turns "a source signalled
    /// after its proc retired" into a loud `SourceFault` instead of the C's F4
    /// use-after-retire.
    pub sources: SourceRegistry,
    /// ★★ **Latched cancels awaiting a lock-free discharge** (`l1_os_shell.md` §7.1).
    ///
    /// Every path that removes a proc from the live set runs under the device WRITE
    /// lock, and every such path must cancel that proc's in-flight verbs — but firing a
    /// cancel is a syscall and R1 forbids syscalls under locks. So the locked phase
    /// LATCHES here and the shell drains it after the guards drop, which is exactly the
    /// two-step `WakeRequest` already uses (§7.1: *"cancel is the third user, which is
    /// the argument that it is the right mechanism rather than a third one"*).
    ///
    /// Private, with [`Spine::take_pending_cancels`] as the only way out, for the same
    /// reason `pending_release` is: [`Cancels`] is `#[must_use]`, so a batch that leaves
    /// this struct cannot be dropped on the floor without the compiler saying so.
    pending_cancels: Cancels,
    /// ★★★ **Deferred isolate materializations awaiting a lock-free spawn** (R1).
    ///
    /// The exact counterpart of [`Self::pending_cancels`], and it exists for the identical
    /// reason one increment further out: every path that *decides* a `(Proc, GpuId)` needs
    /// an isolate runs under the device WRITE lock ([`Spine::apply`] and the
    /// [`Spine::refresh`] it drives), and **spawning one is the most blocking thing this
    /// port does** — `clone` into six namespaces, `execveat` of a sealed memfd, a blocking
    /// hello handshake, and under `KAYFABE_ISOLATES=real` a chain of real host RM ioctls.
    /// R1 admits no exception for it. So the locked phase LATCHES here and the shell
    /// spawns after the guards drop, then re-acquires and RE-VALIDATES (R5).
    ///
    /// ⊘ **Not the authority on whether an isolate is still wanted** — that is
    /// [`Proc::wants_isolate`], which outlives this latch. This is a *work list*: taking
    /// it does not cancel the obligation, so a caller that drops it on the floor loses
    /// nothing except promptness (the next verb to that `(proc, gpu)` refuses with
    /// [`kayfabe_fwd::FwdFault::IsolatePending`] and materializes it then). Two derivations
    /// of one fact would be drift; these are two facts with different lifetimes.
    ///
    /// Private, with [`Spine::take_pending_spawns`] as the only way out, for the same
    /// reason `pending_cancels` is: [`PendingSpawns`] is `#[must_use]`.
    pending_spawns: PendingSpawns,
    /// ★★ **w310** — guest-RAM pin reclaim accumulated from procs that have since left the
    /// live set. Summed with the live procs' own tallies by [`Spine::pin_reclaim`].
    ///
    /// ⊘ It exists because the whole-proc teardown arm stages the pins **into a `Proc` that
    /// is then handed to the reap and dropped** — a counter that lived only there would be
    /// destroyed by the very event it counts, and the boot log would read `released=0` on
    /// exactly the runs where the most was released.
    pin_reclaim_gone: PinReclaim,
    /// ★ The isolate factory, behind an `Arc` because a spawn must be reachable with
    /// **zero** ranked locks held (`kayfabe_isolate::IsolateFactory`'s own docs). The L1
    /// shell keeps a clone of this same handle; there is one factory, not two.
    isolates: Arc<dyn IsolateFactory>,
    /// Geometry template for minting a fresh disjoint per-target window.
    geom: TargetGeom,
    next_proc: u32,
    /// Procs retired but not yet reaped (awaiting the quiesce point).
    pub retired: Vec<Proc>,
    /// ★ §12.13 — the **condemned components**: client sets whose proc was retired
    /// **out of band** ([`Spine::retire_proc`]) and which must therefore never be
    /// re-derived into a live [`Proc`] again.
    ///
    /// **Why never re-derived.** Not because the dead worker left host state the core
    /// cannot reason about — the isolate is a *process*, and the host kernel reclaims
    /// what it held. It is because **the guest's data died with it**: a published
    /// backing is host memory (`RmBackend::alloc_sysmem`) owned by that isolate's RM
    /// client, so re-materialising would hand the guest a **zeroed** backing for a VA it
    /// believes still holds its data — silent corruption, strictly worse than the
    /// resurrect it would be fixing. Refusing gives the guest what real hardware gives
    /// it: **sticky-fatal**, like an Xid, and recoverable exactly as an Xid is (see the
    /// key discussion below — a fresh RM client is a *different* component).
    ///
    /// **Why a client SET and not a [`ProcId`] or a [`ProcAnchor`].** A `ProcId` is
    /// minted per derivation and dies with the proc — the very thing that failed was
    /// that the *next* derivation minted a fresh one, so it cannot be the key. A
    /// `ProcAnchor` is only the *smallest* client of the component, so freeing that one
    /// client while keeping the rest would silently re-label the component and slip the
    /// condemnation. The client set is exactly what [`Spine::refresh`] already matches
    /// boundaries on (intersection), so keying on it makes condemnation survive every
    /// re-derivation the guest can provoke, including component splits (both halves
    /// intersect, both stay condemned) and re-labels.
    ///
    /// ★ **NARROWED (§12.39, finding 3): that split argument is sound for the CONDEMNED
    /// path and covers only it.** A condemned boundary is handed `None` by
    /// [`Spine::plan_refresh`] and touches no `Proc` at all, so "both halves intersect,
    /// both stay condemned" is a statement about *this* list and it holds. On the **live**
    /// path the same shape was a defect: both halves of a split matched the one live proc,
    /// `survivors` kept it twice, `vanishing` came out empty and
    /// `sync_proc_to_boundary` ran twice on it — the second call overwriting the first, so
    /// one half lost its clients, vases and channels in silence while its isolate and
    /// arena stayed live under the other. Fixed at `plan_refresh`'s claim set (the first
    /// boundary in anchor order claims the proc; the other half mints a new one), which is
    /// where the live/condemned asymmetry belongs — not here.
    ///
    /// ★ **The same key is what makes RECOVERY possible**, which is the half a user
    /// actually experiences. An application that re-initialises allocates a **new** RM
    /// client; that client is on the live side of the condemnation line, so no dup edge
    /// of its can merge it into a condemned component ([`crate::project`]'s grouping
    /// predicate) — it is a different component, is not condemned, and gets a live
    /// `Proc` with its own isolate and arena. And an application that simply dies needs
    /// no cooperation at all: the guest kernel frees its clients on its behalf, the
    /// entry loses them, and it is dropped. "Sticky-fatal, not bricked" is therefore a
    /// property of this field's type, not a promise made elsewhere.
    ///
    /// Entries are **canonical**: pairwise disjoint and sorted (determinism, decision
    /// #27). They grow over the component that earned the condemnation — a split-then-
    /// rejoined corpse keeps ONE entry — and they **shrink to the clients the graph
    /// still knows**; an entry is dropped when the last of them goes, i.e. when the
    /// **guest itself** freed the client roots.
    ///
    /// ★★ §12.37 (C2) — **and the shrink is load-bearing, not housekeeping.** The
    /// carry-forward used to re-add the *whole* old entry on every refresh, including
    /// handles the guest had since freed and which by then existed nowhere in the graph.
    /// Handle reuse is explicit design ([`RmGraph`]'s resource/handle split exists for
    /// it, and `drop_handle` prunes the client-root index on free precisely so a
    /// namespace can be re-declared), so a retained dead handle **poisons a VALUE**: an
    /// attacker joins N clients into one component, gets one worker killed, frees N−1 of
    /// them and keeps one alive for nothing, and any later, unrelated process whose
    /// `hClient` lands on a freed value is condemned the moment it declares. That
    /// contradicts this field's own rule two paragraphs down — *"it must never reach a
    /// client that did not [share the blast radius]"* — because **a recycled handle value
    /// never shared the blast radius.** Intersecting the carried entry with the clients
    /// the projection still sees keeps the growth over *live* clients (so the evasions
    /// below all still fail) while letting dead handle values fall out.
    condemned: Vec<BTreeSet<ClientId>>,
    /// ★ §12.13 — data-plane routing for condemned components: `(GpuId, Pdb)` → the
    /// condemned component's label. Derived exactly like [`Self::by_pdb`], from the
    /// same projection, so naming the fault costs **no reverse resolution** (the
    /// doctrine `RmGraph::gpu_of` and the address table are held to): the guest's key
    /// is looked up forward, it just resolves to "condemned" instead of to a proc.
    condemned_by_pdb: BTreeMap<(GpuId, Pdb), ProcAnchor>,
    /// ★ §12.13 — exec-plane routing for condemned components: `(GpuId, VChid)` → the
    /// condemned component's label. See [`Self::condemned_by_pdb`].
    condemned_by_vchid: BTreeMap<(GpuId, VChid), ProcAnchor>,
    /// ★★★ **E0b/E1 — how many isolates this device has ever materialized**, across
    /// every proc and every target, monotonically.
    ///
    /// It exists because E0b turned *"an isolate exists"* from a realize-time certainty
    /// into a **guest-caused event**, and an event nothing counts is one that can only
    /// be diagnosed by absence. A boot with zero here and a boot whose isolates all
    /// refuse look identical from inside the guest — and, before this, identical from
    /// outside it too (see [`Spine::isolate_census`]).
    ///
    /// ⊘ It is **not** the E0b acceptance instrument, and must never be cited as one:
    /// it is produced by the code under test, so it can say *whether* a spawn happened
    /// and never *why*. The attributing instrument is
    /// `scripts/bench/e0_isolate_witness.sh`, which stamps host `/proc` sightings
    /// against `boot_capture.sh`'s own phase lines — a timeline this device does not
    /// write.
    isolates_materialized: u64,
}

/// ★★★ **E1 — a clonable handle onto [`Gpu::isolate_census`]'s answer.**
///
/// Exists for exactly the reason [`kayfabe_rmrpc::SharedRefusalCensus`] does, and the
/// shape is deliberately the same one: the policy that owns the [`Gpu`] is installed as a
/// `Box<dyn CommandPolicy>` and is unreachable from the composition root the moment it is
/// boxed, so a census reachable only through `&self` could be read by a test and by
/// nothing else — which is the *"diagnosed by absence"* failure that motivated the
/// refusal census in the first place.
///
/// ⊘ **Published, never accreted.** Each publish REPLACES the value; there is no adding
/// up. The census is a *level* (how many isolates exist, how many refuse) with one
/// monotonic total carried inside it, and a handle that summed would double-count every
/// command.
///
/// [`kayfabe_rmrpc::SharedRefusalCensus`]: https://docs.rs/kayfabe-rmrpc
#[derive(Debug, Clone, Default)]
pub struct SharedIsolateCensus(std::sync::Arc<std::sync::Mutex<IsolateCensus>>);

impl SharedIsolateCensus {
    /// A fresh handle whose census is the empty one — `materialized: 0`, no isolates.
    #[must_use]
    pub fn new() -> SharedIsolateCensus {
        SharedIsolateCensus::default()
    }

    /// A point-in-time copy.
    #[must_use]
    pub fn snapshot(&self) -> IsolateCensus {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Replace the published census with `c`.
    pub fn publish(&self, c: IsolateCensus) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = c;
    }
}

/// Union-find root with path halving, over condemned-entry indices (§12.22).
fn uf_find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// Insert `set` into the canonical condemned list, absorbing (to a fixpoint) every
/// entry it overlaps, then re-sorting so the list is a deterministic function of the
/// graph and not of call order (decision #27). Keeping the entries pairwise disjoint is
/// what makes "does this boundary intersect a condemned component" a single scan with
/// no transitive follow-up.
///
/// ## ★ **Absorb, never widen** — and how that is actually held (§12.37)
///
/// An entry must never reach a client that did not share the blast radius, because
/// condemnation's whole defensibility is that it is **sticky-fatal rather than
/// bricked**: a component is dead because the guest's data in *that* isolate's host
/// memory is gone (`RmBackend::alloc_sysmem`, freed with the process), and an unrelated
/// or freshly-minted client's data is not. Widening would turn a recoverable Xid-shaped
/// fault into a device that refuses everything after one worker crash.
///
/// That used to be a claim about this function's fixpoint, and the fixpoint could not
/// keep it — because *what* it absorbed came from [`Spine::refresh`]'s boundaries, and a
/// boundary could contain a client that the dead component never shared anything with:
/// a guest that dupped across the condemnation line dragged a live client in (C1), and
/// a carried entry retained handle values the guest had freed and which a *future*
/// process would be handed (C2). It is now held by two structural facts instead:
///
/// 1. **[`crate::project`]'s condemnation line** makes every component homogeneous —
///    a boundary is wholly condemned or wholly live — so the sets that arrive here can
///    only ever be clients that were already on the dead side.
/// 2. **The carry-forward intersects with the clients the graph still knows**, so an
///    entry cannot outlive the identities that earned it. Growth is over *live* clients
///    only.
fn absorb_condemned(list: &mut Vec<BTreeSet<ClientId>>, mut set: BTreeSet<ClientId>) {
    loop {
        let mut rest = Vec::with_capacity(list.len());
        let mut grew = false;
        for c in list.drain(..) {
            if c.is_disjoint(&set) {
                rest.push(c);
            } else {
                set.extend(c);
                grew = true;
            }
        }
        *list = rest;
        if !grew {
            break;
        }
    }
    list.push(set);
    list.sort();
}

/// The device: composition root of the logic core.
///
/// `kayfabe_vmm::Device` will be implemented once the register/GSP models port
/// (`kayfabe-regs`-equivalent + `kayfabe-gsp`) — but **by the L1 shell, not by this
/// type**: `Device`'s entry points take `&self` so the port admits per-`Proc` sharding,
/// and the type that owns the ranked locks is `kayfabe_rt::SharedDevice`. This
/// milestone exposes the event-level API the adapters and tests drive.
///
/// `Send + Sync` (compile-time-asserted; concurrency contract, crate docs): share
/// `&Gpu` across vCPU threads for lock-free reads (`kayfabe-fwd::resolve`,
/// `gate_working_set`, routing lookups); mutation (`apply`, doorbells, completion
/// pumping) takes `&mut self` under caller-provided exclusivity.
///
/// **The core-shape sharding split (`l1_concurrency.md` §3.4):** this type is now
/// just the L0 bundle of the three separately-lockable parts — the device-global
/// [`Spine`] (device-write-lock state), the [`Gpu::system`] proc, and the user
/// [`Gpu::procs`]. Its `&mut self` methods are thin split-borrow wrappers over the
/// [`Spine`] ops, for single-threaded callers (tests, the degenerate one-lock L1
/// configuration); the sharded L1 owns the parts itself and calls the `Spine` /
/// per-`Proc` entry points directly.
pub struct Gpu {
    /// The device-global spine (see [`Spine`]).
    pub spine: Spine,
    /// The system proc: kernel RM / scrubber / CeUtils traffic ([`crate::Traffic`]).
    pub system: Proc,
    /// Derived per-process containers.
    pub procs: BTreeMap<ProcId, Proc>,
}

/// Per-target device state (MG-6): each routable GPU has its OWN guest-physical
/// window (arenas) and its OWN completion drain gate over its OWN GSP queue — so a
/// batch outstanding on one GPU never gates another GPU's post (no cross-GPU
/// serialization).
pub struct GpuTarget {
    /// This target's guest-physical window (hands out this target's per-proc arenas).
    pub gpa: GpaSpace,
    /// This target's completion posting policy (drain gate over ITS GSP queue).
    pub delivery: DeliveryPlane,
    /// The arch this target was realized under — MG-6's homogeneity guard compares it
    /// against the device's one arch; a mismatch is a loud [`GpuError::HeterogeneousArch`].
    arch_name: &'static str,
}

/// The window geometry a new [`GpuTarget`] is minted with — cloned from the
/// realize-time `GpaSpace` so every target gets an identically-sized but **disjoint**
/// guest-physical window (a real multi-GPU guest sees each GPU's BAR window at a
/// distinct GPA).
#[derive(Debug, Clone, Copy)]
struct TargetGeom {
    window_len: u64,
    arena_len: u64,
    /// Base of the NEXT target's window (starts past `GpuId::ZERO`'s window end).
    next_base: u64,
}

/// ★ What one [`Spine::refresh`] has already decided (and already carved) before it
/// mutates anything — see [`Spine::plan_refresh`] for why this type exists at all.
/// Every field is index-aligned with [`Boundaries::procs`].
struct RefreshPlan {
    /// `None` = a condemned boundary (it gets nothing). `Some(v)` = the live procs
    /// whose clients intersect it, ascending: `v[0]` survives a merge, `v[1..]` are
    /// absorbed, empty = mint a fresh proc.
    /// One entry per **user** boundary.
    matches: Vec<Option<Vec<ProcId>>>,
    /// ★★ §12.35 — **every `Proc` this refresh will remove from the live set**, decided
    /// here (before a single proc is touched) rather than discovered mid-mutation:
    /// the procs absorbed by a merge, plus the procs whose component vanished. Ascending
    /// and deduplicated, so the removal pass is deterministic (decision #27).
    ///
    /// It exists so that removal can be **one pass in one place**, which is what makes
    /// `decide → stage → drain → remove` structural. It used to be two: step 1 removed
    /// the absorbed procs inline, step 3 computed the vanished ones from `live` *after*
    /// the mutation, and neither staged anything.
    vanishing: Vec<ProcId>,
    /// Every GPU target that boundary's proc spans. ★ §12.27: length is
    /// `bounds.procs.len() + 1` — the LAST entry is the **system** component, which has
    /// no `matches` entry (it is never matched, merged, minted or condemned) but spans
    /// targets and needs arenas exactly like a user proc.
    spans: Vec<BTreeSet<GpuId>>,
    /// Arenas carved for the spanned targets its proc does not already hold. Same
    /// length and same last-entry-is-system convention as [`Self::spans`].
    arenas: Vec<BTreeMap<GpuId, GpaArena>>,
    /// Targets minted for this refresh (already carved from), ready to install.
    new_targets: BTreeMap<GpuId, GpuTarget>,
    /// `geom.next_base` once they are installed.
    next_base: u64,
}

impl Spine {
    /// The Axis-B behavior this device was realized with — **read-only**. There is no
    /// setter, and that absence is load-bearing: see the `arch` field's comment.
    #[must_use]
    pub fn arch(&self) -> &dyn Arch {
        self.arch.as_ref()
    }

    /// ★★★★★ §16.50 — a **snapshot** of `gpu`'s published global context-buffer physicals,
    /// taken at rank 0 for [`crate::promote::apply_promote_ctx`] to join against.
    ///
    /// ★ A clone, and it costs nothing to be one: the map is bounded by the three
    /// [`crate::promote::PhysHalfScope::PerGpu`] ids, so it holds **at most three
    /// entries**. Cloning is what lets the sharded shell read this under the device read
    /// lock, release it, run the join under the owning proc's lock alone, and merge back —
    /// preserving R3's rank order instead of nesting rank 0 inside rank 1.
    #[must_use]
    pub fn global_ctx_phys_for(&self, gpu: GpuId) -> crate::promote::GlobalCtxPhys {
        self.global_ctx_phys.get(&gpu).cloned().unwrap_or_default()
    }

    /// ★★★★★ §16.50 — merge a join's (possibly extended) snapshot back into the
    /// device-global map. Returns how many publications were **newly** recorded.
    ///
    /// ⊘ **First publication wins, and a differing one is DROPPED here rather than
    /// overwriting.** The refusal for a differing re-publication lives in
    /// [`crate::promote::apply_promote_ctx`], which sees the promotion and can name it
    /// ([`crate::promote::PromoteFault::HalfConflict`]); by the time a snapshot reaches
    /// this function the promotion has already been answered, so silently retargeting
    /// contexts that already joined against the old value is the only thing left to
    /// prevent. The window is real but narrow — it needs two promotions racing on one
    /// `buffer_id` with different physicals — and it resolves to *"the loser's
    /// publication did not take"*, never to a wrong binding.
    pub fn merge_global_ctx_phys(
        &mut self,
        gpu: GpuId,
        snapshot: &crate::promote::GlobalCtxPhys,
    ) -> u32 {
        let live = self.global_ctx_phys.entry(gpu).or_default();
        let mut added = 0u32;
        for (id, half) in snapshot {
            if let std::collections::btree_map::Entry::Vacant(slot) = live.entry(*id) {
                slot.insert(*half);
                added += 1;
            }
        }
        added
    }

    /// ★★★ Whose page table is the physical address `phys` part of, on `gpu`?
    ///
    /// **The operand split's one question** (`#102`/#13). A copy-engine command whose
    /// *resolved physical destination* lands on a tracked page-table page is a
    /// **phys-operand** command: its payload is guest-physical PTE values, which cannot be
    /// handed to hardware, so it must be intercepted and translated. Anything else is a
    /// **VA-operand** command whose operands the host MMU resolves for itself once the
    /// address space is resident — forward it, do not intercept it.
    ///
    /// `None` is the ordinary answer and it means *forward*, not *fault*: the overwhelming
    /// majority of copies are data. It is [`AddressTable::resolve`] that is MISS = FAULT,
    /// and it runs first — an unresolvable destination faults there, before this is asked.
    ///
    /// [`AddressTable::resolve`]: kayfabe_mmu::AddressTable::resolve
    ///
    /// ★ E8: **declared first, then discovered.** A root that is also present in
    /// [`Self::pt_learned`] answers from [`Self::pt_roots`], because a PDB *is* its root
    /// page and no decode can be more authoritative about that than the declaration.
    #[must_use]
    pub fn pt_page_owner(&self, gpu: GpuId, phys: u64) -> Option<(ProcId, Pdb)> {
        let key = (gpu, phys & !0xfff);
        // ★ A contested page is indexed for NOBODY — see `Spine::pt_contested`. Checked
        // after roots, because a declared root is never contested: a PDB is its own root.
        if !self.pt_roots.contains_key(&key) && self.pt_contested.contains(&key) {
            return None;
        }
        self.pt_roots
            .get(&key)
            .or_else(|| self.pt_learned.get(&key))
            .copied()
    }

    /// ★★★ E8 **PUBLISH** (spine op, rank 0, and the fourth phase of the decode pass):
    /// install pages a decode learned into [`Self::pt_learned`].
    ///
    /// **R5 — re-validate after re-acquiring.** Every ranked lock was released between the
    /// decode's commit and this call, so `(gpu, pdb)` is re-resolved through
    /// [`Self::by_pdb`] and a page is published **only** if that address space still routes
    /// to `pid`. A `Vas` that died, or one whose PDB value has been recycled onto a
    /// different proc, publishes nothing — which is the same recyclable-value rule
    /// [`Self::ctx_vas`] states, applied to a physical page instead of a handle.
    ///
    /// ⊘ Deliberately **not** idempotent-by-overwrite: a page already owned by a
    /// *different* `(pid, pdb)` is refused rather than re-homed. Two address spaces
    /// claiming one physical page-table page means either the guest is aliasing framebuffer
    /// across processes or our decode is wrong, and quietly letting the last writer win is
    /// how the C's table came to attribute a page to whoever touched it most recently.
    ///
    /// Returns `(published, refused)` — refused counts both the capacity ceiling and the
    /// ownership conflict, and the ceiling half is also accumulated into
    /// [`Self::pt_learned_refused`].
    pub fn publish_pt_pages(
        &mut self,
        pid: ProcId,
        gpu: GpuId,
        pdb: Pdb,
        pages: impl IntoIterator<Item = u64>,
    ) -> (usize, usize) {
        // R5: the address space must still be this proc's. `SYSTEM_PROC` never appears in
        // `by_pdb` under a user anchor, so the system proc's own page tables route through
        // the same check rather than around it.
        if self.by_pdb.get(&(gpu, pdb)) != Some(&pid) {
            let n = pages.into_iter().count();
            return (0, n);
        }
        let (mut published, mut refused) = (0usize, 0usize);
        for page in pages {
            let key = (gpu, page & !0xfff);
            // A declared root is never shadowed by a discovered row.
            if self.pt_roots.contains_key(&key) {
                continue;
            }
            // Already contested — indexed for nobody, and sticky so this path is idempotent.
            if self.pt_contested.contains(&key) {
                refused += 1;
                continue;
            }
            match self.pt_learned.get(&key) {
                Some(&owner) if owner == (pid, pdb) => continue,
                Some(_) => {
                    // ★ DECLINE, do not pick a winner: evict the incumbent too and mark the
                    // page contested, which is exactly what `refresh`'s projection derives.
                    // Keeping the incumbent would make this path first-by-arrival while the
                    // projection is first-by-`ProcId` — the disagreement E8 shipped.
                    self.pt_learned.remove(&key);
                    self.pt_contested.insert(key);
                    refused += 1;
                    continue;
                }
                None => {}
            }
            if self.pt_learned.len() >= MAX_PT_LEARNED {
                refused += 1;
                self.pt_learned_refused += 1;
                continue;
            }
            self.pt_learned.insert(key, (pid, pdb));
            published += 1;
        }
        (published, refused)
    }

    /// Ensure target `gpu` exists (minting its disjoint window + drain gate on first
    /// touch, MG-6), enforcing the homogeneous-arch invariant loudly.
    fn ensure_target(&mut self, gpu: GpuId) -> Result<(), GpuError> {
        if let Some(t) = self.targets.get(&gpu) {
            if t.arch_name != self.arch.name() {
                return Err(GpuError::HeterogeneousArch {
                    gpu,
                    expected: self.arch.name(),
                    got: t.arch_name,
                });
            }
            return Ok(());
        }
        let base = self.geom.next_base;
        let end = base
            .checked_add(self.geom.window_len)
            .ok_or(GpuError::Gpa(GpaError::WindowExhausted))?;
        self.geom.next_base = end;
        self.targets.insert(
            gpu,
            GpuTarget {
                gpa: GpaSpace::new(base..end, self.geom.arena_len).owned_by(gpu),
                delivery: DeliveryPlane::new(),
                arch_name: self.arch.name(),
            },
        );
        Ok(())
    }

    /// Ensure `proc` has a GPA arena for target `gpu` (MG-5: per-`(Proc, GpuId)`) —
    /// **and deliberately NOT an isolate**. Idempotent; disjoint by construction. The
    /// caller resolves which proc (a user proc or the system proc) — the spine never
    /// reaches into the proc set itself here.
    ///
    /// ★★★ **E0b: the arena and the isolate were one act and are now two**
    /// (`execution_plane_increments.md` §3.6). The arena is *address-space* bookkeeping —
    /// it costs a range out of a window and nothing else, it is the thing realize can
    /// legitimately fail on, and carving it early is what keeps
    /// [`GpuError::Gpa`] a realize-time refusal instead of a surprise on the guest's
    /// first RPC. The isolate is a **host process**: `clone` into six namespaces,
    /// `execveat` a sealed memfd, a blocking hello handshake, and — under
    /// `KAYFABE_ISOLATES=real` — real `NV01_ROOT_CLIENT`/`NV01_DEVICE_0` ioctls on the
    /// host GPU. Doing that at realize meant every one of those verbs was caused by
    /// **QEMU realizing a device**, 28 seconds before the guest existed:
    /// `[measured]` rev `e10a6bf`, runs `e0real2`/`e0real3` — child first sighting
    /// **t+3 s**, guest device open **t+30–34 s**.
    ///
    /// [`Spine::materialize_isolate`] is now the only site that spawns, and
    /// [`Spine::apply`] is the only caller — i.e. a guest RM event.
    fn ensure_proc_arena(&mut self, proc: &mut Proc, gpu: GpuId) -> Result<(), GpuError> {
        self.ensure_target(gpu)?;
        let target = self.targets.get_mut(&gpu).expect("ensured above");
        if let std::collections::btree_map::Entry::Vacant(e) = proc.arenas.entry(gpu) {
            e.insert(target.gpa.carve()?);
        }
        proc.targets.insert(gpu);
        Ok(())
    }

    /// ★★★ **Decide that `proc` needs an isolate for `gpu` — the ONE decision site**
    /// (E0b), and since the R1 fix **only** the decision.
    ///
    /// ## ⊘ Why this no longer spawns, and what broke when it did
    ///
    /// This runs under the device WRITE lock: [`Spine::apply`] is a spine op and
    /// [`Spine::refresh`] is inside it. Spawning here therefore ran `clone` into six
    /// namespaces, `execveat` of a sealed memfd and a blocking hello handshake — and,
    /// under `KAYFABE_ISOLATES=real`, real host RM ioctls — **with rank 0 held**. That is
    /// R1's violation exactly, and it was not a theoretical one: it aborted QEMU on the
    /// first guest register write that reached a `GSP_RM_ALLOC`
    /// (`docs/reference/bench_evidence/f0b7efa_run_basereal_qemu.log`, and the same panic
    /// at the base revision, so it is pre-existing rather than caused by E6).
    ///
    /// So the decision is latched ([`Spine::pending_spawns`] for promptness,
    /// [`Proc::pending_isolates`] for durability) and the SPAWN happens in the shell with
    /// zero locks held, landing back through [`Spine::install_isolate`], which
    /// re-validates (R5).
    ///
    /// Idempotent and **infallible**, which is not a convenience: step 3b installs
    /// isolates on a path §12.18 made infallible-by-construction, and neither E0b nor this
    /// may smuggle a failure channel back into it.
    ///
    /// Returns `true` if this call is the one that decided — i.e. the pair was neither
    /// materialized nor already pending.
    fn defer_isolate(&mut self, proc: &mut Proc, gpu: GpuId) -> bool {
        if proc.isolates.contains_key(&gpu) || !proc.pending_isolates.insert(gpu) {
            return false;
        }
        self.pending_spawns.push(proc.id, gpu);
        true
    }

    /// ★★ **Take the latched spawns** for the shell to discharge with no lock held (R1) —
    /// the exact counterpart of [`Spine::take_pending_cancels`], and of
    /// [`crate::reactor::SourceRegistry::take_pending_wake`].
    ///
    /// The caller MUST have released every ranked lock before spawning:
    /// [`kayfabe_isolate::IsolateBox::new`] asserts it, because a sandbox spawn is a
    /// blocking syscall. Returning them rather than performing them here is what makes
    /// that assert satisfiable instead of a rule someone has to remember.
    pub fn take_pending_spawns(&mut self) -> PendingSpawns {
        core::mem::take(&mut self.pending_spawns)
    }

    /// ★ A handle on the isolate factory, usable with **no lock held** — the second half
    /// of the deferral (see [`Spine::take_pending_spawns`]).
    ///
    /// A clone of the one `Arc` this device was realized with; there is no second factory
    /// and no way to install one.
    #[must_use]
    pub fn isolate_factory(&self) -> Arc<dyn IsolateFactory> {
        Arc::clone(&self.isolates)
    }

    /// ★★★ **Install a spawned isolate — and RE-VALIDATE first (R5).**
    ///
    /// The commit half of the deferral. `iso` was spawned with every guard dropped, so
    /// between the decision and this call the world moved: §12.9's compare-and-swap, with
    /// the two staleness shapes it names.
    ///
    /// - **Divergent** — the proc retired, or stopped wanting an isolate for this target.
    ///   [`Proc::wants_isolate`] is false; the sandbox is **surplus**.
    /// - **Converging** — a sibling thread materialized the same `(proc, gpu)` first
    ///   (both raced the same [`kayfabe_fwd::FwdFault::IsolatePending`], which is the
    ///   ordinary case for a multi-threaded guest). The marker is already cleared, so
    ///   `wants_isolate` is false here too and the loser's sandbox is surplus. ⊘ The
    ///   loser is **not** refused: nothing it was doing has failed, it simply re-plans
    ///   against the winner's isolate.
    ///
    /// The caller's proc must be the LIVE `PendingSpawn::proc`; a proc that left the live
    /// set in the gap never reaches here and its sandbox is surplus by the same rule.
    ///
    /// ⊘ **Returns the surplus rather than dropping it**, and that is not fastidiousness:
    /// [`kayfabe_isolate::IsolateBox`]'s `Drop` is `waitpid` + namespace teardown + fd
    /// close, which R1 forbids under the write lock this runs beneath — the very gap
    /// §12.16 G3b closed for the reap. The caller drops it after its guards fall, and the
    /// `IsolateBox` assert says so if it does not.
    ///
    /// `Ok(())` means installed and counted; `Err(surplus)` means it must not be.
    pub fn install_isolate(
        &mut self,
        proc: &mut Proc,
        gpu: GpuId,
        iso: IsolateBox,
    ) -> Result<(), IsolateBox> {
        if !proc.wants_isolate(gpu) {
            return Err(iso);
        }
        proc.pending_isolates.remove(&gpu);
        // Belt and braces against a future caller that clears the marker without filling
        // the slot: an occupied slot is a winner, and overwriting it would drop a live
        // isolate — here, under the write lock.
        if proc.isolates.contains_key(&gpu) {
            return Err(iso);
        }
        proc.isolates.insert(gpu, iso);
        self.isolates_materialized = self.isolates_materialized.saturating_add(1);
        Ok(())
    }

    /// Apply one RM protocol event and re-sync all derived state to the graph.
    ///
    /// **Atomic under hostile input (boundary-1, decision #9).** The derived state is
    /// a *global* projection of the graph (`by_pdb`/`by_vchid` route the whole device),
    /// so an event that makes the graph unprojectable — e.g. a hostile process
    /// declaring two VASpaces with the same PDB (`PdbCollision`) or two channels that
    /// decode to one vChid (`VchidCollision`) — must NOT be allowed to wedge the device
    /// for *every other* process. This apply is therefore transactional: on ANY
    /// derivation fault the graph mutation is **rolled back** and the device is
    /// re-derived from its last-good graph, so the offending event is refused
    /// atomically and no other `Proc`'s state is disturbed. A hostile stream can only
    /// ever earn its own loud refusal.
    ///
    /// ★★ **The "own refusal" half needed a second mechanism, and had not got it
    /// (§12.37).** Atomicity says a refused event moves nothing; it says nothing about
    /// *whose* event gets refused, or about an event that is accepted and still kills a
    /// bystander. A guest could plant a `DUP_OBJECT` into a client namespace that had
    /// never been allocated; the edge was inert, and it fired on the victim's own
    /// `Alloc(Client, User)` — the same apply that first creates the victim's boundary,
    /// so the victim had no live `Proc`, the condemned-merge refusal did not trigger,
    /// and `apply` returned `Ok(())` having condemned the victim permanently, anchored
    /// at the attacker's client. Rollback was working perfectly and was beside the
    /// point. What holds the sentence now is [`crate::project`]'s **condemnation line**:
    /// a dup never merges across it, so a component's fate is never transferable by
    /// another component's event, and the attacker's planted edge earns the attacker
    /// nothing at all.
    ///
    /// ★ **How that is actually held, corrected (`l1_concurrency.md` §12.18).** The
    /// sentence above used to be a claim this function did not keep: the snapshot is of
    /// `self.rmgraph` **only**, while [`Spine::refresh`] also retires and removes
    /// `Proc`s, deregisters completion sources, pushes to [`Spine::retired`], advances
    /// [`Spine::next_proc`], mints [`Spine::targets`] and carves GPA arenas — none of
    /// which the restore undid. A fault raised after an earlier victim was already
    /// retired left it dead, and the re-derivation minted it afresh (new [`ProcId`],
    /// newly spawned isolate, newly carved arena); when the fault was arena exhaustion
    /// the re-derivation could not even *get* an arena back and the rollback's own
    /// `expect` **panicked**, taking the device down. The two mutators are now separated
    /// and each holds the property for a stated reason:
    ///
    /// - **[`Spine::refresh`] — atomic by construction, not by undo.** Every refusal is
    ///   hoisted into [`Spine::plan_refresh`], which decides (and pre-carves) before any
    ///   proc is touched; the mutation pass that follows has no failure path left.
    /// - **[`Spine::sync_rpc_mappings`] — atomic by RE-RUN, which is weaker and is why
    ///   it is said out loud.** It has no plan: a fault can leave the *offending* proc's
    ///   own `Vas` carrying bindings the failed pass already installed. What removes them
    ///   is the re-run below, whose stale-unbind pass drops every `rpc_bound` VA the
    ///   last-good graph no longer desires and re-binds every one it does. The restored
    ///   table is therefore equal in content, and the residue is confined to the proc
    ///   whose event it was — a bystander's table is never reached, because one event
    ///   changes the mapping set of one `(GpuId, Pdb)`.
    ///
    /// **Concurrency shape (L1):** a spine op — runs under the device *write* lock,
    /// with exclusive access to every proc (`system` + the [`ProcSet`]).
    pub fn apply(
        &mut self,
        system: &mut Proc,
        procs: &mut impl ProcSet,
        ev: RmEvent,
    ) -> Result<(), GpuError> {
        // Snapshot the last-good graph so a faulting derivation can be undone.
        //
        // ★ §12.23 — this clone was evaluated for deletion after §12.18 made `refresh`
        // infallible, and it **stays**, for a specific reason: three faults are raised
        // AFTER `RmGraph::apply` has already mutated the graph, and none is pre-computable
        // without the post-event graph — `project`'s `PdbCollision`/`VchidCollision`,
        // `plan_refresh`'s `LateMerge`, and `sync_rpc_mappings`'
        // `UnbackedMapping`/`Address`. A single `Alloc` can promote arbitrarily many
        // parked facts, so the post-state is not a local function of the event. Without
        // the rollback the offending fact stays and EVERY subsequent apply re-faults — a
        // permanent control-plane wedge for every other process.
        //
        // The old note here said a clone is "off the performance-critical path". Measured
        // (§12.23): it is ~24% of a control plane that is O(live objects) **per event**
        // end to end — `project` and `sync_rpc_mappings` are the other two O(graph)
        // passes — so N events cost O(N²). That quadratic is a named, deferred finding;
        // the clone is not what causes it.
        let snapshot = self.rmgraph.clone();
        match self.apply_inner(system, procs, ev) {
            Ok(()) => {
                // ★★★ E0b — THE LAZY SPAWN, and the only unconditional one in the device.
                //
                // The system proc is the guest **kernel**'s objects, and this is the point
                // at which a guest RM event has been accepted. Materializing here rather
                // than in `Gpu::realize` is the whole of E0b: the isolate — a `clone`d,
                // namespaced host child that, under `KAYFABE_ISOLATES=real`, holds
                // `/dev/nvidiactl` and an RM-served `/dev/nvidia0` mapping — now exists
                // **because the guest allocated something**, and a device that is realized
                // and never driven spawns nothing at all.
                //
                // ⊘ On the Ok path only. A refused event moves nothing else (this function
                // is transactional); it must not be the one thing that buys a guest a host
                // process either. And a boot whose every event is refused therefore shows
                // ZERO children — which is a *distinguishable* outcome, not a silent one:
                // `Spine::isolates_materialized` is 0 and the audit says so.
                //
                // ⊘ It does NOT cover the per-`(Proc, GpuId)` isolates of *user* procs;
                // those are step 3b's `or_insert_with`, which was already event-driven and
                // is unchanged. This covers exactly the one isolate `realize` used to
                // create behind the guest's back.
                //
                // ★★★ **R1: the DECISION is here, the SPAWN is not** (see
                // [`Spine::defer_isolate`]). This function runs under the device write
                // lock, so materializing here was a blocking call under rank 0 — the
                // panic `f0b7efa_run_basereal_qemu.log` records. The count moves with the
                // *installation*, in [`Spine::install_isolate`], because "the guest caused
                // a host process to exist" is a fact about the spawn landing, not about
                // our intention to make one.
                self.defer_isolate(system, GpuId::ZERO);
                Ok(())
            }
            Err(e) => {
                // Undo: restore the last-good graph and re-derive from it. That graph
                // projected cleanly before this event (every prior apply upheld the
                // same invariant, inductively), so re-derivation cannot fault.
                self.rmgraph = snapshot;
                self.refresh(system, procs)
                    .expect("last-good graph re-projects");
                self.sync_rpc_mappings(system, procs)
                    .expect("last-good graph re-syncs");
                Err(e)
            }
        }
    }

    /// The non-atomic apply body: mutate the graph, re-derive, forward-populate. Any
    /// error here leaves derived state possibly half-updated — [`Self::apply`] wraps it
    /// to guarantee all-or-nothing.
    fn apply_inner(
        &mut self,
        system: &mut Proc,
        procs: &mut impl ProcSet,
        ev: RmEvent,
    ) -> Result<(), GpuError> {
        self.rmgraph.apply(self.arch.as_ref(), ev)?;
        self.refresh(system, procs)?;
        // Forward-populate the address table from the RPC map source (co-equal with
        // the CE-PT-write capture source — `mode2_address_table.md`). Bindings track
        // the graph's live mappings; unmap eagerly depopulates.
        self.sync_rpc_mappings(system, procs)?;
        Ok(())
    }

    /// Sync each `Vas`'s address table to the graph's live DMA mappings (the RPC
    /// populate source). Idempotent: binds mappings not yet in the table, unbinds
    /// table entries whose mapping is gone.
    ///
    /// ★ **This function is where the miss taxonomy is most visible, and the old doc
    /// here was simply wrong** (`kayfabe_core` crate docs; `l1_concurrency.md` §12.30).
    /// It used to claim "MISS=FAULT is preserved — a mapping with no resolvable PDB or
    /// backing is a loud fault, never a silent skip", while the body deliberately
    /// `continue`s on both an unresolved PDB and an unresolved target. The body is right
    /// and the sentence was wrong; the three misses split as:
    ///
    /// - **no PDB yet ⇒ DEFER.** The guest legitimately issues `MAP_MEMORY_DMA` before
    ///   `SET_PAGE_DIRECTORY`; the fact may still arrive, and this sync re-runs when it
    ///   does. This is the founding exception the whole taxonomy is written around.
    /// - **no resolvable target (no `Device` ancestor) yet ⇒ DEFER.** Same reasoning, on
    ///   the multi-GPU axis: deferring is what keeps `GpuId::ZERO` from being guessed.
    /// - **the memory resolved and has NO backing ⇒ FAULT**
    ///   ([`GpuError::UnbackedMapping`]). A backing is an alloc-time fact, so an unbacked
    ///   memory stays unbacked: this one is *never knowable* and is refused by name.
    ///
    /// Idempotence is what makes the two deferrals sound: "deferred" means the answer
    /// changes when the fact lands, never that the mapping was dropped.
    fn sync_rpc_mappings(
        &self,
        system: &mut Proc,
        procs: &mut impl ProcSet,
    ) -> Result<(), GpuError> {
        // Desired: (GpuId, pdb, va) -> (len, phys) for every live mapping with a
        // resolved target + PDB. The target is derived from the mapping's VAS `Device`
        // ancestor — a `Pdb` is per-GPU, so the binding must be keyed by target too.
        let mut desired: BTreeMap<(GpuId, u64, u64), (u64, u64)> = BTreeMap::new();
        for m in self.rmgraph.mappings() {
            let Some(pdb) = m.pdb else {
                // ★ DEFER (not yet knowable). A mapping whose VAS has no PDB yet is not
                // routable — deferred until SET_PAGE_DIRECTORY arrives (which re-runs
                // this sync). Not a fault: the guest legitimately maps before binding the
                // page directory. THE canonical exception to MISS=FAULT.
                continue;
            };
            let Some(gpu) = self.rmgraph.gpu_of_resource(m.vaspace) else {
                // ★ DEFER (not yet knowable). No resolvable target (no Device ancestor)
                // — deferred until the Device fact lands, never guessed onto GPU 0.
                continue;
            };
            // ★ FAULT (never knowable): the memory resolved and declared no backing, and
            // a backing is an alloc-time fact — no future event supplies one.
            let phys = m
                .mem_phys
                .ok_or(GpuError::UnbackedMapping { pdb, va: m.va.0 })?;
            desired.insert((gpu, pdb.0, m.va.0), (m.len, phys));
        }

        for (_, proc) in procs.iter_mut() {
            Self::sync_proc_rpc_bindings(&desired, proc)?;
        }
        Self::sync_proc_rpc_bindings(&desired, system)?;
        Ok(())
    }

    /// Sync ONE proc's `Vas` tables to the desired RPC-mapping set (the per-proc
    /// half of [`Self::sync_rpc_mappings`]).
    fn sync_proc_rpc_bindings(
        desired: &BTreeMap<(GpuId, u64, u64), (u64, u64)>,
        proc: &mut Proc,
    ) -> Result<(), GpuError> {
        use kayfabe_arch::Aperture;
        use kayfabe_mmu::Binding;

        for (&(gpu, pdb), vas) in proc.vases.iter_mut() {
            // Unbind stale RPC bindings (mapping gone), leaving host-backed
            // publish_backing entries (`Binding::host = Some`) alone.
            let stale: Vec<u64> = vas
                .rpc_bound
                .iter()
                .filter(|&&va| !desired.contains_key(&(gpu, pdb.0, va)))
                .copied()
                .collect();
            for va in stale {
                vas.table.unbind(GpuVa(va));
                vas.rpc_bound.remove(&va);
            }
            // Bind newly-declared mappings for this (target, PDB).
            for (&(mgpu, mpdb, va), &(len, phys)) in desired.iter() {
                if mgpu != gpu || mpdb != pdb.0 || vas.rpc_bound.contains(&va) {
                    continue;
                }
                // ★★★ THE DECISION, at the bind site: an RPC-declared mapping is SYSMEM,
                // so `phys` is a guest-physical address and the bytes are the guest's own
                // pages — the owner's **kind 4, DMA-to-guest-physical**. ⊘ The aperture is
                // a literal and not data here, so the one refusal
                // `declared_by_guest` can raise (`Aperture::Peer`) is unreachable from this
                // site; it is stated rather than swallowed.
                let binding = Binding::declared_by_guest(phys, Aperture::SysmemCoherent)
                    .expect("a sysmem aperture literal is kind 4 — `Peer` is not reachable here");
                vas.table
                    .bind(pdb, GpuVa(va), len, binding)
                    .map_err(GpuError::Address)?;
                vas.rpc_bound.insert(va);
            }
        }
        Ok(())
    }

    /// ★ Sync ONE `Proc`'s derived fields to its boundary — the *only* place a
    /// projection becomes runtime `Proc` state, shared by the user procs (step 2) and by
    /// the system proc (step 2s, §12.27). Extracted rather than duplicated because the
    /// guest kernel's channels must materialize by exactly the same rules as a guest
    /// process's: a second copy is how the two drift.
    fn sync_proc_to_boundary(p: &mut Proc, b: &ProcBoundary, ids: &BTreeMap<ClientKey, ClientId>) {
        p.anchor = b.anchor;
        p.clients = b.clients.clone();
        // ★★★ §12.39 Part B — record the component's namespace IDENTITIES alongside its
        // labels, from the declarations this very projection was taken against. A client
        // in a boundary always has a current declaration (that is what put it there), so a
        // missing id would be an internal inconsistency; it is simply skipped rather than
        // panicked on, and the effect is that the proc no longer matches that namespace —
        // the safe direction (a fresh `Proc`, never an inherited one).
        p.client_ids = b
            .clients
            .iter()
            .filter_map(|c| ids.get(c).copied())
            .collect();
        // Vases: create for newly-declared (GpuId, PDB); drop ones no longer
        // derived. Only vases with a resolvable target AND a declared PDB become
        // runtime `Vas`es. ★ DEFER (not yet knowable): an unroutable one materializes
        // nothing and is re-evaluated on the next apply; its USE takes a named
        // `FwdFault::UnknownPdb`, which is where MISS=FAULT bites.
        let live_keys: BTreeSet<(GpuId, Pdb)> = b
            .vases
            .values()
            .filter_map(|f| Some((f.gpu?, f.pdb?)))
            .collect();
        for (&origin, facts) in &b.vases {
            if let (Some(gpu), Some(pdb)) = (facts.gpu, facts.pdb) {
                p.vases
                    .entry((gpu, pdb))
                    .or_insert_with(|| Vas::new(gpu, pdb, origin));
            }
        }
        // ★★ T0/G2 — FILL BEFORE YOU DROP (`l1_os_shell.md` §7.6 T0).
        Self::stage_dropped_vases(p, &live_keys);
        p.vases.retain(|key, _| live_keys.contains(key));
        // Channels: stable ChanId per node key. A channel whose GPU target does
        // not resolve is NOT materialized (the same deferral as the Vas pattern
        // above): it enters no routing map, so a runtime `Channel` would be inert
        // — and tagging it `GpuId::ZERO` would be a default-target guess (the
        // no-GPU0-guess doctrine). Its ChanId is still minted, so its slot is
        // stable for when the Device fact lands and it materializes.
        // ★★★★★ The guest-facing kind of EVERY channel this boundary owns, resolved ONCE
        // for the whole component rather than per channel: it is a property of whose
        // namespace allocated them, and this loop is the only place a `Channel` is built.
        // See `crate::project::ProcBoundary::channel_kind`.
        let kind = b.channel_kind();
        for (&key, facts) in &b.channels {
            let cid = *p.chan_ids.entry(key).or_insert_with(|| {
                let c = ChanId(p.next_chan);
                p.next_chan += 1;
                c
            });
            let Some(gpu) = facts.gpu else {
                // ★ DEFER (not yet knowable) — unroutable until its Device fact lands,
                // never guessed onto GPU0. Its ChanId is still minted (above), so the
                // slot is stable for the apply that materializes it.
                continue;
            };
            let entry = p.channels.entry(cid).or_insert_with(|| Channel {
                id: cid,
                key,
                gpu,
                vchid: facts.vchid,
                kind,
                vas_pdb: facts.vas_pdb,
                vas_origin: facts.vas_origin,
                vas_device_default: facts.vas_device_default,
                vas_route: facts.vas_route,
                engine: facts.engine,
                host_channel: None,
                host_token: None,
                // ★ A fresh channel's engine has nothing bound and nothing latched. Not
                // refreshed below with the other fields: `refresh` re-derives DECLARED
                // protocol facts, and this is accumulated ENGINE state — re-deriving it
                // would clear a channel's operands every time an unrelated alloc landed.
                method_state: kayfabe_arch::MethodState::new(),
                host_engine_objects: BTreeMap::new(),
                error_notifier: facts.error_notifier,
            });
            entry.gpu = gpu;
            entry.vchid = facts.vchid;
            // ★★★★★ Re-assigned with the other declared facts, never on its own pass —
            // see [`Channel::kind`] for why it cannot actually change, and for why it is
            // assigned anyway.
            entry.kind = kind;
            entry.vas_pdb = facts.vas_pdb;
            // ⊘ Refreshed with `vas_pdb`, never separately: the two are one resolution
            // (`project::resolve_channel_vas` produces both), and letting them refresh on
            // different passes would recreate the disagreement this field exists to end.
            entry.vas_origin = facts.vas_origin;
            // ★★★★ §16.28 — on the SAME pass, for the same reason as `vas_origin` above:
            // it is the fourth answer of the one resolution that produced the other three.
            entry.vas_device_default = facts.vas_device_default;
            // ★★★★ §16.25 — on the SAME pass, for the same reason as `vas_origin` above.
            entry.vas_route = facts.vas_route;
            entry.engine = facts.engine;
            entry.error_notifier = facts.error_notifier;
        }
        let live_chans: BTreeSet<ResourceKey> = b.channels.keys().copied().collect();
        p.chan_ids.retain(|key, _| live_chans.contains(key));
        let live_cids: BTreeSet<ChanId> = p.chan_ids.values().copied().collect();
        // ★★ T0/G2 — the exec plane's half of the same rule.
        Self::stage_dropped_channels(p, &live_cids);
        p.channels.retain(|cid, _| live_cids.contains(cid));
        p.exec.scheduled.retain(|cid| live_cids.contains(cid));
        // ★ #177 — the guest-intent half of the same rule. A freed channel's `ChanId` is
        // mintable again, and an intent that outlived its channel would schedule a
        // *different* channel that never asked.
        p.exec.requested.retain(|cid| live_cids.contains(cid));
        // ★★★ The ring cursor's half, and it is the same rule for a *sharper* reason. A
        // stale `requested` schedules a channel that never asked; a stale `forwarded`
        // makes the re-minted channel's first N entries look **already run**, so its first
        // submission is skipped in silence. Dropped work, no refusal, no name — `#13`'s
        // `CE-DROP` by handle reuse.
        p.exec.forwarded.retain(|cid, _| live_cids.contains(cid));
    }

    /// ★★ **T0/G2, the address plane** (`l1_os_shell.md` §7.6 T0): move the host
    /// identities of every [`Vas`] this refresh is about to drop into the proc's
    /// `pending_release` queue, and return their [`GpaBlock`]s to the proc's own arena —
    /// **before** `retain` makes both unrecoverable.
    ///
    /// Ordering is **unmap-then-free**, and that is RM's rule rather than a preference:
    /// `clientFreeResource_IMPL` auto-unmaps a resource's inter-mappings before
    /// `objDelete` (`ogkm-610:`/`ogkm-580:`
    /// `src/nvidia/src/libraries/resserv/src/rs_client.c:830-849` — same path, same lines,
    /// byte-identical at both tags), so RM itself leaks nothing — but *our* external mirror of those mappings (the address
    /// table's [`kayfabe_mmu::HostBacking`]) goes stale, which is why [`Orphans`] states
    /// the unmaps first and means it. Within `free`, the memory objects mapped into a
    /// host VAS precede the VAS itself, matching RM's children-before-parents order
    /// (`ogkm-610:`/`ogkm-580: .../rs_server.c:963-981`, also identical at both).
    ///
    /// **The GPA half runs right here, under the lock, and that is correct**: returning a
    /// block to `GpaArena` issues no host verb, so R1 does not apply to it — only the
    /// disposal of the host objects has to wait for a worker. Keeping the two in one
    /// place is deliberate for the same reason [`kayfabe_fwd::unpublish_backing`] returns
    /// the GPA *with* its orphans in one call: a GPA recycled while its host memory is
    /// still mapped is the ALREADY-MAPPED class. Here the mapping is queued for release
    /// and the GPA is only reusable by **this same proc**, whose next publication will
    /// map it into a host VAS of its own — so the pair stays consistent.
    ///
    /// A block whose arena is gone or has been re-carved is refused by
    /// [`GpaArena::free`] ([`crate::gpa::ForeignBlock`], keyed on [`crate::gpa::ArenaId`]'s
    /// generation) and stays out of circulation, which is the safe direction: a stale
    /// range re-entering a live free list is the #14 collision class.
    fn stage_dropped_vases(p: &mut Proc, live: &BTreeSet<(GpuId, Pdb)>) -> PinReclaim {
        let mut tally = PinReclaim::default();
        let doomed: Vec<(GpuId, Pdb)> = p
            .vases
            .keys()
            .filter(|k| !live.contains(k))
            .copied()
            .collect();
        for key in doomed {
            let (gpu, _pdb) = key;
            let mut vas = p.vases.remove(&key).expect("just enumerated");
            let host_vas = vas.host_vas;
            let q = p.pending_release.entry(gpu).or_default();
            // ★★★★★ **w310 — THE GUEST-RAM PINS, AND THEY GO FIRST.**
            //
            // `docs/audits/w301_cancellation_error_leaks.md` §3.2: this function *"walks
            // `vas.table` and `vas.blocks` only — it never consults `guest_ram_pins`"*, and
            // dropping the `Vas` *"loses the handles entirely, so the objects become
            // unnameable."* That is the leak, and this block is the removal it needed.
            //
            // # ★★★ THE PIN IS THE UNIT OF RECLAIM, and that is the whole design
            //
            // Not the row. `commit_pin_guest_ram`'s merge writes `Binding::host` **only for
            // an exact-extent row**, and says why: *"one handle written into N rows would be
            // freed N times — a DOUBLE FREE … strictly worse than the leak this closes. A
            // pin whose grant spans several rows therefore binds NOTHING here and behaves
            // exactly as before."* ⇒ a row-driven reclaim structurally **cannot** see a
            // multi-row run pin, and *"exactly as before"* is the leak. Walking
            // `guest_ram_pins` sees every pin, of both shapes, exactly once.
            //
            // # ⚠ HOW MANY TIMES IS THE HOST OBJECT FREED? **Exactly one.** Two facts:
            //
            // 1. `guest_ram_pins` is **injective on `memory`** — one entry per successful
            //    `PinGuestRam`, each of which minted its own `OS_DESCRIPTOR`, and
            //    `overlapping_pin` refuses a second entry over the same range.
            // 2. `pinned_objects` below carries the staged handles into the row walk, which
            //    **skips** any row whose object a pin already staged. That is the exact-extent
            //    overlap w291 created, and it is closed here rather than by weakening the
            //    merge — removing the merge's bound would reintroduce the double free it
            //    exists to prevent, which is the worse direction.
            //
            // ⊘ Ordering: pins are pushed **before** `q.free.extend(host_vas)` below, so the
            // VAS is still freed last. `Orphans` runs all `unmap`s, then all `free`s, then
            // all `guest_ram` `munmap`s — see [`kayfabe_isolate::Orphans::guest_ram`].
            let mut pinned_objects: BTreeSet<HostHandle> = BTreeSet::new();
            for (_va, pin) in vas.take_guest_ram_pins() {
                match classify_pin_release(true, host_vas) {
                    PinReleaseVerdict::Release(hv) => {
                        q.unmap.push((hv, pin.host_va));
                        q.free.push(pin.memory);
                        q.guest_ram.push(pin.mapped);
                        pinned_objects.insert(pin.memory);
                        tally.released += 1;
                    }
                    // ⊘ Refused by name. The pin's disposition falls back to what it always
                    // was — the isolate's death frees the whole client namespace (§7.0) —
                    // which is a *different disposition*, not a failure, and is counted so it
                    // can never be a silent zero.
                    PinReleaseVerdict::RefusedNoHostVas => tally.refused_no_host_vas += 1,
                    // Unreachable here by construction: this function only ever runs over a
                    // `Vas` it has already removed. Left as an arm rather than an `expect`
                    // so the enum stays total and the refusal keeps its one meaning.
                    PinReleaseVerdict::RefusedVasLive => tally.refused_no_host_vas += 1,
                }
            }
            for (_va, _len, binding) in vas.table.iter() {
                // `Binding::host == None` is an RPC-declared binding: nothing host-side
                // exists, so nothing host-side needs reclaiming.
                let Some(h) = binding.host() else { continue };
                // ★★★ **w310 — THE DEDUPE, and it is the double-free door.** w291's merge
                // upgrades an exact-extent row to carry the PIN's own `memory` handle, so
                // this row and the pin above name one object. The pin already staged it
                // (unmap + free + `munmap`); staging it again here would `free` a live host
                // object twice. See this function's pin block for the full argument.
                if pinned_objects.contains(&h.memory()) {
                    tally.rows_deduped += 1;
                    continue;
                }
                // The unmap is conditional on the VAS and the free is not, deliberately:
                // a published binding implies its `Vas` materialized a host VAS, but if
                // that ever stopped holding, the memory object must still be freed rather
                // than silently skipped along with the unmap it has no target for.
                if let Some(host_vas) = host_vas {
                    q.unmap.push((host_vas, h.host_va()));
                }
                // ★★ …but the free is conditional on the EXTENT
                // (`gpga_address_space.md` §8.2/§9.3). One arena object backs many
                // bindings, so an unconditional push would queue the SAME handle once
                // per slice — a double free of a live object, with the sibling slices
                // still mapped. `frees_object()` is the one place that question is
                // asked; the arena is disposed of by its own owner, not by the Vas that
                // happened to die first.
                if h.frees_object() {
                    q.free.push(h.memory());
                }
            }
            // …and the host VAS last: everything mapped into it is freed first.
            q.free.extend(host_vas);
            if let Some(arena) = p.arenas.get_mut(&gpu) {
                for (_va, block) in core::mem::take(&mut vas.blocks) {
                    // A refusal returns the block; dropping it there is the conservative
                    // direction (the range simply never comes back).
                    drop(arena.free(block));
                }
            }
        }
        p.pin_reclaim.absorb(tally);
        tally
    }

    /// ★★ **T0/G2, the exec plane** — the [`Channel`] half of [`Self::stage_dropped_vases`].
    ///
    /// `host_engine_objects` are freed **before** `host_channel`: an engine object is
    /// allocated *on* the channel, so it is the child, and RM frees children ahead of
    /// parents. `host_token` needs no entry — it is not a handle, it is the work-submit
    /// doorbell token the channel object owns, and it dies with it.
    fn stage_dropped_channels(p: &mut Proc, live: &BTreeSet<ChanId>) {
        let doomed: Vec<ChanId> = p
            .channels
            .keys()
            .filter(|c| !live.contains(c))
            .copied()
            .collect();
        for cid in doomed {
            let ch = p.channels.remove(&cid).expect("just enumerated");
            let q = p.pending_release.entry(ch.gpu).or_default();
            q.free.extend(ch.host_engine_objects.into_values());
            q.free.extend(ch.host_channel);
        }
    }

    /// Which GPU targets one boundary's proc spans: the targets of its routable VASes
    /// and of its routable channels. A pure read of the projection (never of what a
    /// previous refresh accreted), shared by the user boundaries and the system one.
    fn span_of(b: &ProcBoundary, bounds: &Boundaries) -> BTreeSet<GpuId> {
        let mut s: BTreeSet<GpuId> = BTreeSet::new();
        for f in b.vases.values() {
            if let (Some(gpu), Some(_pdb)) = (f.gpu, f.pdb) {
                s.insert(gpu);
            }
        }
        for f in b.channels.values() {
            if let Some(gpu) = f.gpu
                && bounds.by_vchid.contains_key(&(gpu, f.vchid))
            {
                s.insert(gpu);
            }
        }
        s
    }

    /// ★ **The decision half of [`Spine::refresh`]** — everything the mutation pass
    /// could have refused, settled from `bounds` + current state BEFORE a single
    /// `Proc` is touched (`l1_concurrency.md` §12.18).
    ///
    /// This exists because [`Spine::apply`]'s rollback only ever restored the
    /// [`RmGraph`]. Every *other* thing `refresh` mutates — procs retired and removed,
    /// [`Spine::retired`], [`Spine::sources`] deregistration, [`Spine::next_proc`],
    /// [`Spine::targets`], [`Spine::geom`] — had no undo, so a fault on a *later*
    /// boundary kept an *earlier* boundary's retirement, and the rollback's
    /// re-derivation minted that proc afresh: new [`ProcId`], newly spawned isolate,
    /// newly carved arena. That is one process's malformed event visibly disturbing
    /// another process's state — precisely the boundary-1/#14 isolation guarantee.
    ///
    /// Snapshotting the proc set is not the fix (a `Proc` owns isolates; it is neither
    /// cloneable nor cheap). Hoisting the refusals is: `project` is already pure and
    /// already runs first, so *all three* fault conditions are decidable up front —
    /// [`GpuError::LateMerge`], [`GpuError::HeterogeneousArch`], and GPA exhaustion.
    /// (There were four: `CondemnedMerge` was retired by §12.37, which made the merge it
    /// named unrepresentable instead of refusable.) With them hoisted the mutation pass
    /// has no `?` left in it and atomicity is **structural, not claimed**.
    ///
    /// The last step is the only one that touches live state: it *carves* the arenas
    /// the mutation pass will hand out, because exhaustion is a property of the
    /// windows, not an arithmetic prediction about them. A failure there releases
    /// everything it carved — [`GpaSpace::release`] is exactly [`GpaSpace::carve`]'s
    /// inverse, so the windows' capacity and their set of ranges are restored (only
    /// the free list's LIFO order differs, which no invariant names).
    fn plan_refresh(
        &mut self,
        system: &Proc,
        procs: &mut impl ProcSet,
        bounds: &Boundaries,
        condemned_bound: &[bool],
        ids: &BTreeMap<ClientKey, ClientId>,
    ) -> Result<RefreshPlan, GpuError> {
        let n = bounds.procs.len();

        // (a) Merge legality, in boundary order — the refusal that used to fire
        //     mid-mutation.
        let mut matches: Vec<Option<Vec<ProcId>>> = Vec::with_capacity(n);
        // ★★★ §12.39 finding 3 — **THE SPLIT, which is the DUAL of the merge and was
        // never checked.** A merge is many procs → one boundary; a split is one proc →
        // many boundaries, and it is reached by ordinary legal guest behaviour: dup-join
        // two user clients into one component, then free the alias.
        //
        // Without this claim set, `matching` is a pure `p.clients ∩ b.clients` test, so
        // BOTH halves of a split matched the SAME live proc. `survivors` took `.first()`
        // of each match, so the proc was a survivor twice over and `vanishing` came out
        // **empty** — nothing staged for death — while step 2 ran `sync_proc_to_boundary`
        // on it once per half and the second call overwrote the first. One half silently
        // lost its clients, vases and channels; its `Pdb` left `by_pdb` entirely (its
        // anchor was no longer any live proc's), and its isolate and arena stayed live
        // under the other half. A later verb naming the lost half then found no proc and
        // minted a fresh one — the resurrect-into-a-dead-data-plane shape the no-resurrect
        // rule forbids. The merge guard (`matching.len() > 1`) structurally cannot see it,
        // because each boundary matches exactly ONE proc.
        //
        // ★ The rule, stated once: **a live `Proc` is claimed by the FIRST boundary (in
        // ascending anchor order) that intersects it; every other boundary that would
        // have matched it mints a NEW `Proc`.** `bounds.procs` is anchor-ordered and an
        // anchor is the component's smallest client, so the claimant is the half that
        // still holds the proc's own anchor whenever that client survives — the proc
        // keeps its identity rather than having it reassigned by iteration order. The
        // departing half's state leaves through the ordinary path and nothing else: step
        // 2's `sync_proc_to_boundary(keeper, b_first)` stages every vas/channel that is
        // not the keeper's into `pending_release`, exactly as a subset-free does, so it is
        // reclaimed **once**; and the new `Proc` starts empty — no isolate, no arena and
        // no host handle is inherited, because host state belongs to the RM client
        // namespace of the isolate that minted it and cannot be moved between them.
        //
        // ★ Absorbed procs are claimed too, not just keepers: an absorbed proc is on its
        // way out through `vanishing`, so a later boundary must not be able to adopt a
        // corpse.
        let mut claimed: BTreeSet<ProcId> = BTreeSet::new();
        for (b, &condemned) in bounds.procs.iter().zip(condemned_bound) {
            // ★★★ §12.39 Part B — the match is on namespace IDENTITY, not on the
            // `HClient` VALUES. An `hClient` is recyclable by RM's own design, so a
            // boundary belonging to a *re-declared* namespace intersects the previous
            // tenant's `Proc` on the value alone — and would have adopted it whole: its
            // isolate (one host RM client namespace), its GPA arena, its host VASes, its
            // `pending_release` queue. Comparing never-reused `ClientId`s makes the old
            // proc simply not match, so it leaves through `vanishing` (staged, exactly
            // once) and the new namespace mints a `Proc` of its own.
            let b_ids: BTreeSet<ClientId> = b
                .clients
                .iter()
                .filter_map(|c| ids.get(c).copied())
                .collect();
            let mut matching: Vec<ProcId> = procs
                .iter_mut()
                .filter(|(id, p)| !claimed.contains(id) && !p.client_ids.is_disjoint(&b_ids))
                .map(|(id, _)| id)
                .collect();
            matching.sort_unstable();
            claimed.extend(matching.iter().copied());
            if condemned {
                // ★★ §12.37 — a condemned boundary gets NOTHING, and `matching` is
                // necessarily empty, which is why this is not a refusal.
                //
                // It used to be `GpuError::CondemnedMerge`: a live proc could be dragged
                // into a condemned component by a `DUP_OBJECT`, and refusing the event
                // was the only honest answer available once the merge had already
                // happened in the projection. But the refusal was **only reachable when
                // the dragged-in side already had a live `Proc`** — plant the same dup
                // into a namespace that had not declared yet and the merge fired on the
                // victim's own `Alloc`, with no proc to protect and therefore in
                // silence. Whether a victim got a refusal or a silent death depended
                // only on the arrival order of its own client-root alloc.
                //
                // `crate::project`'s condemnation line removes the merge itself, so
                // there is no arm to make loud and no asymmetry left: a cross-line dup
                // is a *reference*, the corpse's resources keep answering
                // `FwdFault::Condemned` at their origin, and the live client keeps its
                // proc. A component is homogeneous, so no live proc can hold a client of
                // a condemned boundary.
                debug_assert!(
                    matching.is_empty(),
                    "a live proc held a client of a condemned boundary — the \
                     condemnation line leaked out of the grouping predicate"
                );
                matches.push(None);
                continue;
            }
            if let Some((&keep, absorbed)) = matching.split_first() {
                for &absorbed in absorbed {
                    if !procs
                        .get_mut(absorbed)
                        .expect("matched proc exists")
                        .is_untouched()
                    {
                        return Err(GpuError::LateMerge {
                            kept: keep,
                            absorbed,
                        });
                    }
                }
            } else {
                // ★ G10 (§12.22): this boundary would mint a NEW `Proc`. That is the only
                // guest-reachable action that consumes the two device-global lists, so it
                // is where the caps are enforced — refusing at the growth sites would
                // un-condemn a dead component or leave a worker-less proc live, both
                // strictly worse than backpressure. Recovery is the guest's own: free the
                // dead components' client roots and the entries prune.
                if self.condemned.len() >= MAX_CONDEMNED_COMPONENTS {
                    return Err(GpuError::SpineCapacity {
                        what: SpineCapacity::CondemnedComponents,
                        cap: MAX_CONDEMNED_COMPONENTS,
                    });
                }
                if self.retired.len() >= MAX_RETIRED_PROCS {
                    return Err(GpuError::SpineCapacity {
                        what: SpineCapacity::RetiredProcs,
                        cap: MAX_RETIRED_PROCS,
                    });
                }
            }
            matches.push(Some(matching));
        }

        // (b) Which targets each boundary's proc will span, and which of those its
        //     surviving proc does not already hold an arena for. Derived from the
        //     projection (the same facts step 3b read off the synced procs), so it is
        //     a function of the graph and not of what a previous refresh accreted.
        let mut spans: Vec<BTreeSet<GpuId>> = Vec::with_capacity(n + 1);
        let mut held: Vec<BTreeSet<GpuId>> = Vec::with_capacity(n + 1);
        for (b, m) in bounds.procs.iter().zip(&matches) {
            let s = if m.is_some() {
                Self::span_of(b, bounds)
            } else {
                BTreeSet::new()
            };
            let h: BTreeSet<GpuId> = m
                .as_ref()
                .and_then(|v| v.first())
                .map(|&pid| {
                    procs
                        .get_mut(pid)
                        .expect("matched proc exists")
                        .arenas
                        .keys()
                        .copied()
                        .collect()
                })
                .unwrap_or_default();
            spans.push(s);
            held.push(h);
        }
        // ★ §12.27 — the SYSTEM component rides the same planning, at index `n`. It is
        // never matched, merged, condemned or minted (it *is* `Gpu::system`), so it has
        // no `matches` entry — but the guest kernel's own VASpaces and channels give it
        // a target span exactly like a user proc's, and its arenas must be carved in the
        // same all-or-nothing pass or a GPA exhaustion on a kernel object would fault
        // half-way through the mutation (§12.18).
        spans.push(Self::span_of(&bounds.system, bounds));
        held.push(system.arenas.keys().copied().collect());

        // (c) MG-6 homogeneity, over every target that will be touched. (A target
        //     minted below carries the device's own arch by construction, so only
        //     pre-existing ones can disagree.)
        let arch_name = self.arch.name();
        let wanted: BTreeSet<GpuId> = spans.iter().flatten().copied().collect();
        let mut fresh: BTreeSet<GpuId> = BTreeSet::new();
        for gpu in wanted {
            match self.targets.get(&gpu) {
                Some(t) if t.arch_name != arch_name => {
                    return Err(GpuError::HeterogeneousArch {
                        gpu,
                        expected: arch_name,
                        got: t.arch_name,
                    });
                }
                Some(_) => {}
                None => {
                    fresh.insert(gpu);
                }
            }
        }

        // (d) Mint the new targets into staging. `geom.next_base` advances only in the
        //     plan's copy — the spine's own geometry moves when the plan is installed.
        let mut next_base = self.geom.next_base;
        let mut new_targets: BTreeMap<GpuId, GpuTarget> = BTreeMap::new();
        for gpu in fresh {
            let base = next_base;
            let end = base
                .checked_add(self.geom.window_len)
                .ok_or(GpuError::Gpa(GpaError::WindowExhausted))?;
            next_base = end;
            new_targets.insert(
                gpu,
                GpuTarget {
                    gpa: GpaSpace::new(base..end, self.geom.arena_len).owned_by(gpu),
                    delivery: DeliveryPlane::new(),
                    arch_name,
                },
            );
        }

        // (e) Carve every arena the mutation pass will hand out.
        let mut arenas: Vec<BTreeMap<GpuId, GpaArena>> =
            (0..spans.len()).map(|_| BTreeMap::new()).collect();
        let mut failed: Option<GpaError> = None;
        'carve: for i in 0..spans.len() {
            for &gpu in &spans[i] {
                if held[i].contains(&gpu) {
                    continue;
                }
                let space = match new_targets.get_mut(&gpu) {
                    Some(t) => &mut t.gpa,
                    None => &mut self.targets.get_mut(&gpu).expect("checked in (c)").gpa,
                };
                match space.carve() {
                    Ok(a) => {
                        arenas[i].insert(gpu, a);
                    }
                    Err(e) => {
                        failed = Some(e);
                        break 'carve;
                    }
                }
            }
        }
        if let Some(e) = failed {
            // Give back what was taken from a PRE-EXISTING window; the staged windows
            // die whole with `new_targets`.
            for m in arenas {
                for (gpu, a) in m {
                    if let Some(t) = self.targets.get_mut(&gpu) {
                        t.gpa
                            .release(a)
                            .expect("released into the window that carved it");
                    }
                }
            }
            return Err(GpuError::Gpa(e));
        }

        // (f) ★★ §12.35 — WHICH PROCS VANISH, decided here. A proc survives iff some
        //     boundary matched it *first* (`matching[0]` is the keeper); everything else
        //     in the live set is either absorbed by a merge or has had its component
        //     freed out from under it, and the two are the same event as far as removal
        //     is concerned: it leaves the live set and must be staged on the way out.
        //
        //     Derived from `matches`, which (a) already settled, so this adds no new
        //     refusal and no new failure path — it only *names*, before the mutation
        //     starts, the set the mutation used to discover halfway through.
        let survivors: BTreeSet<ProcId> = matches
            .iter()
            .filter_map(|m| m.as_ref().and_then(|v| v.first()).copied())
            .collect();
        let vanishing: Vec<ProcId> = procs
            .iter_mut()
            .map(|(id, _)| id)
            .filter(|id| !survivors.contains(id))
            .collect();

        Ok(RefreshPlan {
            matches,
            vanishing,
            spans,
            arenas,
            new_targets,
            next_base,
        })
    }

    /// ★★ **THE ONE REMOVAL POINT** (`l1_concurrency.md` §12.35) — the only place in
    /// `refresh` that takes a `Proc` out of the live set, and it **stages first**.
    ///
    /// The rule the owner stated, and the reason this is a function rather than a
    /// convention: *if you do something outside the locks, assume that on re-acquiring
    /// them the data may have changed underneath — including removal. Do the removal in
    /// a **central** place, after the real cleanup, and that out-of-order stops being
    /// possible.* §12.33 is what happens without it: `procs.remove(id)` + `retire()` with
    /// no `sync_proc_to_boundary`, so `stage_dropped_vases`/`stage_dropped_channels`
    /// never ran and the host VAS + backings were not even *queued*. From that instant
    /// the core could not name them, and every downstream door was already shut (a
    /// retired isolate refuses the disposal; the reap runs under the device write lock
    /// where R1 forbids a verb).
    ///
    /// Staging with **empty** live sets is exactly right and is not a special case: a
    /// vanished component has no live `(GpuId, Pdb)` and no live `ChanId`, so the general
    /// "everything the refresh is about to drop" pass drops *everything*. One mechanism,
    /// not a parallel reclamation path.
    ///
    /// Staging is pure bookkeeping (it moves handles into `pending_release` and returns
    /// `GpaBlock`s to the proc's own arena — no verb, so R1 does not apply), which is why
    /// it may run here under the device write lock. The **drain** cannot, and does not:
    /// it happens lock-free at the corpse's [`Proc::drop`], gated on `is_quiesced`.
    fn vacate(&mut self, procs: &mut impl ProcSet, id: ProcId) -> Proc {
        let mut p = procs
            .remove(id)
            .expect("a vanishing proc is in the live set");
        Self::stage_dropped_vases(&mut p, &BTreeSet::new());
        Self::stage_dropped_channels(&mut p, &BTreeSet::new());
        // ★★ **w310** — absorb before the proc leaves, or the tally dies with it. This is
        // the whole-proc arm; the live-proc VAS-death arm accumulates the same numbers into
        // the surviving `Proc` and is read through [`Spine::pin_reclaim`].
        self.pin_reclaim_gone.absorb(p.pin_reclaim);
        p.vases.clear();
        p.channels.clear();
        p.chan_ids.clear();
        p.exec.scheduled.clear();
        p.exec.requested.clear();
        p.exec.forwarded.clear();
        // ★ VACATE, not RETIRE: the isolates stay live so the queue just filled can
        // actually be disposed of. See `Proc::vacate` for the clean-vs-violent split.
        // ★★ §7.6 T2 — and it CANCELS: a guest process that exits (normally, killed, or
        // killed *while a verb is pending*) is all one path here, and the pending verb's
        // requester is gone. The latches ride out on `pending_cancels`.
        self.pending_cancels.absorb(p.vacate());
        p
    }

    /// Re-derive boundaries and sync `procs`/`by_pdb`/`by_vchid` to them.
    ///
    /// ★ **Every refusal is hoisted into [`Spine::plan_refresh`]** (§12.18): after the
    /// plan is taken, nothing below can fail, so a refused event disturbs no `Proc`.
    fn refresh(&mut self, system: &mut Proc, procs: &mut impl ProcSet) -> Result<(), GpuError> {
        // 0. ★ THE CONDEMNATION PASS (§12.13) — re-derive the condemned client sets
        //    against the NEW boundaries, BEFORE step 1 can mint anything. This is the
        //    whole fix: without it, an out-of-band-retired component reaches step 1's
        //    `None` arm and is handed a fresh `ProcId`, a freshly spawned isolate and a
        //    fresh arena — the respawn §7.3 forbids.
        //
        //    A boundary is condemned iff it INTERSECTS a condemned client set — the same
        //    predicate step 1 matches live procs on, so the two agree by construction.
        //    Intersecting boundaries also *grow* their entry (the component keeps the
        //    blast radius it earned), and an entry no boundary intersects is dropped:
        //    condemnation ends only when the guest itself frees the client roots.
        //
        //    ★ G10 (§12.22): the "does this boundary intersect a condemned component"
        //    question used to be a nested scan — O(|boundaries| × |condemned|) on EVERY
        //    apply, both factors guest-driven, which is a complexity DoS of the same
        //    species the graph's parked-map set was already hardened against. The entries
        //    are pairwise disjoint, so client → entry is a *function*: build it once and
        //    the pass becomes O(total clients · log n).
        //
        //    ★★★ §12.39 Part B: keyed on [`ClientId`], not on `HClient`. A condemned
        //    entry outlives the component that earned it by design (it clears only when
        //    the guest frees the roots), so an `HClient` in it is exactly the kind of
        //    retained recyclable value §12.37's C2 shrink was written to stop poisoning.
        //    C2 alone could not close it: an orphan resource of the dead component keeps
        //    its namespace in `known`, so the freed value never fell out — and the next
        //    process handed that number was condemned on arrival, a bystander death.
        //    A `ClientId` is never reused, so a retained dead one can never name a future
        //    namespace, and the shrink below is now purely a **capacity** rule
        //    ([`MAX_CONDEMNED_COMPONENTS`]) rather than a correctness one.
        let mut owner_of: BTreeMap<ClientId, usize> = BTreeMap::new();
        for (i, c) in self.condemned.iter().enumerate() {
            for &client in c {
                owner_of.insert(client, i);
            }
        }
        // The declaration every namespace currently projects under — the ONE place the
        // recyclable value is turned into a never-reused identity, read once and handed
        // to every consumer below so they cannot disagree.
        let ids: BTreeMap<ClientKey, ClientId> = self
            .rmgraph
            .client_declarations()
            .into_iter()
            .map(|(c, (id, _))| (c, id))
            .collect();

        // ★★ §12.37 (C1) — the projection is taken AGAINST the condemned client set,
        // because grouping is where a merge across the condemnation line has to be
        // refused (`crate::project`, "the condemnation line"). Deriving the boundaries
        // first and adjudicating the merge afterwards is what let a guest plant a
        // `DUP_OBJECT` into a namespace that had not declared yet and have it fire —
        // silently, since there was no live `Proc` to protect — on the victim's own
        // first RM event. With the line in the predicate every component is homogeneous,
        // so the question "is this boundary condemned?" has one answer for all its
        // clients and the merge is not a fault at all.
        //
        // ★★★ §12.42 — `project` takes the condemned set as [`ClientKey`]s, so the
        // translation from the stored never-reused [`ClientId`] happens here, once. It used
        // to hand `project` bare `HClient` VALUES on the argument that its predicate only
        // ever asked about live-rooted endpoints; with the predicate itself now stated over
        // declarations, that argument is no longer needed and the recyclable value never
        // enters the projection at all.
        let condemned_clients: BTreeSet<ClientKey> = ids
            .iter()
            .filter(|(_, id)| owner_of.contains_key(id))
            .map(|(&c, _)| c)
            .collect();
        let bounds: Boundaries = project(&self.rmgraph, self.arch.as_ref(), &condemned_clients)?;
        //
        //    ★ G10, the second half: the CARRY-FORWARD was worse than the scan. Each
        //    intersecting boundary called `absorb_condemned`, which drains and re-sorts
        //    the whole carried list — O(n² log n) per apply, with n guest-driven. Measured
        //    at the cap it dominated everything (a 1024-component test took 55 s).
        //
        //    The same answer, computed instead of searched: boundaries are pairwise
        //    disjoint and so are the entries, so two boundaries' merged sets can overlap
        //    ONLY by hitting a common entry. Union-find over entry indices, keyed by
        //    boundary, therefore yields exactly the fixpoint the repeated absorb was
        //    grinding out — in near-linear time, and with the identical result.
        let mut parent: Vec<usize> = (0..self.condemned.len()).collect();
        let mut condemned_bound: Vec<bool> = Vec::with_capacity(bounds.procs.len());
        let mut boundary_hit: Vec<Option<usize>> = Vec::with_capacity(bounds.procs.len());
        for b in &bounds.procs {
            let hits: BTreeSet<usize> = b
                .clients
                .iter()
                .filter_map(|c| ids.get(c))
                .filter_map(|id| owner_of.get(id).copied())
                .collect();
            // ★ §12.37: with the condemnation line in the grouping predicate a component
            // is homogeneous, so this is a property of the whole boundary — every client
            // of it is condemned, or none is. (`debug_assert`ed below, where the clients
            // are still in scope.)
            debug_assert!(
                hits.is_empty() || b.clients.iter().all(|c| condemned_clients.contains(c)),
                "a boundary mixed condemned and live clients — the condemnation line \
                 leaked out of the grouping predicate"
            );
            condemned_bound.push(!hits.is_empty());
            let mut it = hits.into_iter();
            let first = it.next();
            if let Some(first) = first {
                let root = uf_find(&mut parent, first);
                for i in it {
                    let r = uf_find(&mut parent, i);
                    if r != root {
                        parent[r] = root;
                    }
                }
            }
            boundary_hit.push(first);
        }
        // ★★ §12.37 (C2) — **the clients the graph still knows**, i.e. every client the
        // projection can still see: one that owns a live resource, or has declared a
        // root, or is a declared endpoint of a resolved dup. A carried entry is
        // intersected with this below, so a client handle the guest has FREED — which
        // exists nowhere in the graph, and which the guest kernel is free to hand to an
        // unrelated process next — falls out of the entry instead of poisoning the
        // VALUE forever. The system component is included deliberately: never
        // under-condemn because a namespace re-declared itself as the guest kernel's.
        let known: BTreeSet<ClientId> = bounds
            .procs
            .iter()
            .flat_map(|b| b.clients.iter())
            .chain(bounds.system.clients.iter())
            .filter_map(|c| ids.get(c).copied())
            .collect();
        // The surviving components: an entry with no surviving client is DROPPED —
        // condemnation ends only when the guest itself frees the client roots.
        //
        // Seeded from the intersecting BOUNDARIES' clients (all of which are in `known`
        // by construction, which is why no entry here can come out empty), then extended
        // with the old entries' clients that are still known.
        let mut carried_by_root: BTreeMap<usize, BTreeSet<ClientId>> = BTreeMap::new();
        for (b, hit) in bounds.procs.iter().zip(&boundary_hit) {
            if let Some(i) = *hit {
                let root = uf_find(&mut parent, i);
                carried_by_root
                    .entry(root)
                    .or_default()
                    .extend(b.clients.iter().filter_map(|c| ids.get(c).copied()));
            }
        }
        for i in 0..self.condemned.len() {
            let root = uf_find(&mut parent, i);
            if let Some(set) = carried_by_root.get_mut(&root) {
                set.extend(
                    self.condemned[i]
                        .iter()
                        .filter(|c| known.contains(c))
                        .copied(),
                );
            }
        }
        let mut carried: Vec<BTreeSet<ClientId>> = carried_by_root.into_values().collect();
        carried.sort();
        let condemned_anchors: BTreeSet<ProcAnchor> = bounds
            .procs
            .iter()
            .zip(&condemned_bound)
            .filter(|&(_, &dead)| dead)
            .map(|(b, _)| b.anchor)
            .collect();

        // 0b. ★ §12.18 — DECIDE EVERYTHING REFUSABLE FIRST. From here down there is no
        //     `?` and no early return: the mutation below cannot fail, so a refused
        //     event leaves every other `Proc` byte-for-byte as it was.
        let mut plan = self.plan_refresh(system, procs, &bounds, &condemned_bound, &ids)?;

        // 1. Match each boundary to existing procs by client intersection (decided in
        //    the plan — a condemned boundary is `None` and gets NOTHING: no `Proc`, no
        //    isolate spawn, no arena, no routing entry, since step 4 files its keys
        //    under the condemned maps instead. Its ops therefore miss — MISS=FAULT
        //    working as designed, named exactly (`FwdFault::Condemned`)).
        let mut live: BTreeSet<ProcId> = BTreeSet::new();
        let mut boundary_pid: Vec<Option<ProcId>> = Vec::with_capacity(bounds.procs.len());
        for (i, b) in bounds.procs.iter().enumerate() {
            let Some(matching) = plan.matches[i].take() else {
                boundary_pid.push(None);
                continue;
            };

            let pid = match matching.first() {
                Some(&keep) => {
                    // ★ §12.35: a merge's absorbed procs are NOT removed here any more.
                    // They are in `plan.vanishing` (they are not survivors), so step 3's
                    // single removal pass takes them — staged, like every other death.
                    // The plan already checked each of them untouched (the early-arm
                    // discipline), so their staging is empty; the point is that there is
                    // no second removal site to keep in step with the first.
                    keep
                }
                None => {
                    let id = ProcId(self.next_proc);
                    self.next_proc += 1;
                    // Per-(Proc, GpuId) isolates/arenas are materialized lazily below,
                    // once the proc's target set is derived (MG-5).
                    procs.insert(id, Proc::new(id, b.anchor));
                    id
                }
            };
            live.insert(pid);
            boundary_pid.push(Some(pid));

            // 2. Sync the proc's derived fields to the boundary.
            Self::sync_proc_to_boundary(procs.get_mut(pid).expect("live proc exists"), b, &ids);
        }

        // 2s. ★ §12.27 — sync the SYSTEM proc to the system component (every declared
        //     kernel client: UVM's session, RM's internal clients). Deliberately outside
        //     the loop above and outside `live`, because every lifecycle verb that loop
        //     performs is one the system proc must never be subject to: it is not minted
        //     (it exists from realize), not matched or merged (its membership is the
        //     declared `ClientKind`, not a client intersection), not retired by step 3,
        //     and not condemnable (§12.26 — condemning the guest driver's own component
        //     is device-fatal by definition). What it DOES get is identical: the same
        //     `sync_proc_to_boundary`, so a guest-kernel channel materializes by exactly
        //     the same rules as a guest process's.
        Self::sync_proc_to_boundary(system, &bounds.system, &ids);

        // 3. ★★ §12.35 — **THE ONE REMOVAL POINT**: every proc that leaves the live set
        //    leaves it here, through `Spine::vacate`, which stages before it removes.
        //    Absorbed-by-a-merge and component-vanished are the same event to this pass;
        //    the plan decided the set at (f), before step 1 touched anything.
        //
        //    `live` is asserted equal to the plan's survivors rather than recomputed:
        //    two derivations of "who is still here" is exactly the drift this
        //    centralisation removes.
        debug_assert!(
            plan.vanishing.iter().all(|id| !live.contains(id)),
            "the plan's vanishing set must be disjoint from the procs step 1 kept"
        );
        for id in core::mem::take(&mut plan.vanishing) {
            let p = self.vacate(procs, id);
            // Removal is also a REACTOR event (§6): the dying proc's completion sources
            // must stop routing the instant it stops existing, or a late signal resolves
            // onto a dead proc (F4). Handles are never reused, so any signal still in
            // flight for it now resolves to nothing — a loud `SourceFault`, never a
            // use-after-retire.
            self.sources.deregister_proc(id).latched();
            self.retired.push(p);
        }

        // 3b. ★ MG-5: install each live proc's per-(Proc, GpuId) isolate + arena for
        // every target it now spans (its vases' + routable channels' GPUs), and the
        // per-target windows (MG-6) minted for them. **Infallible by construction**
        // (§12.18): every window and every arena in the plan was already carved before
        // step 1 touched a proc, and `IsolateFactory::spawn` cannot fail.
        self.targets.append(&mut plan.new_targets);
        self.geom.next_base = plan.next_base;
        for (i, &pid) in boundary_pid.iter().enumerate() {
            let Some(pid) = pid else { continue };
            let mut carved = core::mem::take(&mut plan.arenas[i]);
            let p = procs.get_mut(pid).expect("live proc exists");
            for &gpu in &plan.spans[i] {
                // ★ E0b: routed through the one decision site so the census counts it. The
                // `or_insert_with` semantics are unchanged — this pass was already
                // event-driven and is not what E0b moves.
                //
                // ★★★ R1: and it DEFERS rather than spawns. This whole function runs under
                // the device write lock; see [`Spine::defer_isolate`].
                self.defer_isolate(p, gpu);
                if let Some(arena) = carved.remove(&gpu) {
                    let prev = p.arenas.insert(gpu, arena);
                    debug_assert!(
                        prev.is_none(),
                        "the plan carves only for targets the proc does not hold"
                    );
                }
                p.targets.insert(gpu);
            }
        }
        // ★ §12.27 — and the system proc's own targets, from the plan's last entry.
        // Same law (MG-5: one isolate + one arena per (Proc, GpuId)), same infallible
        // installation — the guest kernel's objects live on real targets too.
        {
            let mut carved = core::mem::take(plan.arenas.last_mut().expect("system entry"));
            let span = plan.spans.last().expect("system entry").clone();
            for gpu in span {
                // ★ E0b: same routing as the user-proc pass above, same reason. ★★★ And
                // the same R1 deferral.
                self.defer_isolate(system, gpu);
                if let Some(arena) = carved.remove(&gpu) {
                    let prev = system.arenas.insert(gpu, arena);
                    debug_assert!(
                        prev.is_none(),
                        "the plan carves only for targets the proc does not hold"
                    );
                }
                system.targets.insert(gpu);
            }
        }

        // 4. Rebuild routing maps from the projection (never accreted). Keyed on the
        // target (MG-3): a `Pdb`/`VChid` is a per-GPU namespace. ★ §12.13: a key whose
        // owning component is CONDEMNED is filed under the condemned maps instead of
        // being dropped, so the guest's own key still resolves — forward, from the same
        // projection — to a named refusal rather than to an anonymous miss.
        self.by_pdb.clear();
        self.by_vchid.clear();
        self.by_chan.clear();
        self.pt_roots.clear();
        // ★★★ E8: the discovered half is re-derived here too, and clearing it is the whole
        // reason `publish_pt_pages` may run early without becoming accretion.
        self.pt_learned.clear();
        self.pt_contested.clear();
        self.ctx_vas.clear();
        self.condemned_by_pdb.clear();
        self.condemned_by_vchid.clear();
        let anchor_to_pid: BTreeMap<ProcAnchor, ProcId> =
            procs.iter_mut().map(|(id, p)| (p.anchor, id)).collect();
        // ★ §12.27: `SYSTEM_ANCHOR` resolves to the system proc *by name*, and it can
        // never collide with a user component's anchor because `RmGraph::apply` refuses
        // `RESERVED_CLIENT` as guest input. A guest-kernel PDB therefore routes — to the
        // proc whose data plane `plan_publish` refuses (§12.26), which is a named
        // refusal instead of an anonymous `UnknownPdb`.
        for (&(gpu, pdb), &(anchor, _)) in &bounds.by_pdb {
            if anchor == SYSTEM_ANCHOR {
                self.by_pdb.insert((gpu, pdb), Gpu::SYSTEM_PROC);
            } else if let Some(&pid) = anchor_to_pid.get(&anchor) {
                self.by_pdb.insert((gpu, pdb), pid);
            } else if condemned_anchors.contains(&anchor) {
                self.condemned_by_pdb.insert((gpu, pdb), anchor);
            }
            // ★★★ The page-table ROOT, from the same projection ([`Spine::pt_roots`]).
            // A PDB is the physical address of its root page directory, so the root is a
            // *declared* fact and the operand split gets a non-empty index before any
            // decoding exists. A CONDEMNED component's root is deliberately excluded: its
            // page tables must not attract capture (the C re-checked ownership at decode
            // time for exactly this, `C: :8574-8586`).
            if let Some(&pid) = self.by_pdb.get(&(gpu, pdb)) {
                self.pt_roots.insert((gpu, pdb.0 & !0xfff), (pid, pdb));
            }
        }
        // ★★★ E8 — the DEEPER levels of the same ownership index, projected from every
        // live `Vas::pt_meta` exactly as the roots above are projected from
        // `bounds.by_pdb`. This pass is what lets [`Spine::publish_pt_pages`] run early at
        // the decode's commit point without the index becoming accreted state: anything
        // published for an address space that no longer projects simply fails to reappear.
        //
        // ★ Filtered through the freshly-rebuilt `by_pdb`, so a CONDEMNED component's
        // learned pages drop out on the same edge its root does — its page tables must not
        // attract capture, and the C re-checked ownership at decode time for exactly this
        // (`C: :8574-8586`).
        let mut learned: Vec<((GpuId, u64), (ProcId, Pdb))> = Vec::new();
        {
            let mut project = |pid: ProcId, p: &Proc, by_pdb: &BTreeMap<(GpuId, Pdb), ProcId>| {
                for (&(gpu, pdb), vas) in &p.vases {
                    if by_pdb.get(&(gpu, pdb)) != Some(&pid) {
                        continue;
                    }
                    for &page in vas.pt_meta.keys() {
                        learned.push(((gpu, page & !0xfff), (pid, pdb)));
                    }
                }
            };
            project(Gpu::SYSTEM_PROC, system, &self.by_pdb);
            for (pid, p) in procs.iter_mut() {
                project(pid, p, &self.by_pdb);
            }
        }
        // ★★★ Fold the claims to ONE owner per key, or to CONTESTED. This must produce
        // exactly what `Spine::publish_pt_pages` produces incrementally — that equality is
        // `pt_index_publish_equals_projection`, and E8's first cut FAILED it: this loop used
        // `entry().or_insert()` (lowest `ProcId` wins, silently) while publish refused the
        // second arrival, so `pt_page_owner` flipped its answer across a refresh.
        //
        // ⊘ Neither "first" rule is implementable on both paths — publish cannot know a
        // future claimant and the projection has no arrival order — so both DECLINE.
        let mut claim: BTreeMap<(GpuId, u64), (ProcId, Pdb)> = BTreeMap::new();
        for (key, owner) in learned {
            // A declared root is never shadowed by a discovered row (`pt_page_owner`
            // prefers roots anyway; not inserting keeps the maps disjoint so a census can
            // add their sizes).
            if self.pt_roots.contains_key(&key) {
                continue;
            }
            if self.pt_contested.contains(&key) {
                continue;
            }
            match claim.get(&key) {
                Some(&held) if held == owner => {}
                Some(_) => {
                    claim.remove(&key);
                    self.pt_contested.insert(key);
                }
                None => {
                    claim.insert(key, owner);
                }
            }
        }
        for (key, owner) in claim {
            // ⊘ The ceiling does NOT increment `pt_learned_refused` here. This pass is a
            // RE-derivation of pages already counted when they were first offered, so
            // counting again would make the diagnostic grow on every RM graph event and
            // stop meaning "how many publications were turned away".
            if self.pt_learned.len() >= MAX_PT_LEARNED {
                continue;
            }
            self.pt_learned.insert(key, owner);
        }
        for (&(gpu, vchid), &(anchor, key)) in &bounds.by_vchid {
            if anchor == SYSTEM_ANCHOR {
                if let Some(&cid) = system.chan_ids.get(&key) {
                    self.by_vchid.insert((gpu, vchid), (Gpu::SYSTEM_PROC, cid));
                    self.by_chan.insert(key, (Gpu::SYSTEM_PROC, cid));
                }
            } else if let Some(&pid) = anchor_to_pid.get(&anchor) {
                if let Some(&cid) = procs
                    .get_mut(pid)
                    .expect("anchored proc lives")
                    .chan_ids
                    .get(&key)
                {
                    self.by_vchid.insert((gpu, vchid), (pid, cid));
                    self.by_chan.insert(key, (pid, cid));
                }
            } else if condemned_anchors.contains(&anchor) {
                self.condemned_by_vchid.insert((gpu, vchid), anchor);
            }
        }
        // ★★ The context-object index ([`Spine::ctx_vas`]), from the same projection.
        //
        // ★ It is copied WHOLE, deliberately. A `by_pdb`-membership filter was written
        // here first — to keep a condemned component's context objects out — and it was
        // then measured — by bite-check, poisoning the filter and running the suite — to
        // be **dead defence**: `promote::route_promote_ctx` looks the
        // resulting `(gpu, pdb)` up in `by_pdb` anyway, so a condemned VAS already
        // answers `ContextVasUndeclared` with or without it. Poisoning the filter failed
        // no test; poisoning the route's lookup fails one immediately. A guard never seen
        // to fail is not evidence, so the redundant one is gone and the exclusion lives
        // in exactly one place.
        self.ctx_vas
            .extend(bounds.ctx_vas.iter().map(|(&r, &v)| (r, v)));

        // 5. ★ Commit the re-derived condemnation LAST: every early return above is a
        // faulting derivation that `Spine::apply` rolls back, and a rolled-back apply
        // must find the condemned state exactly as it was (the re-derivation from the
        // last-good graph runs this same pass again).
        self.condemned = carried;
        Ok(())
    }

    /// ★ Retire proc `pid` **out of band** — the teardown edge that does NOT come
    /// from the RM protocol (`l1_concurrency.md` §7.3, the worker-death path).
    ///
    /// The graph-driven retirements inside [`Spine::refresh`] happen because the
    /// guest freed a client root. This one happens because the *host side* failed: an
    /// isolate worker died, and with it went the host memory its RM client owned — which
    /// is where the guest's published data lived. Same obligations, in the same order,
    /// deliberately
    /// sharing this one implementation rather than being open-coded in the adapter:
    /// remove it from the live set, `Proc::retire` it (its isolates stop, new ops
    /// refuse), deregister **every** completion source it owns (or a signal still in
    /// flight resolves onto a dead proc — the C's F4 species), and park it on the
    /// retired list for the deferred reap.
    ///
    /// ★ **And CONDEMN its component (§12.13 — this is what makes "never a resurrect"
    /// true).** Removing the proc is not enough: the *guest's* client root is untouched
    /// in the graph, so the next [`Spine::refresh`] — triggered by any event from any
    /// client — re-derives that boundary, matches no live proc, and used to mint a fresh
    /// `ProcId` with a **freshly spawned isolate** (new sandbox, new handle namespace)
    /// and a fresh GPA arena. Recording the component's client set as condemned makes
    /// the derivation skip it for good; see [`Spine::condemned_pdb`] for what its ops
    /// get instead.
    ///
    /// Condemnation **immediately** moves this proc's `by_pdb`/`by_vchid` entries into
    /// the condemned maps rather than waiting for the next refresh, which settles the
    /// §12.11 question ("should a host-side failure retroactively edit the guest's
    /// routing truth?") in the only way that keeps the two teardown routes
    /// fault-identical: the guest's *truth* — the RM graph and its projection — is never
    /// touched; what changes is the host-side **materialization**, and it changes at the
    /// instant of the failure rather than at the next unrelated event. Without this the
    /// same op would answer `RetiredProc` before the next `apply` and `Condemned` after
    /// it, for no reason the guest could observe or cause.
    ///
    /// **Never a resurrect** (§7.3): there is no path back from here, and it now holds
    /// past the next refresh — because a resurrect would re-materialize the guest's
    /// backings **zeroed** (the isolate's `alloc_sysmem` memory died with its process),
    /// which is silent corruption where this is an honest, Xid-shaped refusal. There is
    /// a path back for the *application*, and it is the one CUDA already takes: build a
    /// new context (a new RM client is a different component, see [`Spine::condemned`]),
    /// or exit and let the guest kernel free the clients. Returns `false` if `pid` was
    /// already gone — idempotent, because a worker HUP and a guest teardown can
    /// legitimately race.
    ///
    /// ★ **The SYSTEM proc is UNCONDEMNABLE, and that is refused here rather than
    /// happening by accident** (`l1_concurrency.md` §12.26). [`Gpu::system`] is not in
    /// the [`ProcSet`], so this used to answer `false` through a **map miss** — the
    /// "retire it loudly, never resurrect" consequence silently evaporating for the one
    /// proc where it matters most. It is refused explicitly now, because the *reason* is
    /// not "it is not in the map": the system component's clients are the **guest
    /// kernel's**, held for the module's lifetime, so condemning it would leave the guest
    /// driver itself with no `Proc`, no isolate, no arena and no route — and §7.3's
    /// recovery story ("a fresh RM client is a different component") requires the guest
    /// kernel to mint clients, which is exactly what would be condemned. Condemning the
    /// system component is therefore **device-fatal by definition**, and a device-fatal
    /// condition must be surfaced as one, not filed as a per-client condemnation entry.
    /// RM's own analogue is the same shape: an unrecoverable kernel-side failure escalates
    /// to `gpuMarkDeviceForReset` + `NV2080_NOTIFIERS_GPU_UNAVAILABLE`
    /// (★ a **version seam**, and the shared part is the part this rests on:
    /// `ogkm-610: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:2771-2792` reaches it from
    /// `_kgspHandleFatalTimeout`, notifying at `:2789` with the classified `errorNum`, and
    /// only when TDR is unsupported, with `gpuMarkDeviceForReset` two lines above it at
    /// `:2779`. At `ogkm-580:` the pair is **split across two functions**: the notify is
    /// unconditional inside `_kgspLogXid119` (`:2130-2205`, notifying at `:2169` with
    /// `GSP_RPC_TIMEOUT`), and `gpuMarkDeviceForReset` is the caller's own three-back-to-back
    /// timeout branch (`:2459`). Different trigger, different payload, different nesting —
    /// but `gpuNotifySubDeviceEvent` at **both**, so the *scope* claim is tag-independent), at **device** level,
    /// never at client level. The adapter half is `SharedDevice::signal_source`'s
    /// `SignalOutcome::DeviceFatal`.
    pub fn retire_proc(&mut self, procs: &mut impl ProcSet, pid: ProcId) -> bool {
        if pid == Gpu::SYSTEM_PROC {
            return false;
        }
        if procs.get_mut(pid).is_none() {
            return false;
        }
        // ★ §12.35 — the violent death goes through the SAME central removal, so the
        // host objects are at least *named* in `pending_release` instead of vanishing
        // with the `Vas`. What differs is only the isolate's fate: `retire()` stops it,
        // so the queue's disposition of record stays §7.0 namespace death — an
        // untrustworthy or already-dead sandbox must not be handed more verbs (§12.17,
        // no resurrect). Staging it anyway is what makes that a *stated* disposition
        // rather than an unnameable set.
        let mut p = self.vacate(procs, pid);
        self.pending_cancels.absorb(p.retire());
        self.sources.deregister_proc(pid).latched();
        let anchor = p.anchor;
        absorb_condemned(&mut self.condemned, p.client_ids.clone());
        // Move (never guess) this proc's derived routing into the condemned maps: the
        // keys are filtered by the VALUE they already name, which is a forward read of a
        // derived table — not a reverse resolve of an address to an owner.
        let pdbs: Vec<(GpuId, Pdb)> = self
            .by_pdb
            .iter()
            .filter(|&(_, &owner)| owner == pid)
            .map(|(&k, _)| k)
            .collect();
        for k in pdbs {
            self.by_pdb.remove(&k);
            self.condemned_by_pdb.insert(k, anchor);
        }
        let vchids: Vec<(GpuId, VChid)> = self
            .by_vchid
            .iter()
            .filter(|&(_, &(owner, _))| owner == pid)
            .map(|(&k, _)| k)
            .collect();
        for k in vchids {
            self.by_vchid.remove(&k);
            self.condemned_by_vchid.insert(k, anchor);
        }
        self.retired.push(p);
        true
    }

    /// ★ §12.13 — is `client` part of a condemned component? The diagnostic half of
    /// the condemned state (the routing half is [`Spine::condemned_pdb`] /
    /// [`Spine::condemned_vchid`]). `true` means: an isolate worker of this client's
    /// dup-connected component died out of band, so the component is dead until the
    /// guest frees its client root — no `Proc`, no isolate, no arena, no route.
    #[must_use]
    pub fn is_condemned(&self, client: HClient) -> bool {
        // ★★★ §12.39 Part B / §12.42 — resolve the VALUE to the declaration(s) it names
        // before asking, because a recycled `hClient` names a *different* namespace from
        // the one that was condemned and answering `true` for it is the bystander death
        // this key exists to make impossible.
        //
        // A value may name several live declarations at once (§12.42), and the two cases
        // are answered differently on purpose:
        //
        //  - it has a **live root** — that is the namespace the guest can still address,
        //    and it is the ONLY one the answer is about. A fresh tenant of a value whose
        //    predecessor is condemned is not condemned.
        //  - it has none — every live declaration here is an orphan whose resources a
        //    foreign alias keeps alive. The corpse is still dead (§12.37's evasion gate:
        //    freeing the root must not launder the condemnation away), so `true` if any of
        //    them is condemned.
        let decls = self.rmgraph.client_declarations();
        let live_root = self
            .rmgraph
            .client_kinds()
            .find(|(k, _)| k.client == client)
            .map(|(k, _)| k);
        let mut ids = decls
            .range(ClientKey::first(client)..=ClientKey::last_for(client))
            .map(|(_, &(id, _))| id);
        match live_root {
            Some(k) => decls
                .get(&k)
                .is_some_and(|&(id, _)| self.condemned.iter().any(|c| c.contains(&id))),
            None => ids.any(|id| self.condemned.iter().any(|c| c.contains(&id))),
        }
    }

    /// ★ §12.13 — how many distinct condemned components the device is carrying
    /// (entries are canonical: pairwise disjoint). Diagnostics, and the executable
    /// statement that condemnation *clears*: this returns to 0 once the guest has freed
    /// every condemned client root.
    #[must_use]
    pub fn condemned_len(&self) -> usize {
        self.condemned.len()
    }

    /// ★ §12.13 — does `(gpu, pdb)` name a **condemned** component's address plane?
    /// Returns its label. The data-plane routing miss that
    /// [`Spine::by_pdb`] takes is MISS=FAULT working as designed; this says *which*
    /// miss it is, resolved **forward** out of the same projection that fills `by_pdb`,
    /// so the fwd plane can name the refusal without a single backwards lookup.
    #[must_use]
    pub fn condemned_pdb(&self, gpu: GpuId, pdb: Pdb) -> Option<ProcAnchor> {
        self.condemned_by_pdb.get(&(gpu, pdb)).copied()
    }

    /// ★ §12.13 — does `(gpu, vchid)` name a **condemned** component's exec plane?
    /// The doorbell/engine-object half of [`Spine::condemned_pdb`].
    #[must_use]
    pub fn condemned_vchid(&self, gpu: GpuId, vchid: VChid) -> Option<ProcAnchor> {
        self.condemned_by_vchid.get(&(gpu, vchid)).copied()
    }

    /// ★ The deferred-reap quiesce point (lesson L10 — the C's P0 fix: reaping the
    /// heavy tables AT the client-root free hung the dying context's residual
    /// polls, so it reaps at the GSP queue re-handshake instead). The core keeps
    /// that split: teardown *retires* eagerly (`Proc::retire` — new ops refused,
    /// isolate stopped) and this call *reaps* deferredly — the **adapter** declares
    /// the quiesce point (its GSP re-handshake / idle-release equivalent) and calls
    /// it.
    ///
    /// Reaping **recycles each reaped proc's GPA arena** into its target window
    /// ([`GpaSpace::release`]) — without this, sequential process churn
    /// (create → destroy → create …, the device teardown→restart lifecycle)
    /// exhausts the window, exactly the leak the C paid for in #80
    /// (`teardown_hardening_done`: "host reaper + GPA free-list"). A retired
    /// proc's undelivered completions die with it — the guest tore the context
    /// down; there is no waiter left to starve.
    ///
    /// ★ **G3b (§12.16): it RETURNS the corpses; it does not drop them.** A real
    /// [`kayfabe_isolate::Isolate`]'s `Drop` is `waitpid` + namespace teardown — a
    /// blocking syscall — and every caller of this function holds the device write
    /// lock, because reaping is a spine op. Dropping in place was therefore a live R1
    /// violation that no assert covered (`Worker::execute` guards verbs, not drops).
    /// The caller now drops the returned [`Reclaimed`] **after** releasing every lock;
    /// [`kayfabe_isolate::IsolateBox`]'s own `Drop` asserts that it did.
    ///
    /// ★ **G3 (§12.16): it CHECKS quiescence; it does not trust it.** A retired proc
    /// whose isolate still has a worker checked out is put **back** on the retired
    /// list and reaped at a later quiesce point. The adapter declares *when* to try
    /// (the L10 quiesce edge); [`Proc::is_quiesced`] decides whether trying is safe.
    /// Without the check, `SharedDevice::verb_op`'s lock-free execute gap — worker
    /// checked out, all locks released, a `Box<dyn RmBackend>` live on a foreign
    /// thread's stack — is a window in which the executor may legally run this reap
    /// and tear the sandbox down underneath it.
    ///
    /// Returns the reaped procs plus a count of those deferred for not being
    /// quiesced.
    pub fn reap_retired(&mut self) -> Reclaimed {
        self.reap_retired_with(ReapPolicy::Unbudgeted)
    }

    /// ★★★★★ **w317 — [`Spine::reap_retired`], with the disposal policy stated at the call
    /// site instead of assumed.**
    ///
    /// See [`ReapPolicy`] for what the two arms mean and why this is an enum rather than a
    /// `bool`. `reap_retired()` is the [`ReapPolicy::Unbudgeted`] arm and is unchanged, so
    /// every existing caller keeps exactly the behaviour it was written against.
    pub fn reap_retired_with(&mut self, policy: ReapPolicy) -> Reclaimed {
        let mut procs = Vec::new();
        let mut deferred = Vec::new();
        let mut deferred_for_drain = 0usize;
        let mut orphaned: Vec<(GpuId, core::ops::Range<u64>)> = Vec::new();
        // Order-preserving partition: `retired` is a deterministic sequence and a
        // deferred proc keeps its place in it (decision #27).
        for mut p in core::mem::take(&mut self.retired) {
            if !p.is_quiesced() {
                deferred.push(p);
                continue;
            }
            // ★★★★★ **w317 — THE GATE THAT MAKES THE BUDGET BIND.**
            //
            // Without this line the budgeted drain buys nothing: it would take its 40 ms
            // slice, the proc would then be quiesced again (the worker went back), the reap
            // would take it, and [`Proc::drop`] would issue **the whole remainder** in one
            // unbounded blocking burst inside `Regs::write` — the 2.65–3.70 s w314 measured.
            // Deferring instead keeps the proc, its isolates and its queue exactly where
            // they are until the drain has emptied it, at which point this predicate goes
            // false and the drop below is a no-op (`Proc::drop` returns early on an empty
            // queue) plus the isolate `waitpid` it always was.
            //
            // ⚠ **Termination, and it is a property rather than a hope.** A *retired* proc is
            // out of every routing map and refuses every new op, so nothing the guest does
            // can add to its queue; the only remaining writer is §7.5's residue staging for
            // verbs that were already in flight, which is finite. The queue is therefore
            // **closed and monotonically decreasing** — each drain turn removes
            // `min(budget, len)` and never puts anything back — so it empties in at most
            // `ceil(len / budget)` turns of a recurring edge and this defer cannot be
            // permanent. `Proc::has_drainable_releases` is deliberately the *narrow*
            // predicate for the other half of that argument: see its docs.
            if policy == ReapPolicy::HoldUndrained && p.has_drainable_releases() {
                deferred_for_drain += 1;
                deferred.push(p);
                continue;
            }
            // Release EACH target's arena back to ITS target window (MG-5: per-GPU
            // arena recycle — the #80 class per target). Taken out of the proc so the
            // proc itself can travel to the caller for its lock-free drop.
            //
            // ★ G7 (§12.19): routed by the arena's OWN owner, not by the map key — the
            // arena names its window, so there is no key here to get wrong. An arena
            // that cannot be routed home (no such target, or the window refuses it) is
            // recorded on [`Reclaimed::orphaned`]; the previous `if let Some(_)` DROPPED
            // it, permanently losing that GPA range with nothing said.
            for (_key, arena) in core::mem::take(&mut p.arenas) {
                debug_assert_eq!(_key, arena.owner(), "an arena is filed under its target");
                let (owner, range) = (arena.owner(), arena.range.clone());
                let home = match self.targets.get_mut(&owner) {
                    Some(t) => t.gpa.release(arena).is_ok(),
                    None => false,
                };
                if !home {
                    orphaned.push((owner, range));
                }
            }
            procs.push(p);
        }
        let deferred_count = deferred.len();
        self.retired = deferred;
        Reclaimed {
            procs,
            deferred: deferred_count,
            deferred_for_drain,
            orphaned,
        }
    }

    /// ★★★★★ **w317 — PLAN ONE BUDGETED DRAIN TURN over the retired set.**
    ///
    /// The locked half of the bounded disposal: pick the first retired proc with drainable
    /// staged work, check a worker out of that target's isolate, and split at most `budget`
    /// disposals off its queue. Pure state — **no verb is issued here** — so it is legal
    /// under the device write lock, exactly like the staging it drains.
    ///
    /// The caller runs [`Orphans::release_plan`] on the returned worker **with zero ranked
    /// locks held** (R1, asserted by `Worker::execute`) and returns the worker afterwards;
    /// [`Spine::checkin_retired`] is the path for a proc that is on this list, and it already
    /// exists for exactly this shape.
    ///
    /// ★ **Deliberately ONE turn, not a loop.** The budget that matters is *wall-clock time
    /// with the BQL held*, and this crate has no clock (§8.3). Splitting the loop out to the
    /// caller is what lets the shell spend a real time budget while this stays a pure
    /// function of state — and it is what makes the budget testable offline with a counting
    /// closure instead of a sleep.
    #[must_use = "the returned batch names host objects and holds a checked-out Worker — \
                  dispose of the Orphans and return the Worker, or both leak"]
    pub fn plan_retired_drain(&mut self, budget: usize) -> Option<RetiredDrain> {
        if budget == 0 {
            return None;
        }
        for p in &mut self.retired {
            let pid = p.id;
            if let Some((gpu, worker, orphans)) = p.checkout_retired_release_budgeted(budget) {
                return Some(RetiredDrain {
                    pid,
                    gpu,
                    worker,
                    orphans,
                });
            }
        }
        None
    }

    /// ★ Return a checked-out [`Worker`] to a proc that has **already left the live
    /// set** (`l1_concurrency.md` §12.16, gap G3 — the hazard the quiesce check
    /// creates and must therefore also close).
    ///
    /// The ordinary return path resolves the proc through the live map. But a verb
    /// executes with every lock released, so its proc can be retired in the gap — by a
    /// graph-driven teardown, or by a sibling worker's HUP — and then the live-map
    /// lookup misses and the worker handle is simply dropped. Before G3 that was
    /// merely untidy; **with** G3 it is a wedge: the abandoned slot stays checked out,
    /// the isolate never quiesces, the proc is deferred at every quiesce point
    /// forever, and its GPA arena never returns to the window. A permanent leak is not
    /// an acceptable price for closing a use-after-free.
    ///
    /// So a retired proc still accepts returns. It accepts nothing else — it refuses
    /// new checkouts (§5.4), it is out of every routing map, and no op can reach it.
    /// Returns `false` if no retired proc has that id (already reaped, or never
    /// retired), in which case the worker dies with its isolate, which is correct:
    /// there is no slot left to un-busy.
    ///
    /// C reference: the C had no interlock here at all. Its session reaper argued
    /// rather than checked — `C: src/qemu/virtio_nvgpu.c:113-118`, "a pooled IOCTL
    /// worker may still be unwinding after `nvkvm_isolate_kill` (which joins the
    /// isolate's reader thread, **not** the pool workers) … so freeing the session
    /// struct here cannot UAF it." That is an argument about what the worker touches,
    /// not a guarantee that it is done. This pair — [`Isolate::in_flight`] plus this
    /// return path — is that missing interlock.
    pub fn checkin_retired(&mut self, pid: ProcId, gpu: GpuId, worker: Worker) -> bool {
        let Some(p) = self.retired.iter_mut().find(|p| p.id == pid) else {
            return false;
        };
        let Some(iso) = p.isolates.get_mut(&gpu) else {
            return false;
        };
        iso.checkin(worker);
        true
    }

    /// ★ Mutable access to a **retired-but-unreaped** proc — the fallback half of the
    /// shell's stage-orphans path (`l1_os_shell.md` §7.5), the exact counterpart of
    /// [`Spine::checkin_retired`] and there for the same reason: a verb runs lock-free,
    /// so its proc can leave the live set in the gap, and the handles it left behind must
    /// still land somewhere nameable rather than being dropped on the floor.
    pub fn retired_mut(&mut self, pid: ProcId) -> Option<&mut Proc> {
        self.retired.iter_mut().find(|p| p.id == pid)
    }

    /// ★★ **Take the latched cancels** for the shell to discharge with no lock held
    /// (`l1_os_shell.md` §7.1) — the exact counterpart of
    /// [`crate::reactor::SourceRegistry::take_pending_wake`].
    ///
    /// The caller MUST have released every ranked lock before discharging:
    /// [`kayfabe_isolate::CancelRequest::discharge`] asserts it, because firing a cancel
    /// is a syscall. Returning them rather than firing them here is what makes that
    /// assert satisfiable instead of a rule someone has to remember.
    pub fn take_pending_cancels(&mut self) -> Cancels {
        core::mem::take(&mut self.pending_cancels)
    }

    /// How many retired procs are still awaiting a reap — either never reaped yet, or
    /// deferred by [`Spine::reap_retired`] for not being quiesced (§12.16, G3).
    /// Diagnostics, and the executable statement that a deferred reap is *deferred*
    /// rather than lost.
    #[must_use]
    pub fn retired_len(&self) -> usize {
        self.retired.len()
    }

    /// ★★ **w310** — guest-RAM pin reclaim from procs that have already left the live set.
    /// [`Gpu::pin_reclaim`] adds the live procs' own tallies to this.
    #[must_use]
    pub fn pin_reclaim_gone(&self) -> PinReclaim {
        self.pin_reclaim_gone
    }

    /// ★ The retired-but-unreaped procs themselves — a **read-only** window, for the
    /// teardown post-condition audit (`l1_concurrency.md` §12.35).
    ///
    /// A vacated proc is out of every routing map and out of [`ProcSet`], but it is not
    /// *gone*: it still owns its isolates, its arenas and its staged
    /// [`Proc::staged_releases`] queue until the reap. Any statement of the form "every
    /// host object is either reachable or queued" is therefore false unless it can see
    /// this list, which is why the audit needs it and why `retired_len` alone was not
    /// enough. `&[Proc]` and not `&mut`: nothing outside the spine may mutate a corpse.
    #[must_use]
    pub fn retired_procs(&self) -> &[Proc] {
        &self.retired
    }

    /// ★★★ **The DEVICE-GLOBAL half of an isolate census** — the materialization counter
    /// plus every retired-but-unreaped proc's isolates, and no live proc.
    ///
    /// # Why this is a seam rather than a field getter (E2)
    ///
    /// `Gpu::isolate_census` walks live procs through `&self`, which a **sharded** shell
    /// cannot do: its procs are behind rank-1 locks and R3 forbids holding two at once, so
    /// it must seed the census here (rank 0, alone) and then visit each proc on its own.
    /// Exposing `isolates_materialized` as a bare getter would have let the two callers
    /// disagree about what else belongs in the seed — the corpses are the easy thing to
    /// forget, and forgetting them under-reports exactly the isolates that already failed.
    /// `Gpu::isolate_census` is built on this, so there is one definition of the seed.
    #[must_use]
    pub fn isolate_census_seed(&self) -> IsolateCensus {
        let mut c = IsolateCensus {
            materialized: self.isolates_materialized,
            ..Default::default()
        };
        for p in &self.retired {
            for iso in p.isolates.values() {
                c.observe(&**iso);
            }
        }
        c
    }

    /// Compose+post one completion batch for target `gpu` if ITS drain gate is open
    /// (§4.3.2, MG-6: per-target GSP queue). Composes from the procs that span this
    /// target (+ the system proc) — a batch outstanding on one GPU never gates
    /// another's post. The caller encodes it on that target's GSP queue and raises
    /// SWGEN0 via `Vmm::raise_irq`.
    ///
    /// A spine op (device-write-lock section in L1 — it composes across procs'
    /// queues and consults the per-target drain gate; pure + microseconds, R1-safe).
    pub fn pump_completions(
        &mut self,
        system: &mut Proc,
        procs: &mut impl ProcSet,
        gpu: GpuId,
    ) -> Option<PostBatch> {
        let target = self.targets.get_mut(&gpu)?;
        let mut queues: Vec<&mut CompletionQueue> = Vec::new();
        if system.targets.contains(&gpu) {
            queues.push(&mut system.completion);
        }
        for (_, p) in procs.iter_mut().filter(|(_, p)| p.targets.contains(&gpu)) {
            queues.push(&mut p.completion);
        }
        target.delivery.try_post(queues)
    }

    /// ★ The starvation fix's entry point, per target (MG-6): proc `pid` issued a
    /// completion-poll RPC on target `gpu`. Its un-acked completions are re-posted off
    /// its OWN poll, regardless of any other proc's doorbell activity.
    pub fn completion_poll(
        &mut self,
        system: &mut Proc,
        procs: &mut impl ProcSet,
        gpu: GpuId,
        pid: ProcId,
        now: Instant,
    ) -> Option<PostBatch> {
        if !self.targets.contains_key(&gpu) {
            return None;
        }
        // Split borrows: take the poller out, poll against the rest, put it back.
        let mut poller = procs.remove(pid)?;
        poller.poll.last_poll = Some(now);
        let batch = {
            let target = self.targets.get_mut(&gpu).expect("checked above");
            let mut others: Vec<&mut CompletionQueue> = Vec::new();
            if system.targets.contains(&gpu) {
                others.push(&mut system.completion);
            }
            for (_, p) in procs.iter_mut().filter(|(_, p)| p.targets.contains(&gpu)) {
                others.push(&mut p.completion);
            }
            target.delivery.on_poll(&mut poller.completion, others)
        };
        procs.insert(pid, poller);
        batch
    }

    /// The guest drained target `gpu`'s outstanding batch (IRQSCLR observed).
    pub fn completions_drained(&mut self, system: &mut Proc, procs: &mut impl ProcSet, gpu: GpuId) {
        let Some(target) = self.targets.get_mut(&gpu) else {
            return;
        };
        let mut queues: Vec<&mut CompletionQueue> = Vec::new();
        if system.targets.contains(&gpu) {
            queues.push(&mut system.completion);
        }
        for (_, p) in procs.iter_mut().filter(|(_, p)| p.targets.contains(&gpu)) {
            queues.push(&mut p.completion);
        }
        target.delivery.drained(queues);
    }
}

impl Gpu {
    /// System proc's reserved id.
    pub const SYSTEM_PROC: ProcId = ProcId(0);

    /// Realize a **single-GPU** device — the N=1 case of [`Gpu::realize`].
    ///
    /// # Errors
    /// See [`Gpu::realize`].
    pub fn new(
        arch: Box<dyn Arch>,
        isolates: Box<dyn IsolateFactory>,
        gpa: GpaSpace,
    ) -> Result<Self, GpuError> {
        Self::realize(arch, isolates, gpa, &[GpuId::ZERO])
    }

    /// Realize a device: pick the arch (once), the isolate factory, the
    /// `GpuId::ZERO` target's GPA window geometry, and — ★ G9 (`l1_concurrency.md`
    /// §12.21) — the **entitlement**: the roster of physical GPUs this device actually
    /// has. Carves the system proc's `GpuId::ZERO` **arena** eagerly.
    ///
    /// ⊘ **It does NOT materialize an isolate** — see [`Spine::ensure_proc_arena`] (E0b).
    /// Every isolate this device ever owns is spawned by [`Spine::apply`], i.e. by a
    /// guest RM event.
    ///
    /// The roster is the only thing standing between a guest-supplied `deviceInstance`
    /// and an unbounded supply of [`GpuTarget`]s (each one a guest-physical window and a
    /// delivery plane, never pruned). It is enforced in [`RmGraph`], at the `Device`
    /// alloc, because that is where RM enforces it.
    ///
    /// # Errors
    /// [`GpuError::Gpa`] if the realize-time window cannot supply the system proc's arena.
    ///
    /// # Panics
    /// If `gpus` is empty, does not contain [`GpuId::ZERO`], or exceeds
    /// [`crate::rmgraph::MAX_GPUS`] — realize-time configuration, never guest input.
    pub fn realize(
        arch: Box<dyn Arch>,
        isolates: Box<dyn IsolateFactory>,
        gpa: GpaSpace,
        gpus: &[GpuId],
    ) -> Result<Self, GpuError> {
        assert!(
            gpus.contains(&GpuId::ZERO),
            "the realize-time target GpuId::ZERO is always part of the roster"
        );
        let window = gpa.window();
        let arch_name = arch.name();
        let geom = TargetGeom {
            window_len: window.end - window.start,
            arena_len: gpa.arena_len(),
            next_base: window.end,
        };
        let mut targets = BTreeMap::new();
        targets.insert(
            GpuId::ZERO,
            GpuTarget {
                gpa: gpa.owned_by(GpuId::ZERO),
                delivery: DeliveryPlane::new(),
                arch_name,
            },
        );
        let mut rmgraph = RmGraph::new();
        rmgraph.entitle(gpus.iter().map(|g| g.0));
        let mut spine = Spine {
            arch,
            rmgraph,
            by_pdb: BTreeMap::new(),
            by_vchid: BTreeMap::new(),
            by_chan: BTreeMap::new(),
            pt_roots: BTreeMap::new(),
            pt_learned: BTreeMap::new(),
            pt_contested: BTreeSet::new(),
            pt_learned_refused: 0,
            ctx_vas: BTreeMap::new(),
            global_ctx_phys: BTreeMap::new(),
            targets,
            sources: SourceRegistry::new(),
            pending_cancels: Cancels::new(),
            pending_spawns: PendingSpawns::new(),
            pin_reclaim_gone: PinReclaim::default(),
            // ★ The caller still HANDS the factory over — ownership is unchanged and the
            // constructor's contract does not leak R1's mechanism. Behind the seam it is
            // shared, because the L1 shell must reach it with no lock held
            // (`kayfabe_isolate::IsolateFactory`'s docs).
            isolates: Arc::from(isolates),
            geom,
            next_proc: 1,
            retired: Vec::new(),
            condemned: Vec::new(),
            condemned_by_pdb: BTreeMap::new(),
            condemned_by_vchid: BTreeMap::new(),
            isolates_materialized: 0,
        };
        let mut system = Proc::new(Self::SYSTEM_PROC, SYSTEM_ANCHOR);
        // The system proc always touches the default target (kernel/scrubber traffic), so
        // its arena is carved here — ★ and its ISOLATE deliberately is NOT (E0b; see
        // [`Spine::ensure_proc_arena`]). Realize keeps exactly the failure it always had
        // (the window cannot supply the arena) and loses exactly the side effect it should
        // never have had (a host process, and under `KAYFABE_ISOLATES=real` a chain of real
        // host RM ioctls, before the guest has executed one instruction).
        spine.ensure_proc_arena(&mut system, GpuId::ZERO)?;
        Ok(Gpu {
            spine,
            system,
            procs: BTreeMap::new(),
        })
    }

    // ---- Split-borrow wrappers over the spine ops (the single-threaded/one-lock
    // shape; the sharded L1 calls the `Spine` entry points directly). -------------

    /// Apply one RM protocol event (see [`Spine::apply`]).
    ///
    /// ★★★ **And it materializes what the apply decided** — the single-threaded half of
    /// R1's spawn deferral. `Spine::apply` latches the isolates the event calls for
    /// instead of spawning them, because under the L1 shell it runs holding rank 0. This
    /// entry point holds **no** lock at all (it reaches its procs by `&mut`, which is the
    /// whole point of the #35 split), so the spawn happens right here and the observable
    /// behaviour of a composed single-threaded device is exactly what it was: when this
    /// returns, every isolate the event called for exists.
    pub fn apply(&mut self, ev: RmEvent) -> Result<(), GpuError> {
        let out = self.spine.apply(&mut self.system, &mut self.procs, ev);
        self.materialize_pending();
        out
    }

    /// ★ Spawn and install every isolate the spine has latched (R1's deferral), with no
    /// lock held — the single-threaded counterpart of `kayfabe_rt::SharedDevice`'s drain.
    ///
    /// Runs on the error path too: [`Spine::apply`]'s rollback re-derives from the
    /// last-good graph, which can itself decide a target needs an isolate, and a refused
    /// event must not leave the device in a state where the *next* op refuses for a
    /// reason the guest cannot act on.
    ///
    /// ⊘ A surplus is possible even here — a proc can be decided-for and then vacated by
    /// the rollback's own re-derivation — so the R5 install is the same one the sharded
    /// shell uses, not a simplified copy.
    fn materialize_pending(&mut self) {
        // One pass is exhaustive: installing an isolate decides nothing, so the latch
        // cannot refill behind us. A loop here would be a spin waiting to happen.
        for PendingSpawn { proc: pid, gpu } in self.spine.take_pending_spawns() {
            let factory = self.spine.isolate_factory();
            let iso = IsolateBox::new(factory.spawn(IsolateId::new(pid.0, gpu)));
            let p = if pid == Gpu::SYSTEM_PROC {
                Some(&mut self.system)
            } else {
                self.procs.get_mut(&pid)
            };
            // A proc that vanished in the gap is the divergent case; the sandbox is
            // surplus and falls here, lock-free, exactly as `install_isolate` refusing it
            // would.
            let surplus = match p {
                Some(p) => self.spine.install_isolate(p, gpu, iso).err(),
                None => Some(iso),
            };
            drop(surplus);
        }
    }

    /// ★★★ **E1 — the isolate plane's health, over every isolate this device holds.**
    ///
    /// Two questions in one value, and they are different questions:
    ///
    /// - `materialized` — **did the guest cause a spawn at all?** Since E0b the answer is
    ///   not a foregone conclusion (`realize` no longer spawns anything), so `0` is a
    ///   diagnosis rather than a blank.
    /// - `no_plane` / `spawn_failed` — **of the isolates that exist, which refuse, and
    ///   because of what?** Before [`kayfabe_isolate::Isolate::refusal`] these two were
    ///   one silence at this seam (`bench_rebuild_notes.md` §5 row 7).
    ///
    /// Retired-but-unreaped procs are counted: they still hold isolates, and an isolate
    /// that refused is a fact about this boot whether or not its proc is still live.
    #[must_use]
    pub fn isolate_census(&self) -> IsolateCensus {
        // ★ The device-global half comes from ONE definition, shared with the sharded
        // shell — see `Spine::isolate_census_seed`.
        let mut c = self.spine.isolate_census_seed();
        for iso in self.system.isolates.values() {
            c.observe(&**iso);
        }
        for p in self.procs.values() {
            for iso in p.isolates.values() {
                c.observe(&**iso);
            }
        }
        c
    }

    /// ★★ Apply one context promotion — the composed, single-owner form of
    /// [`crate::promote::route_promote_ctx`] + [`crate::promote::apply_promote_ctx`].
    ///
    /// The two phases stay separate functions because the sharded L1 shell must run them
    /// under two *different* locks (rank 0, then the owning proc's rank 1, one at a
    /// time). `&mut Gpu` is an exclusivity proof rather than a lock, so this composition
    /// is legal here and nowhere else — the same split `Gpu::retire_proc` makes.
    ///
    /// ★ It routes to the owner of the **address space**, which is not necessarily the
    /// proc that issued the control.
    ///
    /// # Errors
    ///
    /// [`crate::promote::PromoteFault`], by variant.
    pub fn promote_ctx(
        &mut self,
        p: &crate::promote::CtxPromotion,
    ) -> Result<crate::promote::PromoteJoin, crate::promote::PromoteFault> {
        let route =
            crate::promote::route_promote_ctx(&self.spine, p.client, p.chan_client, p.object)?;
        // ★★★★★ §16.50 — snapshot the GPU-scoped publications, join against them, merge
        // back. `&mut Gpu` is an exclusivity proof, so this could touch `self.spine`
        // in place; it goes through the same snapshot/merge the sharded shell must use so
        // that the two compositions cannot drift into different behaviour.
        let mut globals = self.spine.global_ctx_phys_for(route.gpu);
        let proc = if route.proc == Gpu::SYSTEM_PROC {
            &mut self.system
        } else {
            self.procs
                .get_mut(&route.proc)
                .ok_or(crate::promote::PromoteFault::RetiredProc(route.proc))?
        };
        let join = crate::promote::apply_promote_ctx(proc, &route, p, &mut globals)?;
        self.spine.merge_global_ctx_phys(route.gpu, &globals);
        Ok(join)
    }

    /// ★★★★ **Every live channel's ADDRESSING, grouped — the instrument that says which VA
    /// space a channel named and whether that VA space has a page-directory base.**
    ///
    /// # ⊘⊘ Why this moved here, and it is a fix rather than a new feature
    ///
    /// This census already existed, complete, in `kayfabe_qemu_raw`'s
    /// `SharedDoorbell::vas_census_line` — and it was reachable **only from inside a
    /// doorbell refusal sentence**. `[measured 2026-08-09]`: the string `census[` appears
    /// in exactly **two** of the boot logs in `traces/guest_boots/` (`s24_cf18883_cup2`,
    /// `s25_01d12e6_cup2`), and in none of the fifteen since. The reason is not that the
    /// instrument broke — it is that **the plane it was gated behind started succeeding**:
    /// `s35_03a7e10_dup` reports `doorbells: 124 arrived, 124 served, 0 REFUSED by name`,
    /// so the refusal that carried the census never happened and the census printed
    /// nothing.
    ///
    /// ⇒ ★★★ A diagnostic for the **address** plane was gated on a failure of the
    /// **execution** plane. Fixing the second silenced the first, and the boot report gave
    /// no sign of it — there is no "census suppressed" line, only an absence. Three rungs
    /// then recorded *"which VA space the channel names is unread"* and one prescribed a
    /// shim ABI bump to add an instrument that **was already built and already crossed the
    /// ABI**. `a_saturated_instrument_looks_exactly_like_absence`, with the twist that the
    /// saturation was somebody else's green.
    ///
    /// # ★★ Sampled at the EVENT, never at teardown
    ///
    /// ⚠ The caller must latch this **when the refusal happens**. By the time the device's
    /// exit notifier runs, the CUDA process has exited and its channels are freed, so a
    /// teardown-time call returns `NO-LIVE-CHANNELS` — a true sentence about the wrong
    /// instant. That is `a_correct_capture_can_answer_the_wrong_question` exactly: the
    /// question is about a lifetime, and this instrument samples one moment of it.
    ///
    /// `mark` is the channel the caller's refusal is *about*; it is flagged `*` in the
    /// output so a reader never has to hold a `ChanId` in their head while scanning
    /// groups. Pass `None` when the refusal names no channel (promote-ctx names a handle,
    /// not a `ChanId`).
    #[must_use]
    pub fn vas_census_string(&self, mark: Option<ChanId>) -> String {
        format_vas_census(&self.vas_census(), mark)
    }

    /// ★★★ **The census's rows, unformatted** — every live channel of every proc,
    /// including the system proc's.
    ///
    /// ⊘ The system proc is included: RM's own scrubber/CeUtils channels live there, and a
    /// census that showed only user procs would report "no channels" for a boot whose only
    /// channels are the kernel's.
    ///
    /// ★ Extracted from [`Self::vas_census_string`] rather than duplicated, so a test that
    /// asks *"what does a channel declare"* reads the **same rows** the boot log prints. A
    /// second enumeration would be the shape [`VasCensusRow`]'s own re-export doc warns
    /// about: two computations that agree today are not corroboration.
    #[must_use]
    pub fn vas_census(&self) -> Vec<VasCensusRow> {
        let mut rows: Vec<VasCensusRow> = Vec::new();
        for (pid, p) in std::iter::once((Gpu::SYSTEM_PROC, &self.system))
            .chain(self.procs.iter().map(|(k, v)| (*k, v)))
        {
            rows.extend(p.channels.values().map(|c| VasCensusRow::of(pid, c)));
        }
        rows
    }

    /// ★★★ **#177 — perform the guest's `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`.**
    ///
    /// Records (or withdraws) the guest's declaration that the channel at
    /// `(client, object)` is runnable. This is the *whole* of what this port performs for
    /// that control, and the honest scope is written down at
    /// [`ExecPlane::requested`] and in `docs/design/gpfifo_schedule.md`:
    ///
    /// - **performed here** — the eligibility transition. It is enforced: after it, and
    ///   only after it, `kayfabe_fwd::plan_doorbell` will build a submission plan for this
    ///   channel.
    /// - **deferred to the first doorbell** — the host-side runlist submit
    ///   (`NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` on the isolate's own channel group). No work
    ///   can execute on a GPFIFO channel between this control returning and that doorbell,
    ///   so the two are observationally indistinguishable to the guest; this is the same
    ///   architecture the C artifact used to carry a stock driver to a correct matmul.
    /// - **not modelled at all** — `bSkipSubmit` / `bSkipEnable`, refused by name in the
    ///   decoder (`kayfabe_abi::submit::GpfifoScheduleError::UnmodelledSkip`), and every
    ///   runlist ordering property (timeslice, interleave, preemption).
    ///
    /// The route is three forward hops with no fallback, exactly like
    /// [`crate::promote::route_promote_ctx`], but it must **not** use that function: it
    /// routes through `ctx_vas`, and the channel this control is asked about first — RM's
    /// global CeUtils scrubber — is allocated with `hVASpace = NV01_NULL_OBJECT` on
    /// purpose (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:86-93`,
    /// *"For physical CE channels, we will use RM internal VAS"*). A VAS-keyed route would
    /// refuse it for a reason that has nothing to do with scheduling.
    ///
    /// # Errors
    /// [`ScheduleFault`], by variant.
    pub fn schedule_channel(
        &mut self,
        client: HClient,
        object: HObject,
        enable: bool,
    ) -> Result<ScheduleAck, ScheduleFault> {
        let route = route_schedule_channel(&self.spine, client, object)?;
        let proc = if route.proc == Gpu::SYSTEM_PROC {
            &mut self.system
        } else {
            self.procs
                .get_mut(&route.proc)
                .ok_or(ScheduleFault::ChannelNotMaterialized { client, object })?
        };
        Ok(apply_schedule_channel(proc, &route, enable))
    }

    /// ★★★★ **§16.56 — perform the guest's `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`** (the TSG
    /// form, `0xa06c0101`).
    ///
    /// `[measured 2026-08-10, boot s44_b17381c_rmtrace]` this is the **first** thing
    /// `cuCtxCreate` cannot get past: libcuda builds a TSG, eight channels, eight compute
    /// objects and eight copy objects — all `status=0` — then asks RM to schedule the
    /// group, reads back `NV_ERR_NOT_SUPPORTED`, and every record after it is a `FREE`
    /// (`execution_plane_increments.md` §16.55.1).
    ///
    /// # ⊘⊘ What this is NOT: a bare `NV_OK`
    ///
    /// The scope is identical to [`Self::schedule_channel`]'s and is honest for the same
    /// reason — the *eligibility* transition is performed here and it is **enforced**
    /// (`kayfabe_fwd::plan_doorbell` refuses `FwdFault::NotScheduled` for a channel not in
    /// [`ExecPlane::requested`]), while the host-side runlist submit happens at the
    /// channel's first doorbell, where `kayfabe_isolate::RmBackend::schedule` issues
    /// `0xa06c0101` against the **host** group. So the ack is falsifiable: break the gate
    /// and a test goes red (`scripts/bite_gpfifo_schedule.py`).
    ///
    /// ★ That deferral is the C artifact's own architecture, which is the only one a real
    /// driver has accepted end to end: the C answers the guest's schedule from a table and
    /// schedules the *host* TSG at the first doorbell
    /// (`C: src/qemu/nvkvm_gpu_emul.c:8038-8048`, `:4176-4194`) — read
    /// `docs/design/gpfifo_schedule.md` §2 for why the two are observationally
    /// indistinguishable to the guest, and §3 for what is still false.
    ///
    /// ⊘ It is the same deferral, not a weaker one: `plan_doorbell` gates on the member
    /// channel, so a group whose members we could not place refuses **here**
    /// ([`ScheduleGroupFault::NoMemberMaterialized`]) rather than acking and hanging.
    ///
    /// # Errors
    /// [`ScheduleGroupFault`], by variant.
    pub fn schedule_group(
        &mut self,
        client: HClient,
        object: HObject,
        enable: bool,
    ) -> Result<ScheduleGroupAck, ScheduleGroupFault> {
        let route = route_schedule_group(&self.spine, client, object)?;
        let proc = if route.proc == Gpu::SYSTEM_PROC {
            &mut self.system
        } else {
            self.procs
                .get_mut(&route.proc)
                .ok_or(ScheduleGroupFault::NoMemberMaterialized {
                    client,
                    object,
                    members: route.chans.len() + route.unmaterialized,
                })?
        };
        Ok(apply_schedule_group(proc, &route, enable))
    }

    /// ★★★★ **§16.59 — VERIFY the guest's `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE`.**
    ///
    /// ⊘ **Deliberately `&self`.** Every other arm on this list is `&mut self` because it
    /// records something; this one records nothing, because there is nothing to record —
    /// the postcondition the guest asks for either already holds unconditionally
    /// (wait-for-idle) or names machinery this port does not have. A `&mut self` here would
    /// invite a future reader to add a field, and a field whose only reader is the writer is
    /// the unfalsifiable-ack shape in a different costume.
    ///
    /// # Errors
    /// [`CtxswPreemptionFault`], by variant.
    pub fn set_ctxsw_preemption_mode(
        &self,
        client: HClient,
        h_channel: HObject,
    ) -> Result<CtxswPreemptionAck, CtxswPreemptionFault> {
        route_ctxsw_preemption(&self.spine, client, h_channel)
    }

    /// ★★★ **E9/§13.6 — perform the guest's `NVA06F_CTRL_CMD_BIND`.**
    ///
    /// Records which engine the channel at `(client, object)` was bound to.
    /// `rm_engine_type` is in **RM engine space** — the policy converts the wire's
    /// `NV2080_ENGINE_TYPE` first *and* checks it against the device's advertised engine
    /// set, so by the time this runs the engine question is settled and every fault here
    /// is about the channel. Same PLAN/COMMIT split as [`Self::schedule_channel`], for
    /// the same two-lock reason.
    ///
    /// # Errors
    /// [`BindFault`], by variant.
    pub fn bind_channel(
        &mut self,
        client: HClient,
        object: HObject,
        rm_engine_type: u32,
    ) -> Result<BindAck, BindFault> {
        let route = route_bind_channel(&self.spine, client, object)?;
        let proc = if route.proc == Gpu::SYSTEM_PROC {
            &mut self.system
        } else {
            self.procs
                .get_mut(&route.proc)
                .ok_or(BindFault::ChannelNotMaterialized { client, object })?
        };
        Ok(apply_bind_channel(proc, &route, rm_engine_type))
    }

    /// Reap retired procs at the quiesce point (see [`Spine::reap_retired`]).
    ///
    /// Returns [`Reclaimed`] rather than a count for the same reason the spine op
    /// does (§12.16, G3b): the corpses are dropped by whoever binds the value, and the
    /// caller is the only one who knows whether it is holding a lock. `&mut Gpu` is
    /// itself an exclusivity proof rather than a lock, so a single-threaded caller may
    /// simply let it fall.
    pub fn reap_retired(&mut self) -> Reclaimed {
        self.spine.reap_retired()
    }

    /// Retire proc `pid` out of band — the composed form of [`Spine::retire_proc`],
    /// which needs the spine and the proc set as two disjoint borrows. `&mut Gpu` owns
    /// both, so the split lives here once instead of at every call site.
    pub fn retire_proc(&mut self, pid: ProcId) -> bool {
        let Gpu { spine, procs, .. } = self;
        spine.retire_proc(procs, pid)
    }

    /// How many retired procs are awaiting a reap (see [`Spine::retired_len`]).
    #[must_use]
    pub fn retired_len(&self) -> usize {
        self.spine.retired_len()
    }

    /// ★★★ **w310 — THE DEVICE-WIDE PIN RECLAIM TALLY.**
    ///
    /// Live procs' cumulative tallies, plus what vacated procs contributed before they were
    /// handed to the reap ([`Spine::pin_reclaim_gone`]). The system proc is included: it
    /// owns the kernel/CeUtils traffic and can hold pins like any other.
    ///
    /// ⊘ **Monotone, never a gauge.** `released` is *"how many pins this device has ever
    /// staged for release"*, not *"how many are outstanding"* — a bench criterion must grade
    /// it as a **floor** (`> 0`), never as an exact value, for the reason w304's criterion
    /// (E) was rewritten: an exact grade fails on correct results.
    #[must_use]
    pub fn pin_reclaim(&self) -> PinReclaim {
        let mut t = self.spine.pin_reclaim_gone();
        t.absorb(self.system.pin_reclaim);
        for p in self.procs.values() {
            t.absorb(p.pin_reclaim);
        }
        t
    }

    /// Compose+post one completion batch (see [`Spine::pump_completions`]).
    pub fn pump_completions(&mut self, gpu: GpuId) -> Option<PostBatch> {
        self.spine
            .pump_completions(&mut self.system, &mut self.procs, gpu)
    }

    /// A proc's own completion poll (see [`Spine::completion_poll`]).
    pub fn completion_poll(&mut self, gpu: GpuId, pid: ProcId, now: Instant) -> Option<PostBatch> {
        self.spine
            .completion_poll(&mut self.system, &mut self.procs, gpu, pid, now)
    }

    /// The guest drained target `gpu`'s batch (see [`Spine::completions_drained`]).
    pub fn completions_drained(&mut self, gpu: GpuId) {
        self.spine
            .completions_drained(&mut self.system, &mut self.procs, gpu);
    }
}

/// ★★★★ **One live channel's addressing, in the shape the census prints** — the row type
/// [`format_vas_census`] consumes.
///
/// # ⊘ Why this type exists rather than two structs that print the same
///
/// The census had **two** producers reading two different sources: `Gpu`'s own procs (this
/// crate) and `kayfabe_rt`'s lock-ranked `live_pids`/`with_proc` walk, which is the only
/// legal way to read the sharded shell. Both then formatted independently. Two
/// computations that agree today are not corroboration — they are a drift waiting for a
/// reader to compare a `s24` census against a later one and conclude something about the
/// guest from a difference in *our* formatting
/// (`measure_at_the_boundary_not_inside`).
///
/// ⇒ The two **sources** stay separate, because their locking disciplines genuinely
/// differ and collapsing them would put a whole-`Gpu` walk on a path that may hold a
/// rank-1 lock. The **format** is here, once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VasCensusRow {
    /// The proc the channel belongs to.
    pub proc: ProcId,
    /// Its core-assigned slot — the `ChanId` a `FwdFault::NoVas` names.
    pub chan: ChanId,
    /// Its exec-plane demux identity.
    pub vchid: VChid,
    /// ★★★★★ **What the channel is to the guest** — [`Channel::kind`], carried, not
    /// re-derived from [`Self::proc`].
    ///
    /// ⊘ It is not redundant with `p0` meaning *"the system proc"*. That reading is
    /// exactly the implicit re-derivation this field exists to delete: a human scanning a
    /// census had to know that `ProcId(0)` is reserved and that reserved means *the guest
    /// kernel's*, which is two facts about the projection that the row never stated. It
    /// now states one fact about the channel.
    pub kind: crate::channel_kind::GuestChannelKind,
    /// Its engine kind, refined by any engine object allocated on it.
    pub engine: EngineKind,
    /// The `hClient` of the channel's **own** origin declaration. ⚠ Not the VA space's
    /// namespace — so a row can be compared against the client a publication arrived on.
    pub client: u32,
    /// The channel's own origin `hObject`.
    pub handle: u32,
    /// Whether the channel resolved a PDB at all. ★ This is the field the whole census is
    /// read for: `pdb=N` on a channel whose route says `ok(...)` means the VA space was
    /// *named and found* and simply has no page-directory base.
    pub has_pdb: bool,
    /// ★ The discriminating half: which routes ran and what each one hit.
    pub route: crate::project::VasRoutes,
}

impl VasCensusRow {
    /// Build a row from a live [`Channel`].
    #[must_use]
    pub fn of(proc: ProcId, ch: &Channel) -> VasCensusRow {
        VasCensusRow {
            proc,
            chan: ch.id,
            vchid: ch.vchid,
            kind: ch.kind,
            engine: ch.engine,
            client: ch.key.origin.client.0,
            handle: ch.key.origin.handle.0,
            has_pdb: ch.vas_pdb.is_some(),
            route: ch.vas_route,
        }
    }
}

/// ★★★★ **The VA-space census, formatted — the ONE implementation.**
///
/// Groups by `(routes, has_pdb)` and names up to [`VAS_CENSUS_EXEMPLARS`] channels per
/// group, reporting the overflow rather than dropping it. `mark` is flagged `*`.
///
/// See [`Gpu::vas_census_string`] for what this instrument is for, why its absence from
/// fifteen consecutive boot logs was a gating accident rather than a break, and why the
/// caller must sample it **at the refusal** rather than at teardown.
#[must_use]
pub fn format_vas_census(rows: &[VasCensusRow], mark: Option<ChanId>) -> String {
    if rows.is_empty() {
        // ⊘ A TRUE statement, and a loud one. It is also exactly what a caller sees if it
        // sampled too late (after the CUDA process exited and its channels were freed), so
        // it must never be read as "the guest declared no channels".
        return " census[NO-LIVE-CHANNELS]".to_string();
    }
    // Group key: the routes, plus whether a PDB resolved. `String` because `VasRoutes` is
    // `Display`-shaped and the grouping is exactly "prints the same".
    let mut groups: Vec<(String, bool, Vec<&VasCensusRow>)> = Vec::new();
    for r in rows {
        let key = r.route.to_string();
        match groups
            .iter_mut()
            .find(|(k, p, _)| *k == key && *p == r.has_pdb)
        {
            Some((_, _, v)) => v.push(r),
            None => groups.push((key, r.has_pdb, vec![r])),
        }
    }
    let mut out = format!(" census[{} chans, {} outcomes]", rows.len(), groups.len());
    for (key, has_pdb, v) in &groups {
        let pdb = if *has_pdb { "pdb=Y" } else { "pdb=N" };
        out.push_str(&format!(" {{{}x {pdb} {key}", v.len()));
        for r in v.iter().take(VAS_CENSUS_EXEMPLARS) {
            let m = if Some(r.chan) == mark { "*" } else { "" };
            out.push_str(&format!(
                " p{}/c{}{m}:vc{} {} {:?} c0x{:x}/0x{:x}",
                r.proc.0, r.chan.0, r.vchid.0, r.kind, r.engine, r.client, r.handle
            ));
        }
        if v.len() > VAS_CENSUS_EXEMPLARS {
            out.push_str(&format!(" +{} more", v.len() - VAS_CENSUS_EXEMPLARS));
        }
        out.push('}');
    }
    out
}
