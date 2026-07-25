//! UNIT: RmGraph derivation is **order-independent** (decision #14, arch §4.3.1a;
//! the protocol-not-observed-order guarantee, decision #4).
//!
//! Build the graph from a sequence of `RmEvent`s, derive `by_pdb`/`by_vchid`/`Proc`
//! grouping, then **shuffle the event order** deterministically and assert the derived
//! boundaries are IDENTICAL. This is the executable statement that a reordered or
//! retried guest yields the same process/VAS/channel boundaries — the property #14
//! needs and the property the C's order-accreted routing lacked.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use kayfabe_arch::ids::GpuId;
use kayfabe_arch::ids::{HClient, HObject, Pdb, VChid};
use kayfabe_core::project::project;
use kayfabe_core::rmgraph::RmGraph;
use kayfabe_mocks::MockArch;
use kayfabe_tests::{Scenario, identical_handles};

/// Deterministic index permutations of `n` items (no RNG — reproducible).
/// A rotation family plus the reverse: enough distinct orders to catch any
/// order dependence without a fuzzing dependency.
fn permutations(n: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    for shift in 0..n {
        out.push((0..n).map(|i| (i + shift) % n).collect());
    }
    out.push((0..n).rev().collect());
    // An interleave (evens then odds) — a genuinely different order.
    let mut il: Vec<usize> = (0..n).step_by(2).collect();
    il.extend((1..n).step_by(2));
    out.push(il);
    out
}

/// Build the canonical two-process + UVM scenario used across these tests.
fn scenario() -> Scenario {
    let mut s = Scenario::new();
    // Process A: compute client + UVM client dup'ing A's compute VASpace.
    let a_vas = s.compute_process(
        HClient(0xAA),
        Pdb(0x3401_000),
        identical_handles(0x10, 0x11),
    );
    s.uvm_dup(
        HClient(0xA1),
        HObject(0x9000_0000),
        HObject(0x9000_0001),
        HObject(0x9000_0010),
        Pdb(0x2efa_6c000), // UVM's own VAS PDB (distinct from compute)
        HObject(0x9000_00ff),
        a_vas,
    );
    // Process B: IDENTICAL guest handles, IDENTICAL guest VAs would follow, DISTINCT
    // PDB, distinct vChids (E0).
    let b_vas = s.compute_process(
        HClient(0xBB),
        Pdb(0x3405_000),
        identical_handles(0x20, 0x21),
    );
    s.uvm_dup(
        HClient(0xB1),
        HObject(0x8000_0000),
        HObject(0x8000_0001),
        HObject(0x8000_0010),
        Pdb(0x2eff_00000),
        HObject(0x8000_00ff),
        b_vas,
    );
    s
}

#[test]
fn by_pdb_by_vchid_and_proc_grouping_are_order_independent() {
    let arch = MockArch::new();
    let events = scenario().events;

    // Reference derivation from the scripted order.
    let reference = {
        let mut g = RmGraph::new();
        for &ev in &events {
            g.apply(&arch, ev).expect("scripted events are valid");
        }
        project(&g, &arch).expect("projects cleanly")
    };

    // Every permutation must yield byte-identical boundaries.
    for perm in permutations(events.len()) {
        let mut g = RmGraph::new();
        for &i in &perm {
            g.apply(&arch, events[i])
                .expect("same events, any order, still valid");
        }
        let derived = project(&g, &arch).expect("projects cleanly in any order");
        assert_eq!(
            derived, reference,
            "derived boundaries must be independent of event order (perm {perm:?})"
        );
    }
}

#[test]
fn dup_edge_groups_uvm_and_compute_into_one_proc() {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    for ev in scenario().events {
        g.apply(&arch, ev).unwrap();
    }
    let b = project(&g, &arch).unwrap();

    // Exactly two processes (A+its UVM, B+its UVM) despite four clients.
    assert_eq!(b.procs.len(), 2, "two dup-connected components");
    for p in &b.procs {
        assert_eq!(
            p.clients.len(),
            2,
            "each proc = compute client + UVM client (dup-joined)"
        );
    }
}

