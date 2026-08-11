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
use std::fmt;

use kayfabe_arch::fault::ErrorNotifier;
use kayfabe_arch::ids::{EngineKind, GpuId, HClient, HObject, Pdb, VChid};
use kayfabe_arch::{Arch, ClientKind, ObjectKind};

use crate::ProcAnchor;
use crate::rmgraph::{ClientId, ClientKey, NodeKey, RESERVED_CLIENT, ResourceKey, RmGraph, RmNode};

/// ★ §12.27 — the reserved anchor of the **system component**: the guest *kernel*'s
/// clients (UVM's session, RM's internal clients), which are one component by rule.
///
/// It is [`RESERVED_CLIENT`] (`HClient(0)` = `NV01_NULL_OBJECT`), which
/// [`RmGraph::apply`] refuses as guest input — so no user component can ever anchor here
/// and be mistaken for the system proc. The label is a *reservation*, not a coincidence.
pub const SYSTEM_ANCHOR: ProcAnchor = ProcAnchor(ClientKey {
    client: RESERVED_CLIENT,
    incarnation: 0,
});

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
    /// ★★★★ **§16.28 — route 4's answer: the `hVASpace` naming the parent DEVICE's
    /// default address space**, when the channel declared no VA space of its own, no
    /// CtxShare, and its parent is a `Device`. `None` on every other shape, including a
    /// Device that has not named one yet ([`VasHop::DeviceDefaultUndeclared`]).
    ///
    /// # ⚠ It is a NAME, and it deliberately does not populate [`Self::vas_origin`]
    ///
    /// The resource that handle named is normally **dead**: RM mints it, publishes the
    /// address space's page-directory root under it, and frees it
    /// (`kayfabe_arch::VaSpaceRole` carries the sequence). So there is no `ResourceKey` to
    /// report and no `Pdb` to attach — [`Self::vas_pdb`] stays `None` and the channel
    /// enters no routing map, exactly as before this route existed.
    ///
    /// ★ What the name buys is the only thing it needs to: the guest's publication is
    /// filed under `(hClient, hObject)` (`kayfabe_device::gvaspub`), so a consumer holding
    /// this handle can resolve that address space's page directories. ⊘ Which consumer,
    /// and under whose authorisation, is the *dispatch's* business and not this
    /// projection's — nothing here rings anything.
    pub vas_device_default: Option<HObject>,
    /// ★★★★ §16.25 — **which of the three routes produced [`Self::vas_origin`], and what
    /// the ones that ran actually hit.** Report-only; see [`VasRoutes`] for the boot in
    /// which 15 of 24 doorbells refused `NoVas` and the report could say only that the
    /// channel itself declared nothing.
    pub vas_route: VasRoutes,
    /// ★ The fine [`EngineKind`] of this channel's context (`execution_plane.md`
    /// §2.1/§2.2): the channel *class*'s declared kind, **refined by the engine
    /// object allocated on it** (an NVENC session on a GR-class channel makes it an
    /// `NvEnc` context — distinguishable AT the channel, so routing and
    /// completion-arm selection are exact). Graph-derived, hence order/replay
    /// independent: the refinement is a pure function of the graph, not of when the
    /// engine-object alloc arrived.
    pub engine: EngineKind,
    /// ★★★ The channel's declared **error notifier** — see
    /// [`crate::rmgraph::AllocFacts::error_notifier`]. Carried through the projection
    /// unchanged, because it is a declared fact of the channel's own alloc and nothing
    /// in the graph refines it.
    ///
    /// `None` means task #111 must **not** emit a simulated fault for this channel:
    /// `RC_TRIGGERED` with no notifier write is the hang it exists to replace
    /// (`kayfabe_core::fault::NotifierGap`).
    pub error_notifier: Option<ErrorNotifier>,
}

