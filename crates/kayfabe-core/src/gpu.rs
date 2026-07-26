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

use kayfabe_arch::Arch;
use kayfabe_arch::ids::{ClassId, EngineKind, GpuId, GpuVa, HClient, Pdb, VChid};
use kayfabe_completion::{CompletionQueue, DeliveryPlane, FenceArms, PostBatch};
use kayfabe_isolate::{HostHandle, Isolate, IsolateBox, IsolateFactory, IsolateId, Worker};
use kayfabe_mmu::{AddressFault, AddressTable};
use kayfabe_util::Instant;

use crate::gpa::{GpaArena, GpaBlock, GpaError, GpaSpace};
use crate::project::{Boundaries, ProcBoundary, ProjectionError, SYSTEM_ANCHOR, project};
use crate::reactor::SourceRegistry;
use crate::rmgraph::{NodeKey, RmEvent, RmGraph, RmGraphError};
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
    /// ★ §12.13: a dup edge would have joined a **live** proc's component to a
    /// **condemned** one (a component whose isolate worker died out of band, see
    /// [`Spine::retire_proc`]).
    ///
    /// Dup-connected clients are ONE proc by construction — one isolate, one arena,
    /// one blast radius — so honouring the dup would mean either resurrecting the
    /// condemned component around the live proc's isolate, or condemning a healthy
    /// proc that had nothing to do with the dead worker (a self-inflicted DoS). The
    /// third answer is the only honest one: refuse the *event*. [`Spine::apply`] is
    /// transactional, so the offending `DUP_OBJECT` is rolled back atomically and
    /// **nothing** is disturbed — the live proc keeps serving, the condemned
    /// component stays dead. MISS=FAULT applied to a merge.
    CondemnedMerge {
        /// The live proc the dup would have folded into the condemned component.
        live: ProcId,
        /// The condemned component's label (its smallest client handle).
        condemned: ProcAnchor,
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
    pub origin: NodeKey,
    /// The forward-populated VA→backing table (MISS=FAULT).
    pub table: AddressTable,
    /// This Vas's own host VAS object, once materialized by the fwd plane.
    pub host_vas: Option<HostHandle>,
    /// Captured page-table pages of this VAS (#13's per-PDB `m2_cpt` equivalent;
    /// populated by the CE-PT-write capture feed once the mmu port lands).
    pub pt_pages: BTreeSet<u64>,
    /// ★ G6 (§12.20): the live [`GpaBlock`] behind each **host-published** VA — the
    /// token that lets that GPA range be given BACK to the proc's arena instead of
    /// leaking until the whole proc is reaped. Keyed by VA, exactly like the binding it
    /// accompanies; `Binding` is `Copy` and a free token must not be (that is what makes
    /// the double free unrepresentable), so the two live side by side — the same split
    /// G1 made between the placement and the allocation.
    pub blocks: BTreeMap<u64, GpaBlock>,
    /// VAs currently bound into `table` by the **RPC map source** (`MapMemoryDma`),
    /// so the sync can idempotently add/remove them without disturbing bindings from
    /// other populate sources (`publish_backing`, CE-PT-write capture).
    pub rpc_bound: BTreeSet<u64>,
}

impl Vas {
    fn new(gpu: GpuId, pdb: Pdb, origin: NodeKey) -> Self {
        Vas {
            gpu,
            pdb,
            origin,
            table: AddressTable::new(),
            host_vas: None,
            pt_pages: BTreeSet::new(),
            blocks: BTreeMap::new(),
            rpc_bound: BTreeSet::new(),
        }
    }
}

