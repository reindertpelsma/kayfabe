//! ★ 2026-11-18 — **Option C2, stage 3, GPU-free: `exact-sparse` through the real apply path**
//! (`docs/design/V3_SPARSE_RESIDENCY_RCA.md` §8.9).
//!
//! The space's static guest range is tiled ([`Sim::tile`], as `HostRm::alloc_vaspace` does with
//! `exact-sparse` on). The walker's SPARSE runs (ABI 7, [`DiffRun::sparse`]) and its leaves go through
//! `apply_entry` → `BatchedVas` → the model's `SpaceVerbs`, which route a tiled range through the same
//! glue `kf_host`'s piece functions use (`kf_host::tiling::map_tiled` / `unmap_tiled`).
//!
//! Also here: the PLACEMENT LOCK against the measured race (M3), and a refused host call at EVERY
//! step of a re-tile.

use super::*;
use crate::apply::DiffRun;
use crate::ledger::NO_EXACT_TILING;
use kf_host::placement::PlacementLock;
use kf_host::tiling::{Attr, Base, TILE, TileVerbs, Tiling, TilingCfg};
use std::cell::{Cell, RefCell as Rc};

const R: u64 = (1 << 32) + (1 << 29);
const R_END: u64 = R + (8 << 20);
const SLOT: u64 = TILE;
const RES: u64 = R + (2 << 20);

fn slot(i: u64) -> u64 {
    RES + i * SLOT
}

/// A SPARSE MAP run as the walker emits it (ABI 7: `gpga == va`, no aperture, no kind).
fn sparse_run(va: u64, len: u64) -> DiffRun {
    DiffRun {
        sparse: true,
        leaf: SLOT,
        ..map_run(va, len, false, va, false)
    }
}

fn leaf_run(i: u64) -> DiffRun {
    DiffRun {
        leaf: SLOT,
        ..map_run(slot(i), SLOT, true, 0x4000_0000 + i * SLOT, false)
    }
}

const PATHS: [(bool, bool); 3] = [(false, false), (true, false), (true, true)];

fn tiled_sim() -> Sim {
    let sim = Sim::new(space(R, R_END));
    sim.tile();
    sim
}

/// The residency image: 32 slots, even ones bound (64 KiB leaves), odd ones SPARSE runs.
fn residency(m: &SimMirror<'_>, c: &mut BTreeMap<u64, Committed>) -> crate::apply::Applied {
    let mut runs: Vec<DiffRun> = (0..32).step_by(2).map(leaf_run).collect();
    runs.extend((1..32).step_by(2).map(|i| sparse_run(slot(i), SLOT)));
    apply_and_commit(m, &runs, c)
}

/// ★ **S1 AND S2 through the real apply path, with `exact-sparse`.** Every odd slot reads zero on the
/// host, every even slot is mapped exactly (the guard held through every host call), and the rest
/// of the guest range still faults. On every apply path.
#[test]
fn exact_apply_keeps_s1_and_s2_for_the_residency_pattern() {
    for (batching, low) in PATHS {
        let sim = tiled_sim();
        let mut c = BTreeMap::new();
        let m = SimMirror::new(&sim, batching, low);
        let out = residency(&m, &mut c);
        assert_eq!(
            out.refused, 0,
            "batching={batching} low={low}: {:?}",
            out.first_refusal
        );
        check(&sim, &c, &BTreeMap::new(), R, R_END).unwrap_or_else(|e| panic!("{e}"));
        let rm = sim.0.borrow();
        for i in 0..32 {
            let got = rm.host_pte(slot(i) + 0x800);
            if i % 2 == 1 {
                assert_eq!(
                    got,
                    HostPte::Sparse,
                    "S1: batching={batching} low={low} slot {i}"
                );
            } else {
                assert!(matches!(got, HostPte::Mapped(_)), "slot {i}: {got:?}");
            }
        }
        for p in [R, RES - P, slot(32), R_END - P] {
            assert_eq!(
                rm.host_pte(p),
                HostPte::Invalid,
                "S2: batching={batching} low={low} {p:#x}"
            );
        }
    }
}

