// ── Extra adversary tests (breaker, 2026-10-26) ────────────────────────────────────────────────

/// I4: `retry` runs at the start of EVERY walk (`begin_refresh`, kf-qemu mem.rs:1691) whether or
/// not any intent is held; it visits every block x 32 tiles x 16 pages. Its cost must not scale
/// with the tree when nothing is held.
#[test]
fn breaker_i4_retry_cost_does_not_scale_with_the_tree() {
    let small = cost::retry_us(800, 50).max(0.5);
    let large = cost::retry_us(8000, 20);
    assert!(
        large / small < 3.0,
        "10x the blocks costs {:.1}x ({small:.1} us -> {large:.1} us) on every walk",
        large / small
    );
}

/// I4: `retire` of a range scans every block below the range end (`range(..end).filter(va <
/// b.hi)`), not only the blocks that overlap it.
#[test]
fn breaker_i4_retire_cost_does_not_scale_with_the_tree() {
    let small = cost::retire_us(800, 200).max(0.2);
    let large = cost::retire_us(8000, 200);
    assert!(
        large / small < 3.0,
        "10x the blocks costs {:.1}x ({small:.1} us -> {large:.1} us)",
        large / small
    );
}

/// I2: a page that was a plain mapping and becomes sparse in the guest (unbind) is admitted while
/// still live (HELD), then unmapped by the same entry. Nothing makes it sparse until the NEXT
/// walk's `retry` (`apply_entry` runs retire, admit, unmaps, maps, invalidate; `begin_refresh`
/// is only at the entry start). Between the two the page is PLAIN INVALID although the guest
/// declared it sparse: a GPU write into it faults (Xid 31), the H6 fault again.
/// Minimal sequence: map 64 KiB; admit sparse over it (held); unmap it. No walk follows.
#[test]
#[ignore = "Bookkeeping-only mark has no RM Ops; actual apply now retries before invalidate, \
    covered by ens5 vidmem/dma same-apply tests. Changed VAs may fault during \
    refresh."]
fn breaker_i2_backed_to_sparse_has_a_plain_invalid_window_until_the_next_walk() {
    let (mut t, r) = wide(100);
    let va = LO + 5 * NORMAL_GRAIN;
    t.route_map(&r, RANGE, va, SPARSE_GRAIN, true)
        .expect("plain map");
    r.backed(va, SPARSE_GRAIN);
    assert_eq!(t.admit(&r, va, SPARSE_GRAIN), Ok(false), "held while live");
    r.unmap(va, SPARSE_GRAIN);
    t.mark(va, SPARSE_GRAIN, false);
    // the entry ends here: the apply calls no retry after its unmaps
    assert_eq!(
        r.state(va),
        Some(0),
        "the guest's sparse page is plain invalid after the entry that unmapped it"
    );
}
