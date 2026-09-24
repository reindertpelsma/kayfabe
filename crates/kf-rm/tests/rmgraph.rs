//! `kf_rm::rmgraph::RmGraph` — the object graph's own invariants, v3.
//!
//! Ported from the old tree's graph-level properties (`tests/tests/rmgraph_order_independence.rs`,
//! `tests/tests/fuzz_rmgraph_invariants.rs` a4, `tests/tests/t0_subset_free.rs`). ⊘ Those files
//! asserted mostly on the old `Proc` PROJECTION (`kayfabe_core::project`, `Boundaries`,
//! `by_pdb`, `Gpu`, isolates); v3 drops the projection and the address plane, so each test
//! here keeps the part that is a fact about the GRAPH and compares graph snapshots
//! (`nodes`, `dups`, `client_declarations`, `references`, `origin_of`) instead of `project()`.
//! `SetPageDir`/`MapMemoryDma`/`Unmap` events are gone from `RmEvent` in v3 and are simply
//! absent from every scenario.
//!
//! New here: the three defects `V3_P2_PORT_MAP.md` §4.4 names in the doorbell graph, each
//! proved absent, and the per-family class gate + classification quantified over
//! `kf_chip::classes::FAMILIES`.

#![allow(clippy::unusual_byte_groupings)]

use std::collections::{BTreeMap, BTreeSet};

use kf_abi::generated::classes as nv;
use kf_arch::ids::{ClassId, EngineKind, HClient, HObject};
use kf_arch::{ClientKind, ObjectKind};
use kf_chip::{Family, Kind};
use kf_rm::rmgraph::{AllocFacts, ClientKey, NodeKey, RmEvent, RmGraph, RmGraphError, RmNode};

// ---------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------

/// The measured identities of the old suite (RTX 3060 / 580.159.04): the kernel client's
/// handle lies BETWEEN the two user clients', so handle value is not a discriminator.
const A: HClient = HClient(0xc1d0_0067);
const B: HClient = HClient(0xc1d0_0068);
const UVM: HClient = HClient(0xc1d0_0069);
const A2: HClient = HClient(0xc1d0_006b);

fn user(client: HClient) -> AllocFacts {
    AllocFacts { client_kind: Some(ClientKind::User { pid: client.0 }), ..Default::default() }
}

fn kernel() -> AllocFacts {
    AllocFacts { client_kind: Some(ClientKind::Kernel), ..Default::default() }
}

fn client_root(client: HClient) -> RmEvent {
    RmEvent::Alloc {
        client,
        parent: HObject(client.0),
        handle: HObject(client.0),
        class: ClassId(nv::NV01_ROOT_CLIENT),
        facts: user(client),
    }
}

fn kernel_client_root(client: HClient) -> RmEvent {
    RmEvent::Alloc {
        client,
        parent: HObject(client.0),
        handle: HObject(client.0),
        class: ClassId(nv::NV01_ROOT_CLIENT),
        facts: kernel(),
    }
}

fn alloc(client: HClient, parent: HObject, handle: HObject, class: u32) -> RmEvent {
    let facts = if class == nv::NV01_DEVICE_0 {
        AllocFacts { device_instance: Some(0), ..Default::default() }
    } else {
        AllocFacts::default()
    };
    RmEvent::Alloc { client, parent, handle, class: ClassId(class), facts }
}

fn free(client: HClient, handle: HObject) -> RmEvent {
    RmEvent::Free { client, handle }
}

fn dup(src: NodeKey, dst: NodeKey) -> RmEvent {
    RmEvent::Dup { src, dst }
}

/// A compute process's subgraph: root, device, subdevice, VA space, TSG. Returns the VA
/// space's key. ⊘ The old helper also emitted `SetPageDir` + channels with vChids — the PDB
/// is cut from the graph in v3 and the channel's vChid was a projection fact.
fn compute_process(ev: &mut Vec<RmEvent>, client: HClient, base: u32) -> NodeKey {
    let root = HObject(client.0);
    let (dev, sub, vas, tsg) =
        (HObject(base + 1), HObject(base + 2), HObject(base + 0x10), HObject(base + 0x12));
    ev.push(client_root(client));
    ev.push(alloc(client, root, dev, nv::NV01_DEVICE_0));
    ev.push(alloc(client, dev, sub, nv::NV20_SUBDEVICE_0));
    ev.push(alloc(client, dev, vas, nv::FERMI_VASPACE_A));
    ev.push(alloc(client, dev, tsg, nv::KEPLER_CHANNEL_GROUP_A));
    NodeKey::new(client, vas)
}

/// The old `scenario()`: one kernel/UVM session client every process dups its VA space
/// into, two user processes, and a second user client of A joined by a peer dup.
fn scenario() -> Vec<RmEvent> {
    let mut ev = vec![kernel_client_root(UVM)];
    ev.push(alloc(UVM, HObject(UVM.0), HObject(0x9000_0001), nv::NV01_DEVICE_0));
    ev.push(alloc(UVM, HObject(0x9000_0001), HObject(0x9000_0010), nv::FERMI_VASPACE_A));
    let a_vas = compute_process(&mut ev, A, 0x5c00_0000);
    ev.push(dup(a_vas, NodeKey::new(UVM, HObject(0x9000_00a7))));
    let b_vas = compute_process(&mut ev, B, 0x5d00_0000);
    ev.push(dup(b_vas, NodeKey::new(UVM, HObject(0x9000_00a8))));
    compute_process(&mut ev, A2, 0x7000_0000);
    ev.push(dup(a_vas, NodeKey::new(A2, HObject(0x7000_00ff))));
    ev
}

