//! # SECURITY BOUNDARY PASS — the boundary-1 posture, as tests (decision #18A)
//!
//! The priority ladder (decision #8) puts **catastrophic security boundaries FIRST**,
//! above even correctness comprehensiveness. This suite is that ladder rung made
//! executable: a **compromised / hostile guest process** — modeled as arbitrary,
//! adversarial input on the guest→core attack surface (RM event streams, pushbuffers,
//! doorbell tokens, addresses/offsets) — must NEVER be able to
//!
//!   1. read, corrupt, or influence **another process's** state (boundary-1 / #9),
//!   2. cause **unbounded allocation** (OOM the host, taking every VM down),
//!   3. **panic / abort** the core, or
//!   4. obtain a silent **wrong-resolve or cross-process leak** (MISS=FAULT), or
//!   5. confuse the **handle/namespace** into a cross-object / cross-proc binding, or
//!   6. exhaust a resource **non-gracefully** (corruption instead of a loud fault).
//!
//! Every test's doc names WHICH boundary (decision #9) it guards. The core is pure
//! logic (no OS/time/NVIDIA deps, `#![forbid(unsafe_code)]`), so this whole suite runs
//! at unit speed with no GPU/hypervisor/OS — the hexagonal payoff.
//!
//! ## ⚠ Explicitly OUT OF SCOPE here (honestly deferred to L1, decisions #16/#16b)
//!
//! **Pointer-bounds / host-memory OOB is NOT a core concern and is NOT faked here.**
//! The pure core has NO host memory, NO raw pointers, and NO `unsafe` — the entire
//! memory-safety *breakout* surface is confined OFF the core to the future L1 VMM
//! adapter (the hexagonal win). The bounded-memory type, its `trybuild` compile-fail
//! assertions, and fuzz-the-offsets tests live in that L1 layer and are written WHEN
//! it is built. Asserting them against the core would be theater. What the core CAN
//! and DOES assert — because its whole attack surface is the guest→core *logical*
//! input — is everything below.
//!
//! ## Relationship to the existing fuzz suite
//!
//! `fuzz_rmgraph_invariants.rs` (A1–A4) already fuzzes the `RmGraph`/`project`
//! never-panic + structural-invariant + refcount properties. This suite BUILDS ON
//! that (it does not re-derive it): it adds the **cross-process isolation** property
//! end-to-end through `Gpu`, extends never-panic to the **fwd plane** (doorbell +
//! pushbuffer + resolve), and asserts the **unbounded-allocation caps** and
//! **confused-deputy** rejections as dedicated, named boundary guards.

#![allow(clippy::unusual_byte_groupings)]

use nvkvm_arch::ids::{ClassId, GpuVa, HClient, HObject, Pdb, VChid};
use nvkvm_completion::{
    CompletionError, CompletionQueue, OsEventRef, MAX_OUTSTANDING_COMPLETIONS,
};
use nvkvm_core::gpa::GpaSpace;
use nvkvm_core::gpu::{Gpu, GpuError};
use nvkvm_core::project::{project, Boundaries, ProcBoundary};
use nvkvm_core::rmgraph::{
    AllocFacts, Capacity, NodeKey, RmEvent, RmGraph, RmGraphError, MAX_LIVE_HANDLES,
    MAX_LIVE_MAPPINGS, MAX_PARKED,
};
use nvkvm_fwd::{handle_doorbell, parse_pushbuffer, resolve, FwdFault};
use nvkvm_mocks::{mock_classes as mc, MockArch, MockIsolateFactory, MockVmm};
use nvkvm_tests::{identical_handles, Scenario};
use nvkvm_vmm::Vmm;
use proptest::prelude::*;

// =================================================================================
// Shared fixtures
// =================================================================================

fn fresh_gpu() -> Gpu {
    let arch = Box::new(MockArch::new());
    let (factory, _rec) = MockIsolateFactory::new();
    // A generous window so exhaustion is a deliberate act, not an accident.
    let gpa = GpaSpace::new(0x1_0000_0000..0x1000_0000_0000, 0x1_0000_0000);
    Gpu::new(arch, Box::new(factory), gpa).expect("device realizes")
}

// The BENIGN victim process B — a full compute proc plus a mapped MEMORY object, so
// its projection AND its address-table resolutions are observable and comparable.
const B_CLIENT: HClient = HClient(0xB000);
const B_PDB: Pdb = Pdb(0x000B_0000);
const B_MEM: HObject = HObject(0x5c00_0100);
const B_MEM_PHYS: u64 = 0x9_0000_0000;
const B_MAP_VA: GpuVa = GpuVa(0x2_0020_0000);
const B_MAP_LEN: u64 = 0x10000;

/// B's canonical event set (a compute process + a mapped memory object). Handle
/// VALUES deliberately overlap the hostile generator's (the #14 identical-handle
/// shape); only the owning `HClient` differs, which is exactly the isolation the
/// core must uphold.
fn benign_b_events() -> Vec<RmEvent> {
    let mut s = Scenario::new();
    let h = identical_handles(0x40, 0x41);
    s.compute_process(B_CLIENT, B_PDB, h);
    s.memory(B_CLIENT, h.device, B_MEM, B_MEM_PHYS);
    s.map(B_CLIENT, h.vaspace, B_MEM, B_MAP_VA, B_MAP_LEN);
    s.events
}

/// The `ProcBoundary` of the process that owns `client`, or `None` if absent.
fn boundary_of(b: &Boundaries, client: HClient) -> Option<ProcBoundary> {
    b.procs.iter().find(|p| p.clients.contains(&client)).cloned()
}

// =================================================================================
// BOUNDARY 1 — CROSS-PROCESS ISOLATION (the core boundary, #9)
// =================================================================================

