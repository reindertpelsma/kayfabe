//! Pure projections of the [`crate::rmgraph::RmGraph`] — `Proc` grouping, `by_pdb`,
//! `by_vchid` (arch doc §4.3.1a "Derivation rules").
//!
//! These are **deterministic pure functions of the graph**: a reordered or retried
//! guest yields the same graph, so it yields the same boundaries (the shuffle
//! property test in this crate is the executable statement of that guarantee).
//! Nothing here is accreted from observed event order, and nothing here mutates —
//! the runtime ([`crate::gpu::Gpu`]) *syncs* its owned state to these boundaries.
//!
//! Derivation rules:
//! - **`Vas` (PDB) = the address-plane owner.** A channel resolves its VASpace via
//!   declared facts only: `hVASpace`, else its CtxShare's, else its parent TSG's.
//! - **`Channel` (vChid) = the exec-plane owner** (E0's demux identity).
//! - **`Proc` = the grouping node** (isolate + arena + lifecycle only): one
//!   dup-connected component of clients. Never inferred from timing.

use std::collections::{BTreeMap, BTreeSet};

use nvkvm_arch::ids::{EngineKind, GpuId, HClient, HObject, Pdb, VChid};
use nvkvm_arch::{Arch, ObjectKind};

use crate::ProcAnchor;
use crate::rmgraph::{NodeKey, RmGraph, RmNode};

/// Declared facts of one VASpace origin, resolved against the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VasFacts {
    /// The owning GPU target, derived via the VASpace's `Device` ancestor
    /// ([`RmGraph::gpu_of`]). `None` = not yet resolvable (no Device ancestor / no
    /// declared instance): the VAS is not routable until the fact lands — MISS at
    /// use, never a default-GPU0 guess.
    pub gpu: Option<GpuId>,
    /// The declared PDB, once `SET_PAGE_DIRECTORY` arrives.
    pub pdb: Option<Pdb>,
}

/// Declared facts of one channel, fully resolved against the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelFacts {
    /// The exec-plane identity, recovered by the arch from declared flags.
    pub vchid: VChid,
    /// The channel's own GPU target, derived via its `Device` ancestor
    /// ([`RmGraph::gpu_of`]). `None` = not routable (yet): the channel enters no
    /// routing map and materializes no runtime state until the fact resolves.
    pub gpu: Option<GpuId>,
    /// Origin VASpace node this channel is bound to (dup-aliases resolved), if any
    /// declared path exists. `None` = GSP-managed with no declared VAS (routed to
    /// the system/minted VAS by higher layers — out of scope this milestone).
    pub vas_origin: Option<NodeKey>,
    /// The PDB of that VASpace, once declared via `SetPageDir`.
    pub vas_pdb: Option<Pdb>,
    /// ★ The fine [`EngineKind`] of this channel's context (`execution_plane.md`
    /// §2.1/§2.2): the channel *class*'s declared kind, **refined by the engine
    /// object allocated on it** (an NVENC session on a GR-class channel makes it an
    /// `NvEnc` context — distinguishable AT the channel, so routing and
    /// completion-arm selection are exact). Graph-derived, hence order/replay
    /// independent: the refinement is a pure function of the graph, not of when the
    /// engine-object alloc arrived.
    pub engine: EngineKind,
}

/// One derived process boundary (a dup-connected component of clients).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcBoundary {
    /// Deterministic label: the smallest client handle in the component.
    pub anchor: ProcAnchor,
    /// All clients in the component.
    pub clients: BTreeSet<HClient>,
    /// VASpace **origin** nodes owned by this component → resolved facts (target GPU
    /// and declared PDB). A process may own several (compute + UVM) — one `Proc`,
    /// many `Vas`.
    pub vases: BTreeMap<NodeKey, VasFacts>,
    /// Channel nodes owned by this component → resolved facts.
    pub channels: BTreeMap<NodeKey, ChannelFacts>,
}

