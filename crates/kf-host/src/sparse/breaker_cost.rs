
// ── COST adversary (breaker, 2026-10-26): work per call must not grow with the tree (I4) ──────
//
// The space's reservation tree holds up to `BLOCK_LIMIT` (8192) blocks and only ever GROWS
// (nothing merges empty neighbours), so a long VA churn fills it. These tests build a full tree
// of N blocks through the public routing verb and time each verb in release mode. `admit`
// clones the whole tree and, on any failure or decline, rescans every block x 512 pages
// (sparse.rs:294, :316-329); `mark`/`retire` scan every block below the range end
// (sparse.rs:421-430, :397-402); `retry` scans every block x 32 tiles x 16 pages (:433-449).
// The table printed by `breaker_cost_scaling_table` is the measurement; the asserts are the
// invariant (bounded work per call).

mod cost {
    use super::*;
    use std::time::Instant;

    /// An RM that accepts everything (the tree's own cost is what is measured).
    #[derive(Default)]
    pub struct NullRm {
        next: Cell<u32>,
    }
    impl Ops for NullRm {
        fn reserve(&self, _: u64, _: u64, _: bool) -> Result<u32, RmError> {
            let h = self.next.get() + 2;
            self.next.set(h);
            Ok(h)
        }
        fn release(&self, _: u32) -> Result<(), RmError> {
            Ok(())
        }
    }

    pub const BASE: u64 = 1 << 40;

    /// A tree whose block count is `blocks` (+-1) and exactly at its limit: one populated
    /// tile in every other 2 MiB slot (each costs two blocks: the tile and its coarse gap).
    pub fn full_tree(blocks: usize) -> (Reservations, NullRm) {
        let span = (blocks as u64 + 8) * 2 * NORMAL_GRAIN;
        let mut t = Reservations::new(
            &[GuestVaRange {
                handle: 1,
                lo: BASE,
                hi: BASE + span,
            }],
            blocks,
        );
        let rm = NullRm::default();
        let mut i = 0;
        while t.blocks.len() + 2 <= blocks {
            let va = BASE + (2 * i + 1) * NORMAL_GRAIN;
            t.route_map(&rm, 9, va, PAGE, true).expect("plain map");
            i += 1;
        }
        t.limit = t.blocks.len(); // exactly at the cap
        (t, rm)
    }

    pub fn per_call_us(reps: u32, mut f: impl FnMut(u32)) -> f64 {
        let t0 = Instant::now();
        for k in 0..reps {
            f(k);
        }
        t0.elapsed().as_secs_f64() * 1e6 / f64::from(reps)
    }

    /// Declined sparse admission: the run needs one more block than the cap allows.
    pub fn decline_us(blocks: usize, reps: u32) -> f64 {
        let (mut t, rm) = full_tree(blocks);
        // a 64 KiB tile in an EMPTY gap slot near the middle of the space
        let va = BASE + 2 * (blocks as u64 / 4) * NORMAL_GRAIN;
        assert_eq!(t.admit(&rm, va, SPARSE_GRAIN), Ok(false), "precondition: declined");
        per_call_us(reps, |_| {
            let _ = t.admit(&rm, va, SPARSE_GRAIN);
        })
    }

    /// A refused (hard error) sparse admission: the run lands in a populated block.
    pub fn refused_us(blocks: usize, reps: u32) -> f64 {
        let (mut t, rm) = full_tree(blocks);
        // the coarse remainder after the last tile, populated in its first 2 MiB by a map that
        // could not be refined (refine = false stands for a declined/refused refinement)
        let last = t.blocks.values().next_back().map(|b| b.lo).expect("block");
        t.route_map(&rm, 9, last + PAGE, PAGE, false).expect("plain map");
        // a sparse run elsewhere in that coarse block: replace() meets the occupied mask
        let va = last + 8 * NORMAL_GRAIN;
        assert!(t.admit(&rm, va, SPARSE_GRAIN).is_err(), "precondition: hard refusal");
        per_call_us(reps, |_| {
            let _ = t.admit(&rm, va, SPARSE_GRAIN);
        })
    }

    /// One plain map + unmap at the top of the space (worst case for the prefix scans).
    pub fn map_unmap_us(blocks: usize, reps: u32) -> f64 {
        let (mut t, rm) = full_tree(blocks);
        let va = BASE + (blocks as u64 + 5) * 2 * NORMAL_GRAIN - 2 * PAGE;
        per_call_us(reps, |_| {
            let p = t.route_map(&rm, 9, va, PAGE, true).expect("plain map");
            for (_, a, l) in p {
                t.mark(a, l, false);
            }
        })
    }

    pub fn retry_us(blocks: usize, reps: u32) -> f64 {
        let (mut t, rm) = full_tree(blocks);
        per_call_us(reps, |_| {
            let _ = t.retry(&rm);
        })
    }

    pub fn retire_us(blocks: usize, reps: u32) -> f64 {
        let (mut t, rm) = full_tree(blocks);
        let va = BASE + (blocks as u64 + 5) * 2 * NORMAL_GRAIN - 2 * PAGE;
        per_call_us(reps, |_| {
            let _ = t.retire(&rm, va, SPARSE_GRAIN, false);
        })
    }

