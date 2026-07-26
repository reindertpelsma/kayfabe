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
use kayfabe_core::rmgraph::{RmEvent, RmGraph};
use kayfabe_fwd::{FwdFault, handle_doorbell, publish_backing};
use kayfabe_mocks::{MockArch, MockIsolateFactory};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

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
        b.system.clients,
        std::collections::BTreeSet::from([UVM]),
        "the one UVM session client is the guest KERNEL's, not any process's"
    );
    assert_eq!(b.system.anchor, SYSTEM_ANCHOR);

    let proc_a = b
        .procs
        .iter()
        .find(|p| p.clients.contains(&A))
        .expect("proc A present");
    let proc_b = b
        .procs
        .iter()
        .find(|p| p.clients.contains(&B))
        .expect("proc B present");
    // A merged with its second USER client (genuine sharing = one blast radius) and
    // with nothing else. B is alone. Neither contains the kernel client.
    assert_eq!(
        proc_a.clients,
        std::collections::BTreeSet::from([A, A2]),
        "a user↔user dup merges; the UVM dup does not"
    );
    assert_eq!(proc_b.clients, std::collections::BTreeSet::from([B]));
    for p in &b.procs {
        assert!(
            !p.clients.contains(&UVM),
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

    let proc_a = b.procs.iter().find(|p| p.clients.contains(&A)).unwrap();
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
    assert_eq!(reference.system.clients.len(), 1);
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
    // Concretely: proc A's clients are {0xc1d00067, 0xc1d0006b}; its anchor is the
    // smaller. Note the UVM session (0xc1d00069) lies BETWEEN them and is still not a
    // member — the anchor is a minimum over the component, never over the handle space.
    let proc_a = b.procs.iter().find(|p| p.clients.contains(&A)).unwrap();
    assert_eq!(proc_a.anchor.0, A, "0xc1d00067 < 0xc1d0006b, so A anchors");
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
    assert_ne!(pa.host_va, pb.host_va, "…and distinct host VA");

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
                gpu.procs[p].clients.clone(),
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
        assert_eq!(&gpu.procs[pid].clients, clients);
    }
    let pid_c = gpu.spine.by_pdb[&(GpuId::ZERO, C_PDB)];
    assert!(pid_c != pid_a && pid_c != pid_b);
    // The session client still belongs to exactly one component: the system's.
    assert_eq!(gpu.system.clients, std::collections::BTreeSet::from([UVM]));
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
    assert_eq!(gpu.system.clients, std::collections::BTreeSet::from([UVM]));
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
        vec![(A, kayfabe_arch::ClientKind::User { pid: A.0 })],
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
        vec![(A, kayfabe_arch::ClientKind::Kernel)],
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
        !g.client_kinds().any(|(c, _)| c == B),
        "a namespace whose root handle is freed declares nothing",
    );
    g.apply(&arch, kayfabe_tests::kernel_client_root(B))
        .expect("★ the emptied namespace may declare a fresh root");
    assert_eq!(
        g.client_kinds().collect::<Vec<_>>(),
        vec![
            (A, kayfabe_arch::ClientKind::User { pid: A.0 }),
            (B, kayfabe_arch::ClientKind::Kernel),
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
        g.client_kinds().find(|(c, _)| *c == B).map(|(_, k)| k),
        Some(kayfabe_arch::ClientKind::Kernel),
        "the dead alias pruned the LIVE root's declaration",
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
        before.procs[0].clients,
        std::collections::BTreeSet::from([A])
    );

    // ---- Declared USER first, THEN the dup: it is a grouping edge.
    let mut declared = s.clone();
    declared.push(kayfabe_tests::client_root(A2));
    declared.push(planted);
    let joined = project(&graph_of(&declared.events), &arch, &NO_CONDEMNED).unwrap();
    assert_eq!(joined.procs.len(), 1, "declared user → the dup merges");
    assert_eq!(
        joined.procs[0].clients,
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
        referenced.procs[0].clients,
        std::collections::BTreeSet::from([A])
    );
    assert_eq!(
        referenced.system.clients,
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
    assert!(
        !reference.by_pdb.contains_key(&(GpuId::ZERO, FIRST_PDB)),
        "★★ the SUPERSEDED declaration's address plane must belong to nobody"
    );
    let second = reference
        .procs
        .iter()
        .find(|p| p.clients.contains(&RECYCLED))
        .expect("the second tenant has a component");
    assert!(
        !second.clients.contains(&KEEPER),
        "★★ the keeper's alias to the FIRST tenant's VASpace must not merge it with the \
         SECOND — the edge's `src` names a recyclable VALUE, not the namespace it was \
         allocated in"
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
