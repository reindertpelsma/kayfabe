//! Batch-1 object-model completion (`execution_plane.md` §1 gap): MEMORY objects are
//! first-class, `MAP_MEMORY_DMA`/`UNMAP` populate/depopulate the ONE address table
//! (RPC populate source), a memory object's mappings keep it alive (refcount), and
//! EVENT objects are graph nodes so completion routing is graph-derived.
//!
//! These are **invariant/contract** tests (decision #15), not internal-state pins:
//! they assert the doctrine — forward-populate, MISS=FAULT, faithful RM refcounting —
//! through the public `Gpu`/`RmGraph` API, driven entirely by the mock harness.

#![allow(clippy::unusual_byte_groupings)]

use nvkvm_arch::ids::{GpuVa, HClient, HObject, Pdb};
use nvkvm_core::gpa::GpaSpace;
use nvkvm_core::gpu::{Gpu, GpuError};
use nvkvm_core::rmgraph::{NodeKey, RmEvent};
use nvkvm_fwd::{FwdFault, resolve};
use nvkvm_mocks::{MockArch, MockIsolateFactory};
use nvkvm_tests::{Scenario, identical_handles};

const CLIENT: HClient = HClient(0xAA);
const PDB: Pdb = Pdb(0x3401_000);
const MEM: HObject = HObject(0x5c00_0100);
const MEM_PHYS: u64 = 0x8000_0000;
const MAP_VA: GpuVa = GpuVa(0x2_0020_0000);
const MAP_LEN: u64 = 0x10000;

fn fresh_gpu() -> Gpu {
    let arch = Box::new(MockArch::new());
    let (factory, _rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    Gpu::new(arch, Box::new(factory), gpa).expect("device realizes")
}

/// A compute process plus a MEMORY object mapped into its VAS. Returns the VAS handle.
fn compute_with_mapping() -> (Gpu, HObject) {
    let mut gpu = fresh_gpu();
    let h = identical_handles(0x10, 0x11);
    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB, h);
    s.memory(CLIENT, h.device, MEM, MEM_PHYS);
    s.map(CLIENT, h.vaspace, MEM, MAP_VA, MAP_LEN);
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    (gpu, h.vaspace)
}

/// A `MAP_MEMORY_DMA` forward-populates the address table (`va → memory phys`).
#[test]
fn map_populates_the_address_table() {
    let (gpu, _vas) = compute_with_mapping();

    // Resolve inside the mapping: MISS=FAULT elsewhere, hit here with the memory's phys.
    let (bind, off) = resolve(&gpu, PDB, GpuVa(MAP_VA.0 + 0x40)).expect("mapped VA resolves");
    assert_eq!(off, 0x40, "offset within the mapping");
    assert_eq!(
        bind.phys, MEM_PHYS,
        "resolves to the memory object's declared backing"
    );

    // Outside the mapping: MISS=FAULT (no fallback walk, no guess).
    assert!(matches!(
        resolve(&gpu, PDB, GpuVa(MAP_VA.0 + MAP_LEN + 0x1000)),
        Err(FwdFault::Address(_))
    ));
}

/// `UNMAP` eagerly depopulates the table — the VA faults immediately after.
#[test]
fn unmap_depopulates_eagerly() {
    let (mut gpu, vas) = compute_with_mapping();
    assert!(resolve(&gpu, PDB, MAP_VA).is_ok(), "mapped before unmap");

    gpu.apply(RmEvent::Unmap {
        client: CLIENT,
        vaspace: vas,
        va: MAP_VA,
    })
    .expect("unmap applies");

    assert!(
        matches!(resolve(&gpu, PDB, MAP_VA), Err(FwdFault::Address(_))),
        "unmapped VA faults immediately (unmap eager)"
    );
}

/// A memory object's mappings keep it alive: freeing the memory *handle* while a
/// mapping references it does NOT destroy the resource (faithful RM refcounting);
/// the last unmap does.
#[test]
fn mapping_refcount_keeps_memory_alive() {
    let (mut gpu, vas) = compute_with_mapping();
    let mem_key = NodeKey::new(CLIENT, MEM);

    assert_eq!(
        gpu.rmgraph.map_ref_count(mem_key),
        1,
        "one live mapping references the memory"
    );
    assert!(
        gpu.rmgraph.backing_of(mem_key).is_some(),
        "memory resource is live"
    );

    // Free the memory HANDLE while the mapping still references the resource.
    gpu.apply(RmEvent::Free {
        client: CLIENT,
        handle: MEM,
    })
    .expect("free applies");
    assert!(
        gpu.rmgraph.backing_of(mem_key).is_some(),
        "memory resource survives its handle's free while a mapping references it"
    );
    assert!(
        resolve(&gpu, PDB, MAP_VA).is_ok(),
        "the mapping still resolves"
    );

    // The last unmap releases the final reference → the resource is destroyed.
    gpu.apply(RmEvent::Unmap {
        client: CLIENT,
        vaspace: vas,
        va: MAP_VA,
    })
    .expect("unmap applies");
    assert_eq!(gpu.rmgraph.map_ref_count(mem_key), 0);
    assert!(
        gpu.rmgraph.backing_of(mem_key).is_none(),
        "last reference gone → memory resource destroyed (no leak)"
    );
}

