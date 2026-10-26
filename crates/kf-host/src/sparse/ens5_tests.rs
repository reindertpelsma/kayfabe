// ENS5 failing-first regressions: saturated work is local, fallback is closed and counted.
use std::time::{Duration, Instant};

fn ens5_coarse(first: u64, bind: u64) {
    let (mut t, r) = (tree(BLOCK_LIMIT), Rm::new());
    t.route_map(&r, 99, first, PAGE, false).unwrap();
    r.backed(first, PAGE);
    r.guard.borrow_mut().insert(first, Some(PAGE));
    assert_eq!(t.admit(&r, bind, SPARSE_GRAIN), Ok(false));
    assert_eq!(
        t.stats.overflow, 1,
        "occupied geometry is a counted decline"
    );
    assert_eq!(t.stats.unhonoured_bytes, SPARSE_GRAIN);
    assert_eq!(
        r.calls.get(),
        0,
        "never release a populated coarse reservation"
    );
}
#[test]
fn ens5_occupied_coarse_first_page_is_counted_closed_fallback() {
    ens5_coarse(LO, LO + NORMAL_GRAIN);
}
#[test]
fn ens5_occupied_coarse_second_page_is_counted_closed_fallback() {
    ens5_coarse(LO + PAGE, LO + 2 * NORMAL_GRAIN);
}
#[test]
fn ens5_live_mapping_beyond_first_two_mib_prevents_coarse_release() {
    ens5_coarse(LO + 4 * NORMAL_GRAIN, LO + NORMAL_GRAIN);
}
#[test]
fn ens5_live_mapping_at_far_edge_prevents_coarse_release() {
    ens5_coarse(HI - PAGE, LO + 2 * NORMAL_GRAIN);
}

