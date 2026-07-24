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

use nvkvm_arch::ids::{EngineKind, HClient, HObject, Pdb, VChid};
use nvkvm_arch::{Arch, ObjectKind};

use crate::ProcAnchor;
use crate::rmgraph::{NodeKey, RmGraph, RmNode};

/// Declared facts of one channel, fully resolved against the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelFacts {
    /// The exec-plane identity, recovered by the arch from declared flags.
    pub vchid: VChid,
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
    /// VASpace **origin** nodes owned by this component → declared PDB (if any).
    /// A process may own several (compute + UVM) — one `Proc`, many `Vas`.
    pub vases: BTreeMap<NodeKey, Option<Pdb>>,
    /// Channel nodes owned by this component → resolved facts.
    pub channels: BTreeMap<NodeKey, ChannelFacts>,
}

/// The full derived routing picture. Pure data; `PartialEq` so the
/// order-independence property is directly assertable.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Boundaries {
    /// Process boundaries, ascending anchor order.
    pub procs: Vec<ProcBoundary>,
    /// Data-plane routing: PDB → (owning component, VASpace origin node).
    pub by_pdb: BTreeMap<Pdb, (ProcAnchor, NodeKey)>,
    /// Exec-plane routing: vChid → (owning component, channel node).
    pub by_vchid: BTreeMap<VChid, (ProcAnchor, NodeKey)>,
}

/// Projection failures. All loud: each is a real protocol violation or a graph
/// inconsistency that must never be silently resolved (MISS=FAULT posture).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionError {
    /// Two live channels decode to the same vChid (E0 says this cannot happen on a
    /// sane guest; if it does, demux would be ambiguous — refuse loudly).
    VchidCollision {
        /// The colliding vChid.
        vchid: VChid,
        /// First claimant.
        a: NodeKey,
        /// Second claimant.
        b: NodeKey,
    },
    /// Two distinct VASpace origins declare the same PDB.
    PdbCollision {
        /// The colliding PDB.
        pdb: Pdb,
        /// First claimant.
        a: NodeKey,
        /// Second claimant.
        b: NodeKey,
    },
    /// A dup edge whose origin cannot be resolved in the complete graph.
    DanglingDup {
        /// The alias with no resolvable source.
        dst: NodeKey,
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
    let node = g.origin_of(NodeKey::new(ns, handle))?;
    matches!(node.kind, ObjectKind::VaSpace).then_some(node)
}

/// Resolve a channel node's VASpace origin per the declared-facts precedence:
/// own `hVASpace` → CtxShare's → parent TSG's. Every hop must land on an actual
/// VASpace (see [`resolve_vaspace_handle`]).
fn resolve_channel_vas<'g>(g: &'g RmGraph, chan: &RmNode) -> Option<&'g RmNode> {
    let ns = chan.key.client;
    if let Some(hv) = chan.facts.h_vaspace {
        return resolve_vaspace_handle(g, ns, hv);
    }
    if let Some(hcs) = chan.facts.h_ctx_share
        && let Some(cs) = g.origin_of(NodeKey::new(ns, hcs))
        && matches!(cs.kind, ObjectKind::CtxShare)
        && let Some(hv) = cs.facts.h_vaspace
    {
        return resolve_vaspace_handle(g, cs.key.client, hv);
    }
    // Parent may be a TSG that declares the VAS.
    if let Some(parent) = g.origin_of(NodeKey::new(ns, chan.parent))
        && matches!(parent.kind, ObjectKind::Tsg)
        && let Some(hv) = parent.facts.h_vaspace
    {
        return resolve_vaspace_handle(g, parent.key.client, hv);
    }
    None
}

/// Derive the full boundary picture from the graph. Pure; order-independent by
/// construction (it looks only at the graph's declared facts).
pub fn project(g: &RmGraph, arch: &dyn Arch) -> Result<Boundaries, ProjectionError> {
    // Client universe: explicit client-root nodes + every referenced namespace.
    let clients: BTreeSet<HClient> = g
        .nodes()
        .map(|n| n.key.client)
        .chain(g.dups().flat_map(|(d, s)| [d.client, s.client]))
        .collect();

    // Grouping: dup edges connect client namespaces.
    let mut uf = ClientUnion::new(clients.iter().copied());
    for (dst, src) in g.dups() {
        if g.origin_of(dst).is_none() {
            return Err(ProjectionError::DanglingDup { dst });
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

    let mut by_pdb: BTreeMap<Pdb, (ProcAnchor, NodeKey)> = BTreeMap::new();
    let mut by_vchid: BTreeMap<VChid, (ProcAnchor, NodeKey)> = BTreeMap::new();

    // Pre-pass: the engine-object refinement (channel origin → EngineKind). An
    // engine object's parent is its channel (same namespace, dup-aliases resolved).
    // `nodes()` iterates ascending origin-key order, so with several engine objects
    // on one channel the smallest-key one wins — deterministic and order-independent
    // (the real protocol allocates one engine object per channel context).
    let mut engine_refine: BTreeMap<NodeKey, EngineKind> = BTreeMap::new();
    for node in g.nodes() {
        if let ObjectKind::EngineObject { engine } = node.kind
            && let Some(chan) = g.origin_of(NodeKey::new(node.key.client, node.parent))
            && matches!(chan.kind, ObjectKind::Channel { .. })
        {
            engine_refine.entry(chan.key).or_insert(engine);
        }
    }

    for node in g.nodes() {
        let anchor = ProcAnchor(uf.find(node.key.client));
        match node.kind {
            ObjectKind::VaSpace => {
                let pdb = g.pdb_of(node.key);
                let boundary = procs.get_mut(&anchor).expect("component exists");
                boundary.vases.insert(node.key, pdb);
                if let Some(pdb) = pdb {
                    if let Some(&(_, prev)) = by_pdb.get(&pdb)
                        && prev != node.key
                    {
                        return Err(ProjectionError::PdbCollision { pdb, a: prev, b: node.key });
                    }
                    by_pdb.insert(pdb, (anchor, node.key));
                }
            }
            ObjectKind::Channel { engine } => {
                let vchid = arch.vchid_from_userd_flags(node.facts.userd_flags);
                let vas = resolve_channel_vas(g, node);
                let facts = ChannelFacts {
                    vchid,
                    vas_origin: vas.map(|v| v.key),
                    vas_pdb: vas.and_then(|v| g.pdb_of(v.key)),
                    // The engine-object refinement wins over the channel-class default.
                    engine: engine_refine.get(&node.key).copied().unwrap_or(engine),
                };
                if let Some(&(_, prev)) = by_vchid.get(&vchid)
                    && prev != node.key
                {
                    return Err(ProjectionError::VchidCollision { vchid, a: prev, b: node.key });
                }
                by_vchid.insert(vchid, (anchor, node.key));
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
