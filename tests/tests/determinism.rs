//! CATEGORY A — WHOLE-CORE PROTOCOL-NOT-TRACE DETERMINISM DIFFERENTIAL (decision #4).
//!
//! The design's load-bearing law (decision #4): **correctness = observable end-states
//! only** — the SAME set of declared protocol facts, applied in ANY order, with ANY
//! benign duplication, interleaved across processes any which way, must produce an
//! IDENTICAL observable end-state.
//!
//! ## What was already proven (and where the gap was)
//!
//! `rmgraph_order_independence.rs` permutes `RmEvent`s and asserts the derived
//! **`Boundaries`** (`by_pdb`/`by_vchid`/`Proc` grouping) are identical. `fuzz_rmgraph_
//! invariants.rs::a2` extends that to generated valid streams (forward vs reversed).
//! `security_invariants.rs::i1` proves cross-proc *address* resolution is injective
//! under an interleave. But NONE of these snapshots the **whole `Gpu` runtime's**
//! observable projection: the address resolutions *plus* engine-routing per channel
//! *plus* per-`Vas` backing state *plus* refcount liveness *plus* the `by_pdb`/`by_vchid`
//! maps *plus* per-proc grouping, all at once, asserted EQUAL across orderings. Each
//! prior test pins one facet; this file lifts the guarantee to the whole core's
//! observable end-state as a single canonical snapshot.
//!
//! ## The property
//!
//! [`CoreSnapshot`] is the deterministic, order-insensitive projection of everything an
//! adapter can observe off a [`Gpu`], **canonicalized on stable hardware/graph
//! identities** (`ProcAnchor`, `Pdb`, `VChid`, `NodeKey`) — never on the minted
//! `ProcId`/`ChanId` counters (which ARE order-dependent by design and would leak
//! arrival order into the comparison), and never through a `HashMap` (BTree/BTreeSet
//! throughout, so no iteration-order artifact makes the *test* flaky rather than the
//! *core* wrong). A divergence is therefore a genuine core order-dependence, not a
//! snapshot artifact.
//!
//! Three axes, each a `proptest` over generated multi-proc worlds carrying the #14
//! identical-VA / identical-handle hostile shape:
//! - **`p1`** — permutation + benign duplication: the whole-core snapshot of any
//!   permutation (with order-independent facts duplicated) equals the canonical one.
//!   A random-key sort of the concatenated multi-proc fact list is itself an arbitrary
//!   cross-proc interleave, so this already exercises the interleave axis.
//! - **`p_interleave`** — the interleave axis made explicit: two procs' fact-streams
//!   woven every which way still yield each proc's identical end-state.
//! - **`p_materialized`** — the fwd data-plane dimensions (host-published backing +
//!   armed mapped-fence completions) driven by a DETERMINISTIC post-apply pass, whose
//!   order-invariant projection (published-ness, resolvability, which channels arm,
//!   armed counts) must be identical across orderings. Host-allocation artifacts
//!   (exact host VA / carved GPA — legitimately order-dependent, they name *which*
//!   arena a proc happened to get) are masked; only the guest-observable invariants
//!   are compared.
//!
//! Frees are lifecycle-ordered (per the rmgraph module's stated rule), so the permuted
//! fact set is alloc/dup/set-page-dir/map/engine-obj only — free ordering is covered by
//! `weird_order_regressions.rs`.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use nvkvm_arch::ids::GpuId;
use std::collections::{BTreeMap, BTreeSet};

use nvkvm_arch::Aperture;
use nvkvm_arch::ids::{EngineKind, GpuVa, HClient, HObject, Pdb, VChid};
use nvkvm_completion::OsEventRef;
use nvkvm_core::ProcAnchor;
use nvkvm_core::gpa::GpaSpace;
use nvkvm_core::gpu::Gpu;
use nvkvm_core::rmgraph::{AllocFacts, NodeKey, RmEvent};
use nvkvm_fwd::{arm_fence, publish_backing, resolve};
use nvkvm_mocks::{MockArch, MockIsolateFactory, mock_classes as mc};
use nvkvm_tests::{Scenario, identical_handles};
use proptest::prelude::*;