/// A hostile process A's arbitrary event: over a client universe DISJOINT from B's
/// (`0xA000..0xA004`) but with **handle and VA** values that deliberately COLLIDE with
/// B's (the #14 identical-value shape — the per-namespace / per-VAS identities that MUST
/// stay isolated unconditionally). Includes junk classes, dangling parents, double-frees,
/// dup cycles — the full adversarial menu.
///
/// A does NOT forge B's PDB or B's vChid — the *global hardware identities* (a physical
/// page-directory base, a channel's HW id). Those are assigned by the guest kernel from
/// physically-distinct resources, so a compromised *userspace* process (boundary-1)
/// cannot pick them; forging them is a compromised-guest-*kernel* act, whose contained
/// (non-catastrophic) handling is asserted separately by
/// [`b1_hw_identity_squat_is_contained_and_third_party_safe`]. Keeping A off B's global
/// identities here isolates the property under test: identical per-namespace identities
/// across processes never interfere.
fn any_a_event() -> impl Strategy<Value = RmEvent> {
    let a_client = || (0u32..4).prop_map(|n| HClient(0xA000 + n));
    let a_handle = || (0u32..6).prop_map(|n| HObject(0x5c00_0000 + n));
    let any_class = || {
        prop_oneof![
            Just(mc::CLIENT),
            Just(mc::DEVICE),
            Just(mc::VASPACE),
            Just(mc::TSG),
            Just(mc::CHANNEL_GR),
            Just(mc::CHANNEL_CE),
            Just(mc::MEMORY),
            (0u32..0x10000).prop_map(ClassId),
        ]
    };
    // A's OWN PDBs / vChids (disjoint from B's B_PDB and B's vChids 0x40/0x41).
    let a_pdb = || (0u32..4).prop_map(|n| Pdb(0x000A_0000 + u64::from(n)));
    // Collide with B's VA on purpose (the per-VAS identity that MUST stay isolated).
    let a_va = || prop_oneof![Just(B_MAP_VA), (0u32..4).prop_map(|n| GpuVa(0x2_0020_0000 + u64::from(n) * 0x1000))];
    prop_oneof![
        (a_client(), a_handle(), a_handle(), any_class(), 0u32..0x20000).prop_map(
            |(client, parent, handle, class, flags)| RmEvent::Alloc {
                client,
                parent,
                handle,
                class,
                facts: AllocFacts {
                    h_vaspace: (flags & 1 == 0).then_some(HObject(0x5c00_0000 + (flags & 3))),
                    h_ctx_share: None,
                    // A's channels take vChids in a high range disjoint from B's — a
                    // vChid is a global HW id (see the doc above), not forged here.
                    userd_flags: MockArch::userd_flags_for(VChid(0x900 + (flags & 0xff) as u16)),
                    mem_phys: (flags & 2 == 0).then_some(0x8000_0000 | u64::from(flags)),
                },
            }
        ),
        (a_client(), a_handle(), a_handle(), a_va()).prop_map(|(client, vaspace, memory, va)| {
            RmEvent::MapMemoryDma { client, vaspace, memory, va, offset: 0, len: 0x10000 }
        }),
        (a_client(), a_handle(), a_va())
            .prop_map(|(client, vaspace, va)| RmEvent::Unmap { client, vaspace, va }),
        (a_client(), a_handle(), a_client(), a_handle()).prop_map(|(sc, sh, dc, dh)| {
            RmEvent::Dup { src: NodeKey::new(sc, sh), dst: NodeKey::new(dc, dh) }
        }),
        (a_client(), a_handle(), a_pdb())
            .prop_map(|(client, vaspace, pdb)| RmEvent::SetPageDir { client, vaspace, pdb }),
        (a_client(), a_handle()).prop_map(|(client, handle)| RmEvent::Free { client, handle }),
    ]
}

