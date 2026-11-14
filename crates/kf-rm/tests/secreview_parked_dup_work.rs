//! Security review 1 (2026-11-13), F-11: after a guest process parks many dangling `DUP_OBJECT`s
//! (sources that never arrive; up to `MAX_PARKED` = 2^18), every later RM alloc or dup of EVERY
//! process rescanned the whole parked table on the register drainer, under the GSP mutex
//! (`RmGraph::resolve_pending_dups`'s linear `find_map`), and every free re-walked it too. The
//! drainer may never block (owner rule).
//!
//! Falsifier: with K parked dangling dups, M unrelated allocs/dups/frees examine more than a small
//! constant times M parked entries. Measured by the graph's own work counter
//! (`RmGraph::parked_dup_work`), not by a clock, so the result is deterministic.
//!
//! The order-tolerance behaviour the table exists for (a dup before its source, chains, pruning on
//! free) is pinned here too, next to the bound, so an index cannot buy speed by dropping it.
use kf_abi::generated::classes as nv;
use kf_arch::ClientKind;
use kf_arch::ids::{ClassId, HClient, HObject};
use kf_chip::Family;
use kf_rm::rmgraph::{AllocFacts, NodeKey, RmEvent, RmGraph};

const A: HClient = HClient(0xc1d0_0067);
const B: HClient = HClient(0xc1d0_0068);
const DEV: HObject = HObject(0x10);

fn root(c: HClient) -> RmEvent {
    RmEvent::Alloc {
        client: c,
        parent: HObject(c.0),
        handle: HObject(c.0),
        class: ClassId(nv::NV01_ROOT_CLIENT),
        facts: AllocFacts {
            client_kind: Some(ClientKind::User { pid: c.0 }),
            ..Default::default()
        },
    }
}

fn alloc(c: HClient, parent: HObject, handle: HObject, class: u32) -> RmEvent {
    let facts = if class == nv::NV01_DEVICE_0 {
        AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        }
    } else {
        AllocFacts::default()
    };
    RmEvent::Alloc {
        client: c,
        parent,
        handle,
        class: ClassId(class),
        facts,
    }
}

fn key(c: HClient, h: u32) -> NodeKey {
    NodeKey::new(c, HObject(h))
}

fn dup(src: NodeKey, dst: NodeKey) -> RmEvent {
    RmEvent::Dup { src, dst }
}

fn free(c: HClient, h: u32) -> RmEvent {
    RmEvent::Free {
        client: c,
        handle: HObject(h),
    }
}

/// Two declared clients, each with a device.
fn base() -> RmGraph {
    let mut g = RmGraph::new(Family::Ada);
    for c in [A, B] {
        g.apply(root(c)).unwrap();
        g.apply(alloc(c, HObject(c.0), DEV, nv::NV01_DEVICE_0))
            .unwrap();
    }
    g
}

/// `k` dangling dups parked by client A (their sources are never allocated).
fn flood(g: &mut RmGraph, k: u32) {
    for i in 0..k {
        g.apply(dup(key(A, 0x2000_0000 + i), key(A, 0x3000_0000 + i)))
            .unwrap();
    }
}

const K: u32 = 20_000;
const M: u32 = 100;
/// Entries a single unrelated event may examine: a constant, independent of `K`.
const PER_EVENT: u64 = 8;

#[test]
fn unrelated_allocs_after_a_dup_flood_do_bounded_parked_work() {
    let mut g = base();
    flood(&mut g, K);
    let before = g.parked_dup_work();
    for i in 0..M {
        g.apply(alloc(B, DEV, HObject(0x4000_0000 + i), nv::FERMI_VASPACE_A))
            .unwrap();
    }
    let work = g.parked_dup_work() - before;
    assert!(g.parked_dup_index_is_exact());
    assert!(
        work <= u64::from(M) * PER_EVENT,
        "{M} allocs examined {work} parked entries with {K} parked: the work grows with the flood"
    );
}

#[test]
fn unrelated_resolved_dups_after_a_dup_flood_do_bounded_parked_work() {
    let mut g = base();
    g.apply(alloc(B, DEV, HObject(0x50), nv::FERMI_VASPACE_A))
        .unwrap();
    flood(&mut g, K);
    let before = g.parked_dup_work();
    for i in 0..M {
        g.apply(dup(key(B, 0x50), key(B, 0x6000_0000 + i))).unwrap();
    }
    let work = g.parked_dup_work() - before;
    assert!(g.parked_dup_index_is_exact());
    assert!(
        work <= u64::from(M) * PER_EVENT,
        "{M} dups examined {work} parked entries with {K} parked"
    );
}

