//! ★★★ THE MISS TAXONOMY, MADE EXECUTABLE (`l1_concurrency.md` §12.38).
//!
//! Every absence in the core is answered in one of three ways
//! (`kayfabe_core` crate docs):
//!
//! | # | category | rule |
//! |---|---|---|
//! | **1** | DEFER (protocol) | the guest may *legally* send this before that |
//! | **2** | ★ DEFER (observation) | the protocol DOES order it, but we may not have observed the earlier fact |
//! | **3** | FAULT | the protocol forbids the ordering **and** we would have observed the earlier fact |
//!
//! §12.30 inventoried the sites and had no name for category 2 — which is exactly why it
//! mis-filed the one site that mattered, a `DUP_OBJECT` into a never-declared client
//! namespace, as a deferral. That deferral was a cross-process isolation break.
//!
//! **This file is the executable half of the corrected inventory**, and it asserts the two
//! things the previous pass could not:
//!
//! - **Every FAULT is refused with its EXACT variant** — never `is_err()`, never a
//!   `matches!` wildcard over the field that carries the meaning. A canary that passes for
//!   the wrong reason is §12.10's lesson.
//! - **Every DEFER actually RESOLVES when the fact arrives.** A deferral with no
//!   re-evaluation path is a *hang*, and until now nothing proved there wasn't one: the
//!   suite proved deferrals did not crash, not that they ever completed. Each test below
//!   asserts the *absence* first (nothing materialized, nothing routed, the use faults by
//!   name) and then the *arrival* (the same state, now correct), so a site that silently
//!   dropped the fact fails here even though it "deferred" correctly.
//!
//! Sites proved elsewhere, deliberately not duplicated: `fwd::checkout` → `Ok(None)`
//! (`l1_mean::mean_run`'s pool-saturation census, §12.29), `fwd::commit_*` →
//! `Refusal { retry: true }` (`retry_ledger.rs`, §12.28), the collision guards
//! (`security_boundary.rs`, `fuzz_rmgraph_invariants.rs`), and the condemnation split
//! (`l1_mean.rs`, §12.13/§12.37).

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use std::collections::BTreeSet;

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, GpuError};
use kayfabe_core::project::{NO_CONDEMNED, project};
use kayfabe_core::rmgraph::{AllocFacts, NodeKey, RmEvent, RmGraph, RmGraphError};
use kayfabe_fwd::{FwdFault, handle_doorbell, resolve};
use kayfabe_mocks::{MockArch, MockIsolateFactory, mock_classes as mc};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

const C: HClient = HClient(0xC1D0_0067);
/// A namespace that is NEVER declared — the use-before-exist destination.
const GHOST: HClient = HClient(0xC1D0_00FF);
const PDB0: Pdb = Pdb(0x3401_000);
const VA: GpuVa = GpuVa(0x2_0020_0000);
const MAP_LEN: u64 = 0x10000;

const H_ROOT: HObject = HObject(0x5c00_0000);
const H_DEV: HObject = HObject(0x5c00_0001);
const H_VAS: HObject = HObject(0x5c00_0010);
const H_TSG: HObject = HObject(0x5c00_0012);
const H_GR: HObject = HObject(0x5c00_0019);
const H_MEM: HObject = HObject(0x5c00_0100);
const GR_VCHID: VChid = VChid(0x10);
const MEM_PHYS: u64 = 0x9_0000_0000;

fn fresh_graph() -> (MockArch, RmGraph) {
    (MockArch::new(), RmGraph::new())
}

fn fresh_gpu() -> Guarded<Gpu> {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    Guarded::new(
        "miss_taxonomy::fresh_gpu",
        Gpu::new(arch, Box::new(factory), gpa).expect("device realizes"),
        rec,
    )
}

/// `C`'s client root — the one fact that is ALWAYS on the GSP wire, and therefore the one
/// whose absence is category 3 rather than category 2.
fn root_of(c: HClient) -> RmEvent {
    RmEvent::Alloc {
        client: c,
        parent: HObject(c.0),
        handle: HObject(c.0),
        class: mc::CLIENT,
        facts: kayfabe_tests::user_client(c),
    }
}

fn device(client: HClient, parent: HObject, handle: HObject) -> RmEvent {
    RmEvent::Alloc {
        client,
        parent,
        handle,
        class: mc::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    }
}

