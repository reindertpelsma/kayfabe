//! ADVERSARIAL REVIEW TESTS (review branch `review/batched-map-adversarial-20261010`, of
//! 2c6c0faa..833a6f5a). Not part of the change under review.
//!
//! What these add over `sim/tests.rs`:
//! - the WALKER is the real commit-on-ack spec (`kf_cuda::diffmodel::{diff, commit}`), per class,
//!   so overlapping committed placements are representable (the in-tree model's `BTreeMap` keyed by
//!   VA silently overwrites them);
//! - host-call ERROR INJECTION at every call index (`Faulty`), which the in-tree model never does
//!   (its `unmap_range` never fails);
//! - a guard keyed on the TRANSLATION only (backing + permission + kind), not on the leaf size.
//!
//! ★ 2026-10-10 (branch `claude/batched-map-review-fixes-20261010`): every test that FAILED at
//! 6fafcc6e passes and is in the DEFAULT suite (`// FAILED at 6fafcc6e …`); the huge-row test is
//! rewritten for the new bound (one mapping per leaf up to `MAX_LEAF_PIECES`, beyond it a micro
//! reservation or a refusal by name), and section 6 adds targeted tests for findings 4, 5, 6.
//! Runtime: `adversarial_error_injection_at_every_call_index` is the long one: 4 924
//! injected runs, 20.4 s in a debug build (measured on the build host, 2026-10-10);
//! everything else is sub-second. ⊘ Was: "Tests that FAIL at 6fafcc6e are marked `// EXPECTED TO
//! FAIL at 6fafcc6e` and are `#[ignore]`d so the default suite stays green; run them with `cargo
//! test -p kf-mem adversarial -- --ignored`." Only `adversarial_debug_replay` is still ignored (it
//! needs its environment).

use super::*;
use crate::apply::PermPolicy;
use kf_cuda::abi::{
    KFWR_ACK_APPLIED, KFWR_ACK_FAILED, KFWR_ACK_HELD, KFWR_OP_MAP, KFWR_RF_PS_SHIFT,
    KFWR_RF_READ_ONLY, KfMapRun,
};
use kf_cuda::diffmodel::{self, AckCode};
use std::cell::Cell;

const BASE: u64 = 0x100_0000;
const PAGES: u64 = 96;
const RESV_LO: u64 = BASE + 32 * P;
const RESV_HI: u64 = BASE + 64 * P;

fn pg(i: u64) -> u64 {
    BASE + i * P
}

// ─── a SpaceVerbs wrapper that fails the N-th host call ────────────────────────────────────────

/// How an injected failure behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Inject {
    /// RM refuses before acting (nothing changes).
    Before,
    /// For range unmaps only: RM acted (all of it) and still answered an error — the superset of
    /// `serverInterUnmapInternal`'s mid-loop `goto done` (earlier mappings already gone).
    AfterRange,
}

struct Faulty<'a> {
    sim: &'a Sim,
    calls: Cell<u64>,
    fail_at: Cell<Option<u64>>,
    mode: Inject,
    fired: Cell<bool>,
}

impl<'a> Faulty<'a> {
    fn new(sim: &'a Sim) -> Self {
        Faulty {
            sim,
            calls: Cell::new(0),
            fail_at: Cell::new(None),
            mode: Inject::Before,
            fired: Cell::new(false),
        }
    }
    /// `true` = this call fails.
    fn tick(&self) -> bool {
        let n = self.calls.get();
        self.calls.set(n + 1);
        if self.fail_at.get() == Some(n) {
            self.fired.set(true);
            return true;
        }
        false
    }
}

impl SpaceVerbs for &Faulty<'_> {
    fn map_row(&self, d: &Desired, defer: bool) -> Result<Mapped, String> {
        if self.tick() {
            return Err("injected map_row".into());
        }
        SpaceVerbs::map_row(&self.sim, d, defer)
    }
    fn map_sked_row(&self, s: &SkedRow, defer: bool) -> Result<Mapped, String> {
        if self.tick() {
            return Err("injected".into());
        }
        SpaceVerbs::map_sked_row(&self.sim, s, defer)
    }
    fn map_scattered(
        &self,
        fd: std::os::fd::BorrowedFd<'_>,
        rows: &[Desired],
        defer: bool,
    ) -> Result<u32, String> {
        if self.tick() {
            return Err("injected map_scattered".into());
        }
        SpaceVerbs::map_scattered(&self.sim, fd, rows, defer)
    }
    fn unmap_whole(&self, va: u64, defer: bool) -> Result<(), String> {
        if self.tick() {
            return Err("injected unmap_whole".into());
        }
        SpaceVerbs::unmap_whole(&self.sim, va, defer)
    }
    fn unmap_row(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        if self.tick() {
            return Err("injected unmap_row".into());
        }
        SpaceVerbs::unmap_row(&self.sim, va, len, defer)
    }
    fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        if self.tick() {
            if self.mode == Inject::AfterRange {
                let _ = SpaceVerbs::unmap_range(&self.sim, va, len, defer);
            }
            return Err("injected unmap_range".into());
        }
        SpaceVerbs::unmap_range(&self.sim, va, len, defer)
    }
    fn free(&self, h: u32) -> Result<(), String> {
        if self.tick() {
            return Err("injected free".into());
        }
        SpaceVerbs::free(&self.sim, h)
    }
    fn splits_safely(&self, va: u64, len: u64) -> bool {
        SpaceVerbs::splits_safely(&self.sim, va, len)
    }
    fn reserve(&self, va: u64, len: u64) -> Result<u32, String> {
        if self.tick() {
            return Err("injected reserve".into());
        }
        SpaceVerbs::reserve(&self.sim, va, len)
    }
    fn map_row_in(&self, h: u32, d: &Desired, defer: bool) -> Result<Mapped, String> {
        if self.tick() {
            return Err("injected map_row_in".into());
        }
        SpaceVerbs::map_row_in(&self.sim, h, d, defer)
    }
    fn map_sked_in(&self, h: u32, s: &SkedRow, defer: bool) -> Result<Mapped, String> {
        if self.tick() {
            return Err("injected".into());
        }
        SpaceVerbs::map_sked_in(&self.sim, h, s, defer)
    }
    fn map_scattered_in(
        &self,
        h: u32,
        fd: std::os::fd::BorrowedFd<'_>,
        rows: &[Desired],
        defer: bool,
    ) -> Result<u32, String> {
        if self.tick() {
            return Err("injected map_scattered_in".into());
        }
        SpaceVerbs::map_scattered_in(&self.sim, h, fd, rows, defer)
    }
    fn unmap_in(&self, h: u32, va: u64, size: u64, defer: bool) -> Result<(), String> {
        if self.tick() {
            if self.mode == Inject::AfterRange && size != 0 {
                let _ = SpaceVerbs::unmap_in(&self.sim, h, va, size, defer);
            }
            return Err("injected unmap_in".into());
        }
        SpaceVerbs::unmap_in(&self.sim, h, va, size, defer)
    }
}

