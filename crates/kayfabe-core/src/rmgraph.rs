//! ★ The RM resource graph — the source of truth (arch doc §4.3.1a, decision #14).
//!
//! There is no GPU concept of a CPU *process*; NVIDIA's real boundary objects are the
//! RM resource hierarchy. This module mirrors that hierarchy from **abstract events**
//! ([`RmEvent`]) whose every field is a *declared protocol fact* (a channel names its
//! `hVASpace`/`hContextShare` in its alloc params; every object names its parent;
//! `DUP_OBJECT` is the only cross-client transfer edge). Because every edge is
//! declared, the graph a sequence of events produces is **independent of the order the
//! events arrive in** — the protocol-not-observed-order guarantee (decision #4),
//! asserted by the shuffle property test.
//!
//! ## ★★★ Decision #4, as amended (`l1_concurrency.md` §12.38)
//!
//! > **Order-independence holds over any order of *LEGAL PROTOCOL FACTS*** — not over
//! > every order a stream can express. Where the NVIDIA protocol itself forbids an
//! > ordering, so do we: **if RM would error on "use before exist", so must we.**
//!
//! The amendment is one clause ("of legal protocol facts") and it changes nothing about
//! the guarantee's purpose: a reordered or retried guest still yields identical
//! boundaries. What it removes is the pretence that we owe order-independence to traces
//! the guest's own RM can never emit. Modelling one of those was a **cross-process
//! isolation break**: a `DUP_OBJECT` planted into a never-declared client namespace was
//! accepted and parked, and fired the instant an unrelated later process was handed that
//! `hClient` — putting it in the *planter's* `Proc` (§12.38, and
//! [`RmGraphError::UndeclaredClient`]).
//!
//! Concretely, exactly ONE ordering is now enforced, and it is enforced centrally in
//! [`RmGraph::undeclared_namespace`]: **a client namespace must be declared (its
//! `NV01_ROOT` observed) before any event may name it.** Everything else stays as
//! order-tolerant as before — an object may precede its parent, a `SET_PAGE_DIRECTORY`
//! its VASpace, a `MAP_MEMORY_DMA` either end, and a `DUP_OBJECT` its **source object**.
//! Those are *object-level* facts that may never have reached the wire at all (only 25 of
//! 82 measured dups do), so they are DEFER-for-observation — category 2 of the crate
//! docs' taxonomy — and faulting them would hang a legal guest.
//!
//! Deliberately NOT here: real NVIDIA structs (`NV_CHANNEL_ALLOC_PARAMS` decoding is
//! the Axis-A adapter's job — it produces these events), and any *policy* (grouping,
//! routing — those are pure projections in [`crate::project`]).
//!
//! ## The resource / handle split (RM refcounting, faithfully)
//!
//! A single RM *resource* (`RsResource`, `resource_list.h`) is reference-counted: a
//! `RM_ALLOC` creates it with one reference, `DUP_OBJECT` (NVOS55) hands **another
//! client its own handle to the SAME resource**, and the resource is destroyed only
//! when its *last* reference is freed. We model this literally: a [`Resource`] carries
//! the object's payload once and the **set of live handles** that reference it, and
//! its liveness is exactly *"that set is non-empty"* — an invariant the type makes
//! obvious. Freeing the source client therefore does NOT destroy a resource that a
//! dup still references; the alias keeps resolving.
//!
//! **Order tolerance mechanics:** `apply` records facts keyed by handle and resolves
//! references only at projection time, so a `Dup` may arrive before its source's
//! `Alloc`, a `SetPageDir` before its VASpace exists, etc. Only [`RmEvent::Free`] is
//! inherently ordered (lifecycle), which is why the shuffle property is stated over
//! alloc/dup/bind facts.

use std::collections::{BTreeMap, BTreeSet};

use kayfabe_arch::fault::ErrorNotifier;
use kayfabe_arch::ids::{ClassId, GpuId, GpuVa, HClient, HObject, Pdb};
use kayfabe_arch::{Arch, ClientKind, ObjectKind, VaSpaceRole};

/// Global identity of an RM *handle*: handles are **per-client namespaces** (two
/// processes routinely present identical `HObject` values — #14 round 1), so a
/// handle is keyed by `(client, handle)`. Several handles may reference one resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeKey {
    /// Owning client namespace.
    pub client: HClient,
    /// Handle within that namespace.
    pub handle: HObject,
}

impl NodeKey {
    /// Convenience constructor.
    #[must_use]
    pub fn new(client: HClient, handle: HObject) -> Self {
        NodeKey { client, handle }
    }
}

/// Stable identity of an underlying RM *resource*, distinct from the handles that
/// reference it. Minted from a monotonic counter at the origin `RM_ALLOC` and never
/// reused — crucially NOT derived from the origin `(client, handle)`, because a handle
/// value can be freed and *re-allocated* while a `DUP_OBJECT` alias keeps the ORIGINAL
/// resource alive; the two must stay distinct identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ResId(u64);

/// ★★★ §12.41 — **the identity of a live RM resource in every derived plane**: its
/// origin handle plus which *incarnation* of that handle value it is.
///
/// ## Why a [`NodeKey`] alone cannot be one
///
/// [`RmNode::key`] is the handle the resource's `RM_ALLOC` created it at — a **label**,
/// not an identity, because RM recycles object-handle values by design and the recycling
/// is not even lazy:
///
/// - `[src]` a caller supplies `hObjectNew` verbatim; `0` means "RM, generate one"
///   (`ogkm src/common/sdk/nvidia/inc/nvos.h:483` — *"new object handle, 0 to
///   generate"*), and `clientAssignResourceHandle` branches on exactly that
///   (`ogkm src/nvidia/src/libraries/resserv/src/rs_client.c:998-1005`).
/// - `[src]` the ONLY validation of a caller-supplied handle is that it is not
///   *currently live*: `clientValidateNewResourceHandle_IMPL` rejects `0`, the client's
///   own `hClient`, an out-of-restrict-range value, and a value `clientGetResourceRef`
///   already resolves — nothing else (`rs_client.c:1446-1470`,
///   `NV_ERR_INSERT_DUPLICATE_NAME`).
/// - `[src]` freeing removes the ref from the live map with **no quarantine and no free
///   list** (`clientDestructResourceRef_IMPL`, `rs_client.c:1137` —
///   `mapRemove(&pClient->resourceMap, pResourceRef)`), so the value passes that
///   validation again on the very next ioctl. RM's own generator is a monotonic index
///   taken `% RS_UNIQUE_HANDLE_RANGE` (`0x0008_0000` = 2^19,
///   `ogkm src/nvidia/generated/g_rs_client_nvoc.h:54-55`) that re-probes the live map
///   and wraps (`clientGenResourceHandle_IMPL`, `rs_client.c:962-974`).
/// - `[measured]` and our own C artifact says the same in its own words:
///   *"handle values are reused across contexts and process lifetimes"*
///   (`C: src/qemu/nvkvm_gpu_emul.c:670-674`, `:1926-1928`, `:1940-1942`).
///
/// So `Alloc (A,H) → Dup to B → Free (A,H) → Alloc (A,H)` leaves **two live resources
/// reporting at one `(client, handle)`**: the orphan B's alias keeps alive (RM refcounts
/// it, `mem.c:1027-1031`) and the current allocation. Keying a derived plane on the
/// `NodeKey` collapsed the two — the orphan lost its `Pdb`/`VChid` routing entirely, its
/// F1 collision claim was overwritten, and (exec plane) both incarnations were handed one
/// `ChanId`, i.e. one host channel.
///
/// ## Why an incarnation ordinal rather than the raw [`ResId`]
///
/// [`ResId`] is already the never-reused identity and would be the obvious key — but its
/// *value* is minted from a counter, so it depends on the order the events arrived in,
/// and [`crate::project::Boundaries`] is compared whole across shuffled orders (that IS
/// decision #4). The same objection that kept [`ClientId`] out of `ProcBoundary` (§12.40)
/// applies here. The incarnation ordinal has no such dependence: allocations at ONE
/// handle value are **totally ordered by the protocol** (an `Alloc` onto a live handle is
/// [`RmGraphError::ConflictingAlloc`], mirroring RM's `NV_ERR_INSERT_DUPLICATE_NAME`), so
/// "the smallest ordinal not held by a live resource at this key" is a pure function of
/// the declared facts. It is `0` for every resource whose handle value is not currently
/// shared with a ghost — i.e. essentially always.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceKey {
    /// The handle this resource's origin `RM_ALLOC` created it at. Ordered first, so a
    /// `BTreeMap<ResourceKey, _>` still iterates in ascending origin-key order and every
    /// determinism property stated over that order is unchanged.
    pub origin: NodeKey,
    /// Which live incarnation of `origin` this is. `0` unless a `DUP_OBJECT` kept an
    /// earlier resource alive past its origin handle's free and the value was re-used.
    pub incarnation: u32,
}

impl ResourceKey {
    /// The identity of the FIRST incarnation at `origin` — the only one that exists
    /// unless the handle value has been recycled while a dup kept a ghost alive.
    #[must_use]
    pub fn first(origin: NodeKey) -> Self {
        ResourceKey {
            origin,
            incarnation: 0,
        }
    }

    /// The upper bound of `origin`'s incarnation range — the inclusive end of the
    /// `first(origin)..=last(origin)` slice that is exactly the live incarnations of one
    /// handle value (never a real identity; a range bound).
    #[must_use]
    fn last(origin: NodeKey) -> Self {
        ResourceKey {
            origin,
            incarnation: u32::MAX,
        }
    }
}

/// ★★★ §12.39 — the identity of a client **NAMESPACE**: the [`ResId`] of its client-root
/// resource, minted at the root `Alloc` and **never reused**.
///
/// ## Why an `HClient` cannot be one
///
/// An `hClient` VALUE is recyclable *by design*, and the design is NVIDIA's:
///
/// - `[src]` the shipped Linux driver honours a **caller-supplied** root handle verbatim
///   — `serverAllocClient`: `hClient = pParams->hClient;`
///   (`ogkm src/nvidia/src/libraries/resserv/src/rs_server.c:612`). The guard that would
///   reject a caller-chosen id is `#if !(RS_COMPATABILITY_MODE)` (`:613-616`) and
///   `RS_COMPATABILITY_MODE=1` is set for the shipped `nv-kernel.o`
///   (`ogkm src/nvidia/Makefile:129`), so it is compiled out. The only gate is the
///   live-duplicate check (`:3352-3357`, `NV_ERR_INSERT_DUPLICATE_NAME`).
/// - `[src]` RM-chosen values come from a monotonic index that **wraps at 2^20 per driver
///   load** with **no free list and no quarantine**
///   (`_serverCreateEntryAndLockForNewClient`, `rs_server.c:3319-3341`; availability is
///   defined purely as absence from the live sorted list, `:3220-3255`).
/// - `[src]` RM carries **no epoch or generation** anywhere that could distinguish two
///   lifetimes of one value: not `RsClient` (`g_rs_client_nvoc.h:99-130`), not `RmClient`
///   (`g_client_nvoc.h:176-212`), not `CLIENT_ENTRY` (`g_rs_server_nvoc.h:75-89`), not
///   `RsResourceRef`. `RmClient::ProcID` is not one either — PIDs recycle too.
///
/// **So re-declaring a recycled namespace is legal and must keep working.** Refusing it
/// would refuse a stream the guest's own RM emits — the hangs-a-legal-guest direction.
/// The identity therefore has to be minted by *us*, at declaration, and this is it.
///
/// ## Why the root's own `ResId` rather than a second counter
///
/// `[src]` **In RM the `hClient` IS its root object's handle** — `serverAllocClient`
/// writes the client handle back as the allocated object's handle (`rs_server.c:625`;
/// `ogkm src/nvidia/src/kernel/rmapi/client.c:226-227` stamps it into
/// `NV0000_ALLOC_PARAMETERS.hClient`), which is why [`RmGraphError::DuplicateClientRoot`]
/// exists at all. "The namespace is its root resource" is what RM already means, not a
/// modelling convenience we impose — and [`ResId`] is *already* documented as never
/// reused, and already exists because a handle value can be freed and re-allocated while
/// a `DUP_OBJECT` keeps the original resource alive. This is that same sentence, one
/// level up; a second never-reused counter beside `next_res_id` would say it twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(u64);

/// ★★★ §12.42 — **the identity of one client-namespace DECLARATION in every derived
/// plane**: its `hClient` value plus which *incarnation* of that value it is.
///
/// It is to [`ClientId`] exactly what [`ResourceKey`] is to [`ResId`], and it exists for
/// the same reason, one level up.
///
/// ## Why an `HClient` alone cannot be one
///
/// [`ClientId`]'s own docs say why the VALUE is recyclable (caller-supplied roots, a
/// generator that wraps at 2^20 with no free list, no epoch anywhere in RM). §12.39/§12.40
/// closed the *grouping* half of that by comparing never-reused [`ClientId`]s. What it
/// left open is the **component plane**: [`crate::ProcAnchor`] and
/// [`crate::project::ProcBoundary::clients`] were `HClient`s, so **two generations of one
/// `hClient` could not be two boundaries**. With only one slot per value, the newer
/// declaration displaced the older — and the older declaration's resources, which RM keeps
/// alive by refcount for as long as a foreign alias references them
/// (`ogkm src/nvidia/src/kernel/mem_mgr/mem.c:1027-1031`), belonged to **no boundary at
/// all**. Their component vanished, `Gpu::vacate` staged it, and
/// `sync_proc_to_boundary`'s `stage_dropped_vases` released the host VAS and the published
/// backing **while a live kernel client still referenced the resource** — the
/// corruption-over-refusal direction §12.40 §1 refuses as D4 and `cross_proc_lifetime`
/// pins against, reached by walking around the rule instead of through it (§12.41 §4).
///
/// ## Why an incarnation ordinal rather than the [`ClientId`]
///
/// Verbatim [`ResourceKey`]'s argument. A [`ClientId`] is minted from a counter, so its
/// value depends on arrival order, and [`crate::project::Boundaries`] is compared whole
/// across shuffled orders — that IS decision #4. The ordinal has no such dependence:
/// **declarations at ONE `hClient` value are totally ordered by the protocol** (a second
/// root `Alloc` while the namespace has a live root is
/// [`RmGraphError::DuplicateClientRoot`], mirroring RM's own
/// `NV_ERR_INSERT_DUPLICATE_NAME` at `rs_server.c:3352-3357`; and the `Free` that releases
/// it must follow the `Alloc` that declared it, because an event naming an undeclared
/// namespace is refused). So "the smallest ordinal no LIVE declaration at this value
/// already holds" is a pure function of the declared facts.
///
/// It is `0` for every namespace that is not currently sharing its value with an orphaned
/// predecessor — i.e. essentially always.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientKey {
    /// The `hClient` value this declaration was made at. Ordered first, so a
    /// `BTreeSet<ClientKey>` still iterates in ascending client order and every
    /// determinism property stated over that order is unchanged — including
    /// [`crate::ProcAnchor`]'s *"the smallest client in the component"* rule.
    pub client: HClient,
    /// Which live declaration of [`Self::client`] this is. `0` unless a `DUP_OBJECT`
    /// kept an earlier declaration's resources alive past its root's free and the value
    /// was then re-declared.
    pub incarnation: u32,
}

impl ClientKey {
    /// The FIRST declaration at `client` — the only one that exists unless the value has
    /// been recycled while a foreign alias kept the predecessor's resources alive.
    #[must_use]
    pub fn first(client: HClient) -> Self {
        ClientKey {
            client,
            incarnation: 0,
        }
    }

    /// The upper bound of `client`'s incarnation range (a range bound, never a real
    /// identity). Public as [`Self::last_for`] for the one cross-module range query
    /// ([`crate::gpu::Spine::is_condemned`]).
    #[must_use]
    pub fn last_for(client: HClient) -> Self {
        Self::last(client)
    }

    /// The upper bound of `client`'s incarnation range (a range bound, never a real
    /// identity).
    #[must_use]
    fn last(client: HClient) -> Self {
        ClientKey {
            client,
            incarnation: u32::MAX,
        }
    }
}

/// ★★★ §12.42 — one live client-namespace **declaration**: its never-reused identity,
/// the privilege it declared, and how many live resources it owns.
///
/// The refcount is what makes a declaration's *liveness* a maintained fact rather than a
/// scan: a declaration is live while it has a live root **or** owns at least one resource
/// a foreign alias keeps alive. Both are the same statement here, because a live root is
/// itself a live resource of its own namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Declaration {
    /// The never-reused identity, for MATCHING (`Proc::client_ids`, `Spine::condemned`).
    id: ClientId,
    /// The privilege the namespace declared about itself at its root `Alloc`.
    kind: ClientKind,
    /// Live resources owned by this declaration. Never 0 for a stored entry.
    refs: u32,
}

