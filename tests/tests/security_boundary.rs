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
//! The pure core has NO host memory, NO raw pointers, and no `unsafe_code` at all — the
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

use kayfabe_arch::ClientKind;
use kayfabe_arch::ids::GpuId;
use kayfabe_arch::ids::{ClassId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_completion::{
    CompletionError, CompletionQueue, MAX_OUTSTANDING_COMPLETIONS, OsEventRef,
};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, GpuError};
use kayfabe_core::project::{Boundaries, NO_CONDEMNED, ProcBoundary, project};
use kayfabe_core::rmgraph::{
    AllocFacts, Capacity, MAX_LIVE_HANDLES, MAX_LIVE_MAPPINGS, MAX_PARKED, NodeKey, RmEvent,
    RmGraph, RmGraphError,
};
use kayfabe_fwd::{FwdFault, handle_doorbell, parse_pushbuffer, resolve};
use kayfabe_mocks::{MockArch, MockIsolateFactory, MockVmm, mock_classes as mc};
use kayfabe_tests::{Guarded, Scenario, identical_handles};
use kayfabe_vmm::Vmm;
use proptest::prelude::*;

// =================================================================================
// Shared fixtures
// =================================================================================

fn fresh_gpu() -> Guarded<Gpu> {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    // A generous window so exhaustion is a deliberate act, not an accident.
    let gpa = GpaSpace::new(0x1_0000_0000..0x1000_0000_0000, 0x1_0000_0000);
    Guarded::new(
        "security_boundary::fresh_gpu",
        Gpu::new(arch, Box::new(factory), gpa).expect("device realizes"),
        rec,
    )
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
    b.procs
        .iter()
        .find(|p| p.clients.contains(&client))
        .cloned()
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
    let a_va = || {
        prop_oneof![
            Just(B_MAP_VA),
            (0u32..4).prop_map(|n| GpuVa(0x2_0020_0000 + u64::from(n) * 0x1000))
        ]
    };
    prop_oneof![
        (
            a_client(),
            a_handle(),
            a_handle(),
            any_class(),
            0u32..0x20000
        )
            .prop_map(|(client, parent, handle, class, flags)| RmEvent::Alloc {
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
                    device_instance: None,
                    // ★ §12.27 — A gets to try ALL THREE client declarations, including
                    // claiming **kernel** privilege (which only a compromised guest
                    // *kernel* could really do — see the access-model split) and
                    // declaring nothing at all (the `UndeclaredClientKind` refusal). B's
                    // isolation must hold against every one of them.
                    client_kind: match flags & 0x30 {
                        0x00 => Some(ClientKind::User { pid: client.0 }),
                        0x10 => Some(ClientKind::Kernel),
                        _ => None,
                    },
                },
            }),
        (a_client(), a_handle(), a_handle(), a_va()).prop_map(|(client, vaspace, memory, va)| {
            RmEvent::MapMemoryDma {
                client,
                vaspace,
                memory,
                va,
                offset: 0,
                len: 0x10000,
            }
        }),
        (a_client(), a_handle(), a_va()).prop_map(|(client, vaspace, va)| RmEvent::Unmap {
            client,
            vaspace,
            va
        }),
        (a_client(), a_handle(), a_client(), a_handle()).prop_map(|(sc, sh, dc, dh)| {
            RmEvent::Dup {
                src: NodeKey::new(sc, sh),
                dst: NodeKey::new(dc, dh),
            }
        }),
        (a_client(), a_handle(), a_pdb()).prop_map(|(client, vaspace, pdb)| RmEvent::SetPageDir {
            client,
            vaspace,
            pdb
        }),
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
        let ref_bounds = project(&ref_gpu.spine.rmgraph, ref_gpu.spine.arch.as_ref(), &NO_CONDEMNED).expect("B projects");
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
        let bounds = project(&gpu.spine.rmgraph, gpu.spine.arch.as_ref(), &NO_CONDEMNED)
            .expect("device still projects after hostile A");
        let b_now = boundary_of(&bounds, B_CLIENT).expect("B still has a boundary");
        prop_assert_eq!(b_ref, b_now, "B's boundary changed under A's hostility");

        // B's address plane is unchanged: its mapped VA resolves to ITS backing.
        let got = resolve(&gpu, GpuId::ZERO, B_PDB, B_MAP_VA).map(|(bind, _)| bind.phys);
        prop_assert_eq!(got, Ok(B_MEM_PHYS), "B's VA no longer resolves to its own backing");

        // B's arena is disjoint from every A arena (no shared GPA — #14 isolation).
        // (The arena RANGE is legitimately order-dependent — A creating procs first
        // shifts B's slot — so only DISJOINTNESS is the invariant, never the range.)
        let b_arena = gpu
            .procs
            .values()
            .find(|p| p.clients.contains(&B_CLIENT))
            .map(|p| p.arenas[&GpuId::ZERO].range.clone())
            .expect("B still has an arena");
        for p in gpu.procs.values() {
            if p.clients.contains(&B_CLIENT) {
                continue;
            }
            // Arenas materialize lazily per (proc, GPU); a hostile A proc that touched
            // no target has none. Check every arena it DID materialize.
            for a in p.arenas.values() {
            prop_assert!(
                a.range.end <= b_arena.start || b_arena.end <= a.range.start,
                "an A proc's arena overlaps B's — cross-process GPA collision"
            );
            }
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
        facts: kayfabe_tests::user_client(c),
    };
    gpu.apply(root(a)).unwrap();
    gpu.apply(RmEvent::Alloc {
        client: a,
        parent: HObject(a.0),
        handle: HObject(1),
        class: mc::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    })
    .unwrap();
    gpu.apply(RmEvent::Alloc {
        client: a,
        parent: HObject(1),
        handle: HObject(2),
        class: mc::VASPACE,
        facts: AllocFacts::default(),
    })
    .unwrap();
    gpu.apply(RmEvent::Alloc {
        client: a,
        parent: HObject(1),
        handle: HObject(3),
        class: mc::VASPACE,
        facts: AllocFacts::default(),
    })
    .unwrap();
    gpu.apply(RmEvent::SetPageDir {
        client: a,
        vaspace: HObject(2),
        pdb: Pdb(0xBAD),
    })
    .unwrap();

    // The colliding second bind is a LOUD, contained refusal…
    let collide = gpu.apply(RmEvent::SetPageDir {
        client: a,
        vaspace: HObject(3),
        pdb: Pdb(0xBAD),
    });
    assert!(
        matches!(collide, Err(GpuError::Projection(_))),
        "PDB collision must be a loud projection fault, got {collide:?}"
    );

    // …and the device is NOT wedged: a wholly-separate process B proceeds normally.
    for ev in benign_b_events() {
        gpu.apply(ev)
            .expect("B is unaffected by A's projection collision");
    }
    assert!(
        resolve(&gpu, GpuId::ZERO, B_PDB, B_MAP_VA).is_ok(),
        "B fully functional after A's collision"
    );
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
    gpu.apply(RmEvent::Alloc {
        client: a,
        parent: HObject(a.0),
        handle: HObject(a.0),
        class: mc::CLIENT,
        facts: kayfabe_tests::user_client(a),
    })
    .unwrap();
    gpu.apply(RmEvent::Alloc {
        client: a,
        parent: HObject(a.0),
        handle: HObject(1),
        class: mc::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    })
    .unwrap();
    gpu.apply(RmEvent::Alloc {
        client: a,
        parent: HObject(1),
        handle: HObject(2),
        class: mc::VASPACE,
        facts: AllocFacts::default(),
    })
    .unwrap();
    gpu.apply(RmEvent::SetPageDir {
        client: a,
        vaspace: HObject(2),
        pdb: Pdb(0xA11),
    })
    .unwrap();
    gpu.apply(RmEvent::Alloc {
        client: a,
        parent: HObject(1),
        handle: HObject(0x10),
        class: mc::CHANNEL_GR,
        facts: AllocFacts {
            h_vaspace: Some(HObject(2)),
            userd_flags: flags,
            ..Default::default()
        },
    })
    .unwrap();

    // The SECOND channel claiming the same vChid is a LOUD, contained refusal.
    let collide = gpu.apply(RmEvent::Alloc {
        client: a,
        parent: HObject(1),
        handle: HObject(0x11),
        class: mc::CHANNEL_CE,
        facts: AllocFacts {
            h_vaspace: Some(HObject(2)),
            userd_flags: flags,
            ..Default::default()
        },
    });
    assert!(
        matches!(collide, Err(GpuError::Projection(_))),
        "two channels decoding to one vChid must be a loud projection fault, got {collide:?}"
    );

    // Contained: a wholly-separate process B proceeds normally after A's collision.
    for ev in benign_b_events() {
        gpu.apply(ev).expect("B unaffected by A's vChid collision");
    }
    assert!(
        resolve(&gpu, GpuId::ZERO, B_PDB, B_MAP_VA).is_ok(),
        "B fully functional after A's collision"
    );
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
    gpu.apply(RmEvent::Alloc {
        client: A_CLIENT,
        parent: a_root,
        handle: a_root,
        class: mc::CLIENT,
        facts: kayfabe_tests::user_client(A_CLIENT),
    })
    .unwrap();
    gpu.apply(RmEvent::Alloc {
        client: A_CLIENT,
        parent: a_root,
        handle: a_dev,
        class: mc::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    })
    .unwrap();
    gpu.apply(RmEvent::Alloc {
        client: A_CLIENT,
        parent: a_dev,
        handle: a_vas,
        class: mc::VASPACE,
        facts: AllocFacts::default(),
    })
    .unwrap();
    let squat = gpu.apply(RmEvent::SetPageDir {
        client: A_CLIENT,
        vaspace: a_vas,
        pdb: B_PDB,
    });

    // The squat is refused (B declared B_PDB first) — loud + contained.
    assert!(
        matches!(squat, Err(GpuError::Projection(_))),
        "PDB squat must be a loud fault, got {squat:?}"
    );
    // B keeps its PDB and its mapping — the victim is not corrupted.
    assert_eq!(
        resolve(&gpu, GpuId::ZERO, B_PDB, B_MAP_VA).map(|(b, _)| b.phys),
        Ok(B_MEM_PHYS)
    );
    assert_eq!(
        gpu.spine
            .by_pdb
            .get(&(GpuId::ZERO, B_PDB))
            .and_then(|pid| gpu.procs.get(pid))
            .map(|p| p.clients.contains(&B_CLIENT)),
        Some(true)
    );
    // The INNOCENT third process C is entirely unaffected — the blast radius never
    // reaches beyond the colliding pair.
    assert!(
        gpu.spine.by_pdb.contains_key(&(GpuId::ZERO, C_PDB)),
        "innocent C still routes"
    );
    // And the device as a whole is still consistent (no wedge, no corruption).
    assert!(project(&gpu.spine.rmgraph, gpu.spine.arch.as_ref(), &NO_CONDEMNED).is_ok());
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
        let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, B_PDB)).expect("B routes");
        let cid = *gpu.procs[&pid].chan_ids.values().next().expect("B has a channel");
        let mut vmm = MockVmm::new();
        // Arbitrary method bytes live in guest RAM; the ring points ranges at them.
        vmm.gpa_write(0x5000_0000, &blob).unwrap();

        for t in tokens {
            // Arbitrary doorbell token → routes to a channel or a loud fault; never panics.
            let _ = handle_doorbell(&mut gpu, GpuId::ZERO, t, &[]);
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
            let _ = resolve(&gpu, GpuId::ZERO, B_PDB, GpuVa(v));
        }
        // The device is still consistent after all of it.
        prop_assert!(project(&gpu.spine.rmgraph, gpu.spine.arch.as_ref(), &NO_CONDEMNED).is_ok());
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
            facts: kayfabe_tests::user_client(c),
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
    assert!(
        faulted,
        "parked-dup table must loud-fault at the cap, not grow unbounded"
    );
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
    assert!(
        faulted,
        "parked-map table must loud-fault at the cap, not grow unbounded"
    );
}

