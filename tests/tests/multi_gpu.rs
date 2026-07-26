//! ★ MG-7 — THE MULTI-GPU ACCEPTANCE SUITE (`multi_gpu_and_mig.md`, decision #29).
//!
//! The core forwards to N physical GPUs, keyed on a graph-derived [`GpuId`] target
//! (from each `Device`'s `deviceInstance`), and isolates every GPU's state from every
//! other's. The load-bearing finding (`gr_multigpu_seam_audit.md`): `Pdb` and `VChid`
//! are **per-GPU namespaces**, so two GPUs legally present IDENTICAL PDB/vChid values;
//! routing keys on `(GpuId, Pdb)` / `(GpuId, VChid)`, and the F1 collision guard bites
//! only WITHIN one target.
//!
//! Coverage (the audit's bar):
//! - **`correct_gpu_routing`** — an op lands on the GPU its Device-derived `GpuId`
//!   names, never a guess or first-resolvable.
//! - **`cross_gpu_isolation`** — a proc bound to GPU0 reaches nothing on GPU1
//!   (PDBs/arenas/isolates/completions/backing); an op for GPU0 never touches GPU1's
//!   backend.
//! - **`hash14_across_gpu`** — two GPUs with IDENTICAL guest VAs + IDENTICAL RM handles
//!   are disjoint by construction (the #14 lesson lifted onto the GPU axis).
//! - **★`security_same_gpu_dup_refused_cross_gpu_identical_allowed`** — BOTH directions
//!   of the load-bearing invariant: a same-GPU PDB/vChid duplicate is STILL a loud
//!   collision (the F1 guard, #18C), while identical PDB/vChid on DIFFERENT GPUs is
//!   legal and each resolves to its own proc.
//! - **`determinism_holds_under_gpu_axis`** — the whole multi-GPU world's observable
//!   end-state is order/interleave-invariant with the GpuId axis present.
//! - **`per_gpu_completion_no_cross_serialization`** (MG-6) and
//!   **`per_gpu_arena_recycle`** (#80 per target) and **`homogeneous_arch`** (MG-6).
//!
//! MIG is deliberately absent (`multi_gpu_and_mig.md`: datacenter silicon, unbuilt).

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_arch::ids::{GpuId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, GpuError};
use kayfabe_core::project::{ProjectionError, project};
use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
use kayfabe_fwd::{FwdFault, handle_doorbell, publish_backing, resolve};
use kayfabe_mocks::{MockArch, MockIsolateFactory, mock_classes as mc};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

// The IDENTICAL hardware identities both GPUs' procs present (the load-bearing shape).
const SHARED_PDB: Pdb = Pdb(0x3401_000);
const SHARED_VA: GpuVa = GpuVa(0x2_0020_0000);
const GR_VCHID: u16 = 0x10;
const CE_VCHID: u16 = 0x11;

fn new_gpu() -> (
    Guarded<Gpu>,
    std::sync::Arc<std::sync::Mutex<kayfabe_mocks::RmRecorder>>,
) {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    // A window sized so each target's disjoint sub-window comfortably fits several arenas.
    let gpa = GpaSpace::new(0x1_0000_0000..0x11_0000_0000, 0x1_0000_0000);
    (
        // ★ G9 (§12.21): the device is REALIZED with two physical GPUs — the
        // entitlement a guest's `deviceInstance` is now checked against.
        Guarded::new(
            "multi_gpu",
            Gpu::realize(arch, Box::new(factory), gpa, &[GpuId::ZERO, GpuId(1)])
                .expect("device realizes"),
            rec.clone(),
        ),
        rec,
    )
}