/// ★ How a live handle came to reference its resource — the **declared protocol fact**
/// `RM_ALLOC` vs `DUP_OBJECT`, recorded at the event that declared it (decision #14,
/// protocol-not-trace).
///
/// This exists because "is this handle the resource's origin allocation?" is a fact the
/// guest TOLD us, and it must never be re-derived from incidental structure. The obvious
/// re-derivation — `handle == resource.node.key` — is **wrong**, because handle values
/// are reusable by design: free the origin handle while a dup keeps the resource alive,
/// then dup it BACK into the origin's namespace at the very same handle value, and the
/// alias becomes literally indistinguishable from the origin allocation. Every predicate
/// built on that equality then mis-fires; for a `Client`-classed resource the mis-fire is
/// catastrophic ([`RmGraph::is_client_root`] took the whole-namespace-teardown branch and
/// destroyed every unrelated object the client had allocated). Recording the declaration
/// makes the question a *read*, not a guess (`l1_concurrency.md` §12.25).
///
/// By construction there is exactly one [`HandleRef::Origin`] entry per resource — only
/// the `Alloc` arm mints one, and it mints a fresh [`ResId`] with it — while every
/// `DUP_OBJECT` (direct or promoted from the parked table) inserts an
/// [`HandleRef::Alias`]. So "origin" is a property of the *handle table entry*, never of
/// the handle VALUE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandleRef {
    /// The handle this resource's origin `RM_ALLOC` created (`key == node.key`, and it
    /// is THE allocation — freeing it is the lifecycle event that ends the object).
    Origin(ResId),
    /// A `DUP_OBJECT` alias: a leaf reference to someone else's allocation. It has no
    /// children of its own, and freeing it drops only this one reference.
    Alias(ResId),
}

impl HandleRef {
    /// The resource this handle references (whichever way it was declared).
    fn res(self) -> ResId {
        match self {
            HandleRef::Origin(id) | HandleRef::Alias(id) => id,
        }
    }

    /// Was this handle declared by `RM_ALLOC` (as opposed to `DUP_OBJECT`)?
    fn is_origin(self) -> bool {
        matches!(self, HandleRef::Origin(_))
    }
}

/// Declared, abstract alloc parameters — ONLY the protocol facts the graph needs.
/// The Axis-A adapter decodes real wire structs into this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AllocFacts {
    /// Declared VASpace handle (`hVASpace`), if the object names one
    /// (TSG, CtxShare, Channel). `None` models `hVASpace=0` (GSP-managed).
    pub h_vaspace: Option<HObject>,
    /// Declared context-share handle (`hContextShare`), channels only.
    pub h_ctx_share: Option<HObject>,
    /// Opaque USERD/flags word from channel alloc params; the arch recovers the
    /// channel's `VChid` from it (`Arch::vchid_from_userd_flags`).
    pub userd_flags: u32,
    /// ★★★★ §16.16 — the channel's declared **USERD object**, decoded past the
    /// version-agreement prefix (`kayfabe_abi::notifier::ChannelUserdWire`). `None` =
    /// *"this port could not read whether the channel declared one"*, ⊘ never *"it declared
    /// none"* — a declared handle of zero is a real declaration and arrives as
    /// `Some(DeclaredUserd { handle: 0, .. })`.
    ///
    /// ⊘ **Reported, and read by no decision.** Its purpose is the canary described on
    /// [`kayfabe_abi::notifier::ChannelUserdWire`]: USERD comes out of the same params as
    /// the ring but has *recognisable* content, so it turns the ring's ambiguous all-zero
    /// null into a discriminating one.
    pub userd: Option<DeclaredUserd>,
    /// Physical/backing address a MEMORY object names, if the alloc declared one
    /// (`NV01_MEMORY_*` / `NV_MEMORY_VIRTUAL` backing). The RPC populate source
    /// resolves a `MapMemoryDma`'s `memory` handle to this, so the address table can
    /// be forward-populated with `memory → phys` at bind time. `None` = a memory
    /// object with no declared backing yet (map against it faults loudly).
    pub mem_phys: Option<u64>,
    /// The physical-GPU index a **Device** object declares (`deviceInstance` in the
    /// NV0080 alloc params) — THE multi-GPU protocol fact (`multi_gpu_and_mig.md`):
    /// every object resolves its [`GpuId`] target via its `Device` ancestor's declared
    /// instance ([`RmGraph::gpu_of`]). `None` = not declared (only meaningful on a
    /// Device; an object under an un-instanced Device has no resolvable target —
    /// MISS at use, never a default-GPU0 guess).
    pub device_instance: Option<u32>,
    /// ★ The privilege a **Client** root declares about itself (`processID` on
    /// `NV0000_ALLOC_PARAMETERS`, decoded by the ABI seam into a [`ClientKind`]) —
    /// THE decision-#14 grouping discriminator (`l1_concurrency.md` §12.27).
    ///
    /// Declared at client-creation time, *before* any `DUP_OBJECT` exists, so the
    /// classification is never a function of the dup graph it decides about. Only
    /// meaningful on an [`ObjectKind::Client`] alloc, where it is **required**:
    /// `None` there is a loud [`RmGraphError::UndeclaredClientKind`], because both
    /// available guesses are catastrophic (guess "user" and a kernel client collapses
    /// every guest process into one `Proc` — #14 un-fixed; guess "kernel" and a guest
    /// process silently joins the guest kernel's isolate). On any other class the field
    /// is ignored: privilege is a property of the client root, and a hostile guest
    /// stamping it on a channel changes nothing.
    pub client_kind: Option<ClientKind>,
    /// ★★★ Where a **Channel** declared it wants to be told about its own death — the
    /// error notifier, resolved by the guest's CPU-RM and RPC'd to the GSP in the same
    /// alloc message (`kayfabe_abi::notifier`).
    ///
    /// `None` = the channel declared none, **or** this driver boundary's layout is not
    /// pinned so the ABI seam could not read it. Only meaningful on a channel alloc; on
    /// any other class the field is ignored, exactly as [`Self::client_kind`] is
    /// everywhere but a client root.
    ///
    /// ⚠ It is a *declared* fact and nothing here validates the address. The write port
    /// refuses an address outside guest RAM by name, and a second opinion here would be a
    /// second source of truth for where guest RAM is.
    pub error_notifier: Option<ErrorNotifier>,
    /// ★★★ Where a **Channel** declared its GPFIFO ring lives — carried so a boot can
    /// *state* the address, and read by nothing on the data path.
    ///
    /// `None` on every non-channel class, and on a channel whose params did not decode.
    /// See [`GpFifoRing`] for what the number is and why `Some(GpFifoRing { va: 0, .. })`
    /// is a real answer rather than an absence.
    pub gp_fifo_ring: Option<GpFifoRing>,
    /// ★★★★ **§16.28 — what a VASpace alloc declared itself to be**, decoded at the ABI
    /// seam into [`VaSpaceRole`] (which carries the whole argument and the RM chain).
    ///
    /// [`VaSpaceRole::DeviceDefault`] is the one value this graph acts on, and it acts by
    /// filing the handle against the **parent Device** in [`RmGraph::device_default_vas`],
    /// where it outlives the handle's own `Free`.
    ///
    /// ⊘ `None` = *this port could not read the declaration* (params stopped short),
    /// never [`VaSpaceRole::Own`] — folding them would make an unreadable message into a
    /// positive claim that the guest asked for a fresh address space. On any non-VASpace
    /// class the field is ignored, exactly as [`Self::client_kind`] is everywhere but a
    /// client root.
    pub vaspace_role: Option<VaSpaceRole>,
    /// ★★★★★ **WHICH ENGINE THE GUEST SAID THIS CHANNEL IS FOR** — `engineType` off
    /// `NV_CHANNEL_ALLOC_PARAMS`, decoded past the version-agreement prefix
    /// (`kayfabe_abi::notifier::ChannelEngineWire`).
    ///
    /// # ★★★ Why this field exists — *"we never had to make it: the guest already did"*
    ///
    /// There is exactly ONE GPFIFO channel class per architecture: a CUDA process's GR
    /// channel and its CE channel are **both** `AMPERE_CHANNEL_GPFIFO_A`, separated only by
    /// this field. So `Arch::classify`, which sees a class id and nothing else, cannot know
    /// — it answers [`kayfabe_arch::ids::EngineKind::GrCompute`] for every channel and a CE
    /// channel becomes one later, when its `AMPERE_DMA_COPY_B` object arrives and
    /// [`crate::project`]'s refinement pass rewrites it. That is a **derivation standing in
    /// for a declaration**, and the guest made the declaration in the same message.
    ///
    /// ⊘ **`None` is `"we could not read it"`, never `"the guest declared GR"`** — the
    /// boundary's layout is unpinned, the params stopped short, or the code is one this port
    /// does not recognise. The consumer's fallback for `None` is exactly the
    /// pre-2026-08-11 behaviour (refinement, then the class default), so an unreadable
    /// declaration costs the accuracy this field adds and nothing that worked before.
    ///
    /// ⚠ **`Some(GrCompute)` is UNDER-DETERMINED and the consumer must treat it so.**
    /// `engineType` cannot distinguish compute from 3D — both are
    /// `NV2080_ENGINE_TYPE_GRAPHICS` — so within GR the engine *object* is the finer
    /// evidence and still wins. See `kayfabe_abi::notifier::ChannelEngineWire::decode_kind`.
    ///
    /// ⊘ **The copy-engine INSTANCE is dropped at the ABI seam** (`COPY2` arrives here as
    /// `Ce`), because this crate speaks a vocabulary and not NVIDIA's numbers (decision #2).
    /// The one consumer that needs the instance —
    /// `kayfabe_isolate_host::rm::declared_copy_engine_type`, §16.106's runlist repair —
    /// takes it from the CE **object**'s params instead, and does not go through here.
    ///
    /// Only meaningful on a channel alloc; on any other class the field is ignored, exactly
    /// as [`Self::client_kind`] is everywhere but a client root.
    pub channel_engine: Option<kayfabe_arch::ids::EngineKind>,
}

/// ★★★ **The GPFIFO ring a channel declared, verbatim** — `gpFifoOffset` /
/// `gpFifoEntries` off `NV_CHANNEL_ALLOC_PARAMS`.
///
/// # ⊘ This is a GPU VIRTUAL address, and that is the whole reason it is recorded
///
/// `[src]` `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080fifo.h:809` names
/// the field *"Gpfifo Virtual Offset"*. On the driver path this port's wall sits on it is
/// `pChannel->pbGpuVA + pChannel->channelPbSize`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/mem_utils_gm107.c:1232`),
/// where `pbGpuVA` is the `dmaOffset` returned by an `NV04_MAP_MEMORY_DMA` (`:842`) —
/// **an RPC that never reaches this port at all**, because `MAP_MEMORY_DMA` is a HAL stub
/// on every GSP-client part (`kayfabe_rmrpc` crate docs, §2.7).
///
/// The same `pbGpuVA` is what a GPFIFO *entry* names: `get = pChannel->pbGpuVA + gpOffset`,
/// packed into `GP_ENTRY0_GET`/`GP_ENTRY1_GET_HI` (`:1871-1879`). `[src]`, all of it — a
/// reading of the driver and nothing more.
///
/// What follows from the reading is that this value and [`kayfabe_arch::PushRange::gpa`]
/// are addresses of the same kind in the same allocation, so a boot that states one has
/// stated the other. `[measured]` at rev `c93930d` — see
/// `docs/design/execution_plane_increments.md` §8.2.3 for the two boots and the answer.
///
/// ★★★★ **The channel's declared USERD object** — §16.16's canary. See
/// [`kayfabe_abi::notifier::ChannelUserdWire`] for why USERD discriminates where the ring
/// does not, and for the version seam both its fields sit past.
///
/// ⊘ Nothing in the core reads it; it exists so a boot can **state** what the guest
/// declared, beside what we resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclaredUserd {
    /// `hUserdMemory[0]` — the memory object the channel names for subdevice 0.
    ///
    /// ⚠ **`0` is a declaration, not a blank**: RM reads it as *"I allocated no USERD
    /// object; allocate one for me"*. That is a statement the guest made, and it is a
    /// different fact from this port being unable to read the field — which is the
    /// surrounding `Option`.
    pub handle: u32,
    /// `userdOffset[0]` — the byte offset of this channel's USERD **inside** that object.
    ///
    /// ⚠★ **A non-zero value here is a documented SILENT-STALL mechanism.**
    /// `kayfabe_abi::submit::ChannelAllocParams::userd_offset_0` records it: a consumer
    /// that ignores a non-zero offset makes hardware see `GP_PUT == GP_GET` forever, **and
    /// no error is reported anywhere**. So it is carried and printed rather than assumed
    /// zero — an assumption that would be invisible in exactly the symptom under
    /// investigation.
    pub offset: u64,
}

/// ⊘ Nothing in the core reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpFifoRing {
    /// `gpFifoOffset` @ +8 — the ring's base, as the guest named it.
    ///
    /// ⚠ **`0` is a value, not a blank**, which is why this whole struct sits behind an
    /// `Option` instead of the field being a bare `u64`: the driver deliberately declares
    /// `gpFifoOffset = 0` for the channel it uses only to build a golden context
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics.c:2420-2424`, *"Set the
    /// gpFifoOffset to zero intentionally"*). Same argument as
    /// `KayfabeRegAudit::doorbell_last_token_valid`.
    pub va: u64,
    /// `gpFifoEntries` @ +16 — how many 8-byte entries the ring holds.
    pub entries: u32,
}

/// One abstract RM protocol event. Produced by the ABI adapter (or a test),
/// consumed by [`RmGraph::apply`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RmEvent {
    /// `RM_ALLOC`: create `handle` of `class` under `parent` in `client`'s
    /// namespace. Allocating the client root itself uses `parent == handle`.
    Alloc {
        /// Owning client namespace.
        client: HClient,
        /// Parent handle within the same namespace.
        parent: HObject,
        /// New object's handle.
        handle: HObject,
        /// Class id — opaque to the core; `Arch::classify` interprets it.
        class: ClassId,
        /// Declared protocol facts from the alloc params.
        facts: AllocFacts,
    },
    /// `DUP_OBJECT` (NVOS55): alias `src` into another client's namespace as `dst`.
    /// The ONLY cross-client transfer edge — the protocol-correct source of process
    /// grouping (how UVM aliases the compute client's VASpace). The dst gets its own
    /// reference to the SAME resource, which now survives until BOTH handles free.
    Dup {
        /// Source object (may not have been observed yet — resolved lazily).
        src: NodeKey,
        /// Destination alias.
        dst: NodeKey,
    },
    /// `SET_PAGE_DIRECTORY`-shaped fact: VASpace `vaspace` (in `client`) is backed
    /// by page-directory base `pdb`. This is where the data-plane identity is born.
    SetPageDir {
        /// Client namespace of the VASpace handle.
        client: HClient,
        /// The VASpace object handle.
        vaspace: HObject,
        /// The declared page-directory base.
        pdb: Pdb,
    },
    /// `RM_MAP_MEMORY_DMA` (NVOS46) — the RPC/control bind-time transport that maps a
    /// MEMORY resource into a VASpace at `va` for `len` bytes (from `offset` into the
    /// memory). This is the **object-model-level** map event: it creates a *mapping*
    /// that references the memory resource (so the memory stays alive while mapped),
    /// and is the RPC populate source for the address table (`va → memory's phys`,
    /// resolved by [`crate::gpu::Gpu`] via [`RmGraph::backing_of`]). Forward-populate
    /// only — see `mode2_address_table.md`.
    MapMemoryDma {
        /// Client namespace issuing the map.
        client: HClient,
        /// The target VASpace handle (in `client`'s namespace).
        vaspace: HObject,
        /// The MEMORY object being mapped (in `client`'s namespace).
        memory: HObject,
        /// Guest VA the mapping starts at.
        va: GpuVa,
        /// Byte offset into the memory resource.
        offset: u64,
        /// Length of the mapping in bytes.
        len: u64,
    },
    /// `RM_UNMAP_MEMORY_DMA` — eagerly drop the mapping at `va` in `vaspace`. Releases
    /// the mapping's reference to its memory resource (unmap eager, reclaim deferred).
    Unmap {
        /// Client namespace issuing the unmap.
        client: HClient,
        /// The target VASpace handle.
        vaspace: HObject,
        /// Guest VA the mapping starts at.
        va: GpuVa,
    },
    /// `RM_FREE`: drop THIS handle's reference to its resource and (per RM semantics)
    /// its subtree. The resource itself dies only when its last reference goes.
    /// Freeing the client root drops every handle in that namespace.
    Free {
        /// Owning client namespace.
        client: HClient,
        /// Handle to free.
        handle: HObject,
    },
}

/// A live DMA mapping: a MEMORY resource mapped into a VASpace resource at a VA.
///
/// The mapping is the object-model witness of a `MAP_MEMORY_DMA`. It holds a
/// reference to the memory resource (so a mapped memory object survives its source
/// handle's free — faithful RM semantics), and carries the facts the address table's
/// RPC populate source needs (`va → memory-phys + offset`, resolved by the runtime).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapping {
    /// ★★★ §12.41 — the VASpace RESOURCE this mapping lives in, by identity. It was an
    /// origin `NodeKey`, and `Gpu::sync_rpc_mappings` resolved that key back through the
    /// live handle table to get the target GPU — so a mapping into a dup-kept ghost VAS
    /// was attributed to whatever the guest had since re-allocated at that handle value.
    pub vaspace: ResourceKey,
    /// The declared PDB of that VASpace at map time, if known (the address-table key).
    pub pdb: Option<Pdb>,
    /// ★★★ §12.41 — the mapped memory RESOURCE, by identity (see [`Self::vaspace`]).
    pub memory: ResourceKey,
    /// Guest VA the mapping starts at.
    pub va: GpuVa,
    /// Byte offset into the memory resource.
    pub offset: u64,
    /// Length in bytes.
    pub len: u64,
    /// Declared physical/backing address of the memory (from its alloc facts), if
    /// declared — the address-table's forward-populate value (`phys = base + offset`).
    pub mem_phys: Option<u64>,
}