/// **Boundary 2.** A live-mapping flood (one VAS + one memory, mapped at unbounded
/// distinct VAs) is CAPPED at [`MAX_LIVE_MAPPINGS`] and loud-faults.
#[test]
fn b2_mapping_flood_is_capped_loud() {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let c = HClient(0xC002);
    // One client → device → vaspace(+pdb) → memory(+backing).
    g.apply(
        &arch,
        RmEvent::Alloc {
            client: c,
            parent: HObject(c.0),
            handle: HObject(c.0),
            class: mc::CLIENT,
            facts: kayfabe_tests::user_client(c),
        },
    )
    .unwrap();
    g.apply(
        &arch,
        RmEvent::Alloc {
            client: c,
            parent: HObject(c.0),
            handle: HObject(1),
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
    )
    .unwrap();
    g.apply(
        &arch,
        RmEvent::Alloc {
            client: c,
            parent: HObject(1),
            handle: HObject(2),
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        },
    )
    .unwrap();
    g.apply(
        &arch,
        RmEvent::SetPageDir {
            client: c,
            vaspace: HObject(2),
            pdb: Pdb(0x5000),
        },
    )
    .unwrap();
    g.apply(
        &arch,
        RmEvent::Alloc {
            client: c,
            parent: HObject(1),
            handle: HObject(3),
            class: mc::MEMORY,
            facts: AllocFacts {
                mem_phys: Some(0x8000_0000),
                ..Default::default()
            },
        },
    )
    .unwrap();

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
    assert!(
        faulted,
        "live-mapping table must loud-fault at the cap, not grow unbounded"
    );
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
    let pid = *gpu.spine.by_pdb.get(&(GpuId::ZERO, B_PDB)).unwrap();
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
    assert!(
        out.sem_releases.is_empty(),
        "empty guest RAM decodes to nothing actionable"
    );
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
    assert_eq!(
        resolve(&gpu, GpuId::ZERO, B_PDB, B_MAP_VA).map(|(b, _)| b.phys),
        Ok(B_MEM_PHYS)
    );
    // Just past the mapping: LOUD miss, not a wrap-around into the next range.
    assert!(matches!(
        resolve(&gpu, GpuId::ZERO, B_PDB, GpuVa(B_MAP_VA.0 + B_MAP_LEN)),
        Err(FwdFault::Address(_))
    ));
    // A wild VA: LOUD miss.
    assert!(matches!(
        resolve(&gpu, GpuId::ZERO, B_PDB, GpuVa(0xdead_beef_0000)),
        Err(FwdFault::Address(_))
    ));
    // An unknown PDB: LOUD routing miss (no VAS to fall through to).
    assert!(matches!(
        resolve(&gpu, GpuId::ZERO, Pdb(0xF00D), B_MAP_VA),
        Err(FwdFault::UnknownPdb { .. })
    ));
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
    assert_eq!(
        resolve(&gpu, GpuId::ZERO, B_PDB, B_MAP_VA).map(|(b, _)| b.phys),
        Ok(B_MEM_PHYS)
    );
    assert_eq!(
        resolve(&gpu, GpuId::ZERO, A_PDB, B_MAP_VA).map(|(b, _)| b.phys),
        Ok(A_MEM_PHYS)
    );
    assert_ne!(
        B_MEM_PHYS, A_MEM_PHYS,
        "the two backings are genuinely distinct"
    );
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
    g.apply(
        &arch,
        RmEvent::Alloc {
            client: c,
            parent: HObject(c.0),
            handle: HObject(c.0),
            class: mc::CLIENT,
            facts: kayfabe_tests::user_client(c),
        },
    )
    .unwrap();
    g.apply(
        &arch,
        RmEvent::Alloc {
            client: c,
            parent: HObject(c.0),
            handle: HObject(1),
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
    )
    .unwrap();
    let fake_vas = HObject(2);
    g.apply(
        &arch,
        RmEvent::Alloc {
            client: c,
            parent: HObject(1),
            handle: fake_vas,
            class: mc::MEMORY,
            facts: AllocFacts {
                mem_phys: Some(0x8000_0000),
                ..Default::default()
            },
        },
    )
    .unwrap();
    // Bait: attach a PDB to the MEMORY handle (a hostile SetPageDir on a non-VASpace).
    g.apply(
        &arch,
        RmEvent::SetPageDir {
            client: c,
            vaspace: fake_vas,
            pdb: Pdb(0x9999),
        },
    )
    .unwrap();
    // A channel that names the memory handle as its VASpace.
    g.apply(
        &arch,
        RmEvent::Alloc {
            client: c,
            parent: HObject(1),
            handle: HObject(3),
            class: mc::CHANNEL_GR,
            facts: AllocFacts {
                h_vaspace: Some(fake_vas),
                userd_flags: MockArch::userd_flags_for(VChid(0x7)),
                ..Default::default()
            },
        },
    )
    .unwrap();

    let bounds = project(&g, &arch, &NO_CONDEMNED).expect("projects");
    // The bait PDB does NOT route (a MEMORY object is not a VASpace).
    assert!(
        !bounds.by_pdb.contains_key(&(GpuId::ZERO, Pdb(0x9999))),
        "a non-VASpace's bait PDB must not route"
    );
    // The channel resolved to NO VAS (loud-miss at use), never the memory's PDB.
    let proc = &bounds.procs[0];
    let chan = proc.channels.values().next().expect("the channel");
    assert_eq!(
        chan.vas_pdb, None,
        "channel must not be confused-deputy bound to a non-VASpace"
    );
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
    gpu.apply(RmEvent::Alloc {
        client: A_CLIENT,
        parent: a_root,
        handle: a_root,
        class: mc::CLIENT,
        facts: kayfabe_tests::user_client(A_CLIENT),
    })
    .unwrap();
    gpu.apply(RmEvent::Alloc {
        client: A_CLIENT,
        parent: a_root,
        handle: a_dev,
        class: mc::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    })
    .unwrap();
    gpu.apply(RmEvent::Alloc {
        client: A_CLIENT,
        parent: a_dev,
        handle: HObject(0xA3),
        class: mc::CHANNEL_GR,
        facts: AllocFacts {
            h_vaspace: Some(hb.vaspace),
            userd_flags: MockArch::userd_flags_for(VChid(0x333)),
            ..Default::default()
        },
    })
    .unwrap();

    let bounds =
        project(&gpu.spine.rmgraph, gpu.spine.arch.as_ref(), &NO_CONDEMNED).expect("projects");
    // A's channel resolved to NO VAS — it never reached across into B's VASpace/PDB.
    let a_proc = boundary_of(&bounds, A_CLIENT).expect("A exists");
    let chan = a_proc.channels.values().next().expect("A's channel");
    assert_eq!(
        chan.vas_pdb, None,
        "A's channel must not cross-bind B's page directory"
    );
    // And B still solely owns B_PDB.
    assert_eq!(
        bounds
            .by_pdb
            .get(&(GpuId::ZERO, B_PDB))
            .map(|(_, k)| k.client),
        Some(B_CLIENT)
    );
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
    assert!(
        inert_dup.is_ok(),
        "an unresolved dup parks (order tolerance), got {inert_dup:?}"
    );
    // No cross-object binding: the dst alias resolves to nothing (no silent reach).
    assert!(
        gpu.spine
            .rmgraph
            .origin_of(NodeKey::new(HClient(0xBEEF), HObject(0xBEEF)))
            .is_none(),
        "a parked dup must not bind its dst to any object"
    );
    // No phantom proc: the never-allocated clients group into nothing.
    let bounds =
        project(&gpu.spine.rmgraph, gpu.spine.arch.as_ref(), &NO_CONDEMNED).expect("projects");
    assert!(
        bounds.procs.iter().all(|p| {
            !p.clients.contains(&HClient(0xBEEF)) && !p.clients.contains(&HClient(0xDEAD))
        }),
        "a dangling dup must not conjure a resource-less phantom proc"
    );
    // B is undisturbed.
    assert!(
        resolve(&gpu, GpuId::ZERO, B_PDB, B_MAP_VA).is_ok(),
        "B undisturbed by a dangling dup"
    );

    // Free of a never-seen handle → loud FreeUnknown.
    let bad_free = gpu.apply(RmEvent::Free {
        client: HClient(0x1234),
        handle: HObject(0x5678),
    });
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

    // Spawn ROUTABLE processes (each carves a per-(proc, GpuId::ZERO) arena on its
    // first materialized target) until the window exhausts. Arenas are lazy per
    // (proc, GPU), so a proc consumes a slot once it declares a routable VAS.
    let mut spawned: Vec<HClient> = Vec::new();
    let mut exhausted = false;
    'outer: for i in 0..16u32 {
        let c = HClient(0xE000 + i);
        let root = HObject(c.0);
        let dev = HObject(0x100);
        let vas = HObject(0x110);
        let steps = [
            RmEvent::Alloc {
                client: c,
                parent: root,
                handle: root,
                class: mc::CLIENT,
                facts: kayfabe_tests::user_client(c),
            },
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
                handle: vas,
                class: mc::VASPACE,
                facts: AllocFacts::default(),
            },
            RmEvent::SetPageDir {
                client: c,
                vaspace: vas,
                pdb: Pdb(0x1000 * u64::from(i + 1)),
            },
        ];
        for ev in steps {
            match gpu.apply(ev) {
                Ok(()) => {}
                Err(GpuError::Gpa(_)) => {
                    exhausted = true;
                    break 'outer;
                }
                Err(e) => panic!("window exhaustion must be a loud GpaError, got {e:?}"),
            }
        }
        spawned.push(c);
    }
    assert!(
        exhausted,
        "the window must exhaust loudly, not grow forever"
    );
    assert!(
        !spawned.is_empty(),
        "at least one process fit before exhaustion"
    );

    // Every process that DID fit is intact and its arena is disjoint from the rest —
    // exhaustion did not corrupt or cross-wire the survivors.
    let ranges: Vec<_> = gpu
        .procs
        .values()
        .flat_map(|p| p.arenas.values().map(|a| a.range.clone()))
        .collect();
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
    g.apply(
        &arch,
        RmEvent::Alloc {
            client: keep,
            parent: HObject(keep.0),
            handle: HObject(keep.0),
            class: mc::CLIENT,
            facts: kayfabe_tests::user_client(keep),
        },
    )
    .unwrap();

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
    assert!(
        g.node(NodeKey::new(keep, HObject(keep.0))).is_some(),
        "pre-flood object intact"
    );
    g.apply(
        &arch,
        RmEvent::Free {
            client: keep,
            handle: HObject(keep.0),
        },
    )
    .expect("free still works");
    assert!(
        g.node(NodeKey::new(keep, HObject(keep.0))).is_none(),
        "freed cleanly after a flood"
    );
}