/// The full derived routing picture. Pure data; `PartialEq` so the
/// order-independence property is directly assertable.
///
/// ★ Routing keys are `(GpuId, Pdb)` / `(GpuId, VChid)` — `Pdb`/`VChid` are
/// **per-GPU namespaces** (two GPUs legally present identical values), so the target
/// is part of the key by construction (the #14 lesson lifted onto the GPU axis).
/// Only objects whose GPU target resolves enter routing; an unresolvable target is a
/// deferred/unroutable object (loud MISS at use), never a guessed GPU0 entry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Boundaries {
    /// Process boundaries, ascending anchor order.
    pub procs: Vec<ProcBoundary>,
    /// Data-plane routing: (target, PDB) → (owning component, VASpace origin node).
    pub by_pdb: BTreeMap<(GpuId, Pdb), (ProcAnchor, NodeKey)>,
    /// Exec-plane routing: (target, vChid) → (owning component, channel node).
    pub by_vchid: BTreeMap<(GpuId, VChid), (ProcAnchor, NodeKey)>,
}

/// Projection failures. All loud: each is a real protocol violation or a graph
/// inconsistency that must never be silently resolved (MISS=FAULT posture).
///
/// ★ Collisions are scoped **per GPU target** (the F1 guard, decision #18C, under
/// the multi-GPU axis): identical `Pdb`/`VChid` values on *different* GPUs are
/// LEGAL (per-GPU namespaces — refusing them would be a false-positive DoS at N=2),
/// while two claimants on the SAME target are still the hostile ambiguity the guard
/// exists for. `gpu: None` is the not-yet-resolvable scope (objects whose Device
/// target is unknown collide among themselves — the conservative pre-multi-GPU
/// behavior, never silently dropped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionError {
    /// Two live channels on ONE target decode to the same vChid (E0 says this cannot
    /// happen on a sane guest; if it does, demux would be ambiguous — refuse loudly).
    VchidCollision {
        /// The target GPU scope of the collision (`None` = unresolved-target scope).
        gpu: Option<GpuId>,
        /// The colliding vChid.
        vchid: VChid,
        /// First claimant.
        a: NodeKey,
        /// Second claimant.
        b: NodeKey,
    },
    /// Two distinct VASpace origins on ONE target declare the same PDB.
    PdbCollision {
        /// The target GPU scope of the collision (`None` = unresolved-target scope).
        gpu: Option<GpuId>,
        /// The colliding PDB.
        pdb: Pdb,
        /// First claimant.
        a: NodeKey,
        /// Second claimant.
        b: NodeKey,
    },
}

/// Tiny deterministic union-find over client handles.
struct ClientUnion {
    parent: BTreeMap<HClient, HClient>,
}

impl ClientUnion {
    fn new(clients: impl IntoIterator<Item = HClient>) -> Self {
        ClientUnion { parent: clients.into_iter().map(|c| (c, c)).collect() }
    }

    fn find(&mut self, c: HClient) -> HClient {
        let p = *self.parent.entry(c).or_insert(c);
        if p == c {
            return c;
        }
        let root = self.find(p);
        self.parent.insert(c, root);
        root
    }

    /// Union by minimum handle so the representative IS the anchor (deterministic).
    fn union(&mut self, a: HClient, b: HClient) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        self.parent.insert(hi, lo);
    }
}

/// Resolve `(ns, handle)` to its origin **only if that origin is a VASpace**.
///
/// A channel/TSG/CtxShare's `hVASpace` is a *declared protocol fact* that names a
/// `FERMI_VASPACE_A`; a hostile or buggy guest may instead name a TSG, a memory
/// object, or a dangling handle (possibly one that happens to carry a `SetPageDir`
/// PDB). Binding a channel to a non-VASpace's PDB would make the channel's
/// `vas_pdb` disagree with `by_pdb` (which only routes real VASpaces) — a
/// confused-deputy inconsistency the fuzz property caught. So resolution to a
/// non-VASpace returns `None`: the channel is treated as having no declared VAS
/// (a loud MISS at use time), never silently bound to an unrelated object's PDB.
fn resolve_vaspace_handle(g: &RmGraph, ns: HClient, handle: HObject) -> Option<&RmNode> {
    // The ONE typed-resolution primitive (decision #18C): resolving a handle to the
    // wrong `ObjectKind` is a single centrally-enforced check, not a per-caller
    // `matches!` a new site could forget.
    g.origin_of_kind(NodeKey::new(ns, handle), ObjectKind::VaSpace)
}

