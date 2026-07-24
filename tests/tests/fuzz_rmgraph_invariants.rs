//! CATEGORY A — FUZZ / PROPERTY-BASED (the highest bar: this fuzzes the guest→core
//! attack surface, decision #8/#9 boundary 1).
//!
//! A malicious guest is an **arbitrary, hostile `RmEvent` stream**. These proptest
//! properties generate valid AND malformed AND adversarial streams — dangling
//! parents, double-free, free-before-child, DUP of a freed object, DUP cycles,
//! use-after-free, handle reuse after free, unknown classes, out-of-order /
//! duplicated / interleaved events, events for non-existent clients — and assert the
//! core's INVARIANTS never break:
//!
//! - **Never panics / never UB** on ANY input: the core either derives consistent
//!   boundaries or returns a LOUD typed error. `RmGraph::apply` /
//!   `project` / `Gpu::apply` must return `Result`, never abort.
//! - **Order-independence as a PROPERTY**: for any *valid* stream, `project()` of any
//!   permutation == `project()` of the reference order (generalizes the fixed test).
//! - **Structural invariants** on any derived `Boundaries`: every channel maps to
//!   exactly one Vas/PDB; no two Procs share a PDB or vChid; identical guest
//!   handles/VAs across Procs never collide; a freed object never appears in any
//!   projection; every proc's clients are dup-connected.
//!
//! Security framing (boundary 1/3): a hostile stream must never corrupt another
//! Proc's state or the core — it may only produce a loud typed refusal.

#![allow(clippy::unusual_byte_groupings)]

use nvkvm_arch::ids::GpuId;
use std::collections::{BTreeMap, BTreeSet};

use nvkvm_arch::ids::{ClassId, HClient, HObject, Pdb};
use nvkvm_arch::{Arch, ObjectKind};
use nvkvm_core::gpa::GpaSpace;
use nvkvm_core::gpu::Gpu;
use nvkvm_core::project::project;
use nvkvm_core::rmgraph::{AllocFacts, NodeKey, RmEvent, RmGraph};
use nvkvm_mocks::{MockArch, MockIsolateFactory, mock_classes as mc};
use proptest::collection::vec;
use proptest::prelude::*;

// ---------------------------------------------------------------------------------
// Bounded generators — small identifier universes so collisions/reuse/cycles are
// LIKELY (a wide random space would almost never exercise the adversarial shapes).
// ---------------------------------------------------------------------------------

/// A tiny universe of client handles (so two "processes" routinely reuse identity).
fn any_client() -> impl Strategy<Value = HClient> {
    (0u32..4).prop_map(|n| HClient(0xC000 + n))
}

/// A tiny universe of object handles (so free-before-child / reuse are frequent).
fn any_handle() -> impl Strategy<Value = HObject> {
    (0u32..6).prop_map(|n| HObject(0x5c00_0000 + n))
}

/// A small PDB universe (so identical-PDB collision across VASpaces is reachable).
fn any_pdb() -> impl Strategy<Value = Pdb> {
    (0u32..4).prop_map(|n| Pdb(0x3400_000 + u64::from(n) * 0x1000))
}

/// Every class the mock knows, PLUS deliberate junk class ids (unknown-class path).
fn any_class() -> impl Strategy<Value = ClassId> {
    prop_oneof![
        Just(mc::CLIENT),
        Just(mc::DEVICE),
        Just(mc::VASPACE),
        Just(mc::TSG),
        Just(mc::CTXSHARE),
        Just(mc::CHANNEL_GR),
        Just(mc::CHANNEL_CE),
        Just(mc::MEMORY),
        Just(mc::EVENT),
        // Junk: not a class the arch recognizes → ObjectKind::Unknown. Must never panic.
        (0u32..0x10000).prop_map(ClassId),
    ]
}

/// A tiny VA universe (so map/unmap of the same VA — the overlap/eager-unmap paths —
/// are frequent).
fn any_va() -> impl Strategy<Value = nvkvm_arch::ids::GpuVa> {
    (0u32..4).prop_map(|n| nvkvm_arch::ids::GpuVa(0x2_0020_0000 + u64::from(n) * 0x10000))
}