/// Key of a live mapping: `(vaspace resource, va)`. A VA is unique within one VAS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct MapKey {
    vaspace: ResId,
    va: GpuVa,
}

/// One node of the graph — the resolved *payload* of a resource, reported at its
/// stable origin key. (The set of handles that reference it lives on [`Resource`].)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RmNode {
    /// The resource's ORIGIN handle (its first allocator). Stable across dups and across
    /// the origin client's free, as long as any reference survives.
    ///
    /// ★★★ §12.41 — a **label, not an identity**: RM recycles object-handle values, so
    /// two live resources can report the same `key`. Everything that keys, indexes or
    /// dedups on a resource must use [`Self::id`]; `key` is for naming it.
    pub key: NodeKey,
    /// ★★★ §12.41 — which live incarnation of [`Self::key`] this resource is. See
    /// [`ResourceKey`] for why the disambiguator exists and why it is an ordinal.
    pub incarnation: u32,
    /// Declared parent handle (same namespace). For a client root, equals `key.handle`.
    pub parent: HObject,
    /// Raw class id as declared.
    pub class: ClassId,
    /// Arch-classified kind (recorded at apply; classification is stable per arch).
    pub kind: ObjectKind,
    /// Declared alloc facts.
    pub facts: AllocFacts,
}

impl RmNode {
    /// ★★★ §12.41 — this resource's IDENTITY: unique among live resources, and the only
    /// thing a derived plane may key on. See [`ResourceKey`].
    #[must_use]
    pub fn id(&self) -> ResourceKey {
        ResourceKey {
            origin: self.key,
            incarnation: self.incarnation,
        }
    }
}

/// An underlying RM resource: its payload plus the set of handles referencing it.
///
/// **The liveness invariant, in the type:** a resource is alive ⟺ `refs` is
/// non-empty. `apply` upholds it — a resource is only ever inserted with a
/// non-empty `refs`, and is removed the instant `refs` becomes empty.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Resource {
    /// The resolved payload, reported at the origin key.
    node: RmNode,
    /// Every live handle `(client, handle)` that references this resource: the origin
    /// alloc plus each surviving `DUP_OBJECT` alias. NEVER empty for a live resource.
    refs: BTreeSet<NodeKey>,
    /// Declared page-directory base, for a VASpace resource (`SET_PAGE_DIRECTORY`).
    /// A property of the RESOURCE, not of any one handle — so it survives the origin
    /// handle's free as long as a dup keeps the resource alive. Last declaration wins.
    pdb: Option<Pdb>,
    /// ★ Cached GPU target (`multi_gpu_and_mig.md`): resolved from this resource's
    /// `Device` ancestor's `deviceInstance` the moment it becomes resolvable, then
    /// STICKY — a property of the resource that survives the origin handle's (and its
    /// device's) free, exactly like [`Self::pdb`]. Without this, a UVM dup that
    /// outlives the source client would lose its VAS's target when the source `Device`
    /// is freed (the parent chain breaks). Which physical GPU a PDB lives on is an
    /// immutable fact of the VAS, so caching it is correct, not a guess.
    gpu: Option<GpuId>,
    /// Live DMA mappings referencing this resource (a MEMORY resource is kept alive by
    /// its mappings as well as its handles — a mapping references the memory it maps).
    /// Faithful RM refcounting: liveness ⟺ (`refs` non-empty OR `map_refs` > 0).
    map_refs: usize,
    /// ★★★ §12.39 — the [`ClientKind`] the namespace that **allocated** this resource
    /// declared about itself, recorded at the origin `RM_ALLOC` and immutable thereafter.
    ///
    /// It exists because a resource **outlives its namespace**: a foreign `DUP_OBJECT`
    /// alias keeps it alive after its origin client's root is freed (faithful RM
    /// refcounting, `mem.c:986-1039`), and the projection must still be able to say which
    /// *side* of §12.27's grouping rule such an orphan belongs to. Before this field the
    /// only available answer was an absence, and [`crate::project`]'s `anchor_of` read
    /// that absence as **"not kernel", i.e. user** — so an orphaned resource of the guest
    /// **kernel**'s own namespace minted a *user* `ProcBoundary` with an isolate, a GPA
    /// arena and a routable `Vas`. That is precisely the guest-kernel-gets-a-user-data
    /// -plane shape `kayfabe_fwd::FwdFault::SystemDataPlane` exists to forbid, and it is
    /// the same "absence read as user" defect §12.27 removed one level up — the grouping
    /// predicate honours *"absence is never read as user"*, `anchor_of` did not.
    ///
    /// Recorded rather than re-derived, for the reason [`HandleRef`] is: the namespace's
    /// root is exactly the thing that has gone away by the time the question is asked.
    owner_kind: ClientKind,
    /// ★★★ §12.39 Part B — the **identity** of the namespace that allocated this
    /// resource ([`ClientId`] = its client root's never-reused [`ResId`]), recorded at the
    /// origin `RM_ALLOC`.
    ///
    /// This is the other half of `owner_kind`, and it is the security-relevant half. The
    /// resource's own `node.key.client` is an `HClient` **VALUE**, and a value is
    /// recyclable: freeing a root empties the namespace and the guest may hand the same
    /// number to an unrelated later process (see [`ClientId`] for RM's own recycling). A
    /// resource kept alive by a foreign alias outlives its namespace, so that stale value
    /// used to re-attach the orphan to whoever holds the number *now* — attributing an
    /// attacker's VASpace to a victim's component, merging the two into one `Proc`, one
    /// isolate, one GPA arena and one host VAS (§12.39 Shape B).
    ///
    /// With this recorded, a resource belongs to a component only while its `owner` is
    /// still the declaration its namespace projects under. It is a plain [`ClientId`] and
    /// **not** an `Option`: §12.38's rule guarantees every creating event names a declared
    /// namespace, so it always resolves — the deleted-`unwrap_or(0)` discipline (§12.27).
    owner: ClientId,
    /// ★★★ §12.42 — which **incarnation** of `node.key.client` allocated this resource.
    /// With `node.key.client` it forms the resource's owning [`ClientKey`], which is the
    /// order-independent *label* [`crate::project`] may report, where [`Self::owner`] is
    /// the order-dependent identity it may only compare. Recorded at the origin
    /// `RM_ALLOC` and immutable, exactly like its two siblings above.
    owner_incarnation: u32,
}

impl Resource {
    /// The declaration that allocated this resource — the reportable half of
    /// [`Self::owner`].
    fn owner_key(&self) -> ClientKey {
        ClientKey {
            client: self.node.key.client,
            incarnation: self.owner_incarnation,
        }
    }
}

/// Which capacity-bounded table a hostile guest overflowed (see [`RmGraphError::CapacityExceeded`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capacity {
    /// The live-handle table (`Alloc`/`Dup` flood) — bound [`MAX_LIVE_HANDLES`].
    Handles,
    /// The live-mapping table (`MapMemoryDma` flood) — bound [`MAX_LIVE_MAPPINGS`].
    Mappings,
    /// Parked (unresolved) dup edges — bound [`MAX_PARKED`].
    PendingDups,
    /// Parked (unresolved) `SET_PAGE_DIRECTORY` facts — bound [`MAX_PARKED`].
    PendingPdbs,
    /// Parked (unresolved) `MAP_MEMORY_DMA` facts — bound [`MAX_PARKED`].
    PendingMaps,
}

/// Maximum live handles (origin allocs + surviving dup aliases) the graph tracks
/// before a hostile flood is refused. **Boundary-1: no unbounded allocation from
/// guest input** — without this bound an attacker could `Alloc`/`Dup` until the host
/// OOM-aborts (taking every other guest's process down with it). The bound is orders
/// of magnitude above any real guest's live object count, so it never trips a benign
/// workload — only a flood, which gets a loud [`RmGraphError::CapacityExceeded`].
pub const MAX_LIVE_HANDLES: usize = 1 << 18;
/// Maximum live DMA mappings (see [`MAX_LIVE_HANDLES`] rationale; `MapMemoryDma` flood).
pub const MAX_LIVE_MAPPINGS: usize = 1 << 18;
/// Maximum parked (order-tolerance) facts of any one kind — a hostile guest can name
/// endpoints that never arrive, so each parked table is bounded too (dangling-`Dup`,
/// orphan-`SetPageDir`, orphan-`MapMemoryDma` floods). See [`MAX_LIVE_HANDLES`].
pub const MAX_PARKED: usize = 1 << 18;
/// ★ G9 — the largest roster a device can be **realized** with (`NV_MAX_DEVICES`, the
/// bound RM itself enforces on `deviceInstance`: `ogkm src/nvidia/src/kernel/gpu/device.c`).
/// A realize-time configuration bound, never a guest-input bound — guest input is checked
/// against the actual roster ([`RmGraph::entitle`]), which is stricter and is the point.
pub const MAX_GPUS: usize = 32;

/// Errors from [`RmGraph::apply`]. All loud; the caller decides whether a guest
/// protocol violation is fatal or logged-and-refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RmGraphError {
    /// A second, *different* alloc for an existing key (an identical re-send is
    /// accepted idempotently — retried-RPC tolerance, testing strategy `wo_retried_rpc`).
    ConflictingAlloc(NodeKey),
    /// A second, *different* dup for an existing dst key.
    ConflictingDup(NodeKey),
    /// ★ G9 (§12.21) — a `Device` alloc named a physical-GPU instance this device was
    /// **not realized with**. The guest-supplied `deviceInstance` is the sole selector
    /// for which GPU a whole object subtree lands on, and it used to be accepted
    /// unbounded and unvalidated: every fresh value minted a `GpuTarget` with its own
    /// guest-physical window and delivery plane, and `targets` is never pruned.
    ///
    /// The refusal deliberately mirrors RM's: real RM rejects an out-of-range instance
    /// with `NV_ERR_INVALID_CLASS` (`ogkm src/nvidia/src/kernel/gpu/device.c:118-129`)
    /// and, for an in-range-but-unpopulated one, resolves through
    /// `gpumgrGetPrimaryForDevice`, which **fails open to GPU 0**
    /// (`gpu_mgr.c:688-691`). We refuse both, so a guest cannot distinguish us from a
    /// real single-GPU box by probing instances.
    InvalidDeviceInstance {
        /// The instance the guest named.
        instance: u32,
    },
    /// A second, *different* `MAP_MEMORY_DMA` at a `(vaspace, va)` already mapped
    /// (an identical re-send is idempotent). Overlapping/replacing a live mapping
    /// without an eager unmap is a loud fault (the `ALREADY-MAPPED` collision class).
    ConflictingMap {
        /// The VASpace RESOURCE whose VA is doubly-mapped (★ §12.41 — by identity, not
        /// by origin handle: two live VASpaces can report at one recycled handle value).
        vaspace: ResourceKey,
        /// The colliding VA.
        va: GpuVa,
    },
    /// ★ §12.27 — a **Client** root alloc that declared no [`ClientKind`]. Refused at
    /// the declaring event, because both available guesses are catastrophic and
    /// MISS=FAULT forbids either: "user" folds the guest kernel's UVM session into a
    /// guest process's blast radius (and, through it, every other process — #14
    /// un-fixed), "kernel" folds a guest process into the guest kernel's isolate.
    /// The fact is present on every real `NV01_ROOT` (RM stamps `processID`
    /// unconditionally, `ogkm rpc.h:67-77`), so an absent one means the ABI seam
    /// failed to decode it — a defect that must be loud where it happens, not
    /// papered over with a default (the deleted `unwrap_or(0)` disease).
    UndeclaredClientKind(NodeKey),
    /// ★ §12.27 — an event named client handle **0**, which is reserved. `0` is
    /// `NV01_NULL_OBJECT`: RM mints client handles from `RS_CLIENT_HANDLE_BASE` and can
    /// never produce it, and the projection reserves it as the **system component's**
    /// anchor (the guest kernel's `Proc`). Refusing it here is what makes that
    /// reservation a fact rather than a hope — otherwise a guest could declare client
    /// `0`, anchor a user component at the reserved label, and have its PDBs and vChids
    /// resolve onto the system proc.
    ReservedClient(HClient),
    /// ★ §12.27 — a second, *different* client root inside one namespace. In RM the
    /// `hClient` **is** its root object's handle, so this cannot happen on a real
    /// driver; permitting it would let a guest declare two roots with different
    /// [`ClientKind`]s and make the whole namespace's classification a tie-break.
    DuplicateClientRoot {
        /// The root this namespace already has.
        existing: NodeKey,
        /// The second root that was refused.
        attempted: NodeKey,
    },
    /// ★★★ §12.38 — an event named a **client namespace that has never declared a root**
    /// (RM's `NV_ERR_INVALID_CLIENT`). THE use-before-exist refusal; see
    /// [`RmGraph::undeclared_namespace`] for the rule, its exemptions and its citations.
    ///
    /// **A protocol violation, not a missing fact** — and that distinction is the whole
    /// of §12.38's corrected criterion: order-independence holds where the NVIDIA
    /// protocol *allows* an ordering, not wherever an ordering is merely expressible.
    ///
    /// **What it closes.** The `Dup` arm of this rule is a cross-process isolation break.
    /// A `DUP_OBJECT` planted into a namespace nobody owned yet used to be accepted and
    /// parked; the edge then fired the instant an unrelated later process declared that
    /// `hClient`, merging it into the *planter's* live `Proc` — one isolate, one GPA
    /// arena, one host VAS, i.e. #14 un-fixed for a chosen pair, at the cost of one dup
    /// per guessable `hClient` (in RM the `hClient` **is** its root object's handle).
    /// Refusing at acceptance makes that state unrepresentable rather than detected
    /// downstream (`l1_mean.rs`'s
    /// `a_planted_dup_alias_cannot_squat_a_later_process_into_the_attackers_proc`).
    ///
    /// The `Alloc`/`SetPageDir`/`MapMemoryDma` arms close the same shape one level down:
    /// an object allocated into a namespace that had not declared itself used to mint a
    /// whole user `ProcBoundary` — isolate, GPA arena, routable `Vas` — anchored at a
    /// client of **unknown [`kayfabe_arch::ClientKind`]**, which is precisely the guess
    /// §12.27 refuses to make. Declaring the root *afterwards* as `Kernel` then migrated
    /// those objects to the system component, i.e. the guest kernel could obtain a real
    /// user data plane (the thing `FwdFault::SystemDataPlane` exists to forbid) simply by
    /// declaring its client root last.
    ///
    /// ★★ **NARROWED (§12.39, finding 1).** This entry's own summary once read *"an
    /// object allocated into an undeclared namespace mints no boundary"*, full stop. That
    /// is true **at alloc time** — the alloc is refused here — and it was **false after a
    /// root free**, which is now the only way a namespace can be undeclared at all: a
    /// resource kept alive by a foreign `DUP_OBJECT` alias outlives its namespace's root
    /// (faithful RM refcounting), still reports at its origin `(client, handle)`, and so
    /// still mints its owning component's boundary. That survival is **correct and
    /// deliberate** — retiring the owning `Proc` would free host memory RM says is live
    /// (`cross_proc_lifetime.rs`) — but the *classification* of such a namespace was a
    /// default rather than a fact until [`Resource::owner_kind`] recorded it. This rule
    /// closes the never-declared case; the recycled case is §12.39's.
    UndeclaredClient(HClient),
    /// Free of a handle that references no resource (never allocated or dup'd).
    FreeUnknown(NodeKey),
    /// A capacity-bounded table was full: a hostile guest tried to grow the graph
    /// past its bound. Loud-fault, never OOM (boundary-1). The existing graph is
    /// unchanged — the offending event is simply refused.
    CapacityExceeded(Capacity),
}