/// Resolve a channel node's VASpace origin per the declared-facts precedence:
/// own `hVASpace` → CtxShare's → parent TSG's. Every hop is a typed resolution
/// through [`RmGraph::origin_of_kind`] — an `hVASpace` naming a non-VASpace, an
/// `hContextShare` naming a non-CtxShare, or a `parent` that is not a TSG each
/// resolves to `None` (a loud MISS at use time), never a silent cross-object bind.
fn resolve_channel_vas<'g>(g: &'g RmGraph, chan: &RmNode) -> Option<&'g RmNode> {
    let ns = chan.key.client;
    if let Some(hv) = chan.facts.h_vaspace {
        return resolve_vaspace_handle(g, ns, hv);
    }
    if let Some(hcs) = chan.facts.h_ctx_share
        && let Some(cs) = g.origin_of_kind(NodeKey::new(ns, hcs), ObjectKind::CtxShare)
        && let Some(hv) = cs.facts.h_vaspace
    {
        return resolve_vaspace_handle(g, cs.key.client, hv);
    }
    // Parent may be a TSG that declares the VAS.
    if let Some(parent) = g.origin_of_kind(NodeKey::new(ns, chan.parent), ObjectKind::Tsg)
        && let Some(hv) = parent.facts.h_vaspace
    {
        return resolve_vaspace_handle(g, parent.key.client, hv);
    }
    None
}