fn vaspace(client: HClient, parent: HObject, handle: HObject) -> RmEvent {
    RmEvent::Alloc {
        client,
        parent,
        handle,
        class: mc::VASPACE,
        facts: AllocFacts::default(),
    }
}

fn memory(client: HClient, parent: HObject, handle: HObject, phys: u64) -> RmEvent {
    RmEvent::Alloc {
        client,
        parent,
        handle,
        class: mc::MEMORY,
        facts: AllocFacts {
            mem_phys: Some(phys),
            ..Default::default()
        },
    }
}

// =================================================================================
// CATEGORY 3 — FAULT: use-before-exist, refused with the EXACT variant
//
// The rule (`RmGraph::undeclared_namespace`): no event may name a client namespace that
// does not exist. RM resolves `hClient` first at every ioctl-reachable entry point and
// answers `NV_ERR_INVALID_OBJECT_HANDLE` / `NV_ERR_INVALID_CLIENT`
// (`ogkm src/nvidia/src/libraries/resserv/src/rs_server.c:778`, `:1503`, `:1674`,
// `:2218`; `rmapi/client.c:782`), so refusing loses no legal trace.
// =================================================================================

/// ★★ Every event shape that can NAME a namespace is refused when that namespace has
/// never declared a root — with the exact `UndeclaredClient`, carrying the exact client.
///
/// Table-driven on purpose: the rule is one central gate, so "did we remember this event
/// variant?" must be a list a reader can check against `RmEvent`'s own variants rather
/// than five tests that could each be missing.
#[test]
fn every_event_naming_an_undeclared_namespace_is_refused_by_name() {
    let (arch, mut g) = fresh_graph();
    g.apply(&arch, root_of(C)).expect("C declares itself");
    g.apply(&arch, device(C, H_ROOT, H_DEV))
        .expect("…and owns a device");

    let refused: Vec<(&str, RmEvent)> = vec![
        (
            "Alloc (non-root) into a ghost namespace",
            vaspace(GHOST, HObject(1), H_VAS),
        ),
        (
            "SetPageDir on a ghost namespace",
            RmEvent::SetPageDir {
                client: GHOST,
                vaspace: H_VAS,
                pdb: PDB0,
            },
        ),
        (
            "MapMemoryDma from a ghost namespace",
            RmEvent::MapMemoryDma {
                client: GHOST,
                vaspace: H_VAS,
                memory: H_MEM,
                va: VA,
                offset: 0,
                len: MAP_LEN,
            },
        ),
        (
            "Dup INTO a ghost namespace (★ the squat vector)",
            RmEvent::Dup {
                src: NodeKey::new(C, H_DEV),
                dst: NodeKey::new(GHOST, HObject(0x777)),
            },
        ),
        (
            "Dup OUT OF a ghost namespace",
            RmEvent::Dup {
                src: NodeKey::new(GHOST, HObject(0x777)),
                dst: NodeKey::new(C, HObject(0x900)),
            },
        ),
    ];

    let before: Vec<NodeKey> = g.nodes().map(|n| n.key).collect();
    for (what, ev) in refused {
        assert_eq!(
            g.apply(&arch, ev),
            Err(RmGraphError::UndeclaredClient(GHOST)),
            "★★ {what} must be refused with the EXACT `UndeclaredClient(GHOST)`"
        );
        assert_eq!(
            g.nodes().map(|n| n.key).collect::<Vec<_>>(),
            before,
            "{what}: the refusal mutated the graph"
        );
        assert!(
            !g.dups()
                .any(|(d, s)| d.client == GHOST || s.client == GHOST),
            "{what}: the refusal left a parked edge behind"
        );
    }
    assert!(
        !g.client_kinds().any(|(c, _)| c == GHOST),
        "the ghost namespace never entered the graph at all"
    );
}

/// ★ The root alloc itself is the ONE exemption — it is what CREATES the namespace, which
/// is why RM bypasses the client lock for exactly it (`serverAllocClient`,
/// `rs_server.c:764`). Asserted so the gate cannot be tightened into a deadlock where no
/// namespace can ever be declared.
#[test]
fn the_client_root_alloc_is_the_one_event_that_may_name_a_new_namespace() {
    let (arch, mut g) = fresh_graph();
    g.apply(&arch, root_of(GHOST))
        .expect("★ a client root declares its own namespace");
    // …and now everything else in that namespace is legal.
    g.apply(&arch, device(GHOST, HObject(GHOST.0), H_DEV))
        .expect("a device under the now-declared root");
    assert_eq!(g.client_kinds().count(), 1);
}