/// The client namespaces an event names, and the one it declares — for [`legal_order`].
fn names(ev: RmEvent, g: &RmGraph) -> (Vec<HClient>, Option<HClient>) {
    match ev {
        RmEvent::Alloc { client, class, .. } if matches!(g.classify(class), ObjectKind::Client) => {
            (vec![], Some(client))
        }
        RmEvent::Alloc { client, .. } | RmEvent::Free { client, .. } => (vec![client], None),
        RmEvent::Dup { src, dst } => (vec![src.client, dst.client], None),
    }
}

/// The old `kayfabe_tests::legal_order`: stable-defer every event that names a namespace not
/// yet declared (the ONE ordering RM itself enforces, §12.38).
fn legal_order(events: &[RmEvent]) -> Vec<RmEvent> {
    let probe = RmGraph::new(Family::Ampere);
    let mut declared: BTreeSet<HClient> = BTreeSet::new();
    let mut pending = events.to_vec();
    let mut out = Vec::with_capacity(events.len());
    while !pending.is_empty() {
        let before = out.len();
        let mut deferred = Vec::new();
        for ev in pending {
            let (need, declares) = names(ev, &probe);
            if need.iter().all(|c| declared.contains(c)) {
                if let Some(c) = declares {
                    declared.insert(c);
                }
                out.push(ev);
            } else {
                deferred.push(ev);
            }
        }
        assert!(out.len() > before, "the event set names a namespace that is never declared");
        pending = deferred;
    }
    out
}

/// The old `permutations`: rotations, the reverse, and an even/odd interleave.
fn permutations(n: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    for shift in 0..n {
        out.push((0..n).map(|i| (i + shift) % n).collect());
    }
    out.push((0..n).rev().collect());
    let mut il: Vec<usize> = (0..n).step_by(2).collect();
    il.extend((1..n).step_by(2));
    out.push(il);
    out
}

/// An order-independent snapshot of the graph's facts. ⊘ `ClientId` is excluded: it is minted
/// from a monotonic resource counter and is therefore arrival-order dependent BY DESIGN (the
/// old projection never compared it either).
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    nodes: BTreeMap<NodeKey, RmNode>,
    dups: Vec<(NodeKey, NodeKey)>,
    declarations: BTreeMap<ClientKey, ClientKind>,
    refs: BTreeMap<NodeKey, BTreeSet<NodeKey>>,
}

fn snapshot(g: &RmGraph) -> Snapshot {
    let nodes: BTreeMap<NodeKey, RmNode> = g.nodes().map(|n| (n.key, *n)).collect();
    let refs = nodes.keys().map(|k| (*k, g.references(*k).collect())).collect();
    Snapshot {
        dups: g.dups().collect(),
        declarations: g.client_declarations().into_iter().map(|(k, (_, kind))| (k, kind)).collect(),
        nodes,
        refs,
    }
}

fn graph_of(events: &[RmEvent]) -> RmGraph {
    let mut g = RmGraph::new(Family::Ampere);
    for &ev in events {
        g.apply(ev).expect("scripted events are valid");
    }
    g
}

// ---------------------------------------------------------------------------------------
// Ported: order independence + refusals (rmgraph_order_independence.rs)
// ---------------------------------------------------------------------------------------

/// Old `by_pdb_by_vchid_and_proc_grouping_are_order_independent` and
/// `the_kernel_declaration_may_arrive_before_between_or_after`: the GRAPH a legal fact set
/// produces is independent of arrival order. ⊘ The by_pdb / vChid / Proc grouping halves
/// were projection facts (dropped with the projection).
#[test]
fn the_graph_is_order_independent_over_legal_orders() {
    let events = scenario();
    let reference = snapshot(&graph_of(&events));
    assert!(reference.nodes.len() > 15 && !reference.dups.is_empty(), "non-vacuous scenario");
    for p in permutations(events.len()) {
        let shuffled: Vec<RmEvent> = p.iter().map(|&i| events[i]).collect();
        let g = graph_of(&legal_order(&shuffled));
        assert_eq!(snapshot(&g), reference, "order {p:?}");
    }
}