#[test]
fn unrelated_frees_after_a_dup_flood_do_bounded_parked_work() {
    let mut g = base();
    for i in 0..M {
        g.apply(alloc(B, DEV, HObject(0x4000_0000 + i), nv::FERMI_VASPACE_A))
            .unwrap();
    }
    flood(&mut g, K);
    let before = g.parked_dup_work();
    for i in 0..M {
        g.apply(free(B, 0x4000_0000 + i)).unwrap();
    }
    let work = g.parked_dup_work() - before;
    assert!(g.parked_dup_index_is_exact());
    assert!(
        work <= u64::from(M) * PER_EVENT,
        "{M} frees examined {work} parked entries with {K} parked"
    );
}

#[test]
fn a_parked_chain_resolves_when_its_far_end_arrives() {
    let mut g = base();
    // c -> b -> a, all parked (a not allocated yet), declared in reverse order.
    g.apply(dup(key(A, 0xb), key(B, 0xc))).unwrap();
    g.apply(dup(key(A, 0xa), key(A, 0xb))).unwrap();
    assert!(g.node(key(B, 0xc)).is_none(), "parked, not live");
    g.apply(alloc(A, DEV, HObject(0xa), nv::FERMI_VASPACE_A))
        .unwrap();
    let a = g.node(key(A, 0xa)).expect("a is live").id();
    assert_eq!(g.node(key(A, 0xb)).expect("b promoted").id(), a);
    assert_eq!(g.node(key(B, 0xc)).expect("c promoted through b").id(), a);
    // Both promoted edges are real references: freeing the origin keeps the resource.
    g.apply(free(A, 0xa)).unwrap();
    assert_eq!(
        g.node(key(B, 0xc)).expect("kept alive by the alias").id(),
        a
    );
}

#[test]
fn a_parked_chain_resolves_when_a_dup_mints_its_middle() {
    let mut g = base();
    g.apply(alloc(A, DEV, HObject(0xa), nv::FERMI_VASPACE_A))
        .unwrap();
    // c parked against b; b is then minted by a resolved dup of a.
    g.apply(dup(key(A, 0xb), key(B, 0xc))).unwrap();
    assert!(g.node(key(B, 0xc)).is_none());
    g.apply(dup(key(A, 0xa), key(A, 0xb))).unwrap();
    let a = g.node(key(A, 0xa)).unwrap().id();
    assert_eq!(g.node(key(B, 0xc)).expect("promoted").id(), a);
}

#[test]
fn freeing_a_parked_dst_or_its_named_source_prunes_the_edge() {
    let mut g = base();
    g.apply(dup(key(A, 0xa), key(A, 0xb))).unwrap();
    g.apply(dup(key(A, 0xa2), key(B, 0xb2))).unwrap();
    // Free the parked dst: the edge is gone, a later alloc of the source promotes nothing.
    g.apply(free(A, 0xb)).unwrap();
    g.apply(alloc(A, DEV, HObject(0xa), nv::FERMI_VASPACE_A))
        .unwrap();
    assert!(g.node(key(A, 0xb)).is_none(), "a pruned edge never fires");
    // The other edge still fires when its source arrives.
    g.apply(alloc(A, DEV, HObject(0xa2), nv::FERMI_VASPACE_A))
        .unwrap();
    assert!(g.node(key(B, 0xb2)).is_some());
    assert_eq!(g.dups().count(), 1);
    assert!(g.parked_dup_index_is_exact());
}

#[test]
fn freeing_the_middle_of_a_parked_chain_prunes_the_edge_that_names_it() {
    let mut g = base();
    // c -> b -> a, all parked; b (a parked dst) is freed: both edges go (b's own, and c's,
    // whose source b now names nothing).
    g.apply(dup(key(A, 0xa), key(A, 0xb))).unwrap();
    g.apply(dup(key(A, 0xb), key(B, 0xc))).unwrap();
    g.apply(free(A, 0xb)).unwrap();
    assert!(g.parked_dup_index_is_exact());
    assert_eq!(g.dups().count(), 0);
    g.apply(alloc(A, DEV, HObject(0xa), nv::FERMI_VASPACE_A))
        .unwrap();
    assert!(g.node(key(A, 0xb)).is_none());
    assert!(g.node(key(B, 0xc)).is_none(), "a pruned chain never fires");
    assert!(g.parked_dup_index_is_exact());
}