/// ★ An unbind back to SPARSE, as the walker's diff spells it (UNMAP the leaf, UNMAP the old sparse
/// neighbour, MAP the coalesced sparse run), then the guest frees the resource (UNMAP every
/// placement): SPARSE where the guest is SPARSE, INVALID after, and the tiling merges back.
#[test]
fn exact_apply_unbind_then_free_returns_to_one_fence() {
    for (batching, low) in PATHS {
        let sim = tiled_sim();
        let mut c = BTreeMap::new();
        let m = SimMirror::new(&sim, batching, low);
        assert_eq!(residency(&m, &mut c).refused, 0);
        let runs = vec![
            unmap_run(slot(0), &c[&slot(0)]),
            unmap_run(slot(1), &c[&slot(1)]),
            sparse_run(slot(0), 2 * SLOT),
        ];
        let out = apply_and_commit(&m, &runs, &mut c);
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
        check(&sim, &c, &BTreeMap::new(), R, R_END).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            sim.0.borrow().host_pte(slot(0)),
            HostPte::Sparse,
            "S1 after the unbind"
        );
        // The guest frees the whole resource.
        let all: Vec<DiffRun> = c.iter().map(|(&v, x)| unmap_run(v, x)).collect();
        let out = apply_and_commit(&m, &all, &mut c);
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
        check(&sim, &c, &BTreeMap::new(), R, R_END).unwrap_or_else(|e| panic!("{e}"));
        let rm = sim.0.borrow();
        for i in 0..32 {
            assert_eq!(
                rm.host_pte(slot(i)),
                HostPte::Invalid,
                "S2 after the free: slot {i}"
            );
        }
        drop(rm);
        assert_eq!(
            sim.1.borrow().as_ref().map(|t| t.blocks().len()),
            Some(1),
            "batching={batching} low={low}: back to one fence"
        );
    }
}

/// ★ S2 through the apply path with `exact-sparse`: a row mapped and plainly unmapped, and VAs never
/// mapped, FAULT on the host (no SPARSE run was ever sent).
#[test]
fn exact_apply_keeps_s2_without_sparse_runs() {
    for (batching, low) in PATHS {
        let sim = tiled_sim();
        let mut c = BTreeMap::new();
        let m = SimMirror::new(&sim, batching, low);
        let r = apply_and_commit(&m, &[map_run(R, 4 * P, true, 0x9000, false)], &mut c);
        assert_eq!(r.refused, 0, "{:?}", r.first_refusal);
        let r = apply_and_commit(&m, &[unmap_run(R, &c[&R])], &mut c);
        assert_eq!(r.refused, 0, "{:?}", r.first_refusal);
        check(&sim, &c, &BTreeMap::new(), R, R_END).unwrap_or_else(|e| panic!("{e}"));
        let rm = sim.0.borrow();
        for p in [R, R + 3 * P, R + (2 << 20), R_END - P] {
            assert_eq!(
                rm.host_pte(p),
                HostPte::Invalid,
                "batching={batching} low={low} {p:#x}"
            );
        }
    }
}

/// ★ `exact-sparse` OFF (the default): a SPARSE run is refused by name (FAILED, so the walker keeps
/// offering it), every other run applies as before, and S2 holds — exactly today's behaviour.
#[test]
fn exact_off_refuses_sparse_runs_by_name_and_changes_nothing_else() {
    for (batching, low) in PATHS {
        let sim = Sim::new(space(R, R_END));
        let mut c = BTreeMap::new();
        let m = SimMirror::new(&sim, batching, low);
        let out = residency(&m, &mut c);
        assert_eq!(out.refused, 16, "the 16 SPARSE runs, nothing else");
        assert_eq!(out.first_refusal.as_deref(), Some(NO_EXACT_TILING));
        check(&sim, &c, &BTreeMap::new(), R, R_END).unwrap_or_else(|e| panic!("{e}"));
        let rm = sim.0.borrow();
        for i in (1..32).step_by(2) {
            assert_eq!(
                rm.host_pte(slot(i)),
                HostPte::Invalid,
                "off: slot {i} stays INVALID"
            );
        }
    }
}