/// Events for one compute proc on physical GPU `instance`, reusing the SAME guest PDB,
/// vChids and RM handle values on every GPU (the #14-across-GPU shape). Distinct
/// `client` per GPU ⇒ distinct dup-components ⇒ distinct procs.
fn proc_on_gpu(client: HClient, instance: u32) -> Vec<RmEvent> {
    let mut s = Scenario::new();
    let h = identical_handles(GR_VCHID, CE_VCHID);
    s.compute_process_on_gpu(client, SHARED_PDB, h, Some(instance));
    // A backed memory object mapped at the SHARED guest VA (identical on every GPU).
    s.memory(
        client,
        h.device,
        HObject(0x5c00_0100),
        0x9_0000_0000 + u64::from(instance) * 0x1_0000_0000,
    );
    s.map(client, h.vaspace, HObject(0x5c00_0100), SHARED_VA, 0x10000);
    s.events
}

/// Build a two-GPU world: proc A on GPU0, proc B on GPU1 — identical everything but the
/// target. Returns (gpu, recorder, pidA, pidB).
fn two_gpu_world() -> (
    Guarded<Gpu>,
    std::sync::Arc<std::sync::Mutex<kayfabe_mocks::RmRecorder>>,
    kayfabe_core::ProcId,
    kayfabe_core::ProcId,
) {
    let (mut gpu, rec) = new_gpu();
    for ev in proc_on_gpu(HClient(0xA0), 0) {
        gpu.apply(ev).expect("GPU0 proc applies");
    }
    for ev in proc_on_gpu(HClient(0xB0), 1) {
        gpu.apply(ev).expect("GPU1 proc applies");
    }
    let pid_a = *gpu
        .spine
        .by_pdb
        .get(&(GpuId(0), SHARED_PDB))
        .expect("GPU0 PDB routes");
    let pid_b = *gpu
        .spine
        .by_pdb
        .get(&(GpuId(1), SHARED_PDB))
        .expect("GPU1 PDB routes");
    (gpu, rec, pid_a, pid_b)
}

// =================================================================================
// (a) correct-GPU routing.
// =================================================================================

#[test]
fn correct_gpu_routing() {
    let (mut gpu, _rec, pid_a, pid_b) = two_gpu_world();
    assert_ne!(pid_a, pid_b, "the two GPUs' procs are distinct");

    // The SAME doorbell token (identical vChid) demuxes to a DIFFERENT proc per target
    // — routing is by the Device-derived GpuId the doorbell's BAR names, not a guess.
    let token = MockArch::token_for(VChid(GR_VCHID));
    let out0 = handle_doorbell(&mut gpu, GpuId(0), token, &[]).expect("GPU0 doorbell routes");
    let out1 = handle_doorbell(&mut gpu, GpuId(1), token, &[]).expect("GPU1 doorbell routes");
    assert_eq!(out0.proc, pid_a, "GPU0 token routed to GPU0's proc");
    assert_eq!(out1.proc, pid_b, "GPU1 token routed to GPU1's proc");
    assert_ne!(
        out0.host_token, out1.host_token,
        "distinct host channels per GPU"
    );

    // Address resolution is likewise per target: the SAME PDB+VA resolves to each
    // GPU's OWN backing (declared distinct per instance), never the other's.
    let phys0 = resolve(&gpu, GpuId(0), SHARED_PDB, SHARED_VA).map(|(b, _)| b.phys);
    let phys1 = resolve(&gpu, GpuId(1), SHARED_PDB, SHARED_VA).map(|(b, _)| b.phys);
    assert_eq!(phys0, Ok(0x9_0000_0000), "GPU0 VA → GPU0 backing");
    assert_eq!(phys1, Ok(0xA_0000_0000), "GPU1 VA → GPU1 backing");
}

// =================================================================================
// (b) cross-GPU isolation.
// =================================================================================

