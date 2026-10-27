// ENS6 follow-up invariants, written after the first mutation ledger showed rules guarded by a
// single test or by none (back-off steps, refused windows, speculative skip, recycle limits,
// cleanup rechecks, retry and the slice).

fn e6_spend(slice: &slice::Slice, verbs: u32) {
    for _ in 0..verbs {
        slice.spend(0);
    }
}

#[test]
fn ens6_backoff_steps_double_to_the_cap_and_stop() {
    let (mut t, m, gap) = ens6::gapped();
    m.refuse_when(|_, _, sparse| sparse);
    let mut seen = Vec::new();
    for i in 0..9u64 {
        t.tick = t.tick.max(t.backoff_until);
        assert_eq!(t.admit(&m, gap + i * 4 * SPARSE_GRAIN, SPARSE_GRAIN), Ok(false));
        seen.push(t.backoff_step);
    }
    assert_eq!(seen, [BACKOFF_FIRST, 4, 8, 16, 32, 64, 64, 64, 64]);
    assert_eq!(BACKOFF_MAX, 64);
}

#[test]
fn ens6_a_success_resets_the_backoff_only_once_it_has_expired() {
    let (mut t, m, gap) = ens6::gapped();
    m.refuse_when(|_, _, sparse| sparse);
    t.admit(&m, gap, SPARSE_GRAIN).unwrap();
    assert_eq!(t.backoff_step, BACKOFF_FIRST);
    t.note_success();
    assert_eq!(t.backoff_step, BACKOFF_FIRST, "still backing off: not reset");
    t.tick = t.backoff_until;
    t.note_success();
    assert_eq!(t.backoff_step, 0);
}

#[test]
fn ens6_retry_advances_the_tick_even_with_nothing_pending() {
    let (mut t, m, _) = ens6::gapped();
    for _ in 0..3 {
        t.retry(&m).unwrap();
    }
    assert_eq!(t.tick, 3);
}

#[test]
fn ens6_adjacent_refused_tiles_merge_into_one_window() {
    let (mut t, m, gap) = ens6::gapped();
    m.refuse_when(|_, _, sparse| sparse);
    t.admit(&m, gap, SPARSE_GRAIN).unwrap();
    t.tick = t.backoff_until;
    t.admit(&m, gap + SPARSE_GRAIN, SPARSE_GRAIN).unwrap();
    assert_eq!(t.refused.len(), 1);
    assert_eq!(t.refused[&gap].0, gap + 2 * SPARSE_GRAIN);
}

#[test]
fn ens6_refused_windows_are_bounded() {
    let (mut t, m, gap) = ens6::gapped();
    m.refuse_when(|_, _, sparse| sparse);
    for i in 0..100u64 {
        t.tick = t.tick.max(t.backoff_until);
        let _ = t.admit(&m, gap + i * 4 * SPARSE_GRAIN, SPARSE_GRAIN);
    }
    assert_eq!(t.refused.len(), WINDOWS);
}

#[test]
fn ens6_a_refused_tile_costs_no_verb_after_the_backoff_ended() {
    let (mut t, m, gap) = ens6::gapped();
    m.refuse_when(|_, _, sparse| sparse);
    t.admit(&m, gap, SPARSE_GRAIN).unwrap();
    let verbs = m.verbs();
    t.tick = t.backoff_until + 1; // the back-off is over, the window is still remembered
    assert_eq!(t.admit(&m, gap, SPARSE_GRAIN), Ok(false));
    assert_eq!(m.verbs(), verbs);
    assert_eq!(t.stats.window_hits, 1);
}

#[test]
fn ens6_a_refused_window_expires_after_its_ttl() {
    let (mut t, m, gap) = ens6::gapped();
    m.refuse_when(|_, _, sparse| sparse);
    t.admit(&m, gap, SPARSE_GRAIN).unwrap();
    m.refuse_when(|_, _, _| false);
    t.tick = t.backoff_until + WINDOW_TTL;
    assert_eq!(t.admit(&m, gap, SPARSE_GRAIN), Ok(true), "asked again and accepted");
}

#[test]
fn ens6_skipped_refinements_leave_the_geometry_alone_over_many_maps() {
    let (mut t, m, coarse) = ens6::tree(1000, 990);
    for i in 0..990u64 {
        t.mark(LO + i * NORMAL_GRAIN, PAGE, true);
    }
    let before = t.blocks.len();
    for i in 0..100u64 {
        t.prepare_map(&m, 99, coarse + (i + 1) * NORMAL_GRAIN, PAGE, true)
            .unwrap();
    }
    assert_eq!(t.blocks.len(), before);
    assert_eq!(t.stats.speculative_skipped, 100);
    assert_eq!(m.verbs(), 0);
}

