// ENS7: policy, permanent exclusions, current missing coverage and warning limits.
#[test]
fn ens7_eight_gib_budget_matches_the_natural_tile_bound() {
    let p = policy::Budget::from_framebuffer(8 << 30);
    assert_eq!((p.per_space, p.per_vm), (65536, 131072));
}
#[test]
fn ens7_small_framebuffer_retains_the_policy_floor() {
    let p = policy::Budget::from_framebuffer(1);
    assert_eq!((p.per_space, p.per_vm), (8192, 8192));
}
#[test]
fn ens7_hostile_framebuffer_cannot_raise_the_absolute_ceiling() {
    let p = policy::Budget::from_framebuffer(u64::MAX);
    assert_eq!((p.per_space, p.per_vm), (65536, 131072));
}
#[test]
fn ens7_middle_framebuffer_scales_without_rounding_up() {
    let p = policy::Budget::from_framebuffer((2 << 30) + SPARSE_GRAIN - 1);
    assert_eq!((p.per_space, p.per_vm), (32768, 32768));
}
fn ens7_guest() -> (Reservations, ens6::Metered) {
    let rows = [GuestVaRange {
        handle: 1,
        lo: LO,
        hi: LO + 4096 * NORMAL_GRAIN,
    }];
    (
        Reservations::new_guest(&rows, 128),
        ens6::Metered::with_roots(&rows),
    )
}
fn ens7_withheld(va: u64) {
    let (mut t, r) = ens7_guest();
    assert_eq!(t.admit(&r, va, SPARSE_GRAIN), Ok(false));
    assert_eq!(
        r.verbs(),
        0,
        "never ask RM for a permanently excluded window"
    );
    assert_eq!(t.counters().admit_hard_err, 0);
    assert_eq!(t.counters().withheld_windows, 1);
    for i in 0..300u64 {
        assert_eq!(t.admit(&r, va + i * SPARSE_GRAIN, SPARSE_GRAIN), Ok(false));
        t.retry(&r).unwrap();
    }
    assert_eq!(r.verbs(), 0);
    assert_eq!(
        t.counters().withheld_windows,
        1,
        "count once per permanent window"
    );
}
#[test]
fn ens7_rm_low_window_never_attempts_a_sparse_reservation() {
    ens7_withheld(0x1920_0000);
}
#[test]
fn ens7_host_placement_hole_never_attempts_a_sparse_reservation() {
    ens7_withheld(crate::channel::HOST_HOLE_LO);
}
fn ens7_missing(va: u64) {
    let (mut t, r) = (tree(100), Rm::new());
    t.mark(LO, PAGE, true);
    assert_eq!(t.admit(&r, va, SPARSE_GRAIN), Ok(false));
    assert_eq!(t.counters().current_unhonoured_bytes, SPARSE_GRAIN);
    assert_eq!(t.admit(&r, va, SPARSE_GRAIN), Ok(false));
    assert_eq!(
        t.counters().current_unhonoured_bytes,
        SPARSE_GRAIN,
        "unique current VA"
    );
    t.retire(&r, va, SPARSE_GRAIN / 2, false).unwrap();
    assert_eq!(t.counters().current_unhonoured_bytes, SPARSE_GRAIN / 2);
    t.retire(&r, va + SPARSE_GRAIN / 2, SPARSE_GRAIN / 2, false)
        .unwrap();
    assert_eq!(t.counters().current_unhonoured_bytes, 0);
}
#[test]
fn ens7_missing_bytes_do_not_accumulate_duplicate_prefix_declines() {
    ens7_missing(LO + NORMAL_GRAIN);
}
#[test]
fn ens7_missing_bytes_retire_a_declined_suffix_exactly_once() {
    ens7_missing(LO + 3 * NORMAL_GRAIN);
}
#[test]
fn ens7_retry_honouring_a_tile_clears_its_missing_bytes() {
    let (mut t, r) = (tree(100), Rm::new());
    populate(&mut t, &r, LO, PAGE);
    assert_eq!(t.admit(&r, LO, SPARSE_GRAIN), Ok(false));
    assert_eq!(t.counters().current_unhonoured_bytes, SPARSE_GRAIN);
    r.unmap(LO, PAGE);
    t.mark(LO, PAGE, false);
    t.retry(&r).unwrap();
    assert_eq!(t.counters().current_unhonoured_bytes, 0);
    assert!(block(&t, LO).sparse);
}
fn ens7_warning(va: u64) {
    let (mut t, r) = (tree(100), Rm::new());
    t.mark(LO, PAGE, true);
    assert_eq!(t.admit(&r, va, NORMAL_GRAIN), Ok(false));
    let message = t.missing_warning(0).expect("a missing MiB is never silent");
    assert!(message.contains("current_unhonoured_bytes="));
    assert!(message.contains("ranges="));
    assert!(!message.contains('\n'));
    assert!(message.len() <= 512);
    assert!(t.missing_warning(1).is_none(), "rate limited");
    assert!(t.missing_warning(30_000_000_000).is_some());
    t.retire(&r, va, NORMAL_GRAIN, false).unwrap();
    assert!(t.missing_warning(60_000_000_000).is_none());
}
#[test]
fn ens7_large_missing_prefix_warns_with_ranges_and_a_cooldown() {
    ens7_warning(LO + NORMAL_GRAIN);
}
#[test]
fn ens7_large_missing_suffix_stops_warning_after_retirement() {
    ens7_warning(LO + 3 * NORMAL_GRAIN);
}
fn ens7_twenty_thousand(fb: u64, complete: bool) {
    let p = policy::Budget::from_framebuffer(fb);
    let rows = [GuestVaRange {
        handle: 1,
        lo: LO,
        hi: LO + 20000 * SPARSE_GRAIN,
    }];
    let mut t = Reservations::new_guest(&rows, p.per_space);
    let r = ens6::Metered::with_roots(&rows);
    // Thousands of native reservations are deliberately spread across bounded refreshes.
    let slice = slice::Slice::default();
    let clock = || r.now();
    let ops = slice::Sliced {
        inner: &r,
        slice: &slice,
        clock: &clock,
    };
    let result = t.admit(&ops, LO, 20000 * SPARSE_GRAIN).unwrap();
    for _ in 0..100 {
        slice.renew();
        t.retry(&ops).unwrap();
    }
    assert!(t.blocks.len() <= p.per_space);
    let placed = r.sparse_tiles().len();
    if complete {
        assert_eq!(placed, 20000);
        assert_eq!(t.counters().current_unhonoured_bytes, 0);
    } else {
        assert!(!result);
        assert!(placed < 20000);
        assert!(t.stats.admit_decline > 0);
        assert!(t.counters().current_unhonoured_bytes > 0);
    }
}
#[test]
fn ens7_twenty_thousand_tiles_are_honoured_for_eight_gib() {
    ens7_twenty_thousand(8 << 30, true);
}
#[test]
fn ens7_twenty_thousand_tiles_are_bounded_for_small_framebuffer() {
    ens7_twenty_thousand(128 << 20, false);
}
#[test]
fn ens7_zero_framebuffer_still_has_a_finite_floor() {
    let p = policy::Budget::from_framebuffer(0);
    assert_eq!((p.per_space, p.per_vm), (8192, 8192));
}
#[test]
fn ens7_large_configured_framebuffer_still_obeys_the_absolute_cap() {
    let p = policy::Budget::from_framebuffer(32 << 30);
    assert_eq!((p.per_space, p.per_vm), (65536, 131072));
}
fn ens7_fragmented_deficit(offset: u64) {
    let (mut t, r) = (tree(100), Rm::new());
    t.mark(LO, PAGE, true);
    for i in 0..300 {
        t.admit(&r, LO + offset + i * 2 * PAGE, PAGE).unwrap();
    }
    let stats = t.counters();
    assert!(
        stats.missing_capped,
        "fragmented hostile input is explicit, never silently zero"
    );
    assert!(stats.current_unhonoured_bytes >= 300 * PAGE);
    assert!(t.missing_warning(0).unwrap().contains("capped=true"));
    t.retire(&r, LO + offset, PAGE, false).unwrap();
    assert!(
        t.counters().missing_capped,
        "partial retirement retains the upper bound"
    );
    t.clear(&r).unwrap_err();
    assert!(
        t.counters().missing_capped,
        "refused clear cannot hide uncertainty"
    );
    t.mark(LO, PAGE, false);
    t.clear(&r).unwrap();
    assert_eq!(t.counters().current_unhonoured_bytes, 0);
    assert!(!t.counters().missing_capped);
}
#[test]
fn ens7_fragmented_prefix_deficit_is_bounded_and_explicit() {
    ens7_fragmented_deficit(NORMAL_GRAIN);
}
#[test]
fn ens7_fragmented_suffix_deficit_survives_a_refused_clear() {
    ens7_fragmented_deficit(3 * NORMAL_GRAIN);
}
#[test]
fn ens7_overlapping_sparse_declines_count_the_union() {
    let (mut t, r) = (tree(100), Rm::new());
    t.mark(LO, PAGE, true);
    t.admit(&r, LO + NORMAL_GRAIN, 2 * SPARSE_GRAIN).unwrap();
    t.admit(&r, LO + NORMAL_GRAIN + SPARSE_GRAIN, 2 * SPARSE_GRAIN)
        .unwrap();
    assert_eq!(t.counters().current_unhonoured_bytes, 3 * SPARSE_GRAIN);
}
#[test]
fn ens7_retiring_the_middle_of_a_deficit_preserves_both_ends() {
    let (mut t, r) = (tree(100), Rm::new());
    t.mark(LO, PAGE, true);
    let start = LO + NORMAL_GRAIN;
    t.admit(&r, start, 3 * SPARSE_GRAIN).unwrap();
    t.retire(&r, start + SPARSE_GRAIN, SPARSE_GRAIN, false)
        .unwrap();
    assert_eq!(t.counters().current_unhonoured_bytes, 2 * SPARSE_GRAIN);
}
fn ens7_warning_ranges(offset: u64) {
    let (mut t, r) = (tree(100), Rm::new());
    t.mark(LO, PAGE, true);
    for i in 0..8 {
        t.admit(&r, LO + offset + i * NORMAL_GRAIN, 1 << 20)
            .unwrap();
    }
    let warning = t.missing_warning(0).unwrap();
    assert_eq!(
        warning.matches('(').count(),
        4,
        "first four ranges only: {warning}"
    );
    assert!(warning.len() <= 512);
}
#[test]
fn ens7_warning_keeps_only_four_aligned_missing_ranges() {
    ens7_warning_ranges(0);
}
#[test]
fn ens7_warning_keeps_only_four_unaligned_missing_ranges() {
    ens7_warning_ranges(PAGE);
}
fn ens7_session_limit(fb: u64) {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    let p = policy::Budget::from_framebuffer(fb);
    let live = AtomicUsize::new(p.per_vm - 1);
    assert!(initial::claim_block(&live, p.per_vm, true));
    assert!(!initial::claim_block(&live, p.per_vm, true));
    assert_eq!(live.load(Relaxed), p.per_vm);
    live.fetch_sub(1, Relaxed);
    assert!(initial::claim_block(&live, p.per_vm, true));
}
#[test]
fn ens7_small_vm_spaces_share_and_recover_the_policy_budget() {
    ens7_session_limit(0);
}
#[test]
fn ens7_large_vm_spaces_share_and_recover_the_policy_budget() {
    ens7_session_limit(8 << 30);
}

