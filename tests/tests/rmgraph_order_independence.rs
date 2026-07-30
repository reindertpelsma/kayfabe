//! UNIT: RmGraph derivation is **order-independent** (decision #14, arch §4.3.1a;
//! the protocol-not-observed-order guarantee, decision #4), and — ★ `l1_concurrency.md`
//! §12.27 — it groups clients by the rule the **hardware measurement** supports.
//!
//! Build the graph from a sequence of `RmEvent`s, derive `by_pdb`/`by_vchid`/`Proc`
//! grouping, then **shuffle the event order** deterministically and assert the derived
//! boundaries are IDENTICAL. This is the executable statement that a reordered or
//! retried guest yields the same process/VAS/channel boundaries — the property #14
//! needs and the property the C's order-accreted routing lacked.
//!
//! ## ★ What this file used to assert, and why it was fiction
//!
//! The scenario gave process A and process B **one UVM client each**
//! (`HClient(0xA1)`, `HClient(0xB1)`), and
//! `dup_edge_groups_uvm_and_compute_into_one_proc` asserted "A + its UVM, B + its UVM".
//! **That shape cannot occur.** `nvUvmInterfaceSessionCreate` fires exactly once per
//! `nvidia_uvm` module load; measured on an RTX 3060 / driver 580.159.04 with kprobes on
//! `rmapiDupObjectWithSecInfo` and `rpcRmApiDupObject_GSP`, two concurrent CUDA processes
//! issued 82 dups **each**, every single one with the same destination `0xc1d00069`, and
//! a third process started later joined that same destination. Dups with the session as
//! *source*: 0. Dups with a user client as *destination*: 0. Userspace
//! `NV_ESC_RM_DUP_OBJECT`: 0.
//!
//! So the scenario below is the measured one: **A, B, and ONE shared kernel client both
//! dup into** — with the client handles taken from the measurement, `0xc1d00067` (A),
//! `0xc1d00068` (B) and the UVM session `0xc1d00069` sitting numerically *between* them,
//! because the handle value is not, and must never become, the discriminator.
//!
//! Process A additionally holds a **second user client** joined to it by a `DUP_OBJECT`.
//! That is the other half of the rule — a user↔user dup is genuine sharing, hence one
//! blast radius, hence one `Proc` — and it keeps the union-find path (and its
//! minimum-anchor tie-break) under test now that the UVM edge no longer merges.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use kayfabe_arch::ids::GpuId;
use kayfabe_arch::ids::{GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, GpuError};
use kayfabe_core::project::{NO_CONDEMNED, SYSTEM_ANCHOR, project};
use kayfabe_core::rmgraph::{ClientKey, RmEvent, RmGraph};
use kayfabe_fwd::{FwdFault, handle_doorbell, publish_backing};
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_tests::{Guarded, Scenario, identical_handles};
use std::collections::BTreeSet;

// ---------------------------------------------------------------------------------
// The measured identities. Taken verbatim from the 2026-07-25 RTX 3060 / 580.159.04
// trace so the "handle value is not a discriminator" property is load-bearing here:
// the KERNEL client's handle lies strictly BETWEEN the two user clients' handles, and
// all three share `RS_CLIENT_HANDLE_BASE`.
// ---------------------------------------------------------------------------------

/// Process A's compute client (`GSPALLOC hClient=0xc1d00067 processID=0xdd13`).
const A: HClient = HClient(0xc1d0_0067);
/// Process B's compute client (`GSPALLOC hClient=0xc1d00068 processID=0xdd14`).
const B: HClient = HClient(0xc1d0_0068);
/// ★ THE one UVM session client (`GSPALLOC hClient=0xc1d00069 processID=0xffffffff`).
/// One per `nvidia_uvm` module load; every guest process dups into it.
const UVM: HClient = HClient(0xc1d0_0069);
/// A **second user** client of process A, joined to `A` by a genuine sharing dup.
/// Numerically ABOVE the UVM session, so "min client" cannot accidentally be the
/// kernel one.
const A2: HClient = HClient(0xc1d0_006b);
/// A third process, started LATER (the measurement's control case).
const C: HClient = HClient(0xc1d0_0072);

const A_PDB: Pdb = Pdb(0x3401_000);
const B_PDB: Pdb = Pdb(0x3405_000);
const A2_PDB: Pdb = Pdb(0x3409_000);
const C_PDB: Pdb = Pdb(0x340d_000);
/// UVM's own VASpace PDB (the session client allocates address spaces of its own).
const UVM_PDB: Pdb = Pdb(0x2efa_6c000);

/// The identical guest VA every process uses (the #14 shape).
const SHARED_VA: GpuVa = GpuVa(0x2_0020_0000);

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

/// One compute process's subgraph plus its `DUP_OBJECT` into the shared UVM session —
/// the per-process half of the measured shape.
fn compute_and_uvm_dup(
    s: &mut Scenario,
    client: HClient,
    pdb: Pdb,
    gr_vchid: u16,
    ce_vchid: u16,
    uvm_alias: HObject,
) {
    let vas = s.compute_process(client, pdb, identical_handles(gr_vchid, ce_vchid));
    // ★ The measured edge: the USER client's VASpace, dup'd INTO the one kernel
    // session client. One-directional: the session is always the destination.
    s.push(RmEvent::Dup {
        src: vas,
        dst: kayfabe_core::rmgraph::NodeKey::new(UVM, uvm_alias),
    });
}

/// The UVM session client's own subgraph: a KERNEL client root, a device, and a
/// VASpace of its own. Emitted separately so tests can place it *before*, *between* or
/// *after* the user clients and assert the grouping is identical.
fn uvm_session(s: &mut Scenario) {
    s.push(kayfabe_tests::kernel_client_root(UVM));
    s.push(RmEvent::Alloc {
        client: UVM,
        parent: HObject(UVM.0),
        handle: HObject(0x9000_0001),
        class: kayfabe_mocks::mock_classes::DEVICE,
        facts: kayfabe_core::rmgraph::AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    });
    s.push(RmEvent::Alloc {
        client: UVM,
        parent: HObject(0x9000_0001),
        handle: HObject(0x9000_0010),
        class: kayfabe_mocks::mock_classes::VASPACE,
        facts: kayfabe_core::rmgraph::AllocFacts::default(),
    });
    s.push(RmEvent::SetPageDir {
        client: UVM,
        vaspace: HObject(0x9000_0010),
        pdb: UVM_PDB,
    });
}

/// ★ The canonical scenario, rewritten to the measurement (§12.27): processes A and B,
/// ONE shared kernel/UVM client that BOTH dup into, and a second *user* client that
/// genuinely shares with A (the merging edge).
///
/// ★ §12.38 — the session is declared FIRST, which is also what the hardware does:
/// `nvUvmInterfaceSessionCreate` runs once from `uvm_global_init` at `nvidia_uvm`
/// **module load**, so the session client exists before any CUDA process has started, let
/// alone dup'd into it. The scripted order used to dup into the session before declaring
/// it — an ordering RM refuses (`NV_ERR_INVALID_CLIENT`) and therefore never emits.
fn scenario() -> Scenario {
    let mut s = Scenario::new();
    uvm_session(&mut s);
    compute_and_uvm_dup(&mut s, A, A_PDB, 0x10, 0x11, HObject(0x9000_00a7));
    compute_and_uvm_dup(&mut s, B, B_PDB, 0x20, 0x21, HObject(0x9000_00a8));
    // Process A's second USER client: a user↔user dup, which DOES merge.
    s.peer_dup(
        A2,
        HObject(A2.0),
        HObject(0x7000_0001),
        HObject(0x7000_0010),
        A2_PDB,
        HObject(0x7000_00ff),
        kayfabe_core::rmgraph::NodeKey::new(A, identical_handles(0x10, 0x11).vaspace),
    );
    s
}

/// Apply a scenario to a fresh graph.
fn graph_of(events: &[RmEvent]) -> RmGraph {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    for &ev in events {
        g.apply(&arch, ev).expect("scripted events are valid");
    }
    g
}

#[test]
fn by_pdb_by_vchid_and_proc_grouping_are_order_independent() {
    let arch = MockArch::new();
    let events = scenario().events;

    // Reference derivation from the scripted order.
    let reference = project(&graph_of(&events), &arch, &NO_CONDEMNED).expect("projects cleanly");

    // Every permutation must yield byte-identical boundaries — ★ §12.38: every
    // permutation the PROTOCOL can produce, i.e. every linear extension of the one
    // ordering RM imposes (a dup's destination namespace declares before the dup).
    for perm in permutations(events.len()) {
        let permuted: Vec<RmEvent> = perm.iter().map(|&i| events[i]).collect();
        let mut g = RmGraph::new();
        for ev in kayfabe_tests::legal_order(&permuted) {
            g.apply(&arch, ev)
                .expect("same events, any LEGAL order, still valid");
        }
        let derived = project(&g, &arch, &NO_CONDEMNED).expect("projects cleanly in any order");
        assert_eq!(
            derived, reference,
            "derived boundaries must be independent of event order (perm {perm:?})"
        );
    }
}

/// ★★ §12.38 — the other half of the corrected property, and the half that makes the
/// first half honest: an order the protocol **forbids** is refused, by name, and the
/// refusal is itself order-independent.
///
/// Order-independence is not "every order yields the same boundaries"; it is "every
/// order of *legal* protocol facts yields the same boundaries". So the illegal orders
/// need their own statement, or the restriction in
/// [`by_pdb_by_vchid_and_proc_grouping_are_order_independent`] reads as a weakening.
/// Here it is: the same dup, presented before its destination namespace declares, is
/// `UndeclaredDupDst` **wherever** it appears in the stream, and it mutates nothing — so
/// the graph that follows is exactly the graph of the events that were accepted.
#[test]
fn a_dup_into_an_undeclared_namespace_is_refused_in_every_order() {
    use kayfabe_core::rmgraph::{NodeKey, RmGraphError};
    let arch = MockArch::new();
    let events = scenario().events;
    let planted = RmEvent::Dup {
        src: kayfabe_core::rmgraph::NodeKey::new(A, identical_handles(0x10, 0x11).vaspace),
        dst: NodeKey::new(HClient(0xc1d0_00ff), HObject(0x7777_0001)),
    };
    let refusal = Err(RmGraphError::UndeclaredClient(HClient(0xc1d0_00ff)));

    // Insert the illegal dup at EVERY position in the reference stream.
    for at in 0..=events.len() {
        let mut g = RmGraph::new();
        for (i, &ev) in events.iter().enumerate() {
            if i == at {
                assert_eq!(
                    g.apply(&arch, planted),
                    refusal,
                    "a dup into a never-declared namespace must be refused at position {at}"
                );
            }
            g.apply(&arch, ev).expect("the legal events still apply");
        }
        if at == events.len() {
            assert_eq!(g.apply(&arch, planted), refusal, "…including last");
        }
        // The refusals changed nothing: the end state is the reference end state.
        assert_eq!(
            project(&g, &arch, &NO_CONDEMNED).expect("projects"),
            project(&graph_of(&events), &arch, &NO_CONDEMNED).expect("projects"),
            "a refused event must mutate nothing (insertion point {at})"
        );
    }
}