// =================================================================================
// ★ APPLY IS ATOMIC — a refused event disturbs no OTHER process (boundary-1, #14)
//
// `Spine::apply` snapshots the `RmGraph` and restores it on any derivation fault. That
// covered the graph and nothing else: `refresh` also retires and REMOVES procs,
// deregisters completion sources, pushes to the retired list, mints `ProcId`s, mints
// GPU targets and carves GPA arenas. So a fault raised after an earlier victim had
// already been retired left that victim dead, and the rollback's re-derivation minted
// it afresh — new `ProcId`, newly spawned isolate, newly carved arena. One process's
// malformed event visibly destroying another process's state is exactly what
// boundary-1 exists to forbid, and the function's own doc asserted the opposite
// ("no other `Proc`'s state is disturbed").
//
// The fix (`l1_concurrency.md` §12.18) hoists every refusal into `Spine::plan_refresh`,
// which runs before a single proc is touched. These two tests are the executable
// statement of that: one for each pass a fault used to be able to land in.
// =================================================================================

/// A bystander's complete observable identity — everything a hostile event must not be
/// able to change. `ProcId` and the isolate's `IsolateId` catch a respawn; the arena
/// ranges catch a re-carve; the client set catches a silent re-grouping.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcFingerprint {
    id: kayfabe_core::ProcId,
    arenas: Vec<(GpuId, std::ops::Range<u64>)>,
    isolates: Vec<(GpuId, kayfabe_isolate::IsolateId)>,
    clients: Vec<HClient>,
    vases: Vec<(GpuId, Pdb)>,
}