/// ★ `Free` and `Unmap` — the TEARDOWN verbs — are exempt, and the exemption is argued,
/// not convenient: a namespace with no root is indistinguishable from one whose root was
/// *just* freed, and a teardown verb arriving after its namespace died is a benign race a
/// real guest produces. `Free` keeps the more precise `FreeUnknown`; `Unmap` keeps its
/// documented idempotent-`Ok`.
///
/// The exemption costs nothing, and this test is the reason why: once no *creating* event
/// can name an undeclared namespace, an undeclared namespace holds no handles, so both
/// verbs are inert by construction.
#[test]
fn the_teardown_verbs_are_exempt_and_inert_on_a_namespace_that_never_existed() {
    let (arch, mut g) = fresh_graph();
    g.apply(&arch, root_of(C)).expect("C declares itself");
    let before: Vec<NodeKey> = g.nodes().map(|n| n.key).collect();

    assert_eq!(
        g.apply(
            &arch,
            RmEvent::Free {
                client: GHOST,
                handle: H_VAS
            }
        ),
        Err(RmGraphError::FreeUnknown(NodeKey::new(GHOST, H_VAS))),
        "a free in a namespace that never existed is the precise `FreeUnknown`, \
         not a namespace refusal — the handle is what was not there",
    );
    assert_eq!(
        g.apply(
            &arch,
            RmEvent::Unmap {
                client: GHOST,
                vaspace: H_VAS,
                va: VA
            }
        ),
        Ok(()),
        "an unmap racing a namespace's death stays idempotent",
    );
    assert_eq!(
        g.nodes().map(|n| n.key).collect::<Vec<_>>(),
        before,
        "and neither touched a thing"
    );
}

/// ★★ The consequence the `Alloc` arm closes, asserted as state rather than argued: an
/// object allocated into an undeclared namespace used to mint a whole user
/// `ProcBoundary` — isolate, GPA arena, routable `Vas` — anchored at a client of
/// **unknown `ClientKind`**, which is exactly the guess §12.27 refuses to make. Declaring
/// the root *afterwards* as `Kernel` then migrated those objects to the system component,
/// so a guest kernel could obtain a real user data plane (`FwdFault::SystemDataPlane`'s
/// whole purpose) simply by declaring its client root last.
///
/// The **end-state claim is asserted before the refusal claim**, on purpose: reverting the
/// `Alloc` arm must report the boundary that should not exist, not merely "the alloc was
/// accepted".
#[test]
fn an_object_allocated_into_an_undeclared_namespace_mints_no_boundary() {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    g.apply(&arch, root_of(C)).expect("C declares itself");
    g.apply(&arch, device(C, H_ROOT, H_DEV))
        .expect("C's device");

    // The ghost tries to build a whole routable address plane without ever declaring
    // itself. Results are collected, not asserted yet.
    let attempts: Vec<Result<(), RmGraphError>> = [
        device(GHOST, HObject(GHOST.0), H_DEV),
        vaspace(GHOST, H_DEV, H_VAS),
        RmEvent::SetPageDir {
            client: GHOST,
            vaspace: H_VAS,
            pdb: PDB0,
        },
    ]
    .into_iter()
    .map(|ev| g.apply(&arch, ev))
    .collect();

    // ---- The end state that must not exist.
    let b = project(&g, &arch, &NO_CONDEMNED).expect("projects");
    assert!(
        b.procs.iter().all(|p| !p.clients.contains(&GHOST)),
        "★★ a boundary was minted for a client whose `ClientKind` is UNKNOWN — the \
         grouping guess §12.27 exists to refuse, reached by omission instead of a default"
    );
    assert!(
        !b.by_pdb.contains_key(&(GpuId::ZERO, PDB0)),
        "★★ an undeclared client obtained a ROUTABLE address plane"
    );
    assert_eq!(
        b.procs.len(),
        1,
        "exactly one component — C's, and nothing the ghost tried to build"
    );
    assert_eq!(b.procs[0].clients, BTreeSet::from([C]));
    assert!(
        b.procs[0].vases.is_empty(),
        "the ghost's VAS was never accepted"
    );

    // ---- …and the mechanism: each attempt earned its own exact refusal.
    for r in attempts {
        assert_eq!(
            r,
            Err(RmGraphError::UndeclaredClient(GHOST)),
            "an event in an undeclared namespace is refused at acceptance"
        );
    }
}

