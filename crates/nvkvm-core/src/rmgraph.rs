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

use nvkvm_arch::ids::{ClassId, GpuId, GpuVa, HClient, HObject, Pdb};
use nvkvm_arch::{Arch, ObjectKind};

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
    /// The VASpace resource's origin key (stable identity).
    pub vaspace: NodeKey,
    /// The declared PDB of that VASpace at map time, if known (the address-table key).
    pub pdb: Option<Pdb>,
    /// The mapped memory resource's origin key.
    pub memory: NodeKey,
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
    /// Identity: the resource's ORIGIN handle (its first allocator). Stable across
    /// dups and across the origin client's free, as long as any reference survives.
    pub key: NodeKey,
    /// Declared parent handle (same namespace). For a client root, equals `key.handle`.
    pub parent: HObject,
    /// Raw class id as declared.
    pub class: ClassId,
    /// Arch-classified kind (recorded at apply; classification is stable per arch).
    pub kind: ObjectKind,
    /// Declared alloc facts.
    pub facts: AllocFacts,
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

/// Errors from [`RmGraph::apply`]. All loud; the caller decides whether a guest
/// protocol violation is fatal or logged-and-refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RmGraphError {
    /// A second, *different* alloc for an existing key (an identical re-send is
    /// accepted idempotently — retried-RPC tolerance, testing strategy `wo_retried_rpc`).
    ConflictingAlloc(NodeKey),
    /// A second, *different* dup for an existing dst key.
    ConflictingDup(NodeKey),
    /// A second, *different* `MAP_MEMORY_DMA` at a `(vaspace, va)` already mapped
    /// (an identical re-send is idempotent). Overlapping/replacing a live mapping
    /// without an eager unmap is a loud fault (the `ALREADY-MAPPED` collision class).
    ConflictingMap {
        /// The VASpace origin whose VA is doubly-mapped.
        vaspace: NodeKey,
        /// The colliding VA.
        va: GpuVa,
    },
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
#[derive(Debug, Clone, Default)]
pub struct RmGraph {
    /// Live resources, keyed by their stable origin id.
    resources: BTreeMap<ResId, Resource>,
    /// Every live handle → the resource it references. The origin handle and each
    /// dup alias both appear here; freeing a handle removes exactly its entry.
    handles: BTreeMap<NodeKey, ResId>,
    /// Dup edges whose source resource is not observed *yet* (order tolerance): a
    /// `Dup` may arrive before its `src`'s `Alloc`. Kept as `dst → src` and resolved
    /// lazily by [`Self::resource_of`]. When `src` becomes known and `dst` is used,
    /// the edge still resolves; freeing either end prunes it.
    pending_dups: BTreeMap<NodeKey, NodeKey>,
    /// `SET_PAGE_DIRECTORY` facts whose target handle is not observed *yet* (order
    /// tolerance): declaring-handle → declared PDB. Drained onto the resource as soon
    /// as its handle resolves (see [`Self::resolve_pending_pdbs`]).
    pending_pdbs: BTreeMap<NodeKey, Pdb>,
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
}

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