fn proc_fingerprint(gpu: &Gpu, pid: kayfabe_core::ProcId) -> ProcFingerprint {
    let p = gpu.procs.get(&pid).expect("bystander is still live");
    ProcFingerprint {
        id: p.id,
        arenas: p
            .arenas
            .iter()
            .map(|(g, a)| (*g, a.range.clone()))
            .collect(),
        isolates: p.isolates.iter().map(|(g, i)| (*g, i.id())).collect(),
        clients: p.clients.iter().copied().collect(),
        vases: p.vases.keys().copied().collect(),
    }
}

/// Handles for one compute process, based at `base` so several devices can live in one
/// client namespace (the client root stays the client's own handle).
fn handles_at(client: HClient, base: u32, gr: u16, ce: u16) -> kayfabe_tests::ProcessHandles {
    kayfabe_tests::ProcessHandles {
        client_root: HObject(client.0),
        device: HObject(base),
        vaspace: HObject(base + 1),
        tsg: HObject(base + 2),
        gr_channel: HObject(base + 3),
        gr_vchid: VChid(gr),
        ce_channel: HObject(base + 4),
        ce_vchid: VChid(ce),
    }
}

/// ★ **Boundary-1: a refused merge must not consume the victims it reached first.**
///
/// Three independent procs; one hostile `Alloc` resolves two parked dups at once, so a
/// single boundary matches all three. The merge absorbs them in ascending `ProcId`
/// order: the middle proc is untouched and legally absorbed (retired, removed,
/// sources deregistered), and only THEN does the third — which has published a backing
/// — earn the `LateMerge` refusal. The middle proc had nothing to do with the event.
///
/// Before §12.18 it was retired anyway and the rollback's re-derivation handed its
/// client a **fresh** `ProcId`, a **freshly spawned** isolate and a **freshly carved**
/// arena: the guest kept its handles and PDB but every host identity behind them had
/// silently changed, and its published backing was gone. That is the #14 blast radius
/// crossing a process boundary through a *refused* event.
#[test]
fn a_refused_merge_leaves_the_victim_it_reached_first_bit_identical() {
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x1000_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(MockArch::new()), Box::new(factory), gpa).expect("device realizes");

    const C1: HClient = HClient(0x10);
    const C2: HClient = HClient(0x20);
    const C3: HClient = HClient(0x30);
    const PDB1: Pdb = Pdb(0x1_0000);
    const PDB2: Pdb = Pdb(0x2_0000);
    const PDB3: Pdb = Pdb(0x3_0000);
    const GPU0: GpuId = GpuId::ZERO;

    let h1 = handles_at(C1, 0x100, 0x10, 0x11);
    let h2 = handles_at(C2, 0x200, 0x20, 0x21);
    let h3 = handles_at(C3, 0x300, 0x30, 0x31);
    let mut s = Scenario::new();
    s.compute_process(C1, PDB1, h1);
    s.compute_process(C2, PDB2, h2);
    s.compute_process(C3, PDB3, h3);
    for ev in s.events {
        gpu.apply(ev).expect("three independent procs");
    }
    let (p1, p2, p3) = (
        gpu.spine.by_pdb[&(GPU0, PDB1)],
        gpu.spine.by_pdb[&(GPU0, PDB2)],
        gpu.spine.by_pdb[&(GPU0, PDB3)],
    );
    assert!(p1 < p2 && p2 < p3, "victims are absorbed in ProcId order");

    // P3 touches its data plane, so absorbing it is the illegal late merge.
    let published = kayfabe_fwd::publish_backing(
        gpu.procs.get_mut(&p3).expect("p3"),
        GPU0,
        PDB3,
        GpuVa(0x2_0000_0000),
        0x1000,
    )
    .expect("p3 publishes");

    let before2 = proc_fingerprint(&gpu, p2);
    let before1 = proc_fingerprint(&gpu, p1);
    let retired_before = gpu.spine.retired_len();

    // Two dup edges parked on a handle C1 has not allocated yet…
    let future = HObject(0x9999_0000);
    for (dst_client, alias) in [(C2, HObject(0x7777_0002)), (C3, HObject(0x7777_0003))] {
        gpu.apply(RmEvent::Dup {
            src: NodeKey::new(C1, future),
            dst: NodeKey::new(dst_client, alias),
        })
        .expect("a parked dup is not yet a grouping edge");
    }
    // …and ONE alloc that resolves both, folding all three procs into one boundary.
    let fault = gpu
        .apply(RmEvent::Alloc {
            client: C1,
            parent: HObject(C1.0),
            handle: future,
            class: mc::MEMORY,
            facts: AllocFacts {
                mem_phys: Some(0x8_0000_0000),
                ..Default::default()
            },
        })
        .expect_err("absorbing a proc that has published is a LateMerge");
    assert_eq!(
        fault,
        GpuError::LateMerge {
            kept: p1,
            absorbed: p3
        },
        "the refusal must name the exact merge it refused"
    );

    // ★ The property: the FIRST victim is untouched. Nothing was retired…
    assert_eq!(
        gpu.spine.retired_len(),
        retired_before,
        "a refused event must retire NOBODY — the middle proc was absorbed before the \
         fault and its retirement was never rolled back"
    );
    // …and every identity behind its handles is the same object it was.
    assert_eq!(
        proc_fingerprint(&gpu, p2),
        before2,
        "the bystander's ProcId / isolate / GPA arena / clients / vases must be identical"
    );
    assert_eq!(proc_fingerprint(&gpu, p1), before1, "the keeper too");
    assert_eq!(
        gpu.spine.by_pdb.get(&(GPU0, PDB2)),
        Some(&p2),
        "the bystander still owns its own PDB route"
    );
    assert_eq!(gpu.spine.condemned_len(), 0, "nothing was condemned");
    // And the touched proc's host state survived its own refusal.
    let (binding, _) = resolve(&gpu, GPU0, PDB3, GpuVa(0x2_0000_0000)).expect("p3 still resolves");
    assert_eq!(
        binding.host.expect("still published").host_va,
        published.host_va,
        "the refused event must not disturb the backing that earned the refusal either"
    );
}

