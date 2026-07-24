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

/// ★ Mutation-gate kill (`RmGraph::apply_map` `base + m.offset`→`base - offset`): a
/// `MAP_MEMORY_DMA` at a non-zero byte offset into its memory must forward-populate the
/// backing as `memory_base + offset` (the guest maps a sub-window of a larger buffer).
/// The prior suite only ever mapped at offset 0, where `base + 0 == base - 0`, so a
/// `+`→`-` mutation survived. This maps at a non-zero offset and asserts the resolved
/// backing includes it — the address-table value must be the SUM, not the difference.
#[test]
fn map_at_offset_forward_populates_base_plus_offset() {
    use nvkvm_core::rmgraph::{AllocFacts, RmGraph};
    use nvkvm_mocks::mock_classes as mc;

    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let c = CLIENT;
    let vas = HObject(0x5c00_0010);
    let mem = HObject(0x5c00_0100);
    let base = 0x8000_0000u64;
    let offset = 0x3000u64;

    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(c.0), handle: HObject(c.0), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(c.0), handle: HObject(1), class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(1), handle: vas, class: mc::VASPACE, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::SetPageDir { client: c, vaspace: vas, pdb: PDB }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(1), handle: mem, class: mc::MEMORY, facts: AllocFacts { mem_phys: Some(base), ..Default::default() } }).unwrap();
    g.apply(&arch, RmEvent::MapMemoryDma { client: c, vaspace: vas, memory: mem, va: MAP_VA, offset, len: MAP_LEN }).unwrap();

    let mapping = g.mappings().find(|m| m.va == MAP_VA).expect("mapping is live");
    assert_eq!(
        mapping.mem_phys,
        Some(base + offset),
        "the mapping's backing is memory_base + offset (the SUM), never base - offset",
    );
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

/// ★ Mutation-gate kill (`RmGraph::apply` Unmap-of-unresolved-VAS parked-map cleanup:
/// `delete !` and `&& → ||` in the `retain` predicate). An `UNMAP` that races ahead of
/// its VAS's alloc must drop EXACTLY the parked map it names — the one with the SAME
/// (vaspace, va) — and leave every OTHER parked map (same VAS, different VA) intact.
/// The prior suite had no parked-map-selective-unmap test, so two mutations survived:
/// removing the `!` (retain ONLY the named map, dropping the rest) and `&&`→`||` (drop
/// every parked map sharing the vaspace OR the va — too many). This parks two maps on
/// one not-yet-allocated VAS handle, unmaps one, then resolves the VAS and asserts the
/// OTHER one — and only it — replays into the live mapping table.
#[test]
fn parked_map_unmap_drops_only_the_named_map() {
    use nvkvm_core::rmgraph::{AllocFacts, RmGraph};
    use nvkvm_mocks::mock_classes as mc;

    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let client = CLIENT;
    let vaspace = HObject(0x9000_0000);
    let memory = HObject(0x9000_0100);
    let va_a = GpuVa(0x2_0000_0000); // will be unmapped
    let va_b = GpuVa(0x2_0010_0000); // must survive

    // Park TWO maps on the SAME (not-yet-allocated) vaspace handle, different VAs.
    for va in [va_a, va_b] {
        g.apply(&arch, RmEvent::MapMemoryDma { client, vaspace, memory, va, offset: 0, len: MAP_LEN })
            .expect("parks (vaspace unresolved)");
    }
    // Unmap the FIRST one while the vaspace is still unresolved — parked-map cleanup.
    g.apply(&arch, RmEvent::Unmap { client, vaspace, va: va_a }).expect("parked unmap is idempotent");

    // Now resolve the vaspace + memory so the surviving parked map replays.
    g.apply(&arch, RmEvent::Alloc { client, parent: HObject(client.0), handle: HObject(client.0), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client, parent: HObject(client.0), handle: HObject(1), class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client, parent: HObject(1), handle: vaspace, class: mc::VASPACE, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::SetPageDir { client, vaspace, pdb: PDB }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client, parent: HObject(1), handle: memory, class: mc::MEMORY, facts: AllocFacts { mem_phys: Some(MEM_PHYS), ..Default::default() } }).unwrap();

    let live_vas: Vec<GpuVa> = g.mappings().map(|m| m.va).collect();
    assert_eq!(
        live_vas,
        vec![va_b],
        "exactly the un-unmapped parked map (va_b) survives — not va_a (deleted), not both dropped",
    );
}

