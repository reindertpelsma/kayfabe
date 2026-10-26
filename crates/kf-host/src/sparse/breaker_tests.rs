
// ── ADVERSARY (breaker) tests, 2026-10-26. Every test here is EXPECTED RED against 3e52e0161. ──
//
// Occupancy accounting. `Block::live` is a 512-bit mask: it can only describe the FIRST 2 MiB of
// a block (`Reservations::bits`). A plain map that cannot be refined lands in a COARSE block
// (budget, transient RM refusal): its pages beyond `block.lo + 2 MiB` are never marked live, so
// the tree believes the coarse block is empty and a later refinement / sparse admission /
// recycle FREES it. Freeing a host reservation unmaps everything in it (V3_SPARSE_TWIN.md, host
// fact 4), so live guest-valid VAs vanish from the host mirror (invariant I1/I2).

/// I1: a plain map that lands in a coarse block is live wherever it is inside that block.
#[test]
fn breaker_i1_coarse_map_beyond_two_mib_is_marked_live() {
    let (mut t, r) = wide(2); // budget 2: no 3-way split of the coarse block fits
    let va = LO + 40 * NORMAL_GRAIN;
    t.route_map(&r, RANGE, va, PAGE, true).expect("plain map");
    r.backed(va, PAGE);
    let b = block(&t, va);
    assert!(
        b.hi - b.lo > NORMAL_GRAIN,
        "precondition: the map went into the coarse block"
    );
    assert!(
        Reservations::occupied(b),
        "a live plain map {:#x} (block offset {} MiB) is invisible to the occupancy mask",
        va,
        (va - b.lo) >> 20
    );
}

/// I1: after a transient RM refusal (restore path) left a plain map in the coarse block, the
/// next map in ANOTHER tile of that block must not free the block under the live page.
/// Minimal sequence: reserve#1 refused; map A (LO+80M); map B (LO+120M).
#[test]
fn breaker_i1_refinement_after_transient_refusal_frees_live_coarse_block() {
    let (mut t, r) = wide(100);
    let a = LO + 40 * NORMAL_GRAIN;
    r.fail.set(1); // exactly one refused reserve: the split of A's tile
    t.route_map(&r, RANGE, a, PAGE, true).expect("plain map A");
    r.backed(a, PAGE);
    r.guard.borrow_mut().insert(a, Some(PAGE));
    // the RM is healthy again; B is in the same coarse block
    let b = LO + 60 * NORMAL_GRAIN;
    // Rm::release asserts "a populated block must never be freed"
    t.route_map(&r, RANGE, b, PAGE, true).expect("plain map B");
    r.backed(b, PAGE);
    assert_eq!(r.state(a), Some(PAGE), "map A survived the refinement for B");
}

/// I1/I7: with the block count one below the cap, a map in the MIDDLE of a coarse block is
/// declined (needs +2) but a map at the block EDGE is not (needs +1): the edge map then frees
/// the coarse block that holds the declined map's live page.
/// Minimal sequence (limit 4): map T1 (3 blocks), map T2 in the right coarse block (declined,
/// coarse), map the first tile of that coarse block (admitted, frees it).
#[test]
fn breaker_i1_edge_tile_refinement_at_the_cap_frees_populated_coarse_sibling() {
    let (mut t, r) = wide(4);
    let t1 = LO + 10 * NORMAL_GRAIN;
    t.route_map(&r, RANGE, t1, PAGE, true).expect("T1");
    r.backed(t1, PAGE);
    assert_eq!(t.blocks.len(), 3);
    let t2 = LO + 30 * NORMAL_GRAIN;
    t.route_map(&r, RANGE, t2, PAGE, true).expect("T2");
    r.backed(t2, PAGE);
    r.guard.borrow_mut().insert(t2, Some(PAGE));
    assert_eq!(t.stats.coarse_maps, 1, "precondition: T2 was declined");
    let edge = LO + 11 * NORMAL_GRAIN; // first tile of the right coarse block
    t.route_map(&r, RANGE, edge, PAGE, true).expect("edge");
    r.backed(edge, PAGE);
    assert_eq!(r.state(t2), Some(PAGE));
}

/// I2: a sparse admission over a tile of a coarse block that holds a live (unmarked) plain
/// page frees the block and the page goes with it.
#[test]
fn breaker_i2_sparse_admission_frees_a_coarse_block_with_a_live_page() {
    let (mut t, r) = wide(2);
    let live = LO + 40 * NORMAL_GRAIN;
    t.route_map(&r, RANGE, live, PAGE, true).expect("plain map");
    r.backed(live, PAGE);
    r.guard.borrow_mut().insert(live, Some(PAGE));
    t.limit = 100; // the budget has room again (it can: another admission path, a bigger cap)
    // a sparse run in the SAME coarse block, far from the live page
    let _ = t.admit(&r, LO + 50 * NORMAL_GRAIN, SPARSE_GRAIN);
    assert_eq!(r.state(live), Some(PAGE));
}

/// I1: recycling a space must refuse while any plain map is live. A live page of a coarse
/// block beyond 2 MiB is not counted, so `clear` succeeds and the space is reused under it.
#[test]
fn breaker_i1_clear_accepts_a_space_with_a_live_coarse_page() {
    let (mut t, r) = wide(2);
    let va = LO + 40 * NORMAL_GRAIN;
    t.route_map(&r, RANGE, va, PAGE, true).expect("plain map");
    r.backed(va, PAGE);
    assert!(t.clear(&r).is_err(), "a space with a live mapping is not recyclable");
}