/// Old `a_dup_into_an_undeclared_namespace_is_refused_in_every_order`: refused at every
/// insertion point, and the refusal mutates nothing (graph snapshot, ids included).
#[test]
fn a_dup_into_an_undeclared_namespace_is_refused_in_every_order() {
    let events = scenario();
    let planted = dup(NodeKey::new(A, HObject(0x5c00_0010)), NodeKey::new(HClient(0xc1d0_00ff), HObject(0x7777_0001)));
    let refusal = Err(RmGraphError::UndeclaredClient(HClient(0xc1d0_00ff)));
    let reference = graph_of(&events);
    for at in 0..=events.len() {
        let mut g = RmGraph::new(Family::Ampere);
        for (i, &ev) in events.iter().enumerate() {
            if i == at {
                assert_eq!(g.apply(planted), refusal, "position {at}");
            }
            g.apply(ev).expect("the legal events still apply");
        }
        if at == events.len() {
            assert_eq!(g.apply(planted), refusal, "…including last");
        }
        assert_eq!(snapshot(&g), snapshot(&reference), "a refused event mutated the graph (at {at})");
        assert_eq!(g.client_declarations(), reference.client_declarations(), "ids too (at {at})");
    }
}

/// Old `one_kernel_client_two_processes_stay_two_procs` / `a_dupd_vaspace_stays_with_the_
/// client_that_allocated_it` — graph half: a VA space dup'd into the kernel session is ONE
/// resource with two references, owned by the declaration that ALLOCATED it.
#[test]
fn a_dupd_vaspace_stays_owned_by_the_client_that_allocated_it() {
    let g = graph_of(&scenario());
    let a_vas = NodeKey::new(A, HObject(0x5c00_0010));
    let alias = NodeKey::new(UVM, HObject(0x9000_00a7));
    assert_eq!(g.origin_of(alias).map(|n| n.key), Some(a_vas));
    assert_eq!(g.owner_key_of(alias), Some(ClientKey::first(A)));
    assert_eq!(g.owner_key_of(a_vas), Some(ClientKey::first(A)));
    let refs: BTreeSet<NodeKey> = g.references(a_vas).collect();
    assert_eq!(refs, BTreeSet::from([a_vas, alias, NodeKey::new(A2, HObject(0x7000_00ff))]));
    // ⊘ The Proc-grouping half (user↔user merges, kernel refs do not) was a projection rule.
}

/// Old `identical_handles_across_procs_do_not_collide` — and §4.4 defect 1 (see below).
#[test]
fn identical_handles_across_clients_do_not_collide() {
    let mut ev = Vec::new();
    compute_process(&mut ev, A, 0x5c00_0000);
    compute_process(&mut ev, B, 0x5c00_0000); // the SAME object handle values
    let g = graph_of(&ev);
    for h in [0x5c00_0001u32, 0x5c00_0002, 0x5c00_0010, 0x5c00_0012] {
        let (ka, kb) = (NodeKey::new(A, HObject(h)), NodeKey::new(B, HObject(h)));
        assert_eq!(g.node(ka).map(|n| n.key), Some(ka));
        assert_eq!(g.node(kb).map(|n| n.key), Some(kb));
        assert_eq!(g.owner_key_of(ka), Some(ClientKey::first(A)));
        assert_eq!(g.owner_key_of(kb), Some(ClientKey::first(B)));
    }
}

/// Old `client_handle_zero_is_refused_so_the_system_anchor_cannot_be_squatted`.
#[test]
fn client_handle_zero_is_refused() {
    let mut g = RmGraph::new(Family::Ampere);
    let zero = HClient(0);
    assert_eq!(g.apply(client_root(zero)), Err(RmGraphError::ReservedClient(zero)));
    assert_eq!(
        g.apply(dup(NodeKey::new(A, HObject(1)), NodeKey::new(zero, HObject(2)))),
        Err(RmGraphError::ReservedClient(zero))
    );
    assert_eq!(
        g.apply(dup(NodeKey::new(zero, HObject(1)), NodeKey::new(A, HObject(2)))),
        Err(RmGraphError::ReservedClient(zero))
    );
    assert_eq!(g.apply(free(zero, HObject(1))), Err(RmGraphError::ReservedClient(zero)));
    assert_eq!(g.client_kinds().count(), 0, "nothing entered the graph");
}

/// Old `an_undeclared_or_doubly_declared_client_root_is_a_loud_refusal`.
#[test]
fn an_undeclared_or_doubly_declared_client_root_is_a_loud_refusal() {
    let mut g = RmGraph::new(Family::Ampere);
    let root = HObject(A.0);
    let undeclared = RmEvent::Alloc {
        client: A,
        parent: root,
        handle: root,
        class: ClassId(nv::NV01_ROOT_CLIENT),
        facts: AllocFacts::default(),
    };
    assert_eq!(g.apply(undeclared), Err(RmGraphError::UndeclaredClientKind(NodeKey::new(A, root))));
    assert_eq!(g.nodes().count(), 0, "the refusal changed nothing");

    g.apply(client_root(A)).expect("declared");
    g.apply(client_root(A)).expect("an identical re-send is idempotent");

    let second = HObject(A.0 + 0x100);
    assert_eq!(
        g.apply(RmEvent::Alloc {
            client: A,
            parent: second,
            handle: second,
            class: ClassId(nv::NV01_ROOT_CLIENT),
            facts: kernel(),
        }),
        Err(RmGraphError::DuplicateClientRoot {
            existing: NodeKey::new(A, root),
            attempted: NodeKey::new(A, second),
        }),
    );
    assert_eq!(
        g.client_kinds().collect::<Vec<_>>(),
        vec![(ClientKey::first(A), ClientKind::User { pid: A.0 })],
        "the namespace kept its ONE declared kind",
    );

    g.apply(free(A, root)).expect("the root frees");
    g.apply(kernel_client_root(A)).expect("an empty namespace may declare a fresh root");
    assert_eq!(g.client_kinds().collect::<Vec<_>>(), vec![(ClientKey::first(A), ClientKind::Kernel)]);
}