/// `SimMirror`'s glue (= `GpuMirror`'s `MapTarget`), over the faulty verbs.
struct FMirror<'a> {
    bv: BatchedVas<'static, &'a Faulty<'a>>,
    rows: RefCell<BTreeMap<u64, u64>>,
    batching: bool,
}

impl<'a> FMirror<'a> {
    fn new(f: &'a Faulty<'a>, batching: bool, low: bool) -> Self {
        FMirror {
            bv: BatchedVas::with_low_reserve(f, low),
            rows: RefCell::new(BTreeMap::new()),
            batching,
        }
    }
}

impl MapTarget for FMirror<'_> {
    fn gpu_space(&self) -> bool {
        true
    }
    fn map(&self, d: &Desired, defer: bool) -> Result<Mapped, String> {
        let m = self.bv.map(d, defer);
        restore_owned_rows(&mut self.rows.borrow_mut(), &self.bv, d.va, d.va + d.len);
        if m == Ok(Mapped::Placed) {
            self.rows.borrow_mut().insert(d.va, d.len);
        }
        m
    }
    fn map_batch(&self, rows: &[Desired], defer: bool) -> Result<(), String> {
        if !self.batching {
            return Err(NOT_BATCHED.into());
        }
        let res = self
            .bv
            .place(std::os::fd::AsFd::as_fd(&std::io::stdin()), rows, defer);
        if let (Some(a), Some(b)) = (rows.first(), rows.last()) {
            restore_owned_rows(&mut self.rows.borrow_mut(), &self.bv, a.va, b.va + b.len);
        }
        res?;
        let mut r = self.rows.borrow_mut();
        for d in rows {
            r.insert(d.va, d.len);
        }
        Ok(())
    }
    fn unmap(&self, va: u64, defer: bool) -> Result<(), String> {
        let Some(len) = self.rows.borrow_mut().remove(&va) else {
            return Ok(());
        };
        let r = self.bv.unmap_run(va, Some(len), defer);
        if r.is_err() {
            self.rows.borrow_mut().insert(va, len);
            restore_owned_rows(&mut self.rows.borrow_mut(), &self.bv, va, va + len);
        }
        r
    }
    fn unmap_range(&self, va: u64, len: u64, defer: bool) -> Result<(), String> {
        let end = va + len;
        let saved = self.rows.borrow().clone();
        cut_sim_rows(&mut self.rows.borrow_mut(), va, end);
        let res = self.bv.unmap_range(va, len, defer);
        if res.is_err() {
            *self.rows.borrow_mut() = saved;
            restore_owned_rows(&mut self.rows.borrow_mut(), &self.bv, va, end);
        }
        res
    }
    fn invalidate(&self) -> Result<(), String> {
        Ok(())
    }
    fn own_view(&self, va: u64, end: u64) -> Option<crate::ledger::OwnView> {
        Some(self.bv.own_view(va, end))
    }
}

// ─── the guest page table and the real walker spec ─────────────────────────────────────────────

/// One guest PTE: `(aperture: true = sys-coherent RAM / false = vidmem, backing page, read-only,
/// page-size code)`.
type Pte = Option<(bool, u64, bool, u8)>;

fn flags_of(ram: bool, ro: bool, ps: u8) -> u32 {
    u32::from(if ram { AP_SYS_COHERENT } else { AP_VIDMEM })
        | if ro { KFWR_RF_READ_ONLY } else { 0 }
        | (u32::from(ps) << KFWR_RF_PS_SHIFT)
}

/// The walk of a 4 KiB-grain guest table: VA+GPA-contiguous leaves of one key coalesced into runs
/// (what `kf_walk.cu` reports), per class.
fn walk(table: &[Pte]) -> Vec<KfMapRun> {
    let mut out: Vec<KfMapRun> = Vec::new();
    for (i, e) in table.iter().enumerate() {
        let Some((ram, back, ro, ps)) = *e else {
            continue;
        };
        let va = pg(i as u64);
        let flags = flags_of(ram, ro, ps);
        if let Some(l) = out.last_mut()
            && l.va + l.len == va
            && l.flags == flags
            && l.gpga + l.len == back * P
        {
            l.len += P;
            continue;
        }
        out.push(KfMapRun {
            va,
            gpga: back * P,
            len: P,
            flags,
            op: KFWR_OP_MAP,
            pdb_index: 0,
        });
    }
    // Per class, sorted: the walk spec takes one class's runs at a time.
    out.sort_by_key(|r| (diffmodel::class_of(r.flags), r.va));
    out
}

fn ack(c: u8) -> AckCode {
    match c {
        KFWR_ACK_APPLIED => AckCode::Applied,
        KFWR_ACK_HELD => AckCode::Held,
        KFWR_ACK_FAILED => AckCode::Failed,
        other => panic!("unknown ack {other}"),
    }
}

/// One refresh: walk → diff (spec) → the REAL apply over the mirror → commit (spec).
fn refresh(
    m: &dyn MapTarget,
    com: &diffmodel::Committed,
    table: &[Pte],
    ram_limit: u64,
) -> (diffmodel::Committed, crate::apply::Applied) {
    let d = diffmodel::diff(com, &walk(table), usize::MAX / 2);
    assert!(!d.overflow && !d.partial);
    let runs: Vec<DiffRun> = d
        .runs
        .iter()
        .map(|r| PermPolicy::default().diff_run(r))
        .collect();
    let ram = move |gpa: u64, len: u64| (gpa + len <= ram_limit).then_some(gpa);
    let cfg = cfg(&ram);
    let out = apply_entry(m, &runs, &cfg);
    let codes: Vec<AckCode> = out.codes.iter().map(|&c| ack(c)).collect();
    (diffmodel::commit(com, &d.runs, &codes), out)
}

