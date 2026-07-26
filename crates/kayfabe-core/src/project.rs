//! Pure projections of the [`crate::rmgraph::RmGraph`] — `Proc` grouping, `by_pdb`,
//! `by_vchid` (arch doc §4.3.1a "Derivation rules").
//!
//! These are **deterministic pure functions of the graph** (plus, for grouping, the
//! condemned client set — see the condemnation line below, which is itself independent
//! of event order): a reordered or retried guest yields the same graph, so it yields the
//! same boundaries (the shuffle property test in this crate is the executable statement
//! of that guarantee).
//! Nothing here is accreted from observed event order, and nothing here mutates —
//! the runtime ([`crate::gpu::Gpu`]) *syncs* its owned state to these boundaries.
//!
//! Derivation rules:
//! - **`Vas` (PDB) = the address-plane owner.** A channel resolves its VASpace via
//!   declared facts only: `hVASpace`, else its CtxShare's, else its parent TSG's.
//! - **`Channel` (vChid) = the exec-plane owner** (E0's demux identity).
//! - **`Proc` = the grouping node** (isolate + arena + lifecycle only): one
//!   dup-connected component of **user** clients, plus one reserved **system**
//!   component holding every kernel client. Never inferred from timing.
//!
//! ## ★ The grouping rule, and the measurement that fixed it (§12.27)
//!
//! The rule used to be "one dup-connected component of clients", full stop, on the
//! reading that UVM keeps a *per-process* gpu-ops client. **It does not.**
//! `nvUvmInterfaceSessionCreate` runs once per `nvidia_uvm` module load and every guest
//! CUDA process dups into that one client — measured on an RTX 3060 / 580.159.04 with
//! kprobes on RM's dup funnel: two concurrent processes issued 82 dups each, *every one*
//! with the same destination (`0xc1d00069`), and a third process run later joined the
//! same destination. Dups with the session as **source**: 0. Dups with a user client as
//! **destination**: 0. It is a strict one-directional star into the session client.
//!
//! Under the old rule that star collapses **every guest process plus the guest kernel
//! into a single `Proc`** — one isolate, one arena, one host VAS — which is #14
//! un-fixed; and the second process would not even reach that, because its UVM dup
//! merges a component that has already touched its data plane
//! (`GpuError::LateMerge`).
//!
//! So the edge predicate is now typed, on a fact the client *declared about itself*
//! ([`kayfabe_arch::ClientKind`], from the `processID` on its `NV01_ROOT`):
//!
//! > **A `DUP_OBJECT` edge is a GROUPING edge iff BOTH endpoints are declared
//! > [`ClientKind::User`] clients. Every declared [`ClientKind::Kernel`] client belongs
//! > to the ONE reserved system component ([`SYSTEM_ANCHOR`]), by rule and never by
//! > dup. A client that has not (yet) declared merges with nobody.**
//!
//! A dup into a kernel client is therefore a **reference**, not a merge — which is
//! exactly what it is on the wire. User↔user dups still merge, because that is genuine
//! sharing and genuine sharing is one blast radius, which is what #14 is about.
//!
//! ## ★★ The second half of the predicate — the CONDEMNATION LINE (§12.37, C1)
//!
//! A dup edge additionally merges only when **both endpoints are on the same side of
//! the condemnation line** ([`Spine::condemned`](crate::gpu::Spine)): both alive, or
//! both already dead.
//!
//! Condemnation is a *completed fact about a set of clients* — their host data is gone,
//! because the isolate process that owned it died. A live client that later aliases a
//! condemned client's resource does not thereby acquire the corpse's history; it
//! acquires exactly one dead resource, which is attributed to its ORIGIN (the condemned
//! client) and is therefore already refused by name (`FwdFault::Condemned`) wherever it
//! is reached. Merging instead **transfers the fatality**, and the direction of the
//! transfer is chosen by whoever issued the `DUP_OBJECT` — which is how a hostile stream
//! earned a *bystander's* death:
//!
//! ```text
//! A: Alloc(Client cA, User) … A's worker dies       ⇒ cA condemned
//! A: Dup { src: (cA, obj), dst: (cV, 0x7777) }      ⇒ accepted, inert (cV undeclared)
//! V: Alloc(Client cV, User)                          ⇒ V's OWN first event
//!    …used to make the parked edge a grouping edge, on the very apply that first
//!    creates V's boundary — so V had no live `Proc` yet, the merge was not a loud
//!    `CondemnedMerge`, and V died SILENTLY, anchored at the attacker's client.
//! ```
//!
//! The predicate is what removes that at the root, rather than making the silent arm
//! loud: refusing V's own `Alloc` would still be V paying for A's action. With the line
//! in the predicate every component is **homogeneous** — an allowed edge never has
//! exactly one condemned end, so a component is either wholly condemned or wholly alive
//! — which is also why a merge can no longer be a fault at all (`Spine::plan_refresh`).
//!
//! Three properties worth naming, because each was a way to get this wrong:
//! - **The handle VALUE is never consulted.** UVM's session (`0xc1d00069`) sits
//!   numerically *between* the two user clients that dup into it.
//! - **The classification cannot depend on the dup graph**, because it is declared at
//!   client-creation time, before any dup exists. That is what keeps grouping
//!   order-independent under the new rule as well as the old.
//! - **Attribution is by ORIGIN.** [`RmGraph::nodes`] reports each resource at the
//!   handle that *allocated* it, so a user's VASpace dup'd into the UVM session stays in
//!   the user's `Proc`. The kernel component owns only what the kernel itself allocated
//!   — so the new cross-`Proc` reference materializes no second `Vas` and no second
//!   backing (`l1_concurrency.md` §12.27, the coherence re-verification).