/// Old `a_root_kept_alive_by_a_dup_no_longer_occupies_its_own_namespace` — verbatim graph
/// assertions.
#[test]
fn a_root_kept_alive_by_a_dup_no_longer_occupies_its_own_namespace() {
    let mut g = RmGraph::new(Family::Ampere);
    g.apply(client_root(A)).expect("A's root");
    g.apply(client_root(B)).expect("B's root");
    g.apply(dup(NodeKey::new(B, HObject(B.0)), NodeKey::new(A, HObject(0x900))))
        .expect("A aliases B's client object");
    g.apply(free(B, HObject(B.0))).expect("B's root frees");
    assert_eq!(
        g.origin_of(NodeKey::new(A, HObject(0x900))).map(|n| n.key),
        Some(NodeKey::new(B, HObject(B.0))),
        "the alias still resolves — the resource outlived its origin handle",
    );
    assert!(!g.client_kinds().any(|(c, _)| c.client == B), "a freed root declares nothing");
    g.apply(kernel_client_root(B)).expect("★ the emptied namespace may declare a fresh root");
    assert_eq!(
        g.client_kinds().collect::<Vec<_>>(),
        vec![
            (ClientKey::first(A), ClientKind::User { pid: A.0 }),
            (ClientKey { client: B, incarnation: 1 }, ClientKind::Kernel),
        ],
    );
    g.apply(free(A, HObject(0x900))).expect("the alias frees");
    assert_eq!(
        g.client_kinds().find(|(c, _)| c.client == B).map(|(_, k)| k),
        Some(ClientKind::Kernel),
        "the dead alias pruned the LIVE root's declaration",
    );
    assert_eq!(
        g.client_declarations().keys().filter(|k| k.client == B).copied().collect::<Vec<_>>(),
        vec![ClientKey { client: B, incarnation: 1 }],
        "the orphaned declaration outlived the last resource it owned",
    );
}

/// Old `a_freed_and_redeclared_namespace_projects_identically_in_every_order` — graph half:
/// a recycled `hClient` value is two declarations; the survivor kept alive by a keeper's
/// alias stays owned by the FIRST, the new tenant's objects by the second, in every legal
/// order. ⊘ The by_pdb / Proc-component assertions were projection facts.
#[test]
fn a_freed_and_redeclared_namespace_is_two_declarations_in_every_order() {
    const RECYCLED: HClient = HClient(0xc1d0_0080);
    const KEEPER: HClient = HClient(0xc1d0_0081);
    let mut phase1 = Vec::new();
    let first_vas = compute_process(&mut phase1, RECYCLED, 0x5e00_0000);
    phase1.push(client_root(KEEPER));
    let alias = NodeKey::new(KEEPER, HObject(0x6e00_0011));
    phase1.push(dup(first_vas, alias));
    let mut phase2 = Vec::new();
    let second_vas = compute_process(&mut phase2, RECYCLED, 0x6f00_0000);

    let run = |a: &[RmEvent], b: &[RmEvent]| {
        let mut g = RmGraph::new(Family::Ampere);
        for ev in legal_order(a) {
            g.apply(ev).expect("phase 1 applies in any legal order");
        }
        g.apply(free(RECYCLED, HObject(RECYCLED.0))).expect("the first tenant's root frees");
        for ev in legal_order(b) {
            g.apply(ev).expect("★ re-declaring a recycled namespace is LEGAL");
        }
        g
    };
    let reference = run(&phase1, &phase2);
    let gen1 = ClientKey::first(RECYCLED);
    let gen2 = ClientKey { client: RECYCLED, incarnation: 1 };
    assert_eq!(reference.owner_key_of(alias), Some(gen1), "the survivor stays with its allocator");
    assert_eq!(reference.origin_of(alias).map(|n| n.key), Some(first_vas));
    assert_eq!(reference.owner_key_of(second_vas), Some(gen2), "the new tenant is its own declaration");
    let decls: Vec<ClientKey> =
        reference.client_declarations().keys().filter(|k| k.client == RECYCLED).copied().collect();
    assert_eq!(decls, vec![gen1, gen2]);
    for pa in permutations(phase1.len()) {
        for pb in permutations(phase2.len()) {
            let a: Vec<RmEvent> = pa.iter().map(|&i| phase1[i]).collect();
            let b: Vec<RmEvent> = pb.iter().map(|&i| phase2[i]).collect();
            assert_eq!(snapshot(&run(&a, &b)), snapshot(&reference), "perms {pa:?} / {pb:?}");
        }
    }
}

