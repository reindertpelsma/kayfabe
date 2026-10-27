// ENS6 failing-first regressions (campaign 7330, host evidence).
//
// Measured on 7330: at the 8192-block cap every plain map re-ran recycle (release a, release b,
// reserve a merged 4 MiB) and the split; RM refused the reserve, the pair was restored, and the
// map was counted coarse: ~120 000 refusals, ~300 000 made / ~282 000 freed, 14 applies of
// 0.3 .. 1.95 s (about 17 us per verb). Boot-time hard errors were RM refusals of sparse tiles
// below 4.5 GiB, which host RM withholds. These tests state the rules that end that:
//   * speculative refinement above the high-water mark costs no RM verb,
//   * an RM NoMemory is a counted closed fallback with a back-off and a refused-window cache,
//   * optional layout work a refresh STARTS is bounded by a time slice, and resumes next refresh,
//   * recycle makes headroom in one batch (hysteresis).

mod ens6 {
    use super::*;

    /// Virtual cost of one RM verb (measured on 7330: 271 ms / 15 798 calls).
    pub const VERB_NS: u64 = 17_000;
    /// Extra verbs one step may overshoot the slice by (one replacement and its rollback).
    pub const OVERSHOOT_VERBS: u64 = 8;

    type Refuse = Box<dyn Fn(u64, u64, bool) -> bool>;

    /// A model host RM with a virtual clock: every verb costs `VERB_NS`.
    pub struct Metered {
        pub blocks: RefCell<BTreeMap<u32, (u64, u64, bool)>>,
        next: Cell<u32>,
        pub clock: Cell<u64>,
        pub reserves: Cell<u64>,
        pub releases: Cell<u64>,
        pub refusals: Cell<u64>,
        pub refuse: RefCell<Refuse>,
    }
    impl Metered {
        pub fn with_roots(rows: &[GuestVaRange]) -> Self {
            let m = Self {
                blocks: RefCell::new(BTreeMap::new()),
                next: Cell::new(rows.iter().map(|r| r.handle).max().unwrap_or(0) + 1),
                clock: Cell::new(0),
                reserves: Cell::new(0),
                releases: Cell::new(0),
                refusals: Cell::new(0),
                refuse: RefCell::new(Box::new(|_, _, _| false)),
            };
            for r in rows.iter().filter(|r| r.handle != 0) {
                m.blocks.borrow_mut().insert(r.handle, (r.lo, r.hi, false));
            }
            m
        }
        pub fn verbs(&self) -> u64 {
            self.reserves.get() + self.releases.get()
        }
        pub fn now(&self) -> u64 {
            self.clock.get()
        }
        pub fn refuse_when(&self, f: impl Fn(u64, u64, bool) -> bool + 'static) {
            *self.refuse.borrow_mut() = Box::new(f);
        }
        pub fn sparse_tiles(&self) -> Vec<u64> {
            self.blocks
                .borrow()
                .values()
                .filter(|b| b.2)
                .map(|b| b.0)
                .collect()
        }
    }
    impl Ops for Metered {
        fn reserve(&self, lo: u64, len: u64, sparse: bool) -> Result<u32, RmError> {
            self.clock.set(self.clock.get() + VERB_NS);
            self.reserves.set(self.reserves.get() + 1);
            if (self.refuse.borrow())(lo, len, sparse)
                || self
                    .blocks
                    .borrow()
                    .values()
                    .any(|&(a, b, _)| lo < b && a < lo + len)
            {
                self.refusals.set(self.refusals.get() + 1);
                return Err(RmError::NoMemory);
            }
            let h = self.next.get();
            self.next.set(h + 1);
            self.blocks.borrow_mut().insert(h, (lo, lo + len, sparse));
            Ok(h)
        }
        fn release(&self, h: u32) -> Result<(), RmError> {
            self.clock.set(self.clock.get() + VERB_NS);
            self.releases.set(self.releases.get() + 1);
            self.blocks.borrow_mut().remove(&h).expect("own handle");
            Ok(())
        }
    }