/// One derived process boundary: a dup-connected component of **user** clients, or the
/// single reserved **system** component (every declared kernel client — see
/// [`SYSTEM_ANCHOR`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcBoundary {
    /// Deterministic label: the smallest client DECLARATION in the component (§12.42).
    pub anchor: ProcAnchor,
    /// All client **declarations** in the component.
    ///
    /// ★★★ §12.42 — a [`ClientKey`], not an `HClient`. RM recycles `hClient` values by
    /// design (`ClientKey`'s docs cite the four places), so a value is a *label* of a
    /// namespace lifetime and not an identity of one. Keyed by the value, two generations
    /// of one `hClient` were **not expressible as two components**: the newer displaced
    /// the older, the older's still-live resources belonged to no boundary, its component
    /// vanished and `stage_dropped_vases` released host memory RM still refcounted for the
    /// alias holder (§12.41 §4 / N1). Use [`Self::client_values`] where the question really
    /// is about the value.
    pub clients: BTreeSet<ClientKey>,
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
    /// ★★ **Context-object → address space**: a **channel or TSG** resource → the
    /// `(target, PDB)` whose table its GR/compute context buffers belong in.
    ///
    /// The routing input of `GPU_PROMOTE_CTX` ([`crate::promote`]), whose `hObject` the
    /// SDK documents as *"either a single channel or a channel group"*. Both real
    /// producers pass a channel handle
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics_object.c:131`,
    /// `.../kernel_graphics_context.c:2166`), and the contract admits the group — so both
    /// are derived here rather than one being refused for the convenience of the index.
    ///
    /// A TSG's own `hVASpace` is resolved through the same scoped, typed
    /// [`resolve_declared_handle`] every other declared handle fact goes through: a
    /// group belonging to a superseded client declaration resolves to nothing rather
    /// than to whatever the recycled value's next tenant allocated (★★★ §12.44).
    ///
    /// Only fully-resolved entries appear — an unresolved target or an undeclared PDB is
    /// a **DEFER**, and a promotion naming one takes
    /// [`crate::promote::PromoteFault::ContextVasUndeclared`] rather than landing
    /// somewhere plausible.
    pub ctx_vas: BTreeMap<ResourceKey, (GpuId, Pdb)>,
}

impl ProcBoundary {
    /// The `hClient` VALUES this component's declarations were made at — for the callers
    /// whose question genuinely is about the value ("is namespace X in this component?"),
    /// which is most diagnostics and most assertions.
    ///
    /// It is a *lossy* view by construction: two generations of one value collapse to one
    /// entry here, which is precisely the collapse [`Self::clients`] exists to stop making
    /// silently.
    #[must_use]
    pub fn client_values(&self) -> BTreeSet<HClient> {
        self.clients.iter().map(|k| k.client).collect()
    }

    /// ★★★★★ **THE ONE DERIVATION of a channel's guest-facing kind** — the owner's
    /// 2026-08-11 split, resolved here and nowhere else (see
    /// [`crate::channel_kind`] for the whole model and for what it refutes).
    ///
    /// Every channel in [`Self::channels`] is presented to whatever privilege level owns
    /// this component, so the kind is a property of the **boundary** and not of the
    /// channel's own `RM_ALLOC` — which is exactly why it is not a field of
    /// [`ChannelFacts`]. That struct's fields are read off the channel's own alloc; this
    /// one is read off *whose namespace allocated it*, one hop away, and putting it there
    /// would have made the two look like the same kind of fact.
    ///
    /// # ⊘ Why the ANCHOR and not a `ClientKind` scan over [`Self::clients`]
    ///
    /// They cannot disagree — [`SYSTEM_ANCHOR`] is [`RESERVED_CLIENT`], which
    /// [`RmGraph::apply`] refuses as guest input, so no user component can anchor there
    /// (`:107`) and a user boundary *"never carries `SYSTEM_ANCHOR`"* (`Boundaries::procs`)
    /// — but only one of them is a **total** function of one field. A scan has to decide
    /// what an empty client set, or a mixed one, means; §12.27's rule is that every
    /// declared [`ClientKind::Kernel`] client is already folded into the ONE system
    /// component before this is asked, so the fold has happened and re-doing it here would
    /// be a second projection of one fact.
    #[must_use]
    pub fn channel_kind(&self) -> crate::channel_kind::GuestChannelKind {
        if self.anchor == SYSTEM_ANCHOR {
            crate::channel_kind::GuestChannelKind::Emulated
        } else {
            crate::channel_kind::GuestChannelKind::Passthrough
        }
    }

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
            ctx_vas: BTreeMap::new(),
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
    /// ★★ A channel's `NVOS04_FLAGS` word **names no channel at all**.
    ///
    /// Sits beside [`Self::VchidCollision`] because it is the other half of the same
    /// question: that one is *two channels claiming one vChid*, this one is *a channel
    /// claiming none*. [`kayfabe_arch::Arch::vchid_from_userd_flags`] returns `Option`
    /// precisely so this case has somewhere to go — RM's own reader has two arms where the
    /// flags do not carry a chid (`_PAGE_FIXED` false leaves it to the allocator;
    /// `_PAGE_FIXED` and `_FIXED` both true is `NV_ERR_INVALID_STATE`), and boundary-1
    /// posture is that a guest can send either.
    ///
    /// ⊘ Refusing is the *whole* answer here. There is no fallback vChid to pick: any
    /// choice would put this channel in the exec-plane index under a number the guest
    /// never asked for, which is the silent mis-route the `Option` was widened to prevent.
    UnnamedVchid {
        /// The channel that declared them.
        channel: ResourceKey,
        /// The word itself, verbatim, so the refusal can be read against the driver.
        userd_flags: u32,
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

/// Tiny deterministic union-find over client DECLARATIONS (§12.42).
struct ClientUnion {
    parent: BTreeMap<ClientKey, ClientKey>,
}

impl ClientUnion {
    fn new(clients: impl IntoIterator<Item = ClientKey>) -> Self {
        ClientUnion {
            parent: clients.into_iter().map(|c| (c, c)).collect(),
        }
    }

    fn find(&mut self, c: ClientKey) -> ClientKey {
        let p = *self.parent.entry(c).or_insert(c);
        if p == c {
            return c;
        }
        let root = self.find(p);
        self.parent.insert(c, root);
        root
    }

    /// Union by minimum declaration so the representative IS the anchor (deterministic).
    /// ★★★ §12.42 — [`ClientKey`] orders by `hClient` first, so "the smallest client in
    /// the component" is unchanged for every component that holds one declaration per
    /// value, which is every component a guest that does not recycle can build.
    fn union(&mut self, a: ClientKey, b: ClientKey) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        self.parent.insert(hi, lo);
    }
}

/// ★★★ §12.44 — resolve ONE declared handle fact of a resource, **inside the namespace
/// DECLARATION that allocated it**, and only if that declaration still owns the namespace.
///
/// ## What this closes (the §12.41 family's last member)
///
/// A `hVASpace` / `hContextShare` / `parent` is a handle **into the allocating client's
/// own handle table**. That table belongs to a *declaration*, not to an `hClient` VALUE —
/// and RM recycles the value by design (`ClientKey`'s docs cite the four places). Freeing
/// a client root destroys every handle in the namespace, so the table a live handle sits
/// in is always the **live-rooted** declaration's. A resource that outlives its
/// namespace's root (a foreign `DUP_OBJECT` alias keeps it — faithful RM refcounting) is
/// therefore a resource whose declared handle facts name a table that **no longer exists**.
///
/// Resolving them anyway, through the bare `HClient`, hands the ghost whatever the value's
/// NEXT tenant allocated at that handle number:
///
/// ```text
/// N: Alloc (N,chan) Channel, hVASpace = (N,0x110)   ⇒ the VAS fact, declared
/// A: Dup { src: (N,chan), dst: (A,alias) }          ⇒ A keeps the channel alive
/// N: Free (N,root)                                   ⇒ N's table is gone; the channel is a GHOST
/// N: Alloc (N,root) — a DIFFERENT process              ⇒ the value is recycled (legal, by design)
/// N: Alloc (N,0x110) VASpace, SET_PAGE_DIRECTORY pdb  ⇒ the VICTIM's address plane
/// ```
///
/// Measured on the pre-fix tree: the ghost channel — owned by declaration `{N, 0}`, in
/// `{N, 0}`'s own `Proc` — projected `vas_origin` = the VASpace owned by `{N, 1}` and
/// `vas_pdb` = the victim's `Pdb`, which `by_pdb` routes to the victim. That is §12.39
/// Shape B on the **exec plane's** binding, reached through the one resolver
/// [`ResourceKey`] did not cover: §12.41 moved `project`'s per-node *lookups* to resource
/// identity (`pdb_of_resource` / `gpu_of_resource`) but left the declared-fact *resolvers*
/// keyed on the recyclable value.
///
/// The fix may not be a refusal of the recycle — RM recycles by design and refusing hangs
/// a legal guest — so it is a **MISS**: a dead declaration's handle facts resolve to
/// nothing, the ghost channel materializes with no VAS, and its use faults loudly by name
/// rather than silently binding to a stranger (MISS ⇒ DEFER here, FAULT at use).
///
/// ★ Note the blast radius is exactly the recycled case and nothing else: when the owner's
/// namespace has no live root at all, the handle lookup already missed (the table is
/// empty). The guard changes an answer only where a *different* declaration now holds the
/// value — which is the breach and only the breach.
///
/// ## The typed half (decision #18C — unchanged)
///
/// `want` is the ONE typed-resolution primitive: a hostile or buggy guest may name a TSG,
/// a memory object or a dangling handle as its `hVASpace`, and binding a channel to a
/// non-VASpace's PDB would make the channel's `vas_pdb` disagree with `by_pdb` (which only
/// routes real VASpaces) — the confused-deputy inconsistency the fuzz property caught. The
/// wrong-kind case is never knowable and is fused into the same `None` by
/// [`RmGraph::origin_of_kind`] — see that function, and `l1_concurrency.md` §12.30
/// finding B for the open question.
///
/// # ★★★★ §16.25 — it returns a `Result`, and the `Err` is the whole point
///
/// The decision is unchanged (`.ok()` is exactly the old `Option`). What is new is that
/// the **reason** for a miss is produced by *this* function — the one that makes the
/// decision — rather than by a second, parallel "diagnose why" pass. A separate diagnoser
/// is the "two descriptions of one fact" shape this port refuses everywhere else, and it
/// drifts silently: the day the §12.44 guard changes, a diagnoser still reports the old
/// story and reads exactly like a measurement.
///
/// ⊘ [`HandleMiss`] is **report-only**. No caller branches on which variant it is; every
/// one of them collapses it with `.ok()`. Distinguishing `Absent` from `WrongKind` costs
/// one extra [`RmGraph::origin_of`] **on the miss path only**.
fn resolve_declared_handle<'g>(
    g: &'g RmGraph,
    live_root: &BTreeMap<HClient, ClientKey>,
    owner: ClientKey,
    handle: HObject,
    want: ObjectKind,
) -> Result<&'g RmNode, HandleMiss> {
    // ★★★ §12.44 — the declaration must still own its namespace. A superseded declaration
    // has no handle table to read, and the value's current tenant's table is not its own.
    if live_root.get(&owner.client) != Some(&owner) {
        return Err(HandleMiss::OwnerSuperseded);
    }
    let key = NodeKey::new(owner.client, handle);
    match g.origin_of_kind(key, want) {
        Some(n) => Ok(n),
        // ⊘ The kind check is the ONLY thing `origin_of_kind` adds over `origin_of`, so a
        // node here is a node of the WRONG kind and nothing else. Reporting which kind is
        // what separates "the guest never allocated this" from "the guest named a TSG
        // where a VASpace was wanted" — decision #18C's fused `None`, un-fused for the
        // report and still fused for the decision.
        None => Err(match g.origin_of(key) {
            None => HandleMiss::Absent,
            Some(n) => HandleMiss::WrongKind(n.kind),
        }),
    }
}

