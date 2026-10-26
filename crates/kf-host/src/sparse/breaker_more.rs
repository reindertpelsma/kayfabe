
// ── More adversary tests (breaker, 2026-10-26). EXPECTED RED unless a comment says otherwise ───

/// I3 / documented fallback: "a sparse run that lands in a POPULATED normal block stays INVALID,
/// is counted and is not an error". A coarse block populated in its FIRST 2 MiB (a plain map that
/// could not be refined, after a transient RM refusal) meets `replace()`'s occupancy check before
/// any budget check: the run is a hard `Err(NoMemory)` with every counter still zero, which the
/// apply layer ACKs FAILED so the walker re-emits it on every walk (sparse.rs:183-185, :293-339).
/// Minimal sequence: reserve#1 refused; map A (LO+PAGE); admit a sparse tile at LO+10 MiB.
#[test]
fn breaker_i3_sparse_run_in_a_populated_coarse_block_is_a_counted_fallback_not_an_error() {
    let (mut t, r) = wide(100);
    r.fail.set(1);
    t.route_map(&r, RANGE, LO + PAGE, PAGE, true).expect("plain map");
    r.backed(LO + PAGE, PAGE);
    assert_eq!(t.stats.coarse_maps, 1, "precondition: the map is in the coarse block");
    let got = t.admit(&r, LO + 10 * NORMAL_GRAIN, SPARSE_GRAIN);
    assert_eq!(got, Ok(false), "closed fallback expected, got {got:?}");
    assert!(
        t.stats.unhonoured_bytes + t.stats.overflow > 0,
        "the fallback must be counted"
    );
}

/// I3: the same hard error must not repeat identically on the next walk: the second identical
/// admission is expected to be a counted HELD, never a fresh refusal.
#[test]
fn breaker_i3_a_refused_sparse_run_is_not_refused_again_by_the_next_walk() {
    let (mut t, r) = wide(100);
    r.fail.set(1);
    t.route_map(&r, RANGE, LO + PAGE, PAGE, true).expect("plain map");
    r.backed(LO + PAGE, PAGE);
    let va = LO + 10 * NORMAL_GRAIN;
    let first = t.admit(&r, va, SPARSE_GRAIN);
    let second = t.admit(&r, va, SPARSE_GRAIN);
    assert!(
        first.is_ok() && second.is_ok(),
        "walk 1 {first:?}, walk 2 {second:?}: an ACKed-FAILED run is re-emitted every walk"
    );
}

/// I7: the block budget must track LIVE geometry. Mapping then unmapping a page in many distinct
/// tiles leaves the space EMPTY, yet the tree never merges, so the budget is spent forever and
/// every later map is a counted coarse map and every sparse run a counted overflow
/// (sparse.rs:173-251: `replace` only splits).
/// Minimal sequence (limit 12): map+unmap a page in 8 distinct tiles; map in a 9th.
#[test]
fn breaker_i7_budget_recovers_when_every_mapping_is_gone() {
    let (mut t, r) = wide(12);
    for i in 0..8u64 {
        let va = LO + (4 + 3 * i) * NORMAL_GRAIN;
        t.route_map(&r, RANGE, va, PAGE, true).expect("plain map");
        r.backed(va, PAGE);
        r.unmap(va, PAGE);
        t.mark(va, PAGE, false);
    }
    let coarse = t.stats.coarse_maps;
    let va = LO + 60 * NORMAL_GRAIN;
    t.route_map(&r, RANGE, va, PAGE, true).expect("plain map");
    assert_eq!(
        t.stats.coarse_maps, coarse,
        "an empty space (no live map) still cannot refine: {} blocks of budget 12",
        t.blocks.len()
    );
}

/// I7: the same for sparse. With nothing mapped and nothing wanted, a sparse run must still be
/// honoured after churn through many other tiles.
#[test]
fn breaker_i7_sparse_is_honoured_in_an_empty_space_after_churn() {
    let (mut t, r) = wide(12);
    for i in 0..8u64 {
        let va = LO + (4 + 3 * i) * NORMAL_GRAIN;
        t.route_map(&r, RANGE, va, PAGE, true).expect("plain map");
        r.backed(va, PAGE);
        r.unmap(va, PAGE);
        t.mark(va, PAGE, false);
    }
    assert_eq!(t.admit(&r, LO + 61 * NORMAL_GRAIN, SPARSE_GRAIN), Ok(true));
}

