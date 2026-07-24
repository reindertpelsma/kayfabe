//! UNIT: RmGraph derivation is **order-independent** (decision #14, arch §4.3.1a;
//! the protocol-not-observed-order guarantee, decision #4).
//!
//! Build the graph from a sequence of `RmEvent`s, derive `by_pdb`/`by_vchid`/`Proc`
//! grouping, then **shuffle the event order** deterministically and assert the derived
//! boundaries are IDENTICAL. This is the executable statement that a reordered or
//! retried guest yields the same process/VAS/channel boundaries — the property #14
//! needs and the property the C's order-accreted routing lacked.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use nvkvm_arch::ids::{HClient, HObject, Pdb, VChid};
use nvkvm_core::project::project;
use nvkvm_core::rmgraph::RmGraph;
use nvkvm_mocks::MockArch;
use nvkvm_tests::{Scenario, identical_handles};

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
    let a_vas = s.compute_process(HClient(0xAA), Pdb(0x3401_000), identical_handles(0x10, 0x11));
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
    let b_vas = s.compute_process(HClient(0xBB), Pdb(0x3405_000), identical_handles(0x20, 0x21));
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
            g.apply(&arch, events[i]).expect("same events, any order, still valid");
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
        assert_eq!(p.clients.len(), 2, "each proc = compute client + UVM client (dup-joined)");
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
    let pdbs: Vec<Pdb> = proc_a.vases.values().flatten().copied().collect();
    assert!(pdbs.contains(&Pdb(0x3401_000)), "compute VAS present");
    assert!(pdbs.contains(&Pdb(0x2efa_6c000)), "UVM VAS present");
    assert_eq!(pdbs.len(), 2, "exactly two VASes under one Proc");

    // by_pdb keys on the VAS (PDB), and each PDB routes to A — address ops key on
    // Vas/PDB, never on Proc (the multi-VAS-per-proc rule, decision #14).
    assert_eq!(b.by_pdb.get(&Pdb(0x3401_000)).map(|x| x.0), Some(proc_a.anchor));
    assert_eq!(b.by_pdb.get(&Pdb(0x2efa_6c000)).map(|x| x.0), Some(proc_a.anchor));
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
    let gr_a = b.by_vchid.get(&VChid(0x10)).expect("A's GR vchid");
    let gr_b = b.by_vchid.get(&VChid(0x20)).expect("B's GR vchid");
    assert_ne!(gr_a.0, gr_b.0, "identical handles, distinct procs");
    // Four distinct channels total (2 per proc), zero vChid collisions.
    assert_eq!(b.by_vchid.len(), 4);
}