/// ★★★★ §16.25 — why a declared handle did not resolve to an object of the wanted kind.
///
/// ⊘ **Report-only.** It exists so a `NoVas` refusal can say which of the three
/// `resolve_channel_vas` routes was tried and what each one hit, instead of naming the
/// absence and nothing else (`kayfabe_qemu_raw::shim`'s own comment, §16.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleMiss {
    /// ★★★ §12.44 — the owning declaration no longer owns its namespace, so it has no
    /// handle table to read. ⚠ Distinct from [`Self::Absent`] in the way that matters: the
    /// handle may well name a live object *in the value's current tenant's table*, and
    /// reading it there would be the recycled-namespace confusion the guard exists to stop.
    OwnerSuperseded,
    /// Nothing resolves under `(owner.client, handle)` at all — the guest never allocated
    /// it, or it has been freed and no dup keeps it alive.
    Absent,
    /// A live object resolves, but its kind is not the one wanted. Carries the kind that
    /// **was** found, which is the fact that distinguishes a guest bug from ours.
    WrongKind(ObjectKind),
}

impl fmt::Display for HandleMiss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OwnerSuperseded => write!(f, "owner-superseded"),
            Self::Absent => write!(f, "absent"),
            Self::WrongKind(k) => write!(f, "wrong-kind({k:?})"),
        }
    }
}