/// The RM resource graph. Facts in, projections out; no policy.
///
/// Resources are refcounted by their live-handle set; the handle table maps every
/// live `(client, handle)` to the resource it references (origin alloc or dup alias).
#[derive(Debug, Clone)]
pub struct RmGraph {
    /// Live resources, keyed by their stable origin id.
    resources: BTreeMap<ResId, Resource>,
    /// Every live handle → the resource it references **and how it was declared**
    /// ([`HandleRef`]). The origin handle and each dup alias both appear here; freeing a
    /// handle removes exactly its entry. The `Origin`/`Alias` discriminator is the
    /// recorded protocol fact every "is this the allocation?" question reads.
    handles: BTreeMap<NodeKey, HandleRef>,
    /// Dup edges whose source resource is not observed *yet* (order tolerance): a
    /// `Dup` may arrive before its `src`'s `Alloc`. Kept as `dst → src` and resolved
    /// lazily by [`Self::resource_of`]. When `src` becomes known and `dst` is used,
    /// the edge still resolves; freeing either end prunes it.
    pending_dups: BTreeMap<NodeKey, NodeKey>,
    /// `SET_PAGE_DIRECTORY` facts whose target handle is not observed *yet* (order
    /// tolerance): declaring-handle → declared PDB. Drained onto the resource as soon
    /// as its handle resolves (see [`Self::resolve_pending_pdbs`]).
    pending_pdbs: BTreeMap<NodeKey, Pdb>,
    /// ★★★★ **§16.28 — each Device's DEFAULT VA space, by the transient handle RM named
    /// it with.** Key = the **Device**'s `(client, handle)`; value = the `hVASpace` of the
    /// `FERMI_VASPACE_A` alloc that declared
    /// [`kayfabe_abi::bringup::NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE`] under it.
    ///
    /// # ⊘ Why this is not the handle table, and why the handle's free must not clear it
    ///
    /// The alloc RM issues for a device-default VA space is a **naming** operation: it
    /// mints a handle so the GSP side has something to call the VA space, publishes the
    /// reserved PDEs at that handle, and frees it again three RPCs later
    /// (`gpu_vaspace.c:4101 → 4113 → 4128 → 4135`). The VA space itself — `pDevice->pVASpace`
    /// — is untouched by that free and dies only with the Device
    /// (`device_share.c:307-320`). So this map is keyed on the **Device**, which is the
    /// thing RM says owns the address space, and is cleared when the *Device's* handle is
    /// freed. The transient VASpace handle's own `Free` prunes nothing here, because it
    /// is not a key.
    ///
    /// ⚠ **This map does not keep anything alive.** The VASpace resource still dies with
    /// its handle exactly as before, `refs` keeps its "never empty for a live resource"
    /// invariant, and no lifetime is extended. What survives is one `HObject` — the
    /// *name* the guest's own publication is filed under
    /// (`kayfabe_device::gvaspub`'s key is `(hClient, hObject)`), which is why a name is
    /// all that is needed.
    ///
    /// ⚠ Bounded (boundary-1) by [`MAX_PARKED`]: a guest may allocate index-3 VA spaces
    /// under fabricated Device handles. A re-declaration of an existing key REPLACES and
    /// is never gated, for [`Self::pending_pdbs`]'s reason: last declaration wins is
    /// protocol-legal and refusing a replacement costs nothing but hangs a legal guest.
    device_default_vas: BTreeMap<NodeKey, HObject>,
    /// Live DMA mappings, keyed by `(vaspace resource, va)`. Each holds a reference to
    /// the memory resource it maps (counted in `Resource::map_refs`).
    mappings: BTreeMap<MapKey, Mapping>,
    /// The memory `ResId` each mapping references — kept alongside `mappings` so a
    /// mapping can release its memory ref even after ALL of that memory's *handles*
    /// were freed (the resource is then reachable only by ResId, kept alive by this
    /// very map-ref). Faithful RM refcounting requires this back-pointer.
    map_mem_res: BTreeMap<MapKey, ResId>,
    /// `MapMemoryDma` facts whose VASpace or memory handle is not observed *yet*
    /// (order tolerance), replayed once both resolve. A **set** (not a Vec): a hostile
    /// guest can flood orphan maps, and a linear-scan dedup would make that O(n²) CPU —
    /// a complexity DoS even under the count cap. Ordered so replay is deterministic.
    pending_maps: BTreeSet<PendingMap>,
    /// Monotonic resource-id counter — never reused, so a re-allocated handle value
    /// mints a fresh resource distinct from a survivor a dup still holds.
    next_res_id: u64,
    /// ★ G9 (§12.21) — the **entitlement**: the physical-GPU instances this device was
    /// realized with. A `Device` alloc naming anything else is refused at alloc time.
    /// Defaults to `{0}` (the single-GPU realize); [`RmGraph::entitle`] sets it.
    gpus: BTreeSet<u32>,
    /// ★ §12.27 — client namespace → its ONE root client resource. An **index**, not a
    /// second source of truth: it is derived from the same `Alloc`/free events as
    /// `resources` and always points at a live entry there.
    ///
    /// It exists for complexity, not convenience (the G10 lesson, §12.22): the
    /// one-root-per-namespace check and the projection's per-client kind lookup are both
    /// "find this namespace's root", and a scan would make them O(clients × resources)
    /// with both factors guest-driven — the same quadratic shape the parked-map set and
    /// the condemnation pass were already hardened against.
    client_roots: BTreeMap<HClient, ResId>,
    /// ★★★ §12.41 — resource IDENTITY → resource. An **index**, not a second source of
    /// truth: every entry is inserted and removed with its [`Resource`] in `resources`.
    ///
    /// It exists for two jobs that are the same job. (1) It makes
    /// [`Self::next_incarnation`] a range query over the (tiny) set of live incarnations
    /// at one handle value instead of a scan of every resource — the G10 complexity rule
    /// (§12.22): an O(n) probe per `Alloc` is O(n²) on a guest-driven alloc flood, i.e. a
    /// complexity DoS. (2) It resolves a [`ResourceKey`] to its resource in O(log n),
    /// which is what let the five by-origin-key **linear scans** §12.40 finding 4 had to
    /// make `rev()`-newest-wins become exact lookups — the projection called two of them
    /// per node, so the old shape was O(resources²) per event as well.
    by_origin: BTreeMap<ResourceKey, ResId>,
    /// ★★★ §12.42 — every LIVE client-namespace **declaration**, keyed by its
    /// order-independent [`ClientKey`]. An **index**, not a second source of truth: an
    /// entry is created by the root `Alloc` that declared it and refcounted by the
    /// resources that namespace allocates, so it disappears exactly when the declaration
    /// owns nothing live any more.
    ///
    /// It replaces the ascending-`ResId` scan of every resource that
    /// [`Self::client_declarations`] used to be — which was O(resources) per projection
    /// *and*, because it wrote one entry per `HClient`, kept only the **newest**
    /// declaration per value. That newest-wins collapse is §12.41 §4's defect: an older
    /// declaration whose resources a kernel alias keeps alive stopped projecting the
    /// instant the value was re-declared, its component was vacated, and its host memory
    /// was freed while RM still refcounted it.
    ///
    /// Bounded by the live resource count (every entry owns at least one), so it is not a
    /// guest-growable ghost log — the G10 complexity rule (§12.22).
    decls: BTreeMap<ClientKey, Declaration>,
}

/// ★ §12.27 — the reserved client handle. `0` is `NV01_NULL_OBJECT`, which RM can never
/// mint as an `hClient` (client handles come from `RS_CLIENT_HANDLE_BASE`), so the
/// projection claims it as the **system component's** anchor and the graph refuses it as
/// guest input ([`RmGraphError::ReservedClient`]).
pub const RESERVED_CLIENT: HClient = HClient(0);

/// A parked `MAP_MEMORY_DMA` awaiting its VASpace/memory handles (order tolerance).
/// `Ord` so the parked set dedups + caps in O(log n) (see [`RmGraph::pending_maps`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PendingMap {
    client: HClient,
    vaspace: HObject,
    memory: HObject,
    va: GpuVa,
    offset: u64,
    len: u64,
}

impl Default for RmGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl RmGraph {
    /// Empty graph, entitled to the **single-GPU** roster `{0}` (the realize-time
    /// target). [`RmGraph::entitle`] widens it for a multi-GPU device.
    #[must_use]
    pub fn new() -> Self {
        RmGraph {
            resources: BTreeMap::new(),
            handles: BTreeMap::new(),
            pending_dups: BTreeMap::new(),
            pending_pdbs: BTreeMap::new(),
            device_default_vas: BTreeMap::new(),
            mappings: BTreeMap::new(),
            map_mem_res: BTreeMap::new(),
            pending_maps: BTreeSet::new(),
            next_res_id: 0,
            gpus: BTreeSet::from([0]),
            client_roots: BTreeMap::new(),
            by_origin: BTreeMap::new(),
            decls: BTreeMap::new(),
        }
    }

    /// ★ G9 (§12.21) — declare the device's **entitlement**: the physical-GPU instances
    /// it was realized with. Set once, at realize.
    ///
    /// This is the cap that matters, and it is deliberately *not* `NV_MAX_DEVICES`. RM
    /// already enforces `deviceInstance < 32` in three places (`ogkm alloc_free.c:1372`,
    /// `device.c:118-129` → `NV_ERR_INVALID_CLASS`, `device.c:357`), so a `< 32` bound
    /// here would be security theatre: it would still let a guest mint 31 `GpuTarget`s —
    /// 31 guest-physical windows and 31 delivery planes — on a single-GPU box, from ~20
    /// lines of raw `NV_ESC_RM_ALLOC` on `/dev/nvidiactl` with **no patched guest kernel**
    /// (stock userspace never emits one). The real check in RM is `osIsGpuAccessible` →
    /// `nv_is_gpu_accessible` (`kernel-open/nvidia/nv.c:5904-5910`), which scans the host
    /// process's fd table; device allocs go through `/dev/nvidiactl`, which carries **no
    /// GPU identity**, so `deviceInstance` is the sole selector — and
    /// `gpumgrGetPrimaryForDevice` **fails open to GPU 0** for an in-range-but-unpopulated
    /// instance (`gpu_mgr.c:688-691`). Our entitlement is the equivalent of RM's fd-table
    /// scan: the roster this device was actually realized with, nothing more.
    ///
    /// # Panics
    /// If the roster is empty, or larger than [`MAX_GPUS`] — a realize-time configuration
    /// error, never guest input.
    pub fn entitle(&mut self, gpus: impl IntoIterator<Item = u32>) {
        let gpus: BTreeSet<u32> = gpus.into_iter().collect();
        assert!(
            !gpus.is_empty() && gpus.len() <= MAX_GPUS,
            "a device is realized with 1..={MAX_GPUS} GPUs"
        );
        self.gpus = gpus;
    }

    /// This namespace's ONE client-root handle, if its root has been observed and is
    /// still live (§12.27). O(log n) — an index read, never a scan.
    ///
    /// ★★★ **MISS ⇒ FAULT, corrected in §12.38.** This doc used to read *"an object may
    /// legally arrive before its client root (order tolerance, decision #4), so absence
    /// here means not yet knowable"*. **That is false about RM**: every ioctl-reachable
    /// entry point resolves `hClient` in the client database before anything else and
    /// answers `NV_ERR_INVALID_OBJECT_HANDLE` / `NV_ERR_INVALID_CLIENT` when it is not
    /// there (`ogkm src/nvidia/src/libraries/resserv/src/rs_server.c:778`, `:1503`,
    /// `:1674`, `:2218`). And the fact is one we would always have observed — a client
    /// root is on the GSP wire by construction, since that RPC is where
    /// [`AllocFacts::client_kind`] comes from. So absence here is **never knowable in a
    /// legal trace**, and [`RmGraph::undeclared_namespace`] refuses the event.
    ///
    /// This predicate is therefore now a *gate*, not a deferral: by the time any arm of
    /// `apply` runs, every namespace the event names exists. Read-side callers
    /// ([`Self::client_kinds`], the projection) still treat an absent kind as "groups with
    /// nobody", which remains right — a namespace can be emptied by a `Free` between the
    /// event that created an alias and the projection that reads it.
    /// ★★★ §12.41 — the incarnation ordinal a resource allocated at `key` **now** takes:
    /// the smallest one no LIVE resource at that key already holds.
    ///
    /// Almost always `0`. It is non-zero exactly when a `DUP_OBJECT` alias is keeping an
    /// earlier resource alive past its origin handle's free (RM refcounts it —
    /// `ogkm src/nvidia/src/kernel/mem_mgr/mem.c:1027-1031`) and the guest re-allocates
    /// that handle value, which RM permits with no quarantine
    /// (`rs_client.c:1137` free, `:1446-1470` the live-only validation).
    ///
    /// Reuse of a **dead** incarnation's ordinal is deliberate and safe: identity only has
    /// to separate things that are simultaneously live, and bounding the ordinal by the
    /// live set is what keeps this table from being a guest-growable ghost log.
    fn next_incarnation(&self, key: NodeKey) -> u32 {
        // The live incarnations at one handle value, ascending. Bounded by how many
        // dup-kept ghosts the guest has parked there, and each one costs it a live alias
        // (so `MAX_LIVE_HANDLES` bounds it).
        let mut want = 0u32;
        for k in self
            .by_origin
            .range(ResourceKey::first(key)..=ResourceKey::last(key))
            .map(|(k, _)| k.incarnation)
        {
            if k != want {
                break;
            }
            want += 1;
        }
        want
    }

    /// ★★★ §12.41 — the resource whose identity is `id`, or `None` if it is not live.
    /// The ONE `ResourceKey` → resource resolution, in O(log n) through
    /// [`Self::by_origin`].
    fn resource_at(&self, id: ResourceKey) -> Option<&Resource> {
        let rid = self.by_origin.get(&id)?;
        self.resources.get(rid)
    }

    /// ★★★ §12.41 — the resource a bare origin `NodeKey` names, when the caller has no
    /// identity to offer: **the incarnation currently declared there**.
    ///
    /// "Currently declared" is a recorded fact, not a tie-break: the resource whose
    /// ORIGIN handle is live at `key` (there is at most one — the handle table is keyed
    /// by handle) is the current tenant. Only when no origin handle is live there — every
    /// incarnation is a dup-kept ghost — does this fall back to the newest, which is
    /// §12.40 finding 4's `rev()` rule preserved exactly, now as an O(k) max over the
    /// key's own incarnations rather than a reverse scan of every resource.
    ///
    /// Callers that DO have an identity must use [`Self::resource_at`]; this exists for
    /// the handle-shaped public API ([`Self::pdb_of`], [`Self::references`], …), whose
    /// question genuinely is "what is at this handle value now?".
    fn current_at(&self, key: NodeKey) -> Option<&Resource> {
        if let Some(h) = self.handles.get(&key)
            && h.is_origin()
        {
            return self.resources.get(&h.res());
        }
        self.by_origin
            .range(ResourceKey::first(key)..=ResourceKey::last(key))
            .map(|(_, &rid)| rid)
            .max()
            .and_then(|rid| self.resources.get(&rid))
    }

    fn client_root_of(&self, client: HClient) -> Option<NodeKey> {
        let id = self.client_roots.get(&client)?;
        self.resources.get(id).map(|r| r.node.key)
    }