/// A doorbell/vchid flags word — arbitrary bits, so vChid collisions are reachable.
fn any_userd_flags() -> impl Strategy<Value = u32> {
    (0u32..0x2_0000).prop_map(|n| n << 6)
}

/// Generate ONE arbitrary — possibly hostile — RM event.
fn any_event() -> impl Strategy<Value = RmEvent> {
    prop_oneof![
        // Alloc: parent may dangle, class may be junk, handle may already exist.
        (any_client(), any_handle(), any_handle(), any_class(), any_userd_flags(), any_pdb())
            .prop_map(|(client, parent, handle, class, flags, vpdb)| {
                RmEvent::Alloc {
                    client,
                    parent,
                    handle,
                    class,
                    facts: AllocFacts {
                        // Randomly declare an hVASpace (may point at a non-VAS / absent handle).
                        h_vaspace: if flags & 1 == 0 {
                            Some(HObject(0x5c00_0000 + (u32::from((vpdb.0 as u16) & 3))))
                        } else {
                            None
                        },
                        h_ctx_share: None,
                        userd_flags: flags,
                        // A memory alloc may or may not declare a backing (both paths).
                        mem_phys: (flags & 2 == 0).then_some(0x8000_0000 | u64::from(flags)),
                        // Single-GPU fuzz: Devices default to instance 0 (GpuId::ZERO).
                        device_instance: None,
                    },
                }
            }),
        // MapMemoryDma: vaspace/memory may dangle, not be a VAS/memory, or double-map.
        (any_client(), any_handle(), any_handle(), any_va())
            .prop_map(|(client, vaspace, memory, va)| RmEvent::MapMemoryDma {
                client, vaspace, memory, va, offset: 0, len: 0x10000,
            }),
        // Unmap: of any (vaspace, va) — mapped, never-mapped, or already-unmapped.
        (any_client(), any_handle(), any_va())
            .prop_map(|(client, vaspace, va)| RmEvent::Unmap { client, vaspace, va }),
        // Dup: src/dst may dangle, form cycles, or alias a freed object.
        (any_client(), any_handle(), any_client(), any_handle()).prop_map(
            |(sc, sh, dc, dh)| RmEvent::Dup {
                src: NodeKey::new(sc, sh),
                dst: NodeKey::new(dc, dh),
            }
        ),
        // SetPageDir: on any (client, handle), possibly not a VAS, possibly colliding PDB.
        (any_client(), any_handle(), any_pdb())
            .prop_map(|(client, vaspace, pdb)| RmEvent::SetPageDir { client, vaspace, pdb }),
        // Free: of anything — allocated, dup'd, already-freed, or never-seen.
        (any_client(), any_handle()).prop_map(|(client, handle)| RmEvent::Free { client, handle }),
    ]
}

/// A hostile stream: up to 40 arbitrary events.
fn any_stream() -> impl Strategy<Value = Vec<RmEvent>> {
    vec(any_event(), 0..40)
}

// ---------------------------------------------------------------------------------
// Structural-invariant checker — run on every derived Boundaries.
// ---------------------------------------------------------------------------------