/// Old `a_recycled_object_handle_projects_identically_in_every_order` — graph half: a freed
/// OBJECT handle re-allocated while a dup keeps the first resource alive mints a NEW
/// incarnation; the alias still names the old one. ⊘ The second tenant's `SetPageDir` is cut
/// (no PDB in the v3 graph), and the by_pdb halves were projection facts.
#[test]
fn a_recycled_object_handle_is_a_new_incarnation_in_every_order() {
    let mut phase1 = Vec::new();
    let vas = compute_process(&mut phase1, A, 0x5c00_0000);
    phase1.push(client_root(B));
    let alias = NodeKey::new(B, HObject(0x7f00_0011));
    phase1.push(dup(vas, alias));
    let realloc = alloc(A, HObject(0x5c00_0001), vas.handle, nv::FERMI_VASPACE_A);
    let run = |a: &[RmEvent]| {
        let mut g = RmGraph::new(Family::Ampere);
        for ev in legal_order(a) {
            g.apply(ev).expect("phase 1");
        }
        g.apply(free(A, vas.handle)).expect("the origin handle frees");
        g.apply(realloc).expect("★ re-allocating a freed object handle is LEGAL");
        g
    };
    let g = run(&phase1);
    assert_eq!(g.node(vas).map(|n| n.incarnation), Some(1), "the new tenant of the handle value");
    assert_eq!(g.origin_of(alias).map(|n| (n.key, n.incarnation)), Some((vas, 0)), "the survivor");
    assert_eq!(g.nodes().filter(|n| n.key == vas).count(), 2, "two live resources at one handle value");
    for p in permutations(phase1.len()) {
        let a: Vec<RmEvent> = p.iter().map(|&i| phase1[i]).collect();
        assert_eq!(snapshot(&run(&a)), snapshot(&g), "perm {p:?}");
    }
}

/// Old `a_dup_whose_source_declaration_lost_its_root_never_regroups_onto_the_recycled_value`
/// — graph half: the edge survives the source root's free and keeps naming the DEAD
/// declaration after the value is re-declared.
#[test]
fn a_dup_whose_source_lost_its_root_keeps_naming_the_dead_declaration() {
    const SRC: HClient = HClient(0xE1);
    const DST: HClient = HClient(0xE2);
    let alias = NodeKey::new(DST, HObject(0x7f00_0001));
    let mut g = RmGraph::new(Family::Ampere);
    g.apply(client_root(DST)).expect("dst");
    g.apply(client_root(SRC)).expect("src");
    g.apply(dup(NodeKey::new(SRC, HObject(SRC.0)), alias)).expect("the alias applies");
    g.apply(free(SRC, HObject(SRC.0))).expect("the source frees its root");
    assert_eq!(g.origin_of(alias).map(|n| n.key), Some(NodeKey::new(SRC, HObject(SRC.0))));
    g.apply(client_root(SRC)).expect("★ re-declaring a recycled namespace is LEGAL");
    assert_eq!(g.owner_key_of(alias), Some(ClientKey::first(SRC)), "never the new tenant");
    assert_eq!(
        g.client_declarations().keys().filter(|k| k.client == SRC).copied().collect::<Vec<_>>(),
        vec![ClientKey::first(SRC), ClientKey { client: SRC, incarnation: 1 }],
    );
}

/// §12.39 Part A — a dup PARKED against a not-yet-allocated source is destroyed with its
/// namespace's root; it can never fire into the next tenant of the recycled value.
#[test]
fn a_parked_dup_dies_with_its_namespace_root() {
    let mut g = RmGraph::new(Family::Ampere);
    g.apply(client_root(A)).expect("A");
    g.apply(client_root(B)).expect("B");
    let src = NodeKey::new(A, HObject(0x1234));
    let dst = NodeKey::new(B, HObject(0x5678));
    g.apply(dup(src, dst)).expect("a dup may precede its source object (order tolerance)");
    assert_eq!(g.dups().collect::<Vec<_>>(), vec![(dst, src)], "parked");
    g.apply(free(A, HObject(A.0))).expect("the source namespace dies");
    assert_eq!(g.dups().count(), 0, "the parked edge died with the namespace");
    g.apply(client_root(A)).expect("re-declared");
    g.apply(alloc(A, HObject(A.0), src.handle, nv::FERMI_VASPACE_A)).expect("the new tenant allocates the handle");
    assert!(g.origin_of(dst).is_none(), "★ the planted alias never fired");
}

#[test]
fn a_parked_dup_resolves_when_its_source_arrives() {
    let mut g = RmGraph::new(Family::Ampere);
    g.apply(client_root(A)).expect("A");
    g.apply(client_root(B)).expect("B");
    let src = NodeKey::new(A, HObject(0x1234));
    let dst = NodeKey::new(B, HObject(0x5678));
    g.apply(dup(src, dst)).expect("parks");
    assert!(g.node(dst).is_none());
    g.apply(alloc(A, HObject(A.0), src.handle, nv::FERMI_VASPACE_A)).expect("source");
    assert_eq!(g.origin_of(dst).map(|n| n.key), Some(src));
    assert_eq!(g.references(src).collect::<BTreeSet<_>>(), BTreeSet::from([src, dst]));
}