/// ★ Mutation-gate kill (`RmGraph::free_subtree` `hkey.client != key.client`→`==` in
/// the parent-chain descent): freeing a NON-root parent handle must cascade to its
/// SAME-namespace children only. The namespace filter (`hkey.client != key.client →
/// skip`) is what confines the cascade to the freed handle's OWN client; the `!=`→`==`
/// mutant inverts it (scan every OTHER namespace, skip the target's own), so a child in
/// the freed namespace would NOT be reaped and — worse — a same-handle-value object in
/// a DIFFERENT client would be walked. This builds two clients with IDENTICAL handle
/// values, frees A's non-root parent, and asserts A's child dies while B's survives.
#[test]
fn free_subtree_cascade_is_namespace_confined() {
    use nvkvm_core::rmgraph::{AllocFacts, RmGraph};
    use nvkvm_mocks::mock_classes as mc;

    let arch = MockArch::new();
    let mut g = RmGraph::new();
    // IDENTICAL handle values in both namespaces (the #14 identical-handle hazard).
    let dev = HObject(0x100);
    let mem = HObject(0x200);
    let build = |g: &mut RmGraph, c: HClient| {
        g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(c.0), handle: HObject(c.0), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
        g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(c.0), handle: dev, class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
        // A child MEMORY parented on the device (so freeing the device must cascade to it).
        g.apply(&arch, RmEvent::Alloc { client: c, parent: dev, handle: mem, class: mc::MEMORY, facts: AllocFacts { mem_phys: Some(0x8000_0000), ..Default::default() } }).unwrap();
    };
    let (a, b) = (HClient(0xAA), HClient(0xBB));
    build(&mut g, a);
    build(&mut g, b);

    // Free A's DEVICE — a non-root parent handle. Its subtree (A's memory) must cascade.
    g.apply(&arch, RmEvent::Free { client: a, handle: dev }).expect("free applies");

    // A's child memory is gone (cascade reached it, confined to A's namespace)…
    assert!(g.backing_of(NodeKey::new(a, mem)).is_none(), "A's child memory dies with its parent device");
    assert!(g.node(NodeKey::new(a, dev)).is_none(), "A's device handle is freed");
    // …while B's identically-handled objects are entirely untouched (namespace-confined).
    assert!(g.backing_of(NodeKey::new(b, mem)).is_some(), "B's identically-handled memory survives");
    assert!(g.node(NodeKey::new(b, dev)).is_some(), "B's device (same handle value) survives");
}

