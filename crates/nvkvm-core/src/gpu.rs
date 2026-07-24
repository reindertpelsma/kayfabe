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
use nvkvm_arch::ids::{EngineClass, GpuVa, HClient, Pdb, VChid};
use nvkvm_completion::{CompletionQueue, DeliveryPlane, PostBatch};
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
    /// The hardware identity (the GPU's CR3).
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
    fn new(pdb: Pdb, origin: NodeKey) -> Self {
        Vas {
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
    /// The exec-plane identity the doorbell demuxes on.
    pub vchid: VChid,
    /// The PDB of the VAS this channel is declared against (None = GSP-managed
    /// with no declared VAS — system-routed).
    pub vas: Option<Pdb>,
    /// Engine class.
    pub engine: EngineClass,
    /// Host channel object, once materialized by the fwd plane.
    pub host_channel: Option<HostHandle>,
    /// Host work-submit token, once materialized.
    pub host_token: Option<u64>,
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
    /// ★ The address plane: one [`Vas`] per declared PDB. A proc holds several
    /// (compute + UVM) — which is exactly why address ops must key on `Vas`.
    pub vases: BTreeMap<Pdb, Vas>,
    /// The exec plane's channels.
    pub channels: BTreeMap<ChanId, Channel>,
    /// Channel node → slot (stable across graph re-derivations).
    pub chan_ids: BTreeMap<NodeKey, ChanId>,
    /// Per-proc execution plane state.
    pub exec: ExecPlane,
    /// Per-proc completion queue (§4.3.2 — the starvation fix's per-proc half).
    pub completion: CompletionQueue,
    /// This proc's own unprivileged host isolate (`session == ProcId`).
    pub isolate: Box<dyn Isolate>,
    /// This proc's private GPA arena (disjoint by construction).
    pub arena: GpaArena,
    /// Per-proc poll bookkeeping.
    pub poll: PollState,
    retired: bool,
    next_chan: u32,
}

impl Proc {
    fn new(id: ProcId, anchor: ProcAnchor, isolate: Box<dyn Isolate>, arena: GpaArena) -> Self {
        Proc {
            id,
            anchor,
            clients: BTreeSet::new(),
            vases: BTreeMap::new(),
            channels: BTreeMap::new(),
            chan_ids: BTreeMap::new(),
            exec: ExecPlane::default(),
            completion: CompletionQueue::new(),
            isolate,
            arena,
            poll: PollState::default(),
            retired: false,
            next_chan: 0,
        }
    }

    /// Stage 1 of teardown (lesson L10): stop the isolate, mark retired. Heavy
    /// data-plane reap happens at the proven quiesce point, then drop.
    pub fn retire(&mut self) {
        self.retired = true;
        self.isolate.retire();
    }

    /// True once retired (a retired proc must refuse new ops).
    #[must_use]
    pub fn is_retired(&self) -> bool {
        self.retired
    }

    /// True while this proc has touched no data-plane state (merge legality,
    /// lesson L9).
    fn is_untouched(&self) -> bool {
        // "Touched" = host-materialized data-plane state (arena carved, host channel /
        // host VAS allocated, or a binding published into a host VAS). Pure RPC
        // address-table bookkeeping (host_va = None) is NOT host state — it is
        // re-derivable from the graph, so it never blocks an early merge.
        self.arena.is_untouched()
            && self.channels.values().all(|c| c.host_channel.is_none())
            && self
                .vases
                .values()
                .all(|v| v.host_vas.is_none() && v.table.iter().all(|(_, _, b)| b.host_va.is_none()))
    }
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
/// pumping) takes `&mut self` under caller-provided exclusivity. Device-global
/// state here (the graph, routing maps, the delivery gate) is exactly the state
/// that needs a device-wide lock; everything per-process lives in [`Proc`] and
/// parallelizes per-proc.
pub struct Gpu {
    /// The Axis-B behavior this device was realized with. The core only ever
    /// calls trait methods on it — never names a generation.
    pub arch: Box<dyn Arch>,
    /// ★ Source of truth (decision #14).
    pub rmgraph: RmGraph,
    /// Derived per-process containers.
    pub procs: BTreeMap<ProcId, Proc>,
    /// The system proc: kernel RM / scrubber / CeUtils traffic ([`crate::Traffic`]).
    pub system: Proc,
    /// Data-plane routing (derived): PDB → owning proc. The `Vas` itself lives in
    /// `procs[pid].vases[pdb]`.
    pub by_pdb: BTreeMap<Pdb, ProcId>,
    /// Exec-plane routing (derived): vChid → (proc, channel).
    pub by_vchid: BTreeMap<VChid, (ProcId, ChanId)>,
    /// The guest-physical window (hands out per-proc arenas).
    pub gpa: GpaSpace,
    /// Device-global completion posting policy (drain gate over the one GSP queue).
    pub delivery: DeliveryPlane,
    isolates: Box<dyn IsolateFactory>,
    next_proc: u32,
    /// Procs retired but not yet reaped (awaiting the quiesce point).
    pub retired: Vec<Proc>,
}

impl Gpu {
    /// System proc's reserved id.
    pub const SYSTEM_PROC: ProcId = ProcId(0);

    /// Realize a device: pick the arch (once), the isolate factory, and the GPA
    /// window geometry. Spawns the system proc's isolate eagerly.
    pub fn new(
        arch: Box<dyn Arch>,
        mut isolates: Box<dyn IsolateFactory>,
        mut gpa: GpaSpace,
    ) -> Result<Self, GpuError> {
        let system_isolate = isolates.spawn(IsolateId(Self::SYSTEM_PROC.0));
        let system_arena = gpa.carve()?;
        let system =
            Proc::new(Self::SYSTEM_PROC, ProcAnchor(HClient(0)), system_isolate, system_arena);
        Ok(Gpu {
            arch,
            rmgraph: RmGraph::new(),
            procs: BTreeMap::new(),
            system,
            by_pdb: BTreeMap::new(),
            by_vchid: BTreeMap::new(),
            gpa,
            delivery: DeliveryPlane::new(),
            isolates,
            next_proc: 1,
            retired: Vec::new(),
        })
    }

    /// Apply one RM protocol event and re-sync all derived state to the graph.
    pub fn apply(&mut self, ev: RmEvent) -> Result<(), GpuError> {
        self.rmgraph.apply(self.arch.as_ref(), ev)?;
        self.refresh()?;
        // Forward-populate the address table from the RPC map source (co-equal with
        // the CE-PT-write capture source — `mode2_address_table.md`). Bindings track
        // the graph's live mappings; unmap eagerly depopulates.
        self.sync_rpc_mappings()?;
        Ok(())
    }

    /// Sync each `Vas`'s address table to the graph's live DMA mappings (the RPC
    /// populate source). Idempotent: binds mappings not yet in the table, unbinds
    /// table entries whose mapping is gone. MISS=FAULT is preserved — a mapping with
    /// no resolvable PDB or backing is a loud fault, never a silent skip.
    fn sync_rpc_mappings(&mut self) -> Result<(), GpuError> {
        use nvkvm_arch::Aperture;
        use nvkvm_mmu::Binding;

        // Desired: (pdb, va) -> (len, phys) for every live mapping with a resolved PDB.
        let mut desired: BTreeMap<(u64, u64), (u64, u64)> = BTreeMap::new();
        for m in self.rmgraph.mappings() {
            let Some(pdb) = m.pdb else {
                // A mapping whose VAS has no PDB yet is not routable — deferred until
                // SET_PAGE_DIRECTORY arrives (which re-runs this sync). Not a fault:
                // the guest legitimately maps before binding the page directory.
                continue;
            };
            let phys = m.mem_phys.ok_or(GpuError::UnbackedMapping { pdb, va: m.va.0 })?;
            desired.insert((pdb.0, m.va.0), (m.len, phys));
        }

        for proc in self.procs.values_mut().chain(core::iter::once(&mut self.system)) {
            for (&pdb, vas) in proc.vases.iter_mut() {
                // Unbind stale RPC bindings (mapping gone), leaving host-backed
                // publish_backing entries (host_va = Some) alone.
                let stale: Vec<u64> = vas
                    .rpc_bound
                    .iter()
                    .filter(|&&va| !desired.contains_key(&(pdb.0, va)))
                    .copied()
                    .collect();
                for va in stale {
                    vas.table.unbind(GpuVa(va));
                    vas.rpc_bound.remove(&va);
                }
                // Bind newly-declared mappings for this PDB.
                for (&(mpdb, va), &(len, phys)) in desired.iter() {
                    if mpdb != pdb.0 || vas.rpc_bound.contains(&va) {
                        continue;
                    }
                    vas.table
                        .bind(
                            pdb,
                            GpuVa(va),
                            len,
                            Binding { phys, aperture: Aperture::SysmemCoherent, host_va: None },
                        )
                        .map_err(GpuError::Address)?;
                    vas.rpc_bound.insert(va);
                }
            }
        }
        Ok(())
    }

    /// Re-derive boundaries and sync `procs`/`by_pdb`/`by_vchid` to them.
    fn refresh(&mut self) -> Result<(), GpuError> {
        let bounds: Boundaries = project(&self.rmgraph, self.arch.as_ref())?;

        // 1. Match each boundary to existing procs by client intersection.
        let mut live: BTreeSet<ProcId> = BTreeSet::new();
        for b in &bounds.procs {
            let mut matching: Vec<ProcId> = self
                .procs
                .iter()
                .filter(|(_, p)| !p.clients.is_disjoint(&b.clients))
                .map(|(&id, _)| id)
                .collect();
            matching.sort_unstable();

            let pid = match matching.first() {
                Some(&keep) => {
                    // A merge: every other matching proc must still be untouched
                    // (the early-arm discipline).
                    for &absorbed in &matching[1..] {
                        let p = self.procs.get(&absorbed).expect("matched proc exists");
                        if !p.is_untouched() {
                            return Err(GpuError::LateMerge { kept: keep, absorbed });
                        }
                        let mut dead = self.procs.remove(&absorbed).expect("exists");
                        dead.retire();
                        self.retired.push(dead);
                    }
                    keep
                }
                None => {
                    let id = ProcId(self.next_proc);
                    self.next_proc += 1;
                    let isolate = self.isolates.spawn(IsolateId(id.0));
                    let arena = self.gpa.carve()?;
                    self.procs.insert(id, Proc::new(id, b.anchor, isolate, arena));
                    id
                }
            };
            live.insert(pid);

            // 2. Sync the proc's derived fields to the boundary.
            let p = self.procs.get_mut(&pid).expect("live proc exists");
            p.anchor = b.anchor;
            p.clients = b.clients.clone();
            // Vases: create for newly-declared PDBs; drop ones no longer derived.
            let live_pdbs: BTreeSet<Pdb> = b.vases.values().flatten().copied().collect();
            for (&origin, &pdb) in &b.vases {
                if let Some(pdb) = pdb {
                    p.vases.entry(pdb).or_insert_with(|| Vas::new(pdb, origin));
                }
            }
            p.vases.retain(|pdb, _| live_pdbs.contains(pdb));
            // Channels: stable ChanId per node key.
            for (&key, facts) in &b.channels {
                let cid = *p.chan_ids.entry(key).or_insert_with(|| {
                    let c = ChanId(p.next_chan);
                    p.next_chan += 1;
                    c
                });
                let entry = p.channels.entry(cid).or_insert_with(|| Channel {
                    id: cid,
                    key,
                    vchid: facts.vchid,
                    vas: facts.vas_pdb,
                    engine: facts.engine,
                    host_channel: None,
                    host_token: None,
                });
                entry.vchid = facts.vchid;
                entry.vas = facts.vas_pdb;
                entry.engine = facts.engine;
            }
            let live_chans: BTreeSet<NodeKey> = b.channels.keys().copied().collect();
            p.chan_ids.retain(|key, _| live_chans.contains(key));
            let live_cids: BTreeSet<ChanId> = p.chan_ids.values().copied().collect();
            p.channels.retain(|cid, _| live_cids.contains(cid));
            p.exec.scheduled.retain(|cid| live_cids.contains(cid));
        }

        // 3. Retire procs whose component vanished (client root freed).
        let dead: Vec<ProcId> = self.procs.keys().filter(|id| !live.contains(id)).copied().collect();
        for id in dead {
            let mut p = self.procs.remove(&id).expect("exists");
            p.retire();
            self.retired.push(p);
        }

        // 4. Rebuild routing maps from the projection (never accreted).
        self.by_pdb.clear();
        self.by_vchid.clear();
        let anchor_to_pid: BTreeMap<ProcAnchor, ProcId> =
            self.procs.iter().map(|(&id, p)| (p.anchor, id)).collect();
        for (&pdb, &(anchor, _)) in &bounds.by_pdb {
            if let Some(&pid) = anchor_to_pid.get(&anchor) {
                self.by_pdb.insert(pdb, pid);
            }
        }
        for (&vchid, &(anchor, key)) in &bounds.by_vchid {
            if let Some(&pid) = anchor_to_pid.get(&anchor)
                && let Some(&cid) = self.procs[&pid].chan_ids.get(&key)
            {
                self.by_vchid.insert(vchid, (pid, cid));
            }
        }
        Ok(())
    }

    /// Compose+post one completion batch across all procs if the drain gate is
    /// open (§4.3.2). The caller (fwd/adapter) encodes it on the GSP queue and
    /// raises SWGEN0 via `Vmm::raise_irq` — the core stays VMM-free.
    pub fn pump_completions(&mut self) -> Option<PostBatch> {
        let queues =
            core::iter::once(&mut self.system.completion)
                .chain(self.procs.values_mut().map(|p| &mut p.completion));
        self.delivery.try_post(queues)
    }

    /// ★ The starvation fix's entry point: proc `pid` issued a completion-poll
    /// RPC. Its un-acked completions are re-posted off its OWN poll, regardless
    /// of any other proc's doorbell activity.
    pub fn completion_poll(&mut self, pid: ProcId, now: Instant) -> Option<PostBatch> {
        // Split borrows: take the poller out, poll against the rest, put it back.
        let mut poller = self.procs.remove(&pid)?;
        poller.poll.last_poll = Some(now);
        let batch = {
            let others = core::iter::once(&mut self.system.completion)
                .chain(self.procs.values_mut().map(|p| &mut p.completion));
            self.delivery.on_poll(&mut poller.completion, others)
        };
        self.procs.insert(pid, poller);
        batch
    }

    /// The guest drained the outstanding batch (IRQSCLR observed).
    pub fn completions_drained(&mut self) {
        let queues =
            core::iter::once(&mut self.system.completion)
                .chain(self.procs.values_mut().map(|p| &mut p.completion));
        self.delivery.drained(queues);
    }
}
