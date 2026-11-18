//! ★ 2026-11-18 — **Option C2, stage 1, GPU-free: exact block tiling keeps S1 AND S2**
//! (`docs/design/V3_SPARSE_RESIDENCY_RCA.md` §8; engine: `kf_host::tiling`).
//!
//! The host is `sim`'s model of one VA space with NO static guest reservation over the tiled range
//! (C2 replaces the static fence by the engine's blocks). The model's reservations take the engine's
//! base (`_SPARSE` or not) and attr (host rounding, `[measured kf-va-tiling-probe M5]`), and
//! [`SimRm::host_pte`] answers what the host MMU finds. The guest side of each test is driven the way
//! stage 2's apply will drive it: a SPARSE run is [`Tiling::set_base`], a row is `prepare_row` → one
//! map per piece → `commit_row`, an unmap is one unmap per piece → `set_base` (where the guest's PTE
//! went) → `release_row`.
//!
//! Invariants, per page (the oracle is brute force over the guest's own page states):
//! - mapped guest page ⇒ the host translates it exactly (and the GUARD holds: no host call of any
//!   step unmaps another live row, checked after every host call);
//! - **S1**: an unmapped page of a tile whose unmapped pages are all guest-SPARSE is SPARSE on the host;
//! - **S2**: an unmapped page of a tile with any guest-INVALID page is INVALID on the host;
//! - exact tiling: the engine's blocks are exactly the host's reservations (no rounding, no leak, no
//!   overlap), ordered and inside the range;
//! - a mismatch is allowed only where [`Tiling::pending`] shows it (budget, cap, refusal, or a mixed
//!   tile pinned by a row), and an S2 mismatch only for a mixed tile under a live row.

use super::*;
use kf_host::tiling::{Attr, Base, TILE, TileError, TileVerbs, Tiling, TilingCfg};

/// The tiled range: 4 MiB at 4.5 GiB (two 2 MiB pages, so 2 MiB rounding is exercised).
const R: u64 = (1 << 32) + (1 << 29);
const R_END: u64 = R + (4 << 20);

struct V<'a>(&'a Sim);

impl TileVerbs for V<'_> {
    fn reserve(&self, lo: u64, hi: u64, base: Base, attr: Attr) -> Result<u32, String> {
        self.0.0.borrow_mut().reserve_tile(
            lo,
            hi - lo,
            base == Base::Sparse,
            attr == Attr::BigForced,
        )
    }
    fn free(&self, handle: u32) -> Result<(), String> {
        self.0.0.borrow_mut().free(handle)
    }
}