/// ★ **The same property one pass later: a GPA-window exhaustion.**
///
/// The merge itself is legal — the absorbed proc is untouched — so step 1 retires and
/// removes it, and the fault lands afterwards, when the surviving proc turns out to
/// need an arena on a target whose window is full. Before §12.18 the absorbed proc was
/// simply gone.
///
/// This also pins the plan's **undo**: the merged boundary needs arenas on two targets,
/// and the first carve succeeds before the second fails. Its range must go back to the
/// window, which the test proves by requiring the *last* free arena on that target to
/// still be available afterwards.
#[test]
fn a_refused_arena_carve_returns_every_arena_it_took_and_loses_no_proc() {
    let (factory, _rec) = MockIsolateFactory::new();
    // 3 arenas per target window, for every target (the geometry is cloned).
    let gpa = GpaSpace::new(0x1_0000_0000..0x4_0000_0000, 0x1_0000_0000);
    // ★ G9 (§12.21): realized with three physical GPUs — the entitlement this test's
    // `deviceInstance`s are checked against.
    let mut gpu = Gpu::realize(
        Box::new(MockArch::new()),
        Box::new(factory),
        gpa,
        &[GpuId::ZERO, GpuId(1), GpuId(2)],
    )
    .expect("device realizes");

    const GPU0: GpuId = GpuId::ZERO;
    const GPU1: GpuId = GpuId(1);
    const GPU2: GpuId = GpuId(2);
    const C1: HClient = HClient(0x10); // GPU0 only — the merge's keeper
    const C3: HClient = HClient(0x30); // GPU1 + GPU2 — the absorbed proc
    const C4: HClient = HClient(0x40); // GPU2 filler
    const C5: HClient = HClient(0x50); // GPU2 filler (window now FULL)
    const C6: HClient = HClient(0x60); // GPU1 filler (one arena left)
    const C7: HClient = HClient(0x70); // the prover: claims that last GPU1 arena

    let mut s = Scenario::new();
    s.compute_process_on_gpu(C1, Pdb(0x1_0000), handles_at(C1, 0x100, 0x10, 0x11), None);
    s.compute_process_on_gpu(
        C3,
        Pdb(0x3_0000),
        handles_at(C3, 0x300, 0x30, 0x31),
        Some(1),
    );
    s.compute_process_on_gpu(
        C3,
        Pdb(0x3_1000),
        handles_at(C3, 0x310, 0x32, 0x33),
        Some(2),
    );
    s.compute_process_on_gpu(
        C4,
        Pdb(0x4_0000),
        handles_at(C4, 0x400, 0x40, 0x41),
        Some(2),
    );
    s.compute_process_on_gpu(
        C5,
        Pdb(0x5_0000),
        handles_at(C5, 0x500, 0x50, 0x51),
        Some(2),
    );
    s.compute_process_on_gpu(
        C6,
        Pdb(0x6_0000),
        handles_at(C6, 0x600, 0x60, 0x61),
        Some(1),
    );
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies");
    }
    let p1 = gpu.spine.by_pdb[&(GPU0, Pdb(0x1_0000))];
    let p3 = gpu.spine.by_pdb[&(GPU1, Pdb(0x3_0000))];
    assert!(p1 < p3, "the keeper is the smaller ProcId");
    assert_eq!(
        gpu.procs[&p3].arenas.len(),
        2,
        "the absorbed proc spans GPU1 and GPU2"
    );
    let before3 = proc_fingerprint(&gpu, p3);
    let retired_before = gpu.spine.retired_len();

    // Merge C1's proc (GPU0) with C3's (GPU1 + GPU2). The keeper must carve a GPU1
    // arena (one left → succeeds) and then a GPU2 arena (window full → refused).
    let fault = gpu
        .apply(RmEvent::Dup {
            src: NodeKey::new(C1, HObject(0x100 + 1)),
            dst: NodeKey::new(C3, HObject(0x7777_0003)),
        })
        .expect_err("the merged proc cannot be given a GPU2 arena");
    assert_eq!(
        fault,
        GpuError::Gpa(kayfabe_core::gpa::GpaError::WindowExhausted),
        "exhaustion is loud and exact"
    );

    // ★ The absorbed proc was already retired when the fault landed — and must not be.
    assert_eq!(
        gpu.spine.retired_len(),
        retired_before,
        "the absorbed proc must not have been retired by a refused event"
    );
    assert_eq!(
        proc_fingerprint(&gpu, p3),
        before3,
        "the absorbed proc keeps its ProcId, isolates and both GPA arenas"
    );
    assert_eq!(gpu.spine.by_pdb.get(&(GPU1, Pdb(0x3_0000))), Some(&p3));
    assert_eq!(gpu.spine.by_pdb.get(&(GPU2, Pdb(0x3_1000))), Some(&p3));

    // ★ The undo: the GPU1 arena carved before the failing carve went BACK to the
    // window. If it had leaked, GPU1 would now be full and this proc could not exist.
    let mut s7 = Scenario::new();
    s7.compute_process_on_gpu(
        C7,
        Pdb(0x7_0000),
        handles_at(C7, 0x700, 0x70, 0x71),
        Some(1),
    );
    for ev in s7.events {
        gpu.apply(ev)
            .expect("the released GPU1 arena is available again");
    }
    let p7 = gpu.spine.by_pdb[&(GPU1, Pdb(0x7_0000))];
    assert_eq!(
        gpu.procs[&p7].arenas.len(),
        1,
        "the recovered arena went to a real proc"
    );
}