// =================================================================================
// CATEGORY 2 — DEFER (observation): the protocol DOES order it, but the earlier fact is
// OBJECT-level and may never have reached the wire (only 25 of 82 measured dups do —
// `docs/reference/rm_semantics_measured.md` §3). Each must PARK **and RESOLVE**.
// =================================================================================

/// ★★ A dup's **source object** may be one RM saw and we did not. It parks — and the
/// deferral resolves the moment the source is observed.
///
/// This is the half of the dup taxonomy that must NOT become a fault: faulting it would
/// hang a legal guest whose source object simply never reached GSP.
///
/// It also pins the **grouping** half, which nothing else does directly since §12.38
/// repurposed `an_undeclared_client_merges_with_nobody_until_it_declares` into a refusal
/// test: a parked edge groups nothing, and the SAME edge becomes a grouping edge the
/// instant it is promoted (`project` skips edges whose origin does not resolve).
#[test]
fn defer_a_dup_source_object_parks_and_resolves_when_the_source_arrives() {
    let (arch, mut g) = fresh_graph();
    let peer = HClient(0xC1D0_006B);
    for ev in [root_of(C), root_of(peer)] {
        g.apply(&arch, ev).expect("both namespaces declare");
    }
    let alias = NodeKey::new(peer, HObject(0x7000_00FF));

    // ---- The absence: the source OBJECT has not been observed.
    g.apply(
        &arch,
        RmEvent::Dup {
            src: NodeKey::new(C, H_MEM),
            dst: alias,
        },
    )
    .expect("★ an unobserved source object PARKS — it may still arrive");
    assert!(
        g.origin_of(alias).is_none(),
        "a parked dup binds its dst to nothing"
    );
    assert_eq!(
        g.references(NodeKey::new(C, H_MEM)).count(),
        0,
        "and refcounts nothing"
    );
    let parked = project(&g, &arch, &NO_CONDEMNED).expect("projects");
    assert_eq!(
        parked.procs.len(),
        2,
        "★ a parked edge is not a grouping edge — two user clients, two components"
    );

    // ---- The arrival: the deferral RESOLVES.
    g.apply(&arch, device(C, H_ROOT, H_DEV))
        .expect("C's device");
    g.apply(&arch, memory(C, H_DEV, H_MEM, MEM_PHYS))
        .expect("★ the source object finally arrives");
    assert_eq!(
        g.origin_of(alias).map(|n| n.key),
        Some(NodeKey::new(C, H_MEM)),
        "★★ the parked dup was promoted — a deferral that never resolved would be a hang"
    );
    assert_eq!(
        g.references(NodeKey::new(C, H_MEM))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([NodeKey::new(C, H_MEM), alias]),
        "and the promotion took a real reference (faithful RM refcounting)"
    );
    let promoted = project(&g, &arch, &NO_CONDEMNED).expect("projects");
    assert_eq!(
        promoted.procs.len(),
        1,
        "★★ …and the SAME edge is now a grouping edge — user↔user sharing, one blast \
         radius. The deferral resolved in the projection too, not only in the graph"
    );
    assert_eq!(promoted.procs[0].clients, BTreeSet::from([C, peer]));
}

/// ★★ A `SET_PAGE_DIRECTORY` whose VASpace has not been observed parks, and drains onto
/// the resource the moment the VASpace alloc lands.
///
/// Category 2, not 1: RM's control path requires its object to exist
/// (`serverControl` → `clientGetResourceRef`, `ogkm rs_server.c:1560`) — but the VASpace
/// alloc is an object-level fact, so the gap is real and the deferral is right.
#[test]
fn defer_a_parked_setpagedir_resolves_when_its_vaspace_arrives() {
    let (arch, mut g) = fresh_graph();
    g.apply(&arch, root_of(C)).expect("C declares itself");
    g.apply(&arch, device(C, H_ROOT, H_DEV))
        .expect("C's device");

    g.apply(
        &arch,
        RmEvent::SetPageDir {
            client: C,
            vaspace: H_VAS,
            pdb: PDB0,
        },
    )
    .expect("★ a PDB declared on an unobserved handle PARKS");
    assert_eq!(
        g.pdb_of(NodeKey::new(C, H_VAS)),
        None,
        "the parked fact is not yet ATTRIBUTED — there is no resource to attribute it to, \
         and inventing one would be the guess MISS=FAULT forbids",
    );
    assert_eq!(g.nodes().count(), 2, "no VASpace resource exists yet");

    g.apply(&arch, vaspace(C, H_DEV, H_VAS))
        .expect("★ the VASpace finally arrives");
    assert_eq!(
        g.pdb_of(NodeKey::new(C, H_VAS)),
        Some(PDB0),
        "★★ the parked PDB drained onto the resource"
    );
    // And it is on the RESOURCE, not the parked table: it survives a re-read after the
    // handle is aliased elsewhere, which only a drained fact does.
    assert!(
        g.nodes().any(|n| n.key == NodeKey::new(C, H_VAS)),
        "the VASpace resource is live"
    );
}

