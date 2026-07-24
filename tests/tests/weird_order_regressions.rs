//! CATEGORY B — SPEC-COMPLIANT WEIRD-ORDER REGRESSIONS (protocol, not trace).
//!
//! One named, commented test per hard-won C incident, each reproducing the *shape*
//! of the bug in abstract `RmEvent`s + mock ops and asserting the rewrite handles it
//! where the C failed. These are the orders "normal workflows don't hit but the spec
//! allows" — the protocol-not-trace principle (decision #4) made testable. Each
//! test's doc comment names the C incident it guards against.
//!
//! Every test drives the real `Gpu` spine (apply → project → sync) and/or the fwd
//! plane through `MockArch`/`MockIsolate`/`MockVmm` — no GPU, no OS.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use nvkvm_arch::ids::GpuId;
use nvkvm_arch::ids::{GpuVa, HClient, HObject, Pdb, VChid};
use nvkvm_completion::OsEventRef;
use nvkvm_core::gpa::GpaSpace;
use nvkvm_core::gpu::Gpu;
use nvkvm_core::project::project;
use nvkvm_core::rmgraph::{AllocFacts, NodeKey, RmEvent, RmGraph};
use nvkvm_fwd::{FwdFault, handle_doorbell, publish_backing, resolve};
use nvkvm_mocks::{MockArch, MockIsolateFactory, mock_classes as mc};
use nvkvm_tests::{Scenario, identical_handles};

/// Build a fresh empty Gpu with a generous 4-GiB-arena window.
fn fresh_gpu() -> (Gpu, std::sync::Arc<std::sync::Mutex<nvkvm_mocks::RmRecorder>>) {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    (Gpu::new(arch, Box::new(factory), gpa).expect("device realizes"), rec)
}

/// Apply a whole scenario's events to a Gpu, asserting each applies cleanly.
fn apply_all(gpu: &mut Gpu, evs: impl IntoIterator<Item = RmEvent>) {
    for ev in evs {
        gpu.apply(ev).expect("scenario event applies");
    }
}

// =================================================================================
// #12 — 2nd-context: create → destroy → RECREATE reusing identical handles/VAs.
// =================================================================================

/// **Guards #12** (`mode2_12_layered_status`, cont.17-34): CTX1 is created and torn
/// down, then CTX2 is created **reusing the identical guest handles AND the identical
/// guest VA**. In the C this left stale per-context state (a sticky one-shot
/// `doorbell_setup`, an `ALREADY-MAPPED` sysmem pin) so CTX2's channel rang
/// off-runlist / re-backed onto a stale mapping.
///
/// The rewrite must: (1) after CTX1's client-root free, its Proc/Vas/channels vanish
/// from every projection; (2) CTX2, using the SAME handles/VA/PDB, derives a FRESH
/// Proc and backs the identical VA cleanly with no residue; (3) CTX2's doorbell
/// schedules its OWN channel (no sticky one-shot left armed by CTX1).
#[test]
fn wo_12_second_context_recreate_identical_handles_no_stale_state() {
    let (mut gpu, recorder) = fresh_gpu();
    const CLIENT: HClient = HClient(0xAA);
    const PDB: Pdb = Pdb(0x3401_000);
    const VA: GpuVa = GpuVa(0x2_0020_0000);
    let gr_token = MockArch::token_for(VChid(0x10));

    // ---- CTX1: build, back its VA, ring its doorbell (schedules its channel) ----
    let mut s1 = Scenario::new();
    s1.compute_process(CLIENT, PDB, identical_handles(0x10, 0x11));
    apply_all(&mut gpu, s1.events);

    let pid1 = *gpu.by_pdb.get(&(GpuId::ZERO, PDB)).expect("CTX1 routed by PDB");
    publish_backing(gpu.procs.get_mut(&pid1).unwrap(), GpuId::ZERO, PDB, VA, 0x10000).expect("CTX1 backs VA");
    let out1 = handle_doorbell(&mut gpu, GpuId::ZERO, gr_token, &[]).expect("CTX1 doorbell");
    assert!(out1.scheduled_now, "CTX1's channel scheduled on first submit");

    // ---- CTX1 teardown: free the client root (destroys the whole namespace) ----
    gpu.apply(RmEvent::Free { client: CLIENT, handle: HObject(0x5c00_0000) }).expect("free CTX1");
    assert!(!gpu.by_pdb.contains_key(&(GpuId::ZERO, PDB)), "CTX1's PDB no longer routes after teardown");
    assert!(!gpu.by_vchid.contains_key(&(GpuId::ZERO, VChid(0x10))), "CTX1's vChid retired");
    // The old Proc is retired (staged reap), not live — cross-teardown use is refused.
    assert!(gpu.procs.values().all(|p| !p.clients.contains(&CLIENT)), "CTX1 Proc gone from live set");

    // ---- CTX2: IDENTICAL handles, IDENTICAL VA, IDENTICAL PDB — a fresh context ----
    let mut s2 = Scenario::new();
    s2.compute_process(CLIENT, PDB, identical_handles(0x10, 0x11));
    apply_all(&mut gpu, s2.events);

    let pid2 = *gpu.by_pdb.get(&(GpuId::ZERO, PDB)).expect("CTX2 routed by its (reused) PDB");
    // A FRESH Proc: a new isolate session was spawned for CTX2 (no stale reuse).
    let pub2 = publish_backing(gpu.procs.get_mut(&pid2).unwrap(), GpuId::ZERO, PDB, VA, 0x10000)
        .expect("CTX2 backs the IDENTICAL VA cleanly — no ALREADY-MAPPED residue");
    // CTX2 resolves its OWN fresh backing at the identical VA.
    let (bind2, off) = resolve(&gpu, GpuId::ZERO, PDB, GpuVa(VA.0 + 0x100)).unwrap();
    assert_eq!(off, 0x100);
    assert_eq!(bind2.host_va, Some(pub2.host_va), "CTX2 resolves its own backing");

    // CTX2's doorbell schedules CTX2's OWN channel — the sticky-one-shot #12 bug
    // would have left it off-runlist (scheduled_now == false).
    let out2 = handle_doorbell(&mut gpu, GpuId::ZERO, gr_token, &[]).expect("CTX2 doorbell");
    assert!(out2.scheduled_now, "CTX2's fresh channel is scheduled (no CTX1 one-shot residue)");

    // Two distinct isolate sessions were spawned across the two contexts.
    let sessions: Vec<_> = recorder.lock().unwrap().log.iter().map(|(id, _)| id.0).collect();
    let distinct: std::collections::BTreeSet<_> = sessions.into_iter().collect();
    assert!(distinct.len() >= 2, "CTX1 and CTX2 ran on distinct isolate sessions");
}