/// ★ **The third mutator `apply` wraps: `sync_rpc_mappings`.**
///
/// `refresh` is now atomic by construction (§12.18), but the RPC-map forward-populate
/// runs *after* it and mutates address tables, and it CAN fault (`UnbackedMapping`, and
/// an `Overlap` from the bind). It has no plan/undo of its own — it is restored by the
/// rollback's re-run, and this test is the executable statement of that rather than an
/// assumed one, because the claim is subtle: the re-run's stale-unbind pass is what
/// removes a binding the failed run had already installed.
///
/// The residue case is reachable in one event: two `MapMemoryDma`s park on a memory
/// handle that does not exist yet, at **overlapping** VAs; the alloc that resolves them
/// promotes both, so the sync binds the first and then faults on the second.
#[test]
fn a_refused_map_sync_restores_the_binding_it_had_already_installed() {
    let mut gpu = fresh_gpu();
    const CA: HClient = HClient(0xA0);
    const CB: HClient = HClient(0xB0);
    const PDBA: Pdb = Pdb(0xA_0000);
    const PDBB: Pdb = Pdb(0xB_0000);
    const GPU0: GpuId = GpuId::ZERO;
    const MEM0: HObject = HObject(0x6000_0000);
    const LATE: HObject = HObject(0x6000_0001);
    const VA0: GpuVa = GpuVa(0x1_0000_0000);
    const VA1: GpuVa = GpuVa(0x2_0000_0000);
    const VA2: GpuVa = GpuVa(0x2_0000_1000); // overlaps VA1's 64K range

    let ha = handles_at(CA, 0x100, 0x10, 0x11);
    let hb = handles_at(CB, 0x200, 0x20, 0x21);
    let mut s = Scenario::new();
    s.compute_process(CA, PDBA, ha);
    s.memory(CA, ha.device, MEM0, 0x8_0000_0000);
    s.map(CA, ha.vaspace, MEM0, VA0, 0x10000);
    s.compute_process(CB, PDBB, hb);
    s.memory(CB, hb.device, MEM0, 0x9_0000_0000);
    s.map(CB, hb.vaspace, MEM0, VA0, 0x10000);
    for ev in s.events {
        gpu.apply(ev).expect("two mapped procs");
    }
    let pa = gpu.spine.by_pdb[&(GPU0, PDBA)];
    let pb = gpu.spine.by_pdb[&(GPU0, PDBB)];

    let snap = |gpu: &Gpu, pid, pdb| -> Vec<(u64, u64, kayfabe_mmu::Binding)> {
        gpu.procs[&pid].vases[&(GPU0, pdb)]
            .table
            .iter()
            .map(|(va, len, b)| (va, len, *b))
            .collect()
    };
    let before_a = snap(&gpu, pa, PDBA);
    let before_b = snap(&gpu, pb, PDBB);
    assert_eq!(before_a.len(), 1, "A starts with exactly its one mapping");

    // Two maps parked on a handle CA has not allocated yet, at overlapping VAs.
    for va in [VA1, VA2] {
        gpu.apply(RmEvent::MapMemoryDma {
            client: CA,
            vaspace: ha.vaspace,
            memory: LATE,
            va,
            offset: 0,
            len: 0x10000,
        })
        .expect("a map on an unobserved memory handle parks");
    }
    // The alloc that promotes BOTH: the sync binds VA1, then refuses VA2.
    let fault = gpu
        .apply(RmEvent::Alloc {
            client: CA,
            parent: ha.device,
            handle: LATE,
            class: mc::MEMORY,
            facts: AllocFacts {
                mem_phys: Some(0x7_0000_0000),
                ..Default::default()
            },
        })
        .expect_err("two overlapping mappings cannot both bind");
    assert_eq!(
        fault,
        GpuError::Address(kayfabe_mmu::AddressFault::Overlap { pdb: PDBA, va: VA2 }),
        "the refusal names the exact overlapping range"
    );

    assert_eq!(
        snap(&gpu, pa, PDBA),
        before_a,
        "the half-installed binding must be gone — the offending proc's own table is \
         back to its last-good contents"
    );
    assert_eq!(
        snap(&gpu, pb, PDBB),
        before_b,
        "the bystander's address table is untouched"
    );
    assert_eq!(gpu.spine.retired_len(), 0);
    // And the device still works for both of them afterwards.
    resolve(&gpu, GPU0, PDBA, VA0).expect("A still resolves its own mapping");
    resolve(&gpu, GPU0, PDBB, VA0).expect("B still resolves its own mapping");
}

// =================================================================================
// ★ G9 — `deviceInstance` is guest-supplied, and it used to mint GPU targets forever
// (`l1_concurrency.md` §12.21). Boundary-2 (unbounded allocation) + the no-GPU0-guess
// doctrine.
// =================================================================================

/// ★ **An instance the device was not realized with is refused, at the alloc, exactly
/// as RM refuses it.**
///
/// `GpuId` is derived from `AllocFacts::device_instance` — a raw guest `u32` off
/// `NV0080_ALLOC_PARAMETERS` — and `ensure_target` minted a fresh `GpuTarget` (its own
/// guest-physical window + `DeliveryPlane`) on first touch, with no cap and no validation
/// against the GPUs the device actually has; `targets` is never pruned. Every neighbouring
/// guest-reachable surface has a named cap (`MAX_OUTSTANDING_COMPLETIONS`,
/// `MAX_ARMED_FENCES`, `MAX_PUSH_TOTAL_BYTES`, `MAX_LIVE_HANDLES`) — this one did not.
///
/// The cap is the **entitlement**, not `NV_MAX_DEVICES`: RM already bounds the field to
/// `< 32` in three places, so a `< 32` check here would still let a guest mint 31 windows
/// and 31 delivery planes on a single-GPU box. And it is trivially reachable — ~20 lines
/// of raw `NV_ESC_RM_ALLOC` on `/dev/nvidiactl`, no patched guest kernel; stock userspace
/// never emits one.
#[test]
fn g9_an_unentitled_device_instance_is_refused_and_mints_no_target() {
    let mut gpu = fresh_gpu(); // realized with ONE GPU
    const C: HClient = HClient(0x9000);
    let targets_before = gpu.spine.targets.len();
    assert_eq!(targets_before, 1, "a single-GPU device has one target");

    gpu.apply(kayfabe_tests::client_root(C))
        .expect("client root");
    let refused = gpu
        .apply(RmEvent::Alloc {
            client: C,
            parent: HObject(C.0),
            handle: HObject(0x100),
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(7),
                ..Default::default()
            },
        })
        .expect_err("GPU 7 does not exist on this device");
    assert_eq!(
        refused,
        GpuError::Graph(RmGraphError::InvalidDeviceInstance { instance: 7 }),
        "the refusal must name the instance, mirroring RM's NV_ERR_INVALID_CLASS"
    );
    assert_eq!(
        gpu.spine.targets.len(),
        targets_before,
        "a refused Device must mint no GpuTarget"
    );
    assert!(
        gpu.spine
            .rmgraph
            .node(NodeKey::new(C, HObject(0x100)))
            .is_none(),
        "and no node survives the refusal"
    );

    // The entitled instance still works, from the same client, right after.
    gpu.apply(RmEvent::Alloc {
        client: C,
        parent: HObject(C.0),
        handle: HObject(0x101),
        class: mc::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    })
    .expect("GPU 0 is entitled");
}