/// ★★ A `MAP_MEMORY_DMA` whose VASpace *and* memory are both unobserved parks, and
/// replays into a live mapping the moment both resolve.
#[test]
fn defer_a_parked_map_replays_when_both_endpoints_arrive() {
    let (arch, mut g) = fresh_graph();
    g.apply(&arch, root_of(C)).expect("C declares itself");
    g.apply(&arch, device(C, H_ROOT, H_DEV))
        .expect("C's device");

    g.apply(
        &arch,
        RmEvent::MapMemoryDma {
            client: C,
            vaspace: H_VAS,
            memory: H_MEM,
            va: VA,
            offset: 0,
            len: MAP_LEN,
        },
    )
    .expect("★ a map with two unobserved endpoints PARKS");
    assert_eq!(g.mappings().count(), 0, "no live mapping yet");

    g.apply(&arch, vaspace(C, H_DEV, H_VAS))
        .expect("the VAS arrives");
    assert_eq!(
        g.mappings().count(),
        0,
        "still parked — ONE endpoint is not enough"
    );
    g.apply(&arch, memory(C, H_DEV, H_MEM, MEM_PHYS))
        .expect("★ the memory arrives");

    let m = g.mappings().next().expect("★★ the parked map replayed");
    assert_eq!(m.va, VA);
    assert_eq!(
        m.mem_phys,
        Some(MEM_PHYS),
        "and it forward-populated the declared backing"
    );
    assert_eq!(
        g.map_ref_count(NodeKey::new(C, H_MEM)),
        1,
        "the replay took the mapping's reference on the memory"
    );
}

/// ★★ An object whose `Device` ancestor has not been observed has **no GPU target** — and
/// the answer is `None`, never a guessed `GpuId::ZERO` (G9/§12.21). The deferral resolves
/// by the `Device`-triggered back-fill.
#[test]
fn defer_an_unresolved_gpu_target_resolves_when_the_device_arrives() {
    let (arch, mut g) = fresh_graph();
    g.apply(&arch, root_of(C)).expect("C declares itself");
    // The VASpace names a parent that has not arrived (order tolerance on the PARENT —
    // an object-level fact, hence category 2).
    g.apply(&arch, vaspace(C, H_DEV, H_VAS))
        .expect("a VASpace under an unobserved device");
    g.apply(
        &arch,
        RmEvent::SetPageDir {
            client: C,
            vaspace: H_VAS,
            pdb: PDB0,
        },
    )
    .expect("with a declared PDB");

    assert_eq!(
        g.gpu_of(NodeKey::new(C, H_VAS)),
        None,
        "★ no Device ancestor ⇒ NO target — never a default-GPU0 guess"
    );
    let b = project(&g, &arch, &NO_CONDEMNED).expect("projects");
    assert!(
        !b.by_pdb.contains_key(&(GpuId::ZERO, PDB0)),
        "an unroutable VAS enters no routing map"
    );

    g.apply(&arch, device(C, H_ROOT, H_DEV))
        .expect("★ the Device finally arrives");
    assert_eq!(
        g.gpu_of(NodeKey::new(C, H_VAS)),
        Some(GpuId::ZERO),
        "★★ the back-fill resolved the target — a deferral that never resolved is a hang"
    );
    let b = project(&g, &arch, &NO_CONDEMNED).expect("projects");
    assert_eq!(
        b.by_pdb.get(&(GpuId::ZERO, PDB0)).map(|x| x.1),
        Some(NodeKey::new(C, H_VAS)),
        "…and the VAS now routes"
    );
}