// ═══ the placement lock against the race kf-va-tiling-probe M3 found (2026-11-18, traces/sparse_c2_tiling_probe_20261118) ═══════════════════════════════════════════

/// The model's tiling verbs with the PLACEMENT LOCK, and an adversary that tries a NON-FIXED
/// placement (64 KiB at the lowest free VA from `R`, as host RM placed its buffers at 4.5 GiB in M3)
/// after EVERY free of a re-tile — exactly where M3 (2026-11-18, `traces/sparse_c2_tiling_probe_20261118`) landed.
struct Raced<'a> {
    sim: &'a Sim,
    lock: &'a PlacementLock,
    guard: Rc<Option<kf_host::placement::PlacementGuard<'a>>>,
    use_lock: bool,
    placed_inside: Cell<u64>,
    blocked: Cell<u64>,
    next_id: Cell<u32>,
}

impl Raced<'_> {
    fn adversary(&self) {
        if self.lock.busy() {
            self.blocked.set(self.blocked.get() + 1);
            return;
        }
        let mut rm = self.sim.0.borrow_mut();
        let mut va = R;
        while va < R_END + (1 << 30) {
            if !rm.blocks.iter().any(|(&s, &e)| s < va + SLOT && va < e) {
                break;
            }
            va += SLOT;
        }
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        rm.blocks.insert(va, va + SLOT); // host RM's own block (a context buffer)
        if (R..R_END).contains(&va) {
            self.placed_inside.set(self.placed_inside.get() + 1);
        }
    }
}

impl TileVerbs for Raced<'_> {
    fn reserve(&self, lo: u64, hi: u64, base: Base, attr: Attr) -> Result<u32, String> {
        SimTiles(self.sim).reserve(lo, hi, base, attr)
    }
    fn free(&self, handle: u32) -> Result<(), String> {
        let r = SimTiles(self.sim).free(handle);
        self.adversary();
        r
    }
    fn retile_begin(&self) -> Result<(), String> {
        if self.use_lock {
            let g = self.lock.hold_for_retile()?;
            *self.guard.borrow_mut() = Some(g);
        }
        Ok(())
    }
    fn retile_end(&self) {
        self.guard.borrow_mut().take();
    }
}

fn race(use_lock: bool) -> (u64, u64, u64) {
    let sim = Sim::new(VaSpace {
        space: 1,
        range: RANGE,
        guest: [kf_host::channel::GuestVaRange::default(); 3],
    });
    let lock = PlacementLock::default();
    let v = Raced {
        sim: &sim,
        lock: &lock,
        guard: Rc::new(None),
        use_lock,
        placed_inside: Cell::new(0),
        blocked: Cell::new(0),
        next_id: Cell::new(1),
    };
    let mut t = Tiling::new(&v, R, R_END, TilingCfg::default()).expect("tile");
    let mut rng = Rng(0xBADC_0FFE);
    for _ in 0..200 {
        let a = R + rng.below((R_END - R) / TILE - 4) * TILE;
        let base = if rng.below(2) == 0 {
            Base::Sparse
        } else {
            Base::Invalid
        };
        let _ = t.set_base(&v, a, (1 + rng.below(3)) * TILE, base);
    }
    (v.placed_inside.get(), v.blocked.get(), t.stats.rollbacks)
}