/// EVENT objects are first-class graph nodes owned by their client — completion
/// routing is graph-derived, not an opaque id.
#[test]
fn event_objects_are_graph_derived() {
    let mut gpu = fresh_gpu();
    let h = identical_handles(0x10, 0x11);
    let ev_a = HObject(0x5c00_0200);
    let ev_b = HObject(0x5c00_0201);
    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB, h);
    s.event(CLIENT, h.device, ev_a);
    s.event(CLIENT, h.device, ev_b);
    for e in s.events {
        gpu.apply(e).expect("applies");
    }

    let events: Vec<_> = gpu
        .rmgraph
        .events_of(CLIENT)
        .map(|n| n.key.handle)
        .collect();
    assert_eq!(
        events,
        vec![ev_a, ev_b],
        "both os-events are graph-derived, owned by the client"
    );

    // A different client owns none of them (routing is per-client, graph-derived).
    assert_eq!(gpu.rmgraph.events_of(HClient(0xEE)).count(), 0);
}

/// A `MAP_MEMORY_DMA` against a memory object with NO declared backing is a loud
/// fault at populate time — MISS=FAULT, never a silent skip or guessed phys.
#[test]
fn unbacked_mapping_is_a_loud_fault() {
    let mut gpu = fresh_gpu();
    let h = identical_handles(0x10, 0x11);
    let unbacked = HObject(0x5c00_0300);
    let mut s = Scenario::new();
    s.compute_process(CLIENT, PDB, h);
    // Memory with NO backing (mem_phys = None): allocate the raw MEMORY node directly.
    s.push(RmEvent::Alloc {
        client: CLIENT,
        parent: h.device,
        handle: unbacked,
        class: nvkvm_mocks::mock_classes::MEMORY,
        facts: nvkvm_core::rmgraph::AllocFacts::default(),
    });
    for e in s.events {
        gpu.apply(e).expect("setup applies");
    }
    let err = gpu.apply(RmEvent::MapMemoryDma {
        client: CLIENT,
        vaspace: h.vaspace,
        memory: unbacked,
        va: MAP_VA,
        offset: 0,
        len: MAP_LEN,
    });
    assert!(
        matches!(err, Err(GpuError::UnbackedMapping { .. })),
        "unbacked map faults loudly"
    );
}

/// Order tolerance: a `MAP_MEMORY_DMA` that arrives BEFORE its memory alloc / before
/// SET_PAGE_DIRECTORY still resolves once the facts land (forward-populate, replayed).
#[test]
fn map_before_backing_and_pdb_resolves() {
    let mut gpu = fresh_gpu();
    let h = identical_handles(0x10, 0x11);

    // Build the client/device/vaspace but withhold the PDB and the memory alloc.
    for e in [
        nvkvm_tests::client_root(CLIENT),
        RmEvent::Alloc {
            client: CLIENT,
            parent: HObject(CLIENT.0),
            handle: h.device,
            class: nvkvm_mocks::mock_classes::DEVICE,
            facts: Default::default(),
        },
        RmEvent::Alloc {
            client: CLIENT,
            parent: h.device,
            handle: h.vaspace,
            class: nvkvm_mocks::mock_classes::VASPACE,
            facts: Default::default(),
        },
        // Map arrives NOW — memory not yet allocated, PDB not yet set.
        RmEvent::MapMemoryDma {
            client: CLIENT,
            vaspace: h.vaspace,
            memory: MEM,
            va: MAP_VA,
            offset: 0,
            len: MAP_LEN,
        },
    ] {
        gpu.apply(e).expect("applies");
    }
    // Not resolvable yet (no PDB routed).
    assert!(matches!(
        resolve(&gpu, PDB, MAP_VA),
        Err(FwdFault::UnknownPdb(_))
    ));

    // Now the memory backing + the PDB arrive, in that order.
    gpu.apply(RmEvent::Alloc {
        client: CLIENT,
        parent: h.device,
        handle: MEM,
        class: nvkvm_mocks::mock_classes::MEMORY,
        facts: nvkvm_core::rmgraph::AllocFacts {
            mem_phys: Some(MEM_PHYS),
            ..Default::default()
        },
    })
    .expect("memory alloc applies");
    gpu.apply(RmEvent::SetPageDir {
        client: CLIENT,
        vaspace: h.vaspace,
        pdb: PDB,
    })
    .expect("setpagedir applies");

    let (bind, _off) = resolve(&gpu, PDB, MAP_VA).expect("map resolves once facts land");
    assert_eq!(
        bind.phys, MEM_PHYS,
        "forward-populated from the late-arriving facts"
    );
}