// =================================================================================
// #13 — multi-iteration realloc churn: free+rebind the same VA at a new backing.
// =================================================================================

/// **Guards #13** (`mode2_13_multiiter_idle_hang`, the `cup8_iter` shape): a compute
/// process runs N iterations, each `alloc → use → free → realloc the SAME guest VA at
/// a NEW backing`. In the C, iteration 2's remap landed on a stale/unbacked mapping
/// (walker dropped the PT write → FAULT_PDE). The rewrite's address table is
/// unmap-eager / map-lazy: each iteration must resolve to ITS OWN current backing,
/// and rebinding the same VA WITHOUT an eager unbind must be a loud overlap fault
/// (never a silent stale-mapping reuse).
#[test]
fn wo_13_multiiter_realloc_same_va_new_backing_each_iter() {
    let (mut gpu, _rec) = fresh_gpu();
    const CLIENT: HClient = HClient(0xAA);
    const PDB: Pdb = Pdb(0x3401_000);
    const VA: GpuVa = GpuVa(0x2_0020_0000);

    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB, identical_handles(0x10, 0x11));
    apply_all(&mut gpu, s.events);
    let pid = *gpu.by_pdb.get(&(GpuId::ZERO, PDB)).unwrap();

    let mut last_gpa = None;
    for iter in 0..5u64 {
        let p = gpu.procs.get_mut(&pid).unwrap();

        // Overlap discipline: rebinding the live VA without unbind first is LOUD.
        if last_gpa.is_some() {
            let overlap = publish_backing(p, GpuId::ZERO, PDB, VA, 0x10000);
            assert!(
                matches!(overlap, Err(FwdFault::Address(_))),
                "iter {iter}: re-back over a live mapping must be a loud overlap fault"
            );
            // Eager unmap (unmap eager, map lazy, reclaim deferred).
            p.vases.get_mut(&(GpuId::ZERO, PDB)).unwrap().table.unbind(VA).expect("eager unbind");
        }

        // Realloc the SAME VA at a fresh backing.
        let published = publish_backing(p, GpuId::ZERO, PDB, VA, 0x10000)
            .unwrap_or_else(|e| panic!("iter {iter}: realloc same VA must succeed: {e:?}"));
        // Each iteration's backing is at a NEW GPA (bump allocator never reuses).
        if let Some(prev) = last_gpa {
            assert_ne!(published.gpa, prev, "iter {iter}: realloc lands at a NEW backing");
        }
        // Resolution returns THIS iteration's current backing, not a stale one.
        let (bind, _) = resolve(&gpu, GpuId::ZERO, PDB, VA).unwrap();
        assert_eq!(bind.phys, published.gpa, "iter {iter}: resolves its own current backing");
        last_gpa = Some(published.gpa);
    }
}

