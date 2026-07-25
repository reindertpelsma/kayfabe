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

use nvkvm_arch::Arch;
use nvkvm_arch::ids::{ClassId, EngineKind, GpuId, GpuVa, HClient, Pdb, VChid};
use nvkvm_completion::{CompletionQueue, DeliveryPlane, FenceArms, PostBatch};
use nvkvm_isolate::{HostHandle, Isolate, IsolateFactory, IsolateId};
use nvkvm_mmu::{AddressFault, AddressTable};
use nvkvm_util::Instant;

use crate::gpa::{GpaArena, GpaError, GpaSpace};
use crate::project::{Boundaries, ProjectionError, project};
use crate::rmgraph::{NodeKey, RmEvent, RmGraph, RmGraphError};
use crate::{ChanId, ProcAnchor, ProcId};

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
/// per-proc entry points (`nvkvm-fwd`'s `publish_backing`, this type's methods)
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
    /// Clients in this proc's dup-connected component.
    pub clients: BTreeSet<HClient>,
    /// ★ The address plane: one [`Vas`] per declared **`(GpuId, Pdb)`** (MG-4). A proc
    /// holds several (compute + UVM, and — spanning GPUs — per target); address ops
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
    pub isolates: BTreeMap<GpuId, Box<dyn Isolate>>,
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
        self.isolates.get(&gpu).map(core::convert::AsRef::as_ref)
    }

    /// Mutable access to this proc's isolate for GPU `gpu` (materialized by [`Gpu`]).
    pub fn isolate_mut(&mut self, gpu: GpuId) -> Option<&mut Box<dyn Isolate>> {
        self.isolates.get_mut(&gpu)
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
        // Pure RPC address-table bookkeeping (host_va = None) is NOT host state — it is
        // re-derivable from the graph, so it never blocks an early merge. (A proc that
        // has materialized no target yet has empty `arenas` → vacuously untouched.)
        self.arenas.values().all(GpaArena::is_untouched)
            && self.channels.values().all(|c| c.host_channel.is_none())
            && self.vases.values().all(|v| {
                v.host_vas.is_none() && v.table.iter().all(|(_, _, b)| b.host_va.is_none())
            })
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
///   (`&mut Proc`) — `nvkvm-fwd`'s route/act split and `publish_backing`;
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
    isolates: Box<dyn IsolateFactory>,
    /// Geometry template for minting a fresh disjoint per-target window.
    geom: TargetGeom,
    next_proc: u32,
    /// Procs retired but not yet reaped (awaiting the quiesce point).
    pub retired: Vec<Proc>,
}