/// Committed placements of one class that overlap each other (the slot must be disjoint).
fn slot_overlaps(com: &diffmodel::Committed) -> Vec<String> {
    let mut v = Vec::new();
    for (c, cls) in com.cls.iter().enumerate() {
        for w in cls.windows(2) {
            if w[0].va + w[0].len > w[1].va {
                v.push(format!(
                    "class {c}: {:#x}+{:#x} overlaps {:#x}+{:#x}",
                    w[0].va, w[0].len, w[1].va, w[1].len
                ));
            }
        }
    }
    v
}

/// The host mirror's translation of every page vs the walker's committed (non-held) set.
fn host_vs_walker(sim: &Sim, com: &diffmodel::Committed) -> Result<(), String> {
    let rm = sim.0.borrow();
    if let Some(v) = rm.violations.first() {
        return Err(format!("VIOLATION {v}"));
    }
    if rm.asserts != 0 {
        return Err(format!("{} gpu_vaspace.c:1639 asserts", rm.asserts));
    }
    let broken = rm.broken_mirror();
    if !broken.is_empty() {
        return Err(format!("FAULT_PTE: {broken:x?}"));
    }
    for i in 0..PAGES {
        let p = pg(i);
        let want: Vec<Backing> = com
            .flat()
            .iter()
            .filter(|r| r.flags & kf_cuda::abi::KFWR_RF_HELD == 0 && r.va <= p && p < r.va + r.len)
            .map(|r| {
                let ram = r.aperture() == AP_SYS_COHERENT;
                (ram, r.gpga + (p - r.va))
            })
            .collect();
        let got = rm.translate(p, Owner::Mirror);
        let ok = match (want.as_slice(), got) {
            ([], None) => true,
            ([w], Some(g)) => *w == g,
            _ => false,
        };
        if !ok {
            return Err(format!(
                "page {i}: walker committed {want:x?}, host translates {got:x?}"
            ));
        }
    }
    Ok(())
}

/// Arm the sim's guard with every page whose TRANSLATION (aperture, backing, read-only) is the
/// same in `old` and `new` and that the host maps that way right now. The leaf size is NOT part of
/// the key: the owner rule is about the guest's mapping, i.e. what the VA translates to.
fn arm_guard(sim: &Sim, old: &[Pte], new: &[Pte]) {
    let mut rm = sim.0.borrow_mut();
    let guard: Vec<(u64, Backing)> = (0..PAGES as usize)
        .filter_map(|i| {
            let (o, n) = (old[i]?, new[i]?);
            let same = o.0 == n.0 && o.1 == n.1 && o.2 == n.2;
            let want = (o.0, o.1 * P);
            (same && rm.translate(pg(i as u64), Owner::Mirror) == Some(want))
                .then_some((pg(i as u64), want))
        })
        .collect();
    rm.guard = guard;
    rm.transient.clear();
}

fn row(table: &mut [Pte], first: u64, n: u64, back: u64, ram: bool) {
    for k in 0..n {
        table[(first + k) as usize] = Some((ram, back + k, false, kf_cuda::abi::PS_4K));
    }
}

// ─── 1. one failed sub-range unmap, then clean refreshes ───────────────────────────────────────