/// ★★★★ §16.25 — what ONE route of [`resolve_channel_vas`] did.
///
/// ⊘ Report-only; nothing branches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VasHop {
    /// ★★★ The route was **never tried**, because an earlier route already returned an
    /// answer — including a returned *miss*. This is the variant that carries the
    /// precedence rule into the report: `resolve_channel_vas` commits to a route the
    /// moment that route's chain of declared facts holds, and does **not** fall through to
    /// the next one when the final VASpace lookup then misses. `NotAttempted` on `tsg`
    /// beside a `VasMiss` on `ctx_share` says exactly that, and it is the difference
    /// between "the TSG had no VA space" and "nobody ever asked the TSG".
    NotAttempted,
    /// The channel declared no handle for this route.
    NotDeclared,
    /// The route's intermediate object (CtxShare / TSG) did not resolve.
    IntermediateMiss {
        /// The handle the channel named for the intermediate — its `hCtxShare`, or (for
        /// the TSG route) its own declared `parent`.
        handle: u32,
        /// Why that handle did not resolve to an object of the wanted kind.
        miss: HandleMiss,
    },
    /// The intermediate resolved, and itself declared no `hVASpace` — so this route had
    /// nothing to inherit. ⊘ Not a failure of ours; a fact about the guest's object.
    IntermediateNoVas {
        /// The handle of the intermediate that was found and declared nothing.
        handle: u32,
    },
    /// The intermediate resolved and named an `hVASpace`, but the intermediate's own
    /// owning declaration could not be recovered, so the final hop had no namespace to
    /// resolve *in*. ⚠ Its own variant because it is the one step that is neither the
    /// guest's declaration nor the VASpace's existence — it is our graph failing to name
    /// an owner for an object it just handed us.
    IntermediateOwnerMiss {
        /// The handle of the intermediate whose owning declaration could not be named.
        handle: u32,
    },
    /// The final `hVASpace` lookup missed.
    VasMiss {
        /// The `hVASpace` that was looked up — the channel's own on route 1, the
        /// intermediate's on routes 2 and 3.
        handle: u32,
        /// Why it did not resolve to a VASpace.
        miss: HandleMiss,
    },
    /// ★★★★ **§16.28 — route 4 answered: the parent DEVICE's default VA space.**
    ///
    /// The channel declared no `hVASpace` and no `hCtxShare`, and its parent resolved to a
    /// **Device** rather than a TSG. RM's answer for that exact shape is the Device's
    /// default address space (`ogkm-580: kernel_channel.c:350-375` →
    /// `kernel_ctxshare.c:127` → `vaspace.c:231-241` → `device_share.c:324`), and the
    /// handle here is the transient name RM itself published that address space under —
    /// see [`crate::rmgraph::RmGraph::device_default_vas`] and
    /// [`kayfabe_arch::VaSpaceRole`].
    ///
    /// ⚠ It carries a **handle and no origin**, and the missing origin is a true
    /// statement rather than a gap: the resource that handle named was freed by the
    /// guest's own RM immediately after it published the address space's root. That is
    /// why this is its own variant and not a [`Self::Resolved`] with a synthetic origin —
    /// a reader must be able to see that this route resolved a *name*, not a live object.
    DeviceDefault {
        /// The Device handle the channel is parented on — the object RM says owns the
        /// address space.
        device: u32,
        /// The `hVASpace` RM minted, published under, and freed.
        vas: u32,
    },
    /// ★ §16.28 — route 4 ran and the Device has declared no default VA space (yet). ⊘ A
    /// **DEFER**, not a verdict: RM names one lazily, so this is *"nothing has needed the
    /// GSP side to have a handle for it"* and never *"this Device has no address space"*.
    DeviceDefaultUndeclared {
        /// The Device handle that was asked.
        device: u32,
    },
    /// Resolved. ★ `handle` differs from `origin_handle` exactly when the guest bound its
    /// VA space through a `DUP_OBJECT` alias (§12.41), so the pair shows the dup being
    /// followed rather than hiding it.
    Resolved {
        /// The `hVASpace` that was looked up.
        handle: u32,
        /// The `hClient` of the resolved VASpace's ORIGIN declaration. ⚠ Need not be the
        /// channel's own client: a CtxShare or TSG owned by another namespace resolves its
        /// `hVASpace` in *that* namespace.
        origin_client: u32,
        /// The `hObject` the resolved VASpace was originally allocated at.
        origin_handle: u32,
    },
}

impl VasHop {
    /// The outcome of a final `hVASpace` hop — the one shape shared by all three routes.
    fn of_vas(handle: HObject, got: Result<&RmNode, HandleMiss>) -> Self {
        match got {
            Ok(n) => Self::Resolved {
                handle: handle.0,
                origin_client: n.key.client.0,
                origin_handle: n.key.handle.0,
            },
            Err(miss) => Self::VasMiss {
                handle: handle.0,
                miss,
            },
        }
    }
}

impl fmt::Display for VasHop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAttempted => write!(f, "not-attempted"),
            Self::NotDeclared => write!(f, "not-declared"),
            Self::IntermediateMiss { handle, miss } => write!(f, "mid-miss(h0x{handle:x},{miss})"),
            Self::IntermediateNoVas { handle } => write!(f, "mid-no-vas(h0x{handle:x})"),
            Self::IntermediateOwnerMiss { handle } => write!(f, "mid-owner-miss(h0x{handle:x})"),
            Self::VasMiss { handle, miss } => write!(f, "vas-miss(h0x{handle:x},{miss})"),
            Self::DeviceDefault { device, vas } => {
                write!(f, "dev-default(dev0x{device:x}=>h0x{vas:x})")
            }
            Self::DeviceDefaultUndeclared { device } => {
                write!(f, "dev-default-undeclared(dev0x{device:x})")
            }
            Self::Resolved {
                handle,
                origin_client,
                origin_handle,
            } => write!(
                f,
                "ok(h0x{handle:x}=>c0x{origin_client:x}/0x{origin_handle:x})"
            ),
        }
    }
}

/// ★★★★ §16.25 — **the discriminating null**: what each of [`resolve_channel_vas`]'s three
/// routes did, carried beside the `Option` it returns.
///
/// # Why this exists
///
/// `[measured 2026-08-08, boot `s23_10a769c_cup2`]` 15 of 24 doorbells refused
/// `FwdFault::NoVas(ChanId(3))`, and the refusal printed `vas=NONE-DECLARED dec=NONE` —
/// which says only that **the channel** declared no `hVASpace`. Whether its CtxShare or
/// its parent TSG declared one, whether those parent objects were even found, and which of
/// the three routes was tried at all were all invisible. Three routes returned `None` and
/// the report could not tell them apart.
///
/// ⊘ **Report-only, and it changes no decision.** `resolve_channel_vas` returns the same
/// `Option` it always did; this rides beside it. The point is that a reader can tell a
/// channel that inherits nothing (`own=not-declared cs=not-declared tsg=not-declared` — a
/// genuinely GSP-managed channel) from one whose CtxShare *was* found and named a VASpace
/// that then missed (`cs=vas-miss(...)`), and those two produce the identical `None`.
///
/// ★ And it is built to discriminate the **working sibling**: nine doorbells in that same
/// boot were served, so every served channel carries this too. A field that reads the same
/// on a served channel and a refused one is not the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VasRoutes {
    /// The channel's own `hVASpace`.
    pub own: VasHop,
    /// The channel's CtxShare's `hVASpace`.
    pub ctx_share: VasHop,
    /// The channel's parent TSG's `hVASpace`.
    pub tsg: VasHop,
    /// ★★★★ **§16.28 — route 4: the parent DEVICE's default VA space.** Runs only when
    /// route 3 found the parent to be a `Device` rather than a TSG, which is the exact
    /// shape RM answers with `deviceGetDefaultVASpace`. See
    /// [`VasHop::DeviceDefault`].
    pub device_default: VasHop,
}