    /// ★ §12.27 — every live client namespace's **declared** [`ClientKind`], ascending
    /// client order (deterministic). THE input to decision #14's grouping rule.
    ///
    /// A bulk iterator rather than a per-client lookup on purpose: the projection needs
    /// all of them, and a convenience single-key accessor would invite the
    /// O(clients × clients) call pattern this index exists to avoid.
    ///
    /// A client absent from this iterator has no live root. ★ §12.38 — that is no longer
    /// the "an object arrived before its client root" case, which is now refused at
    /// acceptance ([`Self::undeclared_namespace`]); what remains is a namespace whose root
    /// has been **freed** while a `DUP_OBJECT` alias elsewhere keeps some of its resources
    /// alive (§12.27's `drop_handle` rule — the namespace is empty and free to
    /// re-declare). Such a client is NOT "probably a user client": it groups with nobody
    /// until a fresh root lands, which is the MISS=FAULT posture rather than a guess.
    ///
    /// ★★★ §12.42 — it yields the [`ClientKey`] of the LIVE-ROOT declaration, not a bare
    /// `HClient`. One `hClient` value may name several live declarations at once (at most
    /// one of which has a live root, by [`RmGraphError::DuplicateClientRoot`]), and
    /// "which one is the guest talking to right now" is precisely the live-root one.
    pub fn client_kinds(&self) -> impl Iterator<Item = (ClientKey, ClientKind)> + '_ {
        self.client_roots.iter().filter_map(|(&c, id)| {
            let r = self.resources.get(id)?;
            let kind = r.node.facts.client_kind?;
            Some((
                ClientKey {
                    client: c,
                    incarnation: r.owner_incarnation,
                },
                kind,
            ))
        })
    }

    /// ★★★ §12.42 — the incarnation ordinal a namespace declared at `client` **now**
    /// takes: the smallest one no LIVE declaration at that value already holds.
    ///
    /// Almost always `0`. It is non-zero exactly when a `DUP_OBJECT` alias is keeping an
    /// earlier declaration's resources alive past its root's free (RM refcounts them —
    /// `ogkm src/nvidia/src/kernel/mem_mgr/mem.c:1027-1031`) and the guest re-declares the
    /// value, which RM permits with no quarantine at all (`rs_server.c:612` honours a
    /// caller-supplied root; `:3319-3341` generates one from a wrapping index with no free
    /// list).
    ///
    /// Reuse of a **dead** declaration's ordinal is deliberate and safe, for the reason
    /// [`Self::next_incarnation`] states: identity only has to separate things that are
    /// simultaneously live, and bounding the ordinal by the live set is what keeps
    /// [`Self::decls`] from being a guest-growable ghost log. Anything that must survive a
    /// declaration's *death* — [`crate::gpu::Spine::condemned`] — keys on [`ClientId`],
    /// which is never reused.
    fn next_client_incarnation(&self, client: HClient) -> u32 {
        let mut want = 0u32;
        for k in self
            .decls
            .range(ClientKey::first(client)..=ClientKey::last(client))
            .map(|(k, _)| k.incarnation)
        {
            if k != want {
                break;
            }
            want += 1;
        }
        want
    }

    /// ★★★ §12.42 — record one more live resource for `key`'s declaration, minting the
    /// entry if this is the root `Alloc` that declared it.
    fn retain_declaration(&mut self, key: ClientKey, id: ClientId, kind: ClientKind) {
        let d = self
            .decls
            .entry(key)
            .or_insert(Declaration { id, kind, refs: 0 });
        debug_assert_eq!(
            d.id, id,
            "two identities at one live ClientKey — the incarnation ordinal was minted \
             against a stale live set"
        );
        d.refs += 1;
    }

    /// ★★★ §12.42 — release one live resource of `key`'s declaration; the declaration
    /// itself dies with its last one. That is the moment its ordinal becomes reusable,
    /// and it is the same moment [`Self::by_origin`] releases a resource's.
    fn release_declaration(&mut self, key: ClientKey) {
        if let Some(d) = self.decls.get_mut(&key) {
            d.refs -= 1;
            if d.refs == 0 {
                self.decls.remove(&key);
            }
        }
    }

    /// ★★★ §12.41/§12.42 — destroy a resource and every index that names it. The ONE
    /// place a [`Resource`] leaves the graph, so the identity index and the declaration
    /// refcount can never drift from `resources`.
    fn destroy_resource(&mut self, id: ResId) {
        let Some(r) = self.resources.remove(&id) else {
            return;
        };
        self.by_origin.remove(&r.node.id());
        self.release_declaration(r.owner_key());
    }

    /// ★★★ §12.39 — the declaration each client namespace **projects under**: its live
    /// client root's [`ClientKind`] if it still has one, else the [`ClientKind`] recorded
    /// on the resources that namespace allocated and that a foreign alias keeps alive.
    ///
    /// [`Self::client_kinds`] answers *"is there a LIVE declared root here?"* and is the
    /// input to the grouping predicate, which requires positive evidence about both ends
    /// of a dup. This answers the different question the **assignment** pass asks —
    /// *"which side of the user/kernel line does this resource's component sit on?"* —
    /// and it has an answer even after the root is gone, because RM's refcounting keeps
    /// the resource alive and `cross_proc_lifetime.rs` pins that the owning `Proc` must
    /// survive with it (retiring it would free host memory RM says is live).
    ///
    /// The two must not be conflated: reading `client_kinds`'s absence as "user" was the
    /// defect [`Resource::owner_kind`] documents. Nothing here mints a boundary that did
    /// not already exist — every key is a namespace that owns a live resource — it only
    /// makes the classification a **read of a declared fact** instead of a default.
    ///
    /// ★★★ §12.42 — **EVERY live declaration, not one per `hClient` value.** This used to
    /// return a `BTreeMap<HClient, _>`, i.e. exactly one slot per value with "a live root
    /// wins, else the newest orphan". That collapse is §12.41 §4's defect: after
    /// `declare A → alias → free root → re-declare A`, generation 1's still-live resources
    /// answered to **no** declaration, so they projected into no boundary, their component
    /// vanished, and `stage_dropped_vases` released the host VAS and published backing of
    /// a resource RM still refcounts for a live kernel client
    /// (`ogkm .../mem_mgr/mem.c:1027-1031`) — the corruption direction. Keyed by
    /// [`ClientKey`], the two generations are two entries and both project.
    pub fn client_declarations(&self) -> BTreeMap<ClientKey, (ClientId, ClientKind)> {
        self.decls
            .iter()
            .map(|(&k, d)| (k, (d.id, d.kind)))
            .collect()
    }

    /// ★★★ §12.39 Part B / §12.42 — every live resource with the **declaration** that
    /// allocated it. Ascending origin-key order, exactly like [`Self::nodes`].
    ///
    /// Deliberately a separate iterator rather than a field on [`RmNode`]: `RmNode` is
    /// compared whole across shuffled orders by the order-independence properties, and the
    /// owner is a derived fact about the namespace rather than about the object.
    pub fn nodes_with_owner(&self) -> impl Iterator<Item = (&RmNode, ClientKey)> {
        self.resources.values().map(|r| (&r.node, r.owner_key()))
    }

    /// ★★★ §12.39 Part B / §12.42 — the **declaration** that allocated the resource `key`
    /// references (dup-aliases followed). `None` if the handle resolves to nothing.
    ///
    /// The dup-grouping predicate reads this through the **destination** handle, which is
    /// live by construction, rather than through the edge's reported `src` key — the `src`
    /// a dup edge reports is the resource's origin `(client, handle)`, and that `HClient`
    /// is exactly the recyclable value the identity model exists to stop trusting.
    #[must_use]
    pub fn owner_key_of(&self, key: NodeKey) -> Option<ClientKey> {
        let id = self.resource_of(key)?;
        self.resources.get(&id).map(Resource::owner_key)
    }

    /// Every client namespace one event names (one for most events, two for a `Dup`).
    /// Enumerated centrally so a future `RmEvent` variant carrying a client is a
    /// mistake in exactly one place — the [`RESERVED_CLIENT`] guard reads this.
    fn clients_named(ev: RmEvent) -> [Option<HClient>; 2] {
        match ev {
            RmEvent::Alloc { client, .. }
            | RmEvent::SetPageDir { client, .. }
            | RmEvent::MapMemoryDma { client, .. }
            | RmEvent::Unmap { client, .. }
            | RmEvent::Free { client, .. } => [Some(client), None],
            RmEvent::Dup { src, dst } => [Some(src.client), Some(dst.client)],
        }
    }

    /// ★★★ §12.38 — **THE NAMESPACE-EXISTENCE RULE: no event may name a client namespace
    /// that does not exist.** Returns the offending [`HClient`], or `None` if the event is
    /// legal on this axis.
    ///
    /// ## Why this is a FAULT and not a DEFER
    ///
    /// Decision #4 (amended, §12.38) is order-independence over **legal protocol facts**,
    /// not over every order a stream can express. RM resolves `hClient` in the client
    /// database as the *first* thing every ioctl-reachable entry point does, and answers
    /// `NV_ERR_INVALID_OBJECT_HANDLE` / `NV_ERR_INVALID_CLIENT` when it is not there:
    ///
    /// | our event | RM entry point | `ogkm src/nvidia/src/libraries/resserv/src/rs_server.c` |
    /// |---|---|---|
    /// | `Alloc` (non-root) | `serverAllocResource` | `:778` (`_serverLockClientWithLockInfo`), then `clientValidate` `:824` |
    /// | `Dup` (**both** ends) | `serverCopyResource` | `:1674` (`_serverLockDualClientWithLockInfo`), then `clientValidate(dst)` `:1696` |
    /// | `SetPageDir` (an RM control) | `serverControl` | `:1503` / `:1519`, then `clientValidate` `:1547` |
    /// | `MapMemoryDma` | `serverInterMap` | `:2218`, then `clientValidate` `:2232` |
    ///
    /// The miss is `NV_ERR_INVALID_OBJECT_HANDLE` inside the lock helper (`:3486-3487`,
    /// `:3547-3550`); `clientValidate`'s own refusal one line later is
    /// `NV_ERR_INVALID_CLIENT` (`ogkm src/nvidia/src/kernel/rmapi/client.c:782`). So the
    /// guest's own RM never emits any of these for a namespace that does not exist —
    /// **no legal trace is lost by refusing them.**
    ///
    /// And the earlier fact is one we *would have observed*: a client root
    /// (`NV01_ROOT`) is always on the GSP wire — the `GSPALLOC hClient=… processID=…`
    /// records are literally where [`AllocFacts::client_kind`] comes from
    /// (`docs/reference/rm_semantics_measured.md` §4). That is what separates this from
    /// the **DEFER-for-observation** cases: a dup's source *object* may genuinely be
    /// unobserved (only 25 of 82 measured dups reach GSP, §3), so it parks; a client
    /// *root* cannot be.
    ///
    /// ## The exemptions, and why each is not an inconsistency
    ///
    /// - **The client-root `Alloc` itself** — it is the event that *creates* the
    ///   namespace (`serverAllocClient`, `rs_server.c:764`, which bypasses the client
    ///   lock for exactly this reason). Its own required fact,
    ///   [`AllocFacts::client_kind`], is already checked at that arm.
    /// - **`Free` and `Unmap` — the TEARDOWN verbs.** A namespace with no root is
    ///   indistinguishable from a namespace whose root was *just freed* (freeing a root
    ///   drops every handle in it, so nothing is left behind to tell them apart), and a
    ///   teardown verb arriving after its namespace died is a benign race the guest can
    ///   legitimately produce — `Unmap`'s unknown-VAS arm is silently `Ok` for precisely
    ///   that reason, and `Free`'s is the more precise [`RmGraphError::FreeUnknown`].
    ///   Faulting here would turn a race into a device-level refusal, which is the
    ///   FAULT-that-should-DEFER direction of the asymmetry. The rule loses nothing:
    ///   once no *creating* event can name an undeclared namespace, an undeclared
    ///   namespace holds no handles, so both verbs are already inert by construction.
    fn undeclared_namespace(&self, arch: &dyn Arch, ev: RmEvent) -> Option<HClient> {
        let missing = |c: HClient| self.client_root_of(c).is_none().then_some(c);
        match ev {
            // The root alloc CREATES the namespace — the one event that may name one
            // that does not exist yet.
            RmEvent::Alloc { class, .. } if matches!(arch.classify(class), ObjectKind::Client) => {
                None
            }
            RmEvent::Alloc { client, .. }
            | RmEvent::SetPageDir { client, .. }
            | RmEvent::MapMemoryDma { client, .. } => missing(client),
            // `dst` first: it is the security-relevant end (the squat vector), so it is
            // the one a refusal names when both are undeclared.
            RmEvent::Dup { src, dst } => missing(dst.client).or_else(|| missing(src.client)),
            // Teardown verbs — see the doc above.
            RmEvent::Unmap { .. } | RmEvent::Free { .. } => None,
        }
    }

    /// Populate the sticky GPU-target cache for any resource that can now resolve its
    /// `Device` ancestor (never overwriting a value already cached — a target is an
    /// immutable fact, so once resolved it survives the source's later free).
    ///
    /// Called ONLY when a new `Device` is allocated (back-filling descendants that
    /// declared before their device — the order-tolerance case). A per-resource inline
    /// resolve in the `Alloc` handler covers the common case, so this O(n) scan runs at
    /// most once per Device alloc (a handful per client), never per object — a flood of
    /// non-device allocs stays O(1) amortized (boundary-1).
    fn cache_targets(&mut self) {
        let pending: Vec<(ResId, NodeKey)> = self
            .resources
            .iter()
            .filter(|(_, r)| r.gpu.is_none())
            .map(|(&id, r)| (id, r.node.key))
            .collect();
        for (id, origin) in pending {
            if let Some(g) = self.walk_gpu(origin)
                && let Some(r) = self.resources.get_mut(&id)
            {
                r.gpu = Some(g);
            }
        }
    }

    /// Live walk of `key`'s parent chain to its nearest `Device` ancestor's declared
    /// target (used to POPULATE the cache; [`Self::gpu_of`] reads the cache first).
    fn walk_gpu(&self, key: NodeKey) -> Option<GpuId> {
        let mut node = self.origin_of(key)?;
        for _ in 0..64 {
            if matches!(node.kind, kayfabe_arch::ObjectKind::Device) {
                // ★ G9 (§12.21): MISS = None, never a guess. A `Device` that declared no
                // instance leaves its descendants **unroutable** (deferred; a loud miss at
                // use) — it used to silently become GPU 0, which is a default-target guess
                // of exactly the kind `gpu_of` refuses everywhere else. A real Device
                // cannot be in this state: `deviceId` is a required field of
                // `NV0080_ALLOC_PARAMETERS`, so the fact is always observed.
                return node.facts.device_instance.map(GpuId);
            }
            let parent = NodeKey::new(node.key.client, node.parent);
            if parent == node.key {
                return None;
            }
            node = self.origin_of(parent)?;
        }
        None
    }

    /// Apply one declared fact. Idempotent for identical re-sends; conflicting
    /// redefinitions are loud errors. The sticky per-resource GPU-target cache is
    /// maintained inline (per-alloc resolve + Device-triggered back-fill), so a
    /// newly-arrived `Device` back-fills targets for objects that declared before it
    /// (order tolerance for the multi-GPU axis) without an O(n) scan per event.
    pub fn apply(&mut self, arch: &dyn Arch, ev: RmEvent) -> Result<(), RmGraphError> {
        // ★ §12.27 — the reserved client handle, checked for EVERY event (an object can
        // enter a namespace by `Alloc`, by a `Dup`'s dst, or by naming it as a `Dup`'s
        // src), so `HClient(0)` never appears anywhere in the graph and the projection's
        // system anchor is unreachable by construction.
        for c in Self::clients_named(ev).into_iter().flatten() {
            if c == RESERVED_CLIENT {
                return Err(RmGraphError::ReservedClient(c));
            }
        }
        // ★★★ §12.38 — no event may name a namespace that does not exist. Checked here,
        // centrally and before any arm, because that is where RM checks it (`hClient` is
        // resolved first by every ioctl-reachable entry point) and because ONE place is
        // what makes the bad state unrepresentable downstream rather than detected there.
        if let Some(c) = self.undeclared_namespace(arch, ev) {
            return Err(RmGraphError::UndeclaredClient(c));
        }
        match ev {
            RmEvent::Alloc {
                client,
                parent,
                handle,
                class,
                facts,
            } => {
                let key = NodeKey::new(client, handle);
                // ★★★ §12.41 — the DECLARATION, incarnation-free. The ordinal is a
                // property of the graph's live state, not of the event, so it is minted
                // only on the arm that creates a resource (below); the idempotent-retry
                // arm compares against this shape so a re-sent alloc never mints one.
                let node = RmNode {
                    key,
                    incarnation: 0,
                    parent,
                    class,
                    kind: arch.classify(class),
                    facts,
                };
                // ★ G9 (§12.21): a `Device` may only name a physical GPU this device was
                // REALIZED with. Checked here, at the alloc, because that is where RM
                // checks it (`ogkm device.c:118-129`, → `NV_ERR_INVALID_CLASS`) — so a
                // guest cannot tell us from a real single-GPU box. Note the same instance
                // twice under one client is LEGAL on bare metal (`device.c:368-380`
                // rejects it only under `IS_VIRTUAL`), so this is a membership test and
                // never a uniqueness one.
                if matches!(node.kind, kayfabe_arch::ObjectKind::Device)
                    && !node
                        .facts
                        .device_instance
                        .is_none_or(|i| self.gpus.contains(&i))
                {
                    return Err(RmGraphError::InvalidDeviceInstance {
                        instance: node.facts.device_instance.unwrap_or_default(),
                    });
                }
                // ★ §12.27 — a Client root MUST declare its [`ClientKind`]. Checked at
                // the alloc, like the Device instance above and for the same reason:
                // this is where the fact is declared, so this is where its absence is
                // decidable. Deferring it would mean carrying an unclassifiable client
                // in the graph and answering "which `Proc` owns this?" with a guess.
                if matches!(node.kind, ObjectKind::Client) {
                    if node.facts.client_kind.is_none() {
                        return Err(RmGraphError::UndeclaredClientKind(key));
                    }
                    // ★ §12.27 — ONE root per namespace, so "what privilege is this
                    // client?" has exactly one answer. In RM the `hClient` **is** its
                    // root object's handle, so a second root inside one namespace is a
                    // protocol impossibility; allowing it would let a guest declare two
                    // roots with *different* kinds and make the classification
                    // order-dependent (whichever the projection happened to pick).
                    // Refused here rather than disambiguated later — ambiguity is a
                    // fault, never a tie-break (an identical re-send is handled by the
                    // idempotent arm below and never reaches this).
                    if let Some(existing) = self.client_root_of(client)
                        && existing != key
                    {
                        return Err(RmGraphError::DuplicateClientRoot {
                            existing,
                            attempted: key,
                        });
                    }
                }
                // A handle names EITHER an allocated resource OR a dup alias, never
                // both — so an alloc onto a handle already claimed by a (parked) dup is
                // a loud conflict, symmetric with the dup-onto-alloc rejection.
                if self.pending_dups.contains_key(&key) {
                    return Err(RmGraphError::ConflictingAlloc(key));
                }
                match self.handles.get(&key) {
                    // Idempotent retry: the same origin alloc re-sent. The handle must
                    // itself be the ORIGIN (a recorded fact) — a dup ALIAS that merely
                    // happens to reference an identically-shaped payload is a DIFFERENT
                    // declaration, and allocating over it is a loud conflict, exactly as
                    // real RM refuses a handle already in use
                    // (`NV_ERR_INSERT_DUPLICATE_OBJECT`). Comparing payloads alone would
                    // silently accept the alias as "the allocation" once a dup re-created
                    // a freed origin's key — the same discarded-fact defect as
                    // `is_client_root`'s (§12.25).
                    Some(h)
                        if h.is_origin()
                            && self.resources.get(&h.res()).map(|r| RmNode {
                                incarnation: 0,
                                ..r.node
                            }) == Some(node) =>
                    {
                        Ok(())
                    }
                    // The key is already taken (by a different alloc, or by a resolved dup) — loud.
                    Some(_) => Err(RmGraphError::ConflictingAlloc(key)),
                    None => {
                        // Boundary-1: a genuinely new resource grows the handle table —
                        // refuse a flood loudly rather than OOM. (Idempotent re-sends
                        // take the branches above and never reach here.)
                        if self.handles.len() >= MAX_LIVE_HANDLES {
                            return Err(RmGraphError::CapacityExceeded(Capacity::Handles));
                        }
                        // ★★★ §12.39 — record the DECLARING NAMESPACE's privilege on the
                        // resource ([`Resource::owner_kind`]), because the resource can
                        // outlive the namespace and the question is asked afterwards. A
                        // client root declares its own; every other object inherits its
                        // namespace's, which §12.38's gate guarantees is present (the
                        // root exists, and a root without a `ClientKind` was refused at
                        // its own alloc). The `Err` arm is therefore unreachable and is
                        // written as a loud refusal rather than an `expect` so that a
                        // future arm that reaches it fails contained, not fatally.
                        let owner_kind = if matches!(node.kind, ObjectKind::Client) {
                            node.facts.client_kind
                        } else {
                            self.client_root_of(client)
                                .and_then(|root| self.node(root))
                                .and_then(|n| n.facts.client_kind)
                        };
                        let Some(owner_kind) = owner_kind else {
                            return Err(RmGraphError::UndeclaredClientKind(key));
                        };
                        let id = ResId(self.next_res_id);
                        self.next_res_id += 1;
                        // ★★★ §12.39 Part B — and the namespace's IDENTITY. A client root
                        // owns itself (RM: the `hClient` IS its root object's handle,
                        // `rs_server.c:625`); every other object is owned by the root its
                        // namespace currently has, which §12.38's gate guarantees exists.
                        //
                        // ★★★ §12.42 — and with it the declaration's order-independent
                        // ORDINAL. A root `Alloc` mints a fresh one against the live set
                        // (non-zero exactly when an orphaned predecessor at this value is
                        // still alive on a foreign alias); every other object inherits the
                        // ordinal its namespace's live root carries.
                        let (owner, owner_incarnation) = if matches!(node.kind, ObjectKind::Client)
                        {
                            (ClientId(id.0), self.next_client_incarnation(client))
                        } else {
                            match self.client_roots.get(&client) {
                                Some(root) => {
                                    let inc =
                                        self.resources.get(root).map_or(0, |r| r.owner_incarnation);
                                    (ClientId(root.0), inc)
                                }
                                // Unreachable: `undeclared_namespace` refused this
                                // event already. Loud and contained, not an `expect`.
                                None => return Err(RmGraphError::UndeclaredClient(client)),
                            }
                        };
                        // ★★★ §12.41 — stamp the incarnation NOW, from the live set, and
                        // index the resource by its identity. A `Dup`-kept ghost at this
                        // very handle value is what makes the ordinal non-zero.
                        let node = RmNode {
                            incarnation: self.next_incarnation(key),
                            ..node
                        };
                        // Declared by `RM_ALLOC` → THE origin handle of this resource.
                        self.handles.insert(key, HandleRef::Origin(id));
                        self.by_origin.insert(node.id(), id);
                        self.resources.insert(
                            id,
                            Resource {
                                node,
                                refs: BTreeSet::from([key]),
                                pdb: None,
                                gpu: None,
                                map_refs: 0,
                                owner_kind,
                                owner,
                                owner_incarnation,
                            },
                        );
                        self.retain_declaration(
                            ClientKey {
                                client,
                                incarnation: owner_incarnation,
                            },
                            owner,
                            owner_kind,
                        );
                        if matches!(node.kind, ObjectKind::Client) {
                            // Upheld by the one-root check above: never overwrites.
                            self.client_roots.insert(client, id);
                        }
                        // ★★★★ §16.28 — **THE DEVICE-DEFAULT LATCH.** A `VaSpace` alloc
                        // declaring `index == NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE` is
                        // RM saying *"this handle is a name for the VA space my parent
                        // Device already owns"*, so the fact is filed against the PARENT,
                        // where it outlives the transient handle's `Free`. See
                        // [`RmGraph::device_default_vas`] and the constant's own docs for
                        // the four-RPC sequence this is the memory of.
                        //
                        // ⊘ The parent is `node.parent` — the RPC header's own `hParent`,
                        // not a params field — so nothing here can attribute an address
                        // space to a Device the guest did not name as the parent.
                        if matches!(node.kind, ObjectKind::VaSpace)
                            && node.facts.vaspace_role == Some(VaSpaceRole::DeviceDefault)
                        {
                            let dev = NodeKey::new(client, node.parent);
                            // Replacement is always allowed (last declaration wins); only
                            // a genuinely NEW key meets the cap — `pending_pdbs`' rule.
                            if self.device_default_vas.contains_key(&dev)
                                || self.device_default_vas.len() < MAX_PARKED
                            {
                                self.device_default_vas.insert(dev, key.handle);
                            }
                        }
                        // A dup / SetPageDir that arrived BEFORE this alloc parked
                        // itself; now that its target exists, promote each parked fact
                        // so the resource is refcounted and PDB-tagged correctly (and so
                        // it survives the source's later free, order-independently).
                        self.resolve_pending_dups();
                        self.resolve_pending_pdbs();
                        self.resolve_pending_maps();
                        // ★ Cache THIS resource's GPU target inline (cheap: a short
                        // parent walk) — the common case, O(1) per alloc. Only when the
                        // new node is a `Device` do we back-fill descendants that
                        // declared before it (order tolerance), a scan that runs at most
                        // once per Device — never per object, so a hostile alloc flood
                        // stays O(n), not O(n²) (boundary-1).
                        if let Some(g) = self.walk_gpu(key)
                            && let Some(r) = self.resources.get_mut(&id)
                        {
                            r.gpu = Some(g);
                        }
                        if matches!(arch.classify(class), kayfabe_arch::ObjectKind::Device) {
                            self.cache_targets();
                        }
                        Ok(())
                    }
                }
            }
            RmEvent::Dup { src, dst } => {
                // ★★★ §12.38 — both client namespaces exist by the time we get here (the
                // central gate above). What remains genuinely unordered is the dup's
                // SOURCE OBJECT, which parks: only 25 of 82 measured dups reach GSP, so a
                // source may be an object RM saw and we did not — DEFER-for-observation,
                // and faulting it would hang a legal guest.
                //
                // The dst handle must be FREE. It is already taken if it references a
                // live resource (`handles`) or names a parked (not-yet-resolved) edge
                // (`pending_dups`). Either way an identical re-send is idempotent and a
                // conflicting one is loud — dst must never end up doubly-bound.
                if let Some(existing) = self.handles.get(&dst) {
                    return if Some(existing.res()) == self.resource_of(src) {
                        Ok(()) // retry: dst already references THIS same resource
                    } else {
                        Err(RmGraphError::ConflictingDup(dst))
                    };
                }
                if let Some(parked_src) = self.pending_dups.get(&dst) {
                    return if *parked_src == src {
                        Ok(()) // retry: dst already parked against THIS same source
                    } else {
                        Err(RmGraphError::ConflictingDup(dst))
                    };
                }
                match self.resource_of(src) {
                    // Source known now: dst gets its own reference to it immediately.
                    Some(id) => {
                        // A new alias handle grows the handle table (boundary-1).
                        if self.handles.len() >= MAX_LIVE_HANDLES {
                            return Err(RmGraphError::CapacityExceeded(Capacity::Handles));
                        }
                        // Declared by `DUP_OBJECT` → an ALIAS, whatever handle VALUE it
                        // lands on (it may be the value a freed origin used to hold).
                        self.handles.insert(dst, HandleRef::Alias(id));
                        self.resources
                            .get_mut(&id)
                            .expect("resource_of returned a live id")
                            .refs
                            .insert(dst);
                        // ★★★ §12.46 — **A `Dup` CREATES A LIVE HANDLE, so it unblocks
                        // the parked tables exactly as an `Alloc` does.** These three
                        // drains used to live only in the `Alloc` arm, on the reading
                        // that a parked fact waits for its target's *allocation*. It
                        // waits for its target's **handle**, and `Alloc` and `Dup` are
                        // the only two events that create one — `Free`/`Unmap` only
                        // remove, `SetPageDir`/`MapMemoryDma` create none. A dup-of-a-dup
                        // whose middle handle is minted by this very `Dup` therefore
                        // stayed parked, and the graph then held a parked edge that was
                        // fully RESOLVABLE — two derivations of one fact that disagreed:
                        //
                        //  - `resource_of`/`origin_of` follow `pending_dups`, so
                        //    `crate::project` resolved the chain and GROUPED on the edge;
                        //  - `refs` — the refcount — never learned about the alias, so
                        //    `references_of` denied it existed.
                        //
                        // The refcount is the one that decides lifetime, and an undercount
                        // is the fatal direction: `free`'s last-reference test
                        // (`refs.is_empty() && map_refs == 0`) destroyed a resource a live
                        // foreign alias still referenced — "free host memory RM says is
                        // live", reached without a single malformed event. The parked
                        // sibling tables carry the same shape and are drained here for the
                        // same reason: a parked `SET_PAGE_DIRECTORY` whose VASpace handle
                        // a `Dup` mints left `Resource::pdb` unset (so no mapping learned
                        // the PDB), and a parked `MAP_MEMORY_DMA` never installed, which is
                        // a MISS=FAULT on a legal guest at first use.
                        //
                        // Order matters and matches the `Alloc` arm: dups first, because
                        // promoting one can be what makes a parked PDB or map resolve.
                        self.resolve_pending_dups();
                        self.resolve_pending_pdbs();
                        self.resolve_pending_maps();
                    }
                    // Source not observed yet (order tolerance). Park the edge so a
                    // later Alloc of `src` (or a chain) resolves it.
                    None => {
                        // A hostile guest can dup endpoints that never arrive; bound
                        // the parked table so the flood is loud, not an OOM (boundary-1).
                        if self.pending_dups.len() >= MAX_PARKED {
                            return Err(RmGraphError::CapacityExceeded(Capacity::PendingDups));
                        }
                        self.pending_dups.insert(dst, src);
                    }
                }
                Ok(())
            }
            RmEvent::SetPageDir {
                client,
                vaspace,
                pdb,
            } => {
                // Re-binding a VASpace to a new PDB is protocol-legal
                // (UNSET/SET_PAGE_DIRECTORY); last declaration wins. The PDB belongs to
                // the RESOURCE (survives the origin handle's free); a declaration on an
                // as-yet-unknown handle parks until that handle resolves.
                let target = NodeKey::new(client, vaspace);
                match self.resource_of(target) {
                    Some(id) => {
                        self.resources
                            .get_mut(&id)
                            .expect("resource_of returned a live id")
                            .pdb = Some(pdb);
                        // Any mapping already installed against this VAS learns the PDB
                        // now (a MAP_MEMORY_DMA that preceded SET_PAGE_DIRECTORY).
                        for (mk, mapping) in self.mappings.iter_mut() {
                            if mk.vaspace == id {
                                mapping.pdb = Some(pdb);
                            }
                        }
                    }
                    None => {
                        // Bound the parked page-dir table (orphan-SetPageDir flood).
                        //
                        // ★ §12.42 (N6) — the gate covers a genuinely NEW entry only.
                        // Re-binding a VASpace whose handle has not resolved yet is the
                        // same protocol-legal `UNSET`/`SET_PAGE_DIRECTORY` re-bind the
                        // resolved arm above allows unconditionally ("last declaration
                        // wins"), and it REPLACES an entry rather than adding one — so
                        // refusing it once the table is full would refuse a legal guest op
                        // that costs nothing, which is the hangs-a-legal-guest direction.
                        // The two sibling parked tables already have this shape: a
                        // re-parked `Dup` returns early as idempotent-or-`ConflictingDup`
                        // before reaching its gate, and `pending_maps` gates behind
                        // `!contains`. This one did not, and its last-wins was undeclared
                        // as well as ungated; both are stated here.
                        if !self.pending_pdbs.contains_key(&target)
                            && self.pending_pdbs.len() >= MAX_PARKED
                        {
                            return Err(RmGraphError::CapacityExceeded(Capacity::PendingPdbs));
                        }
                        self.pending_pdbs.insert(target, pdb);
                    }
                }
                Ok(())
            }
            RmEvent::MapMemoryDma {
                client,
                vaspace,
                memory,
                va,
                offset,
                len,
            } => self.apply_map(
                PendingMap {
                    client,
                    vaspace,
                    memory,
                    va,
                    offset,
                    len,
                },
                false,
            ),
            RmEvent::Unmap {
                client,
                vaspace,
                va,
            } => {
                let vas_key = NodeKey::new(client, vaspace);
                match self.resource_of(vas_key) {
                    Some(vas_id) => {
                        self.drop_mapping(MapKey {
                            vaspace: vas_id,
                            va,
                        });
                        Ok(())
                    }
                    // Unmap of a VAS we never saw: drop any parked map for it (idempotent
                    // teardown), never a loud error — teardown races the bind.
                    None => {
                        self.pending_maps.retain(|m| {
                            !(NodeKey::new(m.client, m.vaspace) == vas_key && m.va == va)
                        });
                        Ok(())
                    }
                }
            }
            RmEvent::Free { client, handle } => {
                let key = NodeKey::new(client, handle);
                // Known iff the handle references a live resource or names a parked dup.
                let known = self.handles.contains_key(&key) || self.pending_dups.contains_key(&key);
                if !known {
                    return Err(RmGraphError::FreeUnknown(key));
                }
                self.free_subtree(key);
                Ok(())
            }
        }
    }

    /// RM free semantics: drop `key`'s reference and those of its same-namespace
    /// descendants; for a client root, drop every handle in the namespace. A resource
    /// survives as long as any *other* client's handle still references it (a dup keeps
    /// it alive); it is removed only when its last reference goes — no leak, no
    /// premature destroy.
    fn free_subtree(&mut self, key: NodeKey) {
        let root_free = self.is_client_root(key);
        // The set of HANDLES (not resources) this free removes, all within one client
        // namespace: the target plus its transitive same-namespace children.
        let doomed: BTreeSet<NodeKey> = if root_free {
            self.handles
                .keys()
                .filter(|k| k.client == key.client)
                .copied()
                .collect()
        } else {
            let mut doomed = BTreeSet::from([key]);
            let mut changed = true;
            while changed {
                changed = false;
                for (&hkey, &href) in &self.handles {
                    // A handle's parent is same-namespace; read it from the resource's
                    // payload (only the ORIGIN handle carries a parent — a dup alias is
                    // a leaf reference with no children of its own).
                    if hkey.client != key.client || doomed.contains(&hkey) {
                        continue;
                    }
                    let Some(res) = self.resources.get(&href.res()) else {
                        continue;
                    };
                    // Only follow the parent edge from the handle DECLARED BY `RM_ALLOC`
                    // (§12.25). Asking `res.node.key != hkey` instead would treat a dup
                    // alias that re-occupies its own resource's freed origin handle value
                    // as an origin, and drag that resource's declared children into an
                    // alias's free — a parent edge the alias never declared.
                    if !href.is_origin() {
                        continue;
                    }
                    let parent_key = NodeKey::new(hkey.client, res.node.parent);
                    if parent_key != hkey && doomed.contains(&parent_key) {
                        doomed.insert(hkey);
                        changed = true;
                    }
                }
            }
            doomed
        };

        // Resource ids referenced by the doomed handles, captured BEFORE dropping so we
        // can tell afterwards which resources actually died (last reference gone) — a
        // VASpace resource that dies takes its mappings with it (releasing memory refs).
        let touched_ids: BTreeSet<ResId> = doomed
            .iter()
            .filter_map(|k| self.handles.get(k).map(|h| h.res()))
            .collect();

        for k in &doomed {
            self.drop_handle(*k);
        }

        // ★★★★ §16.28 — a **Device** losing its handle takes its default VA space with
        // it, and nothing else does. This mirrors RM exactly: `deviceRemoveFromClientShare_IMPL`
        // (`ogkm-580: src/nvidia/src/kernel/gpu/device_share.c:307-320`) is the only site
        // that destroys `pDevice->pVASpace`, and it runs on the Device's teardown.
        //
        // ⊘ The transient VASpace handle's own `Free` is deliberately NOT a removal here,
        // and that asymmetry is the whole point of the latch: RM frees that handle three
        // RPCs after minting it (`gpu_vaspace.c:4135`) while the address space it named
        // lives on. Keys are Device handles, so that free prunes nothing — it is not a key.
        //
        // ⚠ Keyed on the DOOMED HANDLE SET, so a client-root free (which dooms every
        // handle in the namespace) also clears every Device in it, with no second rule.
        for k in &doomed {
            self.device_default_vas.remove(k);
        }

        // Any mapping whose VASpace resource no longer exists is gone: drop it (which
        // releases its memory resource's map-ref, possibly freeing the memory too).
        let dead_vas: Vec<MapKey> = self
            .mappings
            .keys()
            .filter(|mk| {
                touched_ids.contains(&mk.vaspace) && !self.resources.contains_key(&mk.vaspace)
            })
            .copied()
            .collect();
        for mk in dead_vas {
            self.drop_mapping(mk);
        }

        // A parked (unresolved) dup whose dst OR src handle was just freed is stale.
        self.pending_dups
            .retain(|dst, src| !doomed.contains(dst) && !doomed.contains(src));
        // Likewise a parked PDB declared on a now-freed handle.
        self.pending_pdbs
            .retain(|target, _| !doomed.contains(target));
        // And a parked map naming a now-freed VASpace or memory handle.
        self.pending_maps.retain(|m| {
            !doomed.contains(&NodeKey::new(m.client, m.vaspace))
                && !doomed.contains(&NodeKey::new(m.client, m.memory))
        });

        // ★★★ §12.39 Part A — **TEARDOWN COMPLETION: freeing a client root destroys the
        // namespace's PARKED facts as well as its handles.**
        //
        // The three retains above prune by membership in `doomed`, which is the set of
        // LIVE HANDLES the free removed. A parked fact's key is by definition *not* a live
        // handle — that is what "parked" means — so **a parked fact survived the free of
        // its own namespace's client root**, and stayed in the table waiting for a handle
        // that could only ever be created by a *later* declaration of the same `hClient`.
        //
        // That is §12.39's Shape A, and it is the cheapest cross-process isolation break
        // in the model: four events, no host resources, nothing observable until it fires.
        // An attacker declares a namespace it will hand back (`hRoot` is caller-supplied
        // in the shipped Linux driver — `serverAllocClient`: `hClient = pParams->hClient`,
        // `ogkm src/nvidia/src/libraries/resserv/src/rs_server.c:612`, with the reject
        // guard compiled out under `RS_COMPATABILITY_MODE=1`, `ogkm src/nvidia/Makefile:129`),
        // parks a `DUP_OBJECT` in it against a source it has not allocated yet, frees the
        // root, and waits. When an unrelated later process is handed that `hClient` value
        // — sequential and guessable, and recycled by RM's own generator, which wraps at
        // 2^20 with no free list and no epoch (`rs_server.c:3319-3341`) — the attacker
        // allocates the source, `resolve_pending_dups` promotes the edge, and a live ALIAS
        // appears **inside the victim's namespace**. Both ends are declared `User`, both
        // alive, so it is a *grouping* edge: one `Proc`, one isolate, one GPA arena, one
        // host VAS. #14 un-fixed for a pair the attacker chose.
        //
        // **Faithful, not convenient.** RM destroys the whole namespace on a root free,
        // children before parents (`ogkm .../resserv/src/rs_client.c:830-849`), and every
        // subsequent op naming it is `NV_ERR_INVALID_CLIENT` (`rs_server.c:1674` →
        // `rmapi/client.c:782`). A parked fact is one we accepted but could not yet
        // resolve; promoting it after its namespace died would create an object in a
        // namespace RM says does not exist. **Nothing legal is lost** — the promoted alias
        // could never be legally *used*.
        //
        // Both ENDS of a parked dup are purged, not just the `dst`. A parked edge whose
        // **source** is in the dead namespace is the same vector with the roles swapped:
        // the alias lands in the attacker's own namespace and the resource it aliases is
        // one the *victim* allocates after re-declaring, which merges just as hard.
        //
        // ★ This is also what makes §12.39's §3 invariant total: after a root free the
        // graph holds no reference of any kind to that namespace except the recorded
        // owner of resources a foreign alias keeps alive — which is Part B's business.
        if root_free {
            let ns = key.client;
            self.pending_dups
                .retain(|dst, src| dst.client != ns && src.client != ns);
            self.pending_pdbs.retain(|target, _| target.client != ns);
            self.pending_maps.retain(|m| m.client != ns);
        }
    }

    /// Promote every parked dup whose source is now a live resource into a real
    /// reference (so the resource is refcounted by it). Fixpoint to resolve chains
    /// (a dup of a dup whose middle just resolved). A parked dst that would collide
    /// with an existing handle is left parked — [`Self::apply`]'s conflict check owns
    /// that decision; here we only ever add references for free dst keys.
    fn resolve_pending_dups(&mut self) {
        loop {
            let ready: Option<(NodeKey, ResId)> =
                self.pending_dups.iter().find_map(|(dst, src)| {
                    (!self.handles.contains_key(dst))
                        .then(|| self.resource_of(*src).map(|id| (*dst, id)))
                        .flatten()
                });
            let Some((dst, id)) = ready else { break };
            self.pending_dups.remove(&dst);
            // A promoted parked edge is still a `DUP_OBJECT` declaration → an ALIAS.
            self.handles.insert(dst, HandleRef::Alias(id));
            self.resources
                .get_mut(&id)
                .expect("resource_of returned a live id")
                .refs
                .insert(dst);
        }
    }

    /// Attach every parked `SET_PAGE_DIRECTORY` whose target handle now resolves onto
    /// its resource. Order tolerance for a PDB declared before its VASpace's alloc.
    ///
    /// ★ A parked declaration only fills a resource whose PDB is **not already set**
    /// (decision #18C — the parked-PDB wedge fix). A `SET_PAGE_DIRECTORY` that arrives
    /// *directly* on a live VASpace is authoritative and always wins (the handler
    /// overwrites); a *parked* one is by construction older (declared before its handle
    /// existed), so it must never overwrite a PDB a later direct declaration already
    /// set. Without this guard a hostile guest could park a `SET_PAGE_DIRECTORY` on a
    /// handle it *later* dup-aliases onto a different live VASpace: draining the stale
    /// PDB would overwrite that VASpace's real PDB and forge a projection `PdbCollision`
    /// — and because the drain fires inside `RmGraph::apply` while the surrounding
    /// `Gpu::apply` rolls back (restoring the parked fact), the collision re-fires on
    /// EVERY subsequent `Alloc`, wedging the device's control plane for every other
    /// process (a global DoS, not a contained refusal — the #18A violation this closes).
    /// Draining without overwriting is also what makes PDB resolution order-independent:
    /// a direct declaration wins over a parked one regardless of arrival order.
    fn resolve_pending_pdbs(&mut self) {
        let ready: Vec<(NodeKey, ResId, Pdb)> = self
            .pending_pdbs
            .iter()
            .filter_map(|(target, pdb)| self.resource_of(*target).map(|id| (*target, id, *pdb)))
            .collect();
        for (target, id, pdb) in ready {
            // Drain the parked fact unconditionally (so it never lingers to re-fire),
            // but only APPLY it to a resource that has no PDB yet — never overwrite a
            // PDB a direct declaration already set.
            self.pending_pdbs.remove(&target);
            if let Some(res) = self.resources.get_mut(&id)
                && res.pdb.is_none()
            {
                res.pdb = Some(pdb);
            }
        }
    }

    /// Apply (or park) one `MAP_MEMORY_DMA`. Resolves the VASpace and memory handles
    /// to their resources; if both are known, installs a [`Mapping`] and takes a
    /// reference on the memory resource. If either handle is unobserved, parks the map
    /// for order-tolerant replay. An identical re-send is idempotent; a conflicting
    /// map at the same `(vaspace, va)` is a loud [`RmGraphError::ConflictingMap`].
    ///
    /// `replay` is `true` when this is a parked map being retried by
    /// [`Self::resolve_pending_maps`] (as opposed to a guest's direct
    /// `MAP_MEMORY_DMA`). It gates the parked-map wedge guard below (decision #18C).
    fn apply_map(&mut self, m: PendingMap, replay: bool) -> Result<(), RmGraphError> {
        let vas_key = NodeKey::new(m.client, m.vaspace);
        let mem_key = NodeKey::new(m.client, m.memory);
        let (Some(vas_id), Some(_mem_id)) = (self.resource_of(vas_key), self.resource_of(mem_key))
        else {
            // Park (the set dedups identical parked entries so replay is idempotent).
            if !self.pending_maps.contains(&m) {
                // Bound the parked-map table (orphan-MapMemoryDma flood, boundary-1).
                if self.pending_maps.len() >= MAX_PARKED {
                    return Err(RmGraphError::CapacityExceeded(Capacity::PendingMaps));
                }
                self.pending_maps.insert(m);
            }
            return Ok(());
        };
        let vas_origin = self.origin_of(vas_key).expect("resource_of => live").id();
        let mem_node = *self.origin_of(mem_key).expect("resource_of => live");
        let key = MapKey {
            vaspace: vas_id,
            va: m.va,
        };
        let mapping = Mapping {
            vaspace: vas_origin,
            pdb: self.resource_pdb(vas_id),
            memory: mem_node.id(),
            va: m.va,
            offset: m.offset,
            len: m.len,
            // ★ Typed-resolve the backing through [`Self::backing_of`] (which requires
            // an `ObjectKind::Memory`), NOT by reading `facts.mem_phys` off whatever
            // object the `memory` handle happens to name (decision #18C). Without this,
            // a hostile guest could name a NON-memory object (e.g. a VASpace) carrying
            // an attacker-set `mem_phys` in its alloc facts and have it silently accepted
            // as a mapping's backing — the "two memory→phys resolvers disagree"
            // confused-deputy (`backing_of` type-checks; this site used not to). Now
            // both use the ONE typed path: a non-Memory `memory` resolves to no backing
            // → a loud `UnbackedMapping` fault at sync, never a silent guess.
            mem_phys: self.backing_of(mem_key).map(|base| base + m.offset),
        };
        // ★ Parked-map wedge guard (decision #18C — the parked-PDB wedge's twin). A
        // *replayed* parked map that resolves to an UNBACKED backing can never populate
        // (backing is alloc-time; an unbacked memory stays unbacked), so installing it
        // would make `Gpu::sync_rpc_mappings` fault (`UnbackedMapping`) on whatever
        // UNRELATED apply happened to trigger the replay — and because the parked fact
        // survives that apply's rollback, the fault re-fires on every subsequent
        // `Alloc`, wedging the control plane for every other process (a global DoS). So
        // a replayed unbacked map is dropped (the caller already removed it from
        // `pending_maps`): the VA simply never forward-populates → a loud MISS=FAULT at
        // USE, contained to the toucher. A DIRECT unbacked map (`replay == false`) still
        // installs, so the guest's own map op earns its loud `UnbackedMapping` refusal.
        if replay && mapping.mem_phys.is_none() {
            return Ok(());
        }
        match self.mappings.get(&key) {
            Some(existing) if *existing == mapping => Ok(()), // idempotent retry
            Some(_) => Err(RmGraphError::ConflictingMap {
                vaspace: vas_origin,
                va: m.va,
            }),
            None => {
                // A new live mapping grows the mapping table (boundary-1).
                if self.mappings.len() >= MAX_LIVE_MAPPINGS {
                    return Err(RmGraphError::CapacityExceeded(Capacity::Mappings));
                }
                self.mappings.insert(key, mapping);
                if let Some(mem_id) = self.resource_of(mem_key)
                    && let Some(res) = self.resources.get_mut(&mem_id)
                {
                    res.map_refs += 1;
                    self.map_mem_res.insert(key, mem_id);
                }
                Ok(())
            }
        }
    }

    /// Replay every parked map whose endpoints now resolve. Fixpoint (a parked map may
    /// depend on an alloc that also unparked a dup — re-scan until quiescent).
    fn resolve_pending_maps(&mut self) {
        while let Some(m) = self
            .pending_maps
            .iter()
            .find(|m| {
                self.resource_of(NodeKey::new(m.client, m.vaspace))
                    .is_some()
                    && self.resource_of(NodeKey::new(m.client, m.memory)).is_some()
            })
            .copied()
        {
            self.pending_maps.remove(&m);
            // A conflict at replay is dropped silently here (the loud path is the
            // direct `apply`); replay only ever installs cleanly-resolvable maps, and an
            // unbacked replay is dropped by the wedge guard (`replay = true`).
            let _ = self.apply_map(m, true);
        }
    }

    /// Drop the mapping at `key`, releasing its reference to the memory resource
    /// (removing the memory resource iff that was its last reference — unmap eager).
    fn drop_mapping(&mut self, key: MapKey) {
        if self.mappings.remove(&key).is_none() {
            return;
        }
        // Release the memory resource's mapping-reference. Resolved by the stored
        // ResId (not a handle lookup) so it works even after all handles were freed.
        if let Some(mem_id) = self.map_mem_res.remove(&key)
            && let Some(res) = self.resources.get_mut(&mem_id)
        {
            res.map_refs = res.map_refs.saturating_sub(1);
            if res.refs.is_empty() && res.map_refs == 0 {
                // ★★★ §12.41 — the identity index dies with the resource, and it must:
                // the ordinal it held is what a later `Alloc` at that handle value is
                // then free to take. ★★★ §12.42 — and so does its declaration's refcount.
                self.destroy_resource(mem_id);
            }
        }
    }

    /// The declared PDB attached to a resource id, if any (used at map time).
    fn resource_pdb(&self, id: ResId) -> Option<Pdb> {
        self.resources.get(&id).and_then(|r| r.pdb)
    }

    /// Is `key` the client root of its OWN namespace? True only when `key` was
    /// **declared by `RM_ALLOC`** ([`HandleRef::Origin`] — the recorded fact, §12.25) AND
    /// the resource it allocated is a [`ObjectKind::Client`]. A dup *alias* that
    /// references a Client resource is NOT a root — freeing it drops only that alias's
    /// reference, never the aliasing client's whole namespace.
    ///
    /// The predicate used to ask `n.key == key`, which means the same thing ONLY until a
    /// `Dup` re-creates a freed origin's key; from that moment an alias was
    /// indistinguishable from the allocation and freeing it tore down an entire live
    /// namespace. Handle values are reusable, so identity can never be the handle alone.
    fn is_client_root(&self, key: NodeKey) -> bool {
        self.handles.get(&key).is_some_and(|h| {
            h.is_origin()
                && self
                    .resources
                    .get(&h.res())
                    .is_some_and(|r| matches!(r.node.kind, ObjectKind::Client))
        })
    }

    /// Drop ONE handle's reference to its resource; remove the resource (and its
    /// PDB, which lives ON the resource) iff that was its last reference. Freeing the
    /// origin handle while a dup still references the resource keeps BOTH the resource
    /// and its declared PDB alive.
    fn drop_handle(&mut self, key: NodeKey) {
        // ★ §12.27 — the client-root index tracks the root HANDLE, not the resource
        // (§12.25's lesson, one level up). A client root that a `DUP_OBJECT` alias keeps
        // alive in *another* namespace no longer occupies its OWN namespace: that
        // namespace is empty and is free to declare a new root, with a freshly declared
        // `ClientKind`. Pruning on the resource's death instead would leave the
        // namespace permanently un-declarable — and, once it had declared a new root,
        // would prune the NEW entry when the old alias finally died.
        if self.is_client_root(key) {
            self.client_roots.remove(&key.client);
        }
        let Some(id) = self.handles.remove(&key).map(|h| h.res()) else {
            return;
        };
        if let Some(res) = self.resources.get_mut(&id) {
            res.refs.remove(&key);
            // A resource stays alive while any handle OR any live mapping references it
            // (a mapped memory object survives its source handle's free — faithful RM
            // refcounting). Destroyed only when the LAST reference of any kind goes.
            if res.refs.is_empty() && res.map_refs == 0 {
                // ★★★ §12.41/§12.42 — see the twin site in `unmap`: resource, identity
                // index and declaration refcount go together, always.
                self.destroy_resource(id);
            }
        }
    }

    /// Resolve a handle to its resource id through dup aliasing, following a parked
    /// (not-yet-alloc'd) chain if needed. `None` if the chain dangles.
    ///
    /// **MISS ⇒ DEFER**: a dangling chain is the `Dup`-before-`Alloc` ordering the
    /// protocol allows — the source alloc may still arrive and
    /// [`Self::resolve_pending_dups`] promotes the edge when it does. A chain that never
    /// resolves stays inert and its *use* faults loudly, which is where MISS=FAULT bites
    /// (crate docs: the category is a property of the site, not of the absence).
    fn resource_of(&self, key: NodeKey) -> Option<ResId> {
        let mut k = key;
        // Bounded: dup chains are tiny; guard against a (protocol-invalid) cycle.
        for _ in 0..64 {
            if let Some(href) = self.handles.get(&k) {
                return Some(href.res());
            }
            k = *self.pending_dups.get(&k)?;
        }
        None
    }

    /// Resolve `key` through dup aliasing to its **origin** node (the resource's
    /// payload). `None` if the chain dangles (fact not yet observed) or the resource
    /// was never allocated / has been fully freed.
    ///
    /// **MISS ⇒ DEFER**, as [`Self::resource_of`].
    #[must_use]
    pub fn origin_of(&self, key: NodeKey) -> Option<&RmNode> {
        let id = self.resource_of(key)?;
        self.resources.get(&id).map(|r| &r.node)
    }

    /// ★ THE ONE typed-resolution primitive (decision #18C — the confused-deputy
    /// collapse). Resolve `key` (dup-aliases followed) to its origin node **only if**
    /// that origin's kind matches `want`, comparing by *discriminant* so a payload
    /// variant (`Channel { engine }` / `EngineObject { engine }`) matches any engine.
    ///
    /// Every site that turns a guest-supplied handle into a *typed* object — a
    /// channel's `hVASpace` → a VASpace, a `MAP_MEMORY_DMA`'s `memory` → a Memory, a
    /// CtxShare/TSG hop — routes through here, so "resolved a handle to the WRONG
    /// `ObjectKind`" is a **single centrally-enforced check**, not an ad-hoc
    /// `matches!` a new call site could forget (the exact confused-deputy the C's
    /// heuristic resolvers were, and that M2's `resolve_vaspace_handle` fixed
    /// piecemeal). A hostile guest naming an object of the wrong type gets a loud MISS
    /// at use time (`None` here), never a silent bind to an unrelated object.
    ///
    /// ## ★ The one `None` that carries TWO categories — a declared, deliberate fusion
    ///
    /// Per the crate docs' miss taxonomy this site is genuinely both:
    ///
    /// - `key` does not resolve at all ⇒ **DEFER** — the alloc may still arrive.
    /// - `key` resolves, to an object of the **wrong kind** ⇒ **never knowable**. The
    ///   resource exists and is definitively not a `want`; no future event turns a TSG
    ///   into a VASpace. By the taxonomy that is a **FAULT** shape, and the only reason
    ///   it is not raised *here* is that this is a resolver, not a use site: it is a
    ///   hostile-input fact — a guest naming a TSG as its `hVASpace` — reported as an
    ///   absence.
    ///
    /// **The end state is safe either way** (both make the object unroutable, and its use
    /// faults loudly and specifically), so nothing downstream is wrong. What is lost is
    /// the *distinction*: a type-confusion attempt is indistinguishable here from a fact
    /// that has not arrived, and only one of the two is a security-relevant event. That is
    /// the same shape §12.13 decided was worth splitting for condemned-vs-unknown routing
    /// misses. Splitting it means a `Result`-shaped resolver threaded through `project`,
    /// which is a design change and not a documentation pass — recorded as an open finding
    /// (`l1_concurrency.md` §12.30, finding B) rather than done silently.
    #[must_use]
    pub fn origin_of_kind(&self, key: NodeKey, want: ObjectKind) -> Option<&RmNode> {
        let node = self.origin_of(key)?;
        (core::mem::discriminant(&node.kind) == core::mem::discriminant(&want)).then_some(node)
    }

    /// The live handles that reference the resource whose **origin** is `origin_key` —
    /// the origin alloc plus every surviving `DUP_OBJECT` alias. This IS the refcount
    /// set: the resource is alive ⟺ this is non-empty, and it is destroyed the instant
    /// its last reference is freed. Keyed by the resource's stable origin (as reported
    /// by [`Self::nodes`]), so it stays correct even after the origin *handle* itself
    /// has been freed while a dup keeps the resource alive. Empty if no such resource.
    ///
    /// ★★ **NEWEST WINS, and that is a correctness rule** (`l1_concurrency.md` §12.39,
    /// finding 4 — shared by [`Self::pdb_of`], [`Self::backing_of`], [`Self::map_ref_count`]
    /// and [`Self::gpu_of`]'s fallback, which is why it is stated once here).
    ///
    /// An origin key is **not unique among live resources**: `Alloc (A,H) → Dup to B →
    /// Free (A,H) → Alloc (A,H)` leaves two live resources reporting at the same
    /// `(client, handle)` — the orphan a foreign alias keeps alive, and the current
    /// allocation. Handle values are reusable by design (that is why [`ResId`] exists at
    /// all), and guest handle values are *deterministic* in practice — every CUDA process
    /// presents the same ones, which is the whole #14 shape — so this is the ordinary case
    /// on a namespace that has churned, not an exotic one.
    ///
    /// A forward `find` answered with the **oldest**, i.e. the orphan. That disagreed with
    /// `crate::project`, whose `vases`/`channels` maps are keyed on `node.key` and are
    /// filled in ascending [`ResId`] order, so the *newest* wins there — and the
    /// disagreement is guest-reachable and load-bearing: the current allocation's `Vas`
    /// took the **orphan's** declared `Pdb`, so `by_pdb` filed a page-directory base the
    /// current tenant never declared onto the current tenant's `Proc`, and whoever holds
    /// an alias to the orphan can route to it. `rev()` makes the answer the current
    /// declaration's, which is the one that projects.
    ///
    /// Note the scope: this removes the *ambiguity*, it does not move the projection off
    /// `NodeKey` keys. That refactor (§12.25's deferred finding 2) is still open.
    pub fn references(&self, origin_key: NodeKey) -> impl Iterator<Item = NodeKey> + '_ {
        let refs = self.current_at(origin_key).map(|r| &r.refs);
        refs.into_iter().flat_map(|set| set.iter().copied())
    }

    /// ★★★ §12.41 — [`Self::references`] for a resource named by IDENTITY rather than by
    /// handle value: the ghost's own reference set, which the handle-shaped form cannot
    /// reach once the current tenant occupies its key.
    pub fn references_of(&self, id: ResourceKey) -> impl Iterator<Item = NodeKey> + '_ {
        let refs = self.resource_at(id).map(|r| &r.refs);
        refs.into_iter().flat_map(|set| set.iter().copied())
    }

    /// Declared PDB of the VASpace whose **origin** is `origin_key`, if declared.
    ///
    /// **MISS ⇒ DEFER**: `SET_PAGE_DIRECTORY` legally arrives after the VASpace alloc (and
    /// may be parked before it), so an undeclared PDB is *not yet knowable*. The
    /// consequence is that the VAS enters no routing map — and an address op against it
    /// then takes a loud `FwdFault::UnknownPdb` at use.
    ///
    /// The
    /// PDB is a property of the resource (declared via the origin OR any alias handle,
    /// possibly before the alloc), so it resolves regardless of which handle carried
    /// the `SET_PAGE_DIRECTORY` and survives the origin handle's free.
    #[must_use]
    pub fn pdb_of(&self, origin_key: NodeKey) -> Option<Pdb> {
        match self.current_at(origin_key).map(|r| r.node.id()) {
            Some(id) => self.pdb_of_resource(id),
            // No resource at that key at all — a parked `SetPageDir` may still name it.
            None => self.parked_pdb(|n| n.key == origin_key),
        }
    }

    /// ★★★ §12.41 — [`Self::pdb_of`] for a resource named by IDENTITY. **This is the one
    /// the projection uses**, and it has to be: the handle-shaped form answers about the
    /// current tenant of a recycled handle value, so asking it about a dup-kept ghost
    /// returned the *successor's* page-directory base — the ghost's `Vas` was filed under
    /// a PDB it never declared, and the ghost's own PDB was claimed by nobody (which also
    /// silently disarmed the F1 [`crate::project::ProjectionError::PdbCollision`] guard
    /// for that value).
    ///
    /// ★★★ **The live resource NODE named by identity** — its origin handle (hence its
    /// owning `hClient`), its class and its [`AllocFacts`].
    ///
    /// # Why by IDENTITY and not by handle, restated because it is the same trap
    ///
    /// Exactly [`Self::pdb_of_resource`]'s argument: a handle-shaped lookup answers about
    /// the *current tenant* of a recycled handle value, so a dup-kept ghost would report
    /// its successor's facts. Every caller here holds a [`ResourceKey`] already
    /// (`kayfabe_core::gpu::Channel::key`), so nothing is gained by widening it.
    ///
    /// ⊘ This exposes no fact the projection does not already derive; it exists so a
    /// consumer that needs **two** declared facts of one channel — the `hClient` its
    /// namespace is in and the `hVASpace` it named — can read them from the one node that
    /// declared both, rather than joining two projections that could disagree.
    #[must_use]
    pub fn node_of_resource(&self, id: ResourceKey) -> Option<&RmNode> {
        self.resource_at(id).map(|r| &r.node)
    }

    /// **MISS ⇒ DEFER**, exactly as [`Self::pdb_of`].
    #[must_use]
    pub fn pdb_of_resource(&self, id: ResourceKey) -> Option<Pdb> {
        if let Some(pdb) = self.resource_at(id).and_then(|r| r.pdb) {
            return Some(pdb);
        }
        // A SetPageDir may still be parked (declared before the target's alloc, via
        // the origin handle or any alias that resolves to this resource).
        self.parked_pdb(|n| n.id() == id)
    }

    /// The parked-`SET_PAGE_DIRECTORY` half of [`Self::pdb_of_resource`]: a declaration
    /// whose target handle resolves to a node satisfying `want`.
    fn parked_pdb(&self, want: impl Fn(&RmNode) -> bool) -> Option<Pdb> {
        self.pending_pdbs
            .iter()
            .find_map(|(k, p)| self.origin_of(*k).filter(|n| want(n)).map(|_| *p))
    }

    /// ★★★★ **§16.28 — the handle naming a Device's DEFAULT VA space**, if the guest has
    /// declared one under `device` ([`VaSpaceRole::DeviceDefault`]).
    ///
    /// **MISS ⇒ DEFER**, exactly as [`Self::pdb_of`]: a Device legitimately exists before
    /// any device-default VA space is named under it (RM names one lazily, the first time
    /// something needs the GSP side to have a handle for it), so a `None` here is *"not
    /// yet declared"* and never *"this Device has no address space"*.
    ///
    /// ⚠ The handle it returns is **very likely dead** — that is the normal case, not a
    /// degenerate one. RM frees it immediately after publishing the VA space's
    /// page-directory root, so this is a *name*, and it is useful precisely because the
    /// publication is filed under that name (`kayfabe_device::gvaspub` keys on
    /// `(hClient, hObject)` and never prunes). ⊘ Do not resolve it back through the handle
    /// table and read a `None` as this answer being wrong: see [`VaSpaceRole`].
    #[must_use]
    pub fn device_default_vas(&self, device: NodeKey) -> Option<HObject> {
        self.device_default_vas.get(&device).copied()
    }

    /// All live resource payloads, ascending origin-key order (deterministic).
    pub fn nodes(&self) -> impl Iterator<Item = &RmNode> {
        self.resources.values().map(|r| &r.node)
    }

    /// All live DMA mappings, ascending `(vaspace, va)` order (deterministic). The
    /// runtime consumes these to forward-populate the address table (RPC populate
    /// source, co-equal with CE-PT-write capture — `mode2_address_table.md`).
    pub fn mappings(&self) -> impl Iterator<Item = &Mapping> {
        self.mappings.values()
    }

    /// The declared physical/backing base of the MEMORY resource whose **origin** is
    /// `memory_key` (dup-aliases resolved), if declared. This is how a `MapMemoryDma`
    /// resolves `memory → phys` for the address table. `None` if the handle is not a
    /// memory resource, is unobserved, or declared no backing.
    ///
    /// **Two categories under one `None`, and the CALLER separates them.** "Unobserved"
    /// is DEFER; "resolved but not a Memory" and "declared no backing" are *never
    /// knowable* (a backing is an alloc-time fact — an unbacked memory stays unbacked),
    /// and `Gpu::sync_rpc_mappings` duly raises `GpuError::UnbackedMapping` — a FAULT — for
    /// a mapping whose memory resolved. The deferring half is handled one level up, at the
    /// parked-map guard in `apply_map` (a *replayed* parked map with no backing is dropped
    /// rather than installed, so it cannot wedge an unrelated apply). See
    /// [`Self::origin_of_kind`] for the same fusion at the typed-resolution primitive.
    #[must_use]
    pub fn backing_of(&self, memory_key: NodeKey) -> Option<u64> {
        // Resolve by handle when possible; else fall back to the resource's stable
        // origin key (a memory kept alive ONLY by a live mapping has no live handle).
        let node = self
            .origin_of(memory_key)
            .or_else(|| self.current_at(memory_key).map(|r| &r.node))?;
        matches!(node.kind, ObjectKind::Memory)
            .then(|| node.facts.mem_phys)
            .flatten()
    }

    /// Number of live mappings referencing the resource whose origin is `origin_key`
    /// (the map-refcount — a memory object is kept alive by these as well as by its
    /// handles). Zero if no such resource.
    #[must_use]
    pub fn map_ref_count(&self, origin_key: NodeKey) -> usize {
        self.current_at(origin_key).map_or(0, |r| r.map_refs)
    }

    /// ★★★ §12.41 — [`Self::map_ref_count`] for a resource named by IDENTITY, which is
    /// what a [`Mapping`]'s own `memory` field now is.
    #[must_use]
    pub fn map_ref_count_of(&self, id: ResourceKey) -> usize {
        self.resource_at(id).map_or(0, |r| r.map_refs)
    }

    /// All live EVENT (os-event / notifier) nodes owned by `client` — completion
    /// routing is graph-derived from these, not from an opaque id
    /// (`execution_plane.md` §1). Ascending origin-key order (deterministic).
    pub fn events_of(&self, client: HClient) -> impl Iterator<Item = &RmNode> {
        self.resources
            .values()
            .map(|r| &r.node)
            .filter(move |n| n.key.client == client && matches!(n.kind, ObjectKind::Event))
    }

    /// All dup edges as `(dst, src)` where `dst` is a non-origin handle and `src` is
    /// the resource's origin handle — ascending dst order (deterministic). Includes
    /// both resolved references and parked (not-yet-resolved) edges.
    pub fn dups(&self) -> impl Iterator<Item = (NodeKey, NodeKey)> {
        let resolved = self.resources.values().flat_map(|r| {
            let origin = r.node.key;
            r.refs
                .iter()
                .filter(move |h| **h != origin)
                .map(move |h| (*h, origin))
        });
        let parked = self.pending_dups.iter().map(|(d, s)| (*d, *s));
        // Deterministic order: collect into a BTreeMap by dst, then iterate.
        let ordered: BTreeMap<NodeKey, NodeKey> = resolved.chain(parked).collect();
        ordered.into_iter()
    }

    /// Look up a resource's payload by exact handle (origin or alias; no alias
    /// resolution beyond the one-hop handle-table lookup).
    #[must_use]
    pub fn node(&self, key: NodeKey) -> Option<&RmNode> {
        let id = self.handles.get(&key)?.res();
        self.resources.get(&id).map(|r| &r.node)
    }

    /// ★ The Device→[`GpuId`] derivation (`multi_gpu_and_mig.md` item 1): resolve
    /// `key` (dup-aliases followed) to its owning GPU **target** by walking its
    /// declared parent chain to the nearest `Device` ancestor and reading that
    /// Device's declared `device_instance`.
    ///
    /// Pure and order-independent — a projection of declared facts, like every other
    /// derivation. `None` (MISS, never a default-GPU0 guess) when: the key does not
    /// resolve, no `Device` ancestor exists on the chain, or the Device declared no
    /// instance (G9, §12.21 — **not** a silent `GpuId::ZERO`).
    ///
    /// **MISS ⇒ DEFER** (crate docs): callers treat `None` as "not routable (yet)" —
    /// routing and materialization defer until the `Device` fact arrives, and *use* faults
    /// loudly. The third case above is arguably never-knowable (an alloc-time fact that
    /// did not arrive can no longer arrive for that Device), but it is unreachable in
    /// practice — `deviceId` is a required field of `NV0080_ALLOC_PARAMETERS`, so the ABI
    /// layer always observes it — and deferring it costs nothing: the object routes
    /// nowhere and its use is refused by name.
    #[must_use]
    pub fn gpu_of(&self, key: NodeKey) -> Option<GpuId> {
        // The sticky per-resource cache first (survives the source Device's free — a
        // dup-kept VAS keeps its target).
        //
        // ★ CORRECTED (`l1_concurrency.md` §12.30, finding A). This comment used to read
        // "a Device that declared no `deviceInstance` is the single-GPU default selection
        // … routing to `GpuId::ZERO`". That is the PRE-G9 behaviour and has been false
        // since §12.21: `walk_gpu` returns `node.facts.device_instance.map(GpuId)`, so an
        // instance-less Device yields `None` — no default-GPU0 guess anywhere on this
        // path. The doc on the public resolver stating the opposite of the code, inside
        // the one doctrine ("never guess GPU 0") the change existed to enforce, is exactly
        // the drift this audit pass looks for.
        if let Some(id) = self.resource_of(key)
            && let Some(g) = self.resources.get(&id).and_then(|r| r.gpu)
        {
            return Some(g);
        }
        // A live parent walk (uncached objects whose chain is intact).
        if let Some(g) = self.walk_gpu(key) {
            return Some(g);
        }
        // The origin handle was freed but a dup keeps the resource alive: read the
        // cached target off the resource **currently declared** at that origin key
        // (mirrors `pdb_of` — the target, like the PDB, is a resource property that
        // survives the free).
        self.current_at(key).and_then(|r| r.gpu)
    }

    /// ★★★ §12.41 — [`Self::gpu_of`] for a resource named by IDENTITY, which is what the
    /// projection and the RPC-mapping sync must use. The handle-shaped form resolves
    /// through the live handle table, so on a recycled handle value it answers about the
    /// successor; a dup-kept ghost would be routed to the successor's target.
    ///
    /// **MISS ⇒ DEFER**, exactly as [`Self::gpu_of`]: the sticky per-resource target
    /// cache is the whole answer here, because a ghost's parent chain is gone by
    /// construction (its `Device` may have been freed with the rest of its namespace).
    #[must_use]
    pub fn gpu_of_resource(&self, id: ResourceKey) -> Option<GpuId> {
        let res = self.resource_at(id)?;
        res.gpu.or_else(|| self.walk_gpu(res.node.key))
    }
}
