//! The batched path against the host RM model — targeted cases, the property test, and the
//! workload numbers (`V3_BATCHED_MAP.md` §8).

use super::*;
use crate::batch::LOW_RANGE_MIN_RUNS;
use std::sync::atomic::Ordering::Relaxed;

/// The model space: 96 pages; pages 32..64 are the static guest reservation (`NV50`), the rest go
/// through the `NV01` range — the Windows shape (`[1 MiB, 4.5 GiB)` is unreserved) and the
/// Linux/CUDA shape (above 4.5 GiB, reserved) in one space, with two straddle edges.
const BASE: u64 = 0x100_0000;
const PAGES: u64 = 96;
const RESV_LO: u64 = BASE + 32 * P;
const RESV_HI: u64 = BASE + 64 * P;

fn pg(i: u64) -> u64 {
    BASE + i * P
}

fn fresh() -> Sim {
    Sim::new(space(RESV_LO, RESV_HI))
}

fn ok(sim: &Sim, c: &BTreeMap<u64, Committed>, f: &BTreeMap<u32, (u64, u64)>) {
    check(sim, c, f, BASE, pg(PAGES)).unwrap_or_else(|e| panic!("{e}"));
}

/// Map `rows` = `(page, pages, backing page)` of guest RAM through apply; all must be APPLIED.
fn map_ram(m: &SimMirror<'_>, c: &mut BTreeMap<u64, Committed>, rows: &[(u64, u64, u64)]) {
    let runs: Vec<DiffRun> = rows
        .iter()
        .map(|&(p, n, b)| map_run(pg(p), n * P, true, b * P, false))
        .collect();
    let out = apply_and_commit(m, &runs, c);
    assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
}

/// `n` one-page rows from page `p`, scattered backing.
fn scattered(p: u64, n: u64) -> Vec<(u64, u64, u64)> {
    (0..n)
        .map(|i| (p + i, 1, 7919 * (i + 1) % 4093 + 3))
        .collect()
}

fn unmap_pages(m: &SimMirror<'_>, c: &mut BTreeMap<u64, Committed>, pages: &[u64]) {
    let runs: Vec<DiffRun> = pages
        .iter()
        .map(|&p| unmap_run(pg(p), &c[&pg(p)]))
        .collect();
    let out = apply_and_commit(m, &runs, c);
    assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
}

/// ★★★ THE ROOT-CAUSE REGRESSION (Windows runs 242/243, host Xid 31 `FAULT_PTE` at
/// `0x4034000`, `gpu_vaspace.c:1639` at exit): scattered guest-RAM rows in the UNRESERVED part of
/// the space are placed and ONE middle row is unmapped. The others must stay mapped — at every
/// instant (the guard), not only at the end. Before the fix the rows went as one batch through the
/// `NV01` range and the single unmap SPLIT it: host RM freed the whole VA block.
#[test]
fn unmapping_one_row_outside_a_reservation_keeps_the_others_mapped() {
    for (low, n) in [(false, 4u64), (false, 12), (true, 4), (true, 12)] {
        let sim = fresh();
        let m = SimMirror::new(&sim, true, low);
        let mut c = BTreeMap::new();
        map_ram(&m, &mut c, &scattered(4, n));
        ok(&sim, &c, &BTreeMap::new());
        unmap_pages(&m, &mut c, &[5]);
        ok(&sim, &c, &BTreeMap::new());
        let rest: Vec<u64> = (4..4 + n).filter(|&p| p != 5).collect();
        unmap_pages(&m, &mut c, &rest);
        ok(&sim, &c, &BTreeMap::new());
        let rm = sim.0.borrow();
        assert!(rm.maps.is_empty() && rm.resv.is_empty() && rm.objs.is_empty());
        let batched = low && n as usize >= LOW_RANGE_MIN_RUNS;
        assert_eq!(
            m.bv.micro_made.load(Relaxed),
            u64::from(batched),
            "low={low} n={n}: a micro reservation iff batched in the low range"
        );
    }
}

/// The walker's diff for a remap of `pages` inside the committed placement at page `at`: UNMAP of
/// the WHOLE placement (`kf_walk.cu`: placements are unmapped whole), then the pieces — unchanged
/// ones with their old backing, the remapped pages with `new_back`.
fn remap_inside(
    c: &BTreeMap<u64, Committed>,
    at: u64,
    pages: &[u64],
    new_back: u64,
) -> Vec<DiffRun> {
    let old = c[&pg(at)];
    let n = old.len / P;
    let mut runs = vec![unmap_run(pg(at), &old)];
    let mut i = 0;
    while i < n {
        let changed = pages.contains(&(at + i));
        let mut j = i + 1;
        while j < n && pages.contains(&(at + j)) == changed {
            j += 1;
        }
        let back = if changed {
            new_back * P + i * P
        } else {
            old.off + i * P
        };
        runs.push(DiffRun {
            va: pg(at + i),
            len: (j - i) * P,
            at: back,
            unmap: false,
            held: false,
            // A change inside a big leaf is a split of it: the pieces are 4 KiB leaves.
            leaf: if old.run.leaf > P { P } else { old.run.leaf },
            ..old.run
        });
        i = j;
    }
    runs
}