    /// `small` adjacent empty 2 MiB roots (all mergeable pairs), a gap, then one huge coarse root.
    pub fn tree(limit: usize, small: usize) -> (Reservations, Metered, u64) {
        let mut rows: Vec<_> = (0..small as u64)
            .map(|i| GuestVaRange {
                handle: i as u32 + 1,
                lo: LO + i * NORMAL_GRAIN,
                hi: LO + (i + 1) * NORMAL_GRAIN,
            })
            .collect();
        let coarse = LO + (small as u64 + 16) * NORMAL_GRAIN;
        rows.push(GuestVaRange {
            handle: small as u32 + 1,
            lo: coarse,
            hi: coarse + (1 << 32),
        });
        (
            Reservations::new(&rows, limit),
            Metered::with_roots(&rows),
            coarse,
        )
    }

    /// Two roots with an unreserved gap between them (the shape of host RM's withheld window).
    pub fn gapped() -> (Reservations, Metered, u64) {
        let rows = [
            GuestVaRange {
                handle: 1,
                lo: LO,
                hi: LO + 8 * NORMAL_GRAIN,
            },
            GuestVaRange {
                handle: 2,
                lo: LO + 4096 * NORMAL_GRAIN,
                hi: LO + 4104 * NORMAL_GRAIN,
            },
        ];
        (
            Reservations::new(&rows, 1000),
            Metered::with_roots(&rows),
            LO + 64 * NORMAL_GRAIN,
        )
    }

    /// One roomy root, for the slice tests.
    pub fn roomy(limit: usize) -> (Reservations, Metered, u64) {
        let rows = [GuestVaRange {
            handle: 1,
            lo: LO,
            hi: LO + (1 << 32),
        }];
        (
            Reservations::new(&rows, limit),
            Metered::with_roots(&rows),
            LO,
        )
    }

    pub fn refresh(
        t: &mut Reservations,
        m: &Metered,
        slice: &slice::Slice,
        f: impl FnOnce(&mut Reservations, &dyn Ops),
    ) -> u64 {
        slice.renew();
        let start = m.now();
        let clock = || m.now();
        let ops = slice::Sliced {
            inner: m,
            slice,
            clock: &clock,
        };
        f(t, &ops);
        m.now() - start
    }
}

#[test]
fn ens6_speculative_refine_above_the_reserve_issues_no_rm_verb() {
    let (mut t, m, coarse) = ens6::tree(1000, 990);
    let before = t.blocks.len();
    assert!(
        before + 2 + speculative_reserve(1000) > 1000,
        "setup is above the mark"
    );
    let pieces = t
        .prepare_map(&m, 99, coarse + 7 * NORMAL_GRAIN, PAGE, true)
        .unwrap();
    assert_eq!(pieces.len(), 1);
    assert_eq!(t.stats.coarse_maps, 1);
    assert_eq!(t.stats.speculative_skipped, 1);
    assert_eq!(
        m.verbs(),
        0,
        "a speculative refinement above the mark must not touch RM"
    );
    assert_eq!(t.blocks.len(), before);
}

#[test]
fn ens6_a_thousand_plain_maps_at_the_cap_cost_no_layout_verbs() {
    let (mut t, m, coarse) = ens6::tree(1000, 990);
    // 7330: RM refused the merged 4 MiB reservation, so every map churned and failed.
    m.refuse_when(|_, len, sparse| !sparse && len == 2 * NORMAL_GRAIN);
    for i in 0..1000u64 {
        let va = coarse + (i * 3 + 1) * NORMAL_GRAIN;
        t.prepare_map(&m, 99, va, PAGE, true).unwrap();
    }
    assert_eq!(t.stats.coarse_maps, 1000);
    assert_eq!(m.verbs(), 0, "no release/reserve ping-pong for plain maps");
    assert_eq!(t.stats.refused, 0);
}

