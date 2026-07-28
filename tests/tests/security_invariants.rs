//! # SECURITY INVARIANTS — isolation properties over GENERATED hostile op-sequences
//! (independent core red-team, decision #18C)
//!
//! This suite is the *searched-coverage* half of the security pass. Where
//! `security_boundary.rs` (#18A) pins named adversarial **examples** and
//! `fuzz_rmgraph_invariants.rs` (A1–A4) fuzzes the graph's never-panic + structural +
//! refcount properties, this file states the four catastrophic isolation invariants as
//! **proptest properties** — each searched over generated hostile sequences, several
//! backed by a reference-model differential oracle so a divergence *is* the
//! counterexample. It does not re-derive the example suites; it generalizes the
//! properties they only spot-check:
//!
//! - **I1 cross-proc address isolation** — over any K-process hostile world (identical
//!   guest VAs, distinct PDBs, distinct backings, arbitrary interleave + junk), every
//!   process's VA resolves to ITS OWN backing and never another's (injective oracle).
//! - **I2 completion integrity / forgery** — a mapped-fence fires only from a genuinely
//!   armed source, exactly once, at/after target, honoring the #12 backwards-jump guard;
//!   a forged/backwards value never forges a completion, and never cross-signals another
//!   armed key. Differential vs an independent reference fence model.
//! - **I3 refcount soundness (deep trees)** — over generated deep parent-trees with
//!   cross-client dups and interior/root frees: no panic, loud `FreeUnknown` for
//!   double/unknown frees, resolution stays consistent, dup keeps a resource alive
//!   across the origin's free, and freeing every root drains the graph to EMPTY
//!   (no leak). Extends A4 (which oracles only self-parented roots) to deep trees.
//! - **I4 DoS containment** — a benign bystander applied first, then an arbitrary
//!   hostile flood/collision stream: the device ALWAYS still projects (no wedge), the
//!   bystander's routing + resolution stay intact, and the graph is still usable after
//!   the storm. Generalizes #18A's atomic-apply over generated sequences.
//!
//! ## ⚠ Scope (held): LOGICAL isolation only
//! The core is pure logic — no pointers, no memory, no syscalls — so memory-safety /
//! OOB / UAF are *born at L1/L2* (mmap/isolate/trap) and are explicitly OUT OF SCOPE
//! here (deferred to the post-L2 audit; asserting them against pure logic would be
//! theater). Every property below is a *logical* isolation invariant.
//!
//! ## Phase 3 / 4 additions
//! `p3_*` — the confused-deputy audit made structural: every site that turns a guest
//! handle into a typed object routes through the ONE typed primitive
//! (`RmGraph::origin_of_kind`), so resolving to the wrong `ObjectKind` is a single
//! centrally-enforced check. `p4_*` — the regression for the confused-deputy this pass
//! fixed in the core (a `MAP_MEMORY_DMA` naming a NON-memory object as its backing is
//! now a loud unbacked fault, never a silent bind — the two memory→phys resolvers can
//! no longer disagree).

#![allow(clippy::unusual_byte_groupings)]

use kayfabe_arch::ObjectKind;
use kayfabe_arch::ids::GpuId;
use kayfabe_arch::ids::{ClassId, GpuVa, HClient, HObject, Pdb, VChid};
use kayfabe_completion::{CompletionError, FenceArms, MAX_FENCE_JUMP, OsEventRef};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::{Gpu, GpuError};
use kayfabe_core::project::{NO_CONDEMNED, project};
use kayfabe_core::rmgraph::{AllocFacts, NodeKey, RmEvent, RmGraph};
use kayfabe_fwd::resolve;
use kayfabe_mocks::{MockArch, MockIsolateFactory, mock_classes as mc};
use kayfabe_tests::{Guarded, Scenario, identical_handles};
use proptest::prelude::*;

// =================================================================================
// Shared fixtures
// =================================================================================

fn new_gpu() -> Guarded<Gpu> {
    let arch = Box::new(MockArch::new());
    let (factory, rec) = MockIsolateFactory::new();
    // A generous window so exhaustion is a deliberate act, not an accident.
    let gpa = GpaSpace::new(0x1_0000_0000..0x1_0000_0000_0000, 0x1_0000_0000);
    Guarded::new(
        "security_invariants::new_gpu",
        Gpu::new(arch, Box::new(factory), gpa).expect("device realizes"),
        rec,
    )
}