/// Assert every structural invariant on a derived `Boundaries` (or catch a loud
/// error — both are acceptable; a panic or a corrupt projection is not).
fn assert_boundary_invariants(g: &RmGraph, arch: &dyn Arch) {
    let bounds = match project(g, arch) {
        Ok(b) => b,
        Err(_) => return, // A loud typed error is a valid outcome (MISS=FAULT posture).
    };

    // INV1: no two procs share a (target, PDB); by_pdb is 1:1 with a proc's declared
    // (target, PDB). Keyed on the GPU target — a PDB is per-GPU (MG-3).
    let mut pdb_owner: BTreeMap<(Option<GpuId>, Pdb), nvkvm_core::ProcAnchor> = BTreeMap::new();
    for p in &bounds.procs {
        for f in p.vases.values() {
            if let Some(pdb) = f.pdb {
                assert!(
                    pdb_owner.insert((f.gpu, pdb), p.anchor).is_none(),
                    "two Procs share (target, PDB {pdb}) — cross-context memory boundary broken"
                );
            }
        }
    }

    // INV2: no two procs share a (target, vChid); by_vchid is 1:1.
    let mut vchid_owner: BTreeMap<_, nvkvm_core::ProcAnchor> = BTreeMap::new();
    for p in &bounds.procs {
        for facts in p.channels.values() {
            assert!(
                vchid_owner.insert((facts.gpu, facts.vchid), p.anchor).is_none(),
                "two Procs share (target, vChid {:?}) — exec boundary broken",
                facts.vchid
            );
        }
    }

    // INV3: every channel maps to at most one Vas, and the VAS it resolved — wherever
    // it routes in the (target, PDB) map — is owned by the SAME proc (never another
    // proc's address space). Matching on the VAS origin node (not a bare PDB) is robust
    // to the per-GPU scoping of `by_pdb`.
    for p in &bounds.procs {
        for facts in p.channels.values() {
            if let Some(vas_node) = facts.vas_origin {
                for (anchor, node) in bounds.by_pdb.values() {
                    if *node == vas_node {
                        assert_eq!(
                            *anchor, p.anchor,
                            "channel resolved to a VAS owned by another Proc (confused deputy)"
                        );
                    }
                }
            }
        }
    }

    // INV4: by_pdb / by_vchid routing anchors are all real proc anchors.
    let anchors: BTreeSet<_> = bounds.procs.iter().map(|p| p.anchor).collect();
    for (anchor, _) in bounds.by_pdb.values() {
        assert!(anchors.contains(anchor), "by_pdb routes to a non-existent Proc");
    }
    for (anchor, _) in bounds.by_vchid.values() {
        assert!(anchors.contains(anchor), "by_vchid routes to a non-existent Proc");
    }

    // INV5: every proc's client set is dup-connected (a single component). Verified
    // by checking that no two distinct procs' client sets are joined by a dup edge —
    // if they were, they'd be one proc. (Union-find already did this; we re-check the
    // contrapositive: two clients in DIFFERENT procs share no dup edge.)
    let client_proc: BTreeMap<HClient, nvkvm_core::ProcAnchor> = bounds
        .procs
        .iter()
        .flat_map(|p| p.clients.iter().map(move |c| (*c, p.anchor)))
        .collect();
    for (dst, src) in g.dups() {
        // Only edges whose origin resolves participate in grouping.
        if g.origin_of(dst).is_some()
            && let (Some(&da), Some(&sa)) = (client_proc.get(&dst.client), client_proc.get(&src.client))
        {
            assert_eq!(da, sa, "a dup edge joins two DIFFERENT procs — grouping is inconsistent");
        }
    }
}