#[test]
fn a_root_free_prunes_parked_edges_naming_its_namespace_at_either_end() {
    let mut g = base();
    g.apply(dup(key(A, 0xa), key(B, 0xb))).unwrap(); // src in A
    g.apply(dup(key(B, 0xa3), key(A, 0xb3))).unwrap(); // dst in A
    g.apply(dup(key(B, 0xa4), key(B, 0xb4))).unwrap(); // not A
    g.apply(free(A, A.0)).unwrap();
    let left: Vec<_> = g.dups().collect();
    assert_eq!(left, vec![(key(B, 0xb4), key(B, 0xa4))]);
    // Redeclare A and allocate the old source: nothing fires into B.
    g.apply(root(A)).unwrap();
    g.apply(alloc(A, HObject(A.0), DEV, nv::NV01_DEVICE_0))
        .unwrap();
    g.apply(alloc(A, DEV, HObject(0xa), nv::FERMI_VASPACE_A))
        .unwrap();
    assert!(g.parked_dup_index_is_exact());
    assert!(
        g.node(key(B, 0xb)).is_none(),
        "§12.39: no alias into B from a dead namespace"
    );
}

#[test]
fn a_three_link_parked_chain_resolves_whole() {
    let mut g = base();
    // d -> c -> b -> a, declared far end last.
    g.apply(dup(key(B, 0xc), key(B, 0xd))).unwrap();
    g.apply(dup(key(A, 0xb), key(B, 0xc))).unwrap();
    g.apply(dup(key(A, 0xa), key(A, 0xb))).unwrap();
    g.apply(alloc(A, DEV, HObject(0xa), nv::FERMI_VASPACE_A))
        .unwrap();
    let a = g.node(key(A, 0xa)).unwrap().id();
    for (c, h) in [(A, 0xb), (B, 0xc), (B, 0xd)] {
        assert_eq!(g.node(key(c, h)).expect("promoted").id(), a, "{h:#x}");
    }
    assert_eq!(g.dups().count(), 3);
    assert!(g.parked_dup_index_is_exact());
}

#[test]
fn a_pruned_chain_link_frees_its_dst_for_a_new_declaration() {
    let mut g = base();
    g.apply(dup(key(A, 0xa), key(A, 0xb))).unwrap();
    g.apply(dup(key(A, 0xb), key(B, 0xc))).unwrap();
    g.apply(free(A, 0xb)).unwrap();
    // c's edge died with b, so c may now be parked against another source without a conflict.
    g.apply(dup(key(A, 0xe), key(B, 0xc)))
        .expect("c is free again: its old edge was pruned");
    g.apply(alloc(A, DEV, HObject(0xe), nv::FERMI_VASPACE_A))
        .unwrap();
    let e = g.node(key(A, 0xe)).unwrap().id();
    assert_eq!(g.node(key(B, 0xc)).expect("the new edge fired").id(), e);
    assert!(g.parked_dup_index_is_exact());
}

#[test]
fn a_dup_minted_source_promotes_every_edge_waiting_on_it() {
    let mut g = base();
    g.apply(alloc(A, DEV, HObject(0xa), nv::FERMI_VASPACE_A))
        .unwrap();
    g.apply(dup(key(A, 0xb), key(B, 0xc1))).unwrap();
    g.apply(dup(key(A, 0xb), key(B, 0xc2))).unwrap();
    g.apply(dup(key(A, 0xa), key(A, 0xb))).unwrap();
    let a = g.node(key(A, 0xa)).unwrap().id();
    // Real references: the origin and the middle can go, the two aliases keep the resource.
    g.apply(free(A, 0xa)).unwrap();
    g.apply(free(A, 0xb)).unwrap();
    assert_eq!(g.node(key(B, 0xc1)).expect("kept alive").id(), a);
    assert_eq!(g.node(key(B, 0xc2)).expect("kept alive").id(), a);
    assert!(g.parked_dup_index_is_exact());
}