#[test]
fn ens6_sparse_admit_refused_by_rm_is_a_counted_decline_not_a_hard_error() {
    let (mut t, m, gap) = ens6::gapped();
    m.refuse_when(|_, _, sparse| sparse);
    let blocks = t.blocks.len();
    assert_eq!(t.admit(&m, gap, 17 * SPARSE_GRAIN), Ok(false));
    assert_eq!(t.stats.admit_hard_err, 0);
    assert_eq!(t.stats.rm_declined, 1);
    assert_eq!(t.stats.refused, 1, "the RM refusal itself stays counted");
    assert_eq!(t.blocks.len(), blocks, "nothing half made is left behind");
    assert!(!t.poisoned);
}

#[test]
fn ens6_a_refused_window_is_not_asked_of_rm_again() {
    let (mut t, m, gap) = ens6::gapped();
    m.refuse_when(|_, _, sparse| sparse);
    assert_eq!(t.admit(&m, gap, 2 * SPARSE_GRAIN), Ok(false));
    let verbs = m.verbs();
    // A different run over a tile RM already refused: answered without a verb.
    assert_eq!(t.admit(&m, gap, SPARSE_GRAIN), Ok(false));
    assert_eq!(m.verbs(), verbs);
    assert_eq!(t.stats.window_hits, 1);
    assert_eq!(t.stats.admit_hard_err, 0);
}

#[test]
fn ens6_a_refusal_backs_off_layout_work_and_the_back_off_doubles_then_resets() {
    let (mut t, m, gap) = ens6::gapped();
    m.refuse_when(|_, _, sparse| sparse);
    let tile = |i: u64| gap + i * 4 * SPARSE_GRAIN;
    assert_eq!(t.admit(&m, tile(0), SPARSE_GRAIN), Ok(false)); // refusal 1: back off 2 ticks
    let v1 = m.verbs();
    assert_eq!(t.admit(&m, tile(1), SPARSE_GRAIN), Ok(false));
    assert_eq!(m.verbs(), v1, "silenced during the back-off");
    assert!(t.stats.backoff_skips >= 1);
    for _ in 0..BACKOFF_FIRST {
        t.retry(&m).unwrap();
    }
    assert_eq!(t.admit(&m, tile(2), SPARSE_GRAIN), Ok(false)); // asked again: refusal 2
    let v2 = m.verbs();
    assert!(v2 > v1, "the back-off expired");
    for _ in 0..BACKOFF_FIRST {
        t.retry(&m).unwrap();
    }
    assert_eq!(t.admit(&m, tile(3), SPARSE_GRAIN), Ok(false));
    assert_eq!(
        m.verbs(),
        v2,
        "the second back-off is longer than the first"
    );
    for _ in 0..BACKOFF_FIRST {
        t.retry(&m).unwrap();
    }
    assert_eq!(t.admit(&m, tile(4), SPARSE_GRAIN), Ok(false));
    assert!(m.verbs() > v2, "and it expires too");
    // A success ends the escalation: the next refusal starts from the first step again.
    m.refuse_when(|_, _, _| false);
    for _ in 0..BACKOFF_MAX {
        t.retry(&m).unwrap();
    }
    assert_eq!(t.admit(&m, tile(5), SPARSE_GRAIN), Ok(true));
    m.refuse_when(|_, _, sparse| sparse);
    assert_eq!(t.admit(&m, tile(6), SPARSE_GRAIN), Ok(false));
    let v3 = m.verbs();
    for _ in 0..BACKOFF_FIRST {
        t.retry(&m).unwrap();
    }
    assert_eq!(t.admit(&m, tile(7), SPARSE_GRAIN), Ok(false));
    assert!(m.verbs() > v3, "reset after a success");
}