/// ★★ **The race of `kf-va-tiling-probe` M3 (2026-11-18, `traces/sparse_c2_tiling_probe_20261118`),
/// modelled, cannot happen under the lock.** An adversary tries a
/// non-FIXED placement after every free of every re-tile. With the lock it is always blocked inside
/// the window, so nothing lands in the guest range and no re-tile is ever refused; without it (the
/// control) placements land in freed fences and re-tiles roll back — what M3 showed on the RTX 3090 box.
#[test]
fn the_placement_lock_closes_the_measured_race() {
    let (inside, blocked, rollbacks) = race(true);
    assert_eq!(
        inside, 0,
        "under the lock no placement lands in the guest range"
    );
    assert_eq!(rollbacks, 0, "and no re-tile is refused");
    assert!(
        blocked > 0,
        "the adversary did try inside the windows ({blocked} blocked)"
    );
    let (inside, _, rollbacks) = race(false);
    assert!(
        inside > 0 && rollbacks > 0,
        "control: without the lock the race lands ({inside}, {rollbacks})"
    );
}

/// ★ The blocking rule, shown on the model: on a thread marked as a vCPU (what the MMIO trap entry
/// does), a re-tile is refused before ANY host call — re-tiles run on the VA-manager thread, never in
/// a trap — and the layout is intact.
#[test]
fn a_retile_on_a_vcpu_thread_is_refused_before_any_host_call() {
    std::thread::scope(|s| {
        s.spawn(|| {
            let sim = Sim::new(VaSpace {
                space: 1,
                range: RANGE,
                guest: [kf_host::channel::GuestVaRange::default(); 3],
            });
            let lock = PlacementLock::default();
            let v = Raced {
                sim: &sim,
                lock: &lock,
                guard: Rc::new(None),
                use_lock: true,
                placed_inside: Cell::new(0),
                blocked: Cell::new(0),
                next_id: Cell::new(1),
            };
            let mut t = Tiling::new(&v, R, R_END, TilingCfg::default()).expect("tile (VA thread)");
            kf_host::placement::mark_vcpu_thread();
            let before = sim.0.borrow().n;
            let r = t.set_base(&v, R + TILE, TILE, Base::Sparse);
            assert_eq!(
                r,
                Err(kf_host::tiling::TileError::Refused(
                    kf_host::placement::RETILE_ON_VCPU.into()
                ))
            );
            assert_eq!(sim.0.borrow().n, before, "no host call on the vCPU thread");
            exact(&sim, &t).expect("the layout is intact");
            assert_eq!(lock.vcpu_holds(), 1);
            assert_eq!(
                t.pending().len(),
                1,
                "the wish waits for the VA-manager thread"
            );
        });
    });
}

// ═══ a refused host call at EVERY step of a re-tile ══════════════════════════════════════════════

/// The model's verbs that refuse exactly the `fail_at`-th call (reserve or free) from now.
struct NthFails<'a> {
    sim: &'a Sim,
    n: Cell<u64>,
    fail_at: Cell<u64>,
}

impl TileVerbs for NthFails<'_> {
    fn reserve(&self, lo: u64, hi: u64, base: Base, attr: Attr) -> Result<u32, String> {
        self.n.set(self.n.get() + 1);
        if self.n.get() == self.fail_at.get() {
            return Err("refused (injected)".into());
        }
        SimTiles(self.sim).reserve(lo, hi, base, attr)
    }
    fn free(&self, handle: u32) -> Result<(), String> {
        self.n.set(self.n.get() + 1);
        if self.n.get() == self.fail_at.get() {
            return Err("refused (injected)".into());
        }
        SimTiles(self.sim).free(handle)
    }
}

/// The engine's blocks are the host's reservations, one for one (no leak, no rounding).
fn exact(sim: &Sim, t: &Tiling) -> Result<(), String> {
    t.check()?;
    let rm = sim.0.borrow();
    if rm.resv.len() != t.blocks().len() {
        return Err(format!(
            "host {} reservations, engine {} blocks",
            rm.resv.len(),
            t.blocks().len()
        ));
    }
    for (&k, b) in t.blocks() {
        if rm.resv.get(&b.handle) != Some(&(k, b.hi)) {
            return Err(format!("block {k:#x} differs on the host"));
        }
        if rm.sparse_resv.contains(&b.handle) != (b.base == Base::Sparse) {
            return Err(format!("block {k:#x}: base differs on the host"));
        }
    }
    Ok(())
}