/// ★★ A channel whose declared `hVASpace` does not resolve materializes with
/// `vas_pdb: None` and rings **nothing** — the exact `FwdFault::NoVas` — and the deferral
/// resolves when the VASpace and its PDB arrive.
///
/// This is the taxonomy's clearest pairing in one test: the SAME absence is a DEFER in
/// derivation (`Gpu::sync_proc_to_boundary` materializes no `Vas`) and a FAULT at use
/// (`kayfabe_fwd::gate_working_set_in` — at ring time there is no "later").
#[test]
fn defer_a_channel_with_an_unresolved_vaspace_faults_at_use_then_resolves() {
    let mut gpu = fresh_gpu();
    for ev in [
        root_of(C),
        device(C, H_ROOT, H_DEV),
        // The TSG and channel name a VASpace handle that has not been observed.
        RmEvent::Alloc {
            client: C,
            parent: H_DEV,
            handle: H_TSG,
            class: mc::TSG,
            facts: AllocFacts {
                h_vaspace: Some(H_VAS),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: C,
            parent: H_TSG,
            handle: H_GR,
            class: mc::CHANNEL_GR,
            facts: AllocFacts {
                h_vaspace: Some(H_VAS),
                userd_flags: MockArch::userd_flags_for(GR_VCHID),
                ..Default::default()
            },
        },
    ] {
        gpu.apply(ev).expect("the partial bring-up applies");
    }

    // ---- The absence, at USE: the ring is refused BY NAME, not served on a guess.
    let pid = *gpu
        .spine
        .by_vchid
        .get(&(GpuId::ZERO, GR_VCHID))
        .map(|(p, _)| p)
        .expect("the channel routes — it has a resolvable Device");
    let cid = gpu.spine.by_vchid[&(GpuId::ZERO, GR_VCHID)].1;
    assert_eq!(
        gpu.procs[&pid].channels[&cid].vas_pdb, None,
        "★ the channel materialized with no VAS — deferred, not guessed"
    );
    assert_eq!(
        handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(GR_VCHID), &[VA]),
        Err(FwdFault::NoVas(cid)),
        "★★ at ring time there is no 'later' — the EXACT `NoVas`, never a served ring"
    );

    // ---- The arrival: the deferral resolves, and the same channel now works.
    gpu.apply(vaspace(C, H_DEV, H_VAS))
        .expect("the VASpace arrives");
    gpu.apply(RmEvent::SetPageDir {
        client: C,
        vaspace: H_VAS,
        pdb: PDB0,
    })
    .expect("…and binds its page directory");
    assert_eq!(
        gpu.procs[&pid].channels[&cid].vas_pdb,
        Some(PDB0),
        "★★ the channel's VAS resolved — same `ChanId`, no churn"
    );
    kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&pid).unwrap(),
        GpuId::ZERO,
        PDB0,
        VA,
        0x1000,
    )
    .expect("the now-routable VAS publishes");
    let out = handle_doorbell(&mut gpu, GpuId::ZERO, MockArch::token_for(GR_VCHID), &[VA])
        .expect("★★ and the SAME channel now rings");
    assert_eq!(out.proc, pid, "…on its own proc");
}

/// ★★ A channel whose **Device** has not been observed is not materialized at all — and
/// its `ChanId` slot is minted anyway, so the slot is STABLE across the apply that
/// finally materializes it. (A re-minted `ChanId` would be an observable churn the
/// deferral is supposed to avoid.)
#[test]
fn defer_an_unrouted_channel_keeps_a_stable_chanid_until_its_device_arrives() {
    let mut gpu = fresh_gpu();
    for ev in [
        root_of(C),
        vaspace(C, H_DEV, H_VAS),
        RmEvent::SetPageDir {
            client: C,
            vaspace: H_VAS,
            pdb: PDB0,
        },
        RmEvent::Alloc {
            client: C,
            parent: H_DEV,
            handle: H_GR,
            class: mc::CHANNEL_GR,
            facts: AllocFacts {
                h_vaspace: Some(H_VAS),
                userd_flags: MockArch::userd_flags_for(GR_VCHID),
                ..Default::default()
            },
        },
    ] {
        gpu.apply(ev).expect("the device-less bring-up applies");
    }

    let key = NodeKey::new(C, H_GR);
    let pid = *gpu
        .procs
        .keys()
        .next()
        .expect("C has a proc (it owns objects)");
    let cid_before = *gpu.procs[&pid]
        .chan_ids
        .get(&key)
        .expect("★ the ChanId slot is minted even while the channel is unroutable");
    assert!(
        !gpu.procs[&pid].channels.contains_key(&cid_before),
        "★ …but nothing is materialized: no target ⇒ no Channel, never a GPU0 guess"
    );
    assert!(
        !gpu.spine.by_vchid.contains_key(&(GpuId::ZERO, GR_VCHID)),
        "and it enters no routing map"
    );

    gpu.apply(device(C, H_ROOT, H_DEV))
        .expect("★ the Device finally arrives");
    assert_eq!(
        gpu.procs[&pid].chan_ids.get(&key),
        Some(&cid_before),
        "★★ the ChanId slot is the SAME one — the deferral did not churn the identity"
    );
    assert_eq!(
        gpu.procs[&pid].channels[&cid_before].vas_pdb,
        Some(PDB0),
        "★★ and the channel materialized, fully resolved"
    );
}