#[test]
fn ens6_a_refused_split_is_rolled_back_and_the_next_map_does_not_retry_it() {
    let (mut t, m, coarse) = ens6::tree(100000, 2);
    // The second piece of every split is refused.
    m.refuse_when(move |lo, _, sparse| !sparse && lo > coarse && lo < coarse + (1 << 31));
    let blocks = t.blocks.len();
    t.prepare_map(&m, 99, coarse + 4 * NORMAL_GRAIN, PAGE, true)
        .unwrap();
    assert_eq!(t.stats.coarse_maps, 1);
    assert_eq!(t.stats.refused, 1);
    assert_eq!(t.blocks.len(), blocks, "geometry restored");
    assert!(!t.poisoned);
    let verbs = m.verbs();
    t.prepare_map(&m, 99, coarse + 9 * NORMAL_GRAIN, PAGE, true)
        .unwrap();
    assert_eq!(m.verbs(), verbs, "backing off: the next map costs no verb");
    assert!(t.stats.backoff_skips >= 1);
}

#[test]
fn ens6_recycle_makes_its_headroom_in_one_batch() {
    let limit = 4096;
    let (mut t, m, coarse) = ens6::tree(limit, limit - 2);
    assert_eq!(t.blocks.len(), limit - 1);
    // One explicit sparse admission needs room: recycle must open a whole batch, not one slot.
    assert_eq!(t.admit(&m, coarse + NORMAL_GRAIN, SPARSE_GRAIN), Ok(true));
    let free = limit - t.blocks.len();
    assert!(free >= recycle_headroom(limit), "free {free}");
}

#[test]
fn ens6_headroom_batch_means_the_next_admissions_do_not_recycle_again() {
    let limit = 4096;
    let (mut t, m, coarse) = ens6::tree(limit, limit - 2);
    t.admit(&m, coarse + NORMAL_GRAIN, SPARSE_GRAIN).unwrap();
    let releases = m.releases.get();
    let freed = t.stats.freed;
    for i in 2..6u64 {
        t.admit(&m, coarse + i * NORMAL_GRAIN, SPARSE_GRAIN)
            .unwrap();
    }
    // Each admission releases only the coarse piece it splits; none releases an empty pair.
    assert!(
        m.releases.get() - releases <= 8,
        "{}",
        m.releases.get() - releases
    );
    assert_eq!(t.stats.freed - freed, m.releases.get() - releases);
}

#[test]
fn ens6_one_refresh_starts_no_more_layout_work_than_the_slice() {
    let (mut t, m, base) = ens6::roomy(20000);
    let slice = slice::Slice::default();
    let run = 8000 * SPARSE_GRAIN;
    let spent = ens6::refresh(&mut t, &m, &slice, |t, ops| {
        assert_eq!(
            t.admit(ops, base, run),
            Ok(false),
            "deferred, not honoured yet"
        );
    });
    assert!(
        spent <= slice::SLICE_NS + ens6::OVERSHOOT_VERBS * ens6::VERB_NS,
        "one refresh spent {spent} ns of RM verbs"
    );
    assert!(t.stats.slice_deferred >= 1);
    assert!(!m.sparse_tiles().is_empty(), "partial progress is kept");
    assert!(m.sparse_tiles().len() < 8000);
}