/// After a refused step: exact layout, no gap unless a restore was refused, every page whose host
/// base misses the wish reported pending; then (faults off) one batch heals it to the wish.
fn after_refusal(sim: &Sim, t: &mut Tiling, v: &NthFails<'_>, what: &str) {
    exact(sim, t).unwrap_or_else(|e| panic!("{what}: {e}"));
    let covered = {
        let mut cur = R;
        let mut ok = true;
        for (&k, b) in t.blocks() {
            ok &= k == cur;
            cur = b.hi;
        }
        ok && cur == R_END
    };
    assert!(
        covered || t.stats.gaps_left > 0,
        "{what}: a gap no refused restore explains"
    );
    let pending = t.pending();
    let rm = sim.0.borrow();
    for p in (R..R_END).step_by(TILE as usize) {
        let want = t.wished(p);
        let host = match rm.host_pte(p) {
            HostPte::Sparse => Some(Base::Sparse),
            HostPte::Invalid => Some(Base::Invalid),
            HostPte::Mapped(_) => want,
        };
        if host != want {
            assert!(
                pending.iter().any(|&(a, b, _)| a <= p && p < b),
                "{what}: {p:#x} misses its wish and is not pending"
            );
        }
    }
    drop(rm);
    v.fail_at.set(0);
    t.begin_batch(v);
    assert!(
        t.pending().is_empty(),
        "{what}: healed by the next batch: {:x?}",
        t.pending()
    );
    exact(sim, t).unwrap_or_else(|e| panic!("{what} healed: {e}"));
}