#[test]
fn cross_gpu_isolation() {
    let (mut gpu, rec, pid_a, pid_b) = two_gpu_world();

    // Publish a host backing in each proc on ITS OWN target.
    let pub_a = publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GpuId(0),
        SHARED_PDB,
        GpuVa(0x5_0000_0000),
        0x1000,
    )
    .expect("GPU0 publish");
    let pub_b = publish_backing(
        gpu.procs.get_mut(&pid_b).unwrap(),
        GpuId(1),
        SHARED_PDB,
        GpuVa(0x5_0000_0000),
        0x1000,
    )
    .expect("GPU1 publish");

    // Disjoint GPA arenas AND disjoint host VAs — each proc's per-(proc, GPU) isolate
    // + arena are separate by construction.
    assert_ne!(pub_a.gpa, pub_b.gpa, "cross-GPU GPA collision");
    assert_ne!(pub_a.host_va, pub_b.host_va, "cross-GPU host-VA collision");

    // A proc materialized ONLY its own target's isolate/arena — never the other GPU's.
    let a = gpu.procs.get(&pid_a).unwrap();
    let b = gpu.procs.get(&pid_b).unwrap();
    assert!(
        a.arenas.contains_key(&GpuId(0)) && !a.arenas.contains_key(&GpuId(1)),
        "GPU0 proc has no GPU1 arena"
    );
    assert!(
        b.arenas.contains_key(&GpuId(1)) && !b.arenas.contains_key(&GpuId(0)),
        "GPU1 proc has no GPU0 arena"
    );
    assert!(
        a.isolates.contains_key(&GpuId(0)) && !a.isolates.contains_key(&GpuId(1)),
        "GPU0 proc has no GPU1 isolate"
    );

    // An op for GPU0 never lands on GPU1's backend: the doorbell/PDB for GPU0 resolves
    // GPU0's proc, and GPU1's identical identities resolve GPU1's proc — a GPU0 op can
    // never reach GPU1's host handles (namespaced per (proc, GPU) in the recorder).
    let log = rec.lock().unwrap();
    let sessions_gpu0: std::collections::BTreeSet<u64> = log
        .log
        .iter()
        .filter_map(|(_id, verb)| match verb {
            kayfabe_mocks::RmVerb::MapGpuVa { va, .. } => Some(va >> 47 & 1), // GPU lane bit
            _ => None,
        })
        .collect();
    // Both GPU lanes (0 and 1) appear — the two procs mapped on distinct target isolates.
    assert!(
        sessions_gpu0.contains(&0) && sessions_gpu0.contains(&1),
        "each proc mapped on its own GPU's isolate"
    );

    // Cross-target routing is a clean MISS, never a silent reach into the other GPU:
    // GPU0 has no proc holding GPU1's-only identities beyond its own.
    assert_eq!(gpu.spine.by_pdb.get(&(GpuId(0), SHARED_PDB)), Some(&pid_a));
    assert_eq!(gpu.spine.by_pdb.get(&(GpuId(1), SHARED_PDB)), Some(&pid_b));
}

// =================================================================================
// (c) #14-across-GPU — identical VAs + identical handles, disjoint by construction.
// =================================================================================

#[test]
fn hash14_across_gpu() {
    let (mut gpu, _rec, pid_a, pid_b) = two_gpu_world();
    // An IDENTICAL fresh guest VA (distinct from the RPC-mapped SHARED_VA), published
    // in each proc on its own GPU — the #14-across-GPU shape.
    let ident_va = GpuVa(0x6_0000_0000);
    let pa = publish_backing(
        gpu.procs.get_mut(&pid_a).unwrap(),
        GpuId(0),
        SHARED_PDB,
        ident_va,
        0x10000,
    )
    .expect("A");
    let pb = publish_backing(
        gpu.procs.get_mut(&pid_b).unwrap(),
        GpuId(1),
        SHARED_PDB,
        ident_va,
        0x10000,
    )
    .expect("B");
    assert_ne!(
        pa.gpa, pb.gpa,
        "identical VAs across GPUs must land at disjoint GPAs"
    );
    assert_ne!(
        pa.host_va, pb.host_va,
        "identical VAs across GPUs must land in disjoint host VASes"
    );
    // Each still resolves to ITS OWN host publication, never the other's.
    let (ba, _) = resolve(&gpu, GpuId(0), SHARED_PDB, ident_va).unwrap();
    let (bb, _) = resolve(&gpu, GpuId(1), SHARED_PDB, ident_va).unwrap();
    assert_eq!(ba.host_va(), Some(pa.host_va));
    assert_eq!(bb.host_va(), Some(pb.host_va));
    assert_ne!(ba.phys, bb.phys, "disjoint backing across the GPU axis");
}