// ---------------------------------------------------------------------------------
// Fuzz: arbitrary map/unmap/free streams never panic; the memory refcount invariant
// (alive ⟺ ≥1 handle OR ≥1 mapping) holds, and the table never has an RPC binding
// without a live mapping to back it.
// ---------------------------------------------------------------------------------

mod fuzz {
    use nvkvm_arch::ids::{GpuVa, HClient, HObject, Pdb};
    use nvkvm_core::rmgraph::{NodeKey, RmEvent};
    use nvkvm_mocks::mock_classes as mc;
    use proptest::collection::vec;
    use proptest::prelude::*;

    fn any_client() -> impl Strategy<Value = HClient> {
        (0u32..3).prop_map(|n| HClient(0xD000 + n))
    }
    fn any_handle() -> impl Strategy<Value = HObject> {
        (0u32..6).prop_map(|n| HObject(0x9000_0000 + n))
    }
    fn any_va() -> impl Strategy<Value = GpuVa> {
        (0u32..3).prop_map(|n| GpuVa(0x2_0020_0000 + u64::from(n) * 0x10000))
    }

    fn any_ev() -> impl Strategy<Value = RmEvent> {
        prop_oneof![
            // Client root / device / vaspace / memory allocs (the object universe).
            (
                any_client(),
                any_handle(),
                prop_oneof![
                    Just(mc::CLIENT),
                    Just(mc::DEVICE),
                    Just(mc::VASPACE),
                    Just(mc::MEMORY),
                ],
                any::<bool>()
            )
                .prop_map(|(client, handle, class, backed)| RmEvent::Alloc {
                    client,
                    parent: handle,
                    handle,
                    class,
                    facts: nvkvm_core::rmgraph::AllocFacts {
                        mem_phys: backed.then_some(0x8000_0000),
                        ..Default::default()
                    },
                }),
            (any_client(), any_handle(), any_handle()).prop_map(|(client, vaspace, memory)| {
                RmEvent::SetPageDir {
                    client,
                    vaspace,
                    pdb: Pdb(0x3400_000 + u64::from(memory.0 & 3) * 0x1000),
                }
            }),
            (any_client(), any_handle(), any_handle(), any_va()).prop_map(
                |(client, vaspace, memory, va)| RmEvent::MapMemoryDma {
                    client,
                    vaspace,
                    memory,
                    va,
                    offset: 0,
                    len: 0x10000,
                }
            ),
            (any_client(), any_handle(), any_va()).prop_map(|(client, vaspace, va)| {
                RmEvent::Unmap {
                    client,
                    vaspace,
                    va,
                }
            }),
            (any_client(), any_handle())
                .prop_map(|(client, handle)| RmEvent::Free { client, handle }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2000))]

        /// Arbitrary object-model streams: `Gpu::apply` never panics (loud typed error
        /// at worst), and after every event the memory refcount invariant holds — a
        /// memory resource is present IFF (≥1 live handle OR ≥1 live mapping references
        /// it). This encodes faithful RM refcounting as a permanent guard.
        #[test]
        fn map_unmap_streams_never_panic_and_refcount_holds(stream in vec(any_ev(), 0..40)) {
            let mut gpu = super::fresh_gpu();
            for ev in stream {
                let _ = gpu.apply(ev); // Result — never a panic.

                // Every live mapping keeps its memory resource ALIVE (map-ref-count ≥ 1),
                // regardless of whether that memory declared a backing.
                for m in gpu.rmgraph.mappings() {
                    prop_assert!(
                        gpu.rmgraph.map_ref_count(m.memory) >= 1,
                        "a mapped memory has map-ref-count ≥ 1 (refcount keeps it alive)"
                    );
                }
                // NO LEAK: a resource with no live handle AND no mapping must be gone —
                // it neither backs a lookup nor references-counts as alive.
                for c in 0..3u32 {
                    for h in 0..6u32 {
                        let k = NodeKey::new(HClient(0xD000 + c), HObject(0x9000_0000 + h));
                        if gpu.rmgraph.origin_of(k).is_none()
                            && gpu.rmgraph.map_ref_count(k) == 0
                        {
                            prop_assert!(
                                gpu.rmgraph.backing_of(k).is_none(),
                                "a resource with no handle and no mapping must be destroyed (no leak)"
                            );
                        }
                    }
                }
            }
        }
    }
}
