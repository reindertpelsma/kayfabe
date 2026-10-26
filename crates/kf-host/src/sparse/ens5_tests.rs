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
    (Reservations::new(&rows, n), va)
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