#[test]
fn multi_vaspace_per_process_keys_address_ops_on_vas_not_proc() {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    for ev in scenario().events {
        g.apply(&arch, ev).unwrap();
    }
    let b = project(&g, &arch).unwrap();

    // Process A owns TWO VASpaces (compute PDB + UVM PDB) — one Proc, two Vas.
    let proc_a = b
        .procs
        .iter()
        .find(|p| p.clients.contains(&HClient(0xAA)))
        .expect("proc A present");
    let pdbs: Vec<Pdb> = proc_a.vases.values().filter_map(|f| f.pdb).collect();
    assert!(pdbs.contains(&Pdb(0x3401_000)), "compute VAS present");
    assert!(pdbs.contains(&Pdb(0x2efa_6c000)), "UVM VAS present");
    assert_eq!(pdbs.len(), 2, "exactly two VASes under one Proc");

    // by_pdb keys on (target, PDB), and each PDB routes to A on GpuId::ZERO — address
    // ops key on Vas/PDB, never on Proc (the multi-VAS-per-proc rule, decision #14).
    assert_eq!(
        b.by_pdb.get(&(GpuId::ZERO, Pdb(0x3401_000))).map(|x| x.0),
        Some(proc_a.anchor)
    );
    assert_eq!(
        b.by_pdb.get(&(GpuId::ZERO, Pdb(0x2efa_6c000))).map(|x| x.0),
        Some(proc_a.anchor)
    );
}

/// ★ Mutation-gate kill (`ClientUnion::union` `ra < rb`→`ra == rb`): the process
/// anchor is the DETERMINISTIC **minimum** client handle in the dup-connected component
/// (`ProcBoundary::anchor` doc; rule L7 — a stable, order-independent identity, never a
/// minted id). The union must therefore make the SMALLER root the representative; the
/// `<`→`==` mutant (equality is already handled by the early return, so it always takes
/// the `else` branch) picks the OTHER root, yielding a non-minimum anchor. The prior
/// suite asserted the anchor was used consistently but never pinned its VALUE, so the
/// mutant survived. Here we assert, for every component, `anchor == min(clients)`.
#[test]
fn proc_anchor_is_the_minimum_client_in_its_component() {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    for ev in scenario().events {
        g.apply(&arch, ev).unwrap();
    }
    let b = project(&g, &arch).unwrap();
    assert_eq!(b.procs.len(), 2);
    for p in &b.procs {
        let min_client = p
            .clients
            .iter()
            .min()
            .copied()
            .expect("component non-empty");
        assert_eq!(
            p.anchor.0, min_client,
            "the anchor must be the SMALLEST client handle in the component (deterministic identity)",
        );
        // The routing maps must agree with that same minimum-anchor for this proc's PDBs.
        for pdb in p.vases.values().filter_map(|f| f.pdb) {
            assert_eq!(
                b.by_pdb.get(&(GpuId::ZERO, pdb)).map(|x| x.0),
                Some(p.anchor),
                "by_pdb routes to the minimum-client anchor",
            );
        }
    }
    // Concretely: proc A's clients are {0xA1, 0xAA}; its anchor is 0xA1 (the smaller).
    let proc_a = b
        .procs
        .iter()
        .find(|p| p.clients.contains(&HClient(0xAA)))
        .unwrap();
    assert_eq!(
        proc_a.anchor.0,
        HClient(0xA1),
        "0xA1 < 0xAA, so 0xA1 anchors proc A"
    );
}

#[test]
fn identical_handles_across_procs_do_not_collide() {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    for ev in scenario().events {
        g.apply(&arch, ev).unwrap();
    }
    let b = project(&g, &arch).unwrap();

    // Both procs used GR channel handle 0x5c000019 — but they are distinct nodes
    // keyed by (client, handle), so the two GR channels route to distinct vChids.
    let gr_a = b
        .by_vchid
        .get(&(GpuId::ZERO, VChid(0x10)))
        .expect("A's GR vchid");
    let gr_b = b
        .by_vchid
        .get(&(GpuId::ZERO, VChid(0x20)))
        .expect("B's GR vchid");
    assert_ne!(gr_a.0, gr_b.0, "identical handles, distinct procs");
    // Four distinct channels total (2 per proc), zero vChid collisions.
    assert_eq!(b.by_vchid.len(), 4);
}