// =================================================================================
// #14 — concurrent identical-VA / identical-handle two-proc, interleaved op order.
// =================================================================================

/// **Guards #14** (`mode2_14_concurrent_apps`): two processes with IDENTICAL guest
/// handles AND IDENTICAL guest VAs, but with their protocol events **interleaved**
/// (not one process fully built then the other). In the C, the shared GPA arena
/// collided (`back_sys ALREADY-MAPPED`). The rewrite must derive disjoint per-Vas
/// backing regardless of the interleave order — the projection is a pure function of
/// the graph, not of arrival order.
#[test]
fn wo_14_two_proc_identical_va_interleaved_events_disjoint_backing() {
    let (mut gpu, _rec) = fresh_gpu();
    const A: HClient = HClient(0xAA);
    const B: HClient = HClient(0xBB);
    const A_PDB: Pdb = Pdb(0x3401_000);
    const B_PDB: Pdb = Pdb(0x3405_000);
    const VA: GpuVa = GpuVa(0x2_0020_0000);

    // Build both processes' event lists, then INTERLEAVE them (round-robin).
    let mut sa = Scenario::new();
    sa.compute_process(A, A_PDB, identical_handles(0x10, 0x11));
    let mut sb = Scenario::new();
    sb.compute_process(B, B_PDB, identical_handles(0x20, 0x21));

    let (ea, eb) = (sa.events, sb.events);
    let max = ea.len().max(eb.len());
    for i in 0..max {
        if let Some(&e) = ea.get(i) {
            gpu.apply(e).expect("A event applies interleaved");
        }
        if let Some(&e) = eb.get(i) {
            gpu.apply(e).expect("B event applies interleaved");
        }
    }

    // Two disjoint Procs despite interleaving and identical handles.
    assert_eq!(gpu.procs.len(), 2, "interleaved build still yields two disjoint Procs");
    let pid_a = *gpu.by_pdb.get(&(GpuId::ZERO, A_PDB)).unwrap();
    let pid_b = *gpu.by_pdb.get(&(GpuId::ZERO, B_PDB)).unwrap();
    assert_ne!(pid_a, pid_b);

    // Identical guest VA in each → disjoint GPA and disjoint host VA.
    let pa = publish_backing(gpu.procs.get_mut(&pid_a).unwrap(), GpuId::ZERO, A_PDB, VA, 0x10000).unwrap();
    let pb = publish_backing(gpu.procs.get_mut(&pid_b).unwrap(), GpuId::ZERO, B_PDB, VA, 0x10000).unwrap();
    assert_ne!(pa.gpa, pb.gpa, "identical VA, interleaved order → disjoint GPA");
    assert_ne!(pa.host_va, pb.host_va, "identical VA, interleaved order → disjoint host VA");
}

// =================================================================================
// teardown-during-active — free a client while its channels have in-flight work.
// =================================================================================

/// **Guards the teardown-mid-flight class** (lesson L10, the deferred-reap-at-quiesce
/// rule): free a Proc's client root while it still has an outstanding, in-flight
/// completion. The reap must be clean — the Proc leaves the live set, its routing
/// vanishes, a doorbell to its retired channel is REFUSED (never consumed against
/// stale state), and nothing panics.
#[test]
fn wo_teardown_during_active_inflight_completion_is_clean() {
    let (mut gpu, _rec) = fresh_gpu();
    const CLIENT: HClient = HClient(0xAA);
    const PDB: Pdb = Pdb(0x3401_000);
    let gr_token = MockArch::token_for(VChid(0x10));

    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB, identical_handles(0x10, 0x11));
    apply_all(&mut gpu, s.events);

    let pid = *gpu.by_pdb.get(&(GpuId::ZERO, PDB)).unwrap();
    publish_backing(gpu.procs.get_mut(&pid).unwrap(), GpuId::ZERO, PDB, GpuVa(0x2_0020_0000), 0x10000).unwrap();
    handle_doorbell(&mut gpu, GpuId::ZERO, gr_token, &[]).expect("channel rung");
    // An in-flight completion is pending for this proc.
    gpu.procs.get_mut(&pid).unwrap().completion.observe(OsEventRef(0xDEAD)).unwrap();
    assert!(gpu.procs.get(&pid).unwrap().completion.has_outstanding());

    // Teardown WHILE that completion is outstanding.
    gpu.apply(RmEvent::Free { client: CLIENT, handle: HObject(0x5c00_0000) }).expect("free");

    // Clean reap: the proc is out of the live set, its routing is gone.
    assert!(!gpu.procs.contains_key(&pid), "torn-down proc left the live set");
    assert!(!gpu.by_pdb.contains_key(&(GpuId::ZERO, PDB)) && !gpu.by_vchid.contains_key(&(GpuId::ZERO, VChid(0x10))));
    // The retired proc was staged for deferred reap (L10), not dropped mid-flight.
    assert!(gpu.retired.iter().any(|p| p.is_retired()), "proc staged for deferred reap");
    // A doorbell to the now-retired channel is a loud MISS, never a stale consume.
    assert!(matches!(handle_doorbell(&mut gpu, GpuId::ZERO, gr_token, &[]), Err(FwdFault::UnknownVchid { .. })));
}

