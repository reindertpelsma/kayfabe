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
use kayfabe_core::rmgraph::{AllocFacts, NodeKey, ResourceKey, RmEvent, RmGraph, RmGraphError};
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
        !g.client_kinds().any(|(c, _)| c.client == GHOST),
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
        b.procs.iter().all(|p| !p.client_values().contains(&GHOST)),
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
    assert_eq!(b.procs[0].client_values(), BTreeSet::from([C]));
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
    assert_eq!(promoted.procs[0].client_values(), BTreeSet::from([C, peer]));
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

/// ★★★ **A `Free` prunes exactly the parked facts IT destroyed — no more, no less.**
///
/// `free_subtree` prunes the three parked tables by membership in `doomed` (the live
/// handles this free removed). That predicate carries two obligations at once and the
/// suite asserted neither for `pending_pdbs`:
///
/// - **It must fire.** A parked `SET_PAGE_DIRECTORY` on a handle the free destroys has to
///   die with it, or it lingers waiting for a handle only a *later* declaration of the
///   same value can create — §12.39 Shape A, one table over. The one key a `Free` can
///   name that a parked fact can also name is a **parked-dup destination**: `apply`
///   accepts a `Free` on it (`handles ∪ pending_dups`), so `doomed` legitimately holds a
///   key that was never a live handle.
/// - **It must not over-fire.** A parked PDB belonging to a handle this free did *not*
///   touch has to survive, or an unrelated `Free` anywhere in the namespace silently
///   deletes a legal guest's pending page-directory base — and the deferral that was
///   supposed to resolve never does. That is a MISS=FAULT on traffic that did nothing
///   wrong, which is precisely the failure category this file exists to rule out.
///
/// Both halves are asserted here, because a predicate is only pinned from both sides:
/// inverting it (`!doomed.contains` → `doomed.contains`) satisfies each half's *negation*
/// and a test for either one alone would still pass.
#[test]
fn a_free_prunes_its_own_parked_pdb_and_leaves_an_untouched_handles_parked_pdb_alone() {
    const H_DOOMED: HObject = HObject(0x5c00_0020);
    const H_SPARE: HObject = HObject(0x5c00_0021);
    const PDB_DOOMED: Pdb = Pdb(0x7701_000);

    let (arch, mut g) = fresh_graph();
    g.apply(&arch, root_of(C)).expect("C declares itself");
    g.apply(&arch, device(C, H_ROOT, H_DEV))
        .expect("C's device");

    // ---- The fact that must SURVIVE: a PDB parked on `H_VAS`, whose VASpace has not
    // been allocated yet and which the frees below never name.
    g.apply(
        &arch,
        RmEvent::SetPageDir {
            client: C,
            vaspace: H_VAS,
            pdb: PDB0,
        },
    )
    .expect("a PDB declared on an unobserved handle parks");

    // ---- The fact that must DIE: a dup parked at `H_DOOMED` (its source has never been
    // observed, so the edge stays parked and the handle stays un-live), plus a PDB parked
    // on that same handle value.
    g.apply(
        &arch,
        RmEvent::Dup {
            src: NodeKey::new(C, H_SPARE),
            dst: NodeKey::new(C, H_DOOMED),
        },
    )
    .expect("a dup of an unobserved source parks");
    g.apply(
        &arch,
        RmEvent::SetPageDir {
            client: C,
            vaspace: H_DOOMED,
            pdb: PDB_DOOMED,
        },
    )
    .expect("and a PDB parks on the same handle value");

    // ---- One `Free`, naming ONLY the parked-dup handle.
    g.apply(
        &arch,
        RmEvent::Free {
            client: C,
            handle: H_DOOMED,
        },
    )
    .expect("a Free of a parked-dup destination is a KNOWN handle");

    // ---- Half 1: the freed handle's parked PDB is gone. The guest re-uses the handle
    // value for a real VASpace — RM recycles handle values and the guest chooses them —
    // and that VASpace must come up with NO page-directory base, not the one a fact from
    // its dead predecessor declared.
    g.apply(&arch, vaspace(C, H_DEV, H_DOOMED))
        .expect("the handle value is re-used for a real VASpace");
    assert_eq!(
        g.pdb_of(NodeKey::new(C, H_DOOMED)),
        None,
        "★ the parked PDB died with the Free that destroyed its handle — a successor at \
         the same handle value must never inherit its predecessor's page-directory base",
    );

    // ---- Half 2: the untouched handle's parked PDB is intact, and still resolves. This
    // is the deferral completing: the fact was parked before its VASpace existed, an
    // unrelated Free happened in between, and the VASpace still learns its PDB.
    assert_eq!(
        g.pdb_of(NodeKey::new(C, H_VAS)),
        None,
        "still parked — there is no resource to attribute it to yet",
    );
    g.apply(&arch, vaspace(C, H_DEV, H_VAS))
        .expect("the bystander's VASpace finally arrives");
    assert_eq!(
        g.pdb_of(NodeKey::new(C, H_VAS)),
        Some(PDB0),
        "★★ an unrelated Free must not delete a bystander's parked PDB — the deferral \
         still resolves onto the resource when it arrives",
    );
}