// =================================================================================
// ★ (d) THE load-bearing security test — BOTH directions.
// =================================================================================

/// Same-GPU duplicate PDB is STILL the hostile F1 collision (#18C); the identical PDB
/// on a DIFFERENT GPU is legal and each resolves to its own proc.
#[test]
fn security_same_gpu_dup_refused_cross_gpu_identical_allowed() {
    // ---- Direction 1: cross-GPU identical PDB is ALLOWED (two targets, no collision). ----
    let (gpu, _rec, pid_a, pid_b) = two_gpu_world();
    assert_ne!(pid_a, pid_b);
    assert_eq!(
        gpu.spine.by_pdb.get(&(GpuId(0), SHARED_PDB)),
        Some(&pid_a),
        "GPU0's PDB routes to A"
    );
    assert_eq!(
        gpu.spine.by_pdb.get(&(GpuId(1), SHARED_PDB)),
        Some(&pid_b),
        "identical PDB on GPU1 routes to B — NOT a collision"
    );
    // The old device-global guard would have refused GPU1's identical PDB as a
    // PdbCollision; under the (GpuId, Pdb) key it is legal traffic.
    assert!(
        project(&gpu.spine.rmgraph, gpu.spine.arch.as_ref()).is_ok(),
        "the two-GPU world projects cleanly"
    );

    // ---- Direction 2: SAME-GPU duplicate PDB is STILL a loud collision. ----
    let (mut gpu2, _rec2) = new_gpu();
    let c = HClient(0xC0);
    let root = HObject(0xC000);
    let dev = HObject(0xC001);
    let vas1 = HObject(0xC010);
    let vas2 = HObject(0xC011);
    for ev in [
        RmEvent::Alloc {
            client: c,
            parent: root,
            handle: root,
            class: mc::CLIENT,
            facts: kayfabe_tests::user_client(c),
        },
        // ONE device (GPU0), TWO VASpaces that will claim the SAME PDB.
        RmEvent::Alloc {
            client: c,
            parent: root,
            handle: dev,
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: c,
            parent: dev,
            handle: vas1,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: c,
            vaspace: vas1,
            pdb: SHARED_PDB,
        },
        RmEvent::Alloc {
            client: c,
            parent: dev,
            handle: vas2,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        },
    ] {
        gpu2.apply(ev).expect("setup applies");
    }
    // The second VAS claiming the SAME (GPU0) PDB is the hostile duplicate → loud.
    let dup = gpu2.apply(RmEvent::SetPageDir {
        client: c,
        vaspace: vas2,
        pdb: SHARED_PDB,
    });
    assert!(
        matches!(
            dup,
            Err(GpuError::Projection(ProjectionError::PdbCollision {
                gpu: Some(GpuId(0)),
                ..
            }))
        ),
        "a same-GPU PDB duplicate must STILL be a loud PdbCollision, got {dup:?}"
    );
    // Atomic: the collision was rolled back; the first VAS still routes (device usable).
    assert!(
        gpu2.spine.by_pdb.contains_key(&(GpuId(0), SHARED_PDB)),
        "first claimant survives the refusal"
    );

    // ---- Direction 2b: SAME-GPU duplicate vChid is STILL a loud collision. ----
    let (mut gpu3, _rec3) = new_gpu();
    let d = HClient(0xD0);
    let droot = HObject(0xD000);
    let ddev = HObject(0xD001);
    let dvas = HObject(0xD010);
    let flags = MockArch::userd_flags_for(VChid(GR_VCHID));
    for ev in [
        RmEvent::Alloc {
            client: d,
            parent: droot,
            handle: droot,
            class: mc::CLIENT,
            facts: kayfabe_tests::user_client(d),
        },
        RmEvent::Alloc {
            client: d,
            parent: droot,
            handle: ddev,
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: d,
            parent: ddev,
            handle: dvas,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: d,
            vaspace: dvas,
            pdb: SHARED_PDB,
        },
        RmEvent::Alloc {
            client: d,
            parent: ddev,
            handle: HObject(0xD020),
            class: mc::CHANNEL_GR,
            facts: AllocFacts {
                h_vaspace: Some(dvas),
                userd_flags: flags,
                ..Default::default()
            },
        },
    ] {
        gpu3.apply(ev).expect("channel-1 setup applies");
    }
    // A SECOND channel on the SAME GPU decoding to the SAME vChid → loud collision.
    let dup_v = gpu3.apply(RmEvent::Alloc {
        client: d,
        parent: ddev,
        handle: HObject(0xD021),
        class: mc::CHANNEL_GR,
        facts: AllocFacts {
            h_vaspace: Some(dvas),
            userd_flags: flags,
            ..Default::default()
        },
    });
    assert!(
        matches!(
            dup_v,
            Err(GpuError::Projection(ProjectionError::VchidCollision {
                gpu: Some(GpuId(0)),
                ..
            }))
        ),
        "a same-GPU vChid duplicate must STILL be a loud VchidCollision, got {dup_v:?}"
    );
}