/// Take-b-when-true weave of two streams (the #18A `interleave` shape), so a hostile
/// stream acts WHILE the benign one is mid-setup — the most adversarial timing.
fn weave_streams<T: Copy>(b: &[T], a: &[T], weave: &[bool]) -> Vec<(bool, T)> {
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

// =================================================================================
// I1 — CROSS-PROC ADDRESS ISOLATION (injective differential oracle)
// =================================================================================

/// The shared guest VA every process maps (the #14 identical-VA shape).
const I1_VA: GpuVa = GpuVa(0x2_0020_0000);
/// The identical memory-object handle value every process reuses (#14 round 1).
const I1_MEM: HObject = HObject(0x5c00_0100);

/// Build one benign compute process `k`'s full event list: a compute proc with
/// identical handles + a mapped memory object at the shared VA, backed by `phys`.
fn i1_proc_events(k: u32) -> (HClient, Pdb, u64, Vec<RmEvent>) {
    let client = HClient(0xE000 + k);
    // PDBs 0x1_0000 * (k+1): pairwise distinct, disjoint from junk's 0x9300_0000 lane.
    let pdb = Pdb(0x1_0000 * u64::from(k + 1));
    // Backings pairwise distinct (the injectivity the oracle checks).
    let phys = 0x9_0000_0000 + u64::from(k) * 0x1_0000_0000;
    // vChids pairwise distinct across procs (E0: no vChid collisions on a sane guest).
    let h = identical_handles((0x100 + 2 * k) as u16, (0x101 + 2 * k) as u16);
    let mut s = Scenario::new();
    s.compute_process(client, pdb, h);
    s.memory(client, h.device, I1_MEM, phys);
    s.map(client, h.vaspace, I1_MEM, I1_VA, 0x10000);
    (client, pdb, phys, s.events)
}

/// A hostile junk event confined to the junk client lane (`0x9100..0x9104`) with
/// PDB/vChid/handle lanes provably DISJOINT from every I1 process, so it can neither
/// squat an I1 identity nor dup-merge into an I1 process — it can only earn its own
/// loud refusal. (I1 isolates the *address* property; collision-DoS is I4's job.)
fn i1_junk_event() -> impl Strategy<Value = RmEvent> {
    let jc = || (0u32..4).prop_map(|n| HClient(0x9100 + n));
    let jh = || (0u32..6).prop_map(|n| HObject(0x9200_0000 + n));
    let jclass = || {
        prop_oneof![
            Just(mc::CLIENT),
            Just(mc::DEVICE),
            Just(mc::VASPACE),
            Just(mc::TSG),
            Just(mc::CHANNEL_GR),
            Just(mc::MEMORY),
            Just(ClassId(0xDEAD_BEEF)), // an unknown class
        ]
    };
    prop_oneof![
        (jc(), jh(), jh(), jclass()).prop_map(|(client, parent, handle, class)| RmEvent::Alloc {
            client,
            parent,
            handle,
            class,
            facts: AllocFacts {
                mem_phys: Some(0x9_0000_0000),
                ..Default::default()
            },
        }),
        (jc(), jh(), jc(), jh()).prop_map(|(sc, sh, dc, dh)| RmEvent::Dup {
            src: NodeKey::new(sc, sh),
            dst: NodeKey::new(dc, dh),
        }),
        // SetPageDir onto a junk PDB lane (0x9300_0000+), never an I1 PDB.
        (jc(), jh(), 0u64..0x100).prop_map(|(client, vaspace, p)| RmEvent::SetPageDir {
            client,
            vaspace,
            pdb: Pdb(0x9300_0000 + p),
        }),
        (jc(), jh()).prop_map(|(client, handle)| RmEvent::Free { client, handle }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(160))]

    /// **I1.** In any K-process hostile world (2..=4 procs) with IDENTICAL guest VAs,
    /// IDENTICAL handle values, but DISTINCT PDBs backed by DISTINCT physical
    /// addresses — applied in an ARBITRARY interleave with junk hostile events — every
    /// process's VA resolves to ITS OWN backing and NEVER to another process's. The
    /// injective PDB→phys map is the reference oracle: a single crossed resolution is a
    /// counterexample.
    #[test]
    fn i1_no_proc_va_resolves_to_another_procs_backing(
        k in 2u32..=4,
        perm in prop::collection::vec(any::<u64>(), 0..64),
        junk in prop::collection::vec(i1_junk_event(), 0..10),
        weave in prop::collection::vec(any::<bool>(), 0..80),
    ) {
        // Build the K procs; collect the injective (pdb -> phys) oracle.
        let mut legit: Vec<RmEvent> = Vec::new();
        let mut oracle: Vec<(Pdb, u64)> = Vec::new();
        for j in 0..k {
            let (_c, pdb, phys, evs) = i1_proc_events(j);
            oracle.push((pdb, phys));
            legit.extend(evs);
        }

        // Permute the legit events (all alloc/setpagedir/map — order-tolerant, so any
        // permutation still applies cleanly): pair with generated keys and sort.
        // ★ §12.38 — then LEGALIZE: an event may not name a client namespace before its
        // `NV01_ROOT` declares it (RM: `NV_ERR_INVALID_CLIENT`), so the permutation is
        // taken over the orderings the protocol can actually produce. The pass is stable,
        // so the adversarial interleave the random-key sort produces survives intact.
        let mut keyed: Vec<(u64, RmEvent)> = legit
            .iter()
            .enumerate()
            .map(|(i, ev)| (*perm.get(i).unwrap_or(&(i as u64)), *ev))
            .collect();
        keyed.sort_by_key(|(key, _)| *key);
        let permuted: Vec<RmEvent> =
            kayfabe_tests::legal_order(&keyed.into_iter().map(|(_, ev)| ev).collect::<Vec<_>>());

        // Interleave permuted-legit (b) with junk (a) at adversarial timing.
        let mut gpu = new_gpu();
        for (is_legit, ev) in weave_streams(&permuted, &junk, &weave) {
            let r = gpu.apply(ev);
            if is_legit {
                // Junk is provably disjoint, so no benign event may ever be refused.
                prop_assert!(r.is_ok(), "a benign I1 event was refused under junk: {r:?}");
            }
        }

        // Device still projects (hostile junk left it consistent).
        prop_assert!(project(&gpu.spine.rmgraph, gpu.spine.arch(), &NO_CONDEMNED).is_ok());

        // THE property: each PDB resolves the shared VA to ITS OWN phys, never another's.
        for &(pdb, phys) in &oracle {
            let got = resolve(&gpu, GpuId::ZERO, pdb, I1_VA).map(|(b, _)| b.phys);
            prop_assert_eq!(got, Ok(phys), "PDB {:?} resolved the shared VA to the wrong backing", pdb);
        }
        // And injectivity, checked directly: no PDB resolves to a DIFFERENT proc's phys.
        for &(pdb, _) in &oracle {
            let got = resolve(&gpu, GpuId::ZERO, pdb, I1_VA).map(|(b, _)| b.phys).unwrap();
            for &(other_pdb, other_phys) in &oracle {
                if other_pdb != pdb {
                    prop_assert_ne!(got, other_phys, "cross-proc backing leak: {:?} -> another proc's phys", pdb);
                }
            }
        }
    }
}

// =================================================================================
// I2 — COMPLETION INTEGRITY / FORGERY (fence differential oracle)
// =================================================================================

/// An independent reference model of one mapped-fence arm — the executable spec
/// `FenceArms` must match: wrap-correct monotone accumulation with the #12
/// backwards-jump guard. `observe` returns `Ok(true)` on fire, `Ok(false)` if inert,
/// `Err(())` if the step exceeds `MAX_FENCE_JUMP` (refused, state unchanged).
struct RefFence {
    to_go: u64,
    last: u32,
    fired: bool,
}

impl RefFence {
    fn arm(current: u32, target: u32) -> (Self, bool) {
        let to_go = u64::from(target.wrapping_sub(current));
        // At-target arms fire immediately and nothing is left armed.
        (
            RefFence {
                to_go,
                last: current,
                fired: to_go == 0,
            },
            to_go == 0,
        )
    }
    fn observe(&mut self, value: u32) -> Result<bool, ()> {
        if self.fired {
            return Ok(false); // consumed / at-target: further writes are inert
        }
        let step = u64::from(value.wrapping_sub(self.last));
        if step > MAX_FENCE_JUMP {
            return Err(()); // the #12 guard: a backwards/absurd write, state unchanged
        }
        self.last = value;
        self.to_go = self.to_go.saturating_sub(step);
        if self.to_go == 0 {
            self.fired = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// **I2 (integrity).** `FenceArms` matches the reference fence model step-for-step
    /// over any generated arm target + observation stream: it fires exactly once,
    /// at/after target, and a forged/backwards write (step > `MAX_FENCE_JUMP`) is a
    /// loud refusal that never forges a completion and never advances the arm. A
    /// divergence from the oracle IS the counterexample.
    #[test]
    fn i2_fence_matches_reference_model_never_forges_a_completion(
        current in any::<u32>(),
        target in any::<u32>(),
        values in prop::collection::vec(any::<u32>(), 0..40),
    ) {
        let key = (0x0340_1000u64, 0x0002_0050_0000u64);
        let event = OsEventRef(0xE0E0);
        let mut arms = FenceArms::new();
        let (mut refm, ref_fired_now) = RefFence::arm(current, target);

        let real_arm = arms.arm(key, current, target, event);
        // Arm-time firing agrees.
        match (real_arm, ref_fired_now) {
            (Ok(Some(e)), true) => prop_assert_eq!(e, event),
            (Ok(None), false) => {}
            other => prop_assert!(false, "arm disagreed with oracle: {:?}", other),
        }

        let mut real_fired = false;
        for v in values {
            let real = arms.observe(key, v);
            let refr = refm.observe(v);
            match (real, refr) {
                (Ok(Some(e)), Ok(true)) => {
                    prop_assert_eq!(e, event, "fired the wrong event (forgery)");
                    prop_assert!(!real_fired, "fence fired more than once");
                    real_fired = true;
                }
                (Ok(None), Ok(false)) => {}
                (Err(CompletionError::FenceJump { .. }), Err(())) => {
                    // The #12 guard fired on both — a forged/backwards write refused.
                }
                other => prop_assert!(false, "FenceArms diverged from oracle: {:?}", other),
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// **I2 (no cross-signal).** Two fences armed on DISTINCT keys with DISTINCT events:
    /// driving one key to its target fires ONLY its own event and NEVER the other's arm
    /// — a completion cannot cross-signal a different armed source. The second arm stays
    /// live until its own genuine target.
    #[test]
    fn i2_completion_never_cross_signals_another_armed_key(
        base_a in 0u32..1000,
        base_b in 0u32..1000,
        steps in prop::collection::vec(1u32..500, 1..8),
    ) {
        let ka = (0xAAAA_0000u64, 0x1000u64);
        let kb = (0xBBBB_0000u64, 0x2000u64);
        let ea = OsEventRef(0xAA);
        let eb = OsEventRef(0xBB);
        let mut f = FenceArms::new();

        let target_a = base_a.wrapping_add(steps.iter().sum::<u32>().min(MAX_FENCE_JUMP as u32));
        prop_assume!(target_a != base_a);
        f.arm(ka, base_a, target_a, ea).unwrap();
        // b armed far from a's value range, its own target never reached by a's writes.
        f.arm(kb, base_b, base_b.wrapping_add(1), eb).unwrap();

        // Walk key A toward its target in bounded steps; b must never fire.
        let mut cur = base_a;
        let mut fired_a = None;
        for s in &steps {
            cur = cur.wrapping_add(*s);
            if let Ok(Some(ev)) = f.observe(ka, cur) {
                prop_assert_eq!(ev, ea, "A's fire returned a foreign event");
                fired_a = Some(ev);
                break;
            }
        }
        // Whatever happened to A, an observe on A's value against key B is inert unless
        // B genuinely reached its own target — never a cross-signal from A's activity.
        // (b armed at base_b -> base_b+1; only a write AT/after base_b+1 on kb fires it.)
        let _ = fired_a;
        // Feeding A's current value to B fires B only if it genuinely crosses B's target
        // (reference check), never merely because A fired.
        let mut refb = RefFence::arm(base_b, base_b.wrapping_add(1)).0;
        let want_b = refb.observe(cur).unwrap_or(false);
        let got_b = matches!(f.observe(kb, cur), Ok(Some(_)));
        prop_assert_eq!(got_b, want_b, "B fired iff it genuinely reached its own target — no cross-signal");
    }
}

// =================================================================================
// I3 — REFCOUNT SOUNDNESS over DEEP parent-trees + cross-client dups + frees
// (extends A4, which oracles only self-parented roots)
// =================================================================================

/// A generated deep-tree op: an Alloc parented on an earlier handle, a cross-client
/// Dup, or a Free. Indices reference a growing handle table so the tree is genuinely
/// deep and the frees hit interior nodes, not just roots.
#[derive(Debug, Clone)]
enum TreeOp {
    Alloc { parent_idx: usize, class: ClassId },
    Dup { src_idx: usize, into_c2: bool },
    Free { idx: usize },
}

fn tree_op() -> impl Strategy<Value = TreeOp> {
    let class = || {
        prop_oneof![
            Just(mc::VASPACE),
            Just(mc::TSG),
            Just(mc::CTXSHARE),
            Just(mc::CHANNEL_GR),
            Just(mc::MEMORY),
            Just(mc::EVENT),
        ]
    };
    prop_oneof![
        (0usize..24, class()).prop_map(|(parent_idx, class)| TreeOp::Alloc { parent_idx, class }),
        (0usize..24, any::<bool>()).prop_map(|(src_idx, into_c2)| TreeOp::Dup { src_idx, into_c2 }),
        (0usize..24).prop_map(|idx| TreeOp::Free { idx }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// **I3.** Over a generated deep parent-tree (client C1) with cross-client dups
    /// (into C2) and interior/root frees: `RmGraph::apply` never panics; a Free of an
    /// unknown/already-freed handle is a loud `FreeUnknown` (never a panic, never a
    /// silent no-op that leaks); every live node keeps a non-empty reference set and
    /// resolves consistently; a dup keeps its resource alive across the origin's free;
    /// and freeing every client root drains the graph to EMPTY — no free-while-
    /// referenced, no double-free, no permanent leak.
    #[test]
    fn i3_deep_tree_refcount_is_sound_no_leak_no_double_free(
        ops in prop::collection::vec(tree_op(), 0..40),
    ) {
        let arch = MockArch::new();
        let mut g = RmGraph::new();
        const C1: HClient = HClient(0xC1);
        const C2: HClient = HClient(0xC2);

        // Seed each client with its root; handle table tracks (client, handle) nodes.
        let c1_root = HObject(0xC1_0000);
        let c2_root = HObject(0xC2_0000);
        g.apply(&arch, RmEvent::Alloc { client: C1, parent: c1_root, handle: c1_root, class: mc::CLIENT, facts: kayfabe_tests::user_client(C1) }).unwrap();
        g.apply(&arch, RmEvent::Alloc { client: C2, parent: c2_root, handle: c2_root, class: mc::CLIENT, facts: kayfabe_tests::user_client(C2) }).unwrap();
        let mut nodes: Vec<NodeKey> = vec![NodeKey::new(C1, c1_root), NodeKey::new(C2, c2_root)];
        let mut next_handle = 1u32;

        for op in ops {
            match op {
                TreeOp::Alloc { parent_idx, class } => {
                    let Some(parent) = nodes.get(parent_idx % nodes.len()).copied() else { continue };
                    let handle = HObject(0x1_0000 + next_handle);
                    next_handle += 1;
                    let key = NodeKey::new(parent.client, handle);
                    // Alloc under a same-namespace parent — a genuinely deep tree.
                    let r = g.apply(&arch, RmEvent::Alloc { client: parent.client, parent: parent.handle, handle, class, facts: AllocFacts::default() });
                    if r.is_ok() {
                        nodes.push(key);
                    }
                }
                TreeOp::Dup { src_idx, into_c2 } => {
                    let Some(src) = nodes.get(src_idx % nodes.len()).copied() else { continue };
                    let dst_c = if into_c2 { C2 } else { C1 };
                    let handle = HObject(0x2_0000 + next_handle);
                    next_handle += 1;
                    let dst = NodeKey::new(dst_c, handle);
                    let r = g.apply(&arch, RmEvent::Dup { src, dst });
                    if r.is_ok() {
                        nodes.push(dst);
                    }
                }
                TreeOp::Free { idx } => {
                    let Some(target) = nodes.get(idx % nodes.len()).copied() else { continue };
                    // The core's Free knows a handle iff it references a live resource OR
                    // names a *parked* dup dst (resolvable later). A parked dst has no
                    // resolvable origin yet but still appears in `dups()`, so the loud-set
                    // is the union — anything outside it must be a loud FreeUnknown.
                    let known = g.origin_of(target).is_some()
                        || g.dups().any(|(d, _)| d == target);
                    let r = g.apply(&arch, RmEvent::Free { client: target.client, handle: target.handle });
                    // A free of a live/parked handle succeeds; a truly unknown one is a
                    // LOUD fault. Either way: never a panic (we reached here).
                    if !known {
                        prop_assert!(r.is_err(), "free of a truly-unknown handle must be loud");
                    }
                }
            }

            // Invariant after EVERY op: every live resource keeps a non-empty reference
            // set (the liveness-⟺-referenced invariant, via the public API), and EVERY
            // one of those references resolves back to this exact origin — no
            // dangling/half-freed resource, no reference pointing at the wrong resource.
            // (Note: `origin_of(n.key)` itself may be `None` — a dup-kept resource's
            // ORIGIN handle can be freed while an alias keeps the resource alive; the
            // stable origin key still labels it. The load-bearing check is that every
            // live reference resolves to it.)
            for n in g.nodes() {
                let refs: Vec<NodeKey> = g.references(n.key).collect();
                prop_assert!(!refs.is_empty(), "a live resource has no references (would be a leak/UAF shape)");
                for r in refs {
                    prop_assert_eq!(
                        g.origin_of(r).map(|x| x.key),
                        Some(n.key),
                        "a live reference does not resolve to its origin resource"
                    );
                }
            }
        }

        // LEAK-FREEDOM: freeing every live HANDLE drains the graph to EMPTY. Every
        // resource is kept alive by ≥1 handle (no maps in this test), so a resource
        // that survives freeing all handles would be a permanent leak (the dual of
        // free-while-referenced). We free by handle (not just the two roots) because a
        // hostile stream can orphan a subtree by freeing its root early then allocating
        // into the dead namespace — which real RM forbids but the order-tolerant graph
        // permits; those orphans must still be reapable. Bounded loop = subtree ordering.
        let _ = (c1_root, c2_root);
        for _ in 0..64 {
            if g.nodes().count() == 0 {
                break;
            }
            let live: Vec<NodeKey> = g.nodes().flat_map(|n| g.references(n.key)).collect();
            for k in live {
                let _ = g.apply(&arch, RmEvent::Free { client: k.client, handle: k.handle });
            }
        }
        prop_assert_eq!(g.nodes().count(), 0, "graph leaked resources: undrainable after freeing all handles");
        prop_assert_eq!(g.mappings().count(), 0, "graph leaked mappings after freeing all handles");
    }
}

// =================================================================================
// I4 — DoS CONTAINMENT (generalized #18A over generated flood/collision streams)
// =================================================================================

/// The benign bystander C: a full compute proc + a mapped memory object, applied
/// FIRST (so it owns its PDB/vChid; first-declarer-wins protects it from any later
/// squat). Its routing + resolution are what must survive the storm untouched.
const BY_CLIENT: HClient = HClient(0x0BEEF);
const BY_PDB: Pdb = Pdb(0x000B_0000);
const BY_MEM: HObject = HObject(0x5c00_0100);
const BY_PHYS: u64 = 0x7_0000_0000;
const BY_VA: GpuVa = GpuVa(0x2_0020_0000);
const BY_GR_VCHID: u16 = 0x5A;
const BY_CE_VCHID: u16 = 0x5B;

fn bystander_events() -> Vec<RmEvent> {
    let mut s = Scenario::new();
    let h = identical_handles(BY_GR_VCHID, BY_CE_VCHID);
    s.compute_process(BY_CLIENT, BY_PDB, h);
    s.memory(BY_CLIENT, h.device, BY_MEM, BY_PHYS);
    s.map(BY_CLIENT, h.vaspace, BY_MEM, BY_VA, 0x10000);
    s.events
}

/// A hostile DoS event: floods, PDB/vChid collisions with the bystander (a squat, which
/// — bystander-first — must be refused, not honored), junk frees, dup cycles. The
/// attacker MAY target the bystander's exact HW identities; containment must hold.
fn i4_hostile_event() -> impl Strategy<Value = RmEvent> {
    let ac = || (0u32..3).prop_map(|n| HClient(0xA000 + n));
    let ah = || (0u32..8).prop_map(|n| HObject(0x5c00_0000 + n));
    // The attacker may pick the bystander's PDB (a squat) or its own lane.
    let apdb = || prop_oneof![Just(BY_PDB), (0u64..8).prop_map(|n| Pdb(0xA0_0000 + n))];
    // The attacker may pick the bystander's vChid (via userd flags) or its own.
    let aflags = || {
        prop_oneof![
            Just(MockArch::userd_flags_for(VChid(BY_GR_VCHID))),
            (0u32..8).prop_map(|n| MockArch::userd_flags_for(VChid(0x900 + n as u16))),
        ]
    };
    let aclass = || {
        prop_oneof![
            Just(mc::CLIENT),
            Just(mc::DEVICE),
            Just(mc::VASPACE),
            Just(mc::TSG),
            Just(mc::CHANNEL_GR),
            Just(mc::MEMORY),
        ]
    };
    prop_oneof![
        (ac(), ah(), ah(), aclass(), aflags()).prop_map(
            |(client, parent, handle, class, userd_flags)| {
                RmEvent::Alloc {
                    client,
                    parent,
                    handle,
                    class,
                    facts: AllocFacts {
                        userd_flags,
                        mem_phys: Some(0xDEAD_0000),
                        ..Default::default()
                    },
                }
            }
        ),
        (ac(), ah(), apdb()).prop_map(|(client, vaspace, pdb)| RmEvent::SetPageDir {
            client,
            vaspace,
            pdb
        }),
        (ac(), ah(), ac(), ah()).prop_map(|(sc, sh, dc, dh)| RmEvent::Dup {
            src: NodeKey::new(sc, sh),
            dst: NodeKey::new(dc, dh)
        }),
        (ac(), ah()).prop_map(|(client, handle)| RmEvent::Free { client, handle }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// **I4.** A benign bystander is applied first; then an arbitrary hostile
    /// flood/collision stream (which may squat the bystander's exact PDB/vChid). After
    /// the storm: the device STILL PROJECTS (no global wedge), the bystander's PDB still
    /// routes and its VA still resolves to ITS backing (no third-party corruption), and
    /// the graph is still USABLE — a fresh benign op on a new client succeeds. A hostile
    /// stream earns only its own loud refusals; it never wedges the device or breaks the
    /// bystander. (Generalizes #18A's atomic-apply + the fixed `b1_*` collision examples
    /// over generated sequences.)
    #[test]
    fn i4_hostile_flood_is_contained_never_wedges_or_breaks_a_bystander(
        hostile in prop::collection::vec(i4_hostile_event(), 0..40),
    ) {
        let mut gpu = new_gpu();
        // Bystander first — it legitimately owns BY_PDB / its vChids.
        for ev in bystander_events() {
            gpu.apply(ev).expect("benign bystander applies cleanly");
        }
        let baseline = resolve(&gpu, GpuId::ZERO, BY_PDB, BY_VA).map(|(b, _)| b.phys);
        prop_assert_eq!(baseline, Ok(BY_PHYS), "bystander baseline resolution");

        // The storm: apply each hostile event; a loud typed refusal is fine, a panic is
        // not (we would not reach the asserts). Containment is atomic per event.
        for ev in hostile {
            let _ = gpu.apply(ev);
            // No global wedge at ANY point: the device always still projects.
            prop_assert!(
                project(&gpu.spine.rmgraph, gpu.spine.arch(), &NO_CONDEMNED).is_ok(),
                "a hostile event left the device unprojectable (a global wedge)"
            );
        }

        // The bystander is intact: routing + resolution unchanged (a squat on BY_PDB
        // was refused — first-declarer-wins — never honored over the bystander).
        prop_assert!(gpu.spine.by_pdb.contains_key(&(GpuId::ZERO, BY_PDB)), "bystander PDB stopped routing");
        prop_assert_eq!(
            resolve(&gpu, GpuId::ZERO, BY_PDB, BY_VA).map(|(b, _)| b.phys),
            Ok(BY_PHYS),
            "bystander VA no longer resolves to its own backing"
        );

        // The device is still USABLE: a fresh benign process on a new client succeeds.
        const LATE_CLIENT: HClient = HClient(0x0F00D);
        const LATE_PDB: Pdb = Pdb(0x00F0_0000);
        let hl = identical_handles(0x60, 0x61);
        let mut sl = Scenario::new();
        sl.compute_process(LATE_CLIENT, LATE_PDB, hl);
        for ev in sl.events {
            let r = gpu.apply(ev);
            prop_assert!(r.is_ok(), "device unusable after storm: fresh benign op refused: {r:?} on {ev:?}");
        }
        prop_assert!(gpu.spine.by_pdb.contains_key(&(GpuId::ZERO, LATE_PDB)), "post-storm fresh proc did not route");
    }
}

// =================================================================================
// PHASE 3 — CONFUSED-DEPUTY, MADE STRUCTURAL
// Every site that turns a guest handle into a TYPED object routes through the ONE
// primitive `RmGraph::origin_of_kind`; resolving to the wrong `ObjectKind` is a single
// centrally-enforced check (decision #18C). This test is the executable audit.
// =================================================================================

/// Build a graph with exactly one object of each kind under a client, returning their
/// keys so every typed-resolution site can be probed with the WRONG kind.
#[allow(clippy::type_complexity)]
fn one_of_each_kind() -> (
    RmGraph,
    MockArch,
    NodeKey,
    NodeKey,
    NodeKey,
    NodeKey,
    NodeKey,
    NodeKey,
) {
    let arch = MockArch::new();
    let mut g = RmGraph::new();
    let c = HClient(0x1);
    let root = HObject(0x100);
    let dev = HObject(0x101);
    let vas = HObject(0x110);
    let tsg = HObject(0x120);
    let cs = HObject(0x121);
    let chan = HObject(0x130);
    let mem = HObject(0x140);
    let ev = HObject(0x150);
    let al = |client, parent, handle, class, facts| RmEvent::Alloc {
        client,
        parent,
        handle,
        class,
        facts,
    };
    for e in [
        al(c, root, root, mc::CLIENT, kayfabe_tests::user_client(c)),
        al(c, root, dev, mc::DEVICE, AllocFacts::default()),
        al(c, dev, vas, mc::VASPACE, AllocFacts::default()),
        al(c, dev, tsg, mc::TSG, AllocFacts::default()),
        al(c, tsg, cs, mc::CTXSHARE, AllocFacts::default()),
        al(c, tsg, chan, mc::CHANNEL_GR, AllocFacts::default()),
        al(
            c,
            dev,
            mem,
            mc::MEMORY,
            AllocFacts {
                mem_phys: Some(0x5000),
                ..Default::default()
            },
        ),
        al(c, dev, ev, mc::EVENT, AllocFacts::default()),
    ] {
        g.apply(&arch, e).unwrap();
    }
    (
        g,
        arch,
        NodeKey::new(c, vas),
        NodeKey::new(c, tsg),
        NodeKey::new(c, cs),
        NodeKey::new(c, chan),
        NodeKey::new(c, mem),
        NodeKey::new(c, ev),
    )
}

/// **Phase 3.** The centralized typed primitive `origin_of_kind` accepts a handle ONLY
/// for its true kind and rejects EVERY cross-kind pairing — the single check the M2
/// `resolve_vaspace_handle` fix is now generalized into. Also pins the one
/// kind-specific public resolver (`backing_of`) to Memory-only.
#[test]
fn p3_origin_of_kind_rejects_every_cross_kind_pairing() {
    let (g, _arch, vas, tsg, cs, chan, mem, ev) = one_of_each_kind();
    let sites: [(NodeKey, ObjectKind); 6] = [
        (vas, ObjectKind::VaSpace),
        (tsg, ObjectKind::Tsg),
        (cs, ObjectKind::CtxShare),
        (
            chan,
            ObjectKind::Channel {
                engine: kayfabe_arch::ids::EngineKind::GrCompute,
            },
        ),
        (mem, ObjectKind::Memory),
        (ev, ObjectKind::Event),
    ];
    for &(key, correct) in &sites {
        // The ONE primitive resolves for the true kind…
        assert!(
            g.origin_of_kind(key, correct).is_some(),
            "true-kind resolution failed for {correct:?}"
        );
        // …and rejects every other kind (the confused-deputy, centrally refused).
        for &(_, other) in &sites {
            if core::mem::discriminant(&other) != core::mem::discriminant(&correct) {
                assert!(
                    g.origin_of_kind(key, other).is_none(),
                    "confused-deputy: a {correct:?} handle resolved as {other:?}"
                );
            }
        }
    }
    // The kind-specific public memory→phys resolver is Memory-only: no non-memory
    // object ever yields a backing (even the VASpace, TSG, etc. above have no backing).
    assert_eq!(g.backing_of(mem), Some(0x5000));
    for non_mem in [vas, tsg, cs, chan, ev] {
        assert_eq!(
            g.backing_of(non_mem),
            None,
            "a non-Memory object yielded a backing"
        );
    }
}

/// **Phase 3.** The channel-VAS resolution precedence (own hVASpace → CtxShare → parent
/// TSG) type-checks EVERY hop: a channel whose `hVASpace`/`hContextShare`/parent names
/// a wrong-kind object resolves to NO VAS (a loud MISS at use time), never a silent
/// bind to an unrelated object's PDB. Probed end-to-end through the projection.
#[test]
fn p3_channel_vas_resolution_type_checks_every_hop() {
    let arch = MockArch::new();
    let c = HClient(0x7);
    // A channel whose hVASpace points at a MEMORY object carrying a baited SetPageDir.
    let mut g = RmGraph::new();
    let root = HObject(0x700);
    let dev = HObject(0x701);
    let mem = HObject(0x710);
    let chan = HObject(0x720);
    let bait_pdb = Pdb(0x0BAD_0000);
    for e in [
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
            handle: mem,
            class: mc::MEMORY,
            facts: AllocFacts {
                mem_phys: Some(0x1000),
                ..Default::default()
            },
        },
        // A SetPageDir aimed at the MEMORY handle (a hostile bait) — it is NOT a VASpace,
        // so this parks harmlessly and never makes `mem` route as an address space.
        RmEvent::SetPageDir {
            client: c,
            vaspace: mem,
            pdb: bait_pdb,
        },
        RmEvent::Alloc {
            client: c,
            parent: dev,
            handle: chan,
            class: mc::CHANNEL_GR,
            facts: AllocFacts {
                h_vaspace: Some(mem),
                userd_flags: MockArch::userd_flags_for(VChid(0x33)),
                ..Default::default()
            },
        },
    ] {
        g.apply(&arch, e).unwrap();
    }
    let bounds = project(&g, &arch, &NO_CONDEMNED).expect("projects");
    // The channel resolved to NO VAS (its hVASpace named a non-VASpace).
    let chan_facts = bounds
        .procs
        .iter()
        .flat_map(|p| p.channels.values())
        .next()
        .expect("a channel");
    assert_eq!(
        chan_facts.vas_pdb, None,
        "channel bound to a non-VASpace's baited PDB"
    );
    assert_eq!(
        chan_facts.vas_origin, None,
        "channel resolved a VAS origin it should not have"
    );
    // The baited PDB never routes (it was never a real VASpace's PDB).
    assert!(
        !bounds.by_pdb.contains_key(&(GpuId::ZERO, bait_pdb)),
        "a baited non-VASpace PDB entered routing"
    );
}

// =================================================================================
// PHASE 4 — the confused-deputy this pass FIXED in the core.
// A `MAP_MEMORY_DMA` whose `memory` handle names a NON-memory object is now a loud
// unbacked fault, never a silent bind to an attacker-declared `mem_phys` — the two
// memory→phys resolvers (`backing_of` and `apply_map`) can no longer disagree.
// =================================================================================

/// **Phase 4 regression.** Before this fix, `apply_map` read `facts.mem_phys` off
/// WHATEVER object the `memory` handle named — so a hostile guest could name a NON-
/// memory object (here a second VASpace) carrying an attacker-set `mem_phys` and have
/// it SILENTLY accepted as a mapping's backing (`backing_of` type-checked; `apply_map`
/// did not — two resolvers disagreeing, the exact class the "ONE resolution path"
/// directive forbids). Now both route through the ONE typed `backing_of`: a non-Memory
/// backing resolves to nothing → a LOUD `UnbackedMapping` fault, and the VA never binds
/// to the attacker's phys.
#[test]
fn p4_map_naming_a_non_memory_object_is_a_loud_unbacked_fault_not_a_silent_bind() {
    let mut gpu = new_gpu();
    let c = HClient(0x9);
    let root = HObject(0x900);
    let dev = HObject(0x901);
    let real_vas = HObject(0x910);
    let evil_vas = HObject(0x911); // a VASpace masquerading as a memory backing
    let pdb = Pdb(0x0009_0000);
    let evil_phys = 0xEEEE_0000_u64;
    let va = GpuVa(0x3_0000_0000);

    for e in [
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
            handle: real_vas,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: c,
            vaspace: real_vas,
            pdb,
        },
        // The bait: a VASpace carrying an attacker-declared mem_phys in its alloc facts.
        RmEvent::Alloc {
            client: c,
            parent: dev,
            handle: evil_vas,
            class: mc::VASPACE,
            facts: AllocFacts {
                mem_phys: Some(evil_phys),
                ..Default::default()
            },
        },
    ] {
        gpu.apply(e).expect("setup applies");
    }

    // Map the bait VASpace as if it were a memory object into the real VAS.
    let attack = gpu.apply(RmEvent::MapMemoryDma {
        client: c,
        vaspace: real_vas,
        memory: evil_vas,
        va,
        offset: 0,
        len: 0x1000,
    });

    // It is a LOUD unbacked fault (rolled back atomically), NOT a silent bind.
    // ★ §12.38 — exact variant, both fields: the fault must name the VAS being attacked
    // and the VA the confused deputy would have bound, not merely "something failed".
    assert_eq!(
        attack,
        Err(GpuError::UnbackedMapping { pdb, va: va.0 }),
        "mapping a non-Memory object as backing must be a loud UnbackedMapping fault \
         naming the exact PDB and VA"
    );
    // The VA never resolved to the attacker's phys — the confused-deputy is closed.
    assert!(
        resolve(&gpu, GpuId::ZERO, pdb, va).is_err(),
        "the VA silently bound to a non-Memory backing"
    );

    // Sanity: a REAL memory object with the same phys DOES map cleanly (the fix does not
    // over-reject legitimate mappings — only the type-confused ones).
    let real_mem = HObject(0x920);
    gpu.apply(RmEvent::Alloc {
        client: c,
        parent: dev,
        handle: real_mem,
        class: mc::MEMORY,
        facts: AllocFacts {
            mem_phys: Some(evil_phys),
            ..Default::default()
        },
    })
    .unwrap();
    gpu.apply(RmEvent::MapMemoryDma {
        client: c,
        vaspace: real_vas,
        memory: real_mem,
        va,
        offset: 0,
        len: 0x1000,
    })
    .expect("a real memory maps cleanly");
    assert_eq!(
        resolve(&gpu, GpuId::ZERO, pdb, va).map(|(b, _)| b.phys),
        Ok(evil_phys),
        "a legitimate memory mapping must resolve"
    );
}

/// **Phase 4 regression — the PARKED-PDB WEDGE (found by `i4`).** A hostile process
/// parks a `SET_PAGE_DIRECTORY` on a handle it does not yet own, aimed at the VICTIM's
/// PDB; allocates its OWN VASpace with its OWN PDB; then `DUP_OBJECT`s that VASpace onto
/// the parked handle. The stale parked declaration now resolves (via the dup alias) onto
/// the attacker's live VASpace. Before the fix, a later UNRELATED benign `Alloc` would
/// trigger `resolve_pending_pdbs`, OVERWRITE the VASpace's real PDB with the victim's,
/// forge a projection `PdbCollision`, and — because the parked fact survives the rollback
/// — re-fire on EVERY subsequent `Alloc`: a device-wide control-plane WEDGE. The fix
/// (drain-without-overwrite) makes the parked declaration never clobber a set PDB, so the
/// device stays usable and the victim is untouched.
#[test]
fn p4_parked_setpagedir_via_dup_alias_cannot_wedge_the_device() {
    let mut gpu = new_gpu();
    for ev in bystander_events() {
        gpu.apply(ev).expect("bystander applies");
    }
    let a = HClient(0xA001);
    let root = HObject(0xA00);
    let dev = HObject(0xA01);
    let h1 = HObject(0x5c00_0000); // not yet owned when the SetPageDir is parked
    let h2 = HObject(0x5c00_0004);
    let a_pdb = Pdb(0x00A0_0000);
    gpu.apply(RmEvent::Alloc {
        client: a,
        parent: root,
        handle: root,
        class: mc::CLIENT,
        facts: kayfabe_tests::user_client(a),
    })
    .unwrap();
    gpu.apply(RmEvent::Alloc {
        client: a,
        parent: root,
        handle: dev,
        class: mc::DEVICE,
        facts: AllocFacts {
            device_instance: Some(0),
            ..Default::default()
        },
    })
    .unwrap();
    // 1. Park a SetPageDir on the (unowned) h1, aimed at the VICTIM's PDB.
    gpu.apply(RmEvent::SetPageDir {
        client: a,
        vaspace: h1,
        pdb: BY_PDB,
    })
    .unwrap();
    // 2/3. Alloc h2 as the attacker's own VASpace and give it its OWN pdb.
    gpu.apply(RmEvent::Alloc {
        client: a,
        parent: dev,
        handle: h2,
        class: mc::VASPACE,
        facts: AllocFacts::default(),
    })
    .unwrap();
    gpu.apply(RmEvent::SetPageDir {
        client: a,
        vaspace: h2,
        pdb: a_pdb,
    })
    .unwrap();
    // 4. Dup h2 onto h1 — the stale parked SetPageDir now resolves onto h2's resource.
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(a, h2),
        dst: NodeKey::new(a, h1),
    })
    .unwrap();

    // NO WEDGE: a fresh benign process still allocates cleanly (the landmine would make
    // its client-root Alloc trigger resolve_pending_pdbs → collision → rollback).
    const LATE: HClient = HClient(0x0F001);
    const LATE_PDB: Pdb = Pdb(0x00F1_0000);
    let mut sl = Scenario::new();
    sl.compute_process(LATE, LATE_PDB, identical_handles(0x62, 0x63));
    for ev in sl.events {
        gpu.apply(ev)
            .expect("device wedged: fresh benign proc refused after the parked-PDB attack");
    }
    // The victim keeps its PDB + backing; the attacker kept ITS own PDB (never the victim's).
    assert_eq!(
        resolve(&gpu, GpuId::ZERO, BY_PDB, BY_VA).map(|(b, _)| b.phys),
        Ok(BY_PHYS),
        "victim corrupted"
    );
    assert!(
        gpu.spine.by_pdb.contains_key(&(GpuId::ZERO, a_pdb)),
        "attacker VAS lost its own PDB (stale parked one clobbered it)"
    );
    assert!(
        gpu.spine.by_pdb.contains_key(&(GpuId::ZERO, LATE_PDB)),
        "fresh proc did not route"
    );
}

/// **Phase 4 regression — the PARKED-MAP WEDGE (the parked-PDB wedge's twin).** A
/// hostile process parks a `MAP_MEMORY_DMA` naming an unowned handle as its `memory`,
/// against its OWN (PDB-bound) VASpace; allocates an UNBACKED memory object; then
/// `DUP_OBJECT`s that unbacked memory onto the parked handle. The parked map now resolves
/// to an unbacked backing. Before the fix, a later UNRELATED benign `Alloc` would trigger
/// `resolve_pending_maps`, INSTALL the unbacked mapping, and `Gpu::sync_rpc_mappings`
/// would fault `UnbackedMapping` — rolling back the benign op and re-firing forever (a
/// control-plane WEDGE). The fix drops a replayed unbacked map (contained MISS=FAULT at
/// use), so the device stays usable.
#[test]
fn p4_parked_unbacked_map_via_dup_alias_cannot_wedge_the_device() {
    let mut gpu = new_gpu();
    for ev in bystander_events() {
        gpu.apply(ev).expect("bystander applies");
    }
    let a = HClient(0xA002);
    let root = HObject(0xB00);
    let dev = HObject(0xB01);
    let vas = HObject(0xB10);
    let a_pdb = Pdb(0x00A2_0000);
    let mbad = HObject(0xB20); // an UNBACKED memory (mem_phys = None)
    let alias = HObject(0xB21); // will dup-alias mbad after the map is parked
    let va = GpuVa(0x4_0000_0000);
    for e in [
        RmEvent::Alloc {
            client: a,
            parent: root,
            handle: root,
            class: mc::CLIENT,
            facts: kayfabe_tests::user_client(a),
        },
        RmEvent::Alloc {
            client: a,
            parent: root,
            handle: dev,
            class: mc::DEVICE,
            facts: AllocFacts {
                device_instance: Some(0),
                ..Default::default()
            },
        },
        RmEvent::Alloc {
            client: a,
            parent: dev,
            handle: vas,
            class: mc::VASPACE,
            facts: AllocFacts::default(),
        },
        RmEvent::SetPageDir {
            client: a,
            vaspace: vas,
            pdb: a_pdb,
        },
        RmEvent::Alloc {
            client: a,
            parent: dev,
            handle: mbad,
            class: mc::MEMORY,
            facts: AllocFacts::default(),
        },
    ] {
        gpu.apply(e).expect("attacker setup applies");
    }
    // Park a map naming the (unowned) `alias` as its memory, into the PDB-bound VAS.
    gpu.apply(RmEvent::MapMemoryDma {
        client: a,
        vaspace: vas,
        memory: alias,
        va,
        offset: 0,
        len: 0x1000,
    })
    .expect("map parks");
    // Dup the unbacked memory onto `alias` — the parked map now resolves to no backing.
    gpu.apply(RmEvent::Dup {
        src: NodeKey::new(a, mbad),
        dst: NodeKey::new(a, alias),
    })
    .expect("dup applies");

    // NO WEDGE: a fresh benign process still allocates cleanly (the landmine would make
    // its client-root Alloc trigger resolve_pending_maps → UnbackedMapping → rollback).
    const LATE: HClient = HClient(0x0F002);
    const LATE_PDB: Pdb = Pdb(0x00F2_0000);
    let mut sl = Scenario::new();
    sl.compute_process(LATE, LATE_PDB, identical_handles(0x64, 0x65));
    for ev in sl.events {
        gpu.apply(ev)
            .expect("device wedged: fresh benign proc refused after the parked-map attack");
    }
    assert_eq!(
        resolve(&gpu, GpuId::ZERO, BY_PDB, BY_VA).map(|(b, _)| b.phys),
        Ok(BY_PHYS),
        "victim corrupted"
    );
    assert!(
        gpu.spine.by_pdb.contains_key(&(GpuId::ZERO, LATE_PDB)),
        "fresh proc did not route"
    );
    // The unbacked VA never populated — a loud MISS at use, contained (never a wedge).
    assert!(
        resolve(&gpu, GpuId::ZERO, a_pdb, va).is_err(),
        "an unbacked VA must MISS at use"
    );
}