/// ★★★ **THE SAME PROPERTY, SWEPT OVER EVERY PARKED TABLE AND TWO NAMESPACES: an
/// ORDINARY (non-root) `Free` prunes exactly the parked facts rooted at the handle it
/// destroys, and leaves every other parked fact — in this namespace and in any other —
/// untouched.**
///
/// The sibling test above pins `pending_pdbs` with a single witness. That is how the gap
/// this test closes was made: `free_subtree`'s `pending_dups` predicate has **two**
/// terms, and only the `dst` one was exercised. The `src` term —
/// *"a parked dup whose SOURCE handle was just freed is stale"* — was asserted by
/// nothing, in the highest-risk function in the crate.
///
/// **The `src` term is reachable, and reaching it is the whole point.** A parked edge's
/// endpoints are by definition not live handles, so the only key a `Free` can name that a
/// parked dup also names is *another parked dup's destination* — `apply` accepts a `Free`
/// on one (`known` = `handles ∪ pending_dups`). A parked **chain**
/// (`PEER:alias → C:mid → C:unseen`) therefore puts `C:mid` in `doomed` as both a `dst`
/// and a `src`, and the two terms prune the two edges independently.
///
/// **Both failure directions are live, and they arrive together.** Inverting the `src`
/// term simultaneously
///
/// - **retains** the stale cross-namespace edge whose source this `Free` destroyed — so
///   when the guest re-uses that handle value (RM recycles, and the guest picks the
///   values), the edge promotes and a live `DUP_OBJECT` alias appears in `PEER` against
///   `C`'s **successor** object. That is §12.39 Shape A one table over, and an alias is a
///   *grouping* edge: the two become one `Proc`, one isolate, one GPA arena, one host VAS
///   — #14 un-fixed for a pair the attacker chose; and
/// - **drops** every healthy parked dup that this `Free` did not name, in *either*
///   namespace — a legal guest's deferral silently deleted, so the alias it is owed never
///   materializes and its first use is a MISS=FAULT on traffic that did nothing wrong.
///
/// **Asserted by exact CONTENT, never by count.** The two directions cancel in a count
/// (one stale edge kept, healthy ones dropped), so a cardinality check cannot tell this
/// failure from correctness — the only honest assertion is the whole surviving set.
#[test]
fn a_non_root_free_prunes_exactly_the_parked_facts_rooted_at_it_in_every_table() {
    // C's namespace. `H_MID` is the parked-dup destination the `Free` names.
    const H_MID: HObject = HObject(0x6c00_0001);
    const H_UNSEEN: HObject = HObject(0x6c00_0002);
    const H_BY_C: HObject = HObject(0x6c00_0006);
    const H_C_SRC: HObject = HObject(0x6c00_0007);
    // PEER's namespace.
    const H_ALIAS: HObject = HObject(0x6c00_0003);
    const H_BY_PEER: HObject = HObject(0x6c00_0004);
    const H_PEER_SRC: HObject = HObject(0x6c00_0005);
    const H_PEER_VAS: HObject = HObject(0x6c00_0008);
    const H_PEER_MEM: HObject = HObject(0x6c00_0009);
    const PDB_MID: Pdb = Pdb(0x7702_000);
    const VA_MID: GpuVa = GpuVa(0x2_0040_0000);

    let (arch, mut g) = fresh_graph();
    g.apply(&arch, root_of(C)).expect("C declares itself");
    g.apply(&arch, device(C, H_ROOT, H_DEV))
        .expect("C's device");
    g.apply(&arch, memory(C, H_DEV, H_MEM, MEM_PHYS))
        .expect("C's memory object — live, so the map below parks ONLY on its VASpace");
    g.apply(&arch, root_of(PEER)).expect("PEER declares itself");
    g.apply(&arch, device(PEER, HObject(PEER.0), H_DEV))
        .expect("PEER's device");

    let mid = NodeKey::new(C, H_MID);
    let by_c = (NodeKey::new(C, H_BY_C), NodeKey::new(C, H_C_SRC));
    let by_peer = (
        NodeKey::new(PEER, H_BY_PEER),
        NodeKey::new(PEER, H_PEER_SRC),
    );

    // ---- The parked CHAIN. `C:H_MID` is a destination (edge 1) and a source (edge 2),
    // which is what puts one key on both sides of the predicate.
    g.apply(
        &arch,
        RmEvent::Dup {
            src: NodeKey::new(C, H_UNSEEN),
            dst: mid,
        },
    )
    .expect("edge 1: a dup of a never-observed source parks (category 2)");
    g.apply(
        &arch,
        RmEvent::Dup {
            src: mid,
            dst: NodeKey::new(PEER, H_ALIAS),
        },
    )
    .expect("edge 2: a CROSS-NAMESPACE dup whose source is itself parked also parks");

    // ---- The bystanders that must survive: one in the freed handle's OWN namespace
    // (so the prune is proved to be per-HANDLE, not per-namespace) and one in PEER's.
    g.apply(
        &arch,
        RmEvent::Dup {
            src: by_c.1,
            dst: by_c.0,
        },
    )
    .expect("edge 3: a bystander parked dup inside C");
    g.apply(
        &arch,
        RmEvent::Dup {
            src: by_peer.1,
            dst: by_peer.0,
        },
    )
    .expect("edge 4: a bystander parked dup inside PEER");

    // ---- The other two tables, one doomed fact and one bystander each.
    g.apply(
        &arch,
        RmEvent::SetPageDir {
            client: C,
            vaspace: H_MID,
            pdb: PDB_MID,
        },
    )
    .expect("a PDB parked on the doomed handle value");
    g.apply(
        &arch,
        RmEvent::SetPageDir {
            client: PEER,
            vaspace: H_PEER_VAS,
            pdb: PDB0,
        },
    )
    .expect("a bystander PDB parked in PEER");
    g.apply(
        &arch,
        RmEvent::MapMemoryDma {
            client: C,
            vaspace: H_MID,
            memory: H_MEM,
            va: VA_MID,
            offset: 0,
            len: MAP_LEN,
        },
    )
    .expect("a map parked on the doomed handle value as its VASpace");
    g.apply(
        &arch,
        RmEvent::MapMemoryDma {
            client: PEER,
            vaspace: H_PEER_VAS,
            memory: H_PEER_MEM,
            va: VA,
            offset: 0,
            len: MAP_LEN,
        },
    )
    .expect("a bystander map parked in PEER");

    let parked_before: BTreeSet<(NodeKey, NodeKey)> = g.dups().collect();
    assert_eq!(
        parked_before,
        BTreeSet::from([
            (mid, NodeKey::new(C, H_UNSEEN)),
            (NodeKey::new(PEER, H_ALIAS), mid),
            by_c,
            by_peer,
        ]),
        "precondition: all four edges are parked and none has resolved",
    );

    // ---- ONE ordinary `Free`, naming a parked-dup destination and nothing else.
    g.apply(
        &arch,
        RmEvent::Free {
            client: C,
            handle: H_MID,
        },
    )
    .expect("a Free of a parked-dup destination is a KNOWN handle, never `FreeUnknown`");

    // ★★ THE PROPERTY. Exactly the two edges rooted at `C:H_MID` are gone — the one it
    // destinates AND the one it sources — and exactly the two bystanders remain.
    assert_eq!(
        g.dups().collect::<BTreeSet<_>>(),
        BTreeSet::from([by_c, by_peer]),
        "★★★ a Free must prune exactly the parked dups rooted at the handle it destroyed \
         — the `dst` edge AND the `src` edge — and must leave every other namespace's \
         parked dups, and its own namespace's untouched handles', completely alone",
    );

    // ---- The recycle. RM recycles handle values and the guest chooses them, so the same
    // value comes back as a brand-new resource. Nothing the dead handle was named by may
    // attach to it.
    g.apply(&arch, vaspace(C, H_DEV, H_MID))
        .expect("★ re-using a freed handle value is LEGAL and must never be refused");

    assert_eq!(
        g.origin_of(NodeKey::new(PEER, H_ALIAS)).map(|n| n.key),
        None,
        "★★★ the stale cross-namespace edge fired into C's SUCCESSOR object: a live \
         `DUP_OBJECT` alias in PEER against a resource PEER never named — a grouping \
         edge, so the two processes collapse into one isolate/arena/VAS",
    );
    assert_eq!(
        g.node(NodeKey::new(PEER, H_ALIAS)).map(|n| n.key),
        None,
        "★ and no handle-table entry was minted for it either",
    );
    assert_eq!(
        g.references(mid).collect::<BTreeSet<_>>(),
        BTreeSet::from([mid]),
        "★ the successor resource is referenced by its own origin handle and NOTHING else",
    );
    assert_eq!(
        g.pdb_of(mid),
        None,
        "★ the successor must not inherit its dead predecessor's parked page-directory base",
    );
    assert_eq!(
        g.mappings().count(),
        0,
        "★ nor replay a map parked against the dead predecessor into its address plane",
    );

    // ---- And every bystander deferral still COMPLETES. A deferral that resolves to
    // nothing is a hang, so the surviving facts are proved live, not merely present.
    g.apply(&arch, vaspace(C, H_DEV, H_C_SRC))
        .expect("C's bystander source finally arrives");
    g.apply(&arch, vaspace(PEER, H_DEV, H_PEER_SRC))
        .expect("PEER's bystander source finally arrives");
    g.apply(&arch, vaspace(PEER, H_DEV, H_PEER_VAS))
        .expect("PEER's bystander VASpace finally arrives");
    g.apply(&arch, memory(PEER, H_DEV, H_PEER_MEM, MEM_PHYS))
        .expect("PEER's bystander memory finally arrives");

    assert_eq!(
        g.dups().collect::<BTreeSet<_>>(),
        BTreeSet::from([by_c, by_peer]),
        "★★ both bystander edges promoted into real aliases of their own sources — and \
         still no edge into PEER:H_ALIAS",
    );
    assert_eq!(
        g.references(by_c.1).collect::<BTreeSet<_>>(),
        BTreeSet::from([by_c.0, by_c.1]),
        "★ C's bystander alias is refcounted on its source (origin + alias), not merely \
         resolvable — the refcount is what decides lifetime",
    );
    assert_eq!(
        g.references(by_peer.1).collect::<BTreeSet<_>>(),
        BTreeSet::from([by_peer.0, by_peer.1]),
        "★ and so is PEER's",
    );
    assert_eq!(
        g.pdb_of(NodeKey::new(PEER, H_PEER_VAS)),
        Some(PDB0),
        "★★ PEER's parked PDB survived C's Free and drained onto its VASpace",
    );
    assert_eq!(
        g.mappings()
            .map(|m| (m.va, m.len, m.mem_phys, m.pdb))
            .collect::<Vec<_>>(),
        vec![(VA, MAP_LEN, Some(MEM_PHYS), Some(PDB0))],
        "★★ PEER's parked map survived C's Free and replayed — exactly one mapping, \
         PEER's own, at PEER's VA (the doomed map's VA must appear nowhere)",
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
        Some(ResourceKey::first(NodeKey::new(C, H_VAS))),
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
        .get(&ResourceKey::first(key))
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
        gpu.procs[&pid].chan_ids.get(&ResourceKey::first(key)),
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

// =================================================================================
// ★★★ §12.39 Part A — TEARDOWN COMPLETION: a parked fact must not outlive the namespace
// it names.
//
// The parked tables are the second of the graph's two references to a namespace that are
// **not handles** (the first is a resource's recorded owner, Part B's business). Both
// dangled. `free_subtree` prunes the parked tables by membership in `doomed`, which is the
// set of live HANDLES the free removed — and a parked fact's key is by definition not a
// live handle. So every parked fact survived the free of its own namespace's client root
// and sat there waiting for a handle only a LATER declaration of the same recyclable
// `hClient` could create.
//
// Asserted per TABLE rather than per scenario, so a table someone adds later is visibly
// missing a row.
// =================================================================================

/// The namespace the attacker declares, parks a fact in, frees, and waits for.
const RECYCLED: HClient = HClient(0xC1D0_0070);
/// The peer namespace that holds the other end of a parked dup.
const PEER: HClient = HClient(0xC1D0_0071);
/// The handle a parked dup's destination squats.
const H_PLANT: HObject = HObject(0x7c00_0001);
/// A handle that does not exist yet when the parked fact naming it is accepted.
const H_LATER: HObject = HObject(0x5c00_0f01);

/// Declare `RECYCLED`, run `plant`, free its root, then re-declare it — the recycled
/// namespace, in the four events §12.39 costs it.
fn park_free_and_redeclare(arch: &MockArch, g: &mut RmGraph, plant: &[RmEvent]) {
    g.apply(arch, root_of(RECYCLED))
        .expect("the attacker declares the namespace");
    g.apply(arch, device(RECYCLED, HObject(RECYCLED.0), H_DEV))
        .expect("with a device of its own");
    for ev in plant {
        g.apply(arch, *ev)
            .expect("the parked fact is accepted — it is a legal ordering");
    }
    g.apply(
        arch,
        RmEvent::Free {
            client: RECYCLED,
            handle: HObject(RECYCLED.0),
        },
    )
    .expect("the attacker frees the root it will hand back");
    // ★ The re-declaration MUST be accepted: RM recycles `hClient` values by design
    // (caller-supplied roots, `ogkm rs_server.c:612`; a generator that wraps at 2^20 with
    // no free list, `:3319-3341`; no epoch anywhere in RM). Refusing it is the
    // hangs-a-legal-guest error.
    g.apply(arch, root_of(RECYCLED))
        .expect("★ re-declaring a recycled namespace is LEGAL and must never be refused");
    g.apply(arch, device(RECYCLED, HObject(RECYCLED.0), H_DEV))
        .expect("the new tenant builds its own objects");
}

/// ★★ **A parked `DUP_OBJECT` whose DESTINATION namespace is freed must not fire into the
/// namespace's next tenant** (`l1_concurrency.md` §12.39, Shape A — the cheapest
/// cross-process isolation break in the model: four events and no host state).
#[test]
fn a_parked_dup_destination_does_not_survive_its_namespaces_root_free() {
    let (arch, mut g) = fresh_graph();
    g.apply(&arch, root_of(PEER)).expect("the peer declares");
    g.apply(&arch, device(PEER, HObject(PEER.0), H_DEV))
        .expect("peer device");

    park_free_and_redeclare(
        &arch,
        &mut g,
        // The source object does not exist yet, so the edge PARKS (category 2 — a dup's
        // source object may be one RM saw and we did not).
        &[RmEvent::Dup {
            src: NodeKey::new(PEER, H_LATER),
            dst: NodeKey::new(RECYCLED, H_PLANT),
        }],
    );

    // The attacker finally allocates the source — the event that used to promote the
    // parked edge into a live alias INSIDE the new tenant's namespace.
    g.apply(&arch, vaspace(PEER, H_DEV, H_LATER))
        .expect("the source object arrives");

    assert_eq!(
        g.origin_of(NodeKey::new(RECYCLED, H_PLANT)).map(|n| n.key),
        None,
        "★★ a `DUP_OBJECT` parked against a namespace the guest then FREED fired into \
         that namespace's next tenant — the alias is a grouping edge, so the two become \
         one `Proc`: one isolate, one GPA arena, one host VAS"
    );
    assert!(
        !g.dups().any(|(d, _)| d == NodeKey::new(RECYCLED, H_PLANT)),
        "★ and the parked edge itself is gone, not merely unresolvable"
    );
}

/// ★★ The **source** end of the same vector. A parked dup whose source lives in the freed
/// namespace lands its alias in the *attacker's* namespace and aliases a resource the
/// **next tenant** allocates — which merges exactly as hard, with the roles swapped.
#[test]
fn a_parked_dup_source_does_not_survive_its_namespaces_root_free() {
    let (arch, mut g) = fresh_graph();
    g.apply(&arch, root_of(PEER)).expect("the peer declares");
    g.apply(&arch, device(PEER, HObject(PEER.0), H_DEV))
        .expect("peer device");

    park_free_and_redeclare(
        &arch,
        &mut g,
        &[RmEvent::Dup {
            src: NodeKey::new(RECYCLED, H_LATER),
            dst: NodeKey::new(PEER, H_PLANT),
        }],
    );

    // The NEW tenant allocates the handle the stale edge names.
    g.apply(&arch, vaspace(RECYCLED, H_DEV, H_LATER))
        .expect("the new tenant's own VASpace");

    assert_eq!(
        g.origin_of(NodeKey::new(PEER, H_PLANT)).map(|n| n.key),
        None,
        "★★ a `DUP_OBJECT` parked against a source in a namespace the guest then FREED \
         aliased the NEXT tenant's object into the attacker's namespace"
    );
    assert_eq!(
        g.references(NodeKey::new(RECYCLED, H_LATER))
            .collect::<Vec<_>>(),
        vec![NodeKey::new(RECYCLED, H_LATER)],
        "★ the new tenant's object is referenced by its own origin handle and NOTHING else"
    );
}

/// ★★ A parked `SET_PAGE_DIRECTORY` must not drain onto the next tenant's VASpace. A
/// forged PDB there is not merely wrong: two VASpace origins declaring one `Pdb` on one
/// target is a `ProjectionError::PdbCollision`, which faults the projection for **every**
/// process on the device (the §18A global-DoS shape).
#[test]
fn a_parked_page_directory_does_not_survive_its_namespaces_root_free() {
    let (arch, mut g) = fresh_graph();
    park_free_and_redeclare(
        &arch,
        &mut g,
        &[RmEvent::SetPageDir {
            client: RECYCLED,
            vaspace: H_LATER,
            pdb: PDB0,
        }],
    );

    g.apply(&arch, vaspace(RECYCLED, H_DEV, H_LATER))
        .expect("the new tenant allocates a VASpace at that handle value");

    assert_eq!(
        g.pdb_of(NodeKey::new(RECYCLED, H_LATER)),
        None,
        "★★ a `SET_PAGE_DIRECTORY` parked by a namespace the guest then FREED drained \
         onto the next tenant's VASpace — an attacker-chosen page-directory base on a \
         victim's address plane"
    );
}

/// ★★ A parked `MAP_MEMORY_DMA` must not replay into the next tenant's address plane —
/// an attacker-chosen `va → phys` forward-population in a victim's VASpace.
#[test]
fn a_parked_map_does_not_survive_its_namespaces_root_free() {
    let (arch, mut g) = fresh_graph();
    park_free_and_redeclare(
        &arch,
        &mut g,
        &[RmEvent::MapMemoryDma {
            client: RECYCLED,
            vaspace: H_LATER,
            memory: H_MEM,
            va: VA,
            offset: 0,
            len: MAP_LEN,
        }],
    );

    g.apply(&arch, vaspace(RECYCLED, H_DEV, H_LATER))
        .expect("the new tenant's VASpace");
    g.apply(&arch, memory(RECYCLED, H_DEV, H_MEM, MEM_PHYS))
        .expect("the new tenant's memory object");

    assert_eq!(
        g.mappings().count(),
        0,
        "★★ a `MAP_MEMORY_DMA` parked by a namespace the guest then FREED replayed into \
         the next tenant's VASpace"
    );
}

/// ★★★ **THE OTHER HALF OF §12.39 PART A: the namespace purge must not OVER-fire.**
///
/// The four tests above are single witnesses — one parked fact each, and each asserts only
/// that it does *not* fire. A purge that deleted the whole parked table on every root free
/// would pass all four. That is not a hypothetical failure mode: it is the *only* way the
/// three `retain`s in `free_subtree` can be wrong in the direction that hurts a legal
/// guest, and the direction that hurts a legal guest is the one nothing was watching.
///
/// So this sweeps instead: **both** cross-namespace dup orientations (the dying namespace
/// as the edge's `dst` and as its `src`), a parked PDB and a parked map in the dying
/// namespace, and a full bystander set of all three kinds in a namespace that is not being
/// torn down — through one client-root free and an `hClient` **recycle**, asserted by
/// exact content on both sides of the line.
///
/// The recycle is the load-bearing part and it must be *accepted*: RM recycles `hClient`
/// values by design (caller-supplied roots, `ogkm rs_server.c:612`; a generator that wraps
/// at 2^20 with no free list and no epoch, `:3319-3341`). Refusing the re-declaration
/// would "fix" Shape A by hanging a legal guest.
#[test]
fn a_client_root_free_purges_only_its_own_namespaces_parked_facts() {
    /// The bystander namespace's parked-dup destination and its not-yet-observed source.
    const H_P_DST: HObject = HObject(0x6d00_0001);
    const H_P_SRC: HObject = HObject(0x6d00_0002);
    /// A PEER handle the DYING namespace parks an edge against (the `dst`-side vector).
    const H_P_LATER: HObject = HObject(0x6d00_0003);
    const H_P_VAS: HObject = HObject(0x6d00_0004);
    const H_P_MEM: HObject = HObject(0x6d00_0005);
    const PDB_DEAD: Pdb = Pdb(0x7703_000);
    const VA_DEAD: GpuVa = GpuVa(0x2_0060_0000);

    let (arch, mut g) = fresh_graph();
    g.apply(&arch, root_of(PEER))
        .expect("the bystander declares");
    g.apply(&arch, device(PEER, HObject(PEER.0), H_DEV))
        .expect("the bystander's device");
    g.apply(&arch, root_of(RECYCLED))
        .expect("the namespace that will be torn down declares");
    g.apply(&arch, device(RECYCLED, HObject(RECYCLED.0), H_DEV))
        .expect("its device");

    let bystander = (NodeKey::new(PEER, H_P_DST), NodeKey::new(PEER, H_P_SRC));
    // Orientation 1: the alias lands in PEER, the source is the dying namespace's.
    let src_side = (NodeKey::new(PEER, H_PLANT), NodeKey::new(RECYCLED, H_LATER));
    // Orientation 2: the alias lands in the dying namespace, the source is PEER's.
    let dst_side = (
        NodeKey::new(RECYCLED, H_PLANT),
        NodeKey::new(PEER, H_P_LATER),
    );

    for (dst, src) in [bystander, src_side, dst_side] {
        g.apply(&arch, RmEvent::Dup { src, dst })
            .expect("a dup of a not-yet-observed source parks (category 2)");
    }
    g.apply(
        &arch,
        RmEvent::SetPageDir {
            client: RECYCLED,
            vaspace: H_LATER,
            pdb: PDB_DEAD,
        },
    )
    .expect("the dying namespace parks a PDB");
    g.apply(
        &arch,
        RmEvent::SetPageDir {
            client: PEER,
            vaspace: H_P_VAS,
            pdb: PDB0,
        },
    )
    .expect("the bystander parks a PDB");
    g.apply(
        &arch,
        RmEvent::MapMemoryDma {
            client: RECYCLED,
            vaspace: H_LATER,
            memory: H_MEM,
            va: VA_DEAD,
            offset: 0,
            len: MAP_LEN,
        },
    )
    .expect("the dying namespace parks a map");
    g.apply(
        &arch,
        RmEvent::MapMemoryDma {
            client: PEER,
            vaspace: H_P_VAS,
            memory: H_P_MEM,
            va: VA,
            offset: 0,
            len: MAP_LEN,
        },
    )
    .expect("the bystander parks a map");

    assert_eq!(
        g.dups().collect::<BTreeSet<_>>(),
        BTreeSet::from([bystander, src_side, dst_side]),
        "precondition: three parked edges, none resolved",
    );

    // ---- Teardown and recycle, in the four events §12.39 costs.
    g.apply(
        &arch,
        RmEvent::Free {
            client: RECYCLED,
            handle: HObject(RECYCLED.0),
        },
    )
    .expect("the client root frees");
    g.apply(&arch, root_of(RECYCLED))
        .expect("★ re-declaring a recycled `hClient` is LEGAL and must never be refused");
    g.apply(&arch, device(RECYCLED, HObject(RECYCLED.0), H_DEV))
        .expect("the new tenant builds its own objects");

    // ★★ THE PROPERTY. Every parked edge naming the dead namespace — in EITHER role — is
    // gone; the bystander's is untouched.
    assert_eq!(
        g.dups().collect::<BTreeSet<_>>(),
        BTreeSet::from([bystander]),
        "★★★ a root free must purge every parked edge naming its namespace as `dst` OR \
         as `src`, and must not touch a parked edge belonging to anyone else",
    );

    // ---- Now supply every fact the dead parked entries were waiting for. Under a purge
    // that failed to fire, each of these is the promotion that breaks isolation.
    g.apply(&arch, vaspace(RECYCLED, H_DEV, H_LATER))
        .expect("the new tenant allocates the handle value the stale facts named");
    g.apply(&arch, memory(RECYCLED, H_DEV, H_MEM, MEM_PHYS))
        .expect("and the memory object the stale map named");
    g.apply(&arch, vaspace(PEER, H_DEV, H_P_LATER))
        .expect("the bystander allocates the source the stale `dst`-side edge named");

    assert_eq!(
        g.origin_of(src_side.0).map(|n| n.key),
        None,
        "★★ the `src`-side stale edge aliased the NEW tenant's object into PEER",
    );
    assert_eq!(
        g.origin_of(dst_side.0).map(|n| n.key),
        None,
        "★★ the `dst`-side stale edge planted a live alias INSIDE the new tenant",
    );
    assert_eq!(
        g.references(NodeKey::new(RECYCLED, H_LATER))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([NodeKey::new(RECYCLED, H_LATER)]),
        "★ the new tenant's VASpace is referenced by its own origin handle and nothing else",
    );
    assert_eq!(
        g.pdb_of(NodeKey::new(RECYCLED, H_LATER)),
        None,
        "★ and carries no page-directory base its dead predecessor declared",
    );
    assert_eq!(
        g.mappings().count(),
        0,
        "★ and the dead namespace's parked map did not replay into it",
    );

    // ---- The bystander's three deferrals all still COMPLETE.
    g.apply(&arch, vaspace(PEER, H_DEV, H_P_SRC))
        .expect("the bystander's dup source arrives");
    g.apply(&arch, vaspace(PEER, H_DEV, H_P_VAS))
        .expect("the bystander's VASpace arrives");
    g.apply(&arch, memory(PEER, H_DEV, H_P_MEM, MEM_PHYS))
        .expect("the bystander's memory arrives");

    assert_eq!(
        g.dups().collect::<BTreeSet<_>>(),
        BTreeSet::from([bystander]),
        "★★ the bystander's edge resolved into a real alias, and no stale edge came back",
    );
    assert_eq!(
        g.references(bystander.1).collect::<BTreeSet<_>>(),
        BTreeSet::from([bystander.0, bystander.1]),
        "★ and it is refcounted on its source, which is what decides lifetime",
    );
    assert_eq!(
        g.pdb_of(NodeKey::new(PEER, H_P_VAS)),
        Some(PDB0),
        "★★ an unrelated namespace's teardown must not delete a bystander's parked PDB",
    );
    assert_eq!(
        g.mappings()
            .map(|m| (m.va, m.len, m.mem_phys, m.pdb))
            .collect::<Vec<_>>(),
        vec![(VA, MAP_LEN, Some(MEM_PHYS), Some(PDB0))],
        "★★ nor its parked map — exactly one mapping, the bystander's own, and the dead \
         namespace's VA appears nowhere",
    );
}