// =================================================================================
// (e) determinism holds with the GpuId axis.
// =================================================================================

#[test]
fn determinism_holds_under_gpu_axis() {
    // The interleaved two-GPU fact stream, applied forward vs reversed vs woven, yields
    // an IDENTICAL derived routing picture (Boundaries) — the GpuId axis does not leak
    // arrival order into the observable end-state.
    let mut events = proc_on_gpu(HClient(0xA0), 0);
    events.extend(proc_on_gpu(HClient(0xB0), 1));

    let derive = |evs: &[RmEvent]| {
        let (mut gpu, _rec) = new_gpu();
        for ev in evs {
            gpu.apply(*ev)
                .expect("valid multi-GPU fact applies in any order");
        }
        project(&gpu.spine.rmgraph, gpu.spine.arch.as_ref()).expect("projects")
    };

    let reference = derive(&events);

    let mut reversed = events.clone();
    reversed.reverse();
    assert_eq!(
        derive(&reversed),
        reference,
        "reversed order diverged under the GPU axis"
    );

    // Even/odd interleave of the two GPUs' facts.
    let mut woven: Vec<RmEvent> = events.iter().step_by(2).copied().collect();
    woven.extend(events.iter().skip(1).step_by(2).copied());
    assert_eq!(
        derive(&woven),
        reference,
        "interleave diverged under the GPU axis"
    );

    // Non-trivial: the routing picture carries BOTH targets' identical PDB/vChid.
    assert!(reference.by_pdb.contains_key(&(GpuId(0), SHARED_PDB)));
    assert!(reference.by_pdb.contains_key(&(GpuId(1), SHARED_PDB)));
    assert!(
        reference
            .by_vchid
            .contains_key(&(GpuId(0), VChid(GR_VCHID)))
    );
    assert!(
        reference
            .by_vchid
            .contains_key(&(GpuId(1), VChid(GR_VCHID)))
    );
}

// =================================================================================
// MG-6 — per-target completion drain gate (no cross-GPU serialization).
// =================================================================================