/// A space with no static guest reservation (the engine owns the range).
fn bare() -> Sim {
    Sim::new(VaSpace {
        space: 1,
        range: RANGE,
        guest: [GuestVaRange::default(); 3],
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum G {
    Invalid,
    Sparse,
    Mapped(u64),
}

/// The guest side: page states, live rows, sparse resources (tile-aligned spans).
struct Guest {
    pages: BTreeMap<u64, G>,
    rows: BTreeMap<u64, u64>,
}

impl Guest {
    fn new() -> Self {
        let pages = (R..R_END)
            .step_by(P as usize)
            .map(|p| (p, G::Invalid))
            .collect();
        Self {
            pages,
            rows: BTreeMap::new(),
        }
    }
    fn set(&mut self, a: u64, b: u64, g: impl Fn(u64) -> G) {
        for p in (a..b).step_by(P as usize) {
            self.pages.insert(p, g(p));
        }
    }
    /// The tile's wished base: INVALID if any page is guest-INVALID, else SPARSE if any is SPARSE,
    /// else (fully mapped) `None`.
    fn tile(&self, t: u64) -> Option<Base> {
        let st: Vec<G> = (t..t + TILE)
            .step_by(P as usize)
            .map(|p| self.pages[&p])
            .collect();
        if st.contains(&G::Invalid) {
            Some(Base::Invalid)
        } else if st.contains(&G::Sparse) {
            Some(Base::Sparse)
        } else {
            None
        }
    }
}

struct World {
    sim: Sim,
    t: Tiling,
    g: Guest,
}

impl World {
    fn new(cfg: TilingCfg) -> Self {
        let sim = bare();
        let t = Tiling::new(&V(&sim), R, R_END, cfg).expect("tile the range");
        Self {
            sim,
            t,
            g: Guest::new(),
        }
    }

    fn calls(&self) -> u64 {
        let n = self.sim.0.borrow().n;
        n.rm_reserve + n.rm_free
    }

    /// The guest sets `[a, b)` (unmapped pages) to SPARSE or INVALID.
    fn set(&mut self, a: u64, b: u64, base: Base) -> Result<(), TileError> {
        let g = if base == Base::Sparse {
            G::Sparse
        } else {
            G::Invalid
        };
        let pages = self.g.pages.clone();
        self.g.set(a, b, |p| match pages[&p] {
            G::Mapped(o) => G::Mapped(o),
            _ => g,
        });
        self.t.set_base(&V(&self.sim), a, b - a, base)
    }

    /// The guest maps `[va, va+len)` to RAM at `off`; the guard covers every OTHER live row.
    fn map(&mut self, va: u64, len: u64, off: u64) -> Result<(), TileError> {
        self.arm_guard(None);
        let pieces = self.t.prepare_row(&V(&self.sim), va, len)?;
        let mut done = Vec::new();
        for &(h, pv, pl) in &pieces {
            let r = self
                .sim
                .0
                .borrow_mut()
                .map_through(h, pv, pl, RAM_OBJ, off + (pv - va));
            if r.is_err() {
                for &(h, pv) in &done {
                    let _ = self.sim.0.borrow_mut().unmap_whole_in(h, pv, true);
                }
                return Err(TileError::Refused(format!("map {pv:#x}")));
            }
            done.push((h, pv));
        }
        self.t.commit_row(va, len)?;
        self.g.rows.insert(va, va + len);
        self.g.set(va, va + len, |p| G::Mapped(off + (p - va)));
        self.disarm_guard();
        Ok(())
    }

    /// The guest unmaps the row at `va`; its pages become `to`.
    fn unmap(&mut self, va: u64, to: Base) -> Result<(), TileError> {
        self.arm_guard(Some(va));
        let end = self.g.rows.remove(&va).expect("a live guest row");
        for (h, pv, _) in self.t.row_pieces(va) {
            self.sim
                .0
                .borrow_mut()
                .unmap_whole_in(h, pv, true)
                .expect("unmap a piece");
        }
        let g = if to == Base::Sparse {
            G::Sparse
        } else {
            G::Invalid
        };
        self.g.set(va, end, |_| g);
        let r1 = self.t.set_base(&V(&self.sim), va, end - va, to);
        let r2 = self.t.release_row(&V(&self.sim), va);
        self.disarm_guard();
        r1.and(r2)
    }

    fn arm_guard(&self, except: Option<u64>) {
        let mut rm = self.sim.0.borrow_mut();
        rm.guard = self
            .g
            .rows
            .iter()
            .filter(|&(&a, _)| Some(a) != except)
            .flat_map(|(&a, &b)| (a..b).step_by(P as usize))
            .filter_map(|p| match self.g.pages[&p] {
                G::Mapped(o) => Some((p, (true, o))),
                _ => None,
            })
            .collect();
    }

    fn disarm_guard(&self) {
        self.sim.0.borrow_mut().guard.clear();
    }

    /// Exact tiling: the engine's blocks are the host's reservations, one for one, unrounded.
    fn check_tiling(&self) -> Result<(), String> {
        self.t.check()?;
        let rm = self.sim.0.borrow();
        if rm.resv.len() != self.t.blocks().len() {
            return Err(format!(
                "host holds {} reservations, the engine {} blocks",
                rm.resv.len(),
                self.t.blocks().len()
            ));
        }
        for (&k, b) in self.t.blocks() {
            let got = rm.resv.get(&b.handle).copied();
            if got != Some((k, b.hi)) {
                return Err(format!(
                    "block {k:#x}..{:#x} is {got:x?} on the host (rounded or lost)",
                    b.hi
                ));
            }
            if rm.sparse_resv.contains(&b.handle) != (b.base == Base::Sparse) {
                return Err(format!("block {k:#x}: base differs on the host"));
            }
        }
        if !rm.transient.is_empty() || !rm.violations.is_empty() {
            return Err(format!(
                "guard: {:?} {:?}",
                rm.transient.first(),
                rm.violations.first()
            ));
        }
        Ok(())
    }

    /// The per-page oracle over `[a, b)`. Returns (S1 misses, S2 misses), each listing pages; a miss
    /// that the engine does not report as pending is an error, and so is any wrong mapping.
    fn oracle(&self, a: u64, b: u64) -> Result<(Vec<u64>, Vec<u64>), String> {
        let (a, b) = (a.max(R) / TILE * TILE, b.min(R_END).div_ceil(TILE) * TILE);
        let pending = self.t.pending();
        let in_pending = |p: u64| pending.iter().any(|&(x, y, _)| x <= p && p < y);
        let rm = self.sim.0.borrow();
        let (mut s1, mut s2) = (Vec::new(), Vec::new());
        for t in (a..b).step_by(TILE as usize) {
            let want = self.g.tile(t);
            for p in (t..t + TILE).step_by(P as usize) {
                let host = rm.host_pte(p);
                match (self.g.pages[&p], want) {
                    (G::Mapped(o), _) => {
                        if host != HostPte::Mapped((true, o)) {
                            return Err(format!("mapped page {p:#x}: host {host:?}"));
                        }
                    }
                    (_, Some(Base::Sparse)) if host != HostPte::Sparse => s1.push(p),
                    (_, Some(Base::Invalid)) if host != HostPte::Invalid => s2.push(p),
                    _ => {}
                }
            }
        }
        for &p in s1.iter().chain(&s2) {
            if !in_pending(p) {
                return Err(format!(
                    "page {p:#x} misses its wish but the engine does not report it pending"
                ));
            }
        }
        Ok((s1, s2))
    }

    /// Whether the engine's blocks cover the whole range (no gap).
    fn covered(&self) -> bool {
        let mut cur = R;
        for (&k, b) in self.t.blocks() {
            if k != cur {
                return false;
            }
            cur = b.hi;
        }
        cur == R_END
    }

    /// The engine's blocks and rows around `[a, b)`, for a failure message.
    fn dump(&self, a: u64, b: u64) -> String {
        let blocks: Vec<String> = self
            .t
            .blocks()
            .iter()
            .filter(|(k, blk)| blk.hi + 4 * TILE > a && **k < b + 4 * TILE)
            .map(|(k, blk)| format!("{k:#x}..{:#x}:{:?}", blk.hi, blk.base))
            .collect();
        let rows: Vec<String> = self
            .g
            .rows
            .iter()
            .filter(|(k, e)| **e + 4 * TILE > a && **k < b + 4 * TILE)
            .map(|(k, e)| format!("{k:#x}..{e:#x}"))
            .collect();
        format!(
            "blocks {blocks:?} rows {rows:?} pending {:x?} stats {:?}",
            self.t.pending(),
            self.t.stats
        )
    }

    /// Strict: exact tiling, and S1 and S2 with no miss at all over the whole range.
    fn strict(&self) {
        self.check_tiling().unwrap_or_else(|e| panic!("{e}"));
        let (s1, s2) = self.oracle(R, R_END).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            s1.is_empty(),
            "S1 misses: {} pages, first {:#x}",
            s1.len(),
            s1[0]
        );
        assert!(
            s2.is_empty(),
            "S2 misses: {} pages, first {:#x}",
            s2.len(),
            s2[0]
        );
    }
}

const SLOT: u64 = TILE;

/// ★ **S1 under C2 (the residency pattern of the CTS buffer case).** 32 slots of 64 KiB in one sparse
/// resource, even slots bound, odd slots left SPARSE: every odd slot reads zero on the host, every even
/// slot is mapped exactly, and the rest of the range still FAULTS (S2).
#[test]
fn c2_s1_residency_slots_are_sparse_and_s2_holds_around_them() {
    let mut w = World::new(TilingCfg::default());
    let base = R + (1 << 20);
    w.set(base, base + 32 * SLOT, Base::Sparse).unwrap();
    for i in (0..32).step_by(2) {
        w.map(base + i * SLOT, SLOT, 0x4000_0000 + i * SLOT)
            .unwrap();
    }
    w.strict();
    let rm = w.sim.0.borrow();
    for i in (1..32).step_by(2) {
        assert_eq!(
            rm.host_pte(base + i * SLOT + 0x800),
            HostPte::Sparse,
            "odd slot {i}"
        );
    }
    assert_eq!(
        rm.host_pte(base - P),
        HostPte::Invalid,
        "below the resource"
    );
    assert_eq!(
        rm.host_pte(base + 32 * SLOT),
        HostPte::Invalid,
        "above the resource"
    );
}

/// ★ S1 after an unbind back to SPARSE: the slot reads zero again (an unmap inside a `_SPARSE`
/// block returns to sparse) with no re-tile at all.
#[test]
fn c2_s1_an_unbound_slot_returns_to_sparse_without_a_retile() {
    let mut w = World::new(TilingCfg::default());
    let base = R + (1 << 20);
    w.set(base, base + 4 * SLOT, Base::Sparse).unwrap();
    w.map(base, SLOT, 0x5000_0000).unwrap();
    let before = w.calls();
    w.unmap(base, Base::Sparse).unwrap();
    w.strict();
    assert_eq!(w.sim.0.borrow().host_pte(base), HostPte::Sparse);
    // The row block merges back into its sparse neighbour: at most free 2 + reserve 1.
    assert!(
        w.calls() - before <= 3,
        "unbind cost {} host calls",
        w.calls() - before
    );
}

/// ★ **S2 under C2.** Rows mapped and plainly unmapped outside any sparse resource, and VAs never
/// mapped, all FAULT on the host; the tiling returns to one fence.
#[test]
fn c2_s2_guest_invalid_vas_stay_invalid() {
    let mut w = World::new(TilingCfg::default());
    w.map(R, 4 * P, 0x9000).unwrap();
    w.map(R + (2 << 20) + 3 * P, 40 * P, 0x20_0000).unwrap();
    w.strict();
    w.unmap(R, Base::Invalid).unwrap();
    w.unmap(R + (2 << 20) + 3 * P, Base::Invalid).unwrap();
    w.strict();
    let rm = w.sim.0.borrow();
    for p in [R, R + 3 * P, R + (2 << 20), R_END - P] {
        assert_eq!(rm.host_pte(p), HostPte::Invalid, "{p:#x}");
    }
    assert_eq!(
        w.t.blocks().len(),
        1,
        "everything merged back into one fence"
    );
}

/// A sparse run at the very start, at the very end, and over the whole range.
#[test]
fn c2_sparse_runs_at_the_fence_edges() {
    let mut w = World::new(TilingCfg::default());
    w.set(R, R + 3 * TILE, Base::Sparse).unwrap();
    w.set(R_END - 5 * TILE, R_END, Base::Sparse).unwrap();
    w.strict();
    assert_eq!(w.t.blocks().len(), 3);
    w.set(R, R_END, Base::Sparse).unwrap();
    w.strict();
    assert_eq!(w.t.blocks().len(), 1, "the whole range is one sparse block");
    w.set(R, R_END, Base::Invalid).unwrap();
    w.strict();
    assert_eq!(w.t.blocks().len(), 1);
}

/// Two adjacent sparse runs become ONE host block; freeing one splits it again exactly.
#[test]
fn c2_adjacent_sparse_runs_merge() {
    let mut w = World::new(TilingCfg::default());
    let a = R + 7 * TILE;
    w.set(a, a + 4 * TILE, Base::Sparse).unwrap();
    w.set(a + 4 * TILE, a + 9 * TILE, Base::Sparse).unwrap();
    w.strict();
    let sparse: Vec<_> =
        w.t.blocks()
            .iter()
            .filter(|(_, b)| b.base == Base::Sparse)
            .collect();
    assert_eq!(sparse.len(), 1, "{sparse:x?}");
    assert_eq!((*sparse[0].0, sparse[0].1.hi), (a, a + 9 * TILE));
    w.set(a, a + 4 * TILE, Base::Invalid).unwrap();
    w.strict();
    assert_eq!(w.t.blocks().len(), 3);
}

/// Remap, unmap, re-sparse, free: the full lifecycle of one sparse resource with rows in and around it.
#[test]
fn c2_remap_unmap_resparse_sequence() {
    let mut w = World::new(TilingCfg::default());
    let a = R + (1 << 20) + 2 * TILE;
    w.set(a, a + 8 * TILE, Base::Sparse).unwrap();
    w.map(a - 4 * P, 4 * P, 0x1000).unwrap(); // a normal row just below the resource
    w.map(a + 2 * TILE, 2 * TILE, 0x10_0000).unwrap();
    w.strict();
    w.unmap(a + 2 * TILE, Base::Sparse).unwrap();
    w.map(a + 3 * TILE, TILE, 0x30_0000).unwrap();
    w.strict();
    w.unmap(a + 3 * TILE, Base::Sparse).unwrap();
    w.set(a, a + 8 * TILE, Base::Invalid).unwrap(); // the guest frees the resource
    w.strict();
    w.unmap(a - 4 * P, Base::Invalid).unwrap();
    w.strict();
    assert_eq!(w.t.blocks().len(), 1);
}

/// A SPARSE run that covers only part of a tile leaves that tile INVALID (inward), and an INVALID run
/// over part of a sparse tile makes the whole tile INVALID (outward): INVALID wins, so S2 never breaks.
#[test]
fn c2_partial_tiles_resolve_to_invalid() {
    let mut w = World::new(TilingCfg::default());
    let a = R + 10 * TILE;
    w.set(a + P, a + 3 * TILE - P, Base::Sparse).unwrap();
    w.strict();
    let rm = w.sim.0.borrow();
    assert_eq!(rm.host_pte(a + P), HostPte::Invalid, "partial first tile");
    assert_eq!(
        rm.host_pte(a + TILE),
        HostPte::Sparse,
        "the whole middle tile"
    );
    assert_eq!(
        rm.host_pte(a + 3 * TILE - 2 * P),
        HostPte::Invalid,
        "partial last tile"
    );
    drop(rm);
    w.set(a + TILE + P, a + TILE + 2 * P, Base::Invalid)
        .unwrap();
    w.strict();
    assert_eq!(w.sim.0.borrow().host_pte(a + TILE), HostPte::Invalid);
}

/// Blocks of 2 MiB or more are never requested where the host would round them: with
/// `_ATTR_PAGE_SIZE_BIG` fences, and without (an aligned core plus edges).
#[test]
fn c2_two_mib_rounding_is_never_requested() {
    for big_forced in [true, false] {
        let cfg = TilingCfg {
            big_forced,
            ..TilingCfg::default()
        };
        let mut w = World::new(cfg);
        let a = R + 3 * TILE;
        w.set(a, a + (2 << 20) + 5 * TILE, Base::Sparse).unwrap();
        w.map(R + TILE, 40 * TILE, 0x8000_0000).unwrap();
        w.strict();
        w.unmap(R + TILE, Base::Invalid).unwrap();
        w.strict();
    }
}

/// A host refusal in the middle of a re-tile rolls the step back: the layout stays exact and S2
/// holds; the S1 work is pending, and the next batch completes it.
#[test]
fn c2_a_refused_reserve_mid_retile_rolls_back() {
    let mut w = World::new(TilingCfg::default());
    let a = R + (1 << 20);
    w.sim.0.borrow_mut().ext = Some(ExtFault {
        reserve_every: 2,
        ..ExtFault::default()
    });
    let r = w.set(a, a + 4 * TILE, Base::Sparse);
    assert!(matches!(r, Err(TileError::Refused(_))), "{r:?}");
    assert!(w.t.stats.rollbacks >= 1);
    assert_eq!(
        w.t.stats.gaps_left, 0,
        "the rollback restored the old layout"
    );
    w.check_tiling().unwrap();
    assert!(w.covered(), "no gap after a rollback");
    let (s1, s2) = w.oracle(R, R_END).unwrap();
    assert!(s2.is_empty(), "S2 after a rollback: {s2:x?}");
    assert!(!s1.is_empty(), "the sparse run is pending");
    w.sim.0.borrow_mut().ext = None;
    w.t.begin_batch(&V(&w.sim));
    w.strict();
}

/// A refusal of the RESTORE itself leaves an unreserved gap: it reads INVALID (S2 holds) and the next
/// call heals it.
#[test]
fn c2_a_refused_restore_leaves_a_gap_that_heals() {
    let mut w = World::new(TilingCfg::default());
    let a = R + (1 << 20);
    // Reserve 1 (the new left remnant) OK, reserve 2 refused, then the rollback's restore refused.
    w.sim.0.borrow_mut().ext = Some(ExtFault {
        reserve_every: 2,
        ..ExtFault::default()
    });
    w.sim.0.borrow_mut().ext.as_mut().unwrap().seen[0] = 0;
    let _ = w.set(a, a + 4 * TILE, Base::Sparse);
    w.sim.0.borrow_mut().refuse_reserve = true;
    let _ = w.set(a + 8 * TILE, a + 9 * TILE, Base::Sparse);
    let rm = w.sim.0.borrow();
    for p in (R..R_END).step_by(TILE as usize) {
        if w.g.tile(p) == Some(Base::Invalid) {
            assert_eq!(rm.host_pte(p), HostPte::Invalid, "S2 at {p:#x} with a gap");
        }
    }
    drop(rm);
    let mut rm = w.sim.0.borrow_mut();
    rm.refuse_reserve = false;
    rm.ext = None;
    drop(rm);
    w.t.begin_batch(&V(&w.sim));
    w.strict();
}

/// A budget cut: SPARSE work beyond the batch budget is pending (S1 loud), S2 work is never cut, and
/// the next batch completes it.
#[test]
fn c2_a_budget_cut_keeps_s2_and_defers_s1() {
    let cfg = TilingCfg {
        batch_calls: 4,
        ..TilingCfg::default()
    };
    let mut w = World::new(cfg);
    w.t.begin_batch(&V(&w.sim));
    for i in 0..6 {
        let a = R + (2 * i + 1) * 4 * TILE;
        w.set(a, a + 2 * TILE, Base::Sparse).unwrap();
    }
    let (s1, s2) = w.oracle(R, R_END).unwrap();
    assert!(s2.is_empty());
    assert!(!s1.is_empty(), "the budget cut some sparse runs");
    // An S2 restore over cut work is done even with the budget spent.
    w.set(R, R_END, Base::Invalid).unwrap();
    let (_, s2) = w.oracle(R, R_END).unwrap();
    assert!(s2.is_empty(), "S2 is never cut");
    for i in 0..6 {
        let a = R + (2 * i + 1) * 4 * TILE;
        w.set(a, a + 2 * TILE, Base::Sparse).unwrap();
    }
    for _ in 0..8 {
        w.t.begin_batch(&V(&w.sim));
    }
    w.strict();
}

/// ★ The one S2 hazard C2 cannot fix in place, made visible: a MIXED tile (a 4 KiB row bound inside a
/// sparse tile, and a neighbouring page of the same tile returned to INVALID). The row's block is
/// pinned, so the tile stays SPARSE until the row goes; the engine reports it pending, and the
/// release fixes it.
#[test]
fn c2_a_mixed_tile_is_pending_and_fixed_when_the_row_goes() {
    let mut w = World::new(TilingCfg::default());
    let a = R + (1 << 20);
    w.set(a, a + TILE, Base::Sparse).unwrap();
    w.map(a, P, 0x7000).unwrap();
    w.map(a + P, P, 0x8000).unwrap();
    w.unmap(a + P, Base::Invalid).unwrap();
    let (_, s2) = w.oracle(R, R_END).unwrap();
    assert!(
        !s2.is_empty(),
        "a mixed tile under a live row stays SPARSE (pending)"
    );
    w.unmap(a, Base::Sparse).unwrap();
    w.strict();
}

/// At the block cap a row is mapped through its empty block without a carve (pinned): mappings and
/// S2 stay right, and the count never passes the cap.
#[test]
fn c2_the_block_cap_pins_rows_and_stays_correct() {
    let cfg = TilingCfg {
        max_blocks: 6,
        ..TilingCfg::default()
    };
    let mut w = World::new(cfg);
    for i in 0..8 {
        w.map(R + i * 3 * TILE + P, P, 0x1000 * (i + 1)).unwrap();
    }
    w.check_tiling().unwrap();
    assert!(w.t.blocks().len() <= 6);
    assert!(w.t.stats.pinned > 0);
    let (_, s2) = w.oracle(R, R_END).unwrap();
    assert!(s2.is_empty());
}

/// Hostile requests are refused by name before any host call.
#[test]
fn c2_hostile_requests_are_refused_before_any_host_call() {
    let mut w = World::new(TilingCfg::default());
    let n0 = w.calls();
    let v = V(&w.sim);
    assert_eq!(
        w.t.set_base(&v, u64::MAX - P, 2 * P, Base::Sparse),
        Err(TileError::Outside)
    );
    assert_eq!(
        w.t.set_base(&v, R - TILE, TILE, Base::Sparse),
        Err(TileError::Outside)
    );
    assert_eq!(
        w.t.set_base(&v, R, 0, Base::Sparse),
        Err(TileError::Outside)
    );
    assert_eq!(
        w.t.prepare_row(&v, R_END - P, 2 * P),
        Err(TileError::Outside)
    );
    assert_eq!(w.t.release_row(&v, R), Err(TileError::NotARow));
    assert_eq!(w.calls(), n0);
    assert!(Tiling::new(&v, R + P, R_END, TilingCfg::default()).is_err());
}

/// One random op on the world. Sparse resources are tile-aligned spans with no row crossing their
/// edge; a row inside one is unbound back to SPARSE, any other row is plainly unmapped.
fn step(
    w: &mut World,
    rng: &mut Rng,
    res: &mut BTreeMap<u64, u64>,
) -> Result<(u64, u64), TileError> {
    let tiles = (R_END - R) / TILE;
    let in_res = |res: &BTreeMap<u64, u64>, a: u64, b: u64| {
        res.range(..b).next_back().is_some_and(|(_, &e)| e > a)
    };
    let free_rows =
        |g: &Guest, a: u64, b: u64| !g.rows.range(..b).next_back().is_some_and(|(_, &e)| e > a);
    match rng.below(5) {
        0 => {
            let n = if rng.below(4) == 0 {
                33 + rng.below(8)
            } else {
                1 + rng.below(6)
            };
            let a = R + rng.below(tiles - n.min(tiles - 1)) * TILE;
            let b = (a + n * TILE).min(R_END);
            if !in_res(res, a, b) && free_rows(&w.g, a, b) {
                res.insert(a, b);
                w.set(a, b, Base::Sparse)?;
                return Ok((a, b));
            }
        }
        1 => {
            let keys: Vec<(u64, u64)> = res.iter().map(|(&a, &b)| (a, b)).collect();
            if let Some(&(a, b)) = keys.get(rng.below(keys.len().max(1) as u64) as usize)
                && free_rows(&w.g, a, b)
            {
                res.remove(&a);
                w.set(a, b, Base::Invalid)?;
                return Ok((a, b));
            }
        }
        2 | 3 => {
            let va = R + rng.below((R_END - R) / P) * P;
            let len = if rng.below(6) == 0 {
                (32 + rng.below(40)) * TILE
            } else {
                (1 + rng.below(24)) * P
            };
            let Some(end) = va.checked_add(len).filter(|&e| e <= R_END) else {
                return Ok((R, R));
            };
            let inside = res.range(..=va).next_back().is_some_and(|(_, &e)| end <= e);
            let outside = !in_res(res, va / TILE * TILE, end.div_ceil(TILE) * TILE);
            if (inside || outside) && free_rows(&w.g, va, end) {
                w.map(va, len, rng.below(1 << 20) * P)?;
                return Ok((va, end));
            }
        }
        _ => {
            let keys: Vec<(u64, u64)> = w.g.rows.iter().map(|(&a, &b)| (a, b)).collect();
            if let Some(&(a, b)) = keys.get(rng.below(keys.len().max(1) as u64) as usize) {
                let to = if in_res(res, a, b) {
                    Base::Sparse
                } else {
                    Base::Invalid
                };
                w.unmap(a, to)?;
                return Ok((a, b));
            }
        }
    }
    Ok((R, R))
}

/// ★ **Model-based random test against the brute-force oracle** (seeded xorshift): after every op the
/// tiling is exact (engine = host, unrounded, no leak), the guard held through every host call,
/// mapped pages translate exactly, and S1 and S2 hold on every page with NO miss; the host calls of
/// one op are bounded by the blocks it touches, not by the range.
#[test]
fn c2_random_ops_keep_exact_tiling_s1_and_s2() {
    for seed in 1..=24u64 {
        for big_forced in [true, false] {
            let cfg = TilingCfg {
                big_forced,
                ..TilingCfg::default()
            };
            let mut w = World::new(cfg);
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut res = BTreeMap::new();
            for op in 0..250 {
                let n0 = w.calls();
                let touched_before = w.t.blocks().len();
                let (a, b) = step(&mut w, &mut rng, &mut res)
                    .unwrap_or_else(|e| panic!("seed {seed} big={big_forced} op {op}: {e:?}"));
                let cost = w.calls() - n0;
                // Bounded by the blocks around the op (and the pieces their layouts need), never by
                // the size of the range.
                let bound = 6 * (touched_before.min(64) as u64 + 8);
                assert!(
                    cost <= bound,
                    "seed {seed} op {op}: {cost} host calls for {a:#x}..{b:#x}"
                );
                w.check_tiling()
                    .unwrap_or_else(|e| panic!("seed {seed} big={big_forced} op {op}: {e}"));
                let (lo, hi) = if op % 25 == 0 {
                    (R, R_END)
                } else {
                    (a.saturating_sub(TILE), b + TILE)
                };
                let (s1, s2) = w
                    .oracle(lo, hi)
                    .unwrap_or_else(|e| panic!("seed {seed} big={big_forced} op {op}: {e}"));
                assert!(
                    s1.is_empty() && s2.is_empty(),
                    "seed {seed} big={big_forced} op {op} ({a:#x}..{b:#x}): S1 misses {:x?} S2 misses {:x?}\n{}",
                    s1.first(),
                    s2.first(),
                    w.dump(a, b)
                );
            }
            w.strict();
        }
    }
}

/// ★ The same random workload with host refusals injected (every 7th reserve, every 11th free) and a
/// small batch budget: the layout stays exact, mapped pages stay mapped, S2 holds wherever no row pins
/// a mixed tile, every miss is reported pending, and once the faults stop the next batches heal
/// everything.
#[test]
fn c2_random_ops_with_refusals_and_budget_cuts_stay_consistent() {
    for seed in 1..=16u64 {
        let cfg = TilingCfg {
            batch_calls: 12,
            ..TilingCfg::default()
        };
        let mut w = World::new(cfg);
        let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
        let mut res = BTreeMap::new();
        w.sim.0.borrow_mut().ext = Some(ExtFault {
            reserve_every: 7,
            free_every: 11,
            ..ExtFault::default()
        });
        for op in 0..200 {
            if op % 10 == 0 {
                w.t.begin_batch(&V(&w.sim));
            }
            let _ = step_tolerant(&mut w, &mut rng, &mut res);
            w.check_tiling()
                .unwrap_or_else(|e| panic!("seed {seed} op {op}: {e}"));
            assert!(
                w.t.stats.gaps_left > 0 || w.covered(),
                "seed {seed} op {op}: a gap that no refused restore explains"
            );
            // Every miss is reported pending (the oracle checks it); a miss right after a refusal
            // is the host's answer, healed by a later batch (checked below).
            w.oracle(R, R_END)
                .unwrap_or_else(|e| panic!("seed {seed} op {op}: {e}"));
        }
        w.sim.0.borrow_mut().ext = None;
        for _ in 0..64 {
            w.t.begin_batch(&V(&w.sim));
        }
        w.check_tiling().unwrap();
        // What is left can only be a block PINNED by a live row (a row placed without a carve while
        // the budget was cut): it is fixed when that row goes.
        let (s1, s2) = w.oracle(R, R_END).unwrap();
        for p in s1.iter().chain(&s2) {
            let (k, blk) =
                w.t.blocks()
                    .range(..=*p)
                    .next_back()
                    .map(|(k, b)| (*k, *b))
                    .unwrap();
            assert!(
                w.g.rows
                    .range(..blk.hi)
                    .next_back()
                    .is_some_and(|(_, &e)| e > k),
                "seed {seed}: miss at {p:#x} after healing with no row pinning its block\n{}",
                w.dump(*p, p + TILE)
            );
        }
        let rows: Vec<(u64, u64)> = w.g.rows.iter().map(|(&a, &b)| (a, b)).collect();
        for (a, b) in rows {
            let inside = res.range(..=a).next_back().is_some_and(|(_, &e)| b <= e);
            w.unmap(a, if inside { Base::Sparse } else { Base::Invalid })
                .unwrap();
        }
        for _ in 0..64 {
            w.t.begin_batch(&V(&w.sim));
        }
        w.strict();
    }
}

/// [`step`] for the fault-injection test: a refusal is an answer, not a test failure (the engine keeps
/// the guest's wish and retries; a refused row map is not a live row).
fn step_tolerant(
    w: &mut World,
    rng: &mut Rng,
    res: &mut BTreeMap<u64, u64>,
) -> Result<(), TileError> {
    let r = step(w, rng, res).map(|_| ());
    w.disarm_guard();
    r
}

/// ★ S2 is mandatory: at the block cap, freeing a sparse resource still restores INVALID at once.
#[test]
fn c2_s2_ignores_the_block_cap() {
    let cfg = TilingCfg {
        max_blocks: 3,
        ..TilingCfg::default()
    };
    let mut w = World::new(cfg);
    let a = R + 8 * TILE;
    w.set(a, a + 8 * TILE, Base::Sparse).unwrap();
    assert_eq!(w.t.blocks().len(), 3);
    w.set(a + 2 * TILE, a + 3 * TILE, Base::Invalid).unwrap(); // would split the sparse block: over the cap
    let (_, s2) = w.oracle(R, R_END).unwrap();
    assert!(s2.is_empty(), "S2 at the cap: {:x?}", s2.first());
}

/// A row pins only its own tiles: a sparse resource right next to a normal row (other tiles) is SPARSE.
#[test]
fn c2_a_row_pins_only_its_own_tiles() {
    let mut w = World::new(TilingCfg::default());
    w.map(R + 5 * TILE + P, 3 * P, 0x2000).unwrap();
    w.set(R + 6 * TILE, R + 9 * TILE, Base::Sparse).unwrap();
    w.set(R + TILE, R + 5 * TILE, Base::Sparse).unwrap();
    w.strict();
}

/// A tile two rows share is a block of its own: after one row goes, the other pins only that tile,
/// and a sparse resource over the freed tiles is SPARSE.
#[test]
fn c2_a_shared_edge_tile_does_not_pin_its_neighbour_tiles() {
    let mut w = World::new(TilingCfg::default());
    let t = R + 16 * TILE;
    w.map(t + TILE - 2 * P, 2 * TILE, 0x1_0000).unwrap(); // tiles 0..=2, sharing tile 0
    w.map(t + P, 4 * P, 0x9_0000).unwrap(); // tile 0 only
    w.unmap(t + TILE - 2 * P, Base::Invalid).unwrap();
    w.set(t + TILE, t + 3 * TILE, Base::Sparse).unwrap();
    w.strict();
}

/// ★ A refused S2 restore (the host refused freeing the sparse block) is retried FIRST by the next
/// batch, whatever is left of its budget, and S2 holds after it.
#[test]
fn c2_a_refused_s2_restore_is_retried_first() {
    let cfg = TilingCfg {
        batch_calls: 4,
        ..TilingCfg::default()
    };
    let mut w = World::new(cfg);
    let a = R + 8 * TILE;
    w.t.begin_batch(&V(&w.sim));
    w.set(a, a + 4 * TILE, Base::Sparse).unwrap();
    assert_eq!(w.sim.0.borrow().host_pte(a), HostPte::Sparse);
    w.sim.0.borrow_mut().ext = Some(ExtFault {
        free_every: 1,
        ..ExtFault::default()
    });
    assert!(w.set(a, a + 4 * TILE, Base::Invalid).is_err());
    let (_, s2) = w.oracle(R, R_END).unwrap();
    assert!(
        !s2.is_empty(),
        "the refused restore leaves the block SPARSE (pending)"
    );
    w.sim.0.borrow_mut().ext = None;
    // Fill the pending list with S1 work that the budget cannot all take.
    for i in 0..4 {
        let b = R + (20 + 3 * i) * TILE;
        w.set(b, b + TILE, Base::Sparse).unwrap();
    }
    w.t.begin_batch(&V(&w.sim));
    let (_, s2) = w.oracle(R, R_END).unwrap();
    assert!(s2.is_empty(), "S2 is retried first: {:x?}", s2.first());
}