    /// A SUCCESSFUL admission also pays the full clone.
    pub fn admit_ok_us(blocks: usize, reps: u32) -> f64 {
        let (mut t, rm) = full_tree(blocks.saturating_add(8));
        t.limit = blocks + 4000;
        let va = BASE + 2 * (blocks as u64 / 4) * NORMAL_GRAIN;
        per_call_us(reps, |k| {
            let a = va + u64::from(k % 8) * NORMAL_GRAIN * 2;
            let _ = t.admit(&rm, a, SPARSE_GRAIN);
            let _ = t.retire(&rm, a, SPARSE_GRAIN, false);
        })
    }
}

#[test]
fn breaker_cost_scaling_table() {
    eprintln!("blocks | decline us | refused us | map+unmap us | retry us | retire us | admit+retire us");
    for n in [100usize, 1000, 5000, 8190, 20000] {
        let reps = if n >= 5000 { 3 } else { 20 };
        eprintln!(
            "{n:6} | {:10.1} | {:10.1} | {:12.1} | {:8.1} | {:9.1} | {:10.1}",
            cost::decline_us(n, reps),
            cost::refused_us(n, reps),
            cost::map_unmap_us(n, reps),
            cost::retry_us(n, reps),
            cost::retire_us(n, reps),
            cost::admit_ok_us(n, reps),
        );
    }
}

/// I4: a sparse admission the object budget DECLINES (the counted `overflow` fallback; 5000+ of
/// them in the Windows Enscape 07 stall) costs O(tree): at 8190 blocks it must still finish
/// within a few milliseconds, because the decision needs no clone and no rescan.
#[test]
fn breaker_i4_declined_admission_at_a_full_tree_is_cheap() {
    let us = cost::decline_us(8190, 5);
    assert!(
        us < 5_000.0,
        "a declined sparse admission at 8190 blocks takes {us:.0} us (the VA thread's whole budget)"
    );
}

/// I4: the per-call cost of a declined admission must not scale with the block count.
#[test]
fn breaker_i4_declined_admission_cost_does_not_scale_with_the_tree() {
    let small = cost::decline_us(800, 20).max(1.0);
    let large = cost::decline_us(8000, 5);
    assert!(
        large / small < 3.0,
        "10x the blocks costs {:.1}x ({small:.0} us -> {large:.0} us)",
        large / small
    );
}

/// I4: a hard-refused admission (the run lands in a populated coarse block) repeats on every
/// walk (it is ACKed FAILED); each repeat pays the same tree-wide clone and rescan.
#[test]
fn breaker_i4_refused_admission_at_a_full_tree_is_cheap() {
    let us = cost::refused_us(8190, 5);
    assert!(us < 5_000.0, "a refused sparse admission at 8190 blocks takes {us:.0} us");
}

/// I4: a plain map/unmap pair is O(blocks below it) because `mark` filters after `range_mut`.
#[test]
fn breaker_i4_plain_map_cost_does_not_scale_with_the_tree() {
    let small = cost::map_unmap_us(800, 200).max(0.05);
    let large = cost::map_unmap_us(8000, 200);
    assert!(
        large / small < 3.0,
        "10x the blocks costs {:.1}x ({small:.1} us -> {large:.1} us)",
        large / small
    );
}

/// I4: the refresh-time `retry` visits every block (x32 tiles x16 pages) on EVERY walk of a
/// space with sparse-enabled twin, whether or not any intent is held.
#[test]
fn breaker_i4_retry_with_no_held_intent_is_free() {
    let us = cost::retry_us(8190, 5);
    assert!(us < 500.0, "retry of a tree with no held intent takes {us:.0} us");
}

/// I4: a 4000-map apply entry (route_map + mark per map) at a full tree must not reach 25 ms.
#[test]
fn breaker_i4_four_thousand_map_apply_at_a_full_tree_stays_under_25ms() {
    let (mut t, rm) = cost::full_tree(8190);
    let t0 = std::time::Instant::now();
    for k in 0..4000u64 {
        let va = cost::BASE + (8190 + 4 + k % 3) * 2 * NORMAL_GRAIN - 2 * PAGE * (1 + k / 3 % 8);
        let _ = t.route_map(&rm, 9, va, PAGE, true);
    }
    let ms = t0.elapsed().as_secs_f64() * 1e3;
    assert!(ms < 25.0, "4000 maps at a full tree took {ms:.0} ms");
}

/// I4: 4000 sparse admissions the budget declines (the Windows stall counted 5000+) must stay
/// under a second; measured on 100 and scaled, because the real cost is ~0.15 s each.
#[test]
fn breaker_i4_four_thousand_declines_at_a_full_tree_stay_under_one_second() {
    const N: u32 = 40;
    let (mut t, rm) = cost::full_tree(8190);
    let va = cost::BASE + 2 * (8190 / 4) * NORMAL_GRAIN;
    let t0 = std::time::Instant::now();
    for _ in 0..N {
        let _ = t.admit(&rm, va, SPARSE_GRAIN);
    }
    let ms = t0.elapsed().as_secs_f64() * 1e3 * 4000.0 / f64::from(N);
    assert!(ms < 1000.0, "4000 declined admissions would take {ms:.0} ms");
}