/// ★★ THE test the old suite got wrong. One kernel/UVM client, two user processes
/// dup'ing into it: **two** user `Proc`s, and the session client in neither.
///
/// Under the old dup-connected-component rule this graph projects a SINGLE `Proc`
/// holding A, B, A2 and UVM — one isolate, one arena, one host VAS, i.e. #14 un-fixed —
/// and process B's dup would additionally be refused as a `LateMerge` the moment A had
/// touched its data plane (asserted separately below).
#[test]
fn one_kernel_client_two_processes_stay_two_procs() {
    let arch = MockArch::new();
    let b = project(&graph_of(&scenario().events), &arch, &NO_CONDEMNED).unwrap();

    assert_eq!(b.procs.len(), 2, "two USER components");
    // The system component holds the UVM session, and ONLY it.
    assert_eq!(
        b.system.client_values(),
        std::collections::BTreeSet::from([UVM]),
        "the one UVM session client is the guest KERNEL's, not any process's"
    );
    assert_eq!(b.system.anchor, SYSTEM_ANCHOR);

    let proc_a = b
        .procs
        .iter()
        .find(|p| p.client_values().contains(&A))
        .expect("proc A present");
    let proc_b = b
        .procs
        .iter()
        .find(|p| p.client_values().contains(&B))
        .expect("proc B present");
    // A merged with its second USER client (genuine sharing = one blast radius) and
    // with nothing else. B is alone. Neither contains the kernel client.
    assert_eq!(
        proc_a.client_values(),
        std::collections::BTreeSet::from([A, A2]),
        "a user↔user dup merges; the UVM dup does not"
    );
    assert_eq!(
        proc_b.client_values(),
        std::collections::BTreeSet::from([B])
    );
    for p in &b.procs {
        assert!(
            !p.client_values().contains(&UVM),
            "a kernel client leaked into a Proc"
        );
    }
}

/// ★ Attribution is by ORIGIN: the VASpace A dup'd into the UVM session stays in **A's**
/// component, and the session owns only the VASpace it allocated itself. This is what
/// makes the new cross-`Proc` reference a *reference* and not a second materialization
/// (the §12.27 coherence argument, at the projection level).
#[test]
fn a_dupd_vaspace_stays_with_the_client_that_allocated_it() {
    let arch = MockArch::new();
    let b = project(&graph_of(&scenario().events), &arch, &NO_CONDEMNED).unwrap();

    let proc_a = b
        .procs
        .iter()
        .find(|p| p.client_values().contains(&A))
        .unwrap();
    let a_pdbs: Vec<Pdb> = proc_a.vases.values().filter_map(|f| f.pdb).collect();
    assert!(a_pdbs.contains(&A_PDB), "A's own compute VAS is A's");
    assert!(a_pdbs.contains(&A2_PDB), "its merged peer's VAS too");
    assert_eq!(a_pdbs.len(), 2, "and nothing else");

    let sys_pdbs: Vec<Pdb> = b.system.vases.values().filter_map(|f| f.pdb).collect();
    assert_eq!(
        sys_pdbs,
        vec![UVM_PDB],
        "the session owns ONLY the VAS it allocated — never a copy of A's"
    );

    // Routing agrees: A's PDB routes to A, UVM's to the system anchor.
    assert_eq!(
        b.by_pdb.get(&(GpuId::ZERO, A_PDB)).map(|x| x.0),
        Some(proc_a.anchor)
    );
    assert_eq!(
        b.by_pdb.get(&(GpuId::ZERO, UVM_PDB)).map(|x| x.0),
        Some(SYSTEM_ANCHOR)
    );
}

/// ★ Order-independence of the RULE, stated directly: the kernel client may declare
/// itself **before**, **between** or **after** the user clients that dup into it, and
/// the grouping is identical. This is decision #14's whole point, and it is the property
/// that makes classification-at-declaration safe: the fact arrives on the `NV01_ROOT`,
/// *before* any dup, so no arrival order can make a dup group differently.
///
/// ★ §12.38 — "after" now means *after the user clients' own subgraphs*, not after the
/// dups into it. The session's declaration and the dups into it are the one pair the
/// protocol genuinely orders (RM resolves `hClientDst` before it copies anything), and
/// the claim under test was never about that pair: it is that the kernel declaration's
/// position relative to the **user clients** is immaterial. Stated that way it is
/// strictly sharper, because all three arrangements are now traces a real driver could
/// emit.
#[test]
fn the_kernel_declaration_may_arrive_before_between_or_after() {
    let arch = MockArch::new();

    let a_proc = |s: &mut Scenario| {
        s.compute_process(A, A_PDB, identical_handles(0x10, 0x11));
    };
    let b_proc = |s: &mut Scenario| {
        s.compute_process(B, B_PDB, identical_handles(0x20, 0x21));
    };
    let a_dup = |s: &mut Scenario| {
        s.push(RmEvent::Dup {
            src: kayfabe_core::rmgraph::NodeKey::new(A, identical_handles(0x10, 0x11).vaspace),
            dst: kayfabe_core::rmgraph::NodeKey::new(UVM, HObject(0xa7)),
        });
    };
    let b_dup = |s: &mut Scenario| {
        s.push(RmEvent::Dup {
            src: kayfabe_core::rmgraph::NodeKey::new(B, identical_handles(0x20, 0x21).vaspace),
            dst: kayfabe_core::rmgraph::NodeKey::new(UVM, HObject(0xa8)),
        });
    };

    let mut before = Scenario::new();
    uvm_session(&mut before);
    a_proc(&mut before);
    a_dup(&mut before);
    b_proc(&mut before);
    b_dup(&mut before);

    let mut between = Scenario::new();
    a_proc(&mut between);
    uvm_session(&mut between);
    a_dup(&mut between);
    b_proc(&mut between);
    b_dup(&mut between);

    let mut after = Scenario::new();
    a_proc(&mut after);
    b_proc(&mut after);
    uvm_session(&mut after);
    a_dup(&mut after);
    b_dup(&mut after);

    let reference = project(&graph_of(&before.events), &arch, &NO_CONDEMNED).unwrap();
    assert_eq!(reference.procs.len(), 2);
    assert_eq!(reference.system.client_values().len(), 1);
    for (name, s) in [("between", between), ("after", after)] {
        assert_eq!(
            project(&graph_of(&s.events), &arch, &NO_CONDEMNED).unwrap(),
            reference,
            "the kernel declaration arriving {name} the user clients changed the grouping",
        );
    }
}

/// ★ Mutation-gate kill (`ClientUnion::union` `ra < rb`→`ra == rb`): the process
/// anchor is the DETERMINISTIC **minimum** client handle in the dup-connected component
/// (`ProcBoundary::anchor` doc; rule L7 — a stable, order-independent identity, never a
/// minted id). The union must therefore make the SMALLER root the representative; the
/// `<`→`==` mutant (equality is already handled by the early return, so it always takes
/// the `else` branch) picks the OTHER root, yielding a non-minimum anchor.
///
/// ★ §12.27 keeps this alive deliberately: with the UVM edge no longer merging, the ONLY
/// remaining union is the user↔user one (A ∪ A2), so the merging component must stay in
/// the scenario or this mutant survives unkilled.
#[test]
fn proc_anchor_is_the_minimum_client_in_its_component() {
    let arch = MockArch::new();
    let b = project(&graph_of(&scenario().events), &arch, &NO_CONDEMNED).unwrap();
    assert_eq!(b.procs.len(), 2);
    for p in &b.procs {
        let min_client = p
            .client_values()
            .iter()
            .min()
            .copied()
            .expect("component non-empty");
        assert_eq!(
            p.anchor.0.client, min_client,
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
    // Concretely: proc A's clients are {0xc1d00067, 0xc1d0006b}; its anchor is the
    // smaller. Note the UVM session (0xc1d00069) lies BETWEEN them and is still not a
    // member — the anchor is a minimum over the component, never over the handle space.
    let proc_a = b
        .procs
        .iter()
        .find(|p| p.client_values().contains(&A))
        .unwrap();
    assert_eq!(
        proc_a.anchor.0.client, A,
        "0xc1d00067 < 0xc1d0006b, so A anchors"
    );
    assert!(
        UVM > A && UVM < A2,
        "the measured interleaving is preserved"
    );
}

#[test]
fn identical_handles_across_procs_do_not_collide() {
    let arch = MockArch::new();
    let b = project(&graph_of(&scenario().events), &arch, &NO_CONDEMNED).unwrap();

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
    // Four distinct channels total (2 per compute process), zero vChid collisions.
    // The UVM session and A's peer client allocate address spaces, not channels.
    assert_eq!(b.by_vchid.len(), 4);
}

// ---------------------------------------------------------------------------------
// ★ The runtime half: two ISOLATED procs, not merely two boundaries.
// ---------------------------------------------------------------------------------

/// Build a `Gpu` and apply `events`.
fn gpu_of(events: &[RmEvent]) -> Guarded<Gpu> {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("device realizes");
    for &ev in events {
        gpu.apply(ev).expect("the scenario applies");
    }
    Guarded::new("rmgraph_order_independence::gpu_of", gpu, rec)
}

/// ★★ The isolation the boundaries are *for*: with one shared kernel client, A and B
/// still get their own isolate, their own disjoint arena, no shared host handle, and
/// each can publish the identical guest VA and ring its own channel.
///
/// Under the old rule this test cannot even be written: A and B would be one `Proc`.
#[test]
fn two_processes_sharing_one_kernel_client_stay_fully_isolated() {
    let mut gpu = gpu_of(&scenario().events);
    assert_eq!(gpu.procs.len(), 2, "two user procs materialized");

    let pid_a = gpu.spine.by_pdb[&(GpuId::ZERO, A_PDB)];
    let pid_b = gpu.spine.by_pdb[&(GpuId::ZERO, B_PDB)];
    assert_ne!(pid_a, pid_b, "distinct procs");
    assert_ne!(pid_a, Gpu::SYSTEM_PROC);
    assert_ne!(pid_b, Gpu::SYSTEM_PROC);

    // Distinct isolates, disjoint arenas.
    let iso_a = gpu.procs[&pid_a].isolates[&GpuId::ZERO].id();
    let iso_b = gpu.procs[&pid_b].isolates[&GpuId::ZERO].id();
    assert_ne!(iso_a, iso_b, "each proc has its OWN isolate");
    let ra = gpu.procs[&pid_a].arenas[&GpuId::ZERO].range.clone();
    let rb = gpu.procs[&pid_b].arenas[&GpuId::ZERO].range.clone();
    assert!(
        ra.end <= rb.start || rb.end <= ra.start,
        "the two procs' GPA arenas are disjoint by construction"
    );

    // Both publish the IDENTICAL guest VA — into disjoint GPA and disjoint host VA.
    let pa = publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GpuId::ZERO,
        A_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("A publishes");
    let pb = publish_backing(
        gpu.procs.get_mut(&pid_b).unwrap(),
        GpuId::ZERO,
        B_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("B publishes — must NOT collide with A");
    assert_ne!(pa.gpa, pb.gpa, "identical guest VA → distinct GPA");
    // ★★★ #102 — corrected (was `assert_ne!` on the host VA). The two processes here
    // share ONE duplicated RM client, which is precisely why the separation must be the
    // address SPACE and not the address: a client key aliases them (`eight_blockers_
    // resolved.md` §4), and distinct-addresses would have been an accident of the mock.
    assert_eq!(
        (pa.host_va, pb.host_va),
        (SHARED_VA.0, SHARED_VA.0),
        "…both host-mapped AT the guest VA they named"
    );

    // No shared host handles: every host handle names the isolate that minted it
    // (§12.26), and the two procs' host VASes name different isolates.
    let hv_a = gpu.procs[&pid_a].vases[&(GpuId::ZERO, A_PDB)]
        .host_vas
        .expect("A's host VAS");
    let hv_b = gpu.procs[&pid_b].vases[&(GpuId::ZERO, B_PDB)]
        .host_vas
        .expect("B's host VAS");
    assert_ne!(hv_a, hv_b);
    assert_eq!(hv_a.isolate(), iso_a);
    assert_eq!(hv_b.isolate(), iso_b);

    // And both can ring their own channel, demuxing to their own proc.
    let out_a = handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x10)), &[])
        .expect("A rings");
    let out_b = handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(VChid(0x20)), &[])
        .expect("B rings");
    assert_eq!(out_a.proc, pid_a);
    assert_eq!(out_b.proc, pid_b);
    assert_ne!(out_a.host_token, out_b.host_token);
}