/// ★ FINDING 1 (HIGH). A one-page change inside a coalesced 16-page row whose CHANGED-sub-range
/// unmap the host refuses (ANY refusal: an injected RM error here; in production also
/// `SPLIT_OUTSIDE_RESERVATION` for a > 512 MiB 4 KiB row in the NV01 range). `apply_entry` acks
/// the UNMAP run FAILED (the walker keeps the old 16-page placement) but acks the two fully-kept
/// MAP runs APPLIED with no check against `failed_unmaps` (apply.rs `new.is_empty()` →
/// `kept_runs`), so the walker's slot now holds OVERLAPPING placements. On the next clean refresh
/// the old placement is unmapped WHOLE (its pages are no longer "kept" by any MAP), which takes the
/// host mappings of the 15 unchanged pages that the walker still believes are committed.
// FAILED at 6fafcc6e; passes since the review fixes (2026-10-10) — in the default suite.
#[test]
fn a_refused_subrange_unmap_desyncs_walker_and_host() {
    let mut fails = Vec::new();
    for (at, label) in [(4u64, "NV01"), (40, "reservation")] {
        for mode in [Inject::Before, Inject::AfterRange] {
            let sim = Sim::new(space(RESV_LO, RESV_HI));
            let mut f = Faulty::new(&sim);
            f.mode = mode;
            let f = f;
            let m = FMirror::new(&f, true, false);
            let mut table: Vec<Pte> = vec![None; PAGES as usize];
            row(&mut table, at, 16, 500, true);
            let (mut com, _) = refresh(&m, &diffmodel::Committed::default(), &table, 1 << 30);
            host_vs_walker(&sim, &com).unwrap();
            // The guest re-points page at+5; the next host call (the changed sub-range's unmap)
            // fails.
            table[(at + 5) as usize] = Some((true, 9000, false, 0));
            f.fail_at.set(Some(f.calls.get()));
            (com, _) = refresh(&m, &com, &table, 1 << 30);
            assert!(f.fired.get());
            let overl = slot_overlaps(&com);
            // Two clean refreshes: the walker retries; everything must converge.
            for _ in 0..2 {
                (com, _) = refresh(&m, &com, &table, 1 << 30);
            }
            if let Err(e) = host_vs_walker(&sim, &com) {
                fails.push(format!(
                    "{label} {mode:?}: slot overlaps after the failure {overl:?}; after 2 clean refreshes: {e}"
                ));
            } else if !overl.is_empty() {
                fails.push(format!("{label} {mode:?}: walker slot overlaps {overl:?}"));
            }
            // And the guest's 15 unchanged pages must translate to their backing.
            let rm = sim.0.borrow();
            for k in (0..16).filter(|&k| k != 5) {
                if rm.translate(pg(at + k), Owner::Mirror) != Some((true, (500 + k) * P)) {
                    fails.push(format!(
                        "{label} {mode:?}: UNCHANGED page {} lost its host mapping for good",
                        at + k
                    ));
                    break;
                }
            }
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

// ─── 2. a MAP run = kept part + refused new part ───────────────────────────────────────────────

/// ★ FINDING 2 (MEDIUM). The guest changes page 0 of a 16-page row AND extends the row by four
/// linear pages that lie beyond guest RAM (a refused leaf). The walker coalesces pages 1..20 into
/// ONE MAP run: kept part 1..16 (unchanged) + new part 16..20 (refused). The run is FAILED, and the
/// apply's "take its kept part down too" unmaps the 15 UNCHANGED pages — they are transiently AND
/// persistently unmapped while the guest has the bad extension (hardware keeps them valid; a bad
/// leaf must not damage its neighbours, owner rule (1)/(3), OWNER_RULINGS §AA).
// FAILED at 6fafcc6e; passes since the review fixes (2026-10-10) — in the default suite.
#[test]
fn a_refused_extension_takes_down_unchanged_neighbours() {
    let mut fails = Vec::new();
    for (at, label) in [(4u64, "NV01"), (40, "reservation")] {
        let sim = Sim::new(space(RESV_LO, RESV_HI));
        let f = Faulty::new(&sim);
        let m = FMirror::new(&f, true, false);
        // Guest RAM ends at backing page 516: [500, 516) is RAM, 516.. is not.
        let limit = 516 * P;
        let mut table: Vec<Pte> = vec![None; PAGES as usize];
        row(&mut table, at, 16, 500, true);
        let (com, _) = refresh(&m, &diffmodel::Committed::default(), &table, limit);
        host_vs_walker(&sim, &com).unwrap();
        let old = table.clone();
        table[at as usize] = Some((true, 9, false, 0));
        row(&mut table, at + 16, 4, 516, true); // linear extension past RAM end
        row(&mut table, at + 1, 15, 501, true); // unchanged (rewritten identically)
        arm_guard(&sim, &old, &table);
        let (com, out) = refresh(&m, &com, &table, limit);
        let transient = core::mem::take(&mut sim.0.borrow_mut().transient);
        sim.0.borrow_mut().guard.clear();
        if !transient.is_empty() {
            fails.push(format!(
                "{label}: {} transient(s) of UNCHANGED pages; first: {} (refusal: {:?})",
                transient.len(),
                transient[0],
                out.first_refusal
            ));
        }
        let rm = sim.0.borrow();
        let lost = (1..16)
            .filter(|&k| rm.translate(pg(at + k), Owner::Mirror) != Some((true, (500 + k) * P)))
            .count();
        if lost != 0 {
            fails.push(format!(
                "{label}: {lost} of 15 unchanged pages left unmapped after the refresh (walker slot {:?})",
                com.flat().iter().map(|r| (r.va, r.len)).collect::<Vec<_>>()
            ));
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

// ─── 3. leaf-size change with identical translation ────────────────────────────────────────────

/// ★ FINDING 3 (MEDIUM, needs an owner ruling). The guest re-expresses sixteen 4 KiB leaves as one
/// 64 KiB leaf over the SAME backing (or back): every VA translates identically before and after.
/// `same_mapping` keys on the leaf size, so all 16 VAs are unmapped then re-mapped — inside a
/// guest RESERVATION too, where the host mapping unit is the row, not the leaf, and nothing forces
/// a re-make. The in-tree guard cannot see it: `sim::backing_of` puts the leaf into the key.
// FAILED at 6fafcc6e; passes since the review fixes (2026-10-10) — in the default suite.
#[test]
fn a_leaf_size_change_with_identical_translation_transiently_unmaps() {
    let mut fails = Vec::new();
    // 64 KiB-aligned 16-page window inside the static reservation, and one in the NV01 range.
    for (first, label) in [(48u64, "reservation"), (16, "NV01")] {
        assert_eq!(pg(first) % 0x1_0000, 0);
        let sim = Sim::new(space(RESV_LO, RESV_HI));
        let f = Faulty::new(&sim);
        let m = FMirror::new(&f, true, false);
        let mut table: Vec<Pte> = vec![None; PAGES as usize];
        row(&mut table, first, 16, 0x100, true); // backing 64 KiB-aligned
        let (com, _) = refresh(&m, &diffmodel::Committed::default(), &table, 1 << 30);
        let old = table.clone();
        for k in 0..16 {
            table[(first + k) as usize] = Some((true, 0x100 + k, false, kf_cuda::abi::PS_64K));
        }
        arm_guard(&sim, &old, &table);
        let (com, _) = refresh(&m, &com, &table, 1 << 30);
        let t = core::mem::take(&mut sim.0.borrow_mut().transient);
        sim.0.borrow_mut().guard.clear();
        if !t.is_empty() {
            fails.push(format!(
                "{label}: {} transient(s); first: {}",
                t.len(),
                t[0]
            ));
        }
        host_vs_walker(&sim, &com).unwrap();
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

// ─── 4. fuzz: random guest tables, an injected failure at EVERY call index ─────────────────────

fn random_table(rng: &mut Rng, prev: &[Pte]) -> Vec<Pte> {
    let mut t = prev.to_vec();
    for _ in 0..1 + rng.below(4) {
        let first = rng.below(PAGES);
        let n = (1 + rng.below(20)).min(PAGES - first);
        match rng.below(10) {
            // map / re-point a linear row
            0..=3 => {
                let back = 1 + rng.below(3000);
                let ram = rng.below(10) < 8;
                let ro = rng.below(10) == 0;
                for k in 0..n {
                    t[(first + k) as usize] = Some((ram, back + k, ro, 0));
                }
            }
            // scattered pages
            4..=5 => {
                for k in 0..n {
                    t[(first + k) as usize] = Some((true, 1 + rng.below(3000), false, 0));
                }
            }
            // re-point ONE page inside
            6 => {
                t[first as usize] = Some((true, 5000 + rng.below(100), false, 0));
            }
            // extend: linear continuation of the page before `first`
            7 => {
                if first > 0
                    && let Some((ram, b, ro, ps)) = t[(first - 1) as usize]
                {
                    for k in 0..n {
                        t[(first + k) as usize] = Some((ram, b + 1 + k, ro, ps));
                    }
                }
            }
            // unmap
            _ => {
                for k in 0..n {
                    t[(first + k) as usize] = None;
                }
            }
        }
    }
    t
}

/// Run `steps` refreshes; at refresh `inj_step` fail host call number `inj_call` (counted from the
/// start of that refresh). Returns the first broken invariant, and the number of host calls the
/// injected refresh made.
fn fuzz_one(
    seed: u64,
    steps: usize,
    low: bool,
    inj: Option<(usize, u64, Inject)>,
) -> Result<u64, String> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let sim = Sim::new(space(RESV_LO, RESV_HI));
    let mut f = Faulty::new(&sim);
    if let Some((_, _, mode)) = inj {
        f.mode = mode;
    }
    let f = f;
    let m = FMirror::new(&f, true, low);
    // Two foreign mappings in the NV01 range (other kayfabe code, same client + hDma).
    assert!(sim.0.borrow_mut().place_foreign(1, pg(2), 2 * P));
    assert!(sim.0.borrow_mut().place_foreign(2, pg(70), P));
    let foreign_pages = [2u64, 3, 70];
    let mut table: Vec<Pte> = vec![None; PAGES as usize];
    let mut com = diffmodel::Committed::default();
    let mut calls_in_inj = 0;
    let mut injected_at: Option<usize> = None;
    let mut had_overlap = false;
    let r = (|| -> Result<(), String> {
        for step in 0..steps {
            let mut new = random_table(&mut rng, &table);
            for &p in &foreign_pages {
                new[p as usize] = None; // the guest never maps over the foreign windows here
            }
            let injecting = inj.is_some_and(|(s, ..)| s == step);
            let before = f.calls.get();
            if let Some((_, k, _)) = inj.filter(|_| injecting) {
                f.fail_at.set(Some(before + k));
            } else {
                f.fail_at.set(None);
                // Clean refresh: arm the translation guard (only once the system has had two clean
                // refreshes after an injection to converge).
                if injected_at.is_none_or(|s| step >= s + 3) {
                    arm_guard(&sim, &table, &new);
                }
            }
            let (c2, out) = refresh(&m, &com, &new, 1 << 30);
            if std::env::var_os("ADV_DEBUG").is_some() {
                let d = diffmodel::diff(&com, &walk(&new), usize::MAX / 2);
                eprintln!(
                    "step {step} inj={injecting} runs={:x?} codes={:?} refusal={:?} fallback={:?}",
                    d.runs
                        .iter()
                        .map(|r| (r.op, (r.va - BASE) / P, r.len / P, r.gpga / P, r.flags))
                        .collect::<Vec<_>>(),
                    out.codes,
                    out.first_refusal,
                    out.first_batch_fallback
                );
            }
            com = c2;
            if std::env::var_os("ADV_DEBUG").is_some() {
                let own = m.bv.own.lock().unwrap().within(pg(0x40), pg(0x50));
                eprintln!(
                    "  own[40..50]={:x?}",
                    own.iter()
                        .map(|(v, o)| ((v - BASE) / P, o.len / P, o.batch, o.via))
                        .collect::<Vec<_>>()
                );
                let rm = sim.0.borrow();
                let mut hm: Vec<_> = rm
                    .maps
                    .iter()
                    .filter(|x| x.va < pg(0x50) && x.va + x.len > pg(0x40))
                    .map(|x| ((x.va - BASE) / P, x.len / P, x.hdma, x.obj, x.broken))
                    .collect();
                hm.sort();
                eprintln!(
                    "  host[40..50]={hm:x?} resv={:x?} micro={:x?}",
                    rm.resv,
                    m.bv.micro.lock().unwrap()
                );
            }
            if injecting {
                calls_in_inj = f.calls.get() - before;
                injected_at = Some(step);
            }
            let t = core::mem::take(&mut sim.0.borrow_mut().transient);
            sim.0.borrow_mut().guard.clear();
            if !t.is_empty() {
                return Err(format!("step {step}: TRANSIENT {}", t[0]));
            }
            table = new;
            let ov = slot_overlaps(&com);
            if injecting && !ov.is_empty() {
                had_overlap = true;
            }
            if !ov.is_empty() && injected_at.is_none_or(|s| step >= s + 2) {
                return Err(format!("step {step}: walker slot overlaps {ov:?}"));
            }
            // Consistency is required once the walker had two clean refreshes to retry.
            if injected_at.is_none_or(|s| step >= s + 2) {
                host_vs_walker(&sim, &com).map_err(|e| format!("step {step}: {e}"))?;
                // Convergence: every page the guest maps translates to its backing (no refusals in
                // this model: all RAM < 1 GiB, store 1 TiB, no VMM placements).
                let rm = sim.0.borrow();
                for i in 0..PAGES {
                    let want = table[i as usize].map(|(ram, b, ..)| (ram, b * P));
                    if want.is_some() && rm.translate(pg(i), Owner::Mirror) != want {
                        return Err(format!(
                            "step {step}: page {i}: guest maps {want:x?}, host has {:x?} (not converged 2 refreshes after the injection)",
                            rm.translate(pg(i), Owner::Mirror)
                        ));
                    }
                }
            }
            for (id, va, len) in [(1u32, pg(2), 2 * P), (2, pg(70), P)] {
                if !sim.0.borrow().foreign_intact(id, va, len) {
                    return Err(format!("step {step}: foreign #{id} damaged"));
                }
            }
        }
        Ok(())
    })();
    r.map(|()| calls_in_inj).map_err(|e| {
        format!(
            "{e} [{}]",
            if had_overlap {
                "slot-overlap-at-injection"
            } else {
                "NO-overlap-at-injection"
            }
        )
    })
}

/// Baseline: the fuzz with NO injection must be clean (else the harness, not the code, is wrong).
#[test]
fn adversarial_fuzz_without_injection_is_clean() {
    for low in [false, true] {
        for seed in 1..=150u64 {
            fuzz_one(seed, 40, low, None).unwrap_or_else(|e| panic!("low={low} seed {seed}: {e}"));
        }
    }
}

/// ★ Error injection at EVERY host-call index of one refresh, both failure modes, micro on/off.
/// At 6fafcc6e: 362 of 5016 injected runs break an invariant — 348 via finding 1 (walker slot
/// overlap), 14 (micro on only) via finding 4 (`BatchedVas::unmap_run` answers "nothing of ours"
/// from the run's START only: after a range that cut the start but failed on the tail through a
/// micro reservation, the per-run fallback acks APPLIED and the tail mapping outlives the guest's
/// unmap) and finding 5 (a refused micro-reservation free is forgotten from `micro` while its VA
/// block stays: every later map there answers HeldByHost → committed HELD → never mapped).
/// Replay one: `ADV_DEBUG=1 ADV_LOW=1 ADV_SEED=36 ADV_STEP=7 ADV_CALL=2 cargo test -p kf-mem
/// adversarial_debug_replay -- --ignored --nocapture` (finding 4); `ADV_SEED=7 ADV_CALL=4` (5).
/// After two clean refreshes the walker's committed set and the host must agree, every guest page
/// must translate, no foreign mapping may be touched, no PTE lost, no `:1639` assert.
// FAILED at 6fafcc6e; passes since the review fixes (2026-10-10) — in the default suite.
#[test]
fn adversarial_error_injection_at_every_call_index() {
    let mut failures: Vec<String> = Vec::new();
    let mut runs = 0usize;
    for low in [false, true] {
        for mode in [Inject::Before, Inject::AfterRange] {
            for seed in 1..=40u64 {
                for inj_step in [3usize, 7, 12] {
                    let mut k = 0u64;
                    loop {
                        runs += 1;
                        match fuzz_one(seed, inj_step + 6, low, Some((inj_step, k, mode))) {
                            Ok(calls) => {
                                if k + 1 >= calls {
                                    break;
                                }
                            }
                            Err(e) => {
                                failures.push(format!(
                                    "low={low} {mode:?} seed {seed} inj step {inj_step} call {k}: {e}"
                                ));
                                if failures.len() > 4000 {
                                    break;
                                }
                            }
                        }
                        k += 1;
                        if k > 400 {
                            break;
                        }
                    }
                }
            }
        }
    }
    for f in failures
        .iter()
        .filter(|f| f.contains("NO-overlap"))
        .take(12)
    {
        eprintln!("NO-OVERLAP FAILURE: {f}");
    }
    let kinds = [
        "NO-overlap-at-injection",
        "slot-overlap-at-injection",
        "overlaps",
        "TRANSIENT",
        "walker committed",
        "not converged",
        "foreign",
        "FAULT_PTE",
        "asserts",
        "VIOLATION",
    ];
    let hist: Vec<(&str, usize)> = kinds
        .iter()
        .map(|k| (*k, failures.iter().filter(|f| f.contains(k)).count()))
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {runs} injected runs broke an invariant {hist:?}\nfirst 6:\n{}",
        failures.len(),
        failures
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
#[ignore = "debug replay"]
fn adversarial_debug_replay() {
    let seed: u64 = std::env::var("ADV_SEED").unwrap().parse().unwrap();
    let step: usize = std::env::var("ADV_STEP").unwrap().parse().unwrap();
    let call: u64 = std::env::var("ADV_CALL").unwrap().parse().unwrap();
    let low = std::env::var_os("ADV_LOW").is_some();
    let mode = if std::env::var_os("ADV_AFTER").is_some() {
        Inject::AfterRange
    } else {
        Inject::Before
    };
    eprintln!(
        "{:?}",
        fuzz_one(seed, step + 6, low, Some((step, call, mode)))
    );
}

// ─── 5. finding 1 WITHOUT any host error: a > MAX_LEAF_PIECES row in the NV01 range ────────────

/// One refresh of the huge-row tests: the walker spec over `m`, every page RAM at its own GPA.
fn huge_step(
    m: &FMirror<'_>,
    com: &diffmodel::Committed,
    walk: &[KfMapRun],
) -> (diffmodel::Committed, crate::apply::Applied) {
    let ram = |gpa: u64, _len: u64| Some(gpa);
    let cfgv = cfg(&ram);
    let d = diffmodel::diff(com, walk, usize::MAX / 2);
    let runs: Vec<DiffRun> = d
        .runs
        .iter()
        .map(|r| PermPolicy::default().diff_run(r))
        .collect();
    let out = apply_entry(m, &runs, &cfgv);
    let codes: Vec<AckCode> = out.codes.iter().map(|&c| ack(c)).collect();
    (diffmodel::commit(com, &d.runs, &codes), out)
}

fn ram_run(va: u64, len: u64, back: u64) -> KfMapRun {
    KfMapRun {
        va,
        gpga: back,
        len,
        flags: flags_of(true, false, kf_cuda::abi::PS_4K),
        op: KFWR_OP_MAP,
        pdb_index: 0,
    }
}

/// ★ FINDING 1, guest-triggerable with no host error at all — FIXED 2026-10-10. At 6fafcc6e a
/// 4 KiB-leaf row longer than the old `MAX_LEAF_PIECES` (512 MiB + 4 KiB) in the unreserved range
/// stayed ONE host mapping; the guest then re-pointed ONE page of it, the changed sub-range's range
/// unmap was refused before any host call (`SPLIT_OUTSIDE_RESERVATION`), the UNMAP run was acked
/// FAILED while the two fully-kept MAP runs were acked APPLIED → overlapping placements in the
/// walker's slot, and the next refresh unmapped the old placement WHOLE with every unchanged page.
/// Now (same row, same change, no host error): the row is placed one mapping per leaf (the bound
/// is 2^20 leaves), the change is exact, the slot never overlaps, no unchanged page is lost. Rows
/// beyond the bound: [`a_row_beyond_the_leaf_bound_is_refused_or_micro_reserved`].
#[test]
fn a_huge_nv01_row_with_one_changed_page_desyncs_without_any_host_error() {
    let base = 0x4000_0000u64;
    let pages = (1u64 << 17) + 1; // the old bound + 1
    let sim = Sim::new(space(0x10_0000_0000, 0x10_1000_0000));
    let f = Faulty::new(&sim);
    let m = FMirror::new(&f, true, false);
    let back = 0x1_0000_0000u64;
    let (com, out) = huge_step(
        &m,
        &diffmodel::Committed::default(),
        &[ram_run(base, pages * P, back)],
    );
    assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
    assert_eq!(
        sim.0.borrow().maps.len() as u64,
        pages,
        "one host mapping per leaf"
    );
    // Page 5 re-pointed.
    let w1 = [
        ram_run(base, 5 * P, back),
        ram_run(base + 5 * P, P, 0x7000_0000),
        ram_run(base + 6 * P, (pages - 6) * P, back + 6 * P),
    ];
    sim.0.borrow_mut().guard = [0u64, 4, 6, pages - 1]
        .iter()
        .map(|&k| (base + k * P, (true, back + k * P)))
        .collect();
    let (com, out) = huge_step(&m, &com, &w1);
    let transient = core::mem::take(&mut sim.0.borrow_mut().transient);
    sim.0.borrow_mut().guard.clear();
    assert!(transient.is_empty(), "{transient:?}");
    assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
    assert_eq!(
        (out.unmap_calls, out.mapped),
        (1, 1),
        "exactly the changed page"
    );
    let overlaps = slot_overlaps(&com);
    // One more refresh with the same guest table: nothing to do.
    let (com, out) = huge_step(&m, &com, &w1);
    assert_eq!(out.codes.len(), 0, "converged: the walker emits nothing");
    let rm = sim.0.borrow();
    let lost = [0u64, 4, 6, pages - 1]
        .iter()
        .filter(|&&k| rm.translate(base + k * P, Owner::Mirror) != Some((true, back + k * P)))
        .count();
    assert!(
        overlaps.is_empty() && lost == 0,
        "walker slot overlaps {overlaps:?}; {lost} of 4 sampled UNCHANGED pages lost their host mapping (unsafe_splits={}); slot now {:x?}",
        m.bv.unsafe_splits
            .load(std::sync::atomic::Ordering::Relaxed),
        com.flat()
            .iter()
            .map(|r| ((r.va - base) / P, r.len / P))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        rm.translate(base + 5 * P, Owner::Mirror),
        Some((true, 0x7000_0000))
    );
}

/// ★ Review fix 2026-10-10 (finding 1): a row with more leaves than the bound (lowered to 64 here)
/// in the unreserved range is never ONE `NV01` mapping. Without micro reservations it is refused
/// by name (absence; counted) and the walker keeps retrying it; with them it is ONE mapping THROUGH
/// a micro reservation and a later one-page change is an exact range inside it.
#[test]
fn a_row_beyond_the_leaf_bound_is_refused_or_micro_reserved() {
    for low in [false, true] {
        let base = 0x4000_0000u64;
        let pages = 65u64;
        let sim = Sim::new(space(0x10_0000_0000, 0x10_1000_0000));
        let f = Faulty::new(&sim);
        let mut m = FMirror::new(&f, true, low);
        m.bv.max_leaf_pieces = 64;
        let back = 0x1_0000_0000u64;
        let (com, out) = huge_step(
            &m,
            &diffmodel::Committed::default(),
            &[ram_run(base, pages * P, back)],
        );
        if !low {
            assert_eq!((out.refused, out.mapped), (1, 0));
            assert!(
                out.first_refusal
                    .as_deref()
                    .is_some_and(|w| w.contains(crate::batch::HUGE_ROW_OUTSIDE_RESERVATION)),
                "{:?}",
                out.first_refusal
            );
            assert!(out.refusals_are_absence(), "a refused map is absence (§AA)");
            assert_eq!(
                m.bv.huge_refused.load(std::sync::atomic::Ordering::Relaxed),
                1
            );
            assert!(com.is_empty() && sim.0.borrow().maps.is_empty());
            continue;
        }
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
        {
            let rm = sim.0.borrow();
            assert_eq!(
                (rm.maps.len(), rm.resv.len()),
                (1, 1),
                "one mapping through one micro reservation"
            );
        }
        let w1 = [
            ram_run(base, 5 * P, back),
            ram_run(base + 5 * P, P, 0x7000_0000),
            ram_run(base + 6 * P, (pages - 6) * P, back + 6 * P),
        ];
        sim.0.borrow_mut().guard = (0..pages)
            .filter(|&k| k != 5)
            .map(|k| (base + k * P, (true, back + k * P)))
            .collect();
        let (com, out) = huge_step(&m, &com, &w1);
        let transient = core::mem::take(&mut sim.0.borrow_mut().transient);
        sim.0.borrow_mut().guard.clear();
        assert!(transient.is_empty(), "{transient:?}");
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
        assert!(slot_overlaps(&com).is_empty());
        let rm = sim.0.borrow();
        assert_eq!(
            rm.translate(base + 5 * P, Owner::Mirror),
            Some((true, 0x7000_0000))
        );
        assert!(rm.broken_mirror().is_empty() && rm.asserts == 0);
    }
}

// ─── 6. targeted: findings 4, 5, 6 (review fixes 2026-10-10) ───────────────────────────────────

/// Eight scattered guest-RAM pages at 4..12 (one batch: a micro reservation when `low`), and, when
/// `vid_tail`, a vidmem page at 12 (never batched: one `NV01` leaf mapping).
fn eight_scattered(m: &FMirror<'_>, vid_tail: bool) -> (Vec<Pte>, diffmodel::Committed) {
    let mut table: Vec<Pte> = vec![None; PAGES as usize];
    for k in 0..8u64 {
        table[(4 + k) as usize] = Some((true, 700 + 13 * k, false, 0));
    }
    if vid_tail {
        table[12] = Some((false, 40, false, 0));
    }
    let (com, out) = refresh(m, &diffmodel::Committed::default(), &table, 1 << 30);
    assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
    (table, com)
}

/// ★ FINDING 4 (unit). `BatchedVas::unmap_run` with a known length must look at the WHOLE run.
/// At 6fafcc6e it decided "nothing of ours here" from the run's START: after a range that cut the
/// start (and failed on the tail) it answered `Ok` with no host call and the tail kept mapping the
/// old guest page.
#[test]
fn unmap_run_with_a_known_length_takes_its_tail_when_its_start_is_already_gone() {
    let sim = Sim::new(space(RESV_LO, RESV_HI));
    let f = Faulty::new(&sim);
    let m = FMirror::new(&f, true, false);
    let mut table: Vec<Pte> = vec![None; PAGES as usize];
    row(&mut table, 4, 2, 500, true); // one run, two NV01 leaf mappings
    let _ = refresh(&m, &diffmodel::Committed::default(), &table, 1 << 30);
    assert_eq!(sim.0.borrow().maps.len(), 2);
    m.bv.unmap_range(pg(4), P, true).unwrap(); // the start goes first
    m.bv.unmap_run(pg(4), Some(2 * P), true).unwrap();
    assert_eq!(
        sim.0.borrow().translate(pg(5), Owner::Mirror),
        None,
        "the tail of the run must not outlive its unmap"
    );
    assert!(m.bv.own.lock().unwrap().is_empty());
}

/// ★ FINDINGS 4 + 5 (glue). A range over two `hDma`s (a micro-reserved batch, then a vidmem leaf in
/// the `NV01` range) whose SECOND host call fails: the batch's span is gone on the host and in the
/// ledger, so (4) the rows must not claim it again — at 6fafcc6e `GpuMirror::unmap_range` put every
/// row back — and (5) its now-empty micro reservation must still be released — at 6fafcc6e the
/// error returned before `release_micro`.
#[test]
fn a_range_failing_on_its_second_span_leaves_rows_equal_to_the_ledger_and_releases_the_first() {
    let sim = Sim::new(space(RESV_LO, RESV_HI));
    let f = Faulty::new(&sim);
    let m = FMirror::new(&f, true, true);
    let _ = eight_scattered(&m, true);
    assert_eq!(
        sim.0.borrow().resv.len(),
        1,
        "the batch went through a micro reservation"
    );
    // Calls: 0 unmap_in (the batch's span), 1 free (its emptied object), 2 unmap_range (vidmem).
    f.fail_at.set(Some(f.calls.get() + 2));
    let r = m.unmap_range(pg(4), 9 * P, true);
    assert!(r.is_err() && f.fired.get());
    let rows: Vec<u64> = m.rows.borrow().keys().map(|&v| (v - BASE) / P).collect();
    assert_eq!(rows, vec![12], "rows == what the ledger still holds");
    let rm = sim.0.borrow();
    assert!((4..12).all(|p| rm.translate(pg(p), Owner::Mirror).is_none()));
    assert_eq!(rm.translate(pg(12), Owner::Mirror), Some((false, 40 * P)));
    assert!(
        rm.resv.is_empty(),
        "the emptied micro reservation was released despite the error"
    );
}

/// ★ FINDING 5. A micro reservation whose FREE host RM refuses stays tracked: a later map there
/// goes THROUGH it (at 6fafcc6e it was forgotten, so the map found its VA held → HELD forever),
/// the retire reports it, and the next release over it frees it.
#[test]
fn a_refused_micro_free_stays_tracked_and_is_retried() {
    let sim = Sim::new(space(RESV_LO, RESV_HI));
    let f = Faulty::new(&sim);
    let m = FMirror::new(&f, true, true);
    let _ = eight_scattered(&m, false);
    // Calls: 0 unmap_in, 1 free (batch object), 2 free (micro reservation) — refused.
    f.fail_at.set(Some(f.calls.get() + 2));
    m.unmap_range(pg(4), 8 * P, true).unwrap();
    assert!(f.fired.get());
    assert_eq!(sim.0.borrow().resv.len(), 1, "host RM still holds it");
    assert_eq!(m.bv.micro.lock().unwrap().len(), 1, "and so do we");
    assert!(
        m.bv.leftovers() > 0,
        "a retire would not recycle this space"
    );
    let d = Desired {
        va: pg(6),
        len: P,
        off: 900 * P,
        ram: true,
        kind: 0,
        perm: MapPerm::READ_WRITE,
        leaf: 0,
    };
    assert_eq!(
        m.map(&d, true),
        Ok(Mapped::Placed),
        "never HELD by our own reservation"
    );
    assert_eq!(
        sim.0.borrow().translate(pg(6), Owner::Mirror),
        Some((true, 900 * P))
    );
    m.unmap_range(pg(6), P, true).unwrap();
    assert!(
        sim.0.borrow().resv.is_empty(),
        "the free retried and landed"
    );
    assert_eq!(m.bv.leftovers(), 0);
}

/// ★ FINDING 6. The falcon-context steer (`kf_qemu::chan`, `BatchedVas::hand_to_host`) unmaps
/// through the ledger: our mappings only (a foreign neighbour stays), each through its own `hDma`
/// — a micro reservation included, where the raw `HostRm::unmap_range` it replaces named the
/// space's own `hDma` and removed NOTHING (shown below) while the log said "steered" — and the
/// reservation is released once nothing of ours is left in it.
#[test]
fn the_falcon_steer_unmaps_through_the_ledger() {
    // (a) An NV01 row of two leaves, a foreign mapping right after it.
    let sim = Sim::new(space(RESV_LO, RESV_HI));
    let f = Faulty::new(&sim);
    let m = FMirror::new(&f, true, false);
    let mut table: Vec<Pte> = vec![None; PAGES as usize];
    row(&mut table, 4, 2, 500, true);
    let _ = refresh(&m, &diffmodel::Committed::default(), &table, 1 << 30);
    assert!(sim.0.borrow_mut().place_foreign(9, pg(6), P));
    assert_eq!(
        m.bv.hand_to_host(pg(4), 2 * P),
        crate::batch::HandOver::Free
    );
    {
        let rm = sim.0.borrow();
        assert!(
            rm.translate(pg(4), Owner::Mirror).is_none()
                && rm.translate(pg(5), Owner::Mirror).is_none()
        );
        assert!(rm.foreign_intact(9, pg(6), P) && rm.violations.is_empty() && rm.gap_bytes == 0);
    }
    assert!(m.bv.own.lock().unwrap().is_empty(), "no stale ledger entry");

    // (b) The guest's context page inside a micro-reserved batch.
    let sim = Sim::new(space(RESV_LO, RESV_HI));
    let f = Faulty::new(&sim);
    let m = FMirror::new(&f, true, true);
    let _ = eight_scattered(&m, false);
    let g = pg(6);
    let before = sim.0.borrow().translate(g, Owner::Mirror);
    // The raw call the steer used to make: the space's own routing (`NV01` hDma) — a no-op here.
    SpaceVerbs::unmap_range(&&sim, g, P, false).unwrap();
    assert_eq!(
        sim.0.borrow().translate(g, Owner::Mirror),
        before,
        "the raw range removed nothing"
    );
    sim.0.borrow_mut().guard = (4..12u64)
        .filter(|&p| p != 6)
        .map(|p| {
            (
                pg(p),
                sim.0.borrow().translate(pg(p), Owner::Mirror).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        m.bv.hand_to_host(g, P),
        crate::batch::HandOver::StillReserved
    );
    {
        let mut rm = sim.0.borrow_mut();
        assert!(rm.transient.is_empty(), "{:?}", rm.transient);
        rm.guard.clear();
        assert!(rm.translate(g, Owner::Mirror).is_none());
    }
    // Everything else handed over too: the reservation goes, the VA is free for host RM.
    assert_eq!(
        m.bv.hand_to_host(pg(4), 8 * P),
        crate::batch::HandOver::Free
    );
    assert!(sim.0.borrow().resv.is_empty() && m.bv.leftovers() == 0);
}
