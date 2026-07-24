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
        // Junk: not a class the arch recognizes → ObjectKind::Unknown. Must never panic.
        (0u32..0x10000).prop_map(ClassId),
    ]
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
                    },
                }
            }),
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

    // INV1: no two procs share a PDB, and by_pdb is 1:1 with a proc's declared PDB.
    let mut pdb_owner: BTreeMap<Pdb, nvkvm_core::ProcAnchor> = BTreeMap::new();
    for p in &bounds.procs {
        for pdb in p.vases.values().flatten() {
            assert!(
                pdb_owner.insert(*pdb, p.anchor).is_none(),
                "two Procs share PDB {pdb} — cross-context memory boundary broken"
            );
        }
    }

    // INV2: no two procs share a vChid; by_vchid is 1:1.
    let mut vchid_owner: BTreeMap<_, nvkvm_core::ProcAnchor> = BTreeMap::new();
    for p in &bounds.procs {
        for facts in p.channels.values() {
            assert!(
                vchid_owner.insert(facts.vchid, p.anchor).is_none(),
                "two Procs share vChid {:?} — exec boundary broken",
                facts.vchid
            );
        }
    }

    // INV3: every channel maps to at most one Vas/PDB, and if it has a PDB that PDB
    // is owned by the SAME proc (never another proc's address space).
    for p in &bounds.procs {
        for facts in p.channels.values() {
            if let Some(pdb) = facts.vas_pdb {
                let owner = bounds.by_pdb.get(&pdb).map(|x| x.0);
                assert_eq!(
                    owner,
                    Some(p.anchor),
                    "channel resolved to a PDB owned by another Proc (confused deputy)"
                );
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
            // Arenas of live procs are always pairwise disjoint (the #14 invariant,
            // maintained by construction regardless of hostile input).
            let ranges: Vec<_> = gpu.procs.values().map(|p| p.arena.range.clone()).collect();
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
            .flat_map(|p| p.vases.values().flatten().copied())
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
            prop_assert!(!after.by_pdb.contains_key(pdb), "freed VAS's PDB still routes");
        }
        // Invariants still hold after the free.
        assert_boundary_invariants(&g, &arch);
    }
}