/// I3 (retire side): retiring a sparse run must not fail because the budget is spent. A retire
/// that releases a sparse tile replaces it by a NORMAL block: 1 -> 1, so it needs no budget; this
/// pins that it still works at the cap and leaves no sparse state.
/// (Expected GREEN: pins current behaviour.)
#[test]
fn breaker_i3_retire_at_the_cap_still_withdraws_the_sparse_tile() {
    let (mut t, r) = wide(100);
    assert_eq!(t.admit(&r, LO, SPARSE_GRAIN), Ok(true));
    t.limit = t.blocks.len();
    t.retire(&r, LO, SPARSE_GRAIN, false).expect("retire");
    assert_eq!(r.state(LO), None);
}

// ── Edges of the VA space ──

const TOP: u64 = 1 << 47;

fn top_tree(limit: usize) -> (Reservations, cost::NullRm) {
    (
        Reservations::new(
            &[GuestVaRange {
                handle: 1,
                lo: 1 << 40,
                hi: TOP,
            }],
            limit,
        ),
        cost::NullRm::default(),
    )
}

/// Plain maps on the very last page below 2^47 and rows ending exactly at the top are never
/// refused and are routed to a block that contains them. (GREEN expected; pins the edge.)
#[test]
fn breaker_edge_plain_map_on_the_last_page_of_the_range() {
    let (mut t, rm) = top_tree(100);
    for (va, len) in [(TOP - PAGE, PAGE), (TOP - 2 * NORMAL_GRAIN, 2 * NORMAL_GRAIN)] {
        let p = t.route_map(&rm, 9, va, len, true).expect("plain map at the top");
        assert_eq!(p.iter().map(|x| x.2).sum::<u64>(), len);
        assert_eq!(p.last().map(|x| x.1 + x.2), Some(va + len));
    }
}

/// A plain row that runs PAST the top of the last reservation (a guest VA above 2^47 is outside
/// every host reservation): it must still be routed (through the range object), exactly as with
/// the twin OFF, and a later sparse run in the same tile must not free it.
#[test]
fn breaker_edge_row_crossing_the_top_of_the_last_reservation_is_routed() {
    let (mut t, rm) = top_tree(100);
    let va = TOP - 2 * PAGE;
    let p = t.route_map(&rm, 9, va, 4 * PAGE, true).expect("row across the top");
    assert_eq!(p.iter().map(|x| x.2).sum::<u64>(), 4 * PAGE);
    assert!(t.pieces(9, va, 4 * PAGE).is_ok(), "its unmap routes too");
}

/// Sparse on the last 64 KiB tile below the top is honoured exactly, and a sparse run above the
/// top (outside every reservation) is never turned into a reservation there.
#[test]
fn breaker_edge_sparse_at_the_top() {
    let (mut t, rm) = top_tree(100);
    assert_eq!(t.admit(&rm, TOP - SPARSE_GRAIN, SPARSE_GRAIN), Ok(true));
    let above = t.admit(&rm, TOP, SPARSE_GRAIN);
    assert!(
        above != Ok(true),
        "a sparse reservation must not be created above the guest range: {above:?}"
    );
}

/// A row that starts below the 64 KiB floor of a floor-started space and one whose end wraps are
/// refused by name in BOTH twin states; nothing else is refused. (GREEN: pins current behaviour.)
#[test]
fn breaker_edge_arguments_refused_by_the_twin_off_path_only() {
    let (mut t, rm) = top_tree(100);
    assert!(t.route_map(&rm, 9, 0, PAGE, true).is_err());
    assert!(t.route_map(&rm, 9, u64::MAX - PAGE + 1, 2 * PAGE, true).is_err());
    assert!(t.route_map(&rm, 9, (1 << 40) + 1, PAGE, true).is_err(), "unaligned");
}