/// Interleave B's benign events with A's hostile events per a boolean weave (take-B
/// when true, else take-A), so A acts WHILE B is mid-setup — the most adversarial
/// timing. Tag each so the harness can assert B's events always succeed.
fn interleave(b: &[RmEvent], a: &[RmEvent], weave: &[bool]) -> Vec<(bool, RmEvent)> {
    let (mut bi, mut ai) = (0usize, 0usize);
    let mut out = Vec::with_capacity(a.len() + b.len());
    for &take_b in weave {
        if take_b && bi < b.len() {
            out.push((true, b[bi]));
            bi += 1;
        } else if ai < a.len() {
            out.push((false, a[ai]));
            ai += 1;
        } else if bi < b.len() {
            out.push((true, b[bi]));
            bi += 1;
        }
    }
    while bi < b.len() {
        out.push((true, b[bi]));
        bi += 1;
    }
    while ai < a.len() {
        out.push((false, a[ai]));
        ai += 1;
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// **Boundary 1.** No hostile event stream from process A can read, corrupt, or
    /// influence process B's state. Interleaving arbitrary A-hostile events with
    /// B-benign ones, in an adversarial order, must leave:
    ///   - every one of B's own events SUCCEEDING (A never poisons B's control path —
    ///     the atomic-apply property: a hostile event that would make the device
    ///     unprojectable is rolled back, never wedging the device);
    ///   - B's projected boundary (clients/vases/channels) BYTE-IDENTICAL to a
    ///     world where A never existed;
    ///   - B's address-table resolutions UNCHANGED (its VA still resolves to ITS
    ///     backing, never A's);
    ///   - B's GPA arena DISJOINT from every A arena.
    #[test]
    fn b1_hostile_a_cannot_influence_b(
        a_events in prop::collection::vec(any_a_event(), 0..30),
        weave in prop::collection::vec(any::<bool>(), 0..60),
    ) {
        let b_events = benign_b_events();

        // Reference world: B alone.
        let mut ref_gpu = fresh_gpu();
        for ev in &b_events {
            ref_gpu.apply(*ev).expect("benign B applies cleanly on its own");
        }
        let ref_bounds = project(&ref_gpu.rmgraph, ref_gpu.arch.as_ref()).expect("B projects");
        let b_ref = boundary_of(&ref_bounds, B_CLIENT).expect("B has a boundary");

        // Adversarial world: A interleaved with B.
        let mut gpu = fresh_gpu();
        for (is_b, ev) in interleave(&b_events, &a_events, &weave) {
            let r = gpu.apply(ev);
            if is_b {
                // A's hostility must NEVER cause one of B's benign events to fail.
                prop_assert!(r.is_ok(), "B's benign event refused under A's hostility: {r:?}");
            }
            // A's events: a loud typed refusal is fine; a panic/abort is not (we got here).
        }

        // The device still projects (a hostile A left it in a consistent state).
        let bounds = project(&gpu.rmgraph, gpu.arch.as_ref())
            .expect("device still projects after hostile A");
        let b_now = boundary_of(&bounds, B_CLIENT).expect("B still has a boundary");
        prop_assert_eq!(b_ref, b_now, "B's boundary changed under A's hostility");

        // B's address plane is unchanged: its mapped VA resolves to ITS backing.
        let got = resolve(&gpu, B_PDB, B_MAP_VA).map(|(bind, _)| bind.phys);
        prop_assert_eq!(got, Ok(B_MEM_PHYS), "B's VA no longer resolves to its own backing");

        // B's arena is disjoint from every A arena (no shared GPA — #14 isolation).
        // (The arena RANGE is legitimately order-dependent — A creating procs first
        // shifts B's slot — so only DISJOINTNESS is the invariant, never the range.)
        let b_arena = gpu
            .procs
            .values()
            .find(|p| p.clients.contains(&B_CLIENT))
            .map(|p| p.arena.range.clone())
            .expect("B still has an arena");
        for p in gpu.procs.values() {
            if p.clients.contains(&B_CLIENT) {
                continue;
            }
            prop_assert!(
                p.arena.range.end <= b_arena.start || b_arena.end <= p.arena.range.start,
                "an A proc's arena overlaps B's — cross-process GPA collision"
            );
        }
    }
}

/// **Boundary 1 (the fixed poisoning bug).** A hostile process declaring two VASpaces
/// with the SAME PDB makes the global projection collide — but that collision must be
/// contained to the offending event, never wedge the device for OTHER processes. This
/// is the atomic-apply guarantee (a faulting derivation is rolled back).
#[test]
fn b1_projection_collision_is_contained_not_a_device_wedge() {
    let mut gpu = fresh_gpu();
    let a = HClient(0xA000);
    let root = |c: HClient| RmEvent::Alloc {
        client: c,
        parent: HObject(c.0),
        handle: HObject(c.0),
        class: mc::CLIENT,
        facts: AllocFacts::default(),
    };
    gpu.apply(root(a)).unwrap();
    gpu.apply(RmEvent::Alloc { client: a, parent: HObject(a.0), handle: HObject(1), class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
    gpu.apply(RmEvent::Alloc { client: a, parent: HObject(1), handle: HObject(2), class: mc::VASPACE, facts: AllocFacts::default() }).unwrap();
    gpu.apply(RmEvent::Alloc { client: a, parent: HObject(1), handle: HObject(3), class: mc::VASPACE, facts: AllocFacts::default() }).unwrap();
    gpu.apply(RmEvent::SetPageDir { client: a, vaspace: HObject(2), pdb: Pdb(0xBAD) }).unwrap();

    // The colliding second bind is a LOUD, contained refusal…
    let collide = gpu.apply(RmEvent::SetPageDir { client: a, vaspace: HObject(3), pdb: Pdb(0xBAD) });
    assert!(
        matches!(collide, Err(GpuError::Projection(_))),
        "PDB collision must be a loud projection fault, got {collide:?}"
    );

    // …and the device is NOT wedged: a wholly-separate process B proceeds normally.
    for ev in benign_b_events() {
        gpu.apply(ev).expect("B is unaffected by A's projection collision");
    }
    assert!(resolve(&gpu, B_PDB, B_MAP_VA).is_ok(), "B fully functional after A's collision");
}

/// ★ Mutation-gate kill (`project` `prev != node.key`→`==` on the vChid dedup, and the
/// same exec-plane twin of the PDB collision above): two DISTINCT channel nodes that
/// decode to the SAME vChid make the exec-plane demux ambiguous — that MUST be a loud
/// `VchidCollision`, never a silent last-writer-wins (which would route one guest's
/// doorbell to another's channel). The prior suite had a PDB-collision test but no
/// vChid one, and the `!=`→`==` mutant silently accepts the collision (the guard only
/// fires on `prev == node.key`, which never happens for two distinct nodes). This pins
/// it, and confirms the fault is contained (a separate process B is unaffected).
#[test]
fn b1_vchid_collision_is_a_loud_contained_projection_fault() {
    let mut gpu = fresh_gpu();
    let a = HClient(0xA000);
    // Two channels in A's namespace whose userd flags decode to the SAME vChid.
    let dup_vchid = VChid(0x33);
    let flags = MockArch::userd_flags_for(dup_vchid);
    gpu.apply(RmEvent::Alloc { client: a, parent: HObject(a.0), handle: HObject(a.0), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
    gpu.apply(RmEvent::Alloc { client: a, parent: HObject(a.0), handle: HObject(1), class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
    gpu.apply(RmEvent::Alloc { client: a, parent: HObject(1), handle: HObject(2), class: mc::VASPACE, facts: AllocFacts::default() }).unwrap();
    gpu.apply(RmEvent::SetPageDir { client: a, vaspace: HObject(2), pdb: Pdb(0xA11) }).unwrap();
    gpu.apply(RmEvent::Alloc {
        client: a, parent: HObject(1), handle: HObject(0x10), class: mc::CHANNEL_GR,
        facts: AllocFacts { h_vaspace: Some(HObject(2)), userd_flags: flags, ..Default::default() },
    }).unwrap();

    // The SECOND channel claiming the same vChid is a LOUD, contained refusal.
    let collide = gpu.apply(RmEvent::Alloc {
        client: a, parent: HObject(1), handle: HObject(0x11), class: mc::CHANNEL_CE,
        facts: AllocFacts { h_vaspace: Some(HObject(2)), userd_flags: flags, ..Default::default() },
    });
    assert!(
        matches!(collide, Err(GpuError::Projection(_))),
        "two channels decoding to one vChid must be a loud projection fault, got {collide:?}"
    );

    // Contained: a wholly-separate process B proceeds normally after A's collision.
    for ev in benign_b_events() {
        gpu.apply(ev).expect("B unaffected by A's vChid collision");
    }
    assert!(resolve(&gpu, B_PDB, B_MAP_VA).is_ok(), "B fully functional after A's collision");
}

/// **Boundary 1 (honest scope).** A hostile process forging the *global hardware
/// identity* of another — the same physical PDB — is a **contained** loud fault, never
/// catastrophic: the collision is refused (first-declarer keeps the identity), the
/// device stays consistent, and — critically — an INNOCENT THIRD process is entirely
/// unaffected. (A userspace process cannot pick PDBs — the stock guest driver assigns
/// them from physically-distinct page directories — so this is a compromised-guest-
/// *kernel* forgery, decision #9's kernel tier: no crash, no corruption, no cross-VM,
/// no third-party damage. The pure core cannot adjudicate WHICH of two VASes claiming
/// one physical PDB is lying, so "loud + contained" is the correct posture.)
#[test]
fn b1_hw_identity_squat_is_contained_and_third_party_safe() {
    let mut gpu = fresh_gpu();
    // Victim B legitimately owns B_PDB.
    for ev in benign_b_events() {
        gpu.apply(ev).unwrap();
    }
    // Innocent third process C (its own PDB, its own everything).
    const C_CLIENT: HClient = HClient(0xC000);
    const C_PDB: Pdb = Pdb(0x000C_0000);
    let hc = identical_handles(0x70, 0x71);
    let mut sc = Scenario::new();
    sc.compute_process(C_CLIENT, C_PDB, hc);
    for ev in sc.events {
        gpu.apply(ev).unwrap();
    }

    // Attacker A forges B's exact physical PDB.
    const A_CLIENT: HClient = HClient(0xA000);
    let a_root = HObject(0xA1);
    let a_dev = HObject(0xA2);
    let a_vas = HObject(0xA3);
    gpu.apply(RmEvent::Alloc { client: A_CLIENT, parent: a_root, handle: a_root, class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
    gpu.apply(RmEvent::Alloc { client: A_CLIENT, parent: a_root, handle: a_dev, class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
    gpu.apply(RmEvent::Alloc { client: A_CLIENT, parent: a_dev, handle: a_vas, class: mc::VASPACE, facts: AllocFacts::default() }).unwrap();
    let squat = gpu.apply(RmEvent::SetPageDir { client: A_CLIENT, vaspace: a_vas, pdb: B_PDB });

    // The squat is refused (B declared B_PDB first) — loud + contained.
    assert!(matches!(squat, Err(GpuError::Projection(_))), "PDB squat must be a loud fault, got {squat:?}");
    // B keeps its PDB and its mapping — the victim is not corrupted.
    assert_eq!(resolve(&gpu, B_PDB, B_MAP_VA).map(|(b, _)| b.phys), Ok(B_MEM_PHYS));
    assert_eq!(gpu.by_pdb.get(&B_PDB).and_then(|pid| gpu.procs.get(pid)).map(|p| p.clients.contains(&B_CLIENT)), Some(true));
    // The INNOCENT third process C is entirely unaffected — the blast radius never
    // reaches beyond the colliding pair.
    assert!(gpu.by_pdb.contains_key(&C_PDB), "innocent C still routes");
    // And the device as a whole is still consistent (no wedge, no corruption).
    assert!(project(&gpu.rmgraph, gpu.arch.as_ref()).is_ok());
}

/// **Boundary 1.** The per-`Proc` completion plane is private: a hostile process
/// flooding ITS OWN completion queue never grows, drains, or reorders another
/// process's queue (the queues share no state — `execution_plane.md` §4.3.2).
#[test]
fn b1_completion_plane_is_per_process() {
    let mut a = CompletionQueue::new();
    let mut b = CompletionQueue::new();
    b.observe(OsEventRef(0xB0)).unwrap();
    // A floods its own queue hard.
    for i in 0..100_000u64 {
        a.observe(OsEventRef(0xA000_0000 | i)).unwrap();
    }
    // B's queue is exactly what B put there — A's flood is invisible to it.
    assert_eq!(b.outstanding_len(), 1, "A's flood must not reach B's queue");
    assert!(b.has_outstanding());
    b.ack(OsEventRef(0xB0));
    assert!(!b.has_outstanding(), "B drains independently of A");
    // A's queue is unaffected by B's drain.
    assert_eq!(a.outstanding_len(), 100_000);
}

// =================================================================================
// BOUNDARY 1 / 3 — NEVER PANIC ON ANY INPUT, across the FWD plane
// =================================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// **Boundary 1/3.** The FWD plane (doorbell demux + pushbuffer parse + resolve)
    /// never panics on arbitrary hostile input — every path returns a `Result`,
    /// loud-fault at worst. Builds a real proc, then hammers it with arbitrary
    /// doorbell tokens, arbitrary pushbuffer method bytes, and arbitrary resolve VAs.
    /// (Extends the RmGraph fuzz to the exec/address plane — the surface A1 doesn't
    /// reach. The *huge-declared-length* path is covered separately by
    /// `b2_pushbuffer_length_flood_is_bounded`; here the ranges are bounded so the
    /// never-panic fuzz stays cheap while still driving arbitrary method decodes.)
    #[test]
    fn b3_fwd_plane_never_panics(
        tokens in prop::collection::vec(any::<u64>(), 0..8),
        blob in prop::collection::vec(any::<u8>(), 0..512),
        lens in prop::collection::vec(0u64..1024, 0..6),
        // Bounded well below u64::MAX: the core passes gpa straight to the VMM, and the
        // mock's byte-map read would (faithfully to a real bounds-check) not wrap — we
        // are fuzzing the CORE, not the mock's arithmetic.
        gpas in prop::collection::vec(0u64..0x8000_0000_0000, 0..6),
        vas in prop::collection::vec(any::<u64>(), 0..8),
    ) {
        let mut gpu = fresh_gpu();
        for ev in benign_b_events() {
            gpu.apply(ev).expect("B applies");
        }
        let pid = *gpu.by_pdb.get(&B_PDB).expect("B routes");
        let cid = *gpu.procs[&pid].chan_ids.values().next().expect("B has a channel");
        let mut vmm = MockVmm::new();
        // Arbitrary method bytes live in guest RAM; the ring points ranges at them.
        vmm.gpa_write(0x5000_0000, &blob).unwrap();

        for t in tokens {
            // Arbitrary doorbell token → routes to a channel or a loud fault; never panics.
            let _ = handle_doorbell(&mut gpu, t, &[]);
        }
        // A GPFIFO ring of arbitrary (gpa, bounded-len) entries → arbitrary method
        // decodes over arbitrary bytes, always a bounded parse, never a panic.
        let mut ring = Vec::new();
        for (i, &len) in lens.iter().enumerate() {
            let gpa = gpas.get(i).copied().unwrap_or(0x5000_0000);
            ring.extend_from_slice(&gpa.to_le_bytes());
            ring.extend_from_slice(&len.to_le_bytes());
        }
        let _ = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring);
        for v in vas {
            // Arbitrary VA → HIT its backing or a loud MISS; never a wrong-silent-resolve.
            let _ = resolve(&gpu, B_PDB, GpuVa(v));
        }
        // The device is still consistent after all of it.
        prop_assert!(project(&gpu.rmgraph, gpu.arch.as_ref()).is_ok());
    }
}

// =================================================================================
// BOUNDARY 2 — NO UNBOUNDED ALLOCATION FROM GUEST INPUT
// (audited paths: handle table, mapping table, parked-dup/pdb/map tables, the
//  per-proc completion queue, and the pushbuffer read budget — each CAPPED + loud.)
// =================================================================================

/// **Boundary 2.** A handle flood (`Alloc` of distinct clients without bound) is
/// CAPPED at [`MAX_LIVE_HANDLES`] and loud-faults — it never OOMs the host. Driven at
/// the `RmGraph` level (where the cap lives) to keep the flood O(n log n).
#[test]
fn b2_handle_flood_is_capped_loud() {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let mut faulted_at = None;
    for i in 0..=MAX_LIVE_HANDLES as u64 {
        let c = HClient(0x1000_0000 + i as u32); // never wraps within the loop bound
        let ev = RmEvent::Alloc {
            client: c,
            parent: HObject(c.0),
            handle: HObject(c.0),
            class: mc::CLIENT,
            facts: AllocFacts::default(),
        };
        if let Err(RmGraphError::CapacityExceeded(Capacity::Handles)) = g.apply(&arch, ev) {
            faulted_at = Some(i);
            break;
        }
    }
    assert_eq!(
        faulted_at,
        Some(MAX_LIVE_HANDLES as u64),
        "handle table must loud-fault exactly at the cap, not OOM"
    );
}

/// **Boundary 2.** A parked-dup flood (dangling-source `Dup`s that never resolve) is
/// CAPPED at [`MAX_PARKED`] and loud-faults.
#[test]
fn b2_pending_dup_flood_is_capped_loud() {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let mut faulted = false;
    for i in 0..=MAX_PARKED as u64 {
        // Distinct dst, source that will never be allocated → parks forever.
        let dst = NodeKey::new(HClient(0xC000), HObject(i as u32));
        let src = NodeKey::new(HClient(0xDEAD), HObject(0xFFFF_FFFF));
        if let Err(RmGraphError::CapacityExceeded(Capacity::PendingDups)) =
            g.apply(&arch, RmEvent::Dup { src, dst })
        {
            faulted = true;
            break;
        }
    }
    assert!(faulted, "parked-dup table must loud-fault at the cap, not grow unbounded");
}

/// **Boundary 2.** A parked-map flood (orphan `MapMemoryDma`s that never resolve) is
/// CAPPED at [`MAX_PARKED`] and loud-faults — AND the parked table is a set, so the
/// flood is O(n log n) (no O(n²) linear-scan dedup complexity DoS).
#[test]
fn b2_pending_map_flood_is_capped_loud() {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let mut faulted = false;
    for i in 0..=MAX_PARKED as u64 {
        let ev = RmEvent::MapMemoryDma {
            client: HClient(0xC001),
            vaspace: HObject(0xAAAA_AAAA), // never allocated → parks
            memory: HObject(0xBBBB_BBBB),
            va: GpuVa(i * 0x1000),
            offset: 0,
            len: 0x1000,
        };
        if let Err(RmGraphError::CapacityExceeded(Capacity::PendingMaps)) = g.apply(&arch, ev) {
            faulted = true;
            break;
        }
    }
    assert!(faulted, "parked-map table must loud-fault at the cap, not grow unbounded");
}

/// **Boundary 2.** A live-mapping flood (one VAS + one memory, mapped at unbounded
/// distinct VAs) is CAPPED at [`MAX_LIVE_MAPPINGS`] and loud-faults.
#[test]
fn b2_mapping_flood_is_capped_loud() {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let c = HClient(0xC002);
    // One client → device → vaspace(+pdb) → memory(+backing).
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(c.0), handle: HObject(c.0), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(c.0), handle: HObject(1), class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(1), handle: HObject(2), class: mc::VASPACE, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::SetPageDir { client: c, vaspace: HObject(2), pdb: Pdb(0x5000) }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(1), handle: HObject(3), class: mc::MEMORY, facts: AllocFacts { mem_phys: Some(0x8000_0000), ..Default::default() } }).unwrap();

    let mut faulted = false;
    for i in 0..=MAX_LIVE_MAPPINGS as u64 {
        let ev = RmEvent::MapMemoryDma {
            client: c,
            vaspace: HObject(2),
            memory: HObject(3),
            va: GpuVa(0x1_0000_0000 + i * 0x1000),
            offset: 0,
            len: 0x1000,
        };
        if let Err(RmGraphError::CapacityExceeded(Capacity::Mappings)) = g.apply(&arch, ev) {
            faulted = true;
            break;
        }
    }
    assert!(faulted, "live-mapping table must loud-fault at the cap, not grow unbounded");
}

/// **Boundary 2.** A completion flood (a hostile guest triggering completions faster
/// than it drains them) is CAPPED at [`MAX_OUTSTANDING_COMPLETIONS`] and loud-faults —
/// it never grows one process's queue until the host OOMs.
#[test]
fn b2_completion_flood_is_capped_loud() {
    let mut q = CompletionQueue::new();
    let mut faulted_at = None;
    for i in 0..=MAX_OUTSTANDING_COMPLETIONS as u64 {
        if let Err(CompletionError::QueueFull) = q.observe(OsEventRef(i)) {
            faulted_at = Some(i);
            break;
        }
    }
    assert_eq!(
        faulted_at,
        Some(MAX_OUTSTANDING_COMPLETIONS as u64),
        "completion queue must loud-fault exactly at the cap, not OOM"
    );
}

/// **Boundary 2.** A hostile GPFIFO ring declaring enormous per-range lengths (and
/// many of them) does BOUNDED work: the parser reads at most `MAX_PUSH_RANGE_BYTES`
/// per range and a bounded total across ranges — never an arbitrary allocation from an
/// attacker-controlled length, and never a panic.
#[test]
fn b2_pushbuffer_length_flood_is_bounded() {
    let mut gpu = fresh_gpu();
    for ev in benign_b_events() {
        gpu.apply(ev).expect("B applies");
    }
    let pid = *gpu.by_pdb.get(&B_PDB).unwrap();
    let cid = *gpu.procs[&pid].chan_ids.values().next().unwrap();
    let mut vmm = MockVmm::new();

    // 64 GPFIFO entries, each declaring u64::MAX bytes at a wild GPA.
    let mut ring = Vec::new();
    for k in 0..64u64 {
        ring.extend_from_slice(&(0x9000_0000u64 + k * 0x1_0000).to_le_bytes());
        ring.extend_from_slice(&u64::MAX.to_le_bytes());
    }
    // Bounded work, no panic, no OOM: returns a normal (empty-ish) outcome.
    let out = parse_pushbuffer(&mut gpu, &mut vmm, pid, cid, &ring).expect("bounded parse");
    assert!(out.sem_releases.is_empty(), "empty guest RAM decodes to nothing actionable");
}

// =================================================================================
// BOUNDARY 1 — MISS=FAULT UNDER ADVERSARIAL INPUT
// =================================================================================

/// **Boundary 1.** A hostile resolve to an unbacked / out-of-range VA is a LOUD
/// fault, never a silent wrong-resolve. And a resolve in a PDB with no `Vas` is a
/// loud routing miss — never a fall-through to some other VAS.
#[test]
fn b4_miss_is_fault_never_silent_wrong_resolve() {
    let mut gpu = fresh_gpu();
    for ev in benign_b_events() {
        gpu.apply(ev).expect("B applies");
    }
    // In-range: resolves to B's backing.
    assert_eq!(resolve(&gpu, B_PDB, B_MAP_VA).map(|(b, _)| b.phys), Ok(B_MEM_PHYS));
    // Just past the mapping: LOUD miss, not a wrap-around into the next range.
    assert!(matches!(
        resolve(&gpu, B_PDB, GpuVa(B_MAP_VA.0 + B_MAP_LEN)),
        Err(FwdFault::Address(_))
    ));
    // A wild VA: LOUD miss.
    assert!(matches!(resolve(&gpu, B_PDB, GpuVa(0xdead_beef_0000)), Err(FwdFault::Address(_))));
    // An unknown PDB: LOUD routing miss (no VAS to fall through to).
    assert!(matches!(resolve(&gpu, Pdb(0xF00D), B_MAP_VA), Err(FwdFault::UnknownPdb(_))));
}

/// **Boundary 1 (the #14 cross-context leak, made impossible).** Two processes map
/// the IDENTICAL guest VA to DIFFERENT backings in DIFFERENT PDBs. Each resolves to
/// ITS OWN backing — a resolve in one PDB can NEVER return the other's phys.
#[test]
fn b4_identical_va_distinct_pdb_never_cross_leaks() {
    let mut gpu = fresh_gpu();
    // Process B.
    for ev in benign_b_events() {
        gpu.apply(ev).expect("B applies");
    }
    // Process A: identical VA, distinct PDB + distinct backing.
    const A_CLIENT: HClient = HClient(0xA000);
    const A_PDB: Pdb = Pdb(0x000A_0000);
    const A_MEM: HObject = HObject(0x5c00_0100); // SAME handle value as B_MEM
    const A_MEM_PHYS: u64 = 0x1_0000_0000; // distinct backing
    let ha = identical_handles(0x60, 0x61);
    let mut s = Scenario::new();
    s.compute_process(A_CLIENT, A_PDB, ha);
    s.memory(A_CLIENT, ha.device, A_MEM, A_MEM_PHYS);
    s.map(A_CLIENT, ha.vaspace, A_MEM, B_MAP_VA, B_MAP_LEN); // IDENTICAL VA
    for ev in s.events {
        gpu.apply(ev).expect("A applies");
    }

    // Each PDB resolves the identical VA to its OWN backing — never crossed.
    assert_eq!(resolve(&gpu, B_PDB, B_MAP_VA).map(|(b, _)| b.phys), Ok(B_MEM_PHYS));
    assert_eq!(resolve(&gpu, A_PDB, B_MAP_VA).map(|(b, _)| b.phys), Ok(A_MEM_PHYS));
    assert_ne!(B_MEM_PHYS, A_MEM_PHYS, "the two backings are genuinely distinct");
}

// =================================================================================
// BOUNDARY 1 — HANDLE / NAMESPACE CONFUSION (the M2 confused-deputy class)
// =================================================================================

/// **Boundary 1.** A channel naming a `hVASpace` handle that resolves to a NON-VASpace
/// object (here a MEMORY object) in its OWN namespace must NOT be silently bound to
/// that object's (or anyone's) PDB — it is treated as having no declared VAS (a loud
/// MISS at use), never a confused-deputy binding. (The exact class the RmGraph fuzz
/// A1 caught; asserted here deterministically.)
#[test]
fn b5_channel_naming_non_vaspace_handle_does_not_bind() {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let c = HClient(0xCC);
    // client → device → memory(+backing, given a PDB-shaped SetPageDir to bait a bind).
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(c.0), handle: HObject(c.0), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(c.0), handle: HObject(1), class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
    let fake_vas = HObject(2);
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(1), handle: fake_vas, class: mc::MEMORY, facts: AllocFacts { mem_phys: Some(0x8000_0000), ..Default::default() } }).unwrap();
    // Bait: attach a PDB to the MEMORY handle (a hostile SetPageDir on a non-VASpace).
    g.apply(&arch, RmEvent::SetPageDir { client: c, vaspace: fake_vas, pdb: Pdb(0x9999) }).unwrap();
    // A channel that names the memory handle as its VASpace.
    g.apply(&arch, RmEvent::Alloc {
        client: c,
        parent: HObject(1),
        handle: HObject(3),
        class: mc::CHANNEL_GR,
        facts: AllocFacts { h_vaspace: Some(fake_vas), userd_flags: MockArch::userd_flags_for(VChid(0x7)), ..Default::default() },
    }).unwrap();

    let bounds = project(&g, &arch).expect("projects");
    // The bait PDB does NOT route (a MEMORY object is not a VASpace).
    assert!(!bounds.by_pdb.contains_key(&Pdb(0x9999)), "a non-VASpace's bait PDB must not route");
    // The channel resolved to NO VAS (loud-miss at use), never the memory's PDB.
    let proc = &bounds.procs[0];
    let chan = proc.channels.values().next().expect("the channel");
    assert_eq!(chan.vas_pdb, None, "channel must not be confused-deputy bound to a non-VASpace");
}

/// **Boundary 1.** A channel in client A naming an `hVASpace` whose handle VALUE
/// exists only in ANOTHER client B must resolve within A's OWN namespace (where it is
/// absent) → no VAS — never reaching across to bind B's page directory.
#[test]
fn b5_channel_cannot_bind_another_clients_vaspace_handle() {
    let mut gpu = fresh_gpu();
    // Process B owns a real VASpace at handle value `h.vaspace` under B_PDB.
    let hb = identical_handles(0x40, 0x41);
    let mut sb = Scenario::new();
    sb.compute_process(B_CLIENT, B_PDB, hb);
    for ev in sb.events {
        gpu.apply(ev).unwrap();
    }
    // Process A names the SAME handle value as its channel's hVASpace — but in A's
    // namespace that handle is absent.
    const A_CLIENT: HClient = HClient(0xA000);
    let a_root = HObject(0xA1);
    let a_dev = HObject(0xA2);
    gpu.apply(RmEvent::Alloc { client: A_CLIENT, parent: a_root, handle: a_root, class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
    gpu.apply(RmEvent::Alloc { client: A_CLIENT, parent: a_root, handle: a_dev, class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
    gpu.apply(RmEvent::Alloc {
        client: A_CLIENT,
        parent: a_dev,
        handle: HObject(0xA3),
        class: mc::CHANNEL_GR,
        facts: AllocFacts { h_vaspace: Some(hb.vaspace), userd_flags: MockArch::userd_flags_for(VChid(0x333)), ..Default::default() },
    }).unwrap();

    let bounds = project(&gpu.rmgraph, gpu.arch.as_ref()).expect("projects");
    // A's channel resolved to NO VAS — it never reached across into B's VASpace/PDB.
    let a_proc = boundary_of(&bounds, A_CLIENT).expect("A exists");
    let chan = a_proc.channels.values().next().expect("A's channel");
    assert_eq!(chan.vas_pdb, None, "A's channel must not cross-bind B's page directory");
    // And B still solely owns B_PDB.
    assert_eq!(bounds.by_pdb.get(&B_PDB).map(|(_, k)| k.client), Some(B_CLIENT));
}

/// **Boundary 1.** A `Dup` naming a source that is NEVER allocated is INERT, never a
/// silent cross-object binding: it parks (its source may still arrive — a `Dup` before
/// its `Alloc` is a legal ordering, decision #4, and is INDISTINGUISHABLE at apply time
/// from a source that never comes), contributes no grouping and no alias, and leaves the
/// bystander untouched. The containment property is what matters — not the *mechanism*:
/// making it a hard fault instead would reject the order-tolerant `Dup`-before-`Alloc`
/// case and break whole-core determinism (see `tests/determinism.rs`). A `Free` of a
/// never-seen handle IS a loud `FreeUnknown` (a free is lifecycle-ordered, not parkable).
#[test]
fn b5_dangling_dup_is_inert_and_unknown_free_is_loud() {
    let mut gpu = fresh_gpu();
    for ev in benign_b_events() {
        gpu.apply(ev).unwrap();
    }
    // Dup with a source that will never exist → parked, inert. Accepted (it may yet
    // resolve), but it binds NOTHING and groups NOTHING until/unless its source arrives.
    let inert_dup = gpu.apply(RmEvent::Dup {
        src: NodeKey::new(HClient(0xDEAD), HObject(0xDEAD)),
        dst: NodeKey::new(HClient(0xBEEF), HObject(0xBEEF)),
    });
    assert!(inert_dup.is_ok(), "an unresolved dup parks (order tolerance), got {inert_dup:?}");
    // No cross-object binding: the dst alias resolves to nothing (no silent reach).
    assert!(
        gpu.rmgraph.origin_of(NodeKey::new(HClient(0xBEEF), HObject(0xBEEF))).is_none(),
        "a parked dup must not bind its dst to any object"
    );
    // No phantom proc: the never-allocated clients group into nothing.
    let bounds = project(&gpu.rmgraph, gpu.arch.as_ref()).expect("projects");
    assert!(
        bounds.procs.iter().all(|p| {
            !p.clients.contains(&HClient(0xBEEF)) && !p.clients.contains(&HClient(0xDEAD))
        }),
        "a dangling dup must not conjure a resource-less phantom proc"
    );
    // B is undisturbed.
    assert!(resolve(&gpu, B_PDB, B_MAP_VA).is_ok(), "B undisturbed by a dangling dup");

    // Free of a never-seen handle → loud FreeUnknown.
    let bad_free = gpu.apply(RmEvent::Free { client: HClient(0x1234), handle: HObject(0x5678) });
    assert!(
        matches!(bad_free, Err(GpuError::Graph(RmGraphError::FreeUnknown(_)))),
        "unknown free must be loud, got {bad_free:?}"
    );
}

// =================================================================================
// BOUNDARY 1 / 2 — GRACEFUL RESOURCE EXHAUSTION
// =================================================================================

/// **Boundary 2.** Filling the GPA window to capacity is a LOUD, bounded fault: new
/// processes are refused with `GpuError::Gpa`, existing processes keep working, and
/// nothing panics or corrupts. (A hostile process-spawn flood cannot wedge the device
/// or reach another process's arena.)
#[test]
fn b6_gpa_window_exhaustion_is_graceful() {
    // A tiny window: one arena for the system proc + room for just a couple more.
    let arch = Box::new(MockArch::new());
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x4_0000_0000, 0x1_0000_0000); // 3 arenas total
    let mut gpu = Gpu::new(arch, Box::new(factory), gpa).expect("realizes (system takes 1)");

    // Spawn processes until the window exhausts.
    let mut spawned: Vec<HClient> = Vec::new();
    let mut exhausted = false;
    for i in 0..16u32 {
        let c = HClient(0xE000 + i);
        match gpu.apply(RmEvent::Alloc { client: c, parent: HObject(c.0), handle: HObject(c.0), class: mc::CLIENT, facts: AllocFacts::default() }) {
            Ok(()) => spawned.push(c),
            Err(GpuError::Gpa(_)) => {
                exhausted = true;
                break;
            }
            Err(e) => panic!("window exhaustion must be a loud GpaError, got {e:?}"),
        }
    }
    assert!(exhausted, "the window must exhaust loudly, not grow forever");
    assert!(!spawned.is_empty(), "at least one process fit before exhaustion");

    // Every process that DID fit is intact and its arena is disjoint from the rest —
    // exhaustion did not corrupt or cross-wire the survivors.
    let ranges: Vec<_> = gpu.procs.values().map(|p| p.arena.range.clone()).collect();
    for i in 0..ranges.len() {
        for j in (i + 1)..ranges.len() {
            assert!(
                ranges[i].end <= ranges[j].start || ranges[j].end <= ranges[i].start,
                "survivors' arenas overlap after exhaustion"
            );
        }
    }
}

/// **Boundary 2.** After a handle flood loud-faults at the cap, the graph is still
/// consistent and USABLE — the pre-flood objects are untouched and a `Free` still
/// works (exhaustion is graceful, not a corrupt/wedged state).
#[test]
fn b6_graph_usable_after_capacity_refusal() {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    // A real object that must survive a subsequent flood refusal.
    let keep = HClient(0x1);
    g.apply(&arch, RmEvent::Alloc { client: keep, parent: HObject(keep.0), handle: HObject(keep.0), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();

    // A parked-dup flood to the cap → loud refusal.
    let mut refused = false;
    for i in 0..=MAX_PARKED as u64 {
        let dst = NodeKey::new(HClient(0x7000), HObject(i as u32));
        let src = NodeKey::new(HClient(0xDEAD), HObject(0xFFFF_FFFF));
        if g.apply(&arch, RmEvent::Dup { src, dst }).is_err() {
            refused = true;
            break;
        }
    }
    assert!(refused, "flood refused at the cap");

    // The kept object survives, resolves, and can be freed cleanly.
    assert!(g.node(NodeKey::new(keep, HObject(keep.0))).is_some(), "pre-flood object intact");
    g.apply(&arch, RmEvent::Free { client: keep, handle: HObject(keep.0) }).expect("free still works");
    assert!(g.node(NodeKey::new(keep, HObject(keep.0))).is_none(), "freed cleanly after a flood");
}