/// Old `t0_subset_free.rs` — graph half: the guest frees a SUBSET and keeps running; the
/// freed object's subtree goes, its parent and siblings stay. ⊘ The host-state reclamation
/// (host VAS, GPA blocks, engine objects) was the old `Gpu`/isolate plane.
#[test]
fn the_guest_frees_a_subset_and_keeps_running() {
    let mut ev = Vec::new();
    let vas = compute_process(&mut ev, A, 0x5c00_0000);
    let tsg = NodeKey::new(A, HObject(0x5c00_0012));
    let ctx = NodeKey::new(A, HObject(0x5c00_0020));
    ev.push(alloc(A, tsg.handle, ctx.handle, nv::FERMI_CONTEXT_SHARE_A));
    let mut g = graph_of(&ev);
    let before = g.nodes().count();
    g.apply(free(A, tsg.handle)).expect("the guest frees its TSG");
    assert!(g.node(tsg).is_none() && g.node(ctx).is_none(), "the subtree went");
    assert!(g.node(vas).is_some(), "a sibling stayed");
    assert!(g.node(NodeKey::new(A, HObject(0x5c00_0001))).is_some(), "the parent stayed");
    assert_eq!(g.nodes().count(), before - 2);
    assert_eq!(g.apply(free(A, tsg.handle)), Err(RmGraphError::FreeUnknown(tsg)), "double free is named");
}

/// G9 — a Device naming a GPU instance the device was not realized with is refused.
#[test]
fn a_device_instance_outside_the_entitlement_is_refused() {
    let mut g = RmGraph::new(Family::Ampere);
    g.apply(client_root(A)).expect("A");
    let dev1 = RmEvent::Alloc {
        client: A,
        parent: HObject(A.0),
        handle: HObject(0x10),
        class: ClassId(nv::NV01_DEVICE_0),
        facts: AllocFacts { device_instance: Some(1), ..Default::default() },
    };
    assert_eq!(g.apply(dev1), Err(RmGraphError::InvalidDeviceInstance { instance: 1 }));
    g.entitle([0, 1]);
    g.apply(dev1).expect("entitled now");
}

// ---------------------------------------------------------------------------------------
// Ported: fuzz_rmgraph_invariants.rs a4 — DUP_OBJECT is reference counted
// ---------------------------------------------------------------------------------------

/// Independent reference-count model (the old `RefTracker`, minus the cut address-plane
/// events, plus the v3 engine-class gate).
#[derive(Default)]
struct RefTracker {
    handle_to_origin: BTreeMap<(HClient, HObject), (HClient, HObject)>,
    client_roots: BTreeSet<(HClient, HObject)>,
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

    fn live_resources(&self) -> BTreeSet<(HClient, HObject)> {
        self.handle_to_origin.values().copied().collect()
    }

    fn promote_pending(&mut self) {
        loop {
            let ready = self.pending.iter().find_map(|(dst, src)| {
                (!self.handle_to_origin.contains_key(dst)).then(|| self.origin_of(*src).map(|o| (*dst, o))).flatten()
            });
            let Some((dst, origin)) = ready else { break };
            self.pending.remove(&dst);
            self.handle_to_origin.insert(dst, origin);
        }
    }

    fn apply(&mut self, g: &RmGraph, ev: RmEvent) {
        match ev {
            RmEvent::Alloc { client, handle, class, .. } => {
                if g.engine_class_refusal(class).is_some() {
                    return;
                }
                let k = (client, handle);
                let is_client = matches!(g.classify(class), ObjectKind::Client);
                let declared = self.client_roots.iter().any(|r| r.0 == client);
                if (!is_client && !declared)
                    || self.handle_to_origin.contains_key(&k)
                    || self.pending.contains_key(&k)
                    || (is_client && declared)
                {
                    return;
                }
                self.handle_to_origin.insert(k, k);
                if is_client {
                    self.client_roots.insert(k);
                }
                self.promote_pending();
            }
            RmEvent::Dup { src, dst } => {
                let (src, dst) = ((src.client, src.handle), (dst.client, dst.handle));
                if [dst.0, src.0].iter().any(|c| !self.client_roots.iter().any(|r| r.0 == *c)) {
                    return;
                }
                if self.handle_to_origin.contains_key(&dst) || self.pending.contains_key(&dst) {
                    return;
                }
                match self.origin_of(src) {
                    Some(origin) => {
                        self.handle_to_origin.insert(dst, origin);
                        self.promote_pending();
                    }
                    None => {
                        self.pending.insert(dst, src);
                    }
                }
            }
            RmEvent::Free { client, handle } => {
                let key = (client, handle);
                let is_root = self.client_roots.contains(&key);
                if !self.handle_to_origin.contains_key(&key) && !self.pending.contains_key(&key) && !is_root {
                    return;
                }
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
                if is_root {
                    self.pending.retain(|d, s| d.0 != client && s.0 != client);
                }
            }
        }
    }
}