/// ★ **Boundary-2: the flood mints nothing.** 4096 distinct instances, every one refused,
/// and the device still has exactly its realized targets — no windows, no delivery planes,
/// no `targets` growth. Before the fix each of these was a fresh `GpuTarget` with its own
/// guest-physical window, kept forever.
#[test]
fn g9_a_device_instance_flood_grows_no_device_state() {
    let mut gpu = fresh_gpu();
    const C: HClient = HClient(0x9100);
    gpu.apply(kayfabe_tests::client_root(C))
        .expect("client root");

    // Tolerant on each call: the property is that the DEVICE does not grow, not that a
    // particular call returned `Err` — a test that only checked the return value could
    // never show the resource it was protecting.
    let mut outcomes = Vec::new();
    for i in 1..4096u32 {
        outcomes.push((
            i,
            gpu.apply(RmEvent::Alloc {
                client: C,
                parent: HObject(C.0),
                handle: HObject(0x1000 + i),
                class: mc::DEVICE,
                facts: AllocFacts {
                    device_instance: Some(i),
                    ..Default::default()
                },
            }),
        ));
    }
    assert_eq!(
        gpu.spine.targets.len(),
        1,
        "the flood minted {} extra GpuTargets — each one a guest-physical window and a \
         delivery plane, and `targets` is never pruned",
        gpu.spine.targets.len() - 1
    );
    for (i, outcome) in outcomes {
        assert_eq!(
            outcome,
            Err(GpuError::Graph(RmGraphError::InvalidDeviceInstance {
                instance: i
            })),
            "instance {i} is not this device's"
        );
    }
}

/// ★ **The no-GPU0-guess doctrine, applied to our own code.** `walk_gpu` read
/// `device_instance.unwrap_or(0)` — a default-to-GPU-0 guess, in the one resolver whose
/// whole discipline is MISS=None-never-a-guess. A Device with no declared instance now
/// leaves its subtree **unroutable**: no `by_pdb` entry, no arena, no isolate, and a loud
/// `UnknownPdb` at use — never a silent bind onto GPU 0's plane.
///
/// (A real Device cannot be in this state — `deviceId` is a required field of
/// `NV0080_ALLOC_PARAMETERS` — which is exactly why guessing for it was indefensible.)
#[test]
fn g9_an_undeclared_device_instance_is_unroutable_not_gpu_zero() {
    let mut gpu = fresh_gpu();
    const C: HClient = HClient(0x9200);
    const P: Pdb = Pdb(0x9200_0000);
    let dev = HObject(0x100);
    let vas = HObject(0x110);
    for ev in [
        kayfabe_tests::client_root(C),
        RmEvent::Alloc {
            client: C,
            parent: HObject(C.0),
            handle: dev,
            class: mc::DEVICE,
            facts: AllocFacts::default(), // ← no declared instance
        },
        RmEvent::Alloc {
            client: C,
            parent: dev,
            handle: vas,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: C,
            vaspace: vas,
            pdb: P,
        },
    ] {
        gpu.apply(ev)
            .expect("an undeclared Device is not a protocol error");
    }
    assert_eq!(
        gpu.spine.by_pdb.get(&(GpuId::ZERO, P)),
        None,
        "an undeclared instance must NOT route onto GPU 0"
    );
    assert_eq!(
        resolve(&gpu, GpuId::ZERO, P, GpuVa(0x1000)),
        Err(FwdFault::UnknownPdb {
            gpu: GpuId::ZERO,
            pdb: P
        }),
        "it is a loud miss at use, exactly like every other unresolved target"
    );
    // …and it consumed no device resources at all.
    assert_eq!(gpu.spine.targets.len(), 1);
    assert!(
        gpu.procs.values().all(|p| p.arenas.is_empty()),
        "an unroutable proc materializes no arena"
    );
}

/// ★ **Verified against the open kmod, and easy to get wrong while fixing G9:** the SAME
/// `deviceInstance` twice under ONE client is **legal on bare metal**
/// (`ogkm src/nvidia/src/kernel/gpu/device.c:368-380` rejects it only under `IS_VIRTUAL`).
/// Device-per-client is not 1:1, so the entitlement check is a *membership* test and must
/// never become a uniqueness test.
#[test]
fn g9_the_same_device_instance_twice_under_one_client_is_legal() {
    let mut gpu = fresh_gpu();
    const C: HClient = HClient(0x9300);
    gpu.apply(kayfabe_tests::client_root(C))
        .expect("client root");
    for handle in [HObject(0x100), HObject(0x101)] {
        gpu.apply(RmEvent::Alloc {
            client: C,
            parent: HObject(C.0),
            handle,
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        })
        .expect("two Devices on one GPU under one client is bare-metal legal");
    }
    assert_eq!(gpu.spine.targets.len(), 1, "and they share the one target");
}

// =================================================================================
// ★ G10 — `Spine::condemned` and `Spine::retired` were unbounded (§12.22)
// =================================================================================

/// ★ **The condemned list is capped, and the refusal lands where it does no harm.**
///
/// A condemned entry clears only when the guest frees its client root — which a guest
/// that just crashed its own worker has no incentive to do — and the list was rescanned
/// on **every** apply. Every neighbouring guest-reachable surface has a named cap; this
/// one had none.
///
/// The interesting half is *what* gets refused. Refusing a **condemnation** would be
/// worse than useless: it would leave a component whose isolate is dead un-condemned, so
/// the next refresh would re-derive it with a fresh isolate and serve the guest a
/// **zeroed** backing for a VA it believes still holds its data — §12.13's silent
/// corruption, reintroduced by a memory cap. So the refusal lands on the only
/// guest-reachable action that *consumes* the list: deriving a **new** `Proc`. Everything
/// already condemned stays condemned, every live proc keeps serving, and the guest
/// recovers exactly as it does from one condemnation — by letting the dead processes'
/// client roots be freed.
#[test]
fn g10_condemnation_is_capped_and_refuses_new_procs_never_the_condemnation() {
    kayfabe_tests::skip_slow!(
        "g10_condemnation_is_capped_and_refuses_new_procs_never_the_condemnation"
    );
    let mut gpu = fresh_gpu();
    let cap = kayfabe_core::gpu::MAX_CONDEMNED_COMPONENTS;

    // A live bystander that must keep working throughout.
    let victim = HClient(0xB000);
    for ev in benign_b_events() {
        gpu.apply(ev).expect("the bystander applies");
    }
    let _ = victim;

    // The hostile pattern: spawn a worker, kill it, repeat. Retiring out of band keeps
    // the LIVE proc set small while the condemned list grows — which is exactly why the
    // list needed its own bound rather than inheriting the proc set's.
    let mut condemned_clients = Vec::new();
    for i in 0..cap {
        let c = HClient(0x10_0000 + i as u32);
        gpu.apply(kayfabe_tests::client_root(c))
            .expect("a fresh client derives a proc");
        let pid = *gpu
            .procs
            .iter()
            .find(|(_, p)| p.clients.contains(&c))
            .expect("its proc")
            .0;
        assert!(gpu.retire_proc(pid), "worker died");
        drop(gpu.reap_retired()); // keep the OTHER cap out of this test's way
        condemned_clients.push(c);
    }
    assert_eq!(gpu.spine.condemned_len(), cap);

    // ★ A new process is refused — loudly, exactly, and without touching anything.
    let fresh = HClient(0x20_0000);
    assert_eq!(
        gpu.apply(kayfabe_tests::client_root(fresh)),
        Err(GpuError::SpineCapacity {
            what: kayfabe_core::gpu::SpineCapacity::CondemnedComponents,
            cap,
        }),
        "the cap must be a named, loud refusal"
    );
    // Nothing was un-condemned to make room.
    assert_eq!(gpu.spine.condemned_len(), cap);
    assert!(
        condemned_clients.iter().all(|c| gpu.spine.is_condemned(*c)),
        "every condemnation is still in force"
    );
    // The bystander is untouched and still serves.
    assert_eq!(
        resolve(&gpu, GpuId::ZERO, B_PDB, B_MAP_VA).map(|(b, _)| b.phys),
        Ok(B_MEM_PHYS)
    );

    // ★ Recovery is the guest's own, and needs no cooperation from the dead: the guest
    // kernel frees a dead process's client root, the entry prunes, and the device serves
    // new processes again.
    let freed = condemned_clients[0];
    gpu.apply(RmEvent::Free {
        client: freed,
        handle: HObject(freed.0),
    })
    .expect("the guest kernel frees a dead process's client root");
    assert_eq!(gpu.spine.condemned_len(), cap - 1);
    assert!(!gpu.spine.is_condemned(freed));
    gpu.apply(kayfabe_tests::client_root(fresh))
        .expect("the device serves new processes again — backpressure, not a brick");
}