/// ★★ `Gpu::sync_rpc_mappings`' two deferrals, end to end on the runtime: a mapping whose
/// VAS has no PDB yet populates NOTHING, and populates correctly the moment
/// `SET_PAGE_DIRECTORY` lands.
///
/// THE canonical exception the whole taxonomy is written around — the guest legitimately
/// maps before it binds a page directory.
#[test]
fn defer_an_rpc_mapping_with_no_pdb_populates_when_setpagedir_lands() {
    let mut gpu = fresh_gpu();
    for ev in [
        root_of(C),
        device(C, H_ROOT, H_DEV),
        vaspace(C, H_DEV, H_VAS),
        memory(C, H_DEV, H_MEM, MEM_PHYS),
        RmEvent::MapMemoryDma {
            client: C,
            vaspace: H_VAS,
            memory: H_MEM,
            va: VA,
            offset: 0,
            len: MAP_LEN,
        },
    ] {
        gpu.apply(ev).expect("map-before-bind is a LEGAL ordering");
    }

    // ---- The absence: no PDB ⇒ no `Vas`, no routing, and the use faults by name.
    assert!(
        gpu.spine.by_pdb.is_empty(),
        "★ a VAS with no declared PDB routes nowhere"
    );
    assert_eq!(
        resolve(&gpu, GpuId::ZERO, PDB0, VA),
        Err(FwdFault::UnknownPdb {
            gpu: GpuId::ZERO,
            pdb: PDB0
        }),
        "★ …and its use is the EXACT `UnknownPdb`, never a fallback walk"
    );

    // ---- The arrival: the mapping populates, with the DECLARED backing.
    gpu.apply(RmEvent::SetPageDir {
        client: C,
        vaspace: H_VAS,
        pdb: PDB0,
    })
    .expect("★ SET_PAGE_DIRECTORY finally arrives");
    let (binding, off) =
        resolve(&gpu, GpuId::ZERO, PDB0, GpuVa(VA.0 + 0x40)).expect("★★ the mapping populated");
    assert_eq!(off, 0x40);
    assert_eq!(
        binding.phys, MEM_PHYS,
        "★★ forward-populated to the guest-DECLARED backing — never a reverse resolve"
    );
}

/// ★★ `sync_rpc_mappings`' OTHER deferral, on the multi-GPU axis: a mapping whose VAS has
/// no resolvable `Device` ancestor populates nothing — and populates the moment the Device
/// arrives. Deferring is what keeps `GpuId::ZERO` from being *guessed* (G9, §12.21).
///
/// The distinction from the PDB arm matters: here the PDB is known all along, so a core
/// that guessed a target would have produced a perfectly plausible, perfectly wrong
/// binding on GPU 0 rather than an obvious blank.
#[test]
fn defer_an_rpc_mapping_with_no_gpu_target_populates_when_the_device_lands() {
    let mut gpu = fresh_gpu();
    for ev in [
        root_of(C),
        // The VASpace names a Device that has not arrived.
        vaspace(C, H_DEV, H_VAS),
        RmEvent::SetPageDir {
            client: C,
            vaspace: H_VAS,
            pdb: PDB0,
        },
        memory(C, H_DEV, H_MEM, MEM_PHYS),
        RmEvent::MapMemoryDma {
            client: C,
            vaspace: H_VAS,
            memory: H_MEM,
            va: VA,
            offset: 0,
            len: MAP_LEN,
        },
    ] {
        gpu.apply(ev).expect("a device-less bring-up applies");
    }

    // ---- The absence: a KNOWN PDB that routes NOWHERE, because the target is unknown.
    assert!(
        gpu.spine.by_pdb.is_empty(),
        "★ a VAS with no resolvable target routes nowhere — never onto GPU 0"
    );
    assert_eq!(
        resolve(&gpu, GpuId::ZERO, PDB0, VA),
        Err(FwdFault::UnknownPdb {
            gpu: GpuId::ZERO,
            pdb: PDB0
        }),
        "★★ …and probing GPU 0 with the right PDB gets the EXACT `UnknownPdb`, \
         not the plausible-and-wrong binding a default-target guess would produce",
    );

    // ---- The arrival: the deferral resolves onto the DECLARED target.
    gpu.apply(device(C, H_ROOT, H_DEV))
        .expect("★ the Device finally arrives");
    let (binding, _) = resolve(&gpu, GpuId::ZERO, PDB0, VA).expect("★★ the mapping populated");
    assert_eq!(binding.phys, MEM_PHYS);
}