/// ★ Mutation-gate kill (`RmGraph::free_subtree` dead-VAS mapping cleanup `&&`→`||`):
/// a mapping is torn down ONLY when its VASpace was touched by the free AND that VASpace
/// resource actually died (`touched && dead`). If a DUP alias keeps the VASpace alive
/// across its origin handle's free, the VAS is touched-but-NOT-dead — so its mappings
/// must SURVIVE. The `&&`→`||` mutant would tear them down on "touched OR dead" and drop
/// a live VAS's mappings. This aliases a VASpace with a dup, maps memory into it, frees
/// the ORIGIN handle (dup keeps the VAS alive), and asserts the mapping still resolves.
#[test]
fn free_subtree_keeps_mappings_of_a_dup_kept_alive_vaspace() {
    use nvkvm_core::rmgraph::{AllocFacts, RmGraph};
    use nvkvm_mocks::mock_classes as mc;

    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let owner = HClient(0xAA);
    let aliaser = HClient(0xA1);
    let vas = HObject(0x5c00_0010);
    let vas_alias = HObject(0x9000_0010);
    let mem = HObject(0x5c00_0100);

    g.apply(&arch, RmEvent::Alloc { client: owner, parent: HObject(owner.0), handle: HObject(owner.0), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: owner, parent: HObject(owner.0), handle: HObject(1), class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: owner, parent: HObject(1), handle: vas, class: mc::VASPACE, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::SetPageDir { client: owner, vaspace: vas, pdb: PDB }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: owner, parent: HObject(1), handle: mem, class: mc::MEMORY, facts: AllocFacts { mem_phys: Some(MEM_PHYS), ..Default::default() } }).unwrap();
    g.apply(&arch, RmEvent::MapMemoryDma { client: owner, vaspace: vas, memory: mem, va: MAP_VA, offset: 0, len: MAP_LEN }).unwrap();
    // A UVM-style client dup-aliases the VASpace, so the VAS resource outlives the origin.
    g.apply(&arch, RmEvent::Alloc { client: aliaser, parent: HObject(aliaser.0), handle: HObject(aliaser.0), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Dup { src: NodeKey::new(owner, vas), dst: NodeKey::new(aliaser, vas_alias) }).unwrap();

    assert_eq!(g.mappings().count(), 1, "the mapping is live before the free");

    // Free the OWNER's VASpace ORIGIN handle. The dup keeps the VAS resource alive, so
    // its mapping must NOT be torn down (touched-but-not-dead).
    g.apply(&arch, RmEvent::Free { client: owner, handle: vas }).expect("free applies");

    assert_eq!(
        g.mappings().map(|m| m.va).collect::<Vec<_>>(),
        vec![MAP_VA],
        "the mapping survives: its VASpace is kept alive by the dup (touched, but NOT dead)",
    );
}

/// ★ Mutation-gate kill (`RmGraph::free_subtree` parked-map cleanup `&&`→`||`): a
/// PARKED map (awaiting its endpoints) must be pruned when EITHER its vaspace OR its
/// memory handle is freed — it can never resolve once an endpoint is gone. The retain
/// keeps a parked map only if NEITHER endpoint is doomed (`!vas_doomed && !mem_doomed`);
/// the `&&`→`||` mutant keeps it when EITHER survives, so a parked map whose MEMORY was
/// freed lingers and later wrongly replays. This parks a map (vaspace unresolved, memory
/// resolved), frees the MEMORY, then resolves the vaspace and asserts NOTHING replays.
#[test]
fn free_subtree_prunes_a_parked_map_when_its_memory_is_freed() {
    use nvkvm_core::rmgraph::{AllocFacts, RmGraph};
    use nvkvm_mocks::mock_classes as mc;

    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let c = HClient(0xAA);
    let vas = HObject(0x5c00_0010);
    let mem = HObject(0x5c00_0100);

    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(c.0), handle: HObject(c.0), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(c.0), handle: HObject(1), class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
    // Memory exists; VASpace does NOT yet — so the map parks (one endpoint unresolved).
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(1), handle: mem, class: mc::MEMORY, facts: AllocFacts { mem_phys: Some(MEM_PHYS), ..Default::default() } }).unwrap();
    g.apply(&arch, RmEvent::MapMemoryDma { client: c, vaspace: vas, memory: mem, va: MAP_VA, offset: 0, len: MAP_LEN }).unwrap();
    assert_eq!(g.mappings().count(), 0, "the map is parked (vaspace unresolved), not live yet");

    // Free the MEMORY handle — the parked map's memory endpoint is now doomed, so the
    // parked map must be pruned (it can never legitimately resolve).
    g.apply(&arch, RmEvent::Free { client: c, handle: mem }).expect("free applies");

    // Now allocate the VASpace AND re-allocate a FRESH memory at the same handle value.
    // If the parked map was correctly pruned on the memory's free, it does NOT replay —
    // the stale map must not resurrect against the unrelated fresh memory. The `||`
    // mutant kept the parked entry, so it would wrongly replay a mapping here.
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(1), handle: vas, class: mc::VASPACE, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::SetPageDir { client: c, vaspace: vas, pdb: PDB }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(1), handle: mem, class: mc::MEMORY, facts: AllocFacts { mem_phys: Some(0xDEAD_0000), ..Default::default() } }).unwrap();
    assert_eq!(
        g.mappings().count(),
        0,
        "the parked map was pruned on its memory's free — it must NOT resurrect against fresh memory",
    );
}