/// ★ **The retired list is capped too, and reaping is what clears it.**
///
/// `Spine::retired` holds an isolate and a GPA arena per entry and is drained only when
/// the *adapter* declares a quiesce point (lesson L10). An adapter that never reaches
/// one — or a guest that keeps churning processes faster than they are reaped — grew it
/// without limit. Same refusal shape as the condemned cap, and for the same reason:
/// refusing a *retirement* would leave a proc live whose worker is gone, and dropping
/// corpses would leak exactly the isolates and arenas the list exists to reclaim.
#[test]
fn g10_the_retired_list_is_capped_and_a_reap_clears_it() {
    kayfabe_tests::skip_slow!("g10_the_retired_list_is_capped_and_a_reap_clears_it");
    let mut gpu = fresh_gpu();
    let cap = kayfabe_core::gpu::MAX_RETIRED_PROCS;

    // Churn: create a process, let the guest free its root, never reap.
    for i in 0..cap {
        let c = HClient(0x30_0000 + i as u32);
        gpu.apply(kayfabe_tests::client_root(c)).expect("derives");
        gpu.apply(RmEvent::Free {
            client: c,
            handle: HObject(c.0),
        })
        .expect("the guest tears it down");
    }
    assert_eq!(gpu.spine.retired_len(), cap);

    let fresh = HClient(0x40_0000);
    assert_eq!(
        gpu.apply(kayfabe_tests::client_root(fresh)),
        Err(GpuError::SpineCapacity {
            what: kayfabe_core::gpu::SpineCapacity::RetiredProcs,
            cap,
        }),
        "unreaped corpses are a named, loud bound"
    );
    assert_eq!(
        gpu.spine.retired_len(),
        cap,
        "and the refusal drops no corpse — the isolates and arenas they hold are the \
         whole reason the list exists"
    );

    // The adapter reaches its quiesce point; the device serves again.
    let reclaimed = gpu.reap_retired();
    assert_eq!(reclaimed.len(), cap);
    assert!(reclaimed.orphaned().is_empty());
    drop(reclaimed);
    assert_eq!(gpu.spine.retired_len(), 0);
    gpu.apply(kayfabe_tests::client_root(fresh))
        .expect("a reap is what clears it");
}

/// ★ **`RmGraph::apply` is atomic on failure — and that is the load-bearing precondition
/// for ever deleting `Spine::apply`'s per-event graph clone** (`l1_concurrency.md`
/// §12.23). Checked here rather than argued, because the argument ("every error return
/// precedes the mutation") is exactly the kind that decays as handlers are edited.
///
/// Every refusable protocol event, applied to a live graph, must leave the graph's nodes,
/// dup edges and mappings **byte-identical** — and answer with its exact variant.
#[test]
fn rmgraph_apply_is_atomic_on_failure() {
    use kayfabe_core::rmgraph::{Mapping, RmNode};

    let arch = MockArch::new();
    let mut g = RmGraph::new();
    const C: HClient = HClient(0xA0);
    const OTHER: HClient = HClient(0xB0);
    let root = HObject(C.0);
    let dev = HObject(0x100);
    let vas = HObject(0x110);
    let mem = HObject(0x120);
    let facts_dev = AllocFacts {
        device_instance: Some(0),
        ..Default::default()
    };
    for ev in [
        kayfabe_tests::client_root(C),
        RmEvent::Alloc {
            client: C,
            parent: root,
            handle: dev,
            class: mc::DEVICE,
            facts: facts_dev,
        },
        RmEvent::Alloc {
            client: C,
            parent: dev,
            handle: vas,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        },
        RmEvent::Alloc {
            client: C,
            parent: dev,
            handle: mem,
            class: mc::MEMORY,
            facts: AllocFacts {
                mem_phys: Some(0x9_0000_0000),
                ..Default::default()
            },
        },
        RmEvent::MapMemoryDma {
            client: C,
            vaspace: vas,
            memory: mem,
            va: GpuVa(0x2_0000_0000),
            offset: 0,
            len: 0x10000,
        },
        RmEvent::Dup {
            src: NodeKey::new(C, vas),
            dst: NodeKey::new(OTHER, HObject(0x200)),
        },
    ] {
        g.apply(&arch, ev).expect("the good graph builds");
    }

    let fingerprint = |g: &RmGraph| -> (Vec<RmNode>, Vec<(NodeKey, NodeKey)>, Vec<Mapping>) {
        (
            g.nodes().copied().collect(),
            g.dups().collect(),
            g.mappings().copied().collect(),
        )
    };
    let before = fingerprint(&g);

    let refusals: Vec<(RmEvent, RmGraphError)> = vec![
        // A different alloc onto a live handle.
        (
            RmEvent::Alloc {
                client: C,
                parent: dev,
                handle: vas,
                class: mc::MEMORY,
                facts: AllocFacts::default(),
            },
            RmGraphError::ConflictingAlloc(NodeKey::new(C, vas)),
        ),
        // A Device naming an instance this device was not realized with (G9).
        (
            RmEvent::Alloc {
                client: C,
                parent: root,
                handle: HObject(0x130),
                class: mc::DEVICE,
                facts: AllocFacts {
                    device_instance: Some(5),
                    ..Default::default()
                },
            },
            RmGraphError::InvalidDeviceInstance { instance: 5 },
        ),
        // A second, different dup onto a taken alias handle.
        (
            RmEvent::Dup {
                src: NodeKey::new(C, mem),
                dst: NodeKey::new(OTHER, HObject(0x200)),
            },
            RmGraphError::ConflictingDup(NodeKey::new(OTHER, HObject(0x200))),
        ),
        // A different mapping at a live (vaspace, va).
        (
            RmEvent::MapMemoryDma {
                client: C,
                vaspace: vas,
                memory: mem,
                va: GpuVa(0x2_0000_0000),
                offset: 0x1000,
                len: 0x10000,
            },
            RmGraphError::ConflictingMap {
                vaspace: NodeKey::new(C, vas),
                va: GpuVa(0x2_0000_0000),
            },
        ),
        // A free of a handle nobody owns.
        (
            RmEvent::Free {
                client: C,
                handle: HObject(0xDEAD),
            },
            RmGraphError::FreeUnknown(NodeKey::new(C, HObject(0xDEAD))),
        ),
    ];

    for (ev, expected) in refusals {
        let got = g.apply(&arch, ev).expect_err("this event is refusable");
        assert_eq!(got, expected, "the refusal must name itself exactly");
        assert_eq!(
            fingerprint(&g),
            before,
            "a refused event must leave the graph byte-identical — {got:?}"
        );
    }
}