/// Deterministic xorshift — the old proptest stream generator without the dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Old `any_class`, on real class ids: root, device, VA space, TSG, context share, one
/// engine class of EVERY family (so the family gate is exercised), and junk.
fn any_class(r: &mut Rng) -> ClassId {
    let engines: Vec<u32> = kf_chip::classes::FAMILIES.iter().map(|c| c.compute[0]).collect();
    match r.below(8) {
        0 => ClassId(nv::NV01_ROOT_CLIENT),
        1 => ClassId(nv::NV01_DEVICE_0),
        2 => ClassId(nv::FERMI_VASPACE_A),
        3 => ClassId(nv::KEPLER_CHANNEL_GROUP_A),
        4 => ClassId(nv::FERMI_CONTEXT_SHARE_A),
        5 => ClassId(engines[r.below(engines.len() as u64) as usize]),
        _ => ClassId(r.below(0x1_0000) as u32),
    }
}

/// Old `a4_dup_object_is_reference_counted` (3000 proptest cases), deterministically.
#[test]
fn a4_dup_object_is_reference_counted() {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    for case in 0..3000 {
        let mut g = RmGraph::new(Family::ALL[case % Family::ALL.len()]);
        let mut tracker = RefTracker::default();
        let client = |r: &mut Rng| HClient(0xB000 + r.below(3) as u32);
        let handle = |r: &mut Rng| HObject(0x7000_0000 + r.below(8) as u32);
        let mut next = 0x7000_0000u32;
        let len = r.below(30);
        for _ in 0..len {
            let ev = match r.below(3) {
                0 => {
                    let c = client(&mut r);
                    let h = HObject(next);
                    next += 1;
                    RmEvent::Alloc { client: c, parent: h, handle: h, class: any_class(&mut r), facts: user(c) }
                }
                1 => dup(NodeKey::new(client(&mut r), handle(&mut r)), NodeKey::new(client(&mut r), handle(&mut r))),
                _ => free(client(&mut r), handle(&mut r)),
            };
            let _ = g.apply(ev); // never panics — at worst a named refusal
            tracker.apply(&g, ev);

            let graph_live: BTreeSet<(HClient, HObject)> = g.nodes().map(|n| (n.key.client, n.key.handle)).collect();
            assert_eq!(graph_live, tracker.live_resources(), "case {case}: graph vs independent tracker");
            for origin in &graph_live {
                let key = NodeKey::new(origin.0, origin.1);
                let refs: Vec<_> = g.references(key).collect();
                assert!(!refs.is_empty(), "case {case}: a live resource must have ≥1 reference");
                for h in refs {
                    assert_eq!(g.origin_of(h).map(|n| n.key), Some(key), "case {case}");
                }
            }
            for c in 0..3u32 {
                for h in 0..8u32 {
                    let k = NodeKey::new(HClient(0xB000 + c), HObject(0x7000_0000 + h));
                    if tracker.origin_of((k.client, k.handle)).is_none() {
                        assert!(g.origin_of(k).is_none(), "case {case}: a stale handle resolved");
                    }
                }
            }
        }
        let remaining: Vec<NodeKey> = tracker.handle_to_origin.keys().map(|(c, h)| NodeKey::new(*c, *h)).collect();
        for k in remaining {
            let _ = g.apply(free(k.client, k.handle));
        }
        assert_eq!(g.nodes().count(), 0, "case {case}: no resource leaks after every handle is freed");
    }
}

// ---------------------------------------------------------------------------------------
// NEW: the three doorbell-graph defects (V3_P2_PORT_MAP.md §4.4), proved absent
// ---------------------------------------------------------------------------------------

/// §4.4 defect 1 — the doorbell graph keyed nodes on a bare `HandleId`. Here the same
/// `hObject` in two clients is two nodes, and freeing one leaves the other.
#[test]
fn defect1_the_same_hobject_in_two_clients_is_two_nodes() {
    let mut g = RmGraph::new(Family::Ampere);
    g.apply(client_root(A)).expect("A");
    g.apply(client_root(B)).expect("B");
    let h = HObject(0x5c00_0003);
    g.apply(alloc(A, HObject(A.0), h, nv::NV01_DEVICE_0)).expect("A's device");
    g.apply(alloc(B, HObject(B.0), h, nv::NV01_DEVICE_0)).expect("★ B's device at the SAME value is not a duplicate");
    assert_ne!(g.node(NodeKey::new(A, h)).map(|n| n.id()), g.node(NodeKey::new(B, h)).map(|n| n.id()));
    g.apply(free(A, h)).expect("A frees its device");
    assert!(g.node(NodeKey::new(A, h)).is_none());
    assert_eq!(g.node(NodeKey::new(B, h)).map(|n| n.key), Some(NodeKey::new(B, h)), "B's survives");
}