impl RmGraph {
    /// Empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
            if matches!(node.kind, nvkvm_arch::ObjectKind::Device) {
                return Some(GpuId(node.facts.device_instance.unwrap_or(0)));
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
        match ev {
            RmEvent::Alloc {
                client,
                parent,
                handle,
                class,
                facts,
            } => {
                let key = NodeKey::new(client, handle);
                let node = RmNode {
                    key,
                    parent,
                    class,
                    kind: arch.classify(class),
                    facts,
                };
                // A handle names EITHER an allocated resource OR a dup alias, never
                // both — so an alloc onto a handle already claimed by a (parked) dup is
                // a loud conflict, symmetric with the dup-onto-alloc rejection.
                if self.pending_dups.contains_key(&key) {
                    return Err(RmGraphError::ConflictingAlloc(key));
                }
                match self.handles.get(&key) {
                    // Idempotent retry: the same origin alloc re-sent.
                    Some(res) if self.resources.get(res).map(|r| r.node) == Some(node) => Ok(()),
                    // The key is already taken (by a different alloc, or by a resolved dup) — loud.
                    Some(_) => Err(RmGraphError::ConflictingAlloc(key)),
                    None => {
                        // Boundary-1: a genuinely new resource grows the handle table —
                        // refuse a flood loudly rather than OOM. (Idempotent re-sends
                        // take the branches above and never reach here.)
                        if self.handles.len() >= MAX_LIVE_HANDLES {
                            return Err(RmGraphError::CapacityExceeded(Capacity::Handles));
                        }
                        let id = ResId(self.next_res_id);
                        self.next_res_id += 1;
                        self.handles.insert(key, id);
                        self.resources.insert(
                            id,
                            Resource {
                                node,
                                refs: BTreeSet::from([key]),
                                pdb: None,
                                gpu: None,
                                map_refs: 0,
                            },
                        );
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
                        if matches!(arch.classify(class), nvkvm_arch::ObjectKind::Device) {
                            self.cache_targets();
                        }
                        Ok(())
                    }
                }
            }
            RmEvent::Dup { src, dst } => {
                // The dst handle must be FREE. It is already taken if it references a
                // live resource (`handles`) or names a parked (not-yet-resolved) edge
                // (`pending_dups`). Either way an identical re-send is idempotent and a
                // conflicting one is loud — dst must never end up doubly-bound.
                if let Some(existing) = self.handles.get(&dst) {
                    return if Some(*existing) == self.resource_of(src) {
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
                        self.handles.insert(dst, id);
                        self.resources
                            .get_mut(&id)
                            .expect("resource_of returned a live id")
                            .refs
                            .insert(dst);
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
                        if self.pending_pdbs.len() >= MAX_PARKED {
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
        // The set of HANDLES (not resources) this free removes, all within one client
        // namespace: the target plus its transitive same-namespace children.
        let doomed: BTreeSet<NodeKey> = if self.is_client_root(key) {
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
                for (&hkey, &id) in &self.handles {
                    // A handle's parent is same-namespace; read it from the resource's
                    // payload (only the ORIGIN handle carries a parent — a dup alias is
                    // a leaf reference with no children of its own).
                    if hkey.client != key.client || doomed.contains(&hkey) {
                        continue;
                    }
                    let Some(res) = self.resources.get(&id) else {
                        continue;
                    };
                    // Only follow the parent edge from the resource's origin handle.
                    if res.node.key != hkey {
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
            .filter_map(|k| self.handles.get(k).copied())
            .collect();

        for k in &doomed {
            self.drop_handle(*k);
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
            self.handles.insert(dst, id);
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
        let vas_origin = self.origin_of(vas_key).expect("resource_of => live").key;
        let mem_node = *self.origin_of(mem_key).expect("resource_of => live");
        let key = MapKey {
            vaspace: vas_id,
            va: m.va,
        };
        let mapping = Mapping {
            vaspace: vas_origin,
            pdb: self.resource_pdb(vas_id),
            memory: mem_node.key,
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
                self.resources.remove(&mem_id);
            }
        }
    }

    /// The declared PDB attached to a resource id, if any (used at map time).
    fn resource_pdb(&self, id: ResId) -> Option<Pdb> {
        self.resources.get(&id).and_then(|r| r.pdb)
    }

    /// Is `key` the client root of its OWN namespace? True only when `key` is a
    /// resource's origin handle AND that resource is a [`ObjectKind::Client`]. A dup
    /// *alias* that happens to reference a Client resource is NOT a root — freeing it
    /// drops only that alias's reference, never the aliasing client's whole namespace.
    fn is_client_root(&self, key: NodeKey) -> bool {
        self.node(key)
            .is_some_and(|n| n.key == key && matches!(n.kind, ObjectKind::Client))
    }

    /// Drop ONE handle's reference to its resource; remove the resource (and its
    /// PDB, which lives ON the resource) iff that was its last reference. Freeing the
    /// origin handle while a dup still references the resource keeps BOTH the resource
    /// and its declared PDB alive.
    fn drop_handle(&mut self, key: NodeKey) {
        let Some(id) = self.handles.remove(&key) else {
            return;
        };
        if let Some(res) = self.resources.get_mut(&id) {
            res.refs.remove(&key);
            // A resource stays alive while any handle OR any live mapping references it
            // (a mapped memory object survives its source handle's free — faithful RM
            // refcounting). Destroyed only when the LAST reference of any kind goes.
            if res.refs.is_empty() && res.map_refs == 0 {
                self.resources.remove(&id);
            }
        }
    }

    /// Resolve a handle to its resource id through dup aliasing, following a parked
    /// (not-yet-alloc'd) chain if needed. `None` if the chain dangles.
    fn resource_of(&self, key: NodeKey) -> Option<ResId> {
        let mut k = key;
        // Bounded: dup chains are tiny; guard against a (protocol-invalid) cycle.
        for _ in 0..64 {
            if let Some(id) = self.handles.get(&k) {
                return Some(*id);
            }
            k = *self.pending_dups.get(&k)?;
        }
        None
    }

    /// Resolve `key` through dup aliasing to its **origin** node (the resource's
    /// payload). `None` if the chain dangles (fact not yet observed) or the resource
    /// was never allocated / has been fully freed.
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
    pub fn references(&self, origin_key: NodeKey) -> impl Iterator<Item = NodeKey> + '_ {
        let refs = self
            .resources
            .values()
            .find(|r| r.node.key == origin_key)
            .map(|r| &r.refs);
        refs.into_iter().flat_map(|set| set.iter().copied())
    }

    /// Declared PDB of the VASpace whose **origin** is `origin_key`, if declared. The
    /// PDB is a property of the resource (declared via the origin OR any alias handle,
    /// possibly before the alloc), so it resolves regardless of which handle carried
    /// the `SET_PAGE_DIRECTORY` and survives the origin handle's free.
    #[must_use]
    pub fn pdb_of(&self, origin_key: NodeKey) -> Option<Pdb> {
        // The resource's own attached PDB (identified by its stable origin key —
        // survives the origin handle's free).
        if let Some(res) = self.resources.values().find(|r| r.node.key == origin_key)
            && let Some(pdb) = res.pdb
        {
            return Some(pdb);
        }
        // A SetPageDir may still be parked (declared before the target's alloc, via
        // the origin handle or any alias that resolves to this resource).
        self.pending_pdbs.iter().find_map(|(k, p)| {
            (self.origin_of(*k).map(|n| n.key) == Some(origin_key)).then_some(*p)
        })
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
    #[must_use]
    pub fn backing_of(&self, memory_key: NodeKey) -> Option<u64> {
        // Resolve by handle when possible; else fall back to the resource's stable
        // origin key (a memory kept alive ONLY by a live mapping has no live handle).
        let node = self.origin_of(memory_key).or_else(|| {
            self.resources
                .values()
                .map(|r| &r.node)
                .find(|n| n.key == memory_key)
        })?;
        matches!(node.kind, ObjectKind::Memory)
            .then(|| node.facts.mem_phys)
            .flatten()
    }

    /// Number of live mappings referencing the resource whose origin is `origin_key`
    /// (the map-refcount — a memory object is kept alive by these as well as by its
    /// handles). Zero if no such resource.
    #[must_use]
    pub fn map_ref_count(&self, origin_key: NodeKey) -> usize {
        self.resources
            .values()
            .find(|r| r.node.key == origin_key)
            .map_or(0, |r| r.map_refs)
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
        let id = self.handles.get(&key)?;
        self.resources.get(id).map(|r| &r.node)
    }

    /// ★ The Device→[`GpuId`] derivation (`multi_gpu_and_mig.md` item 1): resolve
    /// `key` (dup-aliases followed) to its owning GPU **target** by walking its
    /// declared parent chain to the nearest `Device` ancestor and reading that
    /// Device's declared `device_instance`.
    ///
    /// Pure and order-independent — a projection of declared facts, like every other
    /// derivation. `None` (MISS, never a default-GPU0 guess) when: the key does not
    /// resolve, no `Device` ancestor exists on the chain, or the Device declared no
    /// instance. Callers treat a `None` as "not routable (yet)": routing/materialization
    /// simply defers until the Device fact arrives, and use faults loudly.
    #[must_use]
    pub fn gpu_of(&self, key: NodeKey) -> Option<GpuId> {
        // The sticky per-resource cache first (survives the source Device's free — a
        // dup-kept VAS keeps its target). A Device that declared no `deviceInstance`
        // is the single-GPU default selection (NV0080 `deviceInstance` defaults to 0)
        // — the N=1 case of the axis, routing to `GpuId::ZERO`. Only the absence of a
        // Device ancestor entirely is a MISS (`None`), never a default-GPU0 guess.
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
        // cached target off the resource by its stable origin key (mirrors `pdb_of` —
        // the target, like the PDB, is a resource property that survives the free).
        self.resources
            .values()
            .find(|r| r.node.key == key)
            .and_then(|r| r.gpu)
    }
}