/// The device: composition root of the logic core.
///
/// Will additionally implement `nvkvm_vmm::Device` once the register/GSP models
/// port (`nvkvm-regs`-equivalent + `nvkvm-gsp`); this milestone exposes the
/// event-level API the adapters and tests drive.
///
/// `Send + Sync` (compile-time-asserted; concurrency contract, crate docs): share
/// `&Gpu` across vCPU threads for lock-free reads (`nvkvm-fwd::resolve`,
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
                gpa: GpaSpace::new(base..end, self.geom.arena_len),
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
            .or_insert_with(|| isolates.spawn(IsolateId(pid.0), gpu));
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
    /// **Concurrency shape (L1):** a spine op — runs under the device *write* lock,
    /// with exclusive access to every proc (`system` + the [`ProcSet`]).
    pub fn apply(
        &mut self,
        system: &mut Proc,
        procs: &mut impl ProcSet,
        ev: RmEvent,
    ) -> Result<(), GpuError> {
        // Snapshot the last-good graph so a faulting derivation can be undone. (Apply
        // is the control plane — RM alloc/free/map — not the doorbell/pushbuffer hot
        // path, so a clone here is off the performance-critical path.)
        let snapshot = self.rmgraph.clone();
        match self.apply_inner(system, procs, ev) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Undo: restore the last-good graph and re-derive from it. That graph
                // projected cleanly before this event (every prior apply upheld the
                // same invariant, inductively), so re-derivation cannot fault.
                self.rmgraph = snapshot;
                self.refresh(procs).expect("last-good graph re-projects");
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
        self.refresh(procs)?;
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
        use nvkvm_arch::Aperture;
        use nvkvm_mmu::Binding;

        for (&(gpu, pdb), vas) in proc.vases.iter_mut() {
            // Unbind stale RPC bindings (mapping gone), leaving host-backed
            // publish_backing entries (host_va = Some) alone.
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
                            host_va: None,
                        },
                    )
                    .map_err(GpuError::Address)?;
                vas.rpc_bound.insert(va);
            }
        }
        Ok(())
    }

    /// Re-derive boundaries and sync `procs`/`by_pdb`/`by_vchid` to them.
    fn refresh(&mut self, procs: &mut impl ProcSet) -> Result<(), GpuError> {
        let bounds: Boundaries = project(&self.rmgraph, self.arch.as_ref())?;

        // 1. Match each boundary to existing procs by client intersection.
        let mut live: BTreeSet<ProcId> = BTreeSet::new();
        for b in &bounds.procs {
            let mut matching: Vec<ProcId> = procs
                .iter_mut()
                .filter(|(_, p)| !p.clients.is_disjoint(&b.clients))
                .map(|(id, _)| id)
                .collect();
            matching.sort_unstable();

            let pid = match matching.first() {
                Some(&keep) => {
                    // A merge: every other matching proc must still be untouched
                    // (the early-arm discipline).
                    for &absorbed in &matching[1..] {
                        let p = procs.get_mut(absorbed).expect("matched proc exists");
                        if !p.is_untouched() {
                            return Err(GpuError::LateMerge {
                                kept: keep,
                                absorbed,
                            });
                        }
                        let mut dead = procs.remove(absorbed).expect("exists");
                        dead.retire();
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

            // 2. Sync the proc's derived fields to the boundary.
            let p = procs.get_mut(pid).expect("live proc exists");
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

        // 3. Retire procs whose component vanished (client root freed).
        let dead: Vec<ProcId> = procs
            .iter_mut()
            .map(|(id, _)| id)
            .filter(|id| !live.contains(id))
            .collect();
        for id in dead {
            let mut p = procs.remove(id).expect("exists");
            p.retire();
            self.retired.push(p);
        }

        // 3b. ★ MG-5: materialize each live proc's per-(Proc, GpuId) isolate + arena
        // for every target it now spans (its vases' + routable channels' GPUs). Minting
        // a per-target window (MG-6) happens here too. Collected first to avoid holding
        // a proc borrow across the `&mut self` ensure call.
        let mut needed: BTreeSet<(ProcId, GpuId)> = BTreeSet::new();
        for (pid, p) in procs.iter_mut() {
            for &(gpu, _pdb) in p.vases.keys() {
                needed.insert((pid, gpu));
            }
            for c in p.channels.values() {
                // Only channels that actually route (resolvable target) need a host isolate.
                if bounds.by_vchid.contains_key(&(c.gpu, c.vchid)) {
                    needed.insert((pid, c.gpu));
                }
            }
        }
        for (pid, gpu) in needed {
            let p = procs.get_mut(pid).expect("live proc exists");
            self.ensure_proc_target(p, gpu)?;
        }

        // 4. Rebuild routing maps from the projection (never accreted). Keyed on the
        // target (MG-3): a `Pdb`/`VChid` is a per-GPU namespace.
        self.by_pdb.clear();
        self.by_vchid.clear();
        let anchor_to_pid: BTreeMap<ProcAnchor, ProcId> =
            procs.iter_mut().map(|(id, p)| (p.anchor, id)).collect();
        for (&(gpu, pdb), &(anchor, _)) in &bounds.by_pdb {
            if let Some(&pid) = anchor_to_pid.get(&anchor) {
                self.by_pdb.insert((gpu, pdb), pid);
            }
        }
        for (&(gpu, vchid), &(anchor, key)) in &bounds.by_vchid {
            if let Some(&pid) = anchor_to_pid.get(&anchor)
                && let Some(&cid) = procs
                    .get_mut(pid)
                    .expect("anchored proc lives")
                    .chan_ids
                    .get(&key)
            {
                self.by_vchid.insert((gpu, vchid), (pid, cid));
            }
        }
        Ok(())
    }

    /// ★ The deferred-reap quiesce point (lesson L10 — the C's P0 fix: reaping the
    /// heavy tables AT the client-root free hung the dying context's residual
    /// polls, so it reaps at the GSP queue re-handshake instead). The core keeps
    /// that split: teardown *retires* eagerly (`Proc::retire` — new ops refused,
    /// isolate stopped) and this call *reaps* deferredly — the **adapter** declares
    /// the quiesce point (its GSP re-handshake / idle-release equivalent) and calls
    /// it.
    ///
    /// Reaping drops every retired proc and **recycles its GPA arena** into the
    /// window ([`GpaSpace::release`]) — without this, sequential process churn
    /// (create → destroy → create …, the device teardown→restart lifecycle)
    /// exhausts the window, exactly the leak the C paid for in #80
    /// (`teardown_hardening_done`: "host reaper + GPA free-list"). A retired
    /// proc's undelivered completions die with it — the guest tore the context
    /// down; there is no waiter left to starve. Returns the number reaped.
    pub fn reap_retired(&mut self) -> usize {
        let n = self.retired.len();
        for p in self.retired.drain(..) {
            // Release EACH target's arena back to ITS target window (MG-5: per-GPU
            // arena recycle — the #80 class per target).
            for (gpu, arena) in p.arenas {
                if let Some(t) = self.targets.get_mut(&gpu) {
                    t.gpa.release(arena);
                }
            }
        }
        n
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

    /// Realize a device: pick the arch (once), the isolate factory, and the
    /// `GpuId::ZERO` target's GPA window geometry. Materializes the system proc's
    /// `GpuId::ZERO` isolate + arena eagerly (the N=1 single-target case).
    pub fn new(
        arch: Box<dyn Arch>,
        isolates: Box<dyn IsolateFactory>,
        gpa: GpaSpace,
    ) -> Result<Self, GpuError> {
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
                gpa,
                delivery: DeliveryPlane::new(),
                arch_name,
            },
        );
        let mut spine = Spine {
            arch,
            rmgraph: RmGraph::new(),
            by_pdb: BTreeMap::new(),
            by_vchid: BTreeMap::new(),
            targets,
            isolates,
            geom,
            next_proc: 1,
            retired: Vec::new(),
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
    pub fn reap_retired(&mut self) -> usize {
        self.spine.reap_retired()
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