#[test]
fn ens6_deferred_sparse_work_resumes_each_refresh_until_honoured_and_never_unsparses() {
    let (mut t, m, base) = ens6::roomy(20000);
    let slice = slice::Slice::default();
    let run = 8000 * SPARSE_GRAIN;
    let bound = slice::SLICE_NS + ens6::OVERSHOOT_VERBS * ens6::VERB_NS;
    ens6::refresh(&mut t, &m, &slice, |t, ops| {
        let _ = t.admit(ops, base, run);
    });
    let mut seen: std::collections::BTreeSet<u64> = m.sparse_tiles().into_iter().collect();
    assert!(
        seen.len() < 8000,
        "the first refresh must not do the whole run"
    );
    let mut refreshes = 1;
    while m.sparse_tiles().len() < 8000 {
        let spent = ens6::refresh(&mut t, &m, &slice, |t, ops| {
            t.retry(ops).unwrap();
        });
        assert!(spent <= bound, "refresh {refreshes} spent {spent}");
        let now: std::collections::BTreeSet<u64> = m.sparse_tiles().into_iter().collect();
        assert!(seen.is_subset(&now), "an already sparse VA lost coverage");
        assert!(now.len() > seen.len(), "no progress in refresh {refreshes}");
        seen = now;
        refreshes += 1;
        assert!(refreshes < 200, "never converges");
    }
    for i in 0..8000u64 {
        assert!(seen.contains(&(base + i * SPARSE_GRAIN)), "tile {i}");
    }
}

#[test]
fn ens6_saturated_churn_sim_keeps_every_refresh_inside_the_slice() {
    // 4000 plain maps and 100 sparse binds per refresh on a saturated tree whose RM refuses
    // the merged reservation (the 7330 shape); 20 refreshes; the walk gate is held for at most
    // the slice of layout verbs plus the overshoot, every time.
    let limit = 2000;
    let (mut t, m, coarse) = ens6::tree(limit, limit - 10);
    m.refuse_when(|_, len, sparse| !sparse && len == 2 * NORMAL_GRAIN);
    let slice = slice::Slice::default();
    let bound = slice::SLICE_NS + ens6::OVERSHOOT_VERBS * ens6::VERB_NS;
    let mut worst = 0;
    for r in 0..20u64 {
        let spent = ens6::refresh(&mut t, &m, &slice, |t, ops| {
            t.retry(ops).unwrap();
            for i in 0..4000u64 {
                let va = coarse + ((r * 4000 + i) % 2000 + 1) * NORMAL_GRAIN;
                t.prepare_map(ops, 99, va, PAGE, true).unwrap();
            }
            for i in 0..100u64 {
                let _ = t.admit(
                    ops,
                    coarse + (3000 + r * 100 + i) * NORMAL_GRAIN,
                    SPARSE_GRAIN,
                );
            }
            t.retry(ops).unwrap();
        });
        worst = worst.max(spent);
        assert!(spent <= bound, "refresh {r}: {spent} ns of layout verbs");
    }
    println!(
        "ENS6 churn_sim worst_refresh_layout_ns={worst} verbs={}",
        m.verbs()
    );
    assert!(
        m.verbs() < 2000,
        "was ~12 verbs per map without the fixes: {}",
        m.verbs()
    );
}

#[test]
fn ens6_retire_converts_at_most_one_slice_per_refresh_and_finishes_later() {
    let (mut t, m, base) = ens6::roomy(20000);
    let slice = slice::Slice::default();
    let run = 3000 * SPARSE_GRAIN;
    let mut guard = 0;
    while m.sparse_tiles().len() < 3000 {
        ens6::refresh(&mut t, &m, &slice, |t, ops| {
            let _ = t.admit(ops, base, run);
            t.retry(ops).unwrap();
        });
        guard += 1;
        assert!(guard < 100);
    }
    let bound = slice::SLICE_NS + ens6::OVERSHOOT_VERBS * ens6::VERB_NS;
    let spent = ens6::refresh(&mut t, &m, &slice, |t, ops| {
        t.retire(ops, base, run, false).unwrap();
    });
    assert!(spent <= bound, "retire spent {spent} ns in one refresh");
    let mut refreshes = 0;
    while !m.sparse_tiles().is_empty() {
        let spent = ens6::refresh(&mut t, &m, &slice, |t, ops| {
            t.retry(ops).unwrap();
        });
        assert!(spent <= bound, "cleanup refresh {refreshes}: {spent}");
        refreshes += 1;
        assert!(refreshes < 200, "cleanup never finishes");
    }
}