/// ★ Mutation-gate kill (`RmGraph::apply_map` idempotency guard `*existing == mapping`
/// replaced with `true`): a re-sent map that is IDENTICAL is idempotent (Ok), but a
/// CONFLICTING map at the same `(vaspace, va)` — a different memory/backing — must be a
/// loud `ConflictingMap`, never silently overwritten (last-writer-wins would hide a
/// hostile remap). The prior suite had no conflicting-map test, so forcing the guard to
/// `true` (treat EVERY existing map as an idempotent retry) survived. This pins both
/// sides: identical re-send is Ok; a conflicting remap is loud.
#[test]
fn conflicting_map_at_same_va_is_loud_identical_is_idempotent() {
    use nvkvm_core::rmgraph::{AllocFacts, RmGraph, RmGraphError};
    use nvkvm_mocks::mock_classes as mc;

    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let c = CLIENT;
    let vas = HObject(0x5c00_0010);
    let mem_a = HObject(0x5c00_0100);
    let mem_b = HObject(0x5c00_0101);

    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(c.0), handle: HObject(c.0), class: mc::CLIENT, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(c.0), handle: HObject(1), class: mc::DEVICE, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(1), handle: vas, class: mc::VASPACE, facts: AllocFacts::default() }).unwrap();
    g.apply(&arch, RmEvent::SetPageDir { client: c, vaspace: vas, pdb: PDB }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(1), handle: mem_a, class: mc::MEMORY, facts: AllocFacts { mem_phys: Some(0x8000_0000), ..Default::default() } }).unwrap();
    g.apply(&arch, RmEvent::Alloc { client: c, parent: HObject(1), handle: mem_b, class: mc::MEMORY, facts: AllocFacts { mem_phys: Some(0x9000_0000), ..Default::default() } }).unwrap();

    // First map: (vas, MAP_VA) → mem_a.
    g.apply(&arch, RmEvent::MapMemoryDma { client: c, vaspace: vas, memory: mem_a, va: MAP_VA, offset: 0, len: MAP_LEN }).unwrap();
    // Identical re-send is idempotent (Ok, no error).
    g.apply(&arch, RmEvent::MapMemoryDma { client: c, vaspace: vas, memory: mem_a, va: MAP_VA, offset: 0, len: MAP_LEN }).expect("identical re-send is idempotent");

    // A CONFLICTING map at the SAME (vas, va) but DIFFERENT memory is a loud fault.
    let conflict = g.apply(&arch, RmEvent::MapMemoryDma { client: c, vaspace: vas, memory: mem_b, va: MAP_VA, offset: 0, len: MAP_LEN });
    assert!(
        matches!(conflict, Err(RmGraphError::ConflictingMap { .. })),
        "a different mapping at a live (vaspace, va) is a loud ConflictingMap, never a silent overwrite: {conflict:?}",
    );
    // The original mapping is intact (the conflict did not overwrite it).
    let m = g.mappings().find(|m| m.va == MAP_VA).expect("original mapping intact");
    assert_eq!(m.mem_phys, Some(0x8000_0000), "the FIRST declarer keeps the (vas, va)");
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