/// One guest channel — THE exec boundary (vChid, experiment E0).
pub struct Channel {
    /// Core-assigned per-proc slot.
    pub id: ChanId,
    /// The channel node in the RM graph.
    pub key: NodeKey,
    /// ★ MG-4: the GPU target this channel lives on (graph-derived from its `Device`
    /// ancestor). `VChid` is a per-GPU runlist index, so the exec-plane routing map is
    /// keyed `(GpuId, VChid)` and this tag names which GPU the doorbell demuxes on.
    /// Always a RESOLVED target by construction: a channel whose target has not
    /// resolved yet is never materialized (deferred in `Gpu::refresh`, matching the
    /// `Vas` pattern) — never tagged with a default-GPU0 guess.
    pub gpu: GpuId,
    /// The exec-plane identity the doorbell demuxes on (unique only WITHIN [`Self::gpu`]).
    pub vchid: VChid,
    /// The PDB of the VAS this channel is declared against (None = GSP-managed
    /// with no declared VAS — system-routed). Keyed under [`Self::gpu`] in the Vas
    /// map. Named for what it IS — a [`Pdb`] — matching the projection's
    /// [`crate::project::ChannelFacts::vas_pdb`] (one concept, one name).
    pub vas_pdb: Option<Pdb>,
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
    /// Host engine objects forwarded on this channel, keyed by the guest's declared
    /// engine-object class — the Case-1 forward's **idempotency table**
    /// (`execution_plane.md` §2.2: "the object's Case-1 alloc has been forwarded, so
    /// re-sends are idempotent"). A replayed alloc resolves HERE and never re-allocs
    /// a duplicate host object (the same retried-RPC discipline as the graph's
    /// alloc/DUP replay).
    pub host_engine_objects: BTreeMap<ClassId, HostHandle>,
}

/// Per-proc execution plane. Nothing scalar, nothing one-shot (the C's
/// `m2_gr_*`/`doorbell_setup` cracks, ⚠4): every channel's scheduling state is
/// its own, per proc.
#[derive(Debug, Default)]
pub struct ExecPlane {
    /// Channels whose host TSG/channel has been made runnable.
    pub scheduled: BTreeSet<ChanId>,
}

/// Per-proc poll bookkeeping (the C's `m2_poll_kick`/`m2_last_db_token`
/// singletons, made per-process — crack ⚠7).
#[derive(Debug, Default)]
pub struct PollState {
    /// Virtual time of the proc's last completion-poll RPC.
    pub last_poll: Option<Instant>,
    /// Last doorbell token observed from this proc (for poll-kick replay).
    pub last_token: Option<u64>,
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
    /// Deterministic component label (smallest client handle).
    pub anchor: ProcAnchor,
    /// Clients in this proc's dup-connected component (★ §12.27: declared **user**
    /// clients only, joined by user↔user dups — except on `Gpu::system`, whose set is
    /// every declared **kernel** client, joined by rule rather than by dup).
    pub clients: BTreeSet<HClient>,
    /// ★ The address plane: one [`Vas`] per declared **`(GpuId, Pdb)`** (MG-4). A proc
    /// holds several (several VASpaces, and — spanning GPUs — per target); address ops
    /// key on `(GpuId, Pdb)` because a `Pdb` is a per-GPU namespace (two GPUs legally
    /// present identical PDB values).
    pub vases: BTreeMap<(GpuId, Pdb), Vas>,
    /// The exec plane's channels.
    pub channels: BTreeMap<ChanId, Channel>,
    /// Channel node → slot (stable across graph re-derivations).
    pub chan_ids: BTreeMap<NodeKey, ChanId>,
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
    /// ★ MG-5: this proc's per-**target** private GPA arenas (disjoint by
    /// construction, per GPU). Recycled per target at the reap quiesce point (#80).
    pub arenas: BTreeMap<GpuId, GpaArena>,
    /// The set of GPU targets this proc spans (has materialized an isolate/arena for).
    /// Drives per-target completion composition (no cross-GPU serialization).
    pub targets: BTreeSet<GpuId>,
    /// Per-proc poll bookkeeping.
    pub poll: PollState,
    retired: bool,
    next_chan: u32,
}