#[test]
fn ens6_normal_with_nothing_to_split_never_recycles() {
    let (mut t, m, _) = ens6::tree(1000, 998);
    assert_eq!(t.blocks.len(), 999, "at the cap, with many empty pairs");
    t.normal(&m, LO + 3 * NORMAL_GRAIN, PAGE).unwrap();
    assert_eq!(m.verbs(), 0);
    assert_eq!(t.blocks.len(), 999);
}

#[test]
fn ens6_a_plain_map_into_a_refined_block_at_the_cap_costs_no_verb() {
    let (mut t, m, _) = ens6::tree(1000, 998);
    let pieces = t
        .prepare_map(&m, 99, LO + 3 * NORMAL_GRAIN, PAGE, true)
        .unwrap();
    assert_eq!(pieces.len(), 1);
    assert_eq!(m.verbs(), 0);
    assert_eq!(t.stats.coarse_maps, 0, "already refined: not a coarse map");
}

#[test]
fn ens6_recycle_does_not_start_when_the_demand_slice_is_spent() {
    let (mut t, m, coarse) = ens6::tree(1000, 998);
    let slice = slice::Slice::default();
    e6_spend(&slice, slice::SLICE_VERBS);
    let clock = || m.now();
    let ops = slice::Sliced {
        inner: &m,
        slice: &slice,
        clock: &clock,
    };
    assert_eq!(t.admit(&ops, coarse + NORMAL_GRAIN, SPARSE_GRAIN), Ok(false));
    assert_eq!(m.verbs(), 0);
    assert!(t.stats.slice_deferred >= 1);
}

#[test]
fn ens6_optional_recycle_stops_at_half_the_slice_but_demand_still_recycles() {
    let (mut t, m, coarse) = ens6::tree(1000, 998);
    let slice = slice::Slice::default();
    e6_spend(&slice, slice::SPECULATIVE_VERBS);
    let clock = || m.now();
    let ops = slice::Sliced {
        inner: &m,
        slice: &slice,
        clock: &clock,
    };
    t.prepare_map(&ops, 99, coarse + 7 * NORMAL_GRAIN, PAGE, true)
        .unwrap();
    assert_eq!(m.verbs(), 0, "optional work stopped");
    assert_eq!(t.stats.coarse_maps, 1);
    assert_eq!(t.admit(&ops, coarse + NORMAL_GRAIN, SPARSE_GRAIN), Ok(true));
    assert!(m.verbs() > 0, "the demand half of the slice is still there");
}

#[test]
fn ens6_a_refused_merge_is_a_counted_fallback_for_the_admission() {
    let (mut t, m, coarse) = ens6::tree(1000, 998);
    m.refuse_when(|_, len, sparse| !sparse && len == 2 * NORMAL_GRAIN);
    let result = t.admit(&m, coarse + NORMAL_GRAIN, SPARSE_GRAIN);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(t.stats.admit_hard_err, 0);
    assert!(t.stats.rm_declined >= 1);
    assert!(!t.poisoned);
    assert_eq!(m.blocks.borrow().len(), t.blocks.len(), "geometry restored");
}

#[test]
fn ens6_a_refused_merge_backs_off_the_next_admission() {
    let (mut t, m, coarse) = ens6::tree(1000, 998);
    m.refuse_when(|_, len, sparse| !sparse && len == 2 * NORMAL_GRAIN);
    t.admit(&m, coarse + NORMAL_GRAIN, SPARSE_GRAIN).unwrap();
    let verbs = m.verbs();
    t.admit(&m, coarse + 3 * NORMAL_GRAIN, SPARSE_GRAIN).unwrap();
    assert_eq!(m.verbs(), verbs);
    assert!(t.stats.backoff_skips >= 1);
}

fn e6_queued_cleanup(n: u64) -> (Reservations, ens6::Metered, slice::Slice, u64) {
    let (mut t, m, base) = ens6::roomy(20000);
    let slice = slice::Slice::default();
    let mut guard = 0;
    while m.sparse_tiles().len() < n as usize {
        ens6::refresh(&mut t, &m, &slice, |t, ops| {
            let _ = t.admit(ops, base, n * SPARSE_GRAIN);
            t.retry(ops).unwrap();
        });
        guard += 1;
        assert!(guard < 100);
    }
    // Retire with the optional slice spent: everything is queued, nothing is converted.
    slice.renew();
    e6_spend(&slice, slice::SPECULATIVE_VERBS);
    {
        let clock = || m.now();
        let ops = slice::Sliced {
            inner: &m,
            slice: &slice,
            clock: &clock,
        };
        t.retire(&ops, base, n * SPARSE_GRAIN, false).unwrap();
    }
    assert_eq!(m.sparse_tiles().len(), n as usize, "nothing converted yet");
    (t, m, slice, base)
}