impl VasRoutes {
    /// The starting value: nothing tried yet. ⊘ Every field is overwritten by the route
    /// that runs, so a surviving `NotAttempted` is a true statement that the route was
    /// skipped — never a default that leaked through.
    pub const NOT_ATTEMPTED: Self = Self {
        own: VasHop::NotAttempted,
        ctx_share: VasHop::NotAttempted,
        tsg: VasHop::NotAttempted,
        device_default: VasHop::NotAttempted,
    };
}

impl fmt::Display for VasRoutes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "own={} cs={} tsg={} dev={}",
            self.own, self.ctx_share, self.tsg, self.device_default
        )
    }
}

/// Resolve a channel node's VASpace origin per the declared-facts precedence:
/// own `hVASpace` → CtxShare's → parent TSG's. Every hop is a
/// [`resolve_declared_handle`] — so every hop is both typed (decision #18C) and scoped to
/// the DECLARATION that made the declaration (★★★ §12.44).
///
/// Each hop re-reads the owner from the graph rather than from the handle's `HClient`:
/// a CtxShare or TSG may itself be a dup-kept ghost, and *its* `hVASpace` is a fact about
/// *its* namespace declaration.
///
/// **MISS ⇒ DEFER** (same category and same caveat as [`resolve_declared_handle`]): the
/// channel materializes with `vas_pdb: None` and rings nothing, because
/// `kayfabe_fwd::gate_working_set_in` refuses a channel with no VAS by name.
///
/// # ★★★★ §16.25 — it also returns [`VasRoutes`], and that is REPORT ONLY
///
/// The `Option` is bit-for-bit the one this function always returned; every early return
/// below is the same early return, at the same place, on the same condition. What rides
/// beside it is a record of which routes ran and what each hit — see [`VasRoutes`] for the
/// boot that made a bare `None` unreadable.
///
/// ⚠ **Note what the report preserves: this function COMMITS to a route.** Once a route's
/// chain of declared facts holds, it returns that route's answer *even when the final
/// VASpace lookup misses* — it does **not** fall through to the next route. That was always
/// true and is unchanged; it is now *visible*, as a `NotAttempted` sitting beside a
/// `VasMiss`. ⊘ Do not "fix" this into a fall-through without a boot to attribute it to:
/// falling through would let a channel silently inherit a *different* address space from a
/// parent it never asked, which is the confused-deputy shape decision #18C forbids.
fn resolve_channel_vas<'g>(
    g: &'g RmGraph,
    live_root: &BTreeMap<HClient, ClientKey>,
    chan: &RmNode,
    owner: ClientKey,
) -> (Option<&'g RmNode>, Option<HObject>, VasRoutes) {
    let mut r = VasRoutes::NOT_ATTEMPTED;

    // ── Route 1: the channel's own `hVASpace`. ──────────────────────────────────────────
    if let Some(hv) = chan.facts.h_vaspace {
        let got = resolve_declared_handle(g, live_root, owner, hv, ObjectKind::VaSpace);
        r.own = VasHop::of_vas(hv, got);
        return (got.ok(), None, r);
    }
    r.own = VasHop::NotDeclared;

    // ── Route 2: the CtxShare's. ────────────────────────────────────────────────────────
    match chan.facts.h_ctx_share {
        None => r.ctx_share = VasHop::NotDeclared,
        Some(hcs) => {
            match resolve_declared_handle(g, live_root, owner, hcs, ObjectKind::CtxShare) {
                Err(miss) => {
                    r.ctx_share = VasHop::IntermediateMiss {
                        handle: hcs.0,
                        miss,
                    };
                }
                Ok(cs) => match cs.facts.h_vaspace {
                    None => r.ctx_share = VasHop::IntermediateNoVas { handle: hcs.0 },
                    Some(hv) => match g.owner_key_of(NodeKey::new(owner.client, hcs)) {
                        None => r.ctx_share = VasHop::IntermediateOwnerMiss { handle: hcs.0 },
                        Some(cs_owner) => {
                            let got = resolve_declared_handle(
                                g,
                                live_root,
                                cs_owner,
                                hv,
                                ObjectKind::VaSpace,
                            );
                            r.ctx_share = VasHop::of_vas(hv, got);
                            // ⊘ Returns on a MISS too — the original `&&`-chain returned the
                            // final lookup's result unconditionally once the chain held, so the
                            // TSG route stays `NotAttempted`. Preserved deliberately.
                            return (got.ok(), None, r);
                        }
                    },
                },
            }
        }
    }

    // ── Route 3: the parent may be a TSG that declares the VAS. ─────────────────────────
    match resolve_declared_handle(g, live_root, owner, chan.parent, ObjectKind::Tsg) {
        Err(miss) => {
            r.tsg = VasHop::IntermediateMiss {
                handle: chan.parent.0,
                miss,
            };
        }
        Ok(parent) => match parent.facts.h_vaspace {
            None => {
                r.tsg = VasHop::IntermediateNoVas {
                    handle: chan.parent.0,
                }
            }
            Some(hv) => match g.owner_key_of(NodeKey::new(owner.client, chan.parent)) {
                None => {
                    r.tsg = VasHop::IntermediateOwnerMiss {
                        handle: chan.parent.0,
                    }
                }
                Some(tsg_owner) => {
                    let got =
                        resolve_declared_handle(g, live_root, tsg_owner, hv, ObjectKind::VaSpace);
                    r.tsg = VasHop::of_vas(hv, got);
                    return (got.ok(), None, r);
                }
            },
        },
    }

    // ── ★★★★ Route 4 (§16.28): the parent is a DEVICE, so the VA space is the DEVICE'S
    // DEFAULT. ───────────────────────────────────────────────────────────────────────────
    //
    // # The precondition is POSITIVE, and it is the one RM branches on
    //
    // This runs only when route 3 resolved the channel's declared parent and found it to
    // be a **`Device`** — not when the parent was absent, and not for any other kind.
    // That is exactly RM's own fork: `kernelchannelConstruct` looks the parent up as a
    // `KernelChannelGroupApi` and, when that fails, allocates a wrapper TSG *under the
    // same `hParent`* forwarding `hVASpace = NV01_NULL_OBJECT`
    // (`ogkm-580: kernel_channel.c:350-375`); the CtxShare then resolves that null through
    // `vaspaceGetByHandleOrDeviceDefault` (`kernel_ctxshare.c:127`), which requires the
    // handle to be a Device or Subdevice and returns `deviceGetDefaultVASpace`
    // (`vaspace.c:215-241`). A parent that is neither a TSG nor a Device is a channel RM
    // would have refused, so there is nothing here to be generous about.
    //
    // # ⊘ What this returns, and what it deliberately does NOT return
    //
    // A **handle**, and no node — because there is no live object to return and inventing
    // one would be the fabrication this project forbids. RM minted that handle purely so
    // the GSP side would have a name for the Device's address space, published the address
    // space's page-directory root under it, and freed it again
    // (`gpu_vaspace.c:4101-4135`; the whole sequence is on
    // [`kayfabe_arch::VaSpaceRole`]). ⊘ The address is **not** invented either: it is the
    // guest's own publication, filed under this very `(hClient, hObject)` pair, and this
    // route hands over the key rather than an address.
    //
    // ⊘ It does not touch `vas_origin`, so no channel gains a `Pdb`, no `Vas` is minted,
    // and `by_pdb` is untouched — the routing planes see exactly what they saw before.
    if let VasHop::IntermediateMiss {
        miss: HandleMiss::WrongKind(ObjectKind::Device),
        ..
    } = r.tsg
    {
        let device = NodeKey::new(owner.client, chan.parent);
        match g.device_default_vas(device) {
            Some(vas) => {
                r.device_default = VasHop::DeviceDefault {
                    device: chan.parent.0,
                    vas: vas.0,
                };
                return (None, Some(vas), r);
            }
            None => {
                r.device_default = VasHop::DeviceDefaultUndeclared {
                    device: chan.parent.0,
                };
            }
        }
    }
    (None, None, r)
}