// ---------------------------------------------------------------------------------
// The properties.
// ---------------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// A1 — NEVER PANICS / NEVER UB on ANY input. Feed an arbitrary hostile stream to
    /// `RmGraph::apply` and then `project`: every step must return a `Result`
    /// (loud typed error at worst), and any successful projection must satisfy every
    /// structural invariant. This is the boundary-1 fuzz property.
    #[test]
    fn a1_hostile_stream_never_panics_and_invariants_hold(stream in any_stream()) {
        let arch = MockArch::new();
        let mut g = RmGraph::new();
        for ev in stream {
            // apply returns Result — a protocol violation is a value, never a panic.
            let _ = g.apply(&arch, ev);
            // Re-project after EACH event: an intermediate hostile state must also be
            // either consistent or a loud error, never a corrupt projection.
            assert_boundary_invariants(&g, &arch);
        }
    }

    /// A1b — the FULL Gpu spine (apply → refresh → sync) never panics on a hostile
    /// stream. `Gpu::apply` runs projection + proc/arena/isolate sync + LateMerge
    /// detection; all of it must degrade to a loud `GpuError`, never abort. Also
    /// asserts arenas stay disjoint no matter what the guest does.
    #[test]
    fn a1b_gpu_spine_never_panics_on_hostile_stream(stream in any_stream()) {
        let arch = Box::new(MockArch::new());
        let (factory, _rec) = MockIsolateFactory::new();
        let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
        let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("device realizes");

        for ev in stream {
            let _ = gpu.apply(ev); // Result — never a panic.
            // Arenas of live procs (across every target they span) are always pairwise
            // disjoint (the #14 invariant, maintained by construction regardless of
            // hostile input). Arenas are materialized lazily per (proc, GPU), so a proc
            // that touched no target yet simply contributes none.
            let ranges: Vec<_> = gpu
                .procs
                .values()
                .flat_map(|p| p.arenas.values().map(|a| a.range.clone()))
                .collect();
            for i in 0..ranges.len() {
                for j in (i + 1)..ranges.len() {
                    prop_assert!(
                        ranges[i].end <= ranges[j].start || ranges[j].end <= ranges[i].start,
                        "two live Procs' GPA arenas overlap — ALREADY-MAPPED collision class"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------
// Order-independence as a PROPERTY (over *valid* generated streams).
// ---------------------------------------------------------------------------------

/// Generate a VALID (self-consistent, no-free) graph fact set: a random number of
/// well-formed processes, each a client→device→VASpace(+PDB)→TSG→GR/CE-channel,
/// with globally-unique PDBs and vChids. Any permutation of these facts must project
/// identically (there are no Frees, so lifecycle ordering does not apply).
fn valid_fact_stream() -> impl Strategy<Value = Vec<RmEvent>> {
    (1usize..4).prop_flat_map(|n_procs| {
        // Deterministic per-index identities → globally disjoint by construction.
        let mut events = Vec::new();
        for i in 0..n_procs {
            let client = HClient(0xA000 + i as u32);
            let base = 0x1_0000 * (i as u32 + 1);
            let root = HObject(base);
            let dev = HObject(base + 1);
            let vas = HObject(base + 2);
            let tsg = HObject(base + 3);
            let grc = HObject(base + 4);
            let cec = HObject(base + 5);
            let pdb = Pdb(0x3400_000 + u64::from(i as u32) * 0x1000);
            let gr_vchid = nvkvm_arch::ids::VChid(0x100 + i as u16 * 2);
            let ce_vchid = nvkvm_arch::ids::VChid(0x101 + i as u16 * 2);
            events.push(RmEvent::Alloc { client, parent: root, handle: root, class: mc::CLIENT, facts: AllocFacts::default() });
            events.push(RmEvent::Alloc { client, parent: root, handle: dev, class: mc::DEVICE, facts: AllocFacts::default() });
            events.push(RmEvent::Alloc { client, parent: dev, handle: vas, class: mc::VASPACE, facts: AllocFacts::default() });
            events.push(RmEvent::SetPageDir { client, vaspace: vas, pdb });
            events.push(RmEvent::Alloc { client, parent: dev, handle: tsg, class: mc::TSG, facts: AllocFacts { h_vaspace: Some(vas), ..Default::default() } });
            events.push(RmEvent::Alloc { client, parent: tsg, handle: grc, class: mc::CHANNEL_GR, facts: AllocFacts { h_vaspace: Some(vas), userd_flags: MockArch::userd_flags_for(gr_vchid), ..Default::default() } });
            events.push(RmEvent::Alloc { client, parent: tsg, handle: cec, class: mc::CHANNEL_CE, facts: AllocFacts { h_vaspace: Some(vas), userd_flags: MockArch::userd_flags_for(ce_vchid), ..Default::default() } });
        }
        // Shuffle indices deterministically inside the strategy via a permutation seed.
        let len = events.len();
        (Just(events), proptest::sample::subsequence((0..len).collect::<Vec<_>>(), len))
            .prop_map(|(events, perm)| perm.into_iter().map(|i| events[i]).collect())
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// A2 — ORDER-INDEPENDENCE PROPERTY: for any valid fact stream, projecting it and
    /// projecting a reversed copy yield IDENTICAL boundaries. (The generator already
    /// hands us a shuffled order; we compare against the sorted-by-nothing reference
    /// built from the same facts in a canonical order.)
    #[test]
    fn a2_valid_streams_project_order_independently(stream in valid_fact_stream()) {
        let arch = MockArch::new();

        let project_order = |evs: &[RmEvent]| {
            let mut g = RmGraph::new();
            for &ev in evs {
                g.apply(&arch, ev).expect("valid facts apply");
            }
            project(&g, &arch).expect("valid facts project")
        };

        let forward = project_order(&stream);
        let mut reversed = stream.clone();
        reversed.reverse();
        let backward = project_order(&reversed);
        prop_assert_eq!(forward, backward, "projection must be independent of event order");
    }
}

// ---------------------------------------------------------------------------------
// A targeted structural property: a FREED object never appears in any projection.
// ---------------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1500))]

    /// A3 — a freed object never appears in any projection. Build a valid graph, then
    /// free a random client root; assert none of its nodes survive in `by_pdb`,
    /// `by_vchid`, or any proc's client set. (Use-after-projection of freed state is
    /// exactly the cross-teardown leak class, lesson L10.)
    #[test]
    fn a3_freed_client_vanishes_from_projection(
        stream in valid_fact_stream(),
        victim_idx in 0usize..3,
    ) {
        let arch = MockArch::new();
        let mut g = RmGraph::new();
        for &ev in &stream {
            g.apply(&arch, ev).expect("valid facts apply");
        }
        // Collect distinct client roots present as (client, root-handle) — the handle
        // is NOT derivable from the client id, so read it from the graph node.
        let roots: Vec<(HClient, HObject)> = {
            let mut cs: BTreeMap<HClient, HObject> = BTreeMap::new();
            for n in g.nodes() {
                if matches!(arch.classify(n.class), ObjectKind::Client) {
                    cs.insert(n.key.client, n.key.handle);
                }
            }
            cs.into_iter().collect()
        };
        prop_assume!(!roots.is_empty());
        let (victim, victim_root) = roots[victim_idx % roots.len()];

        // Snapshot the victim's PDBs/vChids BEFORE the free.
        let before = project(&g, &arch).expect("projects");
        let victim_pdbs: BTreeSet<Pdb> = before
            .procs
            .iter()
            .filter(|p| p.clients.contains(&victim))
            .flat_map(|p| p.vases.values().filter_map(|f| f.pdb))
            .collect();

        // Free the victim client root (destroys its whole namespace).
        g.apply(&arch, RmEvent::Free { client: victim, handle: victim_root })
            .expect("freeing an existing client root is legal");

        let after = project(&g, &arch).expect("projects after free");
        // The victim's clients are gone from every proc.
        for p in &after.procs {
            prop_assert!(!p.clients.contains(&victim), "freed client still grouped into a Proc");
        }
        // Its PDBs no longer route.
        for pdb in &victim_pdbs {
            prop_assert!(
                !after.by_pdb.contains_key(&(GpuId::ZERO, *pdb)),
                "freed VAS's PDB still routes"
            );
        }
        // Invariants still hold after the free.
        assert_boundary_invariants(&g, &arch);
    }
}

// ---------------------------------------------------------------------------------
// A4 — DUP_OBJECT reference counting: a resource is alive ⟺ ≥1 live handle references
// it; freeing the LAST reference destroys it (no leak); no ordering ever panics.
// ---------------------------------------------------------------------------------

/// A refcount-shaped event stream over a TINY universe so dup/free interleavings —
/// free-src-first, free-dst-first, double-free, dup-of-freed, free-then-dup — are all
/// frequent. Every `Alloc` uses a FRESH, never-reused handle (a monotonic counter in
/// the strategy), while `Dup`/`Free` range over the already-minted handle universe.
/// Unique allocations keep the "which resource" question unambiguous, so the resource
/// IS its origin `(client, handle)` for the whole run — exactly the case an
/// independent live-handle tracker can mirror without reimplementing the graph.
fn refcount_stream() -> impl Strategy<Value = Vec<(bool, RmEvent)>> {
    // We generate raw choices, then post-process into events assigning fresh alloc
    // handles. The `bool` tags each event as an alloc (for the harness's convenience).
    let client = || (0u32..3).prop_map(|n| HClient(0xB000 + n));
    let handle_pick = || (0u32..8).prop_map(|n| HObject(0x7000_0000 + n));
    #[derive(Debug, Clone, Copy)]
    enum Choice {
        Alloc(HClient, ClassId),
        Dup(HClient, HObject, HClient, HObject),
        Free(HClient, HObject),
    }
    let choice = prop_oneof![
        (client(), any_class()).prop_map(|(c, cl)| Choice::Alloc(c, cl)),
        (client(), handle_pick(), client(), handle_pick())
            .prop_map(|(sc, sh, dc, dh)| Choice::Dup(sc, sh, dc, dh)),
        (client(), handle_pick()).prop_map(|(c, h)| Choice::Free(c, h)),
    ];
    vec(choice, 0..30).prop_map(|choices| {
        let mut next: u32 = 0x7000_0000;
        choices
            .into_iter()
            .map(|c| match c {
                Choice::Alloc(client, class) => {
                    let handle = HObject(next);
                    next += 1;
                    // A fresh top-level object; parent = itself makes it a root of its
                    // own tiny tree (Client roots free their namespace, others are leaves).
                    (true, RmEvent::Alloc {
                        client, parent: handle, handle, class, facts: AllocFacts::default(),
                    })
                }
                Choice::Dup(sc, sh, dc, dh) => (false, RmEvent::Dup {
                    src: NodeKey::new(sc, sh), dst: NodeKey::new(dc, dh),
                }),
                Choice::Free(c, h) => (false, RmEvent::Free { client: c, handle: h }),
            })
            .collect()
    })
}

/// An independent live-handle tracker (NOT a reimplementation of the graph's subtree
/// logic — every generated object is a self-parented leaf/root, so `Free` of a
/// non-client handle drops exactly that handle's reference). It knows only: which
/// handles are live, which origin each references, and which parked dups await their
/// source. From that it computes the refcount-live resource set the graph must match.
#[derive(Default)]
struct RefTracker {
    /// live handle → origin `(client, handle)` it references (self for an alloc).
    handle_to_origin: BTreeMap<(HClient, HObject), (HClient, HObject)>,
    /// which live origins are Client roots (a `Free` of one drops its whole namespace).
    client_roots: BTreeSet<(HClient, HObject)>,
    /// parked dups whose source is not yet allocated: dst → src.
    pending: BTreeMap<(HClient, HObject), (HClient, HObject)>,
}

impl RefTracker {
    fn origin_of(&self, mut k: (HClient, HObject)) -> Option<(HClient, HObject)> {
        for _ in 0..64 {
            if let Some(o) = self.handle_to_origin.get(&k) {
                return Some(*o);
            }
            k = *self.pending.get(&k)?;
        }
        None
    }

    /// Live resources = the set of distinct origins some live handle references.
    fn live_resources(&self) -> BTreeSet<(HClient, HObject)> {
        self.handle_to_origin.values().copied().collect()
    }

    fn apply(&mut self, arch: &dyn Arch, ev: RmEvent) {
        match ev {
            RmEvent::Alloc { client, handle, class, .. } => {
                let k = (client, handle);
                // Mirror the graph: an alloc onto an already-bound handle (e.g. one a
                // dup aliased) is a loud ConflictingAlloc — rejected, changes nothing.
                if self.handle_to_origin.contains_key(&k) || self.pending.contains_key(&k) {
                    return;
                }
                // Fresh alloc handle by construction → a fresh resource.
                self.handle_to_origin.insert(k, k);
                if matches!(arch.classify(class), ObjectKind::Client) {
                    self.client_roots.insert(k);
                }
                // Promote any parked dup whose source just appeared (fixpoint).
                loop {
                    let ready = self.pending.iter().find_map(|(dst, src)| {
                        (!self.handle_to_origin.contains_key(dst))
                            .then(|| self.origin_of(*src).map(|o| (*dst, o)))
                            .flatten()
                    });
                    let Some((dst, origin)) = ready else { break };
                    self.pending.remove(&dst);
                    self.handle_to_origin.insert(dst, origin);
                }
            }
            RmEvent::Dup { src, dst } => {
                let (src, dst) = ((src.client, src.handle), (dst.client, dst.handle));
                if self.handle_to_origin.contains_key(&dst) || self.pending.contains_key(&dst) {
                    return; // dst already bound/parked — graph rejects the conflict.
                }
                match self.origin_of(src) {
                    Some(origin) => {
                        self.handle_to_origin.insert(dst, origin);
                    }
                    None => {
                        self.pending.insert(dst, src);
                    }
                }
            }
            RmEvent::SetPageDir { .. }
            | RmEvent::MapMemoryDma { .. }
            | RmEvent::Unmap { .. } => {}
            RmEvent::Free { client, handle } => {
                let key = (client, handle);
                let is_root = self.client_roots.contains(&key);
                // Mirror the graph: an unknown free is a loud no-op (touches nothing).
                if !self.handle_to_origin.contains_key(&key)
                    && !self.pending.contains_key(&key)
                    && !is_root
                {
                    return;
                }
                // All generated non-client objects are self-parented leaves, so the only
                // subtree free is a Client root freeing every handle in its namespace.
                let doomed: BTreeSet<(HClient, HObject)> = if is_root {
                    self.handle_to_origin.keys().filter(|k| k.0 == client).copied().collect()
                } else {
                    BTreeSet::from([key])
                };
                for k in &doomed {
                    self.handle_to_origin.remove(k);
                    self.client_roots.remove(k);
                }
                self.pending.retain(|d, s| !doomed.contains(d) && !doomed.contains(s));
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    /// A4 — DUP_OBJECT is refcounted. Over ANY alloc/dup/free ordering:
    ///  (1) nothing ever panics — `apply` returns a `Result`;
    ///  (2) a resource is present in the graph IFF ≥1 live handle references it — the
    ///      graph's live-resource set EXACTLY matches an independent live-handle
    ///      tracker (a dup keeps the resource alive after its source frees; the LAST
    ///      free destroys it), asserted every event;
    ///  (3) the refcount set is self-consistent: every handle the graph reports as a
    ///      reference resolves back to that same resource; and
    ///  (4) NO LEAK: after every live handle is freed, the graph drains to empty.
    #[test]
    fn a4_dup_object_is_reference_counted(stream in refcount_stream()) {
        let arch = MockArch::new();
        let mut g = RmGraph::new();
        let mut tracker = RefTracker::default();

        for (_is_alloc, ev) in &stream {
            let _ = g.apply(&arch, *ev); // never panics — a Result, at worst a loud error
            tracker.apply(&arch, *ev);

            // (2) alive ⟺ ≥1 live handle references it.
            let graph_live: BTreeSet<(HClient, HObject)> =
                g.nodes().map(|n| (n.key.client, n.key.handle)).collect();
            prop_assert_eq!(&graph_live, &tracker.live_resources(),
                "graph live-resource set must equal the independent refcount tracker");

            // (3) every reference the graph reports resolves to its resource, and each
            //     live resource has at least one such reference (non-empty refcount).
            for origin in &graph_live {
                let key = NodeKey::new(origin.0, origin.1);
                let refs: Vec<_> = g.references(key).collect();
                prop_assert!(!refs.is_empty(), "a live resource must have ≥1 reference");
                for h in refs {
                    prop_assert_eq!(g.origin_of(h).map(|n| n.key), Some(key),
                        "a reported reference must resolve back to its resource");
                }
            }
            // A fully-freed handle resolves to nothing and references nothing (no leak).
            // (Check the whole small handle universe for stale resolution.)
            for c in 0..3u32 {
                for h in 0..8u32 {
                    let k = NodeKey::new(HClient(0xB000 + c), HObject(0x7000_0000 + h));
                    if tracker.origin_of((k.client, k.handle)).is_none() {
                        prop_assert!(g.origin_of(k).is_none(),
                            "a handle that references nothing must resolve to nothing (no stale/leak)");
                    }
                }
            }
        }

        // (4) NO LEAK: free every remaining live handle; the graph must drain to empty.
        let remaining: Vec<NodeKey> = tracker
            .handle_to_origin
            .keys()
            .map(|(c, h)| NodeKey::new(*c, *h))
            .collect();
        for k in remaining {
            let _ = g.apply(&arch, RmEvent::Free { client: k.client, handle: k.handle });
        }
        prop_assert_eq!(g.nodes().count(), 0, "no resource leaks after every handle is freed");
    }
}