/// Derive the full boundary picture from the graph. Pure; order-independent by
/// construction (it looks only at the graph's declared facts).
pub fn project(g: &RmGraph, arch: &dyn Arch) -> Result<Boundaries, ProjectionError> {
    // Client universe: explicit resource-owning namespaces + the endpoints of every
    // **resolved** dup (a dst client that only receives an alias and allocates nothing
    // has no origin node of its own, so it must be picked up here to be grouped). A
    // still-parked, not-yet-resolvable dup is deliberately NOT chained — it must not
    // conjure a phantom, resource-less proc into the projection (which would make an
    // intermediate `Dup`-before-`Alloc` state differ from the fully-applied one).
    let clients: BTreeSet<HClient> = g
        .nodes()
        .map(|n| n.key.client)
        .chain(
            g.dups()
                .filter(|(d, _)| g.origin_of(*d).is_some())
                .flat_map(|(d, s)| [d.client, s.client]),
        )
        .collect();

    // Grouping: dup edges connect client namespaces. A dup whose origin does not yet
    // resolve is a **still-parked** edge (its source `Alloc` has not arrived — the
    // order-tolerance case the rmgraph layer explicitly supports, decision #4): it is
    // not yet a grouping edge, so it is SKIPPED, never a hard fault. Turning a transient
    // parked dup into a permanent refusal would make `Gpu::apply` — which re-projects
    // after EVERY event — reject a `Dup`-before-`Alloc` ordering that the protocol
    // allows, so the SAME facts in a different order would yield a different observable
    // end-state (the whole-core determinism the differential proves). When the source
    // later allocs, `resolve_pending_dups` promotes the edge and the next projection
    // unions the clients — the union happens iff the edge is resolvable, regardless of
    // arrival order. A dup that NEVER resolves stays inert (no grouping, no alias):
    // MISS=FAULT at use, never a silent wrong-grouping.
    let mut uf = ClientUnion::new(clients.iter().copied());
    for (dst, src) in g.dups() {
        if g.origin_of(dst).is_none() {
            continue;
        }
        uf.union(dst.client, src.client);
    }

    let mut procs: BTreeMap<ProcAnchor, ProcBoundary> = BTreeMap::new();
    for &c in &clients {
        let anchor = ProcAnchor(uf.find(c));
        procs
            .entry(anchor)
            .or_insert_with(|| ProcBoundary {
                anchor,
                clients: BTreeSet::new(),
                vases: BTreeMap::new(),
                channels: BTreeMap::new(),
            })
            .clients
            .insert(c);
    }

    let mut by_pdb: BTreeMap<(GpuId, Pdb), (ProcAnchor, NodeKey)> = BTreeMap::new();
    let mut by_vchid: BTreeMap<(GpuId, VChid), (ProcAnchor, NodeKey)> = BTreeMap::new();
    // ★ The F1 collision guard's scope tables, keyed on `(Option<GpuId>, id)`: the
    // guard still bites within one target (and within the unresolved-`None` scope),
    // while identical ids on DIFFERENT targets are legal and never collide.
    let mut pdb_claims: BTreeMap<(Option<GpuId>, Pdb), NodeKey> = BTreeMap::new();
    let mut vchid_claims: BTreeMap<(Option<GpuId>, VChid), NodeKey> = BTreeMap::new();

    // Pre-pass: the engine-object refinement (channel origin → EngineKind). An
    // engine object's parent is its channel (same namespace, dup-aliases resolved).
    // `nodes()` iterates ascending origin-key order, so with several engine objects
    // on one channel the smallest-key one wins — deterministic and order-independent
    // (the real protocol allocates one engine object per channel context).
    let mut engine_refine: BTreeMap<NodeKey, EngineKind> = BTreeMap::new();
    for node in g.nodes() {
        // The engine-object's parent must typed-resolve to a Channel (any engine —
        // discriminant match) before its kind refines that channel; a hostile engine
        // object parented on a non-channel never refines anyone (decision #18C).
        if let ObjectKind::EngineObject { engine } = node.kind
            && let Some(chan) = g.origin_of_kind(
                NodeKey::new(node.key.client, node.parent),
                ObjectKind::Channel { engine: EngineKind::GrCompute },
            )
        {
            engine_refine.entry(chan.key).or_insert(engine);
        }
    }

    for node in g.nodes() {
        let anchor = ProcAnchor(uf.find(node.key.client));
        match node.kind {
            ObjectKind::VaSpace => {
                let pdb = g.pdb_of(node.key);
                let gpu = g.gpu_of(node.key);
                let boundary = procs.get_mut(&anchor).expect("component exists");
                boundary.vases.insert(node.key, VasFacts { gpu, pdb });
                if let Some(pdb) = pdb {
                    // The F1 guard, scoped per target: same-scope duplicate = loud
                    // refusal; identical PDB on a different GPU never collides.
                    if let Some(&prev) = pdb_claims.get(&(gpu, pdb))
                        && prev != node.key
                    {
                        return Err(ProjectionError::PdbCollision {
                            gpu,
                            pdb,
                            a: prev,
                            b: node.key,
                        });
                    }
                    pdb_claims.insert((gpu, pdb), node.key);
                    // Only a resolved target routes; an unresolved one defers (MISS
                    // at use) until its Device fact lands.
                    if let Some(gpu) = gpu {
                        by_pdb.insert((gpu, pdb), (anchor, node.key));
                    }
                }
            }
            ObjectKind::Channel { engine } => {
                let vchid = arch.vchid_from_userd_flags(node.facts.userd_flags);
                let gpu = g.gpu_of(node.key);
                let vas = resolve_channel_vas(g, node);
                let facts = ChannelFacts {
                    vchid,
                    gpu,
                    vas_origin: vas.map(|v| v.key),
                    vas_pdb: vas.and_then(|v| g.pdb_of(v.key)),
                    // The engine-object refinement wins over the channel-class default.
                    engine: engine_refine.get(&node.key).copied().unwrap_or(engine),
                };
                // The F1 guard, scoped per target (see the PDB arm above).
                if let Some(&prev) = vchid_claims.get(&(gpu, vchid))
                    && prev != node.key
                {
                    return Err(ProjectionError::VchidCollision {
                        gpu,
                        vchid,
                        a: prev,
                        b: node.key,
                    });
                }
                vchid_claims.insert((gpu, vchid), node.key);
                if let Some(gpu) = gpu {
                    by_vchid.insert((gpu, vchid), (anchor, node.key));
                }
                procs
                    .get_mut(&anchor)
                    .expect("component exists")
                    .channels
                    .insert(node.key, facts);
            }
            _ => {}
        }
    }

    Ok(Boundaries { procs: procs.into_values().collect(), by_pdb, by_vchid })
}