/// ★ The condemnation input of an **un-condemned** device — the value every caller that
/// is not [`crate::gpu::Spine::refresh`] passes to [`project`].
///
/// It is a named constant rather than a defaulted argument so that "this projection
/// considers no client dead" is a statement the call site makes, not one it omits.
pub static NO_CONDEMNED: BTreeSet<ClientKey> = BTreeSet::new();

/// ★★★★★ **WHICH ENGINE A CHANNEL IS ON — one authority, three sources, in order.**
///
/// # The principle, and it is §16.106's own sentence
///
/// The runlist repair that took engine-object refusals 14 → 0 put it this way: *"we never
/// had to make it: the guest already did."* This function is that sentence applied to the
/// engine itself. Every GPFIFO channel on an architecture shares ONE class
/// (`AMPERE_CHANNEL_GPFIFO_A` on GA10x), so `Arch::classify` — which sees a class id and
/// nothing else — cannot tell a GR channel from a CE channel and answers
/// [`EngineKind::GrCompute`] for both. The guest stated the answer in the same alloc
/// message, in `NV_CHANNEL_ALLOC_PARAMS.engineType`.
///
/// # The order, and why it is not simply "declaration wins"
///
/// 1. **`declared`** decides *which engine*. It is the guest's own word and it is available
///    at the channel's alloc, i.e. **before** any engine object exists.
/// 2. **`refined`** — the first engine object parented on the channel — decides *within GR*.
///    ⚠ This is the case a naive "declaration wins" gets wrong: `engineType` is
///    `NV2080_ENGINE_TYPE_GRAPHICS` for compute **and** 3D alike, so a declaration of
///    [`EngineKind::GrCompute`] is UNDER-DETERMINED, and letting it overwrite an
///    `AMPERE_B`-derived [`EngineKind::GrGraphics`] would replace a finer fact with a
///    coarser one and call it an improvement.
/// 3. **`class_default`** — `Arch::classify`'s answer — when there is neither.
///
/// ⊘ Outside GR the refinement does **not** get to override the declaration, and that is
/// the correction this function carries: a channel the guest declared `GRAPHICS` whose
/// first engine object happens to be `AMPERE_DMA_COPY_B` used to be re-labelled
/// [`EngineKind::Ce`] and would have been materialized on a copy runlist. The refinement is
/// a stand-in for a declaration; it does not outrank one.
///
/// ⚠ `[NOT MEASURED]` that any boot exercises that disagreement. It is closed here because
/// the ordering that protects it — `.or_insert`, so the *first* engine object wins — is
/// incidental rather than enforced, and nothing in the tree asserts a compute object is
/// allocated first.
fn channel_engine(
    declared: Option<EngineKind>,
    refined: Option<EngineKind>,
    class_default: EngineKind,
) -> EngineKind {
    match (declared, refined) {
        // GR is under-determined on the wire: only the object separates compute from 3D.
        (
            Some(EngineKind::GrCompute),
            Some(r @ (EngineKind::GrCompute | EngineKind::GrGraphics)),
        ) => r,
        (Some(d), _) => d,
        (None, Some(r)) => r,
        (None, None) => class_default,
    }
}

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
    condemned: &BTreeSet<ClientKey>,
) -> Result<Boundaries, ProjectionError> {
    // Client universe: every LIVE client-namespace DECLARATION (★★★ §12.42 — see the
    // block above `clients` below for why it is that and not "the namespaces that own a
    // resource, plus the endpoints of every resolved dup"). A still-parked dup conjures
    // nothing into it, which is what keeps an intermediate `Dup`-before-`Alloc` state from
    // differing from the fully-applied one.
    // ★ §12.27 — the declared privilege of every client whose root has been observed.
    // Read once, up front, because it gates the client universe, the edge predicate and
    // the assignment of every node below. A client absent from this map has not declared
    // itself yet.
    //
    // ★★★ §12.42 — keyed by the DECLARATION, and it comes with a second index: which
    // declaration each `hClient` VALUE currently names. One value may name several live
    // declarations at once (an orphaned predecessor whose resources a foreign alias keeps
    // alive, plus the tenant that re-declared the value), but at most one of them has a
    // live root, and the live-rooted one is the only namespace the guest can still name in
    // an event.
    let kinds: BTreeMap<ClientKey, ClientKind> = g.client_kinds().collect();
    let live_root: BTreeMap<HClient, ClientKey> = kinds.keys().map(|&k| (k.client, k)).collect();
    let is_user = |k: ClientKey| matches!(kinds.get(&k), Some(ClientKind::User { .. }));
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
    let decls: BTreeMap<ClientKey, (ClientId, ClientKind)> = g.client_declarations();
    let is_kernel = |k: ClientKey| matches!(decls.get(&k), Some((_, ClientKind::Kernel)));

    // ★★★ §12.42 — **THE CLIENT UNIVERSE IS THE SET OF LIVE DECLARATIONS**, and there is
    // no membership predicate left to get wrong.
    //
    // §12.39/§12.40 filtered here: a resource projected only while its recorded owner was
    // *the* declaration its `HClient` value projected under, because a value had exactly
    // one slot. That kept the recycled-namespace isolation break closed but paid for it in
    // the corruption direction — §12.41 §4 measured it. After
    // `declare A → alias → free root → re-declare A`, generation 1's resources are still
    // alive (RM refcounts them for the alias holder, `ogkm .../mem_mgr/mem.c:1027-1031`)
    // and they matched nothing, so they entered no boundary, their component vanished
    // through `vanishing`, and `stage_dropped_vases` **released the host VAS and the
    // published backing of a resource a live kernel client still references.** That is
    // §12.40 §1's own rejected D4, reached by a different door.
    //
    // With a [`ClientKey`] the two generations are two declarations, so they are two
    // components — which is what the `HClient`-keyed `ProcAnchor`/`clients` made
    // inexpressible (N2). Every live resource now projects, under its own declaration:
    // isolation is held by the *identity* rather than by an exclusion, and nothing that RM
    // says is live is dropped on the floor.
    //
    // The dup-endpoint chain §12.27 added is gone with it, because it is now provably
    // redundant rather than merely usually so: a resolved dup's `dst` handle is live, so
    // §12.38's rule gives its namespace a live root, and a live root IS a live resource of
    // its own declaration; the `src` side resolves to the declaration that owns the
    // resource, which is live by the same construction. Both are already here.
    let clients: BTreeSet<ClientKey> = decls.keys().copied().collect();

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
    for (dst, _src) in g.dups() {
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
        //
        // ★★★ §12.39 Part B / §12.42 — **both ends are named as DECLARATIONS, and neither
        // is read off the edge's `src` value.** The `dst` end is the live root of the
        // namespace the destination handle lives in (a live handle implies a live root,
        // §12.38). The `src` end is the declaration that **allocated the resource**, read
        // through the destination handle — because the `src` a dup edge reports is the
        // resource's origin `(client, handle)`, and that `HClient` is exactly the
        // recyclable value the identity model exists to stop trusting: an attacker that
        // allocates a VASpace in a namespace it will hand back, aliases it into a client of
        // its own and frees the root leaves an edge whose `src.client` is a *number* the
        // guest kernel gives to an unrelated later process.
        //
        // The source must additionally still be its value's **live-rooted** declaration:
        // §12.27's rule is positive evidence about a live declaration at both ends, and an
        // orphaned predecessor has no root to give it. That conjunct is what refuses the
        // stale edge on the victim's own first RM event.
        //
        // ★ Measured, and stated because it is a testing fact rather than a design one:
        // that conjunct and `is_user`'s [`ClientKey`] key are now **mutually redundant**.
        // `kinds` holds only live-rooted declarations and is keyed by the declaration, so
        // `is_user(src_decl)` is already false for an orphan; and conversely, keying
        // `is_user` by the VALUE (§12.40's shape) is caught by the conjunct. Biting either
        // one ALONE leaves the whole suite green — only biting both together fires
        // (`l1_mean::a_recycled_namespace_cannot_inherit_the_previous_tenants_address_
        // plane`). Both are kept deliberately: this is the predicate an isolation break
        // walks through, and defence in depth here is cheaper than the round that finds
        // the next hole in it.
        let (Some(&dst_decl), Some(src_decl)) = (live_root.get(&dst.client), g.owner_key_of(dst))
        else {
            continue;
        };
        if live_root.get(&src_decl.client) != Some(&src_decl) {
            continue;
        }
        if !(is_user(dst_decl) && is_user(src_decl)) {
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
        if condemned.contains(&dst_decl) != condemned.contains(&src_decl) {
            continue;
        }
        uf.union(dst_decl, src_decl);
    }

    // Assignment: every declared kernel client goes to the ONE system component, by
    // rule; everything else groups by the union above. `ProcAnchor` values from `uf` can
    // never equal `SYSTEM_ANCHOR`, because `RmGraph::apply` refuses `RESERVED_CLIENT`.
    let mut procs: BTreeMap<ProcAnchor, ProcBoundary> = BTreeMap::new();
    let mut system = ProcBoundary::empty(SYSTEM_ANCHOR);
    let anchor_of = |uf: &mut ClientUnion, c: ClientKey| {
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
    let mut ctx_vas: BTreeMap<ResourceKey, (GpuId, Pdb)> = BTreeMap::new();
    // ★ The F1 collision guard's scope tables, keyed on `(Option<GpuId>, id)`: the
    // guard still bites within one target (and within the unresolved-`None` scope),
    // while identical ids on DIFFERENT targets are legal and never collide.
    let mut pdb_claims: BTreeMap<(Option<GpuId>, Pdb), ResourceKey> = BTreeMap::new();
    let mut vchid_claims: BTreeMap<(Option<GpuId>, VChid), ResourceKey> = BTreeMap::new();

    // ★★★★★ Which engine a channel is on — see the free function `channel_engine` below.
    //
    // Pre-pass: the engine-object refinement (channel origin → EngineKind). An
    // engine object's parent is its channel (same namespace, dup-aliases resolved).
    // `nodes()` iterates ascending origin-key order, so with several engine objects
    // on one channel the smallest-key one wins — deterministic and order-independent
    // (the real protocol allocates one engine object per channel context).
    let mut engine_refine: BTreeMap<ResourceKey, EngineKind> = BTreeMap::new();
    for (node, owner) in g.nodes_with_owner() {
        // The engine-object's parent must typed-resolve to a Channel (any engine —
        // discriminant match) before its kind refines that channel; a hostile engine
        // object parented on a non-channel never refines anyone (decision #18C). ★★★
        // §12.44 — and `parent` is a fact about the OWNER's handle table, so a ghost
        // engine object never refines whatever the recycled value's next tenant put there.
        if let ObjectKind::EngineObject { engine } = node.kind
            && let Ok(chan) = resolve_declared_handle(
                g,
                &live_root,
                owner,
                node.parent,
                ObjectKind::Channel {
                    engine: EngineKind::GrCompute,
                },
            )
        {
            engine_refine.entry(chan.id()).or_insert(engine);
        }
    }

    for (node, owner) in g.nodes_with_owner() {
        // ★ §12.27 — attribution is by the resource's ORIGIN, and ★★★ §12.42 the origin is
        // a **declaration**, not an `hClient` value: a user object dup'd into the UVM
        // session stays in the USER component, and an orphan of a superseded declaration
        // stays in that declaration's own component rather than joining whoever now holds
        // the number (§12.39/§12.40) or — the defect this replaced — belonging to nobody
        // and being freed out from under a live kernel reference (§12.41 §4).
        //
        // There is no membership predicate here any more, and there must not be: the
        // universe IS `decls`, `owner` is a live declaration by construction (this
        // resource is what keeps it live), so the `expect` below cannot fire.
        let anchor = anchor_of(&mut uf, owner);
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
                // ★ A word that names no channel is a NAMED refusal, never a substituted
                // vChid — see `ProjectionError::UnnamedVchid`.
                let Some(vchid) = arch.vchid_from_userd_flags(node.facts.userd_flags) else {
                    return Err(ProjectionError::UnnamedVchid {
                        channel: node.id(),
                        userd_flags: node.facts.userd_flags,
                    });
                };
                // ★★★ §12.41 — by resource identity, on both the channel and the VASpace
                // its `hVASpace` resolves to (see the VaSpace arm).
                let gpu = g.gpu_of_resource(node.id());
                // ★★★★ §16.25 — `vas_route` is the SECOND return of the SAME call that
                // decides `vas`. ⊘ Not a re-derivation: a diagnosis computed by a separate
                // pass is free to disagree with the decision it claims to explain.
                let (vas, vas_device_default, vas_route) =
                    resolve_channel_vas(g, &live_root, node, owner);
                let facts = ChannelFacts {
                    vchid,
                    gpu,
                    vas_origin: vas.map(RmNode::id),
                    vas_pdb: vas.and_then(|v| g.pdb_of_resource(v.id())),
                    vas_device_default,
                    vas_route,
                    // ★★★★★ The guest's own declaration first; the engine-object
                    // refinement second; the class default last. ONE authority — see
                    // [`channel_engine`].
                    engine: channel_engine(
                        node.facts.channel_engine,
                        engine_refine.get(&node.id()).copied(),
                        engine,
                    ),
                    // ★ Straight off the channel's OWN alloc facts, never refined and
                    // never inherited from the TSG. `NV_CHANNEL_ALLOC_PARAMS` carries
                    // `errorNotifierMem` per channel — CPU-RM has already applied the
                    // group's fallback by the time it fills the field
                    // (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:506-570`),
                    // so re-deriving it from the group here would be a second, competing
                    // resolution of a question the guest already answered.
                    error_notifier: node.facts.error_notifier,
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
                // ★★ The context-object index. A channel's own resolved VAS is already
                // in hand; it enters only when BOTH halves are declared, because a
                // half-resolved entry is a DEFER and a promotion must fault on it rather
                // than land in a guessed table.
                if let (Some(gpu), Some(pdb)) = (gpu, facts.vas_pdb) {
                    ctx_vas.insert(node.id(), (gpu, pdb));
                }
                boundary.channels.insert(node.id(), facts);
            }
            // ★★ A TSG is the other legal `hObject` of `GPU_PROMOTE_CTX`. It declares
            // its VASpace directly, resolved through the same scoped+typed resolver every
            // other declared handle fact uses. It contributes NOTHING else to the
            // projection — the group's channels are what route work — so this arm exists
            // solely for the promote join and does nothing when nobody promotes.
            ObjectKind::Tsg => {
                if let Some(hv) = node.facts.h_vaspace
                    && let Ok(vas) =
                        resolve_declared_handle(g, &live_root, owner, hv, ObjectKind::VaSpace)
                    && let Some(gpu) = g.gpu_of_resource(node.id())
                    && let Some(pdb) = g.pdb_of_resource(vas.id())
                {
                    ctx_vas.insert(node.id(), (gpu, pdb));
                }
            }
            _ => {}
        }
    }

    Ok(Boundaries {
        procs: procs.into_values().collect(),
        system,
        by_pdb,
        by_vchid,
        ctx_vas,
    })
}