// =================================================================================
// Fixtures — the generated multi-proc world (#14 identical-VA / identical-handle).
// =================================================================================

/// The shared guest VA every process maps (the #14 identical-VA shape — distinct PDBs
/// keep the per-`Vas` tables disjoint, which is exactly what the snapshot must prove).
const VA_MAP: GpuVa = GpuVa(0x2_0020_0000);
/// A distinct VA the deterministic data-plane pass publishes a host backing at (never
/// overlaps `VA_MAP`, so publish-over-a-live-RPC-mapping is never a spurious fault).
const VA_PUB: GpuVa = GpuVa(0x5_0000_0000);
/// The identical memory-object handle value every process reuses (#14 round 1) — keyed
/// by `(client, handle)`, so identical values across procs are correctly distinct.
const MEM_HANDLE: HObject = HObject(0x5c00_0100);
/// The identical NVENC engine-object handle value every process reuses.
const NVENC_OBJ: HObject = HObject(0x5c00_0200);

fn new_gpu() -> Gpu {
    let arch = Box::new(MockArch::new());
    let (factory, _rec) = MockIsolateFactory::new();
    // A generous window so arena exhaustion is never an accident of this test.
    let gpa = GpaSpace::new(0x1_0000_0000..0x1_0000_0000_0000, 0x1_0000_0000);
    Gpu::new(arch, Box::new(factory), gpa).expect("device realizes")
}

/// One benign, fully-formed process `k`'s declared facts (all order-independent — no
/// frees): a compute proc (client→device→VASpace+PDB→TSG→GR/CE channels) reusing the
/// #14 identical handle values, a backed memory object, a DMA map of it at the shared
/// `VA_MAP`, and optionally an NVENC engine object (refining the GR channel's engine to
/// `NvEnc`) and a UVM client that `DUP_OBJECT`s the compute VASpace (one Proc, two Vas).
///
/// Identities are pairwise-disjoint across `k` where the protocol requires it (PDBs,
/// vChids, backings, client ids) and deliberately IDENTICAL where #14 makes them so
/// (handle values, the mapped VA).
fn proc_facts(k: u32, with_uvm: bool, with_nvenc: bool) -> Vec<RmEvent> {
    let client = HClient(0xE000 + k);
    let pdb = Pdb(0x10_000 * (u64::from(k) + 1));
    let phys = 0x9_0000_0000 + u64::from(k) * 0x1_0000_0000;
    // vChids pairwise-distinct across procs (E0: a sane guest never collides vChids).
    let h = identical_handles((0x100 + 2 * k) as u16, (0x101 + 2 * k) as u16);

    let mut s = Scenario::new();
    let compute_vas = s.compute_process(client, pdb, h);
    s.memory(client, h.device, MEM_HANDLE, phys);
    s.map(client, h.vaspace, MEM_HANDLE, VA_MAP, 0x10000);
    if with_nvenc {
        // An NVENC engine object on the GR channel refines its EngineKind — the
        // engine-routing dimension the snapshot must prove order-independent.
        s.push(RmEvent::Alloc {
            client,
            parent: h.gr_channel,
            handle: NVENC_OBJ,
            class: mc::NVENC,
            facts: AllocFacts::default(),
        });
    }
    if with_uvm {
        let uvm_client = HClient(0xD000 + k);
        let uvm_pdb = Pdb(0x8000_0000 + u64::from(k) * 0x10_000);
        s.uvm_dup(
            uvm_client,
            HObject(0x9000_0000),
            HObject(0x9000_0001),
            HObject(0x9000_0010),
            uvm_pdb,
            HObject(0x9000_00ff),
            compute_vas,
        );
    }
    s.events
}

/// Concatenate `n` processes' facts into one canonical (scripted-order) fact list.
fn world(n: usize, uvm: &[bool], nvenc: &[bool]) -> Vec<RmEvent> {
    let mut events = Vec::new();
    for k in 0..n {
        events.extend(proc_facts(k as u32, uvm[k], nvenc[k]));
    }
    events
}