/// ★★ **A refused host call at EVERY step of every kind of re-tile** rolls back to a consistent
/// layout (or a gap that reads INVALID), keeps every miss visible, and heals on the next batch. The
/// steps: a SPARSE carve out of a fence (free 1, reserve 3), an INVALID restore that merges three
/// blocks (free 3, reserve 1), a row carve (free 1, reserve 3) and a release that merges (free 3,
/// reserve 1). Each is driven with the n-th call refused, for every n of that step.
#[test]
fn a_refused_call_at_every_step_of_a_retile_rolls_back_consistently() {
    type Setup = fn(&mut Tiling, &NthFails<'_>);
    type Step = fn(&mut Tiling, &NthFails<'_>) -> Result<(), kf_host::tiling::TileError>;
    let cases: [(&str, Setup, Step); 4] = [
        (
            "sparse carve",
            |_, _| {},
            |t, v| t.set_base(v, R + 16 * TILE, 2 * TILE, Base::Sparse),
        ),
        (
            "invalid restore and merge",
            |t, v| {
                t.set_base(v, R + 16 * TILE, 2 * TILE, Base::Sparse)
                    .expect("setup")
            },
            |t, v| t.set_base(v, R + 16 * TILE, 2 * TILE, Base::Invalid),
        ),
        (
            "row carve",
            |_, _| {},
            |t, v| t.prepare_row(v, R + 16 * TILE + P, 3 * P).map(|_| ()),
        ),
        (
            "release and merge",
            |t, v| {
                t.prepare_row(v, R + 16 * TILE, TILE).expect("setup");
                t.commit_row(R + 16 * TILE, TILE).expect("setup");
            },
            |t, v| t.release_range(v, R + 16 * TILE, TILE),
        ),
    ];
    for (name, setup, step) in cases {
        // How many host calls the step makes when nothing fails.
        let calls = {
            let sim = Sim::new(VaSpace {
                space: 1,
                range: RANGE,
                guest: [kf_host::channel::GuestVaRange::default(); 3],
            });
            let v = NthFails {
                sim: &sim,
                n: Cell::new(0),
                fail_at: Cell::new(0),
            };
            let mut t = Tiling::new(&v, R, R_END, TilingCfg::default()).expect("tile");
            setup(&mut t, &v);
            let n0 = v.n.get();
            step(&mut t, &v).expect("clean step");
            v.n.get() - n0
        };
        assert!(calls >= 4, "{name}: {calls} calls");
        for k in 1..=calls {
            let sim = Sim::new(VaSpace {
                space: 1,
                range: RANGE,
                guest: [kf_host::channel::GuestVaRange::default(); 3],
            });
            let v = NthFails {
                sim: &sim,
                n: Cell::new(0),
                fail_at: Cell::new(0),
            };
            let mut t = Tiling::new(&v, R, R_END, TilingCfg::default()).expect("tile");
            setup(&mut t, &v);
            v.fail_at.set(v.n.get() + k);
            let r = step(&mut t, &v);
            assert!(
                r.is_err(),
                "{name} call {k}/{calls}: the refusal is reported"
            );
            assert!(t.stats.rollbacks >= 1, "{name} call {k}: rolled back");
            after_refusal(
                &sim,
                &mut t,
                &v,
                &format!("{name}, call {k} of {calls} refused"),
            );
        }
    }
}

fn bare_sim() -> Sim {
    Sim::new(VaSpace {
        space: 1,
        range: RANGE,
        guest: [kf_host::channel::GuestVaRange::default(); 3],
    })
}

/// ★ A RANGE unmap that cuts a row (the apply's changed sub-range) keeps the row's outside part
/// pinned: a later re-tile next to it never frees the block that still maps it (the guard holds).
#[test]
fn a_range_unmap_keeps_the_rest_of_the_row_mapped_through_later_retiles() {
    let sim = bare_sim();
    let v = SimTiles(&sim);
    let mut t = Tiling::new(&v, R, R_END, TilingCfg::default()).expect("tile");
    let a = R + 8 * TILE;
    kf_host::tiling::map_tiled(
        &mut t,
        &v,
        a,
        2 * TILE,
        &mut |h, pva, plen| {
            sim.0
                .borrow_mut()
                .map_through(h, pva, plen, RAM_OBJ, pva - a)
        },
        &mut |_, _| {},
    )
    .expect("map the row");
    kf_host::tiling::unmap_tiled(&mut t, &v, a + TILE, TILE, true, &mut |h, pva, size| {
        sim.0.borrow_mut().unmap_range_in(h, pva, pva + size, true);
        Ok::<(), String>(())
    })
    .expect("cut the row");
    sim.0.borrow_mut().guard = (0..TILE / P).map(|i| (a + i * P, (true, i * P))).collect();
    t.set_base(&v, a + TILE, TILE, Base::Sparse).expect("wish");
    let rm = sim.0.borrow();
    assert!(
        rm.transient.is_empty(),
        "the left half stayed mapped: {:?}",
        rm.transient.first()
    );
    assert_eq!(rm.host_pte(a), HostPte::Mapped((true, 0)));
}

/// ★ The coverage after a cut: the row's outside parts are still live rows, and a wish over a
/// still-pinned tile waits (pending) instead of re-tiling under a mapping.
#[test]
fn a_cut_row_keeps_its_outside_parts_as_live_coverage() {
    let sim = bare_sim();
    let v = SimTiles(&sim);
    let mut t = Tiling::new(&v, R, R_END, TilingCfg::default()).expect("tile");
    let a = R + 8 * TILE;
    t.prepare_row(&v, a, 3 * TILE).expect("carve");
    t.commit_row(a, 3 * TILE).expect("commit");
    t.release_range(&v, a + TILE, TILE).expect("cut the middle");
    assert_eq!(t.row_end(a), Some(a + TILE), "the left part is still a row");
    assert_eq!(
        t.row_end(a + 2 * TILE),
        Some(a + 3 * TILE),
        "and the right part"
    );
    t.set_base(&v, a + TILE, TILE, Base::Sparse).expect("wish");
    assert!(
        !t.pending().is_empty(),
        "the middle tile sits in a pinned block: pending"
    );
}