use std::collections::{BTreeMap, BTreeSet};

use kayfabe_arch::ids::{EngineKind, GpuId, HClient, HObject, Pdb, VChid};
use kayfabe_arch::{Arch, ClientKind, ObjectKind};

use crate::ProcAnchor;
use crate::rmgraph::{ClientId, NodeKey, RESERVED_CLIENT, ResourceKey, RmGraph, RmNode};

/// ★ §12.27 — the reserved anchor of the **system component**: the guest *kernel*'s
/// clients (UVM's session, RM's internal clients), which are one component by rule.
///
/// It is [`RESERVED_CLIENT`] (`HClient(0)` = `NV01_NULL_OBJECT`), which
/// [`RmGraph::apply`] refuses as guest input — so no user component can ever anchor here
/// and be mistaken for the system proc. The label is a *reservation*, not a coincidence.
pub const SYSTEM_ANCHOR: ProcAnchor = ProcAnchor(RESERVED_CLIENT);

/// Declared facts of one VASpace origin, resolved against the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VasFacts {
    /// The owning GPU target, derived via the VASpace's `Device` ancestor
    /// ([`RmGraph::gpu_of`]). `None` = not yet resolvable (no Device ancestor / no
    /// declared instance): the VAS is not routable until the fact lands — **DEFER** here,
    /// a loud MISS at use, never a default-GPU0 guess.
    pub gpu: Option<GpuId>,
    /// The declared PDB, once `SET_PAGE_DIRECTORY` arrives. `None` ⇒ **DEFER**: the
    /// guest legitimately allocates a VASpace before binding its page directory.
    pub pdb: Option<Pdb>,
}

