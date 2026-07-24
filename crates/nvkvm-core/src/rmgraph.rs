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
//! **Order tolerance mechanics:** `apply` records facts keyed by handle and resolves
//! references only at projection time, so a `Dup` may arrive before its source's
//! `Alloc`, a `SetPageDir` before its VASpace exists, etc. Only [`RmEvent::Free`] is
//! inherently ordered (lifecycle), which is why the shuffle property is stated over
//! alloc/dup/bind facts.

use std::collections::BTreeMap;

use nvkvm_arch::ids::{ClassId, HClient, HObject, Pdb};
use nvkvm_arch::{Arch, ObjectKind};

/// Global identity of an RM object: handles are **per-client namespaces** (two
/// processes routinely present identical `HObject` values — #14 round 1), so a
/// node is keyed by `(client, handle)`.
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
    /// grouping (how UVM aliases the compute client's VASpace).
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
    /// `RM_FREE`: destroy `handle` and (per RM semantics) its subtree. Freeing the
    /// client root destroys the whole namespace.
    Free {
        /// Owning client namespace.
        client: HClient,
        /// Handle to free.
        handle: HObject,
    },
}

/// One node of the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RmNode {
    /// Identity.
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

/// Errors from [`RmGraph::apply`]. All loud; the caller decides whether a guest
/// protocol violation is fatal or logged-and-refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RmGraphError {
    /// A second, *different* alloc for an existing key (an identical re-send is
    /// accepted idempotently — retried-RPC tolerance, testing strategy `wo_retried_rpc`).
    ConflictingAlloc(NodeKey),
    /// A second, *different* dup for an existing dst key.
    ConflictingDup(NodeKey),
    /// Free of a handle that was never allocated or dup'd.
    FreeUnknown(NodeKey),
}

/// The RM resource graph. Facts in, projections out; no policy.
#[derive(Debug, Clone, Default)]
pub struct RmGraph {
    /// Allocated nodes by key.
    nodes: BTreeMap<NodeKey, RmNode>,
    /// Dup aliases: dst → src. Chains allowed (dup of a dup).
    dups: BTreeMap<NodeKey, NodeKey>,
    /// Declared PDB per VASpace node key (tolerates arriving before the alloc).
    pdbs: BTreeMap<NodeKey, Pdb>,
}

impl RmGraph {
    /// Empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one declared fact. Idempotent for identical re-sends; conflicting
    /// redefinitions are loud errors.
    pub fn apply(&mut self, arch: &dyn Arch, ev: RmEvent) -> Result<(), RmGraphError> {
        match ev {
            RmEvent::Alloc { client, parent, handle, class, facts } => {
                let key = NodeKey::new(client, handle);
                let node =
                    RmNode { key, parent, class, kind: arch.classify(class), facts };
                match self.nodes.get(&key) {
                    Some(existing) if *existing == node => Ok(()), // idempotent retry
                    Some(_) => Err(RmGraphError::ConflictingAlloc(key)),
                    None => {
                        self.nodes.insert(key, node);
                        Ok(())
                    }
                }
            }
            RmEvent::Dup { src, dst } => match self.dups.get(&dst) {
                Some(existing) if *existing == src => Ok(()), // idempotent retry
                Some(_) => Err(RmGraphError::ConflictingDup(dst)),
                None => {
                    self.dups.insert(dst, src);
                    Ok(())
                }
            },
            RmEvent::SetPageDir { client, vaspace, pdb } => {
                // Re-binding a VASpace to a new PDB is protocol-legal
                // (UNSET/SET_PAGE_DIRECTORY); last declaration wins.
                self.pdbs.insert(NodeKey::new(client, vaspace), pdb);
                Ok(())
            }
            RmEvent::Free { client, handle } => {
                let key = NodeKey::new(client, handle);
                let known = self.nodes.contains_key(&key) || self.dups.contains_key(&key);
                if !known {
                    return Err(RmGraphError::FreeUnknown(key));
                }
                self.free_subtree(key);
                Ok(())
            }
        }
    }

    /// RM free semantics: remove `key`, its same-namespace descendants, its dup
    /// aliases (an alias dies with its target's namespace entry), and — for a
    /// client root — the entire namespace.
    fn free_subtree(&mut self, key: NodeKey) {
        let is_client_root = self
            .nodes
            .get(&key)
            .is_some_and(|n| matches!(n.kind, ObjectKind::Client));

        let doomed: Vec<NodeKey> = if is_client_root {
            self.nodes.keys().filter(|k| k.client == key.client).copied().collect()
        } else {
            // key + transitive children within the same client namespace.
            let mut doomed = vec![key];
            let mut changed = true;
            while changed {
                changed = false;
                for n in self.nodes.values() {
                    let parent_key = NodeKey::new(n.key.client, n.parent);
                    if doomed.contains(&parent_key)
                        && !doomed.contains(&n.key)
                        && n.key != parent_key
                    {
                        doomed.push(n.key);
                        changed = true;
                    }
                }
            }
            doomed
        };
        for k in &doomed {
            self.nodes.remove(k);
            self.pdbs.remove(k);
        }
        // Drop dup edges whose src or dst died (dst alias handles too).
        self.dups
            .retain(|dst, src| !doomed.contains(dst) && !doomed.contains(src) && *dst != key);
    }

    /// Resolve `key` through dup aliasing to its **origin** node (the non-alias
    /// source). `None` if the chain dangles (fact not yet observed) or the origin
    /// was never allocated.
    #[must_use]
    pub fn origin_of(&self, key: NodeKey) -> Option<&RmNode> {
        let mut k = key;
        // Bounded: dup chains are tiny; guard against a (protocol-invalid) cycle.
        for _ in 0..64 {
            if let Some(node) = self.nodes.get(&k) {
                return Some(node);
            }
            k = *self.dups.get(&k)?;
        }
        None
    }

    /// Declared PDB of the VASpace whose **origin** is `origin_key`, if declared.
    /// (A PDB declared on an alias key resolves to the same origin.)
    #[must_use]
    pub fn pdb_of(&self, origin_key: NodeKey) -> Option<Pdb> {
        if let Some(p) = self.pdbs.get(&origin_key) {
            return Some(*p);
        }
        // A SetPageDir may have been declared via an alias handle.
        self.pdbs.iter().find_map(|(k, p)| {
            (self.origin_of(*k).map(|n| n.key) == Some(origin_key)).then_some(*p)
        })
    }

    /// All live nodes, ascending key order (deterministic).
    pub fn nodes(&self) -> impl Iterator<Item = &RmNode> {
        self.nodes.values()
    }

    /// All dup edges as `(dst, src)`, ascending dst order (deterministic).
    pub fn dups(&self) -> impl Iterator<Item = (NodeKey, NodeKey)> {
        self.dups.iter().map(|(d, s)| (*d, *s))
    }

    /// Look up a node by exact key (no alias resolution).
    #[must_use]
    pub fn node(&self, key: NodeKey) -> Option<&RmNode> {
        self.nodes.get(&key)
    }
}