/// ★★ The FAULT that sits between those two deferrals: a mapping whose memory resolved and
/// declared **no backing** is `GpuError::UnbackedMapping`, by name. A backing is an
/// alloc-time fact, so an unbacked memory stays unbacked — category 3, and the one miss in
/// `sync_rpc_mappings` that is never knowable.
#[test]
fn fault_a_mapping_whose_memory_declared_no_backing_is_unbacked_by_name() {
    let mut gpu = fresh_gpu();
    for ev in [
        root_of(C),
        device(C, H_ROOT, H_DEV),
        vaspace(C, H_DEV, H_VAS),
        RmEvent::SetPageDir {
            client: C,
            vaspace: H_VAS,
            pdb: PDB0,
        },
        // A MEMORY object with NO declared `mem_phys`.
        RmEvent::Alloc {
            client: C,
            parent: H_DEV,
            handle: H_MEM,
            class: mc::MEMORY,
            facts: AllocFacts::default(),
        },
    ] {
        gpu.apply(ev).expect("the bring-up applies");
    }

    assert_eq!(
        gpu.apply(RmEvent::MapMemoryDma {
            client: C,
            vaspace: H_VAS,
            memory: H_MEM,
            va: VA,
            offset: 0,
            len: MAP_LEN,
        }),
        Err(GpuError::UnbackedMapping {
            pdb: PDB0,
            va: VA.0
        }),
        "★★ the EXACT `UnbackedMapping`, with the faulting PDB and VA — never a guess, \
         and never a silent skip",
    );
    // §12.18: the refused event is rolled back whole.
    assert_eq!(
        resolve(&gpu, GpuId::ZERO, PDB0, VA),
        Err(FwdFault::Address(kayfabe_mmu::AddressFault::Miss {
            pdb: PDB0,
            va: VA
        })),
        "the refusal left no half-installed binding behind",
    );
}

// =================================================================================
// CATEGORY 1 — DEFER (protocol): the guest may LEGALLY send this before that.
// =================================================================================

/// ★ The whole scripted bring-up, applied **backwards** except for the one ordering the
/// protocol imposes, still reaches the identical end state — the category-1 claim in its
/// most concentrated form.
///
/// `kayfabe_tests::legal_order` reorders exactly one thing (a namespace declares before it
/// is named) and nothing else: objects still precede their parents, `SET_PAGE_DIRECTORY`
/// still precedes its VASpace, and `MAP_MEMORY_DMA` still precedes both endpoints. If any
/// of those had quietly become a fault, this test is where it shows up.
#[test]
fn defer_the_whole_bringup_applies_backwards_and_reaches_the_same_end_state() {
    let arch = MockArch::new();
    let mut s = Scenario::new();
    s.compute_process(C, PDB0, identical_handles(0x10, 0x11));
    s.memory(C, H_DEV, H_MEM, MEM_PHYS);
    s.map(C, H_VAS, H_MEM, VA, MAP_LEN);

    let build = |evs: &[RmEvent]| {
        let mut g = RmGraph::new();
        for &ev in evs {
            g.apply(&arch, ev).expect("★ every LEGAL order applies");
        }
        project(&g, &arch, &NO_CONDEMNED).expect("projects")
    };

    let forward = build(&s.events);
    let mut backwards = s.events.clone();
    backwards.reverse();
    let backward = build(&kayfabe_tests::legal_order(&backwards));

    assert_eq!(
        backward, forward,
        "★★ the same declared facts in the reverse LEGAL order must derive the same \
         boundaries — every category-1 deferral resolved on the way"
    );
    assert!(
        forward.by_pdb.contains_key(&(GpuId::ZERO, PDB0)),
        "and the end state is non-trivial: the VAS routes"
    );
}