impl Proc {
    fn new(id: ProcId, anchor: ProcAnchor) -> Self {
        Proc {
            id,
            anchor,
            clients: BTreeSet::new(),
            vases: BTreeMap::new(),
            channels: BTreeMap::new(),
            chan_ids: BTreeMap::new(),
            exec: ExecPlane::default(),
            completion: CompletionQueue::new(),
            fences: FenceArms::new(),
            isolates: BTreeMap::new(),
            arenas: BTreeMap::new(),
            targets: BTreeSet::new(),
            poll: PollState::default(),
            retired: false,
            next_chan: 0,
        }
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

    /// Stage 1 of teardown (lesson L10): stop every per-target isolate, mark retired.
    /// Heavy data-plane reap happens at the proven quiesce point, then drop.
    pub fn retire(&mut self) {
        self.retired = true;
        for iso in self.isolates.values_mut() {
            iso.retire();
        }
    }

    /// True once retired (a retired proc must refuse new ops).
    #[must_use]
    pub fn is_retired(&self) -> bool {
        self.retired
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
        self.arenas.values().all(GpaArena::is_untouched)
            && self.channels.values().all(|c| c.host_channel.is_none())
            && self
                .vases
                .values()
                .all(|v| v.host_vas.is_none() && v.table.iter().all(|(_, _, b)| b.host.is_none()))
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
    orphaned: Vec<(GpuId, core::ops::Range<u64>)>,
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
    pub arch: Box<dyn Arch>,
    /// ★ Source of truth (decision #14).
    pub rmgraph: RmGraph,
    /// ★ Data-plane routing (derived): `(GpuId, PDB)` → owning proc (MG-3). Keyed on
    /// the target because a `Pdb` is a per-GPU namespace. The `Vas` lives in
    /// `procs[pid].vases[(gpu, pdb)]`.
    pub by_pdb: BTreeMap<(GpuId, Pdb), ProcId>,
    /// ★ Exec-plane routing (derived): `(GpuId, vChid)` → (proc, channel) (MG-3).
    pub by_vchid: BTreeMap<(GpuId, VChid), (ProcId, ChanId)>,
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
    isolates: Box<dyn IsolateFactory>,
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
    /// ★ **The same key is what makes RECOVERY possible**, which is the half a user
    /// actually experiences. An application that re-initialises allocates a **new** RM
    /// client; its boundary has no dup edge to any condemned set, so it is a different
    /// component, is not condemned, and gets a live `Proc` with its own isolate and
    /// arena. And an application that simply dies needs no cooperation at all: the guest
    /// kernel frees its clients on its behalf, no boundary intersects the entry any
    /// more, and it is dropped. "Sticky-fatal, not bricked" is therefore a property of
    /// this field's type, not a promise made elsewhere.
    ///
    /// Entries are **canonical**: pairwise disjoint, sorted (determinism, decision
    /// #27), and monotonically *grown* by any boundary that intersects them — a
    /// component that absorbs new clients keeps the blast radius it earned. An entry is
    /// dropped only when NO boundary intersects it any more, i.e. when the **guest
    /// itself** freed the client root.
    condemned: Vec<BTreeSet<HClient>>,
    /// ★ §12.13 — data-plane routing for condemned components: `(GpuId, Pdb)` → the
    /// condemned component's label. Derived exactly like [`Self::by_pdb`], from the
    /// same projection, so naming the fault costs **no reverse resolution** (the
    /// doctrine `RmGraph::gpu_of` and the address table are held to): the guest's key
    /// is looked up forward, it just resolves to "condemned" instead of to a proc.
    condemned_by_pdb: BTreeMap<(GpuId, Pdb), ProcAnchor>,
    /// ★ §12.13 — exec-plane routing for condemned components: `(GpuId, VChid)` → the
    /// condemned component's label. See [`Self::condemned_by_pdb`].
    condemned_by_vchid: BTreeMap<(GpuId, VChid), ProcAnchor>,
}

/// Insert `set` into the canonical condemned list, absorbing (to a fixpoint) every
/// entry it overlaps, then re-sorting so the list is a deterministic function of the
/// graph and not of call order (decision #27). Keeping the entries pairwise disjoint is
/// what makes "does this boundary intersect a condemned component" a single scan with
/// no transitive follow-up.
///
/// **Absorb, never widen.** The fixpoint grows an entry only through clients that are
/// genuinely dup-connected to it — i.e. that shared its blast radius. It must never
/// reach a client that did not, because condemnation's whole defensibility is that it
/// is **sticky-fatal rather than bricked**: a component is dead because the guest's data
/// in *that* isolate's host memory is gone (`RmBackend::alloc_sysmem`, freed with the
/// process), and an unrelated or freshly-minted client's data is not. Widening here
/// would turn a recoverable Xid-shaped fault into a device that refuses everything after
/// one worker crash.
/// Union-find root with path halving, over condemned-entry indices (§12.22).
fn uf_find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn absorb_condemned(list: &mut Vec<BTreeSet<HClient>>, mut set: BTreeSet<HClient>) {
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
/// Will additionally implement `kayfabe_vmm::Device` once the register/GSP models
/// port (`kayfabe-regs`-equivalent + `kayfabe-gsp`); this milestone exposes the
/// event-level API the adapters and tests drive.
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

    /// Ensure `proc` has a materialized isolate + GPA arena for target `gpu`
    /// (MG-5: per-`(Proc, GpuId)`). Idempotent; disjoint by construction. The caller
    /// resolves which proc (a user proc or the system proc) — the spine never
    /// reaches into the proc set itself here.
    fn ensure_proc_target(&mut self, proc: &mut Proc, gpu: GpuId) -> Result<(), GpuError> {
        self.ensure_target(gpu)?;
        // Disjoint field borrows: the factory and the target are separate spine
        // fields; the proc is a caller-provided borrow.
        let isolates = &mut self.isolates;
        let target = self.targets.get_mut(&gpu).expect("ensured above");
        let pid = proc.id;
        proc.isolates
            .entry(gpu)
            .or_insert_with(|| IsolateBox::new(isolates.spawn(IsolateId(pid.0), gpu)));
        if let std::collections::btree_map::Entry::Vacant(e) = proc.arenas.entry(gpu) {
            e.insert(target.gpa.carve()?);
        }
        proc.targets.insert(gpu);
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
        // `plan_refresh`'s `LateMerge`/`CondemnedMerge`, and `sync_rpc_mappings`'
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
            Ok(()) => Ok(()),
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
    /// table entries whose mapping is gone. MISS=FAULT is preserved — a mapping with
    /// no resolvable PDB or backing is a loud fault, never a silent skip.
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
                // A mapping whose VAS has no PDB yet is not routable — deferred until
                // SET_PAGE_DIRECTORY arrives (which re-runs this sync). Not a fault:
                // the guest legitimately maps before binding the page directory.
                continue;
            };
            let Some(gpu) = self.rmgraph.gpu_of(m.vaspace) else {
                // No resolvable target (no Device ancestor) — deferred, never guessed.
                continue;
            };
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
                vas.table
                    .bind(
                        pdb,
                        GpuVa(va),
                        len,
                        Binding {
                            phys,
                            aperture: Aperture::SysmemCoherent,
                            host: None,
                        },
                    )
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
    fn sync_proc_to_boundary(p: &mut Proc, b: &ProcBoundary) {
        p.anchor = b.anchor;
        p.clients = b.clients.clone();
        p.anchor = b.anchor;
        p.clients = b.clients.clone();
        // Vases: create for newly-declared (GpuId, PDB); drop ones no longer
        // derived. Only vases with a resolvable target AND a declared PDB become
        // runtime `Vas`es (an unroutable one defers — MISS at use, never guessed).
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
        p.vases.retain(|key, _| live_keys.contains(key));
        // Channels: stable ChanId per node key. A channel whose GPU target does
        // not resolve is NOT materialized (the same deferral as the Vas pattern
        // above): it enters no routing map, so a runtime `Channel` would be inert
        // — and tagging it `GpuId::ZERO` would be a default-target guess (the
        // no-GPU0-guess doctrine). Its ChanId is still minted, so its slot is
        // stable for when the Device fact lands and it materializes.
        for (&key, facts) in &b.channels {
            let cid = *p.chan_ids.entry(key).or_insert_with(|| {
                let c = ChanId(p.next_chan);
                p.next_chan += 1;
                c
            });
            let Some(gpu) = facts.gpu else {
                continue; // Unroutable (yet) — deferred, never guessed onto GPU0.
            };
            let entry = p.channels.entry(cid).or_insert_with(|| Channel {
                id: cid,
                key,
                gpu,
                vchid: facts.vchid,
                vas_pdb: facts.vas_pdb,
                engine: facts.engine,
                host_channel: None,
                host_token: None,
                host_engine_objects: BTreeMap::new(),
            });
            entry.gpu = gpu;
            entry.vchid = facts.vchid;
            entry.vas_pdb = facts.vas_pdb;
            entry.engine = facts.engine;
        }
        let live_chans: BTreeSet<NodeKey> = b.channels.keys().copied().collect();
        p.chan_ids.retain(|key, _| live_chans.contains(key));
        let live_cids: BTreeSet<ChanId> = p.chan_ids.values().copied().collect();
        p.channels.retain(|cid, _| live_cids.contains(cid));
        p.exec.scheduled.retain(|cid| live_cids.contains(cid));
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
    /// already runs first, so *all four* fault conditions are decidable up front —
    /// [`GpuError::LateMerge`], [`GpuError::CondemnedMerge`],
    /// [`GpuError::HeterogeneousArch`], and GPA exhaustion. With them hoisted the
    /// mutation pass has no `?` left in it and atomicity is **structural, not claimed**.
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
        condemned_bound: &[Option<ProcAnchor>],
    ) -> Result<RefreshPlan, GpuError> {
        let n = bounds.procs.len();

        // (a) Merge legality, in boundary order — the two refusals that used to fire
        //     mid-mutation.
        let mut matches: Vec<Option<Vec<ProcId>>> = Vec::with_capacity(n);
        for (b, &condemned) in bounds.procs.iter().zip(condemned_bound) {
            let mut matching: Vec<ProcId> = procs
                .iter_mut()
                .filter(|(_, p)| !p.clients.is_disjoint(&b.clients))
                .map(|(id, _)| id)
                .collect();
            matching.sort_unstable();
            if let Some(condemned_label) = condemned {
                if let Some(&live) = matching.first() {
                    return Err(GpuError::CondemnedMerge {
                        live,
                        condemned: condemned_label,
                    });
                }
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

        Ok(RefreshPlan {
            matches,
            spans,
            arenas,
            new_targets,
            next_base,
        })
    }

    /// Re-derive boundaries and sync `procs`/`by_pdb`/`by_vchid` to them.
    ///
    /// ★ **Every refusal is hoisted into [`Spine::plan_refresh`]** (§12.18): after the
    /// plan is taken, nothing below can fail, so a refused event disturbs no `Proc`.
    fn refresh(&mut self, system: &mut Proc, procs: &mut impl ProcSet) -> Result<(), GpuError> {
        let bounds: Boundaries = project(&self.rmgraph, self.arch.as_ref())?;

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
        //    condemnation ends only when the guest itself frees the client root.
        //
        //    `condemned_bound[i]` carries the CONDEMNED side's own label (the smallest
        //    client of the intersecting entries, *before* the boundary's own clients are
        //    folded in), so a refused merge can name the corpse rather than the merged
        //    boundary's anchor — which, in exactly the interesting case, is the LIVE
        //    proc's client.
        //
        //    ★ G10 (§12.22): the "does this boundary intersect a condemned component"
        //    question used to be a nested scan — O(|boundaries| × |condemned|) on EVERY
        //    apply, both factors guest-driven, which is a complexity DoS of the same
        //    species the graph's parked-map set was already hardened against. The entries
        //    are pairwise disjoint, so client → entry is a *function*: build it once and
        //    the pass becomes O(total clients · log n).
        let mut owner_of: BTreeMap<HClient, usize> = BTreeMap::new();
        for (i, c) in self.condemned.iter().enumerate() {
            for &client in c {
                owner_of.insert(client, i);
            }
        }
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
        let mut condemned_bound: Vec<Option<ProcAnchor>> = Vec::with_capacity(bounds.procs.len());
        let mut boundary_hit: Vec<Option<usize>> = Vec::with_capacity(bounds.procs.len());
        for b in &bounds.procs {
            let hits: BTreeSet<usize> = b
                .clients
                .iter()
                .filter_map(|c| owner_of.get(c).copied())
                .collect();
            // The CONDEMNED side's own label: the smallest client of the entries this
            // boundary hits, before the boundary's own clients are folded in.
            condemned_bound.push(
                hits.iter()
                    .filter_map(|&i| self.condemned[i].iter().next().copied())
                    .min()
                    .map(ProcAnchor),
            );
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
        // The surviving components: an entry no boundary intersects is DROPPED —
        // condemnation ends only when the guest itself frees the client root.
        let mut carried_by_root: BTreeMap<usize, BTreeSet<HClient>> = BTreeMap::new();
        for (b, hit) in bounds.procs.iter().zip(&boundary_hit) {
            if let Some(i) = *hit {
                let root = uf_find(&mut parent, i);
                carried_by_root
                    .entry(root)
                    .or_default()
                    .extend(b.clients.iter().copied());
            }
        }
        for i in 0..self.condemned.len() {
            let root = uf_find(&mut parent, i);
            if let Some(set) = carried_by_root.get_mut(&root) {
                set.extend(self.condemned[i].iter().copied());
            }
        }
        let mut carried: Vec<BTreeSet<HClient>> = carried_by_root.into_values().collect();
        carried.sort();
        let condemned_anchors: BTreeSet<ProcAnchor> = bounds
            .procs
            .iter()
            .zip(&condemned_bound)
            .filter_map(|(b, dead)| dead.map(|_| b.anchor))
            .collect();

        // 0b. ★ §12.18 — DECIDE EVERYTHING REFUSABLE FIRST. From here down there is no
        //     `?` and no early return: the mutation below cannot fail, so a refused
        //     event leaves every other `Proc` byte-for-byte as it was.
        let mut plan = self.plan_refresh(system, procs, &bounds, &condemned_bound)?;

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
                    // A merge: every other matching proc was checked untouched by the
                    // plan (the early-arm discipline).
                    for &absorbed in &matching[1..] {
                        let mut dead = procs.remove(absorbed).expect("exists");
                        dead.retire();
                        // Retire is also a REACTOR event (§6): the absorbed proc's
                        // completion sources must stop routing the instant it stops
                        // existing, or a late signal resolves onto a dead proc (F4).
                        self.sources.deregister_proc(absorbed).latched();
                        self.retired.push(dead);
                    }
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
            Self::sync_proc_to_boundary(procs.get_mut(pid).expect("live proc exists"), b);
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
        Self::sync_proc_to_boundary(system, &bounds.system);

        // 3. Retire procs whose component vanished (client root freed).
        let dead: Vec<ProcId> = procs
            .iter_mut()
            .map(|(id, _)| id)
            .filter(|id| !live.contains(id))
            .collect();
        for id in dead {
            let mut p = procs.remove(id).expect("exists");
            p.retire();
            // Same reactor law as the merge-absorb path above: deregister every source
            // of the retiring proc, across every GPU target, BEFORE it leaves the live
            // set. Handles are never reused, so any signal still in flight for it now
            // resolves to nothing — a loud `SourceFault`, never a use-after-retire.
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
            let isolates = &mut self.isolates;
            let p = procs.get_mut(pid).expect("live proc exists");
            for &gpu in &plan.spans[i] {
                p.isolates
                    .entry(gpu)
                    .or_insert_with(|| IsolateBox::new(isolates.spawn(IsolateId(pid.0), gpu)));
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
            let isolates = &mut self.isolates;
            for gpu in span {
                system.isolates.entry(gpu).or_insert_with(|| {
                    IsolateBox::new(isolates.spawn(IsolateId(Gpu::SYSTEM_PROC.0), gpu))
                });
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
        }
        for (&(gpu, vchid), &(anchor, key)) in &bounds.by_vchid {
            if anchor == SYSTEM_ANCHOR {
                if let Some(&cid) = system.chan_ids.get(&key) {
                    self.by_vchid.insert((gpu, vchid), (Gpu::SYSTEM_PROC, cid));
                }
            } else if let Some(&pid) = anchor_to_pid.get(&anchor) {
                if let Some(&cid) = procs
                    .get_mut(pid)
                    .expect("anchored proc lives")
                    .chan_ids
                    .get(&key)
                {
                    self.by_vchid.insert((gpu, vchid), (pid, cid));
                }
            } else if condemned_anchors.contains(&anchor) {
                self.condemned_by_vchid.insert((gpu, vchid), anchor);
            }
        }

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
    /// (`ogkm: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:2779-2789`), at **device** level,
    /// never at client level. The adapter half is `SharedDevice::signal_source`'s
    /// `SignalOutcome::DeviceFatal`.
    pub fn retire_proc(&mut self, procs: &mut impl ProcSet, pid: ProcId) -> bool {
        if pid == Gpu::SYSTEM_PROC {
            return false;
        }
        let Some(mut p) = procs.remove(pid) else {
            return false;
        };
        p.retire();
        self.sources.deregister_proc(pid).latched();
        let anchor = p.anchor;
        absorb_condemned(&mut self.condemned, p.clients.clone());
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
        self.condemned.iter().any(|c| c.contains(&client))
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
        let mut procs = Vec::new();
        let mut deferred = Vec::new();
        let mut orphaned: Vec<(GpuId, core::ops::Range<u64>)> = Vec::new();
        // Order-preserving partition: `retired` is a deterministic sequence and a
        // deferred proc keeps its place in it (decision #27).
        for mut p in core::mem::take(&mut self.retired) {
            if !p.is_quiesced() {
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
            orphaned,
        }
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

    /// How many retired procs are still awaiting a reap — either never reaped yet, or
    /// deferred by [`Spine::reap_retired`] for not being quiesced (§12.16, G3).
    /// Diagnostics, and the executable statement that a deferred reap is *deferred*
    /// rather than lost.
    #[must_use]
    pub fn retired_len(&self) -> usize {
        self.retired.len()
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
    /// has. Materializes the system proc's `GpuId::ZERO` isolate + arena eagerly.
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
            targets,
            sources: SourceRegistry::new(),
            isolates,
            geom,
            next_proc: 1,
            retired: Vec::new(),
            condemned: Vec::new(),
            condemned_by_pdb: BTreeMap::new(),
            condemned_by_vchid: BTreeMap::new(),
        };
        let mut system = Proc::new(Self::SYSTEM_PROC, ProcAnchor(HClient(0)));
        // The system proc always touches the default target (kernel/scrubber traffic).
        spine.ensure_proc_target(&mut system, GpuId::ZERO)?;
        Ok(Gpu {
            spine,
            system,
            procs: BTreeMap::new(),
        })
    }

    // ---- Split-borrow wrappers over the spine ops (the single-threaded/one-lock
    // shape; the sharded L1 calls the `Spine` entry points directly). -------------

    /// Apply one RM protocol event (see [`Spine::apply`]).
    pub fn apply(&mut self, ev: RmEvent) -> Result<(), GpuError> {
        self.spine.apply(&mut self.system, &mut self.procs, ev)
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

    /// How many retired procs are awaiting a reap (see [`Spine::retired_len`]).
    #[must_use]
    pub fn retired_len(&self) -> usize {
        self.spine.retired_len()
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