/// Strategy: a 1..=3 process world plus its per-proc UVM / NVENC toggles.
fn world_strategy() -> impl Strategy<Value = Vec<RmEvent>> {
    (1usize..=3).prop_flat_map(|n| {
        (
            prop::collection::vec(any::<bool>(), n),
            prop::collection::vec(any::<bool>(), n),
        )
            .prop_map(move |(uvm, nvenc)| world(n, &uvm, &nvenc))
    })
}

/// Permute `events` by generated keys (a *stable* sort — ties keep relative order, so
/// this is a genuine arbitrary interleave of all procs' facts) and benignly DUPLICATE
/// each event flagged in `dups` (an identical adjacent re-send — the retried-RPC shape,
/// which must be idempotent). Frees would break this (lifecycle-ordered); the fact set
/// has none.
fn permute_and_duplicate(events: &[RmEvent], keys: &[u64], dups: &[bool]) -> Vec<RmEvent> {
    let mut keyed: Vec<(u64, RmEvent)> = events
        .iter()
        .enumerate()
        .map(|(i, ev)| (keys.get(i).copied().unwrap_or(i as u64), *ev))
        .collect();
    keyed.sort_by_key(|(k, _)| *k);
    let mut out = Vec::with_capacity(events.len());
    for (i, (_, ev)) in keyed.iter().enumerate() {
        out.push(*ev);
        if dups.get(i).copied().unwrap_or(false) {
            out.push(*ev); // benign duplicate of an order-independent fact
        }
    }
    out
}

// =================================================================================
// The observable end-state snapshot — canonicalized on STABLE identities only.
// =================================================================================

/// Per-channel routing facts, keyed by the channel's stable [`NodeKey`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChannelObs {
    vchid: VChid,
    vas_pdb: Option<Pdb>,
    engine: EngineKind,
}

/// A resolved address-table entry, guest-observable. `phys_if_rpc` is `Some` only for
/// RPC-map bindings (the guest-declared backing, order-independent); a host-published
/// binding's `phys` is a carved-GPA artifact (order-dependent — masked to `None`), so
/// only its *published-ness* is compared.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedObs {
    phys_if_rpc: Option<u64>,
    aperture: Aperture,
    host_published: bool,
}

/// A live DMA mapping, guest-observable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MappingObs {
    pdb: Option<Pdb>,
    va: GpuVa,
    memory: NodeKey,
    mem_phys: Option<u64>,
}

/// ★ THE whole-core observable end-state. Every field is a `BTree*` keyed on a stable
/// hardware/graph identity, so equality is deterministic and free of any minted-id or
/// hash-iteration-order leakage.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CoreSnapshot {
    /// Per-proc grouping: anchor → its dup-connected client set.
    procs: BTreeMap<ProcAnchor, BTreeSet<HClient>>,
    /// Data-plane routing (canonicalized ProcId → anchor): (target, PDB) → owning
    /// anchor. Keyed on the GPU axis (MG-3) so the axis is part of the observable
    /// end-state and cannot leak arrival order.
    by_pdb: BTreeMap<(GpuId, Pdb), ProcAnchor>,
    /// Exec-plane routing: (target, vChid) → (owning anchor, channel node).
    by_vchid: BTreeMap<(GpuId, VChid), (ProcAnchor, NodeKey)>,
    /// Address plane: anchor → ((target, PDB) → VASpace origin node).
    vases: BTreeMap<ProcAnchor, BTreeMap<(GpuId, Pdb), NodeKey>>,
    /// Engine routing: anchor → (channel node → its routing facts).
    channels: BTreeMap<ProcAnchor, BTreeMap<NodeKey, ChannelObs>>,
    /// Address resolutions / backing state: every bound (target, PDB, VA) → what it
    /// resolves to.
    resolutions: BTreeMap<(GpuId, Pdb, GpuVa), ResolvedObs>,
    /// Refcount liveness: live resource origin → its live reference set.
    refs: BTreeMap<NodeKey, BTreeSet<NodeKey>>,
    /// The graph's live DMA mappings (the RPC populate source).
    mappings: BTreeSet<MappingObs>,
    /// Armed mapped-fence completions per proc.
    fences_armed: BTreeMap<ProcAnchor, usize>,
    /// Outstanding os-event completions per proc.
    completion_outstanding: BTreeMap<ProcAnchor, bool>,
}