/// Declared facts of one channel, fully resolved against the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelFacts {
    /// The exec-plane identity, recovered by the arch from declared flags.
    pub vchid: VChid,
    /// The channel's own GPU target, derived via its `Device` ancestor
    /// ([`RmGraph::gpu_of`]). `None` = not routable (yet) ⇒ **DEFER**: the channel enters
    /// no routing map and materializes no runtime state until the fact resolves. Its
    /// doorbell then misses `by_vchid` and takes a named `FwdFault::UnknownVchid`.
    pub gpu: Option<GpuId>,
    /// The VASpace RESOURCE this channel is bound to (dup-aliases resolved), if any
    /// declared path exists. `None` = GSP-managed with no declared VAS (routed to
    /// the system/minted VAS by higher layers — out of scope this milestone).
    ///
    /// ★★★ §12.41 — a [`ResourceKey`], not an origin handle: a channel legitimately binds
    /// its `hVASpace` through a `DUP_OBJECT` alias, and the alias may resolve to a
    /// dup-kept GHOST whose origin handle the guest has since re-allocated.
    pub vas_origin: Option<ResourceKey>,
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

/// One derived process boundary: a dup-connected component of **user** clients, or the
/// single reserved **system** component (every declared kernel client — see
/// [`SYSTEM_ANCHOR`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcBoundary {
    /// Deterministic label: the smallest client handle in the component.
    pub anchor: ProcAnchor,
    /// All clients in the component.
    pub clients: BTreeSet<HClient>,
    /// VASpace **resources** owned by this component → resolved facts (target GPU and
    /// declared PDB). A process may own several (compute + UVM) — one `Proc`, many `Vas`.
    ///
    /// ★★★ §12.41 — keyed by [`ResourceKey`], **not** by origin `NodeKey`. RM recycles
    /// object-handle values with no quarantine (`ogkm rs_client.c:1137` frees,
    /// `:1446-1470` validates only against the LIVE map), so
    /// `Alloc (A,H) → Dup to B → Free (A,H) → Alloc (A,H)` leaves two live VASpace
    /// resources reporting at one `(client, handle)`. Keyed by handle, the second
    /// `insert` silently **overwrote** the first: the dup-kept ghost — which RM says is
    /// alive and which `cross_proc_lifetime.rs` requires to stay usable — vanished from
    /// the projection, so its `Pdb` left `by_pdb`, its runtime `Vas` was staged for
    /// release, and the alias holder's every address op took `FwdFault::UnknownPdb`.
    pub vases: BTreeMap<ResourceKey, VasFacts>,
    /// Channel **resources** owned by this component → resolved facts.
    ///
    /// ★★★ §12.41 — see [`Self::vases`] for why this is a [`ResourceKey`]. The exec
    /// plane's version of the collapse was the sharper one: `Gpu::sync_proc_to_boundary`
    /// mints a stable `ChanId` per key, so two live channels at one recycled handle value
    /// were handed ONE `ChanId` — one runtime `Channel`, one host channel, one host
    /// token — while `by_vchid` filed BOTH their vChids onto it.
    pub channels: BTreeMap<ResourceKey, ChannelFacts>,
}

/// The full derived routing picture. Pure data; `PartialEq` so the
/// order-independence property is directly assertable.
///
/// ★ Routing keys are `(GpuId, Pdb)` / `(GpuId, VChid)` — `Pdb`/`VChid` are
/// **per-GPU namespaces** (two GPUs legally present identical values), so the target
/// is part of the key by construction (the #14 lesson lifted onto the GPU axis).
/// Only objects whose GPU target resolves enter routing; an unresolvable target is a
/// deferred/unroutable object (loud MISS at use), never a guessed GPU0 entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Boundaries {
    /// **User** process boundaries, ascending anchor order. Never contains a declared
    /// kernel client (§12.27) and never carries [`SYSTEM_ANCHOR`].
    pub procs: Vec<ProcBoundary>,
    /// ★ §12.27 — the **system** component: every declared [`ClientKind::Kernel`]
    /// client, with the VASpaces and channels those clients themselves allocated.
    /// Always present (empty before the guest kernel declares a client), always anchored
    /// at [`SYSTEM_ANCHOR`], and — unlike a user boundary — never minted, merged,
    /// retired or condemned: its clients are the guest driver's, so condemning it is
    /// device-fatal by definition (§12.26).
    pub system: ProcBoundary,
    /// Data-plane routing: (target, PDB) → (owning component, VASpace resource).
    pub by_pdb: BTreeMap<(GpuId, Pdb), (ProcAnchor, ResourceKey)>,
    /// Exec-plane routing: (target, vChid) → (owning component, channel resource). The
    /// owning component may be [`SYSTEM_ANCHOR`] (a guest-kernel channel).
    pub by_vchid: BTreeMap<(GpuId, VChid), (ProcAnchor, ResourceKey)>,
}