/// §4.4 defect 2 — the doorbell graph's `alloc` called the family-BLIND class table. Here,
/// for EVERY family: every engine class the family lists is admitted, and every engine class
/// only OTHER families list is refused by name. Quantified over `FAMILIES`, never hand-listed.
#[test]
fn defect2_the_engine_class_gate_is_per_family_for_every_family() {
    let all: BTreeSet<u32> = kf_chip::classes::FAMILIES
        .iter()
        .flat_map(|c| Kind::ALL.iter().flat_map(move |k| c.of_kind(*k).iter().copied()))
        .collect();
    for family in Family::ALL {
        let mut g = RmGraph::new(family);
        g.apply(client_root(A)).expect("A");
        let dev = HObject(0x10);
        g.apply(alloc(A, HObject(A.0), dev, nv::NV01_DEVICE_0)).expect("device");
        let mut next = 0x1000u32;
        let (mut admitted, mut refused) = (0, 0);
        for &class in &all {
            let h = HObject(next);
            next += 1;
            let got = g.apply(alloc(A, dev, h, class));
            if family.classes().kind_of(class).is_some() {
                assert_eq!(got, Ok(()), "{family:?} lists {class:#x} and must admit it");
                assert!(g.node(NodeKey::new(A, h)).is_some());
                admitted += 1;
            } else {
                assert_eq!(
                    got,
                    Err(RmGraphError::EngineClassNotInFamily { class: ClassId(class), family }),
                    "{family:?} does not list {class:#x}"
                );
                assert!(g.node(NodeKey::new(A, h)).is_none(), "a refused alloc left a node");
                refused += 1;
            }
        }
        assert!(admitted > 0 && refused > 0, "{family:?}: non-vacuous ({admitted} admitted, {refused} refused)");
        // Non-engine classes are not this gate's business.
        assert_eq!(g.engine_class_refusal(ClassId(nv::FERMI_VASPACE_A)), None);
    }
}

/// §4.4 defect 3 — the doorbell graph's `dup` copied the source's refcount into the
/// destination. Here a dup is an ALIAS of one resource: no second node, the resource lives
/// while either handle does, and dies when both are freed — in either free order.
#[test]
fn defect3_dup_aliases_one_resource_it_never_copies_a_count() {
    for src_first in [true, false] {
        let mut g = RmGraph::new(Family::Ampere);
        g.apply(client_root(A)).expect("A");
        g.apply(client_root(B)).expect("B");
        let src = NodeKey::new(A, HObject(0x20));
        let dst = NodeKey::new(B, HObject(0x30));
        g.apply(alloc(A, HObject(A.0), src.handle, nv::FERMI_VASPACE_A)).expect("vas");
        let before = g.nodes().count();
        g.apply(dup(src, dst)).expect("dup");
        assert_eq!(g.nodes().count(), before, "a dup creates no second resource");
        assert_eq!(g.origin_of(dst).map(|n| n.id()), g.origin_of(src).map(|n| n.id()), "one resource");
        assert_eq!(g.references(src).collect::<BTreeSet<_>>(), BTreeSet::from([src, dst]));

        let (first, second) = if src_first { (src, dst) } else { (dst, src) };
        g.apply(free(first.client, first.handle)).expect("first free");
        assert_eq!(g.nodes().count(), before, "alive through the other handle");
        assert_eq!(g.origin_of(second).map(|n| n.key), Some(src));
        assert_eq!(g.references(src).collect::<Vec<_>>(), vec![second], "exactly one reference left");
        g.apply(free(second.client, second.handle)).expect("second free");
        assert_eq!(g.nodes().count(), before - 1, "dies with its LAST reference, not before, not after");
        assert!(g.origin_of(src).is_none() && g.origin_of(dst).is_none());
    }
}

// ---------------------------------------------------------------------------------------
// NEW: classification is kf_chip's, for every family
// ---------------------------------------------------------------------------------------

#[test]
fn classification_is_kf_chips_for_every_family() {
    for set in kf_chip::classes::FAMILIES.iter() {
        let g = RmGraph::new(set.family);
        for &c in set.compute {
            assert_eq!(g.classify(ClassId(c)), ObjectKind::EngineObject { engine: EngineKind::GrCompute });
        }
        for &c in set.dma_copy {
            assert_eq!(g.classify(ClassId(c)), ObjectKind::EngineObject { engine: EngineKind::Ce });
        }
        for &c in set.threed {
            assert_eq!(g.classify(ClassId(c)), ObjectKind::EngineObject { engine: EngineKind::GrGraphics });
        }
        for &c in set.channel_gpfifo {
            assert_eq!(g.classify(ClassId(c)), ObjectKind::Channel { engine: EngineKind::GrCompute });
        }
        for c in 0..=0xffffu32 {
            assert_eq!(g.classify(ClassId(c)), set.classify(ClassId(c)), "{:?} {c:#x}", set.family);
        }
        assert_eq!(g.classify(ClassId(nv::NV01_ROOT_CLIENT)), ObjectKind::Client);
        assert_eq!(g.classify(ClassId(nv::NV01_DEVICE_0)), ObjectKind::Device);
        // A compute class of another family is not an engine object on THIS family.
        for other in kf_chip::classes::FAMILIES.iter().filter(|o| o.family != set.family) {
            for &c in other.compute.iter().filter(|c| set.kind_of(**c).is_none()) {
                assert_eq!(g.classify(ClassId(c)), ObjectKind::Unknown, "{:?} {c:#x}", set.family);
            }
        }
    }
}