/// Project a [`Gpu`] to its canonical observable end-state.
fn snapshot(gpu: &Gpu) -> CoreSnapshot {
    let g = &gpu.rmgraph;

    let mut procs = BTreeMap::new();
    let mut vases = BTreeMap::new();
    let mut channels = BTreeMap::new();
    let mut resolutions = BTreeMap::new();
    let mut fences_armed = BTreeMap::new();
    let mut completion_outstanding = BTreeMap::new();

    for p in gpu.procs.values() {
        let anchor = p.anchor;
        procs.insert(anchor, p.clients.clone());

        let mut vs = BTreeMap::new();
        for (&(gpu, pdb), vas) in &p.vases {
            vs.insert((gpu, pdb), vas.origin);
            for (va, _len, b) in vas.table.iter() {
                let host_published = b.host_va.is_some();
                resolutions.insert(
                    (gpu, pdb, GpuVa(va)),
                    ResolvedObs {
                        phys_if_rpc: (!host_published).then_some(b.phys),
                        aperture: b.aperture,
                        host_published,
                    },
                );
            }
        }
        vases.insert(anchor, vs);

        let mut ch = BTreeMap::new();
        for c in p.channels.values() {
            ch.insert(c.key, ChannelObs { vchid: c.vchid, vas_pdb: c.vas, engine: c.engine });
        }
        channels.insert(anchor, ch);

        fences_armed.insert(anchor, p.fences.armed_len());
        completion_outstanding.insert(anchor, p.completion.has_outstanding());
    }

    // Canonicalize routing off the minted ProcId onto the stable anchor.
    let mut by_pdb = BTreeMap::new();
    for (pdb, pid) in &gpu.by_pdb {
        by_pdb.insert(*pdb, gpu.procs.get(pid).expect("routed proc lives").anchor);
    }
    let mut by_vchid = BTreeMap::new();
    for (vchid, (pid, cid)) in &gpu.by_vchid {
        let p = gpu.procs.get(pid).expect("routed proc lives");
        by_vchid.insert(*vchid, (p.anchor, p.channels.get(cid).expect("routed chan lives").key));
    }

    let mut refs = BTreeMap::new();
    for n in g.nodes() {
        refs.insert(n.key, g.references(n.key).collect());
    }

    let mut mappings = BTreeSet::new();
    for m in g.mappings() {
        mappings.insert(MappingObs {
            pdb: m.pdb,
            va: m.va,
            memory: m.memory,
            mem_phys: m.mem_phys,
        });
    }

    CoreSnapshot {
        procs,
        by_pdb,
        by_vchid,
        vases,
        channels,
        resolutions,
        refs,
        mappings,
        fences_armed,
        completion_outstanding,
    }
}

/// Apply a fact list to a fresh device and snapshot the result. Every generated fact
/// is valid + order-tolerant, so each application must succeed in ANY order.
fn apply_and_snapshot(events: &[RmEvent]) -> CoreSnapshot {
    let mut gpu = new_gpu();
    for ev in events {
        gpu.apply(*ev).expect("a valid declared fact applies in any order");
    }
    snapshot(&gpu)
}

// =================================================================================
// p1 — permutation + benign duplication ⇒ identical whole-core end-state.
// =================================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// **p1.** For any generated multi-proc world, applying its declared facts in ANY
    /// permutation — with any subset benignly DUPLICATED (idempotent re-sends) — yields
    /// a whole-core observable end-state byte-identical to the canonical (scripted-order)
    /// application. The random-key sort is an arbitrary cross-proc interleave, so this
    /// also exercises the interleave axis. A divergence is a real core order-dependence.
    #[test]
    fn p1_whole_core_endstate_is_order_and_duplication_invariant(
        events in world_strategy(),
        keys in prop::collection::vec(any::<u64>(), 0..96),
        dups in prop::collection::vec(any::<bool>(), 0..96),
    ) {
        let reference = apply_and_snapshot(&events);
        let variant = permute_and_duplicate(&events, &keys, &dups);
        let got = apply_and_snapshot(&variant);
        prop_assert_eq!(
            got, reference,
            "whole-core observable end-state diverged under permutation/duplication"
        );
    }
}