// =================================================================================
// dup-then-free-src — DUP an object, then free the SOURCE client.
// =================================================================================

/// **Guards the dup-src-lifetime class** (RM DUP_OBJECT semantics, decision #14): UVM
/// DUPs the compute client's VASpace, then the COMPUTE (source) client is freed while
/// UVM still holds the alias. Per RM refcounting (`RsResource`, `resource_list.h`) the
/// resource stays alive via UVM's reference — freeing the source must NOT destroy it,
/// and the dst alias + its Proc grouping/VAS/PDB must remain resolvable. (A naive impl
/// that tied the resource's life to the src client would drop UVM's VAS out from under
/// it — the gap this test now pins CLOSED.)
///
/// Covers all three lifetime corners: free-src (dst survives), free-dst (src survives),
/// and free-both (resource finally destroyed — no leak).
#[test]
fn wo_dup_then_free_src_keeps_dst_alias_alive() {
    let arch = MockArch::new();

    // Compute client with a VASpace (the dup source), UVM client dups it.
    const COMPUTE: HClient = HClient(0xAA);
    const UVM: HClient = HClient(0xA1);
    const PDB: Pdb = Pdb(0x3401_000);
    let compute_vas = NodeKey::new(COMPUTE, HObject(0x5c00_0010));
    let alias = NodeKey::new(UVM, HObject(0x9000_00ff));

    // Reusable fixture builder: compute (client→device→VASpace+PDB) + UVM dup of the VAS.
    let build = || {
        let mut g = RmGraph::new();
        g.apply(&arch, RmEvent::Alloc { client: COMPUTE, parent: HObject(0x5c00_0000), handle: HObject(0x5c00_0000), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
        g.apply(&arch, RmEvent::Alloc { client: COMPUTE, parent: HObject(0x5c00_0000), handle: HObject(0x5c00_0001), class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
        g.apply(&arch, RmEvent::Alloc { client: COMPUTE, parent: HObject(0x5c00_0001), handle: HObject(0x5c00_0010), class: mc::VASPACE, facts: AllocFacts::default() }).unwrap();
        g.apply(&arch, RmEvent::SetPageDir { client: COMPUTE, vaspace: HObject(0x5c00_0010), pdb: PDB }).unwrap();
        g.apply(&arch, RmEvent::Alloc { client: UVM, parent: HObject(0x9000_0000), handle: HObject(0x9000_0000), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
        g.apply(&arch, RmEvent::Dup { src: compute_vas, dst: alias }).unwrap();
        g
    };

    // ---- Baseline: the dup joins compute+UVM into ONE proc owning the VAS/PDB. ----
    {
        let g = build();
        let before = project(&g, &arch).unwrap();
        assert_eq!(before.procs.len(), 1, "dup joins compute+UVM into one proc");
        assert_eq!(before.by_pdb.get(&(GpuId::ZERO, PDB)).map(|x| x.1), Some(compute_vas), "PDB routes to the VAS");
    }

    // ---- (A) free the SOURCE client: the resource SURVIVES via UVM's dst ref. ----
    {
        let mut g = build();
        g.apply(&arch, RmEvent::Free { client: COMPUTE, handle: HObject(0x5c00_0000) }).unwrap();

        // The alias still resolves to the (living) origin VASpace resource...
        let origin = g.origin_of(alias).expect("dst alias survives the source's free");
        assert_eq!(origin.key, compute_vas, "alias resolves to the same stable resource identity");

        // ...and the projection keeps the VAS/PDB routed, now grouped under UVM's proc.
        let after = project(&g, &arch).unwrap();
        assert!(
            after.procs.iter().any(|p| p.clients.contains(&UVM)),
            "UVM's client still projects after the source client's free"
        );
        assert!(after.by_pdb.contains_key(&(GpuId::ZERO, PDB)), "the dup'd VAS's PDB still routes via the dst ref");
        // The compute client's OTHER nodes (device root) are gone — only the dup'd
        // resource, kept alive by UVM's reference, survives.
        assert!(g.node(NodeKey::new(COMPUTE, HObject(0x5c00_0001))).is_none(), "non-dup'd source nodes are freed");
    }

    // ---- (B) symmetric: free the DST client instead → the SOURCE survives. ----
    {
        let mut g = build();
        g.apply(&arch, RmEvent::Free { client: UVM, handle: HObject(0x9000_0000) }).unwrap();
        // The origin still resolves via its own (source) handle.
        assert!(g.origin_of(compute_vas).is_some(), "source VAS survives the dst client's free");
        assert!(g.origin_of(alias).is_none(), "the freed dst alias no longer resolves");
        let after = project(&g, &arch).unwrap();
        assert!(after.by_pdb.contains_key(&(GpuId::ZERO, PDB)), "source VAS's PDB still routes");
        // With the dup edge gone, compute stands alone as its own proc.
        assert!(after.procs.iter().all(|p| !p.clients.contains(&UVM)), "freed UVM client gone");
    }

    // ---- (C) free BOTH references → the resource is finally destroyed (NO LEAK). ----
    {
        let mut g = build();
        g.apply(&arch, RmEvent::Free { client: COMPUTE, handle: HObject(0x5c00_0000) }).unwrap();
        assert!(g.origin_of(alias).is_some(), "still alive after the first of two refs freed");
        g.apply(&arch, RmEvent::Free { client: UVM, handle: HObject(0x9000_0000) }).unwrap();
        // Last reference gone: the resource is removed and nothing dangles.
        assert!(g.origin_of(alias).is_none(), "resource destroyed once its LAST reference is freed");
        assert!(g.origin_of(compute_vas).is_none(), "no residual origin after full teardown");
        assert_eq!(g.nodes().count(), 0, "no leaked resources after both refs freed");
        let after = project(&g, &arch).unwrap();
        assert!(after.procs.is_empty(), "no procs remain after full teardown");
        assert!(!after.by_pdb.contains_key(&(GpuId::ZERO, PDB)), "PDB no longer routes after the resource is destroyed");
    }
}

// =================================================================================
// retried / duplicate events — same Alloc / Dup delivered twice is idempotent.
// =================================================================================

/// **Guards the retried-RPC class** (`wo_retried_rpc`, testing strategy §3): the guest
/// RM may retry an `RM_ALLOC` / `DUP_OBJECT`; the same event delivered twice must be
/// IDEMPOTENT — no double-count, no conflicting-alloc error, identical projection. A
/// *different* alloc for the same key is still a loud conflict (not silently merged).
#[test]
fn wo_retried_duplicate_events_are_idempotent() {
    let (mut gpu, _rec) = fresh_gpu();
    const CLIENT: HClient = HClient(0xAA);
    const PDB: Pdb = Pdb(0x3401_000);

    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB, identical_handles(0x10, 0x11));
    let events = s.events;

    // Deliver every event, then deliver every event AGAIN (a full retry storm).
    apply_all(&mut gpu, events.clone());
    let after_first: usize = gpu.procs.len();
    let bounds_first = project(&gpu.rmgraph, gpu.arch.as_ref()).unwrap();

    // The retried delivery must be accepted idempotently (no ConflictingAlloc).
    for ev in &events {
        gpu.apply(*ev).expect("retried event is idempotent, not a conflict");
    }
    let bounds_second = project(&gpu.rmgraph, gpu.arch.as_ref()).unwrap();

    assert_eq!(gpu.procs.len(), after_first, "retry did not create a second Proc");
    assert_eq!(bounds_first, bounds_second, "retried events yield identical boundaries");

    // But a CONFLICTING alloc (same key, different class) is still loud.
    let conflict = gpu.apply(RmEvent::Alloc {
        client: CLIENT,
        parent: HObject(0x5c00_0001),
        handle: HObject(0x5c00_0010), // the VASpace handle...
        class: mc::MEMORY,            // ...but a DIFFERENT class than declared
        facts: AllocFacts::default(),
    });
    assert!(conflict.is_err(), "a conflicting redefinition of an existing key is loud");
}