/// ★ The `LateMerge` that the old rule would have fired. A publishes (touching its data
/// plane), and only THEN does B arrive and dup into the shared UVM session. Under the
/// old rule that dup absorbs a touched component and is a hard `GpuError::LateMerge`
/// refusal — process #2 simply cannot start. Under the measured rule it is a reference,
/// so it applies cleanly and B gets its own everything.
#[test]
fn a_second_process_joining_the_shared_kernel_client_is_never_a_late_merge() {
    // The session (declared at `nvidia_uvm` module load) + A.
    let mut first = Scenario::new();
    uvm_session(&mut first);
    compute_and_uvm_dup(&mut first, A, A_PDB, 0x10, 0x11, HObject(0xa7));
    let mut gpu = gpu_of(&first.events);

    let pid_a = gpu.spine.by_pdb[&(GpuId::ZERO, A_PDB)];
    publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GpuId::ZERO,
        A_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("A publishes — its data plane is now TOUCHED");

    // Now B starts and dups into the same session client.
    let mut second = Scenario::new();
    compute_and_uvm_dup(&mut second, B, B_PDB, 0x20, 0x21, HObject(0xa8));
    for ev in second.events {
        gpu.apply(ev).unwrap_or_else(|e| {
            panic!("process #2 must not be refused (would have been a LateMerge): {e:?}")
        });
    }

    assert_eq!(gpu.procs.len(), 2, "B got its own Proc");
    let pid_b = gpu.spine.by_pdb[&(GpuId::ZERO, B_PDB)];
    assert_ne!(pid_a, pid_b);
    assert_eq!(
        gpu.procs[&pid_a].id, pid_a,
        "A was not absorbed, retired or re-minted"
    );
    publish_backing(
        gpu.procs.get_mut(&pid_b).unwrap(),
        GpuId::ZERO,
        B_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("B's own data plane works");
}

/// ★ The measurement's control case: a THIRD process, started later, joins the same
/// session client and must disturb neither of the first two — same `ProcId`s, same
/// arenas, same isolates, and its own everything.
#[test]
fn a_third_process_joining_later_disturbs_neither_of_the_first_two() {
    let mut gpu = gpu_of(&scenario().events);
    let pid_a = gpu.spine.by_pdb[&(GpuId::ZERO, A_PDB)];
    let pid_b = gpu.spine.by_pdb[&(GpuId::ZERO, B_PDB)];
    let before: Vec<_> = [pid_a, pid_b]
        .iter()
        .map(|p| {
            (
                *p,
                gpu.procs[p].arenas[&GpuId::ZERO].range.clone(),
                gpu.procs[p].isolates[&GpuId::ZERO].id(),
                gpu.procs[p].client_values(),
            )
        })
        .collect();
    // Both are already live and touched.
    for (pid, ..) in &before {
        let pdb = if *pid == pid_a { A_PDB } else { B_PDB };
        publish_backing(
            gpu.procs.get_mut(pid).unwrap(),
            GpuId::ZERO,
            pdb,
            SHARED_VA,
            0x10000,
        )
        .expect("publishes");
    }

    let mut third = Scenario::new();
    compute_and_uvm_dup(&mut third, C, C_PDB, 0x30, 0x31, HObject(0x9000_00b2));
    for ev in third.events {
        gpu.apply(ev).expect("the third process starts cleanly");
    }

    assert_eq!(gpu.procs.len(), 3, "three user procs");
    for (pid, range, iso, clients) in &before {
        assert_eq!(
            gpu.procs[pid].arenas[&GpuId::ZERO].range,
            *range,
            "an existing proc's arena moved"
        );
        assert_eq!(gpu.procs[pid].isolates[&GpuId::ZERO].id(), *iso);
        assert_eq!(&gpu.procs[pid].client_values(), clients);
    }
    let pid_c = gpu.spine.by_pdb[&(GpuId::ZERO, C_PDB)];
    assert!(pid_c != pid_a && pid_c != pid_b);
    // The session client still belongs to exactly one component: the system's.
    assert_eq!(
        gpu.system.client_values(),
        std::collections::BTreeSet::from([UVM])
    );
}

/// ★ §12.26 re-verified under the new grouping (§12.27): the system proc now really has
/// clients, a `Vas` and a routable PDB — and its data plane is STILL refused, by name.
///
/// This is the bite the old `the_system_proc_has_no_data_plane` could not have: before
/// the rule change `Gpu::system` owned nothing, so `plan_publish`'s refusal was reached
/// only because there was no `Vas` to publish into. Now the `Vas` exists, `by_pdb`
/// routes to `Gpu::SYSTEM_PROC`, and the refusal is the thing actually doing the work —
/// which is exactly why the cross-`Proc` reference cannot become a coherence problem:
/// the system component can hold an address-plane view and can never mint host memory.
#[test]
fn the_system_proc_has_clients_and_a_vas_and_still_no_data_plane() {
    let mut gpu = gpu_of(&scenario().events);
    assert_eq!(
        gpu.system.client_values(),
        std::collections::BTreeSet::from([UVM])
    );
    assert!(
        gpu.system.vases.contains_key(&(GpuId::ZERO, UVM_PDB)),
        "the session's own VAS materialized on the system proc"
    );
    assert_eq!(
        gpu.spine.by_pdb.get(&(GpuId::ZERO, UVM_PDB)),
        Some(&Gpu::SYSTEM_PROC),
        "a guest-KERNEL PDB routes to the system proc, not to a user proc"
    );
    assert_eq!(
        publish_backing(&mut gpu.system, GpuId::ZERO, UVM_PDB, SHARED_VA, 0x1000),
        Err(FwdFault::SystemDataPlane),
        "the system plane rule holds where it now actually bites",
    );
}

/// ★ The reserved anchor is reserved *by refusal*, not by hope: client handle 0 is
/// `NV01_NULL_OBJECT`, RM can never mint it, and the graph rejects any event naming it —
/// so a guest cannot anchor a user component at `SYSTEM_ANCHOR` and have its PDBs and
/// vChids resolve onto the system proc.
#[test]
fn client_handle_zero_is_refused_so_the_system_anchor_cannot_be_squatted() {
    use kayfabe_core::rmgraph::{NodeKey, RmGraphError};
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let zero = HClient(0);
    assert_eq!(
        g.apply(&arch, kayfabe_tests::client_root(zero)),
        Err(RmGraphError::ReservedClient(zero)),
    );
    // Every event shape that could introduce the namespace is refused, including a dup
    // that only names it as a source or destination.
    assert_eq!(
        g.apply(
            &arch,
            RmEvent::Dup {
                src: NodeKey::new(A, HObject(1)),
                dst: NodeKey::new(zero, HObject(2)),
            }
        ),
        Err(RmGraphError::ReservedClient(zero)),
    );
    assert_eq!(
        g.apply(
            &arch,
            RmEvent::Dup {
                src: NodeKey::new(zero, HObject(1)),
                dst: NodeKey::new(A, HObject(2)),
            }
        ),
        Err(RmGraphError::ReservedClient(zero)),
    );
    assert_eq!(
        g.apply(
            &arch,
            RmEvent::Free {
                client: zero,
                handle: HObject(1),
            }
        ),
        Err(RmGraphError::ReservedClient(zero)),
    );
    assert_eq!(g.client_kinds().count(), 0, "nothing entered the graph");
}

/// ★ MISS=FAULT at the declaration: a client root with NO declared kind is refused
/// loudly, because both guesses are catastrophic — "user" folds the guest kernel's
/// session into a process's blast radius, "kernel" folds a process into the guest
/// kernel's isolate. And a namespace has exactly ONE root, so the classification can
/// never be a tie-break between two declarations.
#[test]
fn an_undeclared_or_doubly_declared_client_root_is_a_loud_refusal() {
    use kayfabe_core::rmgraph::{AllocFacts, NodeKey, RmGraphError};
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let root = HObject(A.0);

    let undeclared = RmEvent::Alloc {
        client: A,
        parent: root,
        handle: root,
        class: kayfabe_mocks::mock_classes::CLIENT,
        facts: AllocFacts::default(),
    };
    assert_eq!(
        g.apply(&arch, undeclared),
        Err(RmGraphError::UndeclaredClientKind(NodeKey::new(A, root))),
    );
    assert_eq!(g.nodes().count(), 0, "the refusal changed nothing");

    // Declared: accepted, and idempotent on re-send.
    g.apply(&arch, kayfabe_tests::client_root(A))
        .expect("declared");
    g.apply(&arch, kayfabe_tests::client_root(A))
        .expect("an identical re-send is idempotent");

    // A SECOND root in the same namespace — the shape that would make the kind
    // ambiguous — is refused, whichever kind it claims.
    let second = HObject(A.0 + 0x100);
    assert_eq!(
        g.apply(
            &arch,
            RmEvent::Alloc {
                client: A,
                parent: second,
                handle: second,
                class: kayfabe_mocks::mock_classes::CLIENT,
                facts: kayfabe_tests::kernel_client(),
            }
        ),
        Err(RmGraphError::DuplicateClientRoot {
            existing: NodeKey::new(A, root),
            attempted: NodeKey::new(A, second),
        }),
    );
    assert_eq!(
        g.client_kinds().collect::<Vec<_>>(),
        vec![(
            ClientKey::first(A),
            kayfabe_arch::ClientKind::User { pid: A.0 }
        )],
        "the namespace kept its ONE declared kind",
    );

    // And a freed root frees the namespace to declare a new one — with a new kind.
    g.apply(
        &arch,
        RmEvent::Free {
            client: A,
            handle: root,
        },
    )
    .expect("the root frees");
    g.apply(&arch, kayfabe_tests::kernel_client_root(A))
        .expect("an empty namespace may declare a fresh root");
    assert_eq!(
        g.client_kinds().collect::<Vec<_>>(),
        vec![(ClientKey::first(A), kayfabe_arch::ClientKind::Kernel)],
    );
}

/// ★ §12.27's own §12.25 trap, and the branch only the refcount fuzz was killing: the
/// client-root index tracks the root **HANDLE**, not the resource.
///
/// Free a client root while a `DUP_OBJECT` alias in ANOTHER namespace keeps the Client
/// resource alive. The resource survives (faithful RM refcounting — that is tested
/// elsewhere), but the namespace it came from is now *empty*, and must be free to
/// declare a fresh root with a freshly declared [`kayfabe_arch::ClientKind`]. Pruning
/// the index on the resource's death instead leaves the namespace permanently
/// un-declarable — and then, once it HAS declared a new root, prunes the new entry when
/// the old alias finally dies.
#[test]
fn a_root_kept_alive_by_a_dup_no_longer_occupies_its_own_namespace() {
    use kayfabe_core::rmgraph::NodeKey;
    let arch = MockArch::new();
    let mut g = RmGraph::new();

    // B allocs a root; A aliases B's root object into its own namespace.
    g.apply(&arch, kayfabe_tests::client_root(A))
        .expect("A's root");
    g.apply(&arch, kayfabe_tests::client_root(B))
        .expect("B's root");
    g.apply(
        &arch,
        RmEvent::Dup {
            src: NodeKey::new(B, HObject(B.0)),
            dst: NodeKey::new(A, HObject(0x900)),
        },
    )
    .expect("A aliases B's client object");

    // B frees its root. The RESOURCE survives on A's alias…
    g.apply(
        &arch,
        RmEvent::Free {
            client: B,
            handle: HObject(B.0),
        },
    )
    .expect("B's root frees");
    assert_eq!(
        g.origin_of(NodeKey::new(A, HObject(0x900))).map(|n| n.key),
        Some(NodeKey::new(B, HObject(B.0))),
        "the alias still resolves — the resource outlived its origin handle",
    );
    // …but B's NAMESPACE is empty, so B may declare a new root — with a NEW kind.
    assert!(
        !g.client_kinds().any(|(c, _)| c.client == B),
        "a namespace whose root handle is freed declares nothing",
    );
    g.apply(&arch, kayfabe_tests::kernel_client_root(B))
        .expect("★ the emptied namespace may declare a fresh root");
    // ★★★ §12.42 — B's fresh root is B's **second** declaration, `incarnation: 1`, and
    // that is the honest reading rather than a wart: A's alias keeps B's ORIGINAL client
    // object alive (RM refcounts it), so declaration `{B, 0}` still owns a live resource
    // and the two are simultaneously live. The ordinal is bounded by the LIVE set, so it
    // returns to 0 the moment the alias dies — asserted below.
    assert_eq!(
        g.client_kinds().collect::<Vec<_>>(),
        vec![
            (
                ClientKey::first(A),
                kayfabe_arch::ClientKind::User { pid: A.0 }
            ),
            (
                ClientKey {
                    client: B,
                    incarnation: 1
                },
                kayfabe_arch::ClientKind::Kernel
            ),
        ],
    );

    // And when the old alias finally dies, it must NOT prune B's new declaration.
    g.apply(
        &arch,
        RmEvent::Free {
            client: A,
            handle: HObject(0x900),
        },
    )
    .expect("the alias frees");
    assert_eq!(
        g.client_kinds()
            .find(|(c, _)| c.client == B)
            .map(|(_, k)| k),
        Some(kayfabe_arch::ClientKind::Kernel),
        "the dead alias pruned the LIVE root's declaration",
    );
    // ★★★ §12.42 — and the superseded declaration is GONE from the graph now that it owns
    // nothing live, so a THIRD tenant of this value takes ordinal 1 again rather than
    // accumulating: the ordinal separates simultaneously-live declarations and nothing
    // else, which is what keeps `RmGraph::decls` from being a guest-growable ghost log.
    assert_eq!(
        g.client_declarations()
            .keys()
            .filter(|k| k.client == B)
            .copied()
            .collect::<Vec<_>>(),
        vec![ClientKey {
            client: B,
            incarnation: 1
        }],
        "the orphaned declaration outlived the last resource it owned",
    );
}

/// ★★ **CHANGED BY §12.38, deliberately — this test used to assert the accept-then-merge
/// behaviour ON PURPOSE, and that behaviour was the vulnerability.**
///
/// What it asserted before: a dup into a client whose root has not arrived is *accepted
/// and parked*, groups with nobody while the destination is undeclared, and then becomes
/// a merge (or stays a reference) the instant the destination declares its
/// [`kayfabe_arch::ClientKind`]. Every one of those sentences is true of the code as it
/// then was, and the last one is the security hole: **the merge fires on the victim's own
/// `Alloc`.** An attacker that plants one `DUP_OBJECT` into a guessable, never-allocated
/// `hClient` (in RM the `hClient` **is** its root object's handle) puts the next process
/// handed that value into the *attacker's* `Proc` — one isolate, one GPA arena, one host
/// VAS, i.e. #14 un-fixed for a chosen pair (`l1_mean.rs`'s
/// `a_planted_dup_alias_cannot_squat_a_later_process_into_the_attackers_proc`).
///
/// Why the old assertion looked right: it was defending decision #4, and "the same facts
/// in any order must yield the same end state" genuinely does forbid turning a transient
/// absence into a refusal. The correction is to the *criterion*, not to the principle —
/// order-independence holds over **legal protocol facts**, and RM resolves `hClientDst`
/// in the client DB before it copies anything (`ogkm rs_server.c:1674` →
/// `NV_ERR_INVALID_OBJECT_HANDLE`; `clientValidate` → `NV_ERR_INVALID_CLIENT`,
/// `rmapi/client.c:782`). A dup into a nonexistent namespace is therefore not a trace the
/// guest's RM can emit, so refusing it loses no legal ordering — while modelling an
/// ordering the hardware forbids was the whole of the breach.
///
/// What it asserts now: the **refusal**, by exact variant and mutation-free — and then
/// the surviving half of the original claim, which is still load-bearing and is what
/// §12.27 is actually about: once the destination HAS declared, the same dup merges or
/// stays a reference purely on its declared kind.
#[test]
fn an_undeclared_client_merges_with_nobody_until_it_declares() {
    use kayfabe_core::rmgraph::{NodeKey, RmGraphError};
    let arch = MockArch::new();
    let mut s = Scenario::new();
    let a_vas = s.compute_process(A, A_PDB, identical_handles(0x10, 0x11));
    let planted = RmEvent::Dup {
        src: a_vas,
        dst: NodeKey::new(A2, HObject(0x7000_00ff)),
    };

    // ---- ★ A2 has NOT declared a root. The dup is refused, and changes nothing.
    let mut g = graph_of(&s.events);
    let before = project(&g, &arch, &NO_CONDEMNED).unwrap();
    assert_eq!(
        g.apply(&arch, planted),
        Err(RmGraphError::UndeclaredClient(A2)),
        "★★ a dup into a namespace with no declared client root is a protocol violation \
         (RM: `NV_ERR_INVALID_CLIENT`), not a fact that has not arrived yet"
    );
    assert_eq!(
        project(&g, &arch, &NO_CONDEMNED).unwrap(),
        before,
        "the refusal mutated nothing"
    );
    assert_eq!(before.procs.len(), 1);
    assert_eq!(
        before.procs[0].client_values(),
        std::collections::BTreeSet::from([A])
    );

    // ---- Declared USER first, THEN the dup: it is a grouping edge.
    let mut declared = s.clone();
    declared.push(kayfabe_tests::client_root(A2));
    declared.push(planted);
    let joined = project(&graph_of(&declared.events), &arch, &NO_CONDEMNED).unwrap();
    assert_eq!(joined.procs.len(), 1, "declared user → the dup merges");
    assert_eq!(
        joined.procs[0].client_values(),
        std::collections::BTreeSet::from([A, A2])
    );

    // ---- Had it declared KERNEL instead, the very same dup stays a reference. This is
    // the §12.27 claim the old test carried, and it survives intact: the ONLY thing that
    // decides merge-vs-reference is the destination's declared kind.
    let mut kernel = s.clone();
    kernel.push(kayfabe_tests::kernel_client_root(A2));
    kernel.push(planted);
    let referenced = project(&graph_of(&kernel.events), &arch, &NO_CONDEMNED).unwrap();
    assert_eq!(referenced.procs.len(), 1, "only A is a user proc");
    assert_eq!(
        referenced.procs[0].client_values(),
        std::collections::BTreeSet::from([A])
    );
    assert_eq!(
        referenced.system.client_values(),
        std::collections::BTreeSet::from([A2])
    );
}

/// A `LateMerge` must still fire for the shape it exists for — a USER↔USER dup that
/// absorbs a component which has already touched its data plane. §12.27 narrows the
/// grouping rule; it does not disarm the guard.
#[test]
fn a_user_peer_dup_onto_a_touched_proc_is_still_a_late_merge() {
    let mut s = Scenario::new();
    let a_vas = s.compute_process(A, A_PDB, identical_handles(0x10, 0x11));
    let mut gpu = gpu_of(&s.events);

    // A2 arrives as its own proc, and it is the one that TOUCHES its plane; merging it
    // into the older proc A therefore absorbs a touched component — the refusal.
    let mut s2 = Scenario::new();
    s2.compute_process(A2, A2_PDB, identical_handles(0x30, 0x31));
    for ev in s2.events {
        gpu.apply(ev).expect("A2's own process");
    }
    let pid_a2 = gpu.spine.by_pdb[&(GpuId::ZERO, A2_PDB)];
    let pid_a = gpu.spine.by_pdb[&(GpuId::ZERO, A_PDB)];
    assert!(
        pid_a < pid_a2,
        "A is the older proc, so A survives the merge"
    );
    publish_backing(
        gpu.procs.get_mut(&pid_a2).unwrap(),
        GpuId::ZERO,
        A2_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("A2 touches its plane");

    let refused = gpu.apply(RmEvent::Dup {
        src: a_vas,
        dst: kayfabe_core::rmgraph::NodeKey::new(A2, HObject(0x7000_00ff)),
    });
    assert_eq!(
        refused,
        Err(GpuError::LateMerge {
            kept: pid_a,
            absorbed: pid_a2,
        }),
        "a user↔user dup absorbing a TOUCHED proc is still loud",
    );
}

/// ★★ §12.27's COHERENCE / LIFETIME re-verification, made executable.
///
/// The open driver does **not** guarantee that UVM's session-client dup dies with the
/// guest process that owns the memory. `uvm_va_space` is bound to the `/dev/nvidia-uvm`
/// **file**, not to the process (`ogkm kernel-open/nvidia-uvm/uvm_va_space_mm.c:75-81`),
/// and `UVM_INIT_FLAGS_MULTI_PROCESS_SHARING_MODE` says so outright: resources "will be
/// freed when the last reference to the file is dropped rather than when this process
/// exits" (`uvm.h:160-167`; "zombie" ranges, `uvm_va_range.h:265-268`). So a kernel
/// client's reference to a user process's object CAN outlive that process.
///
/// That would be a use-after-free if a `Proc`'s host lifetime were keyed on its client
/// root. **It is not.** Attribution is by the resource's ORIGIN, and a component is
/// derived from the resources that are *live*, so as long as any surviving reference —
/// including the kernel session's dup — names a resource this client allocated, the
/// boundary still exists, the same `Proc` survives the match, and its isolate, arena and
/// published host backing are untouched. The isolate outlives the guest client for
/// exactly as long as RM's own refcount says the object does.
#[test]
fn a_kernel_dup_keeps_the_owning_procs_isolate_and_backing_alive_past_the_clients_free() {
    let mut s = Scenario::new();
    uvm_session(&mut s);
    compute_and_uvm_dup(&mut s, A, A_PDB, 0x10, 0x11, HObject(0x9000_00a7));
    let mut gpu = gpu_of(&s.events);

    let pid_a = gpu.spine.by_pdb[&(GpuId::ZERO, A_PDB)];
    let published = publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GpuId::ZERO,
        A_PDB,
        SHARED_VA,
        0x10000,
    )
    .expect("A publishes");
    let arena = gpu.procs[&pid_a].arenas[&GpuId::ZERO].range.clone();
    let iso = gpu.procs[&pid_a].isolates[&GpuId::ZERO].id();

    // The guest process dies: the guest kernel frees A's client root. UVM's session
    // still holds its dup of A's VASpace (the multi-process-sharing / zombie case).
    gpu.apply(RmEvent::Free {
        client: A,
        handle: identical_handles(0x10, 0x11).client_root,
    })
    .expect("A's client root frees");

    assert!(
        gpu.procs.contains_key(&pid_a),
        "★ the owning Proc must NOT be retired while a kernel dup still references \
         a resource it allocated — that retire would reclaim host memory RM says is live",
    );
    assert_eq!(gpu.procs[&pid_a].arenas[&GpuId::ZERO].range, arena);
    assert_eq!(gpu.procs[&pid_a].isolates[&GpuId::ZERO].id(), iso);
    assert_eq!(
        gpu.spine.by_pdb.get(&(GpuId::ZERO, A_PDB)),
        Some(&pid_a),
        "the dup-kept VASpace still routes to its allocator's proc",
    );
    let (binding, _) = kayfabe_fwd::resolve(&gpu, GpuId::ZERO, A_PDB, SHARED_VA).expect("resolves");
    assert_eq!(
        binding.host_va(),
        Some(published.host_va),
        "the published host backing survived the guest client's free",
    );

    // And when the KERNEL side finally releases its dup (UVM's `FreeDupedHandle` at
    // `uvm_va_space_destroy`), the last reference goes and the proc retires normally.
    gpu.apply(RmEvent::Free {
        client: UVM,
        handle: HObject(0x9000_00a7),
    })
    .expect("the session frees its dup");
    assert!(
        !gpu.procs.contains_key(&pid_a),
        "the LAST reference going is what retires the proc",
    );
    let reaped = gpu.reap_retired();
    assert_eq!(
        reaped.len(),
        1,
        "exactly one proc retired — no phantom churn"
    );
    assert!(reaped.orphaned().is_empty(), "nothing was orphaned");
}

// =================================================================================
// ★★★ §12.39 — a namespace that is FREED and RE-DECLARED, inside the shuffle domain
// =================================================================================

/// The namespace declared, freed, and handed to somebody else.
const RECYCLED: HClient = HClient(0xc1d0_0080);
/// A peer that keeps one of the first tenant's resources alive past its root's free.
const KEEPER: HClient = HClient(0xc1d0_0081);
/// The first tenant's PDB.
const FIRST_PDB: Pdb = Pdb(0x3411_000);
/// The second tenant's PDB.
const SECOND_PDB: Pdb = Pdb(0x3415_000);
/// The keeper's own PDB.
const KEEPER_PDB: Pdb = Pdb(0x3419_000);
/// The handle the keeper holds the first tenant's VASpace under.
const KEEP_ALIAS: HObject = HObject(0x7f00_0001);

/// ★★ **A freed-and-re-declared namespace projects identically in every order**
/// (`l1_concurrency.md` §12.39).
///
/// §12.39's identity model gives every namespace a `ClientId` minted from a monotonic
/// counter, so its *value* depends on the order the events arrived in. That is precisely
/// why the id never appears in [`kayfabe_core::project::Boundaries`] — it is compared,
/// never reported — and this test is the executable statement of that: the same facts in
/// any legal order must still project byte-identical boundaries, even across the one
/// transition where two declarations of one `hClient` exist in the same run.
///
/// **`Free` is not in the shuffle domain**, and never was (`rmgraph` module docs: *"only
/// `RmEvent::Free` is inherently ordered (lifecycle), which is why the shuffle property is
/// stated over alloc/dup/bind facts"*). Moving a root-free before the objects it destroys
/// is a different history, not a different order of one history. So the two *phases* are
/// shuffled independently with the free pinned between them, which is exactly the domain
/// the property is stated over.
#[test]
fn a_freed_and_redeclared_namespace_projects_identically_in_every_order() {
    let arch = MockArch::new();

    // Phase 1: the first tenant builds a VASpace; a peer aliases it, so the resource
    // outlives the namespace's root.
    let mut p1 = Scenario::new();
    let first_vas =
        p1.compute_process_on_gpu(RECYCLED, FIRST_PDB, identical_handles(0x20, 0x21), None);
    p1.peer_dup(
        KEEPER,
        HObject(KEEPER.0),
        HObject(0x6e00_0001),
        HObject(0x6e00_0010),
        KEEPER_PDB,
        KEEP_ALIAS,
        first_vas,
    );
    let phase1 = p1.events;

    let free_root = RmEvent::Free {
        client: RECYCLED,
        handle: identical_handles(0x20, 0x21).client_root,
    };

    // Phase 2: an unrelated later process is handed the same `hClient`, with handle
    // values of its own (the object axis is §12.25's deferred finding, not this one).
    let mut p2 = Scenario::new();
    p2.compute_process_on_gpu(
        RECYCLED,
        SECOND_PDB,
        kayfabe_tests::ProcessHandles {
            client_root: HObject(0x6f00_0000),
            device: HObject(0x6f00_0001),
            vaspace: HObject(0x6f00_0010),
            tsg: HObject(0x6f00_0012),
            gr_channel: HObject(0x6f00_0019),
            gr_vchid: VChid(0x22),
            ce_channel: HObject(0x6f00_001a),
            ce_vchid: VChid(0x23),
        },
        None,
    );
    let phase2 = p2.events;

    let run = |a: &[RmEvent], b: &[RmEvent]| {
        let mut g = RmGraph::new();
        for ev in kayfabe_tests::legal_order(a) {
            g.apply(&arch, ev)
                .expect("phase 1 applies in any legal order");
        }
        g.apply(&arch, free_root)
            .expect("the first tenant's root frees");
        for ev in kayfabe_tests::legal_order(b) {
            g.apply(&arch, ev).expect(
                "★ re-declaring a recycled namespace is LEGAL — RM recycles `hClient` \
                 values by design and refusing would hang a real guest",
            );
        }
        project(&g, &arch, &NO_CONDEMNED).expect("projects cleanly")
    };

    let reference = run(&phase1, &phase2);
    // The second tenant is its OWN component, and it is the only one that owns the
    // recycled handle value.
    assert!(
        reference.by_pdb.contains_key(&(GpuId::ZERO, SECOND_PDB)),
        "the second tenant's address plane is routable"
    );
    // ★★★ §12.42 (N1) — **CORRECTED.** This used to assert that `FIRST_PDB` belonged to
    // NOBODY. Belonging to nobody is what vacated the first tenant's component and freed
    // host memory RM still refcounts for the keeper's alias
    // (`ogkm .../mem_mgr/mem.c:1027-1031`). Both declarations of the value are live, so
    // both project — into two DIFFERENT components, which is what an `HClient`-keyed
    // `ProcAnchor` could not express (N2).
    let gen1 = ClientKey::first(RECYCLED);
    let gen2 = ClientKey {
        client: RECYCLED,
        incarnation: 1,
    };
    assert_eq!(
        reference
            .by_pdb
            .get(&(GpuId::ZERO, FIRST_PDB))
            .map(|&(a, _)| a),
        Some(kayfabe_core::ProcAnchor(gen1)),
        "★★ the SUPERSEDED declaration's still-live address plane must stay with the \
         declaration that allocated it"
    );
    assert_ne!(
        reference
            .by_pdb
            .get(&(GpuId::ZERO, FIRST_PDB))
            .map(|&(a, _)| a),
        reference
            .by_pdb
            .get(&(GpuId::ZERO, SECOND_PDB))
            .map(|&(a, _)| a),
        "★★ the successor INHERITED the previous tenant's address plane — one `hClient` \
         VALUE, two lifetimes, and they must never be one component"
    );
    let second = reference
        .procs
        .iter()
        .find(|p| p.clients.contains(&gen2))
        .expect("the second tenant has a component");
    assert_eq!(
        second.clients,
        std::collections::BTreeSet::from([gen2]),
        "★★ the successor's component must hold its OWN declaration and nothing else — \
         not the keeper (whose alias names a recyclable VALUE, not the namespace it was \
         allocated in) and not its own predecessor"
    );

    for pa in permutations(phase1.len()) {
        for pb in permutations(phase2.len()) {
            let a: Vec<RmEvent> = pa.iter().map(|&i| phase1[i]).collect();
            let b: Vec<RmEvent> = pb.iter().map(|&i| phase2[i]).collect();
            assert_eq!(
                run(&a, &b),
                reference,
                "a freed-and-re-declared namespace must project identically in every \
                 legal order (perms {pa:?} / {pb:?})"
            );
        }
    }
}

// =================================================================================
// ★★★ §12.41 — a recycled OBJECT handle, inside the shuffle domain
// =================================================================================

/// The process that recycles one of its own object handle values.
const OBJ_RECYCLED: HClient = HClient(0xc1d0_0090);
/// The peer that keeps the first incarnation alive.
const OBJ_KEEPER: HClient = HClient(0xc1d0_0091);
/// The first incarnation's PDB.
const OBJ_FIRST_PDB: Pdb = Pdb(0x3421_000);
/// The second incarnation's PDB, at the same handle value.
const OBJ_SECOND_PDB: Pdb = Pdb(0x3425_000);
/// The keeper's own PDB.
const OBJ_KEEPER_PDB: Pdb = Pdb(0x3429_000);
/// The handle the keeper holds the first incarnation under.
const OBJ_KEEP_ALIAS: HObject = HObject(0x7f00_0011);

/// ★★ **A RECYCLED OBJECT HANDLE PROJECTS IDENTICALLY IN EVERY LEGAL ORDER**
/// (`l1_concurrency.md` §12.41) — the counter-discipline for the incarnation ordinal.
///
/// §12.41 keys the projection on `ResourceKey` = origin handle + **incarnation ordinal**,
/// and the ordinal is what makes that legal to *report*: a `ResId` (or a `ClientId`) is
/// minted from a counter, so its value depends on arrival order and it may never appear
/// inside [`kayfabe_core::project::Boundaries`], which is compared whole across shuffles
/// (§12.40's finding, restated one level down). The ordinal has no such dependence,
/// because allocations at ONE handle value are **totally ordered by the protocol** — an
/// `Alloc` onto a live handle is `ConflictingAlloc`, mirroring RM's own
/// `NV_ERR_INSERT_DUPLICATE_NAME` (`ogkm .../resserv/src/rs_client.c:1446-1470`).
///
/// This test is the executable form of that claim. As in
/// [`a_freed_and_redeclared_namespace_projects_identically_in_every_order`], **`Free` is
/// not in the shuffle domain**: the two phases are shuffled independently with the
/// object free pinned between them, which is exactly the partial order the property is
/// stated over.
#[test]
fn a_recycled_object_handle_projects_identically_in_every_order() {
    let arch = MockArch::new();
    let h = identical_handles(0x40, 0x41);

    // Phase 1: the process builds a VASpace; a peer aliases it, so the resource outlives
    // its origin HANDLE's free.
    let mut p1 = Scenario::new();
    let first_vas = p1.compute_process_on_gpu(OBJ_RECYCLED, OBJ_FIRST_PDB, h, None);
    p1.peer_dup(
        OBJ_KEEPER,
        HObject(OBJ_KEEPER.0),
        HObject(0x6e00_0101),
        HObject(0x6e00_0110),
        OBJ_KEEPER_PDB,
        OBJ_KEEP_ALIAS,
        first_vas,
    );
    let phase1 = p1.events;

    let free_handle = RmEvent::Free {
        client: OBJ_RECYCLED,
        handle: h.vaspace,
    };

    // Phase 2: a DIFFERENT VASpace at the very same handle value.
    let phase2 = vec![
        RmEvent::Alloc {
            client: OBJ_RECYCLED,
            parent: h.device,
            handle: h.vaspace,
            class: kayfabe_mocks::mock_classes::VASPACE,
            facts: kayfabe_core::rmgraph::AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: OBJ_RECYCLED,
            vaspace: h.vaspace,
            pdb: OBJ_SECOND_PDB,
        },
    ];

    let run = |a: &[RmEvent], b: &[RmEvent]| {
        let mut g = RmGraph::new();
        for ev in kayfabe_tests::legal_order(a) {
            g.apply(&arch, ev)
                .expect("phase 1 applies in any legal order");
        }
        g.apply(&arch, free_handle)
            .expect("the origin handle frees");
        // Applied verbatim, not through `legal_order`: phase 2 declares no client root
        // (the namespace has been live throughout), and both of its events are
        // order-tolerant by construction — a `SET_PAGE_DIRECTORY` may legally precede
        // the VASpace it targets, which is exactly what the permutation exercises.
        for &ev in b {
            g.apply(&arch, ev).expect(
                "★ re-allocating a freed OBJECT handle is LEGAL — RM validates a \
                 caller-supplied handle only against the LIVE map and quarantines \
                 nothing on free",
            );
        }
        project(&g, &arch, &NO_CONDEMNED).expect("projects cleanly")
    };

    let reference = run(&phase1, &phase2);
    // Both incarnations are live and BOTH route — the ghost is the peer's, and §12.33
    // says a surviving reference keeps its object alive *and usable*.
    for pdb in [OBJ_FIRST_PDB, OBJ_SECOND_PDB] {
        assert!(
            reference.by_pdb.contains_key(&(GpuId::ZERO, pdb)),
            "★★ a live VASpace lost its address plane to a recycled handle value ({pdb:?})"
        );
    }
    let owner = reference
        .procs
        .iter()
        .find(|p| p.client_values().contains(&OBJ_RECYCLED))
        .expect("the process has a component");
    assert_eq!(
        owner
            .vases
            .values()
            .filter_map(|f| f.pdb)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([OBJ_FIRST_PDB, OBJ_SECOND_PDB, OBJ_KEEPER_PDB]),
        "★★ the component must own BOTH incarnations (plus the keeper's own VAS — the \
         peer dup merges the two user clients into one `Proc` by rule)"
    );

    for pa in permutations(phase1.len()) {
        for pb in permutations(phase2.len()) {
            let a: Vec<RmEvent> = pa.iter().map(|&i| phase1[i]).collect();
            let b: Vec<RmEvent> = pb.iter().map(|&i| phase2[i]).collect();
            assert_eq!(
                run(&a, &b),
                reference,
                "a recycled OBJECT handle must project identically in every legal order \
                 (perms {pa:?} / {pb:?})"
            );
        }
    }
}

// =================================================================================
// ★★★ §12.44 — the two shapes the A1 fuzz property's shrinker drove out of hiding: a
// grouping edge that OUTLIVES its source declaration, and a ghost whose declared handle
// facts name a namespace that has since changed hands.
// =================================================================================

/// The client whose namespace is orphaned and then handed back — the recycled `hClient`.
const SPLIT_SRC: HClient = HClient(0xE1);
/// The alias holder: a live user client that keeps [`SPLIT_SRC`]'s resource alive.
const SPLIT_DST: HClient = HClient(0xE2);
/// ★ The NEGATIVE control: a third user client that shares NOTHING with anybody and must
/// therefore be its own `Proc` at every step. Without it this test could pass on a
/// projection that simply put every client in one component, which IS #14 un-fixed.
const SPLIT_BYSTANDER: HClient = HClient(0xE3);
/// The handle [`SPLIT_DST`] holds [`SPLIT_SRC`]'s client object under.
const SPLIT_ALIAS: HObject = HObject(0x7f00_0001);

/// ★★★ **A grouping edge that outlives its source declaration must never re-group onto
/// the recycled `hClient`'s NEXT tenant** (`l1_concurrency.md` §12.44 — the hand-written
/// statement of the A1 shrink; the seed itself is pinned in
/// `fuzz_rmgraph_invariants.proptest-regressions`).
///
/// The shrunk stream, in five events:
///
/// ```text
/// Alloc user SPLIT_DST(root) │ Alloc user SPLIT_SRC(root)
/// Dup { src: (SPLIT_SRC, root), dst: (SPLIT_DST, alias) }   ⇒ a user↔user GROUPING edge
/// Free (SPLIT_SRC, root)      ⇒ the resource LIVES on the alias; the declaration is ORPHANED
/// Alloc user SPLIT_SRC(root)  ⇒ an UNRELATED later tenant of the same `hClient` VALUE
/// ```
///
/// After the last event the edge is still there and still resolves — RM refcounts the
/// aliased object for its holder (`ogkm src/nvidia/src/kernel/mem_mgr/mem.c:1027-1031`) —
/// and BOTH of its endpoints' `hClient` values name declared `ClientKind::User` clients.
/// Read as VALUES, that is a live user↔user dup across two `Proc`s, i.e. exactly the thing
/// §12.27 says must be one `Proc`. Read as DECLARATIONS it is nothing of the kind: the
/// edge's source is generation 0, which has no root and therefore no live namespace, while
/// the client the value names *now* is generation 1, an unrelated process that never
/// dup'd anything. Merging them is §12.39 Shape B — the victim inherits the previous
/// tenant's isolate, GPA arena and host VAS.
///
/// Three things are asserted in order, and the first is what makes the other two mean
/// anything: **the grouping edge was real before the free** (so this is a genuine split,
/// not a pair that was never joined), the split then happens, and the re-declaration joins
/// nobody. The bystander pins the negative direction throughout.
#[test]
fn a_dup_whose_source_declaration_lost_its_root_never_regroups_onto_the_recycled_value() {
    use kayfabe_core::ProcAnchor;
    use kayfabe_core::rmgraph::NodeKey;
    let arch = MockArch::new();
    let mut g = RmGraph::new();

    let components = |g: &RmGraph| -> Vec<(ProcAnchor, Vec<ClientKey>)> {
        project(g, &arch, &NO_CONDEMNED)
            .expect("projects")
            .procs
            .iter()
            .map(|p| (p.anchor, p.clients.iter().copied().collect()))
            .collect()
    };
    let decl = |c: HClient, n: u32| ClientKey {
        client: c,
        incarnation: n,
    };

    for c in [SPLIT_DST, SPLIT_SRC, SPLIT_BYSTANDER] {
        g.apply(&arch, kayfabe_tests::client_root(c))
            .expect("a user client root");
    }
    // ---- ★ PRECONDITION (the non-vacuity assertion): before anything is freed, the
    // user↔user dup makes the two ONE component — and leaves the bystander alone.
    g.apply(
        &arch,
        RmEvent::Dup {
            src: NodeKey::new(SPLIT_SRC, HObject(SPLIT_SRC.0)),
            dst: NodeKey::new(SPLIT_DST, SPLIT_ALIAS),
        },
    )
    .expect("the alias applies");
    assert_eq!(
        components(&g),
        vec![
            (
                ProcAnchor(decl(SPLIT_SRC, 0)),
                vec![decl(SPLIT_SRC, 0), decl(SPLIT_DST, 0)]
            ),
            (
                ProcAnchor(decl(SPLIT_BYSTANDER, 0)),
                vec![decl(SPLIT_BYSTANDER, 0)]
            ),
        ],
        "★ precondition: a live user↔user dup IS a grouping edge — and a client that \
         shares nothing is its own component"
    );

    // ---- The source frees its client ROOT. The resource lives on the alias, so the
    // declaration survives as an ORPHAN — and the component splits, because §12.27's
    // predicate wants positive evidence of a LIVE declaration at both ends.
    g.apply(
        &arch,
        RmEvent::Free {
            client: SPLIT_SRC,
            handle: HObject(SPLIT_SRC.0),
        },
    )
    .expect("the source frees its root");
    assert_eq!(
        g.origin_of(NodeKey::new(SPLIT_DST, SPLIT_ALIAS))
            .map(|n| n.key),
        Some(NodeKey::new(SPLIT_SRC, HObject(SPLIT_SRC.0))),
        "the edge must SURVIVE the free — RM refcounts the object for its alias holder",
    );
    assert_eq!(
        components(&g),
        vec![
            (ProcAnchor(decl(SPLIT_SRC, 0)), vec![decl(SPLIT_SRC, 0)]),
            (ProcAnchor(decl(SPLIT_DST, 0)), vec![decl(SPLIT_DST, 0)]),
            (
                ProcAnchor(decl(SPLIT_BYSTANDER, 0)),
                vec![decl(SPLIT_BYSTANDER, 0)]
            ),
        ],
        "freeing the root SPLITS the component — and the orphan keeps a component of its \
         own, because retiring it would free host memory RM still says is live (§12.42 N1)"
    );

    // ---- ★★ THE BREACH. The value is handed to an unrelated later process. Its own
    // first RM event is its client root, and it MUST be accepted (RM recycles by design;
    // refusing hangs a legal guest). The surviving edge must group it with nobody.
    g.apply(&arch, kayfabe_tests::client_root(SPLIT_SRC))
        .expect("★ re-declaring a recycled namespace is LEGAL");
    assert_eq!(
        components(&g),
        vec![
            (ProcAnchor(decl(SPLIT_SRC, 0)), vec![decl(SPLIT_SRC, 0)]),
            (ProcAnchor(decl(SPLIT_SRC, 1)), vec![decl(SPLIT_SRC, 1)]),
            (ProcAnchor(decl(SPLIT_DST, 0)), vec![decl(SPLIT_DST, 0)]),
            (
                ProcAnchor(decl(SPLIT_BYSTANDER, 0)),
                vec![decl(SPLIT_BYSTANDER, 0)]
            ),
        ],
        "★★ the new tenant of the recycled `hClient` was merged into the alias holder's \
         `Proc` by an edge whose source is a declaration that died before it existed"
    );
    // The orphan is still a real, live declaration owning a real, live resource: nothing
    // was dropped on the floor to buy the isolation (§12.42 N1's whole point).
    assert_eq!(
        g.client_declarations()
            .keys()
            .filter(|k| k.client == SPLIT_SRC)
            .copied()
            .collect::<Vec<_>>(),
        vec![decl(SPLIT_SRC, 0), decl(SPLIT_SRC, 1)],
        "both generations of the value must be live declarations",
    );
}

/// The namespace whose `hClient` is orphaned and then recycled.
const GHOST_NS: HClient = HClient(0xE5);
/// The client that keeps the dead namespace's channel alive with a `DUP_OBJECT`.
const GHOST_KEEPER: HClient = HClient(0xE6);
/// The handle value the ghost channel declares as its `hVASpace` — and that the
/// namespace's NEXT tenant then allocates its own VASpace at.
const GHOST_HVAS: HObject = HObject(0x8000_0010);
/// The ghost channel's own handle.
const GHOST_CHAN: HObject = HObject(0x8000_0020);
/// The first tenant's device handle.
const GHOST_DEV1: HObject = HObject(0x8000_0001);
/// The second tenant's device handle.
const GHOST_DEV2: HObject = HObject(0x8000_0002);
/// The handle [`GHOST_KEEPER`] holds the ghost channel under.
const GHOST_ALIAS: HObject = HObject(0x8100_0001);
/// The second tenant's page-directory base — the plane the ghost must never reach.
const GHOST_VICTIM_PDB: Pdb = Pdb(0x5900_0000);

/// ★★★ **A ghost's declared handle facts name a namespace that no longer exists — they
/// must MISS, never bind the recycled `hClient`'s next tenant** (`l1_concurrency.md`
/// §12.44, the exec-plane sibling of §12.39 Shape B).
///
/// `hVASpace`, `hContextShare` and `parent` are handles **into the allocating client's own
/// handle table**, and that table belongs to a DECLARATION: freeing a client root destroys
/// every handle in the namespace. A channel that outlives its namespace's root — kept
/// alive by a foreign `DUP_OBJECT`, which is the steady state, since the measured UVM
/// session holds 82 aliases per CUDA process — therefore declares facts about a table that
/// is gone.
///
/// §12.41 moved `project`'s per-node *lookups* onto resource identity
/// (`pdb_of_resource` / `gpu_of_resource`) but left the declared-fact *resolvers* keyed on
/// the recyclable `HClient` VALUE, so the ghost was handed whatever the value's NEXT tenant
/// had allocated at that handle number. Measured on the pre-fix tree, this exact script
/// projected the ghost channel with `vas_origin` = the second tenant's VASpace and
/// `vas_pdb` = [`GHOST_VICTIM_PDB`] — an attacker-retained channel bound to a victim's
/// page directory, which is #14 in one hop.
///
/// The fix may not refuse the recycle (RM recycles by design), so it is a **MISS**: the
/// ghost projects with no VAS at all and its use faults loudly by name. Both halves are
/// asserted — the ghost gets nothing, *and* the victim keeps everything, including the
/// `by_pdb` route to its own plane.
#[test]
fn a_ghost_channels_declared_hvaspace_never_binds_the_next_tenant_of_its_namespace() {
    use kayfabe_core::ProcAnchor;
    use kayfabe_core::rmgraph::{AllocFacts, NodeKey};
    use kayfabe_mocks::mock_classes as mc;
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let ghost_gen = ClientKey::first(GHOST_NS);
    let victim_gen = ClientKey {
        client: GHOST_NS,
        incarnation: 1,
    };

    for ev in [
        kayfabe_tests::client_root(GHOST_KEEPER),
        kayfabe_tests::client_root(GHOST_NS),
        RmEvent::Alloc {
            client: GHOST_NS,
            parent: HObject(GHOST_NS.0),
            handle: GHOST_DEV1,
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        // The channel, declaring an `hVASpace` at a handle value it will hand back. (The
        // VASpace itself is never allocated: what is under test is the RESOLUTION of the
        // declared fact, and a fact whose target never arrives is the ordinary DEFER.)
        RmEvent::Alloc {
            client: GHOST_NS,
            parent: GHOST_DEV1,
            handle: GHOST_CHAN,
            class: mc::CHANNEL_GR,
            facts: AllocFacts {
                h_vaspace: Some(GHOST_HVAS),
                userd_flags: MockArch::userd_flags_for(VChid(0x77)),
                ..Default::default()
            },
        },
        // The alias that will keep the channel alive past its namespace's death.
        RmEvent::Dup {
            src: NodeKey::new(GHOST_NS, GHOST_CHAN),
            dst: NodeKey::new(GHOST_KEEPER, GHOST_ALIAS),
        },
        RmEvent::Free {
            client: GHOST_NS,
            handle: HObject(GHOST_NS.0),
        },
        // ★ The victim: an unrelated later process handed the same `hClient`, which
        // allocates its own VASpace at the very handle value the ghost still names.
        kayfabe_tests::client_root(GHOST_NS),
        RmEvent::Alloc {
            client: GHOST_NS,
            parent: HObject(GHOST_NS.0),
            handle: GHOST_DEV2,
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: GHOST_NS,
            parent: GHOST_DEV2,
            handle: GHOST_HVAS,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: GHOST_NS,
            vaspace: GHOST_HVAS,
            pdb: GHOST_VICTIM_PDB,
        },
    ] {
        g.apply(&arch, ev).expect("every event is legal RM traffic");
    }

    let b = project(&g, &arch, &NO_CONDEMNED).expect("projects cleanly");
    // ---- ★ PRECONDITION (non-vacuity): the ghost really is still alive and really is a
    // component of its own, distinct from the victim's. A test that lost the ghost here
    // would assert the absence of a binding that had no chance to exist.
    let ghost_proc = b
        .procs
        .iter()
        .find(|p| p.anchor == ProcAnchor(ghost_gen))
        .expect("★ the ghost's declaration must still project — RM says it is alive");
    let victim_proc = b
        .procs
        .iter()
        .find(|p| p.anchor == ProcAnchor(victim_gen))
        .expect("the victim has its own component");
    assert_eq!(
        ghost_proc.channels.len(),
        1,
        "★ the ghost channel must still be projected (kept alive by the keeper's alias)"
    );

    // ---- ★★ THE BREACH: the ghost's declared `hVASpace` must resolve to NOTHING.
    let ghost_chan = ghost_proc.channels.values().next().expect("one channel");
    assert_eq!(
        (ghost_chan.vas_origin, ghost_chan.vas_pdb),
        (None, None),
        "★★ the ghost channel BOUND the next tenant of its recycled `hClient` — an \
         attacker-retained channel on a victim's page directory"
    );

    // ---- …and the victim keeps its own plane, whole. The fix is a MISS for the ghost,
    // never a refusal of the victim's perfectly legal allocation.
    assert_eq!(
        b.by_pdb
            .get(&(GpuId::ZERO, GHOST_VICTIM_PDB))
            .map(|&(a, _)| a),
        Some(ProcAnchor(victim_gen)),
        "the victim's own address plane must route to the victim"
    );
    assert_eq!(
        victim_proc
            .vases
            .values()
            .filter_map(|f| f.pdb)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([GHOST_VICTIM_PDB]),
        "the victim owns its VASpace and its page-directory base"
    );
    assert_eq!(
        ghost_proc.vases.len(),
        0,
        "the ghost's component owns no address plane — it never allocated one"
    );
}

/// The guest-kernel client whose root is freed while a user client's alias keeps its
/// object alive — an ORPHANED kernel declaration.
const ORPHAN_KERNEL: HClient = HClient(0xE7);
/// The user client holding the alias.
const ORPHAN_KEEPER: HClient = HClient(0xE8);
/// The handle it holds the kernel object under.
const ORPHAN_K_ALIAS: HObject = HObject(0x8200_0001);

/// ★★★ **An ORPHANED kernel declaration is still the guest kernel's** (`l1_concurrency.md`
/// §12.44 — the INV6 half of the same collapse).
///
/// §12.39's rule is that the assignment pass must classify a component by the kind its
/// namespace **declared**, never by an absence: a declaration whose client root has been
/// freed still owns live resources (a foreign alias refcounts them) and must keep its
/// side of the user/kernel line — filing the guest kernel's orphan as a USER boundary is
/// the `FwdFault::SystemDataPlane` shape, and the spine would mint it an isolate and a
/// GPA arena.
///
/// `RmGraph::client_kinds` answers the *other* question ("is there a LIVE declared root
/// here?") and is the grouping predicate's input; reading the system component's expected
/// membership off it — which the A1 fuzz checker did — made an orphaned kernel declaration
/// read as "no longer a kernel client" while `project` (correctly) kept it in the system
/// component. This pins the distinction from both sides.
#[test]
fn an_orphaned_kernel_declaration_stays_in_the_system_component() {
    use kayfabe_core::project::SYSTEM_ANCHOR;
    use kayfabe_core::rmgraph::NodeKey;
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let orphan = ClientKey::first(ORPHAN_KERNEL);

    for ev in [
        kayfabe_tests::client_root(ORPHAN_KEEPER),
        kayfabe_tests::kernel_client_root(ORPHAN_KERNEL),
        RmEvent::Dup {
            src: NodeKey::new(ORPHAN_KERNEL, HObject(ORPHAN_KERNEL.0)),
            dst: NodeKey::new(ORPHAN_KEEPER, ORPHAN_K_ALIAS),
        },
    ] {
        g.apply(&arch, ev).expect("legal traffic");
    }
    // ★ PRECONDITION (non-vacuity): while its root is live the kernel client is in the
    // system component and the user client is NOT — so the assertion after the free is
    // about a change of liveness, not about a component that was never populated.
    let before = project(&g, &arch, &NO_CONDEMNED).expect("projects");
    assert_eq!(before.system.clients, BTreeSet::from([orphan]));
    assert_eq!(
        before
            .procs
            .iter()
            .map(|p| p.clients.clone())
            .collect::<Vec<_>>(),
        vec![BTreeSet::from([ClientKey::first(ORPHAN_KEEPER)])],
        "★ precondition: a dup INTO a kernel client is a REFERENCE, never a merge"
    );

    // The kernel client frees its root. The keeper's alias keeps the object — hence the
    // declaration — alive, with no live root.
    g.apply(
        &arch,
        RmEvent::Free {
            client: ORPHAN_KERNEL,
            handle: HObject(ORPHAN_KERNEL.0),
        },
    )
    .expect("the kernel client frees its root");
    assert!(
        !g.client_kinds().any(|(k, _)| k.client == ORPHAN_KERNEL),
        "the orphan must have no LIVE root — that is what makes it an orphan",
    );
    let after = project(&g, &arch, &NO_CONDEMNED).expect("projects");
    assert_eq!(
        after.system.clients,
        BTreeSet::from([orphan]),
        "★★ the guest kernel's orphaned declaration left the system component"
    );
    assert_eq!(
        after
            .procs
            .iter()
            .map(|p| (p.anchor, p.clients.clone()))
            .collect::<Vec<_>>(),
        vec![(
            kayfabe_core::ProcAnchor(ClientKey::first(ORPHAN_KEEPER)),
            BTreeSet::from([ClientKey::first(ORPHAN_KEEPER)])
        )],
        "★★ the guest kernel's orphan was filed as a USER boundary — the spine would mint \
         it an isolate, a GPA arena and a data plane it must never have"
    );
    assert_ne!(after.system.anchor, after.procs[0].anchor);
    assert_eq!(after.system.anchor, SYSTEM_ANCHOR);
}

/// The namespace whose engine object is orphaned and whose `hClient` is then recycled.
const REFINE_NS: HClient = HClient(0xE9);
/// The client keeping that engine object alive.
const REFINE_KEEPER: HClient = HClient(0xEA);
/// The handle value the ghost engine object names as its `parent` — and that the
/// namespace's next tenant allocates its GR channel at.
const REFINE_PARENT: HObject = HObject(0x8300_0020);
/// The ghost engine object's own handle.
const REFINE_ENG: HObject = HObject(0x8300_0030);
/// The handle [`REFINE_KEEPER`] holds the ghost engine object under.
const REFINE_ALIAS: HObject = HObject(0x8400_0001);
/// The victim tenant's page-directory base.
const REFINE_PDB: Pdb = Pdb(0x5a00_0000);

/// ★★★ **A ghost ENGINE OBJECT never refines the channel of the next tenant of its
/// namespace** (`l1_concurrency.md` §12.44 — the same rule on `parent`).
///
/// `ChannelFacts::engine` is refined by *the engine object allocated on the channel*
/// (`execution_plane.md` §2.1/§2.2): an NVENC session on a GR-class channel makes it an
/// `NvEnc` context, which selects the completion arm and the routing. The refinement hops
/// through the engine object's `parent`, and `parent` is a handle in the **allocating
/// client's** table — so a ghost engine object kept alive by a foreign alias would
/// otherwise reach whatever the recycled `hClient`'s next tenant allocated at that handle
/// number and silently retype **the victim's** channel.
///
/// Less severe than the `hVASpace` case ([`a_ghost_channels_declared_hvaspace_never_binds_the_next_tenant_of_its_namespace`])
/// — it mistypes rather than cross-binds — but the same organ and the same rule, so it is
/// pinned rather than left to argument.
#[test]
fn a_ghost_engine_object_never_retypes_the_next_tenants_channel() {
    use kayfabe_arch::ids::EngineKind;
    use kayfabe_core::rmgraph::{AllocFacts, NodeKey};
    use kayfabe_mocks::mock_classes as mc;
    let arch = MockArch::new();
    let mut g = RmGraph::new();

    for ev in [
        kayfabe_tests::client_root(REFINE_KEEPER),
        kayfabe_tests::client_root(REFINE_NS),
        RmEvent::Alloc {
            client: REFINE_NS,
            parent: HObject(REFINE_NS.0),
            handle: HObject(0x8300_0001),
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        // An NVENC engine object naming a `parent` handle it will hand back.
        RmEvent::Alloc {
            client: REFINE_NS,
            parent: REFINE_PARENT,
            handle: REFINE_ENG,
            class: mc::NVENC,
            facts: AllocFacts::default(),
        },
        RmEvent::Dup {
            src: NodeKey::new(REFINE_NS, REFINE_ENG),
            dst: NodeKey::new(REFINE_KEEPER, REFINE_ALIAS),
        },
        RmEvent::Free {
            client: REFINE_NS,
            handle: HObject(REFINE_NS.0),
        },
        // ★ The victim: the same `hClient`, with a plain GR channel at that handle value.
        kayfabe_tests::client_root(REFINE_NS),
        RmEvent::Alloc {
            client: REFINE_NS,
            parent: HObject(REFINE_NS.0),
            handle: HObject(0x8300_0002),
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: REFINE_NS,
            parent: HObject(0x8300_0002),
            handle: HObject(0x8300_0010),
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: REFINE_NS,
            vaspace: HObject(0x8300_0010),
            pdb: REFINE_PDB,
        },
        RmEvent::Alloc {
            client: REFINE_NS,
            parent: HObject(0x8300_0002),
            handle: REFINE_PARENT,
            class: mc::CHANNEL_GR,
            facts: AllocFacts {
                h_vaspace: Some(HObject(0x8300_0010)),
                userd_flags: MockArch::userd_flags_for(VChid(0x88)),
                ..Default::default()
            },
        },
    ] {
        g.apply(&arch, ev).expect("every event is legal RM traffic");
    }

    let b = project(&g, &arch, &NO_CONDEMNED).expect("projects cleanly");
    let victim = b
        .procs
        .iter()
        .find(|p| {
            p.anchor
                == kayfabe_core::ProcAnchor(ClientKey {
                    client: REFINE_NS,
                    incarnation: 1,
                })
        })
        .expect("the victim has its own component");
    // ★ PRECONDITION (non-vacuity): the ghost engine object is still a live resource, so
    // the refinement pass really does visit it.
    assert!(
        g.nodes()
            .any(|n| n.key == NodeKey::new(REFINE_NS, REFINE_ENG)
                && matches!(n.kind, kayfabe_arch::ObjectKind::EngineObject { .. })),
        "★ the ghost engine object must still be live (the keeper's alias holds it)"
    );
    assert_eq!(
        victim
            .channels
            .values()
            .map(|f| f.engine)
            .collect::<Vec<_>>(),
        vec![EngineKind::GrCompute],
        "★★ a dead namespace's NVENC object retyped the NEXT tenant's GR channel"
    );
}