// =================================================================================
// p_interleave — the interleave axis, made explicit (two procs woven every which way).
// =================================================================================

/// Take-b-when-true weave of two fact-streams, so proc B's facts land WHILE proc A's
/// are mid-setup (the most adversarial cross-proc timing).
fn weave(a: &[RmEvent], b: &[RmEvent], take_b: &[bool]) -> Vec<RmEvent> {
    let (mut ai, mut bi) = (0usize, 0usize);
    let mut out = Vec::with_capacity(a.len() + b.len());
    for &tb in take_b {
        if tb && bi < b.len() {
            out.push(b[bi]);
            bi += 1;
        } else if ai < a.len() {
            out.push(a[ai]);
            ai += 1;
        } else if bi < b.len() {
            out.push(b[bi]);
            bi += 1;
        }
    }
    out.extend_from_slice(&a[ai..]);
    out.extend_from_slice(&b[bi..]);
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// **p_interleave.** Two processes with the #14 identical-VA / identical-handle
    /// shape, their fact-streams WOVEN in an arbitrary order, reach the same whole-core
    /// end-state as the un-interleaved (A-then-B) application. Order-independence ACROSS
    /// processes, not merely within one.
    #[test]
    fn p_interleave_two_proc_streams_reach_identical_endstate(
        a_uvm in any::<bool>(),
        a_nvenc in any::<bool>(),
        b_uvm in any::<bool>(),
        b_nvenc in any::<bool>(),
        take_b in prop::collection::vec(any::<bool>(), 0..48),
    ) {
        let a = proc_facts(0, a_uvm, a_nvenc);
        let b = proc_facts(1, b_uvm, b_nvenc);

        // Reference: A fully, then B fully.
        let mut sequential = a.clone();
        sequential.extend_from_slice(&b);
        let reference = apply_and_snapshot(&sequential);

        // Woven: B's facts interleaved into A's at arbitrary points.
        let woven = weave(&a, &b, &take_b);
        let got = apply_and_snapshot(&woven);

        prop_assert_eq!(got, reference, "interleaving two procs changed the end-state");
    }
}

// =================================================================================
// p_materialized — the fwd data-plane dimensions (host-published backing + armed
// mapped-fence completions), driven deterministically post-apply.
// =================================================================================

/// The order-invariant projection of a deterministic data-plane materialization pass.
/// Host artifacts (exact host VA / carved GPA — which name *which* arena a proc got,
/// legitimately order-dependent) are NOT here; only guest-observable invariants are.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DataPlaneProjection {
    /// (anchor, target, PDB) → publish_backing succeeded.
    published: BTreeMap<(ProcAnchor, GpuId, Pdb), bool>,
    /// (anchor, target, PDB) → the published VA resolves AND is host-published.
    pub_resolves: BTreeMap<(ProcAnchor, GpuId, Pdb), bool>,
    /// (anchor, channel node) → an NVENC channel armed its mapped fence successfully.
    arms: BTreeMap<(ProcAnchor, NodeKey), bool>,
    /// anchor → number of mapped fences left armed.
    armed_counts: BTreeMap<ProcAnchor, usize>,
}