#[test]
fn ens7_session_gauge_recovery_preserves_other_spaces() {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    let counter = AtomicU64::new(10);
    adjust_deficit_gauge(&counter, 8, 3);
    assert_eq!(counter.load(Relaxed), 5);
}
#[test]
fn ens7_space_destruction_removes_only_its_own_deficit() {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    let counter = AtomicU64::new(100);
    adjust_deficit_gauge(&counter, 40, 0);
    assert_eq!(counter.load(Relaxed), 60);
}
fn ens7_whole_retirement_warning(offset: u64) {
    let mut d = deficit::Deficit::default();
    for i in 0..300 {
        d.mark(
            LO + offset + 2 * i * PAGE,
            LO + offset + (2 * i + 1) * PAGE,
            true,
            HI,
        );
    }
    assert!(d.capped);
    assert!(d.warning(0, HI).is_some());
    d.mark(crate::channel::TWIN_VA_FLOOR, HI, false, HI);
    assert!(!d.capped);
    assert_eq!(d.bytes, 0);
    d.mark(LO, LO + NORMAL_GRAIN, true, HI);
    assert!(
        d.warning(1, HI).is_none(),
        "whole retirement must preserve the cooldown"
    );
    assert!(d.warning(30_000_000_000, HI).is_some());
}
#[test]
fn ens7_retiring_capped_prefix_coverage_preserves_warning_rate_limit() {
    ens7_whole_retirement_warning(0);
}
#[test]
fn ens7_retiring_capped_suffix_coverage_preserves_warning_rate_limit() {
    ens7_whole_retirement_warning(NORMAL_GRAIN);
}