impl ProcBoundary {
    /// An empty boundary for `anchor` (no clients, no vases, no channels).
    #[must_use]
    fn empty(anchor: ProcAnchor) -> Self {
        ProcBoundary {
            anchor,
            clients: BTreeSet::new(),
            vases: BTreeMap::new(),
            channels: BTreeMap::new(),
        }
    }
}

impl Default for Boundaries {
    /// The boundaries of an empty graph: no user procs, an empty system component.
    fn default() -> Self {
        Boundaries {
            procs: Vec::new(),
            system: ProcBoundary::empty(SYSTEM_ANCHOR),
            by_pdb: BTreeMap::new(),
            by_vchid: BTreeMap::new(),
        }
    }
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
        /// First claimant (★ §12.41 — by resource identity: the guard used to compare
        /// origin `NodeKey`s, so two DIFFERENT live resources sharing one recycled handle
        /// value read as "the same claimant re-declaring" and passed).
        a: ResourceKey,
        /// Second claimant.
        b: ResourceKey,
    },
    /// Two distinct VASpace origins on ONE target declare the same PDB.
    PdbCollision {
        /// The target GPU scope of the collision (`None` = unresolved-target scope).
        gpu: Option<GpuId>,
        /// The colliding PDB.
        pdb: Pdb,
        /// First claimant (★ §12.41 — by resource identity; see
        /// [`Self::VchidCollision`]).
        a: ResourceKey,
        /// Second claimant.
        b: ResourceKey,
    },
}

/// Tiny deterministic union-find over client handles.
struct ClientUnion {
    parent: BTreeMap<HClient, HClient>,
}