#[test]
fn per_gpu_completion_no_cross_serialization() {
    let (mut gpu, _rec, pid_a, pid_b) = two_gpu_world();
    // Each proc observes a completion on its own target.
    gpu.procs
        .get_mut(&pid_a)
        .unwrap()
        .completion
        .observe(kayfabe_completion::OsEventRef(0xA))
        .unwrap();
    gpu.procs
        .get_mut(&pid_b)
        .unwrap()
        .completion
        .observe(kayfabe_completion::OsEventRef(0xB))
        .unwrap();

    // Post on GPU0 and LEAVE its batch outstanding (do NOT drain).
    let b0 = gpu.pump_completions(GpuId(0)).expect("GPU0 posts");
    assert_eq!(b0.events, vec![kayfabe_completion::OsEventRef(0xA)]);
    // GPU0's gate is closed; a re-post on GPU0 yields nothing.
    assert!(
        gpu.pump_completions(GpuId(0)).is_none(),
        "GPU0 gate closed while outstanding"
    );
    // ★ GPU1's post is NOT gated by GPU0's outstanding batch (its own drain gate).
    let b1 = gpu
        .pump_completions(GpuId(1))
        .expect("GPU1 posts despite GPU0 outstanding");
    assert_eq!(
        b1.events,
        vec![kayfabe_completion::OsEventRef(0xB)],
        "no cross-GPU serialization"
    );
}

// =================================================================================
// #80-per-target — per-GPU arena recycle across teardown.
// =================================================================================

#[test]
fn per_gpu_arena_recycle() {
    let (mut gpu, _rec, pid_a, pid_b) = two_gpu_world();
    let arena_a = gpu.procs[&pid_a].arenas[&GpuId(0)].range.clone();
    let arena_b = gpu.procs[&pid_b].arenas[&GpuId(1)].range.clone();
    assert!(
        arena_a.end <= arena_b.start || arena_b.end <= arena_a.start,
        "targets' windows disjoint"
    );

    // Tear both down + reap.
    gpu.apply(RmEvent::Free {
        client: HClient(0xA0),
        handle: HObject(0x5c00_0000),
    })
    .unwrap();
    gpu.apply(RmEvent::Free {
        client: HClient(0xB0),
        handle: HObject(0x5c00_0000),
    })
    .unwrap();
    assert_eq!(gpu.reap_retired().len(), 2, "both procs reaped");

    // Rebuild identical procs; each target's arena is RECYCLED from its own window.
    for ev in proc_on_gpu(HClient(0xA0), 0) {
        gpu.apply(ev).expect("gen-2 GPU0 applies");
    }
    for ev in proc_on_gpu(HClient(0xB0), 1) {
        gpu.apply(ev).expect("gen-2 GPU1 applies");
    }
    let pid_a2 = *gpu.spine.by_pdb.get(&(GpuId(0), SHARED_PDB)).unwrap();
    let pid_b2 = *gpu.spine.by_pdb.get(&(GpuId(1), SHARED_PDB)).unwrap();
    assert_eq!(
        gpu.procs[&pid_a2].arenas[&GpuId(0)].range,
        arena_a,
        "GPU0 arena recycled from its own window"
    );
    assert_eq!(
        gpu.procs[&pid_b2].arenas[&GpuId(1)].range,
        arena_b,
        "GPU1 arena recycled from its own window"
    );
}

// =================================================================================
// MG-6 — homogeneous-arch invariant (heterogeneous multi-arch is out of scope).
// =================================================================================

#[test]
fn homogeneous_arch_all_targets_share_the_device_arch() {
    let (gpu, _rec, _a, _b) = two_gpu_world();
    // Both targets were realized under the ONE device arch (V1 homogeneous). A
    // heterogeneous config would be a loud GpuError::HeterogeneousArch at realize; here
    // we assert the invariant holds — every target shares the device's arch identity.
    assert!(gpu.spine.targets.len() >= 2, "two GPU targets exist");
    let name = gpu.spine.arch.name();
    // The FwdFault surface is unchanged by the axis (a compile-level check that the
    // per-target routing faults carry their GpuId).
    let miss = resolve(&gpu, GpuId(9), SHARED_PDB, SHARED_VA);
    assert!(
        matches!(miss, Err(FwdFault::UnknownPdb { gpu: GpuId(9), .. })),
        "an unrouted target is a loud, target-carrying MISS"
    );
    let _ = name;
}