fn e6_drain(t: &mut Reservations, m: &ens6::Metered, slice: &slice::Slice) {
    for _ in 0..200 {
        ens6::refresh(t, m, slice, |t, ops| t.retry(ops).unwrap());
    }
}

#[test]
fn ens6_intent_that_returns_before_conversion_keeps_its_sparse_block() {
    let (mut t, m, slice, base) = e6_queued_cleanup(64);
    ens6::refresh(&mut t, &m, &slice, |t, ops| {
        assert_eq!(t.admit(ops, base, 8 * SPARSE_GRAIN), Ok(true));
    });
    e6_drain(&mut t, &m, &slice);
    let mut want: Vec<u64> = (0..8).map(|i| base + i * SPARSE_GRAIN).collect();
    let mut have = m.sparse_tiles();
    want.sort_unstable();
    have.sort_unstable();
    assert_eq!(have, want, "only the re-wanted tiles stay sparse");
}

#[test]
fn ens6_a_block_that_becomes_occupied_before_conversion_is_not_converted() {
    let (mut t, m, slice, base) = e6_queued_cleanup(64);
    t.mark(base, PAGE, true);
    e6_drain(&mut t, &m, &slice);
    assert_eq!(m.sparse_tiles(), vec![base]);
}

#[test]
fn ens6_cleanup_conversion_in_retry_respects_the_optional_slice() {
    let (mut t, m, slice, _) = e6_queued_cleanup(64);
    let verbs = m.verbs();
    slice.renew();
    e6_spend(&slice, slice::SPECULATIVE_VERBS);
    let clock = || m.now();
    let ops = slice::Sliced {
        inner: &m,
        slice: &slice,
        clock: &clock,
    };
    t.retry(&ops).unwrap();
    assert_eq!(m.verbs(), verbs);
    assert_eq!(m.sparse_tiles().len(), 64);
}

#[test]
fn ens6_retry_with_a_spent_slice_starts_no_deferred_run() {
    let (mut t, m, base) = ens6::roomy(20000);
    let slice = slice::Slice::default();
    ens6::refresh(&mut t, &m, &slice, |t, ops| {
        let _ = t.admit(ops, base, 8000 * SPARSE_GRAIN);
    });
    let (verbs, calls, deferred) = (m.verbs(), t.stats.admit_calls, t.stats.slice_deferred);
    slice.renew();
    e6_spend(&slice, slice::SLICE_VERBS);
    let clock = || m.now();
    let ops = slice::Sliced {
        inner: &m,
        slice: &slice,
        clock: &clock,
    };
    t.retry(&ops).unwrap();
    assert_eq!(m.verbs(), verbs);
    assert_eq!(t.stats.admit_calls, calls, "the pending run was not even started");
    assert!(t.stats.slice_deferred > deferred);
}

#[test]
fn ens6_gap_tiles_are_reserved_within_the_slice_and_the_rest_resumes() {
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
    let mut t = Reservations::new(&rows, 20000);
    let m = ens6::Metered::with_roots(&rows);
    let gap = LO + 64 * NORMAL_GRAIN;
    let slice = slice::Slice::default();
    let bound = slice::SLICE_NS + ens6::OVERSHOOT_VERBS * ens6::VERB_NS;
    let spent = ens6::refresh(&mut t, &m, &slice, |t, ops| {
        assert_eq!(t.admit(ops, gap, 6000 * SPARSE_GRAIN), Ok(false));
    });
    assert!(spent <= bound, "{spent}");
    let first = m.sparse_tiles().len();
    assert!(
        (1000..=slice::SLICE_VERBS as usize + 8).contains(&first),
        "{first}"
    );
    let mut guard = 0;
    while m.sparse_tiles().len() < 6000 {
        let spent = ens6::refresh(&mut t, &m, &slice, |t, ops| t.retry(ops).unwrap());
        assert!(spent <= bound, "{spent}");
        guard += 1;
        assert!(guard < 50);
    }
}