impl ClientUnion {
    fn new(clients: impl IntoIterator<Item = HClient>) -> Self {
        ClientUnion {
            parent: clients.into_iter().map(|c| (c, c)).collect(),
        }
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
///
/// **MISS ⇒ DEFER here, FAULT at use** (`kayfabe_core` crate docs, the miss taxonomy).
/// Deferring is right for the not-yet-arrived case; the wrong-kind case is never
/// knowable and is fused into the same `None` by [`RmGraph::origin_of_kind`] — see that
/// function for why, and `l1_concurrency.md` §12.30 finding B for the open question.
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
///
/// **MISS ⇒ DEFER** (same category and same caveat as [`resolve_vaspace_handle`]): the
/// channel materializes with `vas_pdb: None` and rings nothing, because
/// `kayfabe_fwd::gate_working_set_in` refuses a channel with no VAS by name.
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

/// ★ The condemnation input of an **un-condemned** device — the value every caller that
/// is not [`crate::gpu::Spine::refresh`] passes to [`project`].
///
/// It is a named constant rather than a defaulted argument so that "this projection
/// considers no client dead" is a statement the call site makes, not one it omits.
pub static NO_CONDEMNED: BTreeSet<HClient> = BTreeSet::new();

/// Derive the full boundary picture from the graph.
///
/// A deterministic pure function of `(graph, condemned)` — order-independent by
/// construction, because it looks only at the graph's declared facts and at a client set
/// that is itself independent of event order.
///
/// `condemned` is the flattened [`crate::gpu::Spine::condemned`] client set: the clients
/// whose component died out of band. It participates in exactly one decision — the
/// grouping predicate's condemnation line (module docs) — and in nothing else; pass
/// [`NO_CONDEMNED`] when there is none.
pub fn project(
    g: &RmGraph,
    arch: &dyn Arch,
    condemned: &BTreeSet<HClient>,
) -> Result<Boundaries, ProjectionError> {
    // Client universe: explicit resource-owning namespaces + the endpoints of every
    // **resolved** dup (a dst client that only receives an alias and allocates nothing
    // has no origin node of its own, so it must be picked up here to be grouped). A
    // still-parked, not-yet-resolvable dup is deliberately NOT chained — it must not
    // conjure a phantom, resource-less proc into the projection (which would make an
    // intermediate `Dup`-before-`Alloc` state differ from the fully-applied one).
    // ★ §12.27 — the declared privilege of every client whose root has been observed.
    // Read once, up front, because it gates the client universe, the edge predicate and
    // the assignment of every node below. A client absent from this map has not declared
    // itself yet.
    let kinds: BTreeMap<HClient, ClientKind> = g.client_kinds().collect();
    let is_user = |c: HClient| matches!(kinds.get(&c), Some(ClientKind::User { .. }));
    // ★★★ §12.39, finding 1 — the ASSIGNMENT pass asks a different question from the
    // grouping predicate, and used to answer it with a default.
    //
    // A resource **outlives its namespace**: a foreign `DUP_OBJECT` alias keeps it alive
    // after its origin client's root is freed (faithful RM refcounting), and the owning
    // `Proc` must survive with it — retiring it would free host memory RM still says is
    // live, and `cross_proc_lifetime.rs` pins that end to end. So the `g.nodes()` branch
    // below deliberately admits a namespace with no live root, and `anchor_of` then had
    // to classify it with **nothing to read**: `is_kernel` was false by absence, so the
    // orphan was filed as a **user** boundary and the spine minted an isolate, a GPA
    // arena and a routable `Vas` for it — *including when the dead namespace was the
    // guest KERNEL's*, which is the guest-kernel-obtains-a-user-data-plane shape
    // `FwdFault::SystemDataPlane` exists to forbid.
    //
    // The fix is not a filter (that would retire a `Proc` RM says is live); it is to stop
    // reading an absence. [`RmGraph::client_declarations`] carries the kind the namespace
    // **declared**, recorded on the resource at its alloc, so an orphan is classified by
    // a fact instead of by a default. `is_user` stays on the LIVE-root map on purpose:
    // grouping requires positive evidence about a live declaration at both ends, which is
    // §12.27's rule and is unchanged.
    let decls: BTreeMap<HClient, (ClientId, ClientKind)> = g.client_declarations();
    let is_kernel = |c: HClient| matches!(decls.get(&c), Some((_, ClientKind::Kernel)));

    // ★★★ §12.39 Part B — **THE RECYCLED NAMESPACE.** A resource projects under its
    // origin `HClient` only while its recorded owner ([`ClientId`], never reused) is still
    // the declaration that namespace projects under. The `HClient` in `node.key` is a
    // *value*, and RM recycles values by design — caller-supplied roots
    // (`ogkm rs_server.c:612`, the reject guard compiled out at `:613-616` under
    // `RS_COMPATABILITY_MODE=1`) and a generator that wraps at 2^20 with no free list and
    // no epoch (`:3319-3341`). So an orphan resource whose namespace was freed and then
    // handed to an **unrelated later process** used to re-attach itself to that process:
    // it entered the victim's boundary, its dup edge became a grouping edge, and the two
    // became one `Proc` — one isolate, one GPA arena, one host VAS. #14 un-fixed for a
    // pair the attacker chose.
    //
    // Note what this does NOT do: while the namespace has **no** live root the orphan's
    // own declaration is still the one it projects under, so its component (and its
    // isolate, arena and host VAS) survives — which is `cross_proc_lifetime.rs`'s rule and
    // RM's refcount. The exclusion bites at exactly one moment: a **re-declaration**,
    // which mints a new `ClientId` and leaves the old declaration's resources owned by a
    // namespace that no longer projects. They belong to no boundary, so the old component
    // is retired through the ordinary staged-death path and the new one inherits nothing.
    let projects =
        |n: &RmNode, owner: ClientId| decls.get(&n.key.client).is_some_and(|&(id, _)| id == owner);

    let clients: BTreeSet<HClient> = g
        .nodes_with_owner()
        .filter(|&(n, owner)| projects(n, owner))
        .map(|(n, _)| n.key.client)
        .chain(
            g.dups()
                .filter(|(d, _)| g.origin_of(*d).is_some())
                .flat_map(|(d, s)| [d.client, s.client])
                // ★ §12.27 — a dup endpoint that owns NO resource of its own is chained
                // in only once it has DECLARED itself. It is chained in at all so a
                // client holding nothing but aliases can still merge with the client it
                // aliases; but with no declared root there is no grouping it could join
                // (the edge predicate needs positive evidence about both ends), so
                // admitting it would mint exactly the resource-less phantom `Proc` the
                // parked-dup rule below already refuses to conjure — one that is then
                // minted, matched, and RETIRED the moment a declaration lands.
                //
                // ★ §12.38 narrowed what this filter can still see: a dup endpoint's
                // namespace is declared at acceptance time, so the only way to reach here
                // undeclared is a root that has since been FREED while an alias keeps its
                // resources alive. The filter is kept — that state is reachable, and
                // "absence is never read as user" is the rule it encodes.
                .filter(|c| kinds.contains_key(c)),
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
    // ★★★ §12.38 — what the `Dup`-before-`Alloc` deferral now covers, exactly. Both
    // client NAMESPACES are guaranteed declared by the time any dup reaches the graph
    // (`RmGraph::undeclared_namespace` refuses otherwise — a dup into a namespace that
    // does not exist is `NV_ERR_INVALID_CLIENT` on real RM, and accepting it let a planted
    // alias merge an unrelated later process into the planter's `Proc`). What is still
    // legitimately deferred here is the dup's **source object**, which is
    // DEFER-for-observation and not a courtesy: only 25 of 82 measured dups reach GSP, so
    // a source may be an object RM saw and we did not. Faulting it would hang a legal
    // guest; the deferral resolves the moment the source is observed
    // (`tests/miss_taxonomy.rs`).
    let mut uf = ClientUnion::new(clients.iter().copied());
    for (dst, src) in g.dups() {
        if g.origin_of(dst).is_none() {
            continue;
        }
        // ★ §12.27 — THE GROUPING PREDICATE. A dup merges only when BOTH ends are
        // declared **user** clients: that is genuine sharing between two guest
        // processes, which is one blast radius by definition. Every other shape is a
        // *reference* and merges nothing:
        //
        //  - dst is a KERNEL client — the measured case. Every guest CUDA process dups
        //    into the one UVM session client; merging on those edges collapses the whole
        //    guest into a single `Proc` (#14 un-fixed) and makes process #2 a
        //    `LateMerge` refusal. The kernel client is the system component's, by rule.
        //  - src is a kernel client (0 observed) — symmetric, and refusing it means a
        //    kernel client can never *pull* a user proc into the system component.
        //  - either end is undeclared (its root has not arrived) — grouping requires
        //    positive evidence about BOTH sides; absence is never read as "user".
        //
        // Requiring positive evidence, rather than excluding the known-bad shapes, is
        // deliberate: a future third [`ClientKind`] is then unmergeable until someone
        // decides what it means, instead of silently defaulting into user grouping.
        if !(is_user(dst.client) && is_user(src.client)) {
            continue;
        }
        // ★★★ §12.39 Part B — and the SOURCE must still be owned by the namespace the
        // `src` key names. A dup edge reports its source as the resource's **origin**
        // `(client, handle)`, and that `HClient` is a recyclable value: an attacker that
        // allocates a VASpace in a namespace it will hand back, aliases it into a client of
        // its own, and then frees the namespace's root leaves an edge whose `src.client`
        // is a *number* the guest kernel is free to give to an unrelated later process.
        // The instant that process declares, `is_user(src.client)` flips true on ITS root
        // and the stale edge became a grouping edge — merging the victim into the
        // attacker's `Proc` on the victim's own first RM event.
        //
        // Read through the **destination** handle (live by construction) so the answer is
        // the resource's recorded owner, and compare it against the declaration `src.client`
        // currently projects under. Identical for every ordinary dup — a live namespace's
        // resources are all owned by its current root — and false exactly for the orphan of
        // a superseded declaration, which is the vector.
        if g.owner_of(dst) != decls.get(&src.client).map(|&(id, _)| id) {
            continue;
        }
        // ★★ §12.37 (C1) — THE CONDEMNATION LINE. A merge across it is the ONE way a
        // guest could make another guest process's component fatal, so it is refused
        // here, in the predicate, rather than adjudicated later:
        //
        //  - both alive   — genuine sharing between two live processes: one blast
        //    radius, exactly as before.
        //  - both dead    — the component that died, re-derived (a split-then-rejoined
        //    corpse); it stays one condemned component.
        //  - one of each  — a REFERENCE, never a merge. The dead side's resources are
        //    attributed to their origin and keep answering `FwdFault::Condemned`; the
        //    live side keeps its `Proc`. Neither direction of the edge moves a client
        //    across the line, so neither a resurrect (absorbing a corpse around a
        //    working isolate) nor a bystander death (condemning a healthy proc by
        //    dupping into a corpse) is representable.
        if condemned.contains(&dst.client) != condemned.contains(&src.client) {
            continue;
        }
        uf.union(dst.client, src.client);
    }

    // Assignment: every declared kernel client goes to the ONE system component, by
    // rule; everything else groups by the union above. `ProcAnchor` values from `uf` can
    // never equal `SYSTEM_ANCHOR`, because `RmGraph::apply` refuses `RESERVED_CLIENT`.
    let mut procs: BTreeMap<ProcAnchor, ProcBoundary> = BTreeMap::new();
    let mut system = ProcBoundary::empty(SYSTEM_ANCHOR);
    let anchor_of = |uf: &mut ClientUnion, c: HClient| {
        if is_kernel(c) {
            SYSTEM_ANCHOR
        } else {
            ProcAnchor(uf.find(c))
        }
    };
    for &c in &clients {
        let anchor = anchor_of(&mut uf, c);
        if anchor == SYSTEM_ANCHOR {
            system.clients.insert(c);
        } else {
            procs
                .entry(anchor)
                .or_insert_with(|| ProcBoundary::empty(anchor))
                .clients
                .insert(c);
        }
    }

    let mut by_pdb: BTreeMap<(GpuId, Pdb), (ProcAnchor, ResourceKey)> = BTreeMap::new();
    let mut by_vchid: BTreeMap<(GpuId, VChid), (ProcAnchor, ResourceKey)> = BTreeMap::new();
    // ★ The F1 collision guard's scope tables, keyed on `(Option<GpuId>, id)`: the
    // guard still bites within one target (and within the unresolved-`None` scope),
    // while identical ids on DIFFERENT targets are legal and never collide.
    let mut pdb_claims: BTreeMap<(Option<GpuId>, Pdb), ResourceKey> = BTreeMap::new();
    let mut vchid_claims: BTreeMap<(Option<GpuId>, VChid), ResourceKey> = BTreeMap::new();

    // Pre-pass: the engine-object refinement (channel origin → EngineKind). An
    // engine object's parent is its channel (same namespace, dup-aliases resolved).
    // `nodes()` iterates ascending origin-key order, so with several engine objects
    // on one channel the smallest-key one wins — deterministic and order-independent
    // (the real protocol allocates one engine object per channel context).
    let mut engine_refine: BTreeMap<ResourceKey, EngineKind> = BTreeMap::new();
    for node in g.nodes() {
        // The engine-object's parent must typed-resolve to a Channel (any engine —
        // discriminant match) before its kind refines that channel; a hostile engine
        // object parented on a non-channel never refines anyone (decision #18C).
        if let ObjectKind::EngineObject { engine } = node.kind
            && let Some(chan) = g.origin_of_kind(
                NodeKey::new(node.key.client, node.parent),
                ObjectKind::Channel {
                    engine: EngineKind::GrCompute,
                },
            )
        {
            engine_refine.entry(chan.id()).or_insert(engine);
        }
    }

    for (node, owner) in g.nodes_with_owner() {
        // ★★★ §12.39 Part B — the same predicate the client universe used, and it MUST be
        // the same one: admitting a node here that the universe excluded would panic on
        // the `expect` below, and excluding one the universe admitted would leave an empty
        // component. A resource of a superseded declaration claims no `Pdb`/`VChid` and
        // enters no boundary, so its use takes a named MISS instead of resolving onto the
        // namespace's new tenant.
        if !projects(node, owner) {
            continue;
        }
        // ★ §12.27 — attribution is by the resource's ORIGIN client (`nodes()` reports
        // each resource at the handle that allocated it), so a user object dup'd into
        // the UVM session stays in the USER component. The system component owns only
        // what the guest kernel itself allocated.
        let anchor = anchor_of(&mut uf, node.key.client);
        let boundary = if anchor == SYSTEM_ANCHOR {
            &mut system
        } else {
            procs.get_mut(&anchor).expect("component exists")
        };
        match node.kind {
            ObjectKind::VaSpace => {
                // ★★★ §12.41 — resolved by resource IDENTITY. `pdb_of`/`gpu_of` answer
                // about whatever incarnation currently holds the handle value, so on a
                // recycled key a dup-kept ghost was given the SUCCESSOR's page-directory
                // base and target.
                let pdb = g.pdb_of_resource(node.id());
                let gpu = g.gpu_of_resource(node.id());
                boundary.vases.insert(node.id(), VasFacts { gpu, pdb });
                if let Some(pdb) = pdb {
                    // The F1 guard, scoped per target: same-scope duplicate = loud
                    // refusal; identical PDB on a different GPU never collides.
                    if let Some(&prev) = pdb_claims.get(&(gpu, pdb))
                        && prev != node.id()
                    {
                        return Err(ProjectionError::PdbCollision {
                            gpu,
                            pdb,
                            a: prev,
                            b: node.id(),
                        });
                    }
                    pdb_claims.insert((gpu, pdb), node.id());
                    // Only a resolved target routes; an unresolved one defers (MISS
                    // at use) until its Device fact lands.
                    if let Some(gpu) = gpu {
                        by_pdb.insert((gpu, pdb), (anchor, node.id()));
                    }
                }
            }
            ObjectKind::Channel { engine } => {
                let vchid = arch.vchid_from_userd_flags(node.facts.userd_flags);
                // ★★★ §12.41 — by resource identity, on both the channel and the VASpace
                // its `hVASpace` resolves to (see the VaSpace arm).
                let gpu = g.gpu_of_resource(node.id());
                let vas = resolve_channel_vas(g, node);
                let facts = ChannelFacts {
                    vchid,
                    gpu,
                    vas_origin: vas.map(RmNode::id),
                    vas_pdb: vas.and_then(|v| g.pdb_of_resource(v.id())),
                    // The engine-object refinement wins over the channel-class default.
                    engine: engine_refine.get(&node.id()).copied().unwrap_or(engine),
                };
                // The F1 guard, scoped per target (see the PDB arm above).
                if let Some(&prev) = vchid_claims.get(&(gpu, vchid))
                    && prev != node.id()
                {
                    return Err(ProjectionError::VchidCollision {
                        gpu,
                        vchid,
                        a: prev,
                        b: node.id(),
                    });
                }
                vchid_claims.insert((gpu, vchid), node.id());
                if let Some(gpu) = gpu {
                    by_vchid.insert((gpu, vchid), (anchor, node.id()));
                }
                boundary.channels.insert(node.id(), facts);
            }
            _ => {}
        }
    }

    Ok(Boundaries {
        procs: procs.into_values().collect(),
        system,
        by_pdb,
        by_vchid,
    })
}