/// Deterministically materialize the data plane on a post-apply device: publish a host
/// backing at `VA_PUB` in every `Vas`, then arm a mapped fence on every NVENC channel.
/// Both are pure functions of the (order-independent) graph state, so their invariant
/// projection must be identical across orderings — if engine routing or per-`Vas`
/// backing were order-dependent, this would catch it.
fn materialize(gpu: &mut Gpu) -> DataPlaneProjection {
    // (ProcId, (GpuId, Pdb), anchor) for every live Vas — the target is part of the
    // address identity (MG-3).
    let vases: Vec<(nvkvm_core::ProcId, (GpuId, Pdb), ProcAnchor)> = gpu
        .procs
        .iter()
        .flat_map(|(pid, p)| p.vases.keys().map(move |k| (*pid, *k, p.anchor)))
        .collect();

    let mut published = BTreeMap::new();
    for (pid, (gpu_t, pdb), anchor) in &vases {
        let r =
            publish_backing(gpu.procs.get_mut(pid).expect("live"), *gpu_t, *pdb, VA_PUB, 0x1000);
        published.insert((*anchor, *gpu_t, *pdb), r.is_ok());
    }
    let mut pub_resolves = BTreeMap::new();
    for (_pid, (gpu_t, pdb), anchor) in &vases {
        let ok = matches!(resolve(gpu, *gpu_t, *pdb, VA_PUB), Ok((b, _)) if b.host_va.is_some());
        pub_resolves.insert((*anchor, *gpu_t, *pdb), ok);
    }

    let chans: Vec<(nvkvm_core::ProcId, nvkvm_core::ChanId)> =
        gpu.by_vchid.values().copied().collect();
    let mut arms = BTreeMap::new();
    for (pid, cid) in &chans {
        let (anchor, key, is_nvenc) = {
            let p = gpu.procs.get(pid).expect("live");
            let c = p.channels.get(cid).expect("live");
            (p.anchor, c.key, c.engine == EngineKind::NvEnc)
        };
        if is_nvenc {
            let ev = OsEventRef(0xF00 + u64::from(key.handle.0));
            let r = arm_fence(gpu, *pid, *cid, VA_PUB, 0, 0x100, ev);
            arms.insert((anchor, key), r.is_ok());
        }
    }

    let mut armed_counts = BTreeMap::new();
    for p in gpu.procs.values() {
        armed_counts.insert(p.anchor, p.fences.armed_len());
    }

    DataPlaneProjection { published, pub_resolves, arms, armed_counts }
}

/// Apply a fact list, then run the deterministic data-plane pass, returning its
/// invariant projection.
fn apply_and_materialize(events: &[RmEvent]) -> DataPlaneProjection {
    let mut gpu = new_gpu();
    for ev in events {
        gpu.apply(*ev).expect("a valid declared fact applies in any order");
    }
    materialize(&mut gpu)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(150))]

    /// **p_materialized.** With the data plane materialized deterministically after
    /// apply, the guest-observable projection of the *backing* (which VAs publish and
    /// resolve host-published) and the *armed mapped-fence completions* (which channels
    /// arm, how many stay armed) is identical whether the facts were applied in canonical
    /// order or in a random permutation. The order-dependent host artifacts (exact host
    /// VA / carved GPA) are masked; only the invariants are compared.
    #[test]
    fn p_materialized_dataplane_projection_is_order_invariant(
        events in world_strategy(),
        keys in prop::collection::vec(any::<u64>(), 0..96),
        dups in prop::collection::vec(any::<bool>(), 0..96),
    ) {
        let reference = apply_and_materialize(&events);
        let variant = permute_and_duplicate(&events, &keys, &dups);
        let got = apply_and_materialize(&variant);
        prop_assert_eq!(
            got, reference,
            "materialized data-plane projection diverged under permutation"
        );
    }
}

// =================================================================================
// A pinned, non-random sanity anchor (documents the property on a concrete world).
// =================================================================================

/// Two full procs (one with UVM + NVENC), applied in scripted order vs fully reversed
/// vs an even/odd interleave: all three produce the identical whole-core snapshot. A
/// human-readable witness of the property the proptests search.
#[test]
fn determinism_reference_world_three_orders_agree() {
    let events = world(2, &[true, false], &[true, false]);

    let scripted = apply_and_snapshot(&events);

    let mut reversed = events.clone();
    reversed.reverse();
    let backward = apply_and_snapshot(&reversed);

    let mut interleaved: Vec<RmEvent> = events.iter().step_by(2).copied().collect();
    interleaved.extend(events.iter().skip(1).step_by(2).copied());
    let woven = apply_and_snapshot(&interleaved);

    assert_eq!(scripted, backward, "reversed order diverged");
    assert_eq!(scripted, woven, "even/odd interleave diverged");

    // And the snapshot is non-trivial: it actually captured routed procs + resolutions.
    assert_eq!(scripted.procs.len(), 2, "two procs projected");
    assert!(!scripted.by_pdb.is_empty(), "PDBs routed");
    assert!(!scripted.resolutions.is_empty(), "address bindings captured");
}