#[test]
fn ens6_slice_peak_tracks_the_busiest_refresh_including_the_running_one() {
    let s = slice::Slice::default();
    for _ in 0..5 {
        s.spend(1000);
    }
    assert_eq!(s.peak(), (5, 5000), "the running refresh counts");
    s.renew();
    for _ in 0..2 {
        s.spend(10);
    }
    assert_eq!(s.spent(), (2, 20));
    assert_eq!(s.peak(), (5, 5000), "a quieter refresh does not lower the peak");
    s.renew();
    s.spend(7);
    s.renew();
    assert_eq!(s.peak(), (5, 5000));
}

#[test]
fn ens6_optional_work_stops_before_demand_work_and_each_has_both_limits() {
    let s = slice::Slice::default();
    assert!(!s.exhausted(false) && !s.exhausted(true));
    e6_spend(&s, slice::SPECULATIVE_VERBS);
    assert!(s.exhausted(false) && !s.exhausted(true));
    e6_spend(&s, slice::SLICE_VERBS - slice::SPECULATIVE_VERBS);
    assert!(s.exhausted(true));
    s.renew();
    s.spend(slice::SPECULATIVE_NS);
    assert!(s.exhausted(false) && !s.exhausted(true), "by time alone");
    s.spend(slice::SLICE_NS);
    assert!(s.exhausted(true));
}

#[test]
fn ens6_a_slow_rm_hits_the_time_limit_before_the_verb_limit() {
    let (mut t, m, base) = ens6::roomy(20000);
    m.verb_ns.set(1_000_000); // 1 ms per verb: the 100 ms limit is 100 verbs
    let slice = slice::Slice::default();
    let spent = ens6::refresh(&mut t, &m, &slice, |t, ops| {
        assert_eq!(t.admit(ops, base, 8000 * SPARSE_GRAIN), Ok(false));
    });
    assert!(spent <= slice::SLICE_NS + ens6::OVERSHOOT_VERBS * 1_000_000, "{spent}");
    assert!(m.verbs() < 200, "{} verbs", m.verbs());
    assert!(m.verbs() >= 90, "{} verbs", m.verbs());
}

#[test]
fn ens6_fast_verbs_hit_the_verb_limit_before_the_time_limit() {
    let (mut t, m, base) = ens6::roomy(20000);
    m.verb_ns.set(100); // 0.1 us per verb: 4096 verbs are 0.4 ms
    let slice = slice::Slice::default();
    ens6::refresh(&mut t, &m, &slice, |t, ops| {
        assert_eq!(t.admit(ops, base, 8000 * SPARSE_GRAIN), Ok(false));
    });
    let limit = u64::from(slice::SLICE_VERBS) + ens6::OVERSHOOT_VERBS;
    assert!(m.verbs() <= limit, "{} verbs", m.verbs());
    assert!(m.verbs() >= u64::from(slice::SLICE_VERBS) - 8, "{} verbs", m.verbs());
}

#[test]
fn ens6_an_admission_refused_by_a_merge_is_retried_when_its_backoff_ends() {
    let (mut t, m, coarse) = ens6::tree(1000, 998);
    m.refuse_when(|_, len, sparse| !sparse && len == 2 * NORMAL_GRAIN);
    let va = coarse + NORMAL_GRAIN;
    assert_eq!(t.admit(&m, va, SPARSE_GRAIN), Ok(false));
    assert!(m.sparse_tiles().is_empty());
    m.refuse_when(|_, _, _| false); // RM recovers
    for _ in 0..BACKOFF_FIRST {
        t.retry(&m).unwrap();
    }
    assert!(m.sparse_tiles().contains(&va), "asked again and honoured");
}

#[test]
fn ens6_a_cached_refusal_answers_until_its_backoff_ends_then_is_reevaluated() {
    let (mut t, m, gap) = ens6::gapped();
    m.refuse_when(|_, _, sparse| sparse);
    assert_eq!(t.admit(&m, gap, SPARSE_GRAIN), Ok(false));
    let verbs = m.verbs();
    assert_eq!(t.admit(&m, gap, SPARSE_GRAIN), Ok(false));
    assert_eq!((m.verbs(), t.stats.negative_hits, t.stats.window_hits), (verbs, 1, 0));
    t.tick = t.backoff_until; // due
    assert_eq!(t.admit(&m, gap, SPARSE_GRAIN), Ok(false));
    assert_eq!(t.stats.negative_hits, 1, "no longer answered from the cache");
    assert_eq!(t.stats.window_hits, 1, "re-evaluated: the refused window answers");
    assert_eq!(m.verbs(), verbs);
}