/// ★★★ THE COALESCED-ROW HAZARD (owner, 2026-10-09), on EVERY path: a 16-page row that is VA- and
/// GPA-contiguous is ONE committed placement; the guest then remaps ONE page of it. The other 15
/// pages are UNCHANGED and must stay mapped at every instant — in the unreserved range (`NV01`)
/// and in a reservation alike, with batching on or off.
#[test]
fn a_one_page_change_inside_a_16_page_row_leaves_the_other_15_mapped() {
    let mut failures = Vec::new();
    for (at, label) in [(4u64, "NV01 range"), (40, "reservation")] {
        for (batching, low) in [(false, false), (true, false), (true, true)] {
            let sim = fresh();
            let m = SimMirror::new(&sim, batching, low);
            let mut c = BTreeMap::new();
            map_ram(&m, &mut c, &[(at, 16, 500)]);
            ok(&sim, &c, &BTreeMap::new());
            let out = apply_and_commit(&m, &remap_inside(&c, at, &[at + 5], 9000), &mut c);
            let r = if out.refused == 0 {
                check(&sim, &c, &BTreeMap::new(), BASE, pg(PAGES))
            } else {
                Err(format!("refused: {:?}", out.first_refusal))
            };
            if let Err(e) = r {
                failures.push(format!("{label} batching={batching} low={low}: {e}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of 6 failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Sparse changes: pages 2, 7 and 11 of a 16-page row are remapped, the other 13 are unchanged —
/// in the `NV01` range and in a reservation, every path.
#[test]
fn sparse_changes_inside_a_row_leave_the_rest_mapped() {
    for at in [4u64, 40] {
        for (batching, low) in [(false, false), (true, false), (true, true)] {
            let sim = fresh();
            let m = SimMirror::new(&sim, batching, low);
            let mut c = BTreeMap::new();
            map_ram(&m, &mut c, &[(at, 16, 700)]);
            let runs = remap_inside(&c, at, &[at + 2, at + 7, at + 11], 7000);
            let out = apply_and_commit(&m, &runs, &mut c);
            assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
            ok(&sim, &c, &BTreeMap::new());
            // Only the three changed pages reached the host: 3 unmaps, 3 maps.
            assert_eq!(out.mapped, 3, "at={at} batching={batching} low={low}");
        }
    }
}

/// A space whose `NV01` range has room for 2 MiB leaves: `[base, base + 8 MiB)` unreserved, the
/// static reservation far above.
fn big_space() -> (Sim, u64) {
    let base = 0x4000_0000u64;
    (Sim::new(space(base + (64 << 20), base + (128 << 20))), base)
}

fn leaf_run(va: u64, len: u64, back: u64, leaf: u64) -> DiffRun {
    DiffRun {
        leaf,
        ..map_run(va, len, true, back, false)
    }
}

/// ★ D1 (2026-10-10): 64 KiB and 2 MiB leaves outside a reservation are placed so that ANY later
/// partial change is exact. With micro reservations (the default): ONE reservation over the row and
/// ONE host mapping through it; without (off, or refused): 4 KiB grain. Changing one leaf unmaps
/// exactly that leaf in ONE range call and maps exactly the new leaf — the other leaves are never
/// touched, and never re-made.
/// ⊘ The test this replaces (2026-10-10, review fix) asserted "one mapping per leaf" in the `NV01`
/// range — the shape the apply had to RE-MAKE when the guest split a leaf.
#[test]
fn big_leaves_are_placed_exactly_and_change_alone() {
    for leaf in [0x1_0000u64, 0x20_0000] {
        for low in [true, false] {
            let (sim, base) = big_space();
            let m = SimMirror::new(&sim, true, low);
            let mut c = BTreeMap::new();
            let row = leaf_run(base, 4 * leaf, 0x1000_0000, leaf);
            assert_eq!(apply_and_commit(&m, &[row], &mut c).refused, 0);
            {
                let rm = sim.0.borrow();
                if low {
                    assert_eq!(
                        (rm.maps.len(), rm.resv.len()),
                        (1, 1),
                        "leaf {leaf:#x}: ONE reservation, ONE mapping through it"
                    );
                } else {
                    assert_eq!(
                        rm.maps.len() as u64,
                        4 * leaf / P,
                        "leaf {leaf:#x}: 4 KiB grain without a reservation"
                    );
                    assert!(rm.resv.is_empty());
                }
            }
            check(&sim, &c, &BTreeMap::new(), base, base + 4 * leaf).unwrap();
            // Leaf 2 is remapped; leaves 0, 1, 3 keep their backing.
            let old = c[&base];
            let runs = vec![
                unmap_run(base, &old),
                leaf_run(base, 2 * leaf, old.off, leaf),
                leaf_run(base + 2 * leaf, leaf, 0x2000_0000, leaf),
                leaf_run(base + 3 * leaf, leaf, old.off + 3 * leaf, leaf),
            ];
            let before = sim.0.borrow().n;
            let out = apply_and_commit(&m, &runs, &mut c);
            let after = sim.0.borrow().n;
            assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
            assert_eq!(out.remade_unchanged_pages, 0);
            assert_eq!(
                (
                    after.rm_unmap - before.rm_unmap,
                    after.rm_map - before.rm_map
                ),
                (1, if low { 1 } else { leaf / P }),
                "leaf {leaf:#x} low={low}: one range for the changed leaf, then only it mapped"
            );
            check(&sim, &c, &BTreeMap::new(), base, base + 4 * leaf).unwrap();
            assert_eq!(
                m.bv.leaf_reserved.load(Relaxed) > 0,
                low,
                "reservations iff available"
            );
            assert_eq!(m.bv.rigid_seen.load(Relaxed), 0);
        }
    }
}

/// How the flat FB alias of [`a_flat_fb_alias_straddling_the_carve_out_is_placed_below_it`] meets
/// host RM's micro reservations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AliasHost {
    /// Reservations accepted (the default): ONE reservation over the whole-leaf row.
    Accepts,
    /// The whole-row reservation is refused but a leaf-sized one is accepted (D1 (a2)).
    RefusesBig,
    /// Every reservation refused: 4 KiB grain, which the 8 GiB row exceeds → refused by name.
    RefusesAll,
    /// Reservations off (`KF3_NEGCTL_NO_MICRO_RESERVE`): the same refusal.
    Off,
}

/// ★★★ REGRESSION 2026-10-10 (`integration/windows-20261010` @ 6fafcc6e: fast suite 0/30, every
/// guest kernel CE channel `REFUSED-AND-POISONED`), through the real [`BatchedVas`] over the host RM
/// model. The measured shape: a guest-KERNEL space's flat FB alias, ONE run `0x120000000+0x1efc00000`
/// of 2 MiB leaves naming store `0x0`, an 8 GiB store whose carve-out starts at `0x1efbe0000` — the
/// run's last leaf straddles the carve-out base. The part below the carve-out is placed (the
/// straddling leaf's lower part in pieces that never reach the carve-out), so the ring at VA
/// `0x30fb55000` translates; no carve-out byte is mapped; and the walker's later UNMAP of the
/// committed run removes exactly our mappings — no `NV01` split, no `gvaspaceFree` assertion.
///
/// ★ D1/D3 (2026-10-10): the whole-leaf row is 7.9 GiB = 2 031 616 grains > `MAX_LEAF_PIECES`, so it
/// can ONLY be placed through a micro reservation. Four host behaviours: accepted (ONE reservation,
/// ONE mapping); the whole-row reservation refused but leaf-sized ones accepted ((a2): one per
/// leaf — still exact, still placed: the regression class cannot return); everything refused, or
/// reservations off (the residual: refused by name, the walker's retry; nothing placed or lost).
fn flat_alias(host: AliasHost) {
    const STORE: u64 = 0x2_0000_0000;
    const CARVE: u64 = STORE - 0x1042_0000;
    const VA: u64 = 0x1_2000_0000;
    const LEN: u64 = 0x1_efc0_0000;
    const LEAF: u64 = 0x20_0000;
    // The guest reservation lies elsewhere: the alias goes through the `NV01` range.
    let sim = Sim::new(space(0x80_0000_0000, 0x80_4000_0000));
    sim.0.borrow_mut().no_guard = true;
    match host {
        AliasHost::RefusesBig => sim.0.borrow_mut().refuse_reserve_over = Some(LEAF),
        AliasHost::RefusesAll => sim.0.borrow_mut().refuse_reserve = true,
        _ => {}
    }
    let m = SimMirror::new(&sim, true, host != AliasHost::Off);
    let id = |gpa: u64, _len: u64| Some(gpa);
    let cfg = ApplyCfg {
        store_bytes: STORE,
        carve: CARVE,
        carve_refuse: true,
        ..cfg(&id)
    };
    let run = DiffRun {
        leaf: LEAF,
        ..map_run(VA, LEN, false, 0, false)
    };
    let out = apply_entry(&m, &[run], &cfg);
    if matches!(host, AliasHost::RefusesAll | AliasHost::Off) {
        assert_eq!(
            (out.codes[0], out.refused),
            (kf_cuda::abi::KFWR_ACK_FAILED, 1),
            "{host:?}"
        );
        assert!(
            out.first_refusal
                .as_deref()
                .is_some_and(|w| w.contains(crate::batch::HUGE_ROW_OUTSIDE_RESERVATION)
                    || w.contains(crate::batch::REFRESH_BUDGET_EXHAUSTED)),
            "{host:?}: {:?}",
            out.first_refusal
        );
        // Refused by name either way: the grain bound (no reservation at all) or, when every
        // per-leaf reservation is refused too, the refresh's amplification budget (review item 4).
        // (The tail row may be refused by the spent budget too: counted per row, one run.)
        assert!(m.bv.huge_refused.load(Relaxed) + m.bv.budget_refused.load(Relaxed) >= 1);
        let rm = sim.0.borrow();
        assert!(
            rm.maps.is_empty() && rm.resv.is_empty(),
            "{host:?}: a refused row leaves nothing of ours behind"
        );
        return;
    }
    assert_eq!(
        (out.codes[0], out.refused),
        (KFWR_ACK_APPLIED, 0),
        "{host:?}: {:?}",
        out.first_refusal
    );
    {
        let rm = sim.0.borrow();
        assert_eq!(
            rm.translate(0x3_0fb5_5000, Owner::Mirror),
            Some((false, 0x1_efb5_5000)),
            "the kernel CE channel's ring (store below the carve-out) translates"
        );
        assert_eq!(
            rm.translate(VA + CARVE - P, Owner::Mirror),
            Some((false, CARVE - P))
        );
        for p in [VA + CARVE, VA + CARVE + 0x1_f000] {
            assert_eq!(
                rm.translate(p, Owner::Mirror),
                None,
                "carve-out page {p:#x}"
            );
        }
        assert!(
            rm.maps.iter().all(|x| x.off + x.len <= CARVE),
            "no host mapping reaches the carve-out"
        );
        assert_eq!(rm.maps.iter().map(|x| x.len).sum::<u64>(), CARVE);
        let whole = CARVE / LEAF * LEAF;
        match host {
            AliasHost::Accepts => assert_eq!(
                rm.maps.iter().filter(|x| x.len == whole).count(),
                1,
                "ONE host mapping for the whole-leaf row, through ONE reservation"
            ),
            _ => {
                assert_eq!(
                    rm.maps.iter().filter(|x| x.len == LEAF).count() as u64,
                    CARVE / LEAF,
                    "(a2): one mapping per leaf, each through its own reservation"
                );
                assert!(rm.resv.len() as u64 >= CARVE / LEAF);
            }
        }
    }
    assert_eq!(m.bv.unsafe_splits.load(Relaxed), 0);
    assert_eq!(m.bv.rigid_seen.load(Relaxed), 0);
    if host == AliasHost::RefusesBig {
        // ★ Review 3 item 4 — THE 6fafcc6e CLASS, GPU-free: with the whole-row reservation refused,
        // 3 965 leaves of 2 MiB land through the per-leaf tier and the refresh stays inside its
        // budget (2 amplified calls per leaf, one placement call per leaf and one for the tail).
        let (placed, amp) = m.bv.budget_spent();
        let leaves = CARVE / LEAF;
        assert_eq!(leaves, 3965);
        assert!(amp >= 2 * leaves && amp <= m.bv.amplification_budget, "{amp}");
        assert!(placed <= m.bv.placement_budget, "{placed}");
        assert_eq!(m.bv.budget_refused.load(Relaxed), 0);
        assert!(m.bv.leaf_reserved.load(Relaxed) >= leaves);
    }
    let c = Committed {
        len: LEN,
        held: false,
        ram: false,
        off: 0,
        run,
    };
    let out = apply_entry(&m, &[unmap_run(VA, &c)], &cfg);
    assert_eq!(out.refused, 0, "{host:?}: {:?}", out.first_refusal);
    let rm = sim.0.borrow();
    assert!(rm.maps.is_empty(), "{} mapping(s) left", rm.maps.len());
    assert!(rm.resv.is_empty(), "{host:?}: reservations are released");
    assert_eq!(rm.asserts, 0);
    assert!(rm.broken_mirror().is_empty());
    assert!(rm.violations.is_empty(), "{:?}", rm.violations);
}

#[test]
fn a_flat_fb_alias_straddling_the_carve_out_is_placed_below_it() {
    flat_alias(AliasHost::Accepts);
}

#[test]
fn the_flat_fb_alias_survives_a_host_that_refuses_the_big_reservation() {
    flat_alias(AliasHost::RefusesBig);
}

#[test]
fn the_flat_fb_alias_is_refused_by_name_without_any_reservation() {
    flat_alias(AliasHost::RefusesAll);
    flat_alias(AliasHost::Off);
}

/// ★ Review fix 2026-10-10 (finding 3; owner rule: an identical TRANSLATION is unchanged, whatever
/// the leaf size). ⊘ This test was "a leaf-size change remakes only those VAs" — its guard keyed on
/// the leaf, so it could not see the sixteen re-made VAs transiently unmapped. Now: sixteen 4 KiB
/// leaves → one 64 KiB leaf over the same pages makes NO host call (the walker's table moves the
/// placement between page-size classes; the host keeps its sixteen mappings), and back again with
/// page 5 re-pointed is exact — one unmap, one map — because those host mappings are 4 KiB.
#[test]
fn a_leaf_size_change_with_the_same_translation_keeps_every_host_mapping() {
    for low in [false, true] {
        let (sim, base) = big_space();
        let m = SimMirror::new(&sim, true, low);
        let mut c = BTreeMap::new();
        // Neighbour leaves that must never be touched.
        let left = leaf_run(base, 0x1_0000, 0x3000_0000, 0x1_0000);
        let right = leaf_run(base + 0x2_0000, 0x1_0000, 0x3100_0000, 0x1_0000);
        let small = leaf_run(base + 0x1_0000, 0x1_0000, 0x3200_0000, 0x1000);
        assert_eq!(
            apply_and_commit(&m, &[left, small, right], &mut c).refused,
            0
        );
        let neighbours: Vec<(u64, u64, u32)> = sim
            .0
            .borrow()
            .maps
            .iter()
            .filter(|x| x.va < base + 0x1_0000 || x.va >= base + 0x2_0000)
            .map(|x| (x.va, x.len, x.hdma))
            .collect();
        // 4 KiB → 64 KiB, same backing: the walker unmaps the 4 KiB placement, maps one 64 KiB leaf.
        let up = vec![
            unmap_run(base + 0x1_0000, &c[&(base + 0x1_0000)]),
            leaf_run(base + 0x1_0000, 0x1_0000, 0x3200_0000, 0x1_0000),
        ];
        let before = sim.0.borrow().n;
        let out = apply_and_commit(&m, &up, &mut c);
        assert_eq!(out.refused, 0);
        assert_eq!(
            out.codes,
            vec![KFWR_ACK_APPLIED; 2],
            "both commit: the class moves"
        );
        assert_eq!(sim.0.borrow().n, before, "no host call at all");
        check(&sim, &c, &BTreeMap::new(), base, base + 0x3_0000).unwrap();
        // ... and back: one 64 KiB leaf → sixteen 4 KiB leaves, page 5 remapped.
        let mut down = vec![unmap_run(base + 0x1_0000, &c[&(base + 0x1_0000)])];
        down.push(leaf_run(base + 0x1_0000, 5 * P, 0x3200_0000, 0x1000));
        down.push(leaf_run(base + 0x1_5000, P, 0x3300_0000, 0x1000));
        down.push(leaf_run(base + 0x1_6000, 10 * P, 0x3200_6000, 0x1000));
        let before = sim.0.borrow().n;
        let out = apply_and_commit(&m, &down, &mut c);
        let after = sim.0.borrow().n;
        assert_eq!(out.refused, 0);
        assert_eq!(
            out.remade_unchanged_pages, 0,
            "exact: the host mappings are 4 KiB"
        );
        assert_eq!(
            (
                after.rm_unmap - before.rm_unmap,
                after.rm_map - before.rm_map
            ),
            (1, 1),
            "exactly the changed page"
        );
        check(&sim, &c, &BTreeMap::new(), base, base + 0x3_0000).unwrap();
        // The neighbours were never touched: exactly the host mappings they were placed with.
        let rm = sim.0.borrow();
        let nb = |m: &SimRm| -> Vec<(u64, u64, u32)> {
            m.maps
                .iter()
                .filter(|x| x.va < base + 0x1_0000 || x.va >= base + 0x2_0000)
                .map(|x| (x.va, x.len, x.hdma))
                .collect()
        };
        assert_eq!(nb(&rm), neighbours, "neighbour leaves untouched");
        assert_eq!(rm.remade_transients, 0);
    }
}

/// ★ D1 (2026-10-10) — **THE FORMER INEXACT CASE IS EXACT** (`V3_BATCHED_MAP.md` §8.7.3, §8.8).
/// A 64 KiB leaf placed outside a reservation used to be ONE `NV01` host mapping; when the guest
/// split it into 4 KiB leaves and re-pointed page 5 in the same refresh, host RM could not remove
/// page 5 alone (it frees the whole VA block), so the apply RE-MADE the mapping — 15 UNCHANGED
/// pages transiently unmapped (`remade_unchanged_pages` = 15). Now the leaf is placed through a
/// micro reservation (exact partial unmap) or at 4 KiB grain (each page its own mapping): the
/// change is ONE unmap and ONE map, nothing re-made, the guard sees no transient, in every
/// configuration — `NV01` range and guest reservation, reservations on and off.
#[test]
fn splitting_a_big_leaf_and_changing_part_of_it_is_exact_everywhere() {
    for (reserved, low, label) in [
        (false, true, "NV01 + micro reservation"),
        (false, false, "NV01 at 4 KiB grain"),
        (true, false, "guest reservation"),
    ] {
        let (sim, base) = big_space();
        let base = if reserved { base + (64 << 20) } else { base };
        let m = SimMirror::new(&sim, true, low);
        let mut c = BTreeMap::new();
        let big = leaf_run(base, 0x1_0000, 0x3200_0000, 0x1_0000);
        assert_eq!(apply_and_commit(&m, &[big], &mut c).refused, 0);
        let down = vec![
            unmap_run(base, &c[&base]),
            leaf_run(base, 5 * P, 0x3200_0000, 0x1000),
            leaf_run(base + 5 * P, P, 0x3300_0000, 0x1000),
            leaf_run(base + 6 * P, 10 * P, 0x3200_6000, 0x1000),
        ];
        let before = sim.0.borrow().n;
        let out = apply_and_commit(&m, &down, &mut c);
        let after = sim.0.borrow().n;
        assert_eq!(out.refused, 0, "{label}: {:?}", out.first_refusal);
        assert_eq!(
            (
                after.rm_unmap - before.rm_unmap,
                after.rm_map - before.rm_map
            ),
            (1, 1),
            "{label}: exactly the changed page"
        );
        let rm = sim.0.borrow();
        assert!(rm.transient.is_empty(), "{label}: {:?}", rm.transient);
        assert_eq!(
            (out.remade_unchanged_pages, rm.remade_transients),
            (0, 0),
            "{label}"
        );
        drop(rm);
        check(&sim, &c, &BTreeMap::new(), base, base + 0x1_0000).unwrap();
        assert_eq!(m.bv.unsafe_splits.load(Relaxed), 0, "{label}");
        assert_eq!(m.bv.rigid_seen.load(Relaxed), 0, "{label}");
    }
}

/// ★ D1 — **the proof that SPLIT_OUTSIDE_RESERVATION and the apply's re-make are unreachable.**
/// Placing every shape the apply can ask for — leaves of 4 KiB / 64 KiB / 2 MiB, aligned and not,
/// rows straddling the guest reservation's edges, SKED pages, batches — leaves NO mapping of ours in
/// the `NV01` range that is bigger than one 4 KiB page (`check` asserts it for every mirror
/// mapping), so [`BatchedVas::own_view`] reports nothing rigid and a page-aligned range unmap never
/// crosses a mapping's edge. The branch stays as the refusal for an UNALIGNED range, and the
/// re-make as a declared last resort for a ledger entry that breaks the rule; both are exercised
/// below by planting one.
#[test]
fn no_nv01_mapping_bigger_than_a_page_can_be_made() {
    for low in [false, true] {
        let (sim, base) = big_space();
        let m = SimMirror::new(&sim, true, low);
        let mut c = BTreeMap::new();
        let rows = [
            leaf_run(base, 0x1_0000, 0x4000_0000, 0x1_0000),
            leaf_run(base + 0x1_0000, 0x3_0000, 0x4100_0000, 0x1_0000),
            leaf_run(base + 0x20_0000, 0x40_0000, 0x4200_0000, 0x20_0000),
            // A big-leaf run that is NOT leaf-aligned.
            leaf_run(base + 0x80_0000 + 0x3000, 0x2_0000, 0x4300_3000, 0x1_0000),
            // Straddling the guest reservation's low edge (base + 64 MiB).
            leaf_run(base + (64 << 20) - 0x8000, 0x10_0000, 0x4400_0000, 0x1_0000),
        ];
        let out = apply_and_commit(&m, &rows, &mut c);
        assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
        check(&sim, &c, &BTreeMap::new(), base, base + (65 << 20)).unwrap();
        assert!(
            m.bv.own_view(base, base + (66 << 20)).rigid.is_empty()
                && m.bv.rigid_seen.load(Relaxed) == 0
        );
        // Every aligned sub-range of every row unmaps without a refusal.
        for off in [0x1000u64, 0x5000, 0x1_1000, 0x21_1000, 0x80_4000] {
            let r = m.bv.unmap_range(base + off, P, true);
            assert!(r.is_ok(), "low={low} off={off:#x}: {r:?}");
        }
        assert_eq!(m.bv.unsafe_splits.load(Relaxed), 0);
    }
}

/// ★ D1 — the LAST RESORT, planted. If a mapping bigger than a page ever did exist outside a
/// reservation (it cannot: [`no_nv01_mapping_bigger_than_a_page_can_be_made`]), the ledger would
/// report it rigid, [`BatchedVas::rigid_seen`] would count it, a range inside it would be refused
/// by name before any host call, and the apply would RE-MAKE it — declared, never silent, and never
/// a held invalidate. This plants one (host mapping and ledger entry) and checks all four.
#[test]
fn a_planted_rigid_mapping_is_counted_refused_and_remade_declared() {
    let (sim, base) = big_space();
    let m = SimMirror::new(&sim, true, false);
    let mut c = BTreeMap::new();
    // The planted 64 KiB host mapping + ledger entry + walker commit + row.
    sim.0
        .borrow_mut()
        .map(base, 0x1_0000, 0x10, 0x3200_0000, Owner::Mirror)
        .unwrap();
    m.bv.own.lock().unwrap().insert(base, 0x1_0000, None, None);
    m.rows.borrow_mut().insert(base, 0x1_0000);
    c.insert(
        base,
        Committed {
            len: 0x1_0000,
            held: false,
            ram: true,
            off: 0x3200_0000,
            run: leaf_run(base, 0x1_0000, 0x3200_0000, 0x1_0000),
        },
    );
    assert_eq!(m.bv.own_view(base, base + 0x1_0000).rigid.len(), 1);
    assert_eq!(m.bv.rigid_seen.load(Relaxed), 1, "counted");
    let r = m.bv.unmap_range(base + 5 * P, P, true);
    assert!(r.is_err_and(|e| e.contains("outside a VA-reserving hDma")));
    assert_eq!(m.bv.unsafe_splits.load(Relaxed), 1);
    assert!(
        sim.0.borrow().ranges.is_empty(),
        "refused before any host call"
    );
    // The guest splits the leaf and re-points page 5: the apply re-makes the 15 kept pages.
    let down = vec![
        unmap_run(base, &c[&base]),
        leaf_run(base, 5 * P, 0x3200_0000, 0x1000),
        leaf_run(base + 5 * P, P, 0x3300_0000, 0x1000),
        leaf_run(base + 6 * P, 10 * P, 0x3200_6000, 0x1000),
    ];
    let out = apply_and_commit(&m, &down, &mut c);
    assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
    assert_eq!(out.remade_unchanged_pages, 15, "declared");
    sim.0.borrow_mut().remade_transients = 0; // the declared transients are the point of this test
    check(&sim, &c, &BTreeMap::new(), base, base + 0x1_0000).unwrap();
}

/// ★ With micro reservations the unreserved range BATCHES (one map for 12 rows) and a partial
/// unmap is exact: one range call, no remap, the reservation kept until its last row goes.
#[test]
fn a_micro_reserved_batch_unmaps_a_piece_exactly() {
    let sim = fresh();
    let m = SimMirror::new(&sim, true, true);
    let mut c = BTreeMap::new();
    map_ram(&m, &mut c, &scattered(4, 12));
    {
        let rm = sim.0.borrow();
        assert_eq!(rm.maps.len(), 1, "ONE host mapping for 12 rows");
        assert_eq!((rm.n.rm_map, rm.n.rm_reserve, rm.n.rm_desc), (1, 1, 1));
    }
    let before = sim.0.borrow().n;
    unmap_pages(&m, &mut c, &[9]);
    let after = sim.0.borrow().n;
    assert_eq!(after.rm_unmap - before.rm_unmap, 1, "one exact range");
    assert_eq!(after.rm_map, before.rm_map, "no remap");
    assert_eq!(sim.0.borrow().resv.len(), 1, "the reservation stays");
    ok(&sim, &c, &BTreeMap::new());
    // A new row in the dead page goes THROUGH the reservation (the NV01 range would find the VA
    // held by it), and a row straddling the reservation's end is two pieces.
    map_ram(&m, &mut c, &[(9, 1, 2222)]);
    map_ram(&m, &mut c, &[(16, 4, 3000)]);
    ok(&sim, &c, &BTreeMap::new());
    let all: Vec<u64> = c.keys().map(|&v| (v - BASE) / P).collect();
    unmap_pages(&m, &mut c, &all);
    ok(&sim, &c, &BTreeMap::new());
    let rm = sim.0.borrow();
    assert!(rm.maps.is_empty() && rm.resv.is_empty() && rm.objs.is_empty());
}

/// ★ Gate 4's shape (`kf-gate4`: 192 scattered channel pages in a guest reservation): ONE batch;
/// one piece unmaps alone (1 call, the object stays); the teardown is 2 ranges and 1 free — the
/// same numbers with micro reservations on or off.
#[test]
fn gate4_numbers_are_unchanged() {
    for low in [false, true] {
        let sim = Sim::new(space(BASE, BASE + 512 * P));
        let m = SimMirror::new(&sim, true, low);
        let mut c = BTreeMap::new();
        let rows: Vec<DiffRun> = (0..192u64)
            .map(|i| map_run(pg(i), P, true, (191 - i) * 7 * P, false))
            .collect();
        let out = apply_and_commit(&m, &rows, &mut c);
        assert_eq!((out.batches, out.batched_runs, out.map_calls), (1, 192, 1));
        let piece = apply_and_commit(&m, &[unmap_run(pg(101), &c[&pg(101)])], &mut c);
        assert_eq!((piece.unmapped, piece.unmap_calls), (1, 1));
        assert_eq!(m.bv.book.lock().unwrap().len(), 1);
        assert_eq!(m.bv.frees.load(Relaxed), 0);
        let rest: Vec<DiffRun> = c.iter().map(|(&v, x)| unmap_run(v, x)).collect();
        let torn = apply_and_commit(&m, &rest, &mut c);
        assert_eq!((torn.range_unmaps, torn.unmap_calls), (2, 2));
        assert_eq!(m.bv.frees.load(Relaxed), 1);
        assert_eq!(
            m.bv.micro_made.load(Relaxed),
            0,
            "no micro reservation in a guest one"
        );
        check(&sim, &c, &BTreeMap::new(), BASE, pg(512)).unwrap();
    }
}

/// A range over two own placements with a FOREIGN mapping in the gap between them takes the two
/// placements and never the foreign one.
#[test]
fn a_range_never_reaches_a_foreign_mapping_between_own_placements() {
    for (lo, label) in [(4u64, "NV01 range"), (40, "NV50 reservation")] {
        let sim = fresh();
        let m = SimMirror::new(&sim, true, true);
        let mut c = BTreeMap::new();
        map_ram(&m, &mut c, &[(lo, 1, 10), (lo + 1, 1, 11)]);
        assert!(sim.0.borrow_mut().place_foreign(1, pg(lo + 2), P));
        map_ram(&m, &mut c, &[(lo + 3, 1, 12), (lo + 4, 1, 13)]);
        let foreign = BTreeMap::from([(1u32, (pg(lo + 2), P))]);
        let r = m.bv.unmap_range(pg(lo), 5 * P, true);
        for p in [lo, lo + 1, lo + 3, lo + 4] {
            c.remove(&pg(p));
        }
        m.rows.borrow_mut().clear();
        assert!(r.is_ok(), "{label}: {r:?}");
        check(&sim, &c, &foreign, BASE, pg(PAGES)).unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(sim.0.borrow().gap_bytes, 0, "{label}: no byte of a gap");
    }
}

/// A range that ends exactly where a foreign mapping starts leaves it alone.
#[test]
fn a_range_ending_at_a_foreign_mapping_leaves_it() {
    let sim = fresh();
    let m = SimMirror::new(&sim, true, true);
    let mut c = BTreeMap::new();
    assert!(sim.0.borrow_mut().place_foreign(7, pg(42), 2 * P));
    map_ram(&m, &mut c, &[(40, 1, 1), (41, 1, 9)]);
    unmap_pages(&m, &mut c, &[40, 41]);
    ok(&sim, &c, &BTreeMap::from([(7, (pg(42), 2 * P))]));
}

/// Sparse unmaps (every other page) around foreign pages: one verdict each, foreign intact.
#[test]
fn sparse_unmaps_around_foreign_pages() {
    for lo in [2u64, 34] {
        let sim = fresh();
        let m = SimMirror::new(&sim, true, true);
        let mut c = BTreeMap::new();
        let mut foreign = BTreeMap::new();
        map_ram(&m, &mut c, &[(lo, 1, 3), (lo + 2, 1, 4), (lo + 4, 1, 5)]);
        for (id, p) in [(1u32, lo + 1), (2, lo + 3)] {
            assert!(sim.0.borrow_mut().place_foreign(id, pg(p), P));
            foreign.insert(id, (pg(p), P));
        }
        unmap_pages(&m, &mut c, &[lo, lo + 2, lo + 4]);
        ok(&sim, &c, &foreign);
    }
}

/// Two own placements across an EMPTY gap: still one host call per owned span — a range is never
/// stretched over VA nothing of ours holds, known-empty or not.
#[test]
fn a_range_across_an_empty_gap_is_one_call_per_owned_span() {
    let sim = fresh();
    let m = SimMirror::new(&sim, true, true);
    let mut c = BTreeMap::new();
    map_ram(&m, &mut c, &[(40, 1, 10), (41, 1, 11)]);
    map_ram(&m, &mut c, &[(44, 1, 12), (45, 1, 13)]);
    m.bv.unmap_range(pg(40), 6 * P, true).unwrap();
    let rm = sim.0.borrow();
    assert_eq!(rm.ranges, vec![(pg(40), 2 * P), (pg(44), 2 * P)]);
    assert_eq!(rm.gap_bytes, 0);
    assert!(rm.maps.is_empty() && rm.objs.is_empty());
}

/// Nothing of ours in the range: no host call at all.
#[test]
fn a_range_over_nothing_of_ours_makes_no_host_call() {
    let sim = fresh();
    let m = SimMirror::new(&sim, true, true);
    assert!(sim.0.borrow_mut().place_foreign(3, pg(10), 4 * P));
    m.bv.unmap_range(pg(8), 8 * P, true).unwrap();
    assert!(sim.0.borrow().ranges.is_empty());
    ok(
        &sim,
        &BTreeMap::new(),
        &BTreeMap::from([(3, (pg(10), 4 * P))]),
    );
}

/// Whole-space retire with foreign windows present: every mirror mapping goes, every batch object
/// and micro reservation is freed, the foreign windows stay.
#[test]
fn retire_with_foreign_windows_takes_only_ours() {
    for low in [false, true] {
        let sim = fresh();
        let m = SimMirror::new(&sim, true, low);
        let mut c = BTreeMap::new();
        let foreign = BTreeMap::from([
            (1u32, (pg(0), 2 * P)),
            (2, (pg(50), 3 * P)),
            (3, (pg(90), P)),
        ]);
        for (&id, &(va, len)) in &foreign {
            assert!(sim.0.borrow_mut().place_foreign(id, va, len));
        }
        map_ram(&m, &mut c, &scattered(2, 10));
        map_ram(
            &m,
            &mut c,
            &[(30, 1, 1), (31, 1, 2), (32, 1, 3), (33, 1, 4)],
        );
        map_ram(
            &m,
            &mut c,
            &[
                (45, 1, 8),
                (46, 1, 80),
                (47, 1, 800),
                (53, 1, 9),
                (54, 1, 90),
            ],
        );
        map_ram(&m, &mut c, &scattered(64, 20));
        ok(&sim, &c, &foreign);
        assert_eq!(m.retire(), 0);
        c.clear();
        ok(&sim, &c, &foreign);
        let rm = sim.0.borrow();
        assert!(rm.maps.iter().all(|x| x.owner != Owner::Mirror));
        assert!(rm.objs.is_empty() && rm.resv.is_empty(), "low={low}");
    }
}

/// ★★ DELIBERATE VIOLATION — proof that the guard catches a remap design. Split-by-remap of a
/// 4-row `NV01` batch (unmap the whole block, re-map the three survivors) transiently unmaps three
/// UNCHANGED rows: the guard must report it after the very first host call.
#[test]
fn the_guard_catches_split_by_remap() {
    let sim = fresh();
    let v: &Sim = &sim;
    let stdin = std::io::stdin();
    let fd = std::os::fd::AsFd::as_fd(&stdin);
    let rows: Vec<Desired> = (0..4u64)
        .map(|i| Desired {
            va: pg(4 + i),
            len: P,
            off: (100 + 3 * i) * P,
            ram: true,
            kind: 0,
            perm: MapPerm::READ_WRITE,
            leaf: 0,
        })
        .collect();
    SpaceVerbs::map_scattered(&v, fd, &rows, true).unwrap();
    // The refresh unmaps row 1; rows 0, 2, 3 are UNCHANGED.
    sim.0.borrow_mut().guard = [0usize, 2, 3]
        .iter()
        .map(|&i| (rows[i].va, (true, rows[i].off)))
        .collect();
    SpaceVerbs::unmap_row(&v, pg(4), 4 * P, true).unwrap();
    let caught = sim.0.borrow().transient.len();
    SpaceVerbs::map_row(&v, &rows[0], true).unwrap();
    SpaceVerbs::map_scattered(&v, fd, &rows[2..], true).unwrap();
    let rm = sim.0.borrow();
    assert!(
        caught >= 3,
        "the whole-block unmap transiently unmapped 3 unchanged rows: {:?}",
        rm.transient
    );
    assert!(
        rm.transient[0].contains("UNCHANGED VA"),
        "{}",
        rm.transient[0]
    );
}

/// ★★★ THE HELD-HOLE REGRESSION (Windows run 289: 14 leaves HELD BY HOST, 4 host Xid 31 `FAULT_PTE`,
/// every faulting VA the TAIL of a 64 KiB unit: `0x1499c000+0x4000`, `0x15bac000+0x4000`,
/// `0x14588000+0x8000`). Host RM rounds a micro reservation to 64 KiB (start down, size up:
/// `[measured, run 289 dmesg]` `RangeLo 0x144d0000` for a request at `0x144d5000`), so a batch of
/// 12 pages held 16 — the 4 pages past the ledger's record were occupied, and when the guest mapped
/// them (a later refresh, the same process) the map found the VA held, the leaf was acknowledged
/// HELD and never mapped. `sim::check` fails any fixed map refused by a non-foreign occupant
/// ("SILENT ABSENCE"); the model's reservations are rounded like the host's.
#[test]
fn a_batch_leaves_no_unrecorded_pad_the_guest_cannot_map() {
    // (start page, rows) in the UNRESERVED part (pages 64..96 = two 64 KiB units): aligned start; unaligned
    // start and end; a tail pad AND a head pad.
    for (first, n) in [(64u64, 12u64), (69, 9), (72, 8), (80, 12), (84, 10)] {
        let sim = fresh();
        let m = SimMirror::new(&sim, true, true);
        let mut c = BTreeMap::new();
        map_ram(&m, &mut c, &scattered(first, n));
        assert_eq!(
            sim.0.borrow().resv.len(),
            1,
            "first={first} n={n}: the batch went through a micro reservation"
        );
        for (&_h, &(lo, hi)) in &sim.0.borrow().resv {
            assert!(
                m.bv.micro.lock().unwrap().get(&lo).is_some_and(|r| r.hi == hi),
                "first={first} n={n}: the host's block {lo:#x}..{hi:#x} is exactly the ledger's record"
            );
        }
        // The guest now maps the rest of the 64 KiB units the batch touched (the pads).
        let lo_unit = first / 16 * 16;
        let hi_unit = (first + n).div_ceil(16) * 16;
        let pads: Vec<(u64, u64, u64)> = (lo_unit..hi_unit)
            .filter(|&p| p < first || p >= first + n)
            .map(|p| (p, 1, 3000 + p))
            .collect();
        map_ram(&m, &mut c, &pads);
        ok(&sim, &c, &BTreeMap::new());
        assert_eq!(m.bv.held_ours.load(Relaxed), 0, "first={first} n={n}: HELD-BY-OURSELVES");
        assert!(
            c.values().all(|x| !x.held),
            "first={first} n={n}: no leaf acknowledged HELD without a foreign occupant"
        );
        // ... and every page of the unit translates (nothing silently absent).
        for p in lo_unit..hi_unit {
            assert!(
                sim.0.borrow().translate(pg(p), Owner::Mirror).is_some(),
                "first={first} n={n}: page {p} unmapped"
            );
        }
    }
}

/// A host buffer (foreign) in a pad IS a legitimate HELD — the only one: the model accepts it and
/// the leaf is acknowledged HELD, while the reservation is refused (NoMemory) and the rows go per run.
#[test]
fn a_foreign_buffer_in_the_pad_makes_the_reservation_impossible_not_the_leaf_silent() {
    let sim = fresh();
    assert!(sim.0.borrow_mut().place_foreign(1, pg(78), P)); // inside the 64 KiB unit of 64..76
    let m = SimMirror::new(&sim, true, true);
    let mut c = BTreeMap::new();
    map_ram(&m, &mut c, &scattered(64, 12));
    assert_eq!(sim.0.borrow().resv.len(), 0, "the unit is not ours to reserve");
    assert_eq!(sim.0.borrow().maps.len(), 12 + 1, "per run at the grain + the foreign buffer");
    let f = BTreeMap::from([(1u32, (pg(78), P))]);
    ok(&sim, &c, &f);
}

/// The micro reservation is refused (a host that does not accept small reservations): the rows go
/// per run, named and counted, exact.
#[test]
fn a_refused_micro_reservation_falls_back_per_run() {
    let sim = fresh();
    sim.0.borrow_mut().refuse_reserve = true;
    let m = SimMirror::new(&sim, true, true);
    let mut c = BTreeMap::new();
    map_ram(&m, &mut c, &scattered(4, 12));
    assert_eq!(m.bv.micro_refused.load(Relaxed), 1);
    assert_eq!(sim.0.borrow().maps.len(), 12, "per run");
    unmap_pages(&m, &mut c, &[9]);
    ok(&sim, &c, &BTreeMap::new());
}

/// The batch map fails AFTER its micro reservation was made: the reservation is released before
/// the refusal returns (the ledger stays exact), the rows then go per run — named, not lost.
#[test]
fn a_batch_map_failing_inside_its_reservation_releases_it() {
    let sim = fresh();
    sim.0.borrow_mut().fail_next_batch_map = true;
    let m = SimMirror::new(&sim, true, true);
    let mut c = BTreeMap::new();
    let out = apply_and_commit(
        &m,
        &scattered(4, 12)
            .iter()
            .map(|&(p, n, b)| map_run(pg(p), n * P, true, b * P, false))
            .collect::<Vec<_>>(),
        &mut c,
    );
    assert_eq!((out.batches, out.batch_fallbacks, out.mapped), (0, 1, 12));
    assert!(
        out.first_batch_fallback
            .is_some_and(|e| e.contains("injected"))
    );
    let rm = sim.0.borrow();
    assert!(
        rm.resv.is_empty() && rm.objs.is_empty(),
        "nothing of the failed batch is left"
    );
    drop(rm);
    assert_eq!(m.bv.micro_freed.load(Relaxed), 1);
    ok(&sim, &c, &BTreeMap::new());
}

/// More rows than one batch carries (> `BATCH_MAX_RUNS`) in the unreserved range: two batches, two
/// micro reservations; unmaps across their boundary are exact; the teardown frees both.
#[test]
fn many_rows_beyond_the_cap_are_two_reserved_batches() {
    let n = (crate::apply::BATCH_MAX_RUNS + 904) as u64;
    let sim = Sim::new(space(BASE + (n + 64) * P, BASE + (n + 128) * P));
    let m = SimMirror::new(&sim, true, true);
    let mut c = BTreeMap::new();
    let runs: Vec<DiffRun> = (0..n)
        .map(|i| map_run(pg(i), P, true, ((i * 7919) % 65_521 + 1) * P, false))
        .collect();
    let out = apply_and_commit(&m, &runs, &mut c);
    assert_eq!((out.batches, out.batched_runs), (2, n as usize));
    assert_eq!(sim.0.borrow().resv.len(), 2);
    let edge = crate::apply::BATCH_MAX_RUNS as u64;
    let cut: Vec<DiffRun> = (edge - 3..edge + 3)
        .map(|p| unmap_run(pg(p), &c[&pg(p)]))
        .collect();
    let r = apply_and_commit(&m, &cut, &mut c);
    assert_eq!(
        (r.range_unmaps, r.unmap_calls),
        (1, 1),
        "one range over two reservations"
    );
    assert_eq!(
        sim.0.borrow().n.rm_unmap,
        2,
        "one exact range per reservation"
    );
    check(&sim, &c, &BTreeMap::new(), BASE, pg(n)).unwrap();
    assert_eq!(m.retire(), 0);
    let rm = sim.0.borrow();
    assert!(rm.maps.is_empty() && rm.resv.is_empty() && rm.objs.is_empty());
}

/// ★ Review 2 item 4, CORRECTED by review 3 (items 6, 7) — the steer removes the placement row FIRST.
/// If `hand_to_host` then does not complete (host RM refused, here), mappings of OURS stay on the
/// host and the row is gone. The first reading of this (a stale mapping, fixed by RESTORING rows) was
/// wrong for the production path: the net apply unmaps every changed piece BY RANGE over the ledger
/// (`GpuMirror::unmap_range` does not consult the rows), so the walker's later UNMAP removes the
/// mapping all the same. This pins that on the real apply path; the restore (which held the rows
/// lock across a ledger walk and could resurrect a stale row) is deleted.
#[test]
fn a_steer_that_did_not_complete_still_lets_the_walkers_unmap_reach_the_host() {
    let sim = fresh();
    let m = SimMirror::new(&sim, true, false);
    let mut c = BTreeMap::new();
    map_ram(&m, &mut c, &[(4, 4, 700)]);
    let va = pg(4);
    // The steer: row first ...
    let row = m.rows.borrow_mut().remove(&va).unwrap();
    // ... then the hand-over, which host RM refuses.
    sim.0.borrow_mut().fail_unmaps = Some(UnmapFault {
        every: 1,
        after: false,
        seen: 0,
    });
    let over = m.bv.hand_to_host(va, row);
    sim.0.borrow_mut().fail_unmaps = None;
    assert!(matches!(over, crate::batch::HandOver::Refused(_)), "{over:?}");
    assert_eq!(
        crate::batch::steer_step(&over, 0, std::time::Duration::ZERO),
        crate::batch::SteerStep::Retry,
        "the host alloc does not go ahead on a refused hand-over"
    );
    assert!(!m.bv.own.lock().unwrap().is_empty(), "our mappings are still there");
    // The walker's UNMAP of the placement (the guest dropped the page), through the real apply.
    let out = apply_and_commit(&m, &[unmap_run(va, &c[&va])], &mut c);
    assert_eq!(out.refused, 0, "{:?}", out.first_refusal);
    assert!(
        sim.0.borrow().maps.iter().all(|x| x.owner != Owner::Mirror),
        "no stale host mapping"
    );
    assert!(m.bv.own.lock().unwrap().is_empty());
    ok(&sim, &c, &BTreeMap::new());
}

/// ★ Review 4 item 1 — degraded configuration (every reservation refused, host RM refusing unmaps):
/// the first try of a falcon-ctx steer removes the placement row and is refused; the guest ctx mapping
/// STAYS in the ledger. A SECOND alloc on the same channel finds no row (`rows.remove` → none) and
/// used to conclude "not mirrored yet" and run the host alloc unsteered (the vvid wrong-frames bug).
/// Now the ctx range is handed over anyway: still refused ⇒ no alloc; host RM recovered ⇒ `Free`
/// ⇒ the alloc goes; a range with nothing of ours (and no reservation over it) is `Free` at once.
#[test]
fn a_second_alloc_after_a_refused_steer_never_runs_the_host_alloc_unsteered() {
    use crate::batch::{HandOver, SteerStep, steer_step};
    let sim = fresh();
    sim.0.borrow_mut().refuse_reserve = true; // degraded: no reservation anywhere
    let m = SimMirror::new(&sim, true, true);
    let mut c = BTreeMap::new();
    map_ram(&m, &mut c, &[(4, 4, 700)]);
    let (va, ctx_len) = (pg(4), 2 * P);
    let step = |over: &HandOver| steer_step(over, 0, std::time::Duration::ZERO);
    // First alloc: row removed, host RM refuses the unmap.
    let row = m.rows.borrow_mut().remove(&va).unwrap();
    sim.0.borrow_mut().fail_unmaps = Some(UnmapFault { every: 1, after: false, seen: 0 });
    let over = m.bv.hand_to_host(va, row);
    assert!(matches!(over, HandOver::Refused(_)));
    assert_ne!(step(&over), SteerStep::Done);
    // Second alloc on the same channel: the row is gone ...
    assert!(m.rows.borrow_mut().remove(&va).is_none());
    // ... and the hand-over of the ctx range is attempted anyway; still refused ⇒ no alloc.
    let over = m.bv.hand_to_host(va, ctx_len);
    assert!(matches!(over, HandOver::Refused(_)), "{over:?}");
    assert_ne!(step(&over), SteerStep::Done, "the host alloc must not run");
    // Host RM recovers: Free ⇒ the alloc may go, and nothing of ours is left over the range.
    sim.0.borrow_mut().fail_unmaps = None;
    let over = m.bv.hand_to_host(va, ctx_len);
    assert_eq!(over, HandOver::Free);
    assert_eq!(step(&over), SteerStep::Done);
    assert!(!m.bv.own.lock().unwrap().any_in(va, va + ctx_len));
    // A range with nothing of ours at all is Free at once (the proof the alloc needs).
    assert_eq!(m.bv.hand_to_host(pg(40), ctx_len), HandOver::Free);
}

/// What one seed exercised (so a green run cannot be a run that never batched).
#[derive(Debug, Default, Clone, Copy)]
struct Stats {
    batches: usize,
    ranges: usize,
    fallbacks: usize,
    held: usize,
    retires: usize,
    micro: u64,
    /// ★ Review fix 2026-10-10: leaf-size re-expressions (same translation), and those that
    /// changed one page in the same refresh.
    releafs: usize,
    releaf_changes: usize,
    /// Refused unmaps injected, runs acknowledged FAILED, retries the walker made.
    injected: u64,
    failed_runs: usize,
    retries: usize,
    /// Unchanged pages the apply had to re-make (an `NV01` mapping the guest split), and the guard
    /// hits inside them.
    remade_pages: u64,
    remade_transients: u64,
    kept_runs: usize,
    /// ★ D1/D2: big-leaf rows placed through reservations / at 4 KiB grain; the most ledger
    /// entries any one lock hold touched; what the injected runtime refusals hit.
    leaf_reserved: u64,
    leaf_grained: u64,
    max_touched: u64,
    ext: ExtHits,
}

impl Stats {
    fn add(&mut self, a: &crate::apply::Applied) {
        self.batches += a.batches;
        self.ranges += a.range_unmaps;
        self.fallbacks += a.batch_fallbacks;
        self.held += a.held;
        self.failed_runs += a
            .codes
            .iter()
            .filter(|&&c| c == kf_cuda::abi::KFWR_ACK_FAILED)
            .count();
        self.remade_pages += a.remade_unchanged_pages;
        self.kept_runs += a.kept_runs;
    }
    fn sum(&mut self, o: Stats) {
        self.batches += o.batches;
        self.ranges += o.ranges;
        self.fallbacks += o.fallbacks;
        self.held += o.held;
        self.retires += o.retires;
        self.micro += o.micro;
        self.releafs += o.releafs;
        self.releaf_changes += o.releaf_changes;
        self.injected += o.injected;
        self.failed_runs += o.failed_runs;
        self.retries += o.retries;
        self.remade_pages += o.remade_pages;
        self.remade_transients += o.remade_transients;
        self.kept_runs += o.kept_runs;
        self.leaf_reserved += o.leaf_reserved;
        self.leaf_grained += o.leaf_grained;
        self.max_touched = self.max_touched.max(o.max_touched);
        self.ext.reserves += o.ext.reserves;
        self.ext.frees += o.ext.frees;
        self.ext.maps += o.ext.maps;
    }
}

/// ★★★ **The property test**: random walker diffs — maps (scattered, contiguous, vidmem,
/// read-only, 64 KiB leaves), remaps, partial and sparse unmaps, straddles of the reservation
/// edges, leaf-size re-expressions with the same translation (with and without a one-page change
/// in the same refresh), whole-space retires — through the REAL apply + [`BatchedVas`] code against
/// the model, with foreign mappings sprinkled into gaps. After EVERY host call: no unchanged VA
/// transiently unmapped; after EVERY step: every invariant of [`check`], and the walker's slot
/// never holds two placements over one VA ([`apply_and_commit`]).
///
/// ★ Review fix 2026-10-10 (finding 8): the same seeds again with host RM REFUSING every 5th /
/// 7th mirror unmap (before acting, and after acting), each refusal followed by the walker's
/// retry of exactly the runs acknowledged FAILED (its next diff) — the walker's slot and the
/// host must agree after it.
#[test]
fn property_batched_mirror_against_the_host_rm_model() {
    for (low, fault) in [
        (false, None),
        (true, None),
        (false, Some((5, false))),
        (true, Some((7, true))),
        (true, Some((5, false))),
        (false, Some((7, true))),
    ] {
        let mut failures = Vec::new();
        let mut total = Stats::default();
        for seed in 1..=300u64 {
            match run_seed(seed, 60, true, low, fault, None) {
                Ok(s) => total.sum(s),
                Err(e) => failures.push(format!("seed {seed}: {e}")),
            }
        }
        let kinds = [
            "VIOLATION",
            "OVERLAP",
            "TRANSIENT",
            "FAULT_PTE",
            "assertion",
            "translates",
            "retire",
        ];
        let histogram: Vec<(&str, usize)> = kinds
            .iter()
            .map(|k| (*k, failures.iter().filter(|f| f.contains(k)).count()))
            .collect();
        assert!(
            failures.is_empty(),
            "low={low} fault={fault:?}: {} of 300 seeds failed {histogram:?}; first: {}",
            failures.len(),
            failures[0]
        );
        eprintln!("property (batched, low_reserve={low}, fault={fault:?}): {total:?}");
        assert!(
            total.batches > 100
                && total.ranges > 100
                && total.held > 10
                && total.retires > 10
                && total.releafs > 100
                && total.releaf_changes > 30,
            "the property run did not exercise the batched path: {total:?}"
        );
        assert_eq!(
            total.micro > 50,
            low,
            "micro reservations iff low_reserve: {total:?}"
        );
        assert_eq!(
            fault.is_some(),
            total.injected > 100 && total.retries > 100,
            "refusals injected iff asked: {total:?}"
        );
    }
}

/// ★ D1/D3 (2026-10-10): the property with RUNTIME refusals of the micro-reservation machinery:
/// reservation allocs refused, reservation frees refused (the reservation stays tracked and is
/// retried), and a row map failing in the middle of a multi-piece row (the pieces already placed
/// are rolled back, nothing placed is lost). Every invariant of [`check`] after every step, the
/// walker's retry after every refusal, `remade_unchanged_pages` == 0, nothing rigid, and every
/// ledger lock hold within one chunk.
fn property_ext(low: bool, fault: Option<(u64, bool)>, ext: ExtFault, expect: [bool; 3]) {
    let mut failures = Vec::new();
    let mut total = Stats::default();
    for seed in 1..=300u64 {
        match run_seed(seed, 60, true, low, fault, Some(ext)) {
            Ok(s) => total.sum(s),
            Err(e) => failures.push(format!("seed {seed}: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "low={low} fault={fault:?} ext={ext:?}: {} of 300 seeds failed; first: {}",
        failures.len(),
        failures[0]
    );
    eprintln!("property_ext (low={low}, fault={fault:?}, ext={ext:?}): {total:?}");
    assert_eq!(total.remade_pages, 0, "the re-make never runs");
    assert!(total.max_touched <= crate::batch::LEDGER_CHUNK as u64 + 1);
    assert_eq!(
        (
            total.ext.reserves > 20,
            total.ext.frees > 20,
            total.ext.maps > 20
        ),
        (expect[0], expect[1], expect[2]),
        "the injected refusals really fired: {total:?}"
    );
    if low {
        assert!(
            total.leaf_reserved > 20,
            "reservations were used: {total:?}"
        );
    }
    if low && ext.reserve_every != 0 {
        assert!(
            total.leaf_grained > 20,
            "refused reservations fell back to the 4 KiB grain: {total:?}"
        );
    }
    assert_eq!(total.leaf_reserved == 0, !low, "{total:?}");
}

#[test]
fn property_micro_reservation_allocs_refused_at_run_time() {
    let ext = ExtFault {
        reserve_every: 3,
        ..ExtFault::default()
    };
    property_ext(true, None, ext, [true, false, false]);
}

#[test]
fn property_micro_reservation_frees_refused_at_run_time() {
    let ext = ExtFault {
        free_every: 2,
        ..ExtFault::default()
    };
    property_ext(true, None, ext, [false, true, false]);
}

#[test]
fn property_a_row_map_failing_mid_row_is_rolled_back() {
    let ext = ExtFault {
        map_every: 7,
        ..ExtFault::default()
    };
    property_ext(true, None, ext, [false, false, true]);
    property_ext(false, None, ext, [false, false, true]);
}

#[test]
fn property_all_runtime_refusals_together_with_unmap_refusals() {
    let ext = ExtFault {
        reserve_every: 4,
        free_every: 3,
        map_every: 11,
        seen: [0; 3],
    };
    property_ext(true, Some((7, true)), ext, [true, true, true]);
    property_ext(true, Some((5, false)), ext, [true, true, true]);
}

/// The same property on the per-run path (`KF3_NO_BATCHED_MAP=1`, the A/B opt-out), with and
/// without injected unmap refusals.
#[test]
#[ignore = "debug replay: KF_SIM_SEED, KF_SIM_FAULT=every,after, KF_SIM_BATCH, KF_SIM_LOW"]
fn property_debug_replay() {
    let seed: u64 = std::env::var("KF_SIM_SEED").unwrap().parse().unwrap();
    let fault = std::env::var("KF_SIM_FAULT").ok().map(|f| {
        let (a, b) = f.split_once(',').unwrap();
        (a.parse().unwrap(), b == "1")
    });
    let r = run_seed(
        seed,
        60,
        std::env::var_os("KF_SIM_BATCH").is_some(),
        std::env::var_os("KF_SIM_LOW").is_some(),
        fault,
        std::env::var("KF_SIM_EXT").ok().map(|f| {
            let v: Vec<u64> = f.split(',').map(|x| x.parse().unwrap()).collect();
            ExtFault {
                reserve_every: v[0],
                free_every: v[1],
                map_every: v[2],
                seen: [0; 3],
            }
        }),
    );
    eprintln!("{r:?}");
}

#[test]
fn property_per_run_path_against_the_host_rm_model() {
    for fault in [None, Some((6, false)), Some((6, true))] {
        for seed in 1..=100u64 {
            let s = run_seed(seed, 60, false, false, fault, None)
                .unwrap_or_else(|e| panic!("fault={fault:?} seed {seed}: {e}"));
            assert_eq!(s.batches, 0, "the opt-out never batches");
        }
    }
}

/// One refresh, then — as the walker's next diff would — a retry of exactly the runs acknowledged
/// FAILED, with injected refusals paused (a refused run stays a difference; its placement, if an
/// UNMAP, stays committed and is emitted again).
fn refresh(m: &SimMirror<'_>, runs: &[DiffRun], c: &mut BTreeMap<u64, Committed>, st: &mut Stats) {
    let out = apply_and_commit(m, runs, c);
    st.add(&out);
    if std::env::var_os("KF_SIM_DEBUG").is_some() {
        eprintln!(
            "  runs {:x?}\n  codes {:?} refusal {:?} own {:x?}",
            runs.iter()
                .map(|r| (r.unmap, (r.va - BASE) / P, r.len / P, r.at / P, r.leaf))
                .collect::<Vec<_>>(),
            out.codes,
            out.first_refusal,
            m.bv.own
                .lock()
                .unwrap()
                .within(BASE, pg(PAGES))
                .iter()
                .map(|(v, o)| ((v - BASE) / P, o.len / P))
                .collect::<Vec<_>>()
        );
    }
    let failed: Vec<DiffRun> = runs
        .iter()
        .zip(&out.codes)
        .filter(|&(_, &code)| code == kf_cuda::abi::KFWR_ACK_FAILED)
        .map(|(r, _)| *r)
        .collect();
    let sim = m.sim();
    if failed.is_empty() || (sim.0.borrow().fail_unmaps.is_none() && sim.0.borrow().ext.is_none()) {
        return;
    }
    // Only runs the walker would emit again: an UNMAP of a placement still committed, a MAP of
    // pages no committed placement holds.
    let failed: Vec<DiffRun> = failed
        .into_iter()
        .filter(|r| {
            if r.unmap {
                c.get(&r.va).is_some_and(|x| x.len == r.len)
            } else {
                true
            }
        })
        .collect();
    let (paused, paused_ext) = {
        let mut rm = sim.0.borrow_mut();
        (rm.fail_unmaps.take(), rm.ext.take())
    };
    st.retries += 1;
    let again = apply_and_commit(m, &failed, c);
    st.add(&again);
    let mut rm = sim.0.borrow_mut();
    rm.fail_unmaps = paused;
    rm.ext = paused_ext;
}

#[allow(clippy::too_many_lines)]
fn run_seed(
    seed: u64,
    steps: usize,
    batching: bool,
    low: bool,
    fault: Option<(u64, bool)>,
    ext: Option<ExtFault>,
) -> Result<Stats, String> {
    let mut st = Stats::default();
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let sim = fresh();
    sim.0.borrow_mut().fail_unmaps = fault.map(|(every, after)| UnmapFault {
        every,
        after,
        seen: seed % every,
    });
    sim.0.borrow_mut().ext = ext.map(|e| ExtFault {
        seen: [seed % 5, seed % 3, seed % 7],
        ..e
    });
    let mut m = SimMirror::new(&sim, batching, low);
    let mut c: BTreeMap<u64, Committed> = BTreeMap::new();
    let mut foreign: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
    let mut next_id = 1u32;
    let busy = |c: &BTreeMap<u64, Committed>, f: &BTreeMap<u32, (u64, u64)>, p: u64| {
        let va = pg(p);
        c.range(..=va)
            .next_back()
            .is_some_and(|(&v, x)| va < v + x.len)
            || f.values().any(|&(v, l)| v <= va && va < v + l)
    };
    let free_at = |c: &BTreeMap<u64, Committed>, p: u64| {
        !c.range(..=pg(p))
            .next_back()
            .is_some_and(|(&v, x)| pg(p) < v + x.len)
    };
    const BIG: u64 = 0x1_0000;
    const BIG_PAGES: u64 = BIG / P;
    for step in 0..steps {
        let op = rng.below(100);
        let what;
        if op < 40 {
            // MAP a window of uncommitted pages (foreign pages may be inside: HeldByHost).
            let start = rng.below(PAGES);
            let want = 1 + rng.below(24);
            let mut n = 0;
            while n < want && start + n < PAGES && free_at(&c, start + n) {
                n += 1;
            }
            let mut runs = Vec::new();
            let mut p = start;
            let ro = rng.below(100) < 15;
            while p < start + n {
                let k = (1 + rng.below(3)).min(start + n - p);
                let ram = rng.below(100) < 85;
                let back = rng.below(4096);
                runs.push(map_run(
                    pg(p),
                    k * P,
                    ram,
                    back * P,
                    ro && rng.below(2) == 0,
                ));
                p += k;
            }
            what = format!("map {} run(s) from page {start}", runs.len());
            refresh(&m, &runs, &mut c, &mut st);
        } else if op < 62 {
            // UNMAP (sparse or contiguous) the placements wholly inside a window.
            let start = rng.below(PAGES);
            let n = 1 + rng.below(16);
            let sparse = rng.below(3) == 0;
            let runs: Vec<DiffRun> = c
                .range(pg(start)..pg(start + n))
                .filter(|&(&v, x)| v + x.len <= pg(start + n))
                .enumerate()
                .filter(|(i, _)| !sparse || i % 2 == 0)
                .map(|(_, (&v, x))| unmap_run(v, x))
                .collect();
            what = format!("unmap {} run(s) in pages {start}+{n}", runs.len());
            refresh(&m, &runs, &mut c, &mut st);
        } else if op < 76 {
            // REMAP: unmap the placements inside a window and map new backing over them.
            let start = rng.below(PAGES);
            let n = 1 + rng.below(10);
            let old: Vec<(u64, Committed)> = c
                .range(pg(start)..pg(start + n))
                .filter(|&(&v, x)| v + x.len <= pg(start + n))
                .map(|(&v, &x)| (v, x))
                .collect();
            let mut runs: Vec<DiffRun> = old.iter().map(|(v, x)| unmap_run(*v, x)).collect();
            for (v, x) in &old {
                let mut q = *v;
                while q < v + x.len {
                    let k = ((1 + rng.below(3)) * P).min(v + x.len - q);
                    runs.push(map_run(
                        q,
                        k,
                        rng.below(100) < 85,
                        rng.below(4096) * P,
                        false,
                    ));
                    q += k;
                }
            }
            what = format!("remap {} placement(s) in pages {start}+{n}", old.len());
            refresh(&m, &runs, &mut c, &mut st);
        } else if op < 80 {
            // PARTIAL REMAP: some pages inside one committed placement change, the rest is
            // re-emitted with its old backing (the walker's diff) — the rest must stay mapped.
            let keys: Vec<u64> = c.iter().filter(|(_, x)| !x.held).map(|(&v, _)| v).collect();
            what = "partial remap".into();
            if let Some(&v) = keys.get(rng.below(keys.len() as u64 + 1) as usize) {
                let at = (v - BASE) / P;
                let n = c[&v].len / P;
                let changed: Vec<u64> = (at..at + n).filter(|_| rng.below(3) == 0).collect();
                let runs = remap_inside(&c, at, &changed, 2000 + rng.below(1000));
                refresh(&m, &runs, &mut c, &mut st);
            }
        } else if op < 86 {
            // ★ Review fix 2026-10-10 (finding 3): LEAF SIZES. A fresh 64 KiB-leaf placement; or a
            // committed placement re-expressed with another leaf size over the SAME translation
            // (4 KiB ↔ 64 KiB), sometimes with one page changed in the same refresh (the guest
            // splits a big leaf and re-points part of it).
            let keys: Vec<u64> = c
                .iter()
                .filter(|&(&v, x)| {
                    !x.held && x.ram && v.is_multiple_of(BIG) && x.len.is_multiple_of(BIG)
                })
                .map(|(&v, _)| v)
                .collect();
            let pick = if rng.below(2) == 0 {
                keys.get(rng.below(keys.len() as u64) as usize).copied()
            } else {
                None
            };
            if let Some(v) = pick {
                let x = c[&v];
                let big = x.run.leaf == BIG;
                let aligned = v.is_multiple_of(BIG) && x.len.is_multiple_of(BIG);
                let change = rng.below(2) == 0;
                let mut runs = vec![unmap_run(v, &x)];
                let piece = |va: u64, len: u64, at: u64, leaf: u64| DiffRun {
                    va,
                    len,
                    at,
                    leaf,
                    unmap: false,
                    held: false,
                    ..x.run
                };
                if big && aligned {
                    // Split ONE of its 64 KiB leaves into 4 KiB leaves (same translation), and
                    // maybe re-point one page of it.
                    let leaf_i = rng.below(x.len / BIG);
                    let (ls, le) = (v + leaf_i * BIG, v + (leaf_i + 1) * BIG);
                    if ls > v {
                        runs.push(piece(v, ls - v, x.off, BIG));
                    }
                    let k = ls + rng.below(BIG_PAGES) * P;
                    if change {
                        if k > ls {
                            runs.push(piece(ls, k - ls, x.off + (ls - v), P));
                        }
                        runs.push(piece(k, P, (3000 + rng.below(1000)) * P, P));
                        if k + P < le {
                            runs.push(piece(k + P, le - k - P, x.off + (k + P - v), P));
                        }
                        st.releaf_changes += 1;
                    } else {
                        runs.push(piece(ls, BIG, x.off + (ls - v), P));
                    }
                    if le < v + x.len {
                        runs.push(piece(le, v + x.len - le, x.off + (le - v), BIG));
                    }
                    st.releafs += 1;
                } else if aligned {
                    // 4 KiB leaves → 64 KiB leaves over the same pages.
                    runs.push(piece(v, x.len, x.off, BIG));
                    st.releafs += 1;
                } else {
                    runs.clear();
                }
                what = format!("re-leaf {v:#x}+{:#x} (big={big}, change={change})", x.len);
                if !runs.is_empty() {
                    refresh(&m, &runs, &mut c, &mut st);
                }
            } else {
                // A fresh 64 KiB-leaf row over a free aligned block.
                let b = rng.below(PAGES / BIG_PAGES);
                let n_leaves = 1 + rng.below(2).min(PAGES / BIG_PAGES - 1 - b);
                let (p0, n) = (b * BIG_PAGES, n_leaves * BIG_PAGES);
                what = format!("big map at page {p0}+{n}");
                if (p0..p0 + n).all(|q| !busy(&c, &foreign, q)) {
                    let run = DiffRun {
                        leaf: BIG,
                        ..map_run(pg(p0), n * P, true, rng.below(256) * BIG, false)
                    };
                    refresh(&m, &[run], &mut c, &mut st);
                }
            }
        } else if op < 91 {
            // A FOREIGN mapping lands in a gap (never inside a micro reservation: other kayfabe
            // code maps through the space's own routing, which RM refuses there).
            let p = rng.below(PAGES);
            let n = 1 + rng.below(3);
            if (p..p + n).all(|q| q < PAGES && !busy(&c, &foreign, q))
                && sim.0.borrow_mut().place_foreign(next_id, pg(p), n * P)
            {
                foreign.insert(next_id, (pg(p), n * P));
                next_id += 1;
            }
            what = format!("foreign at page {p}+{n}");
        } else if op < 96 {
            // Its owner removes a foreign mapping.
            let key = foreign
                .keys()
                .nth(rng.below(foreign.len() as u64 + 1) as usize)
                .copied();
            if let Some(id) = key {
                sim.0.borrow_mut().remove_foreign(id);
                foreign.remove(&id);
            }
            what = "foreign removed".into();
        } else {
            // The guest frees the VA space: retire, then a fresh mirror on the SAME host space.
            // The retire runs with unmap refusals paused (it must refuse nothing) but WITH the
            // reservation-free refusals: a refused free stays tracked (`leftovers`), and the
            // retire's second pass — faults paused — must release it.
            let (paused, paused_ext) = {
                let mut rm = sim.0.borrow_mut();
                let p = (rm.fail_unmaps.take(), rm.ext.take());
                rm.ext = p.1.map(|e| ExtFault {
                    reserve_every: 0,
                    map_every: 0,
                    ..e
                });
                p
            };
            let refused = m.retire();
            let stuck = m.bv.leftovers();
            if stuck != 0 {
                if sim.0.borrow().ext.is_none_or(|e| e.free_every == 0) {
                    return Err(format!(
                        "step {step}: retire left {stuck} mapping(s)/reservation(s)/object(s) of ours with no free refusal injected"
                    ));
                }
                // The caller would free the whole space; the model retries the release instead.
                sim.0.borrow_mut().ext = None;
                let _ = m.bv.drain();
            }
            {
                let mut rm = sim.0.borrow_mut();
                rm.fail_unmaps = paused;
                rm.ext = paused_ext;
            }
            if refused != 0 {
                return Err(format!("step {step}: retire refused {refused} row(s)"));
            }
            if m.bv.leftovers() != 0 {
                return Err(format!(
                    "step {step}: retire left {} mapping(s)/reservation(s)/object(s) of ours",
                    m.bv.leftovers()
                ));
            }
            st.micro += m.bv.micro_made.load(Relaxed);
            st.leaf_reserved += m.bv.leaf_reserved.load(Relaxed);
            st.leaf_grained += m.bv.leaf_grained.load(Relaxed);
            st.max_touched = st.max_touched.max(m.bv.hold_stats().0);
            c.clear();
            check(&sim, &c, &foreign, BASE, pg(PAGES))
                .map_err(|e| format!("step {step} (retire): {e}"))?;
            let rm = sim.0.borrow();
            if rm.maps.iter().any(|x| x.owner == Owner::Mirror) || !rm.resv.is_empty() {
                return Err(format!(
                    "step {step}: a mirror mapping or reservation survived the retire"
                ));
            }
            drop(rm);
            m = SimMirror::new(&sim, batching, low);
            st.retires += 1;
            what = "retire".into();
        }
        if std::env::var_os("KF_SIM_DEBUG").is_some() {
            eprintln!("step {step}: {what}");
        }
        check(&sim, &c, &foreign, BASE, pg(PAGES))
            .map_err(|e| format!("step {step} ({what}): {e}"))?;
        // ★ D1: the apply's re-make is a last resort that must never run, and nothing rigid may ever
        // exist; ★ D2: no lock hold may touch more than one chunk.
        if st.remade_pages != 0 || m.bv.rigid_seen.load(Relaxed) != 0 {
            return Err(format!(
                "step {step} ({what}): {} UNCHANGED page(s) re-made, {} rigid mapping(s) seen",
                st.remade_pages,
                m.bv.rigid_seen.load(Relaxed)
            ));
        }
        let touched = m.bv.hold_stats().0;
        if touched > crate::batch::LEDGER_CHUNK as u64 + 1 {
            return Err(format!(
                "step {step} ({what}): a ledger lock hold touched {touched} entries"
            ));
        }
        let splits = m.bv.unsafe_splits.load(Relaxed);
        if splits != 0 {
            return Err(format!(
                "step {step} ({what}): {splits} range(s) refused as unsafe splits"
            ));
        }
        // A range over a gap is a defect — except the retry of a range host RM answered with an
        // error AFTER acting (the ledger still listed what it had removed; idempotent).
        let gap = sim.0.borrow().gap_bytes;
        if gap != 0 && !fault.is_some_and(|(_, after)| after) {
            return Err(format!(
                "step {step} ({what}): a range unmap covered {gap:#x} gap bytes"
            ));
        }
    }
    st.micro += m.bv.micro_made.load(Relaxed);
    st.leaf_reserved += m.bv.leaf_reserved.load(Relaxed);
    st.leaf_grained += m.bv.leaf_grained.load(Relaxed);
    st.max_touched = st.max_touched.max(m.bv.hold_stats().0);
    st.ext = sim.0.borrow().ext_hits;
    st.injected = sim.0.borrow().injected;
    st.remade_transients = sim.0.borrow().remade_transients;
    Ok(st)
}

/// The two workload shapes of the syscall budget (`V3_BATCHED_MAP.md` §8.4).
#[derive(Debug, Clone, Copy)]
enum Shape {
    /// Windows-like: entries of 1-3 allocations of 1-16 scattered 4 KiB pages, VA-adjacent (a
    /// bump allocator with reuse), in the UNRESERVED range; 40 % of steps free one allocation.
    Windows,
    /// CUDA-like: allocations of 256-2048 scattered pages in a guest reservation, freed whole.
    Cuda,
}

/// Run `steps` of `shape` and return the host calls + stitch syscalls, and the pages mapped.
fn workload(shape: Shape, batching: bool, low: bool, steps: usize) -> (Counters, u64) {
    let pages_total: u64 = 1 << 16;
    let (lo, resv) = match shape {
        Shape::Windows => (
            0u64,
            (BASE + (pages_total + 16) * P, BASE + (pages_total + 32) * P),
        ),
        Shape::Cuda => (0u64, (BASE, BASE + pages_total * P)),
    };
    let sim = Sim::new(space(resv.0, resv.1));
    sim.0.borrow_mut().no_guard = true;
    let m = SimMirror::new(&sim, batching, low);
    let mut c = BTreeMap::new();
    let mut rng = Rng(0xC0FF_EE11);
    let mut allocs: Vec<(u64, u64)> = Vec::new();
    let mut free_va: Vec<(u64, u64)> = Vec::new();
    let mut bump = lo;
    let mut mapped = 0u64;
    for _ in 0..steps {
        let free_one = !allocs.is_empty() && rng.below(100) < 40;
        if free_one {
            let i = rng.below(allocs.len() as u64) as usize;
            let (p, n) = allocs.swap_remove(i);
            let runs: Vec<DiffRun> = (p..p + n)
                .filter_map(|q| c.get(&pg(q)).map(|x| unmap_run(pg(q), x)))
                .collect();
            apply_and_commit(&m, &runs, &mut c);
            free_va.push((p, n));
            continue;
        }
        let per_entry = match shape {
            Shape::Windows => 1 + rng.below(3),
            Shape::Cuda => 1,
        };
        let mut runs = Vec::new();
        for _ in 0..per_entry {
            let n = match shape {
                Shape::Windows => 1 + rng.below(16),
                Shape::Cuda => 256 + rng.below(1793),
            };
            let p = match free_va.iter().position(|&(_, l)| l >= n) {
                Some(i) => {
                    let (fp, fl) = free_va.swap_remove(i);
                    if fl > n {
                        free_va.push((fp + n, fl - n));
                    }
                    fp
                }
                None => {
                    let p = bump;
                    bump += n;
                    p
                }
            };
            if p + n >= pages_total {
                break;
            }
            allocs.push((p, n));
            for q in p..p + n {
                runs.push(map_run(pg(q), P, true, rng.below(1 << 20) * P, false));
            }
            mapped += n;
        }
        apply_and_commit(&m, &runs, &mut c);
    }
    let rest: Vec<DiffRun> = c.iter().map(|(&v, x)| unmap_run(v, x)).collect();
    apply_and_commit(&m, &rest, &mut c);
    let rm = sim.0.borrow();
    assert!(rm.transient.is_empty() && rm.violations.is_empty() && rm.asserts == 0);
    (rm.n, mapped)
}

/// ★ The syscall budget's counts (printed; `cargo test -p kf-mem workload -- --nocapture`), and
/// the guarantees: the CUDA numbers do not depend on the low-range policy; on the Windows shape
/// micro reservations cut the RM calls below per-run.
#[test]
fn workload_numbers() {
    let mut table = Vec::new();
    for shape in [Shape::Windows, Shape::Cuda] {
        let steps = match shape {
            Shape::Windows => 2000,
            Shape::Cuda => 60,
        };
        for (label, batching, low) in [
            ("per-run (KF3_NO_BATCHED_MAP)", false, false),
            ("batch, guest reservations only", true, false),
            ("batch + micro reservations", true, true),
        ] {
            let (n, mapped) = workload(shape, batching, low, steps);
            table.push((shape, label, n, mapped));
        }
    }
    for (shape, label, n, mapped) in &table {
        eprintln!(
            "{shape:?} {label}: pages={mapped} rm_map={} rm_unmap={} rm_desc={} rm_reserve={} rm_free={} rm_total={} ({:.3}/page) mmap={} munmap={} munmap_vmas={} stitched_MiB={}",
            n.rm_map,
            n.rm_unmap,
            n.rm_desc,
            n.rm_reserve,
            n.rm_free,
            n.rm_total(),
            n.rm_total() as f64 / *mapped as f64,
            n.mmap,
            n.munmap,
            n.munmap_vmas,
            n.stitched_bytes >> 20
        );
    }
    let get = |s: &str, l: &str| {
        table
            .iter()
            .find(|(sh, la, ..)| format!("{sh:?}") == s && la.starts_with(l))
            .map(|&(_, _, n, _)| n)
            .unwrap()
    };
    assert_eq!(
        get("Cuda", "batch, guest"),
        get("Cuda", "batch + micro"),
        "CUDA unchanged"
    );
    assert!(get("Windows", "batch + micro").rm_total() < get("Windows", "per-run").rm_total());
}

/// ★ Review 3 item 1 (`rv3_budget_refusal_progress_is_illusory`) — the adversarial shape that made a
/// "walk again while the refresh made progress" livelock: a run of 2 MiB leaves split by the carve-out
/// clip into two rows that each fit a fresh refresh's amplification budget ALONE but not together,
/// with every reservation refused. The first row lands (`mapped` 1), the second is refused for the
/// budget, and the run fails as a whole so the first row is TAKEN DOWN again (`taken_down` 1): net
/// nothing placed, every walk, forever. There is no follow-up walk any more, so this costs one
/// bounded refresh per walk the guest causes; the test pins the per-walk numbers and that nothing is
/// left on the host.
#[test]
fn a_run_whose_rows_fit_a_budget_alone_but_not_together_places_nothing_and_leaves_nothing() {
    const STORE: u64 = 0x2_0000_0000;
    const CARVE: u64 = STORE - 0x1042_0000;
    const LEAF: u64 = 0x20_0000;
    let sim = Sim::new(space(0x80_0000_0000, 0x80_4000_0000));
    sim.0.borrow_mut().no_guard = true;
    sim.0.borrow_mut().refuse_reserve = true; // the FALLBACK configuration
    let m = SimMirror::new(&sim, true, true);
    let id = |gpa: u64, _len: u64| Some(gpa);
    let cfg = ApplyCfg {
        store_bytes: STORE,
        carve: CARVE,
        carve_refuse: true,
        ..cfg(&id)
    };
    let off = 0x1_efa0_0000u64 - 256 * LEAF;
    let run = DiffRun {
        leaf: LEAF,
        ..map_run(0x1_2000_0000, 257 * LEAF, false, off, false)
    };
    let mut calls = Vec::new();
    for walk in 0..3 {
        let before = sim.0.borrow().n.rm_total();
        let out = apply_entry(&m, &[run], &cfg);
        calls.push(sim.0.borrow().n.rm_total() - before);
        assert_eq!(out.budget_refused, 1, "walk {walk}: refused for the budget, by name");
        assert_eq!(
            (out.mapped, out.taken_down),
            (1, 1),
            "walk {walk}: the row that fit landed and was taken down with its run"
        );
        assert!(sim.0.borrow().maps.is_empty(), "walk {walk}: nothing left on the host");
        assert_eq!(m.bv.leftovers(), 0);
    }
    // Each walk is one refresh with a fresh budget: the same bounded cost, never growing.
    assert!(calls.windows(2).all(|w| w[0] == w[1]), "{calls:?}");
    assert!(calls[0] <= kf_mem_budget() * 2 + 4096, "{calls:?}");
}

fn kf_mem_budget() -> u64 {
    crate::batch::REFRESH_PLACEMENT_BUDGET
}