struct Ens5Ops;
impl Ops for Ens5Ops {
    fn reserve(&self, _: u64, _: u64, _: bool) -> Result<u32, RmError> {
        panic!("a saturated decline must issue no RM reservation verb")
    }
    fn release(&self, _: u32) -> Result<(), RmError> {
        panic!("a saturated decline must issue no RM release verb")
    }
}
fn ens5_saturated(n: usize) -> (Reservations, u64) {
    let mut rows: Vec<_> = (0..n - 1)
        .map(|i| GuestVaRange {
            handle: i as u32 + 1,
            lo: LO + i as u64 * NORMAL_GRAIN,
            hi: LO + (i as u64 + 1) * NORMAL_GRAIN,
        })
        .collect();
    let va = LO + n as u64 * NORMAL_GRAIN;
    rows.push(GuestVaRange {
        handle: n as u32,
        lo: va,
        hi: va + 1024 * NORMAL_GRAIN,
    });
    let mut tree = Reservations::new(&rows, n);
    // These slots represent live demand, not recyclable empty geometry.
    for i in 0..n - 1 {
        tree.mark(LO + i as u64 * NORMAL_GRAIN, PAGE, true);
    }
    (tree, va)
}
fn ens5_cost(n: usize, occupied: bool) -> Duration {
    let (mut t, va) = ens5_saturated(n);
    if occupied {
        t.mark(va, PAGE, true);
    }
    let start = Instant::now();
    let result = t.admit(&Ens5Ops, va + NORMAL_GRAIN, SPARSE_GRAIN);
    let elapsed = start.elapsed();
    println!(
        "ENS5 blocks={n} occupied={occupied} admit_ns={} result={result:?}",
        elapsed.as_nanos()
    );
    elapsed
}
#[test]
fn ens5_saturated_decline_is_below_five_ms() {
    assert!(ens5_cost(8191, false) < Duration::from_millis(5));
}
#[test]
fn ens5_saturated_occupied_failure_is_below_five_ms() {
    assert!(ens5_cost(8191, true) < Duration::from_millis(5));
}
#[test]
fn ens5_decline_cost_does_not_scale_with_unrelated_blocks() {
    let small = ens5_cost(2, false);
    let large = ens5_cost(8191, false);
    assert!(large < small * 64 + Duration::from_micros(100));
}
#[test]
fn ens5_occupied_cost_does_not_scale_with_unrelated_blocks() {
    let small = ens5_cost(2, true);
    let large = ens5_cost(8191, true);
    assert!(large < small * 64 + Duration::from_micros(100));
}
#[test]
fn ens5_four_thousand_maps_and_hundred_saturated_binds_finish_within_100ms() {
    let (mut t, va) = ens5_saturated(8191);
    t.mark(va, PAGE, true);
    let start = Instant::now();
    for i in 0..4000 {
        t.route_map(&Ens5Ops, 99, LO + i * NORMAL_GRAIN, PAGE, true)
            .unwrap();
    }
    for _ in 0..100 {
        let _ = t.admit(&Ens5Ops, va + NORMAL_GRAIN, SPARSE_GRAIN);
    }
    let elapsed = start.elapsed();
    println!("ENS5 saturated_apply_ns={}", elapsed.as_nanos());
    assert!(elapsed < Duration::from_millis(100));
}
#[test]
fn ens5_unchanged_occupied_admission_uses_negative_cache() {
    let (mut t, r) = (tree(100), Rm::new());
    t.mark(LO, PAGE, true);
    for _ in 0..8 {
        assert_eq!(t.admit(&r, LO + NORMAL_GRAIN, SPARSE_GRAIN), Ok(false));
    }
    assert_eq!(t.stats.negative_hits, 7);
    assert_eq!(r.calls.get(), 0);
}
#[test]
fn ens5_unrelated_block_change_preserves_negative_cache() {
    let (mut t, va) = ens5_saturated(8);
    t.mark(va, PAGE, true);
    assert_eq!(
        t.admit(&Ens5Ops, va + NORMAL_GRAIN, SPARSE_GRAIN),
        Ok(false)
    );
    t.mark(LO, PAGE, true);
    assert_eq!(
        t.admit(&Ens5Ops, va + NORMAL_GRAIN, SPARSE_GRAIN),
        Ok(false)
    );
    assert_eq!(t.stats.negative_hits, 1);
}
#[test]
fn ens5_emptying_its_block_invalidates_negative_cache() {
    let (mut t, r) = (tree(100), Rm::new());
    t.mark(LO, PAGE, true);
    assert_eq!(t.admit(&r, LO + NORMAL_GRAIN, SPARSE_GRAIN), Ok(false));
    t.mark(LO, PAGE, false);
    t.retry(&r).unwrap();
    assert_eq!(r.state(LO + NORMAL_GRAIN), Some(0));
    assert_eq!(t.stats.negative_hits, 0);
}
#[test]
fn ens5_unmap_at_far_edge_reenables_sparse_admission() {
    let (mut t, r) = (tree(100), Rm::new());
    t.mark(HI - PAGE, PAGE, true);
    assert_eq!(t.admit(&r, LO, SPARSE_GRAIN), Ok(false));
    t.mark(HI - PAGE, PAGE, false);
    t.retry(&r).unwrap();
    assert_eq!(r.state(LO), Some(0));
}
fn ens5_hard_error_cached(failure: usize) {
    let (mut t, r) = (tree(100), Rm::new());
    r.fail.set(failure);
    assert!(t.admit(&r, LO + NORMAL_GRAIN, SPARSE_GRAIN).is_err());
    let calls = r.calls.get();
    assert_eq!(t.admit(&r, LO + NORMAL_GRAIN, SPARSE_GRAIN), Ok(false));
    assert_eq!(r.calls.get(), calls);
    assert_eq!(t.stats.admit_hard_err, 1);
    assert_eq!(t.stats.negative_hits, 1);
}
#[test]
fn ens5_failed_admit_is_not_reissued_without_block_change() {
    ens5_hard_error_cached(1);
}
#[test]
fn ens5_failed_split_suffix_is_not_reissued_without_block_change() {
    ens5_hard_error_cached(2);
}
#[test]
fn ens5_negative_cache_is_bounded_under_unique_guest_requests() {
    let rows = [GuestVaRange {
        handle: 1,
        lo: LO,
        hi: LO + (1 << 30),
    }];
    let mut t = Reservations::new(&rows, 8192);
    t.mark(LO, PAGE, true);
    for i in 1..=264 {
        assert_eq!(
            t.admit(&Ens5Ops, LO + i * SPARSE_GRAIN, SPARSE_GRAIN),
            Ok(false)
        );
    }
    assert_eq!(t.cache.entries.len(), 256);
    assert_eq!(t.stats.negative_evicted, 8);
}
#[test]
fn ens5_partial_cancel_keeps_only_uncancelled_far_sparse_intent() {
    let (mut t, r) = (tree(100), Rm::new());
    let va = LO + 4 * NORMAL_GRAIN;
    t.mark(LO, PAGE, true);
    assert_eq!(t.admit(&r, va, 2 * SPARSE_GRAIN), Ok(false));
    t.retire(&r, va, SPARSE_GRAIN, false).unwrap();
    t.mark(LO, PAGE, false);
    t.retry(&r).unwrap();
    assert_eq!(r.state(va), None);
    assert_eq!(r.state(va + SPARSE_GRAIN), Some(0));
}
#[test]
fn ens5_full_cancel_never_retries_a_retired_negative_intent() {
    let (mut t, r) = (tree(100), Rm::new());
    t.mark(LO, PAGE, true);
    assert_eq!(t.admit(&r, LO + NORMAL_GRAIN, SPARSE_GRAIN), Ok(false));
    t.retire(&r, LO + NORMAL_GRAIN, SPARSE_GRAIN, false)
        .unwrap();
    t.mark(LO, PAGE, false);
    t.retry(&r).unwrap();
    assert_eq!(r.state(LO + NORMAL_GRAIN), None);
}
#[test]
fn ens5_occupancy_cap_pins_the_block_until_a_whole_block_unmap() {
    let (mut t, r) = (tree(100), Rm::new());
    for i in 0..513 {
        t.mark(LO + i * 2 * PAGE, PAGE, true);
    }
    assert_eq!(t.stats.occupancy_capped, 1);
    for i in 0..513 {
        t.mark(LO + i * 2 * PAGE, PAGE, false);
    }
    assert_eq!(t.admit(&r, LO + 4 * NORMAL_GRAIN, SPARSE_GRAIN), Ok(false));
    t.mark(LO, HI - LO, false);
    assert_eq!(t.admit(&r, LO + 4 * NORMAL_GRAIN, SPARSE_GRAIN), Ok(true));
}
#[test]
fn ens5_receipt_counters_include_cached_declines_and_hard_errors() {
    let (mut t, r) = (tree(100), Rm::new());
    t.mark(LO, PAGE, true);
    for _ in 0..3 {
        t.admit(&r, LO, SPARSE_GRAIN).unwrap();
    }
    assert_eq!(t.stats.admit_calls, 3);
    assert_eq!(t.stats.admit_decline, 3);
    assert_eq!(t.stats.admit_hard_err, 0);
    assert!(t.stats.admit_ns_total >= t.stats.admit_ns_max);
}
