//! ★ DIAGNOSTIC ONLY — `KF3_DIAG_GATELOG` (`kf_mem::gatelog`): the walk-gate log, GPU-free.
//!
//! Written for hypothesis H7 of the enscape-07/08 fault (run 5227): *a copy submitted while a tile
//! was mapped is held behind a busy walk, the UNMAP applies first, the held entry is published
//! afterwards and faults.* What these pin:
//! - the ring is fixed-size with a COUNTED overflow, whatever the guest does;
//! - what "seen", "held" and "published" mean, in numbers (FIFO ages, hold times, overlap class);
//! - "held across unmap" is evaluated at the START of the apply, only for the channels of the
//!   unmapped space, and a zero is a zero (the falsifier shape of H7 must read as zero);
//! - the dump: the faulting channel by the Xid's chid, its last 64 records, the unmaps that cover
//!   the fault VA, per-space counts, hold totals; every line prefixed and capped, a fixed number of
//!   lines, a fixed number of dumps, the VAHIST clock header.
//!
//! **Mutants this file is meant to kill (each by at least two tests; checked by hand against
//! `crates/kf-mem/src/gatelog.rs`):**
//! - M1 ring overwrites the newest / forgets the overflow count / returns the wrong order:
//!   `ring_wraps_oldest_first_and_counts_overflow`, `ring_never_grows_past_cap_under_flood`,
//!   `ring_newest_first_matches_last_n`, `dump_shows_only_the_last_64_records_newest_last`.
//! - M2 `put_advance` wraps wrongly or accepts a value outside the ring:
//!   `put_advance_table`, `put_seen_wraps_and_ignores_out_of_range`, `reconcile_*`.
//! - M3 pending queue not FIFO / wrong age / underflow:
//!   `publish_is_fifo_with_ages`, `second_chunk_keeps_its_own_age`,
//!   `publish_with_nothing_announced_is_safe`, `held_now_reports_oldest_age`.
//! - M4 hold state machine (double count, no accumulation, no close):
//!   `gate_opens_one_hold_and_counts_regates`, `pass_closes_the_hold_and_accumulates`,
//!   `handle_flow_closes_the_hold_on_publish`, `held_now_reports_the_hold`.
//! - M5 overlap classification (order of the tests, adjacency):
//!   `classify_overlap_table`, `ranges_overlap_table`, `unrelated_holds_are_accounted_apart`,
//!   `handle_pass_classifies_by_the_walks_applied`.
//! - M6 unmap evaluation (wrong space, counts zero as held, evaluated at the end, map-only diff):
//!   `unmap_counts_only_the_channels_of_that_space`, `zero_held_unmap_reads_as_zero`,
//!   `a_gated_channel_with_nothing_read_counts_as_held`, `a_map_only_diff_evaluates_nothing`,
//!   `unmap_is_evaluated_at_the_start_of_the_apply`.
//! - M7 bounds (unbounded channel table, run list, channel list, rings):
//!   `chan_table_is_bounded_and_evicts_freed_first`, `unmap_run_and_chan_lists_are_capped`,
//!   `walk_and_unmap_rings_are_bounded`, `dump_is_bounded_and_every_line_capped`.
//! - M8 dump (wrong channel match, wrong clock, no cap on the number of dumps, no per-space count):
//!   `dump_matches_the_xid_chid_on_the_low_16_bits`, `dump_header_carries_the_vahist_clock`,
//!   `dumps_per_process_are_capped`, `dump_lists_the_unmaps_that_cover_the_fault_va`,
//!   `dump_counts_held_across_unmap_per_space`, `dump_reports_hold_totals_and_the_top_channels`.
//! - M9 the flag (any value turns it on): `flag_is_exactly_one`, `the_flag_is_off_by_default`.

use kf_mem::apply::DiffRun;
use kf_mem::gatelog::{
    CHANGED_MAX, ChanLog, Changes, DUMP_LAST, DUMP_LINES_MAX, DUMPS_MAX, Gate, Kind, LINE_MAX,
    MAX_CHANS, Overlap, PEND_CHUNKS, Point, RING_N, Recorder, Ring, UNMAP_CHANS, UNMAP_RUNS,
    UNMAPS_N, WALKS_N, WHY_SPLIT_QUEUED, WHY_VA_BUSY, bound_lines, classify_overlap, flag_on,
    put_advance, ranges_overlap,
};
use std::time::Instant;

// the caps these tests lean on: 20 channels overflow the listed ones; the dump shows 64 records
const _: () = assert!(UNMAP_CHANS < 20 && CHANGED_MAX >= 1 && DUMP_LAST == 64);

const SP: u64 = 0xc1d0_027e_ff00_0850;
const SP2: u64 = 0xc1d0_0300_ff00_0001;
const T0_UNIX_MS: u128 = 1_792_876_069_894;

fn run(unmap: bool, va: u64, len: u64) -> DiffRun {
    DiffRun {
        unmap,
        va,
        len,
        at: 0x1000_0000_u64.wrapping_add(va),
        ap: 0,
        held: false,
        kind: 0,
        perm: kf_host::MapPerm::default(),
        privileged: false,
        leaf: 0x1000,
    }
}

fn recorder() -> &'static Recorder {
    Box::leak(Box::new(Recorder::with_clock(Instant::now(), T0_UNIX_MS)))
}

fn born(r: &Recorder, token: u32, vas: u64) {
    assert!(r.chan_birth(0, token, 0xc1d0_0001, token, vas, 256));
}

fn chan(r: &Recorder, token: u32) -> kf_mem::gatelog::ChanRef {
    r.chan_ref(token).expect("tracked")
}

fn gate(point: Point) -> Gate {
    Gate {
        reason: WHY_VA_BUSY,
        point,
        space: 0,
        walk: 0,
    }
}

fn none() -> Changes {
    Changes::default()
}

fn touched(walks: u64) -> Changes {
    Changes {
        walks,
        space_touched: true,
        extents: vec![(0x1000, 0x1000)],
    }
}

fn kinds(l: &ChanLog) -> Vec<Kind> {
    l.last(RING_N).iter().map(|r| r.kind).collect()
}

// ---- the ring (M1) ---------------------------------------------------------------------------

#[test]
fn ring_wraps_oldest_first_and_counts_overflow() {
    let mut r = Ring::new(4);
    assert!(r.is_empty());
    for v in 1..=6u32 {
        r.push(v);
    }
    assert_eq!(r.len(), 4);
    assert_eq!(r.overflow(), 2);
    assert_eq!(r.last_n(4), vec![3, 4, 5, 6]);
    assert_eq!(r.last_n(2), vec![5, 6]);
    assert_eq!(r.last_n(100), vec![3, 4, 5, 6]);
    assert!(r.last_n(0).is_empty());
}

#[test]
fn ring_never_grows_past_cap_under_flood() {
    let mut r = Ring::new(8);
    for v in 0..100_000u32 {
        r.push(v);
    }
    assert_eq!(r.len(), 8);
    assert_eq!(r.overflow(), 100_000 - 8);
    assert_eq!(r.last_n(8), (99_992..100_000).collect::<Vec<_>>());
    let z: Ring<u8> = Ring::new(0);
    assert_eq!(z.len(), 0, "a zero capacity is one record, not a panic");
    let mut z = z;
    z.push(1);
    z.push(2);
    assert_eq!(z.last_n(5), vec![2]);
}

#[test]
fn ring_newest_first_matches_last_n() {
    let mut r = Ring::new(5);
    for v in 0..3u32 {
        r.push(v);
    }
    assert_eq!(r.newest_first().copied().collect::<Vec<_>>(), vec![2, 1, 0]);
    for v in 3..9u32 {
        r.push(v);
    }
    let mut a: Vec<u32> = r.newest_first().copied().collect();
    assert_eq!(a, vec![8, 7, 6, 5, 4]);
    a.reverse();
    assert_eq!(a, r.last_n(5));
}

// ---- GP_PUT arithmetic (M2) ------------------------------------------------------------------

#[test]
fn put_advance_table() {
    assert_eq!(put_advance(0, 5, 256), 5);
    assert_eq!(
        put_advance(250, 3, 256),
        9,
        "wraps through the end of the ring"
    );
    assert_eq!(put_advance(255, 0, 256), 1);
    assert_eq!(
        put_advance(5, 4, 256),
        255,
        "one slot behind is almost a full lap"
    );
    assert_eq!(put_advance(7, 7, 256), 0, "no change, no entry");
    assert_eq!(
        put_advance(0, 256, 256),
        0,
        "a PUT outside the ring adds nothing"
    );
    assert_eq!(put_advance(256, 0, 256), 0);
    assert_eq!(put_advance(0, 5, 0), 0, "a ring of no slots");
    assert_eq!(put_advance(u32::MAX, u32::MAX, u32::MAX), 0);
}

#[test]
fn put_seen_wraps_and_ignores_out_of_range() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    assert_eq!(l.put_seen(10, 250), 250);
    assert_eq!(l.put_seen(20, 3), 9, "wrap");
    assert_eq!(l.held_now(30).count, 259);
    // a hostile PUT: nothing announced, nothing recorded, state unchanged
    let before = l.last(RING_N).len();
    assert_eq!(l.put_seen(40, 256), 0);
    assert_eq!(l.put_seen(40, u32::MAX), 0);
    assert_eq!(l.last(RING_N).len(), before);
    assert_eq!(l.held_now(50).count, 259);
    assert_eq!(
        l.put_seen(60, 3),
        0,
        "the same value again announces nothing"
    );
}

#[test]
fn put_seen_records_slot_range_and_time() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 5);
    l.put_seen(1234, 3);
    let r = l.last(1)[0];
    assert_eq!(r.kind, Kind::PutAdvance);
    assert_eq!((r.t_ns, r.a, r.b, r.c), (1234, 3, 0, 3));
    l.put_seen(2000, 4);
    let r = l.last(1)[0];
    assert_eq!((r.t_ns, r.a, r.b, r.c), (2000, 4, 3, 1));
    assert_eq!(l.stats().announced, 4);
}

// ---- the pending queue (M3) ------------------------------------------------------------------

#[test]
fn publish_is_fifo_with_ages() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.put_seen(100, 3); // slots 0,1,2 announced at 100
    assert_eq!(l.published(150), 50);
    assert_eq!(l.published(160), 60);
    let ps: Vec<(u32, u64)> = l
        .last(RING_N)
        .iter()
        .filter(|r| r.kind == Kind::Published)
        .map(|r| (r.a, r.b))
        .collect();
    assert_eq!(
        ps,
        vec![(0, 50), (1, 60)],
        "slot order, age since announced"
    );
    let h = l.held_now(200);
    assert_eq!(h.count, 1);
    assert_eq!(h.oldest_age_ns, 100);
    assert_eq!(l.stats().published, 2);
}

#[test]
fn second_chunk_keeps_its_own_age() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.put_seen(100, 2);
    l.put_seen(300, 7);
    assert_eq!(l.held_now(400).count, 7);
    assert_eq!(l.published(400), 300);
    assert_eq!(l.published(400), 300);
    assert_eq!(l.published(400), 100, "the second chunk is 300 -> 400 old");
    assert_eq!(l.held_now(500).oldest_age_ns, 200);
    assert_eq!(l.held_now(500).count, 4);
}

#[test]
fn publish_with_nothing_announced_is_safe() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    assert_eq!(l.published(10), 0);
    assert_eq!(l.held_now(20), kf_mem::gatelog::Held::default());
    let r = l.last(1)[0];
    assert_eq!((r.kind, r.a), (Kind::Published, u32::MAX), "marked unseen");
    assert_eq!(l.stats().published, 1);
}

#[test]
fn held_now_reports_oldest_age() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    assert!(!l.held_now(999).any());
    l.put_seen(1_000, 1);
    let h = l.held_now(6_000);
    assert_eq!((h.count, h.oldest_age_ns, h.in_hold), (1, 5_000, false));
    assert!(h.any());
}

#[test]
fn pending_chunks_are_bounded() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 1 << 20, 0);
    for i in 1..=(PEND_CHUNKS as u32 + 500) {
        l.put_seen(u64::from(i), i);
    }
    // all entries are still counted, the queue merged the overflow into its newest chunk
    assert_eq!(l.held_now(1 << 40).count, PEND_CHUNKS as u64 + 500);
    // drain: exactly that many pops, then nothing
    for _ in 0..(PEND_CHUNKS as u64 + 500) {
        l.published(10_000_000);
    }
    assert_eq!(l.held_now(1 << 40).count, 0);
}

#[test]
fn reconcile_drops_entries_the_relay_already_consumed() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.put_seen(10, 4);
    assert_eq!(
        l.reconcile(20, 4),
        4,
        "cursor reached PUT: nothing is pending"
    );
    assert_eq!(l.held_now(30).count, 0);
    l.put_seen(40, 10); // 6 more: slots 4..10
    assert_eq!(l.reconcile(50, 8), 4, "cursor at 8: slots 8,9 remain");
    assert_eq!(l.held_now(60).count, 2);
    assert_eq!(l.reconcile(70, 8), 0, "idempotent");
    assert_eq!(
        l.reconcile(80, 256),
        0,
        "a cursor outside the ring changes nothing"
    );
    assert_eq!(l.held_now(90).count, 2);
    // inferred removals are marked 2 and are not counted as published
    assert_eq!(l.stats().published, 0);
    assert!(
        l.last(RING_N)
            .iter()
            .any(|r| r.kind == Kind::Published && r.c == 2)
    );
}

#[test]
fn reconcile_keeps_what_is_really_pending() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.put_seen(10, 5);
    assert_eq!(l.reconcile(20, 0), 0, "cursor 0, PUT 5: all five pending");
    assert_eq!(l.held_now(30).count, 5);
    l.published(40);
    assert_eq!(
        l.reconcile(50, 1),
        0,
        "the publish hook and the cursor agree"
    );
    assert_eq!(l.held_now(60).count, 4);
}

#[test]
fn entry_seen_records_slot_entry_and_pending() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.put_seen(10, 2);
    l.entry_seen(11, 0, 0xdead_beef_0000_1234);
    let r = l.last(1)[0];
    assert_eq!(r.kind, Kind::Seen);
    assert_eq!((r.t_ns, r.a, r.b, r.c), (11, 0, 0xdead_beef_0000_1234, 2));
    assert_eq!(l.stats().seen, 1);
}

// ---- the gate and its hold (M4, M5) ----------------------------------------------------------

#[test]
fn gate_opens_one_hold_and_counts_regates() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    assert!(l.gate(100, gate(Point::Put)));
    assert!(!l.gate(200, gate(Point::Put)));
    assert!(!l.gate(300, gate(Point::Publish)));
    assert!(l.in_hold());
    let s = l.stats();
    assert_eq!((s.holds, s.regates), (1, 2));
    assert_eq!(
        kinds(&l).iter().filter(|k| **k == Kind::GateStart).count(),
        1,
        "one record per hold, not per gated turn"
    );
    assert_eq!(
        l.hold_gate().map(|g| g.point),
        Some(Point::Put),
        "the first gate stays"
    );
}

#[test]
fn gate_start_record_carries_reason_point_space_and_walk() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.gate(
        77,
        Gate {
            reason: WHY_VA_BUSY | WHY_SPLIT_QUEUED,
            point: Point::Publish,
            space: SP,
            walk: 306_599,
        },
    );
    let r = l.last(1)[0];
    assert_eq!(r.kind, Kind::GateStart);
    assert_eq!(r.t_ns, 77);
    assert_eq!(r.a & 0xff, u32::from(WHY_VA_BUSY | WHY_SPLIT_QUEUED));
    assert_eq!((r.a >> 8) & 1, 1, "publish point");
    assert_eq!((r.b, r.c), (SP, 306_599));
    let mut l2 = ChanLog::new(0x6c, 1, 3, SP, 256, 0);
    l2.gate(1, gate(Point::Put));
    assert_eq!((l2.last(1)[0].a >> 8) & 1, 0, "put point");
}

#[test]
fn pass_closes_the_hold_and_accumulates() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    assert_eq!(
        l.pass(50, 1, &none(), None),
        None,
        "no hold: nothing to close"
    );
    assert_eq!(l.stats().holds, 0);
    l.gate(1_000, gate(Point::Put));
    assert_eq!(l.pass(6_000, 9, &touched(1), None), Some(5_000));
    assert!(!l.in_hold());
    l.gate(10_000, gate(Point::Put));
    assert_eq!(l.pass(13_000, 10, &touched(1), None), Some(3_000));
    let s = l.stats();
    assert_eq!((s.holds, s.hold_ns, s.hold_ns_max), (2, 8_000, 5_000));
    let ends: Vec<_> = l
        .last(RING_N)
        .into_iter()
        .filter(|r| r.kind == Kind::GateEnd)
        .collect();
    assert_eq!(ends.len(), 2);
    assert_eq!((ends[0].t_ns, ends[0].b, ends[0].c), (6_000, 5_000, 9));
    assert_eq!(Overlap::from_code(ends[0].a), Overlap::SpaceTouched);
}

#[test]
fn held_now_reports_the_hold() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.gate(1_000, gate(Point::Put));
    let h = l.held_now(4_500);
    assert!(h.in_hold && h.any());
    assert_eq!((h.count, h.hold_age_ns), (0, 3_500));
    l.pass(5_000, 1, &none(), None);
    let h = l.held_now(6_000);
    assert!(!h.in_hold && !h.any());
    assert_eq!(h.hold_age_ns, 0);
}

#[test]
fn a_gate_marks_the_announced_entries_held() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.put_seen(10, 1); // announced before the gate: marked when it opens
    l.gate(20, gate(Point::Put));
    l.put_seen(30, 2); // announced inside the hold
    l.pass(40, 1, &none(), None);
    l.put_seen(50, 3); // announced after: free
    for t in [60, 61, 62] {
        l.published(t);
    }
    let held: Vec<u64> = l
        .last(RING_N)
        .iter()
        .filter(|r| r.kind == Kind::Published)
        .map(|r| r.c)
        .collect();
    assert_eq!(held, vec![1, 1, 0]);
    assert_eq!(l.stats().published_held, 2);
}

#[test]
fn unrelated_holds_are_accounted_apart() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.gate(0, gate(Point::Put));
    l.pass(1_000, 1, &none(), None); // no diff applied: unrelated
    l.gate(2_000, gate(Point::Put));
    l.pass(
        5_000,
        2,
        &Changes {
            walks: 3,
            space_touched: false,
            extents: vec![],
        },
        None,
    ); // diffs, none in this space: unrelated
    l.gate(6_000, gate(Point::Put));
    l.pass(16_000, 3, &touched(1), None); // this space changed: related
    let s = l.stats();
    assert_eq!((s.holds, s.hold_ns), (3, 14_000));
    assert_eq!((s.unrelated_holds, s.unrelated_hold_ns), (2, 4_000));
}

#[test]
fn pass_with_known_targets_uses_them() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    let ch = Changes {
        walks: 1,
        space_touched: true,
        extents: vec![(0x2_d75e_0000, 0x1_0000)],
    };
    l.gate(0, gate(Point::Put));
    l.pass(10, 1, &ch, Some(&[(0x2_d75e_5000, 0x100)]));
    l.gate(20, gate(Point::Put));
    l.pass(30, 2, &ch, Some(&[(0x5_0000_0000, 0x100)]));
    let codes: Vec<Overlap> = l
        .last(RING_N)
        .iter()
        .filter(|r| r.kind == Kind::GateEnd)
        .map(|r| Overlap::from_code(r.a))
        .collect();
    assert_eq!(codes, vec![Overlap::VaOverlap, Overlap::VaDisjoint]);
    assert_eq!(l.stats().unrelated_holds, 1, "only the disjoint one");
}

#[test]
fn ranges_overlap_table() {
    assert!(ranges_overlap((0x1000, 0x1000), (0x1800, 0x1000)));
    assert!(ranges_overlap((0x1000, 1), (0x1000, 1)));
    assert!(
        !ranges_overlap((0x1000, 0x1000), (0x2000, 0x1000)),
        "adjacent"
    );
    assert!(
        !ranges_overlap((0x2000, 0x1000), (0x1000, 0x1000)),
        "adjacent, swapped"
    );
    assert!(!ranges_overlap((0x1000, 0), (0x1000, 0x1000)), "empty");
    assert!(!ranges_overlap((0x1000, 0x1000), (0x1000, 0)), "empty");
    assert!(
        ranges_overlap((u64::MAX - 1, 100), (u64::MAX - 1, 1)),
        "saturates, no wrap"
    );
    assert!(!ranges_overlap((0, 1), (u64::MAX, 100)));
}

#[test]
fn classify_overlap_table() {
    let ch = [(0x1000u64, 0x1000u64), (0x9000, 0x1000)];
    assert_eq!(
        classify_overlap(0, true, None, &ch),
        Overlap::NoWalk,
        "no diff at all"
    );
    assert_eq!(classify_overlap(0, false, None, &[]), Overlap::NoWalk);
    assert_eq!(
        classify_overlap(4, false, None, &ch),
        Overlap::SpaceUntouched
    );
    assert_eq!(classify_overlap(4, true, None, &ch), Overlap::SpaceTouched);
    let t = [(0x9800u64, 0x10u64)];
    assert_eq!(
        classify_overlap(4, true, Some(&t), &ch),
        Overlap::VaOverlap,
        "second extent"
    );
    let t = [(0x5000u64, 0x10u64)];
    assert_eq!(
        classify_overlap(4, true, Some(&t), &ch),
        Overlap::VaDisjoint
    );
    assert_eq!(
        classify_overlap(4, false, Some(&[(0x1000, 0x10)]), &ch),
        Overlap::SpaceUntouched,
        "another space cannot alias, whatever the VAs"
    );
    assert_eq!(
        classify_overlap(4, true, Some(&[]), &ch),
        Overlap::VaDisjoint
    );
    for o in [
        Overlap::NoWalk,
        Overlap::SpaceUntouched,
        Overlap::SpaceTouched,
        Overlap::VaDisjoint,
        Overlap::VaOverlap,
    ] {
        assert_eq!(Overlap::from_code(o.code()), o);
    }
    assert!(Overlap::NoWalk.unrelated() && Overlap::SpaceUntouched.unrelated());
    assert!(Overlap::VaDisjoint.unrelated());
    assert!(!Overlap::SpaceTouched.unrelated() && !Overlap::VaOverlap.unrelated());
}

// ---- the unmap evaluation (M6) ---------------------------------------------------------------

#[test]
fn unmap_counts_only_the_channels_of_that_space() {
    let r = recorder();
    born(r, 0x6b, SP);
    born(r, 0x6c, SP);
    born(r, 0x70, SP2);
    chan(r, 0x6b).lock().unwrap().put_seen(1_000, 2);
    chan(r, 0x70).lock().unwrap().put_seen(1_000, 5); // other space: never evaluated here
    let tk = r.begin_apply_at(9_000, SP, 306_600, &[run(true, 0x2_d75e_0000, 0x1_0000)]);
    r.end_apply_at(9_500, tk);
    let d = r.dump("xid", 10_000, Some(0x6b), Some(0x2_d75e_5000));
    let u = d
        .iter()
        .find(|l| l.contains("GATELOG UNMAP"))
        .expect("UNMAP");
    assert!(u.contains("walk#306600"), "{u}");
    assert!(u.contains("space_chans=2 held_chans=1"), "{u}");
    assert!(
        u.contains("this_chan: pending=2 oldest_age_ns=8000 in_hold=0"),
        "{u}"
    );
    assert!(u.contains("t_start_ns=9000 t_end_ns=9500"), "{u}");
    // the other channel of the space and the other space got no unmap record
    assert!(!kinds(&chan(r, 0x6c).lock().unwrap()).contains(&Kind::Unmap));
    assert!(!kinds(&chan(r, 0x70).lock().unwrap()).contains(&Kind::Unmap));
    let l = chan(r, 0x6b);
    let l = l.lock().unwrap();
    let rec = l.last(1)[0];
    assert_eq!(rec.kind, Kind::Unmap);
    assert_eq!((rec.a, rec.b, rec.c), (2, 8_000, 306_600));
    assert_eq!(l.stats().held_across_unmap, 1);
}

#[test]
fn zero_held_unmap_reads_as_zero() {
    // The falsifier shape of H7: the copy was already published before the unmap.
    let r = recorder();
    born(r, 0x6b, SP);
    {
        let c = chan(r, 0x6b);
        let mut l = c.lock().unwrap();
        l.put_seen(1_000, 1);
        l.published(2_000);
    }
    let tk = r.begin_apply_at(9_000, SP, 306_600, &[run(true, 0x2_d75e_0000, 0x1_0000)]);
    r.end_apply_at(9_100, tk);
    let d = r.dump("xid", 10_000, Some(0x6b), Some(0x2_d75e_5000));
    let u = d
        .iter()
        .find(|l| l.contains("GATELOG UNMAP"))
        .expect("UNMAP");
    assert!(u.contains("held_chans=0"), "{u}");
    assert!(u.contains("this_chan: pending=0 in_hold=0"), "{u}");
    assert!(
        d.iter()
            .any(|l| l
                .contains("GATELOG SPACE (0xc1d0027e,0xff000850) unmaps=1 held_across_unmap=0")),
        "{d:#?}"
    );
    let l = chan(r, 0x6b);
    let l = l.lock().unwrap();
    assert_eq!(l.stats().held_across_unmap, 0);
    assert!(
        !kinds(&l).contains(&Kind::Unmap),
        "a zero leaves no record in the ring"
    );
}

#[test]
fn a_gated_channel_with_nothing_read_counts_as_held() {
    let r = recorder();
    born(r, 0x6b, SP);
    {
        let c = chan(r, 0x6b);
        // the gate stopped it before GP_PUT was read
        c.lock().unwrap().gate(1_000, gate(Point::Put));
    }
    let tk = r.begin_apply_at(4_000, SP, 7, &[run(true, 0x10_0000, 0x1000)]);
    r.end_apply_at(4_100, tk);
    let d = r.dump("xid", 5_000, Some(0x6b), Some(0x10_0000));
    let u = d
        .iter()
        .find(|l| l.contains("GATELOG UNMAP"))
        .expect("UNMAP");
    assert!(u.contains("held_chans=1"), "{u}");
    assert!(
        u.contains("this_chan: pending=0 oldest_age_ns=0 in_hold=1 hold_age_ns=3000"),
        "{u}"
    );
    let l = chan(r, 0x6b);
    let rec = l.lock().unwrap().last(1)[0];
    assert_eq!(
        (rec.kind, rec.a),
        (Kind::Unmap, kf_mem::gatelog::UNMAP_IN_HOLD)
    );
}

#[test]
fn a_map_only_diff_evaluates_nothing() {
    let r = recorder();
    born(r, 0x6b, SP);
    chan(r, 0x6b).lock().unwrap().put_seen(10, 3);
    let tk = r.begin_apply_at(20, SP, 306_599, &[run(false, 0x2_d75e_0000, 0x1_0000)]);
    r.end_apply_at(30, tk);
    let d = r.dump("xid", 40, Some(0x6b), None);
    assert!(d.iter().all(|l| !l.contains("GATELOG UNMAP")), "{d:#?}");
    assert!(
        d.iter().any(|l| l.contains("GATELOG SPACES spaces=0")),
        "{d:#?}"
    );
    assert_eq!(chan(r, 0x6b).lock().unwrap().stats().held_across_unmap, 0);
    // but the diff is on the walk ring: a hold that saw it is "space touched"
    let ch = r.changes_since(306_598, SP);
    assert_eq!((ch.walks, ch.space_touched), (1, true));
    assert_eq!(ch.extents, vec![(0x2_d75e_0000, 0x1_0000)]);
}

#[test]
fn unmap_is_evaluated_at_the_start_of_the_apply() {
    // An entry published DURING the apply was still waiting when the host began to unmap.
    let r = recorder();
    born(r, 0x6b, SP);
    chan(r, 0x6b).lock().unwrap().put_seen(1_000, 1);
    let tk = r.begin_apply_at(2_000, SP, 5, &[run(true, 0x1000, 0x1000)]);
    chan(r, 0x6b).lock().unwrap().published(2_500); // inside the apply
    r.end_apply_at(3_000, tk);
    let d = r.dump("xid", 4_000, Some(0x6b), Some(0x1000));
    let u = d
        .iter()
        .find(|l| l.contains("GATELOG UNMAP"))
        .expect("UNMAP");
    assert!(u.contains("this_chan: pending=1 oldest_age_ns=1000"), "{u}");
    assert!(u.contains("t_start_ns=2000 t_end_ns=3000"), "{u}");
}

#[test]
fn unmap_record_keeps_range_counts_and_the_first_runs() {
    let r = recorder();
    born(r, 0x6b, SP);
    chan(r, 0x6b).lock().unwrap().put_seen(1, 1);
    let mut runs = vec![run(false, 0x9000_0000, 0x1000)];
    for i in 0..10u64 {
        runs.push(run(true, 0x1000_0000 + i * 0x2000, 0x1000));
    }
    let tk = r.begin_apply_at(10, SP, 1, &runs);
    r.end_apply_at(11, tk);
    let d = r.dump("xid", 20, Some(0x6b), Some(0x1000_0000));
    let u = d
        .iter()
        .find(|l| l.contains("GATELOG UNMAP"))
        .expect("UNMAP");
    assert!(u.contains("unmaps=10 maps=1"), "{u}");
    assert!(u.contains("range=0x10000000..0x90001000"), "{u}");
    let listed = u.split("runs=[").nth(1).unwrap().split(']').next().unwrap();
    assert_eq!(listed.split(',').count(), UNMAP_RUNS, "{u}");
    assert!(
        listed.starts_with("0x10000000+0x1000,0x10002000+0x1000"),
        "{u}"
    );
}

#[test]
fn unmap_run_and_chan_lists_are_capped() {
    let r = recorder();
    for i in 0..20u32 {
        born(r, 0x100 + i, SP);
        chan(r, 0x100 + i).lock().unwrap().put_seen(1, 1);
    }
    let tk = r.begin_apply_at(10, SP, 1, &[run(true, 0x1000, 0x1000)]);
    r.end_apply_at(11, tk);
    // inspect through the dump of a listed channel and of an unlisted one
    let d = r.dump("xid", 20, Some(0x100), Some(0x1000));
    let first = d.iter().find(|l| l.contains("GATELOG UNMAP")).unwrap();
    assert!(first.contains("space_chans=20 held_chans=20"), "{first}");
    assert!(first.contains("this_chan: pending=1"), "{first}");
    let d = r.dump("xid", 21, Some(0x100 + 19), Some(0x1000));
    let last = d.iter().find(|l| l.contains("GATELOG UNMAP")).unwrap();
    assert!(
        last.contains("this_chan: unknown(list truncated)"),
        "a channel past the list is unknown, not zero: {last}"
    );
}

#[test]
fn freed_channels_are_not_evaluated() {
    let r = recorder();
    born(r, 0x6b, SP);
    born(r, 0x6c, SP);
    chan(r, 0x6b).lock().unwrap().put_seen(1, 4);
    r.chan_free(5, 0xc1d0_0001, 0x6b);
    chan(r, 0x6c).lock().unwrap().put_seen(1, 1);
    let tk = r.begin_apply_at(10, SP, 1, &[run(true, 0x1000, 0x1000)]);
    r.end_apply_at(11, tk);
    let d = r.dump("xid", 20, Some(0x6c), Some(0x1000));
    let u = d.iter().find(|l| l.contains("GATELOG UNMAP")).unwrap();
    assert!(u.contains("space_chans=1 held_chans=1"), "{u}");
    assert!(!kinds(&chan(r, 0x6b).lock().unwrap()).contains(&Kind::Unmap));
    assert_eq!(chan(r, 0x6b).lock().unwrap().freed_ns, Some(5));
}

#[test]
fn changes_since_selects_the_diffs_after_the_hold_began() {
    let r = recorder();
    let applies = [
        (5u64, SP, 0x1000u64),
        (6, SP2, 0x2000),
        (7, SP, 0x3000),
        (8, SP, 0x4000),
    ];
    for (walk, space, va) in applies {
        let tk = r.begin_apply_at(walk * 10, space, walk, &[run(false, va, 0x1000)]);
        r.end_apply_at(walk * 10 + 1, tk);
    }
    assert_eq!(r.last_walk(), 8);
    let c = r.changes_since(5, SP);
    assert_eq!((c.walks, c.space_touched), (3, true), "ids 6, 7, 8");
    assert_eq!(
        c.extents,
        vec![(0x4000, 0x1000), (0x3000, 0x1000)],
        "this space only"
    );
    let c = r.changes_since(6, SP2);
    assert_eq!((c.walks, c.space_touched, c.extents.len()), (2, false, 0));
    let c = r.changes_since(8, SP);
    assert_eq!(
        (c.walks, c.space_touched),
        (0, false),
        "ids <= the hold's own are excluded"
    );
    // the extent list is capped
    for i in 0..(2 * CHANGED_MAX as u64) {
        let tk = r.begin_apply_at(
            1000 + i,
            SP,
            100 + i,
            &[run(false, 0x10_0000 + i * 0x1000, 0x1000)],
        );
        r.end_apply_at(1000 + i, tk);
    }
    let c = r.changes_since(8, SP);
    assert_eq!(
        (c.walks, c.extents.len()),
        (2 * CHANGED_MAX as u64, CHANGED_MAX)
    );
}

// ---- the recorder's bounds (M7) --------------------------------------------------------------

#[test]
fn chan_table_is_bounded_and_evicts_freed_first() {
    let r = recorder();
    for i in 0..MAX_CHANS as u32 {
        born(r, 0x1000 + i, SP + u64::from(i));
    }
    assert!(
        !r.chan_birth(1, 0x9000, 1, 0x9000, SP, 256),
        "table full of live channels"
    );
    assert!(r.chan_ref(0x9000).is_none());
    let d = r.dump("shutdown", 2, None, None);
    assert!(
        d[0].contains(&format!("chans={MAX_CHANS} untracked=1")),
        "{}",
        d[0]
    );
    // a freed channel is evicted to make room
    r.chan_free(3, 0xc1d0_0001, 0x1005);
    assert!(r.chan_birth(4, 0x9001, 1, 0x9001, SP, 256));
    assert!(r.chan_ref(0x1005).is_none() && r.chan_ref(0x9001).is_some());
    let d = r.dump("shutdown", 5, None, None);
    assert!(
        d[0].contains(&format!("chans={MAX_CHANS} untracked=1")),
        "{}",
        d[0]
    );
}

#[test]
fn a_reused_token_replaces_the_old_record() {
    let r = recorder();
    born(r, 0x6b, SP);
    chan(r, 0x6b).lock().unwrap().put_seen(1, 3);
    assert!(r.chan_birth(10, 0x6b, 0xc1d0_0099, 5, SP2, 256));
    let l = chan(r, 0x6b);
    let l = l.lock().unwrap();
    assert_eq!(
        (l.vas, l.client, l.held_now(20).count),
        (SP2, 0xc1d0_0099, 0)
    );
    drop(l);
    // the old space no longer lists the token: an unmap there finds no channel
    let tk = r.begin_apply_at(30, SP, 1, &[run(true, 0x1000, 0x1000)]);
    r.end_apply_at(31, tk);
    let d = r.dump("xid", 40, Some(0x6b), None);
    assert!(d.iter().all(|l| !l.contains("held_chans=1")), "{d:#?}");
    assert!(
        d.iter()
            .any(|l| l
                .contains("GATELOG SPACE (0xc1d0027e,0xff000850) unmaps=1 held_across_unmap=0")),
        "the old space saw an unmap with no channel waiting: {d:#?}"
    );
}

#[test]
fn walk_and_unmap_rings_are_bounded() {
    let r = recorder();
    for i in 0..(WALKS_N as u64 + 100) {
        let tk = r.begin_apply_at(i, SP, i + 1, &[run(i % 2 == 0, 0x1000, 0x1000)]);
        r.end_apply_at(i, tk);
    }
    let d = r.dump("shutdown", 1, None, None);
    assert!(
        d[0].contains(&format!("walks_kept={WALKS_N} walks_overflow=100")),
        "{}",
        d[0]
    );
    let unmaps = (WALKS_N as u64 + 100).div_ceil(2) as usize;
    assert!(unmaps < UNMAPS_N);
    assert!(
        d[0].contains(&format!("unmaps_kept={unmaps} unmaps_overflow=0")),
        "{}",
        d[0]
    );
    for i in 0..(2 * UNMAPS_N as u64) {
        let tk = r.begin_apply_at(i, SP, 100_000 + i, &[run(true, 0x1000, 0x1000)]);
        r.end_apply_at(i, tk);
    }
    let d = r.dump("shutdown", 1, None, None);
    assert!(
        d[0].contains(&format!("unmaps_kept={UNMAPS_N} unmaps_overflow=")),
        "{}",
        d[0]
    );
    assert!(!d[0].contains("unmaps_overflow=0"), "{}", d[0]);
}

#[test]
fn per_channel_ring_is_bounded_whatever_the_guest_does() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    for i in 0..(3 * RING_N as u64) {
        l.entry_seen(i, (i % 256) as u32, i);
    }
    let (len, over) = l.ring_counts();
    assert_eq!((len, over), (RING_N, 2 * RING_N as u64));
    assert_eq!(l.last(1)[0].t_ns, 3 * RING_N as u64 - 1);
}

// ---- the dump (M8) ---------------------------------------------------------------------------

#[test]
fn dump_shows_only_the_last_64_records_newest_last() {
    let r = recorder();
    born(r, 0x6b, SP);
    {
        let c = chan(r, 0x6b);
        let mut l = c.lock().unwrap();
        for i in 0..200u64 {
            l.entry_seen(1_000 + i, (i % 256) as u32, i);
        }
    }
    let d = r.dump("xid", 9_999_999, Some(0x6b), None);
    let recs: Vec<&String> = d
        .iter()
        .filter(|l| l.contains("GATELOG REC token=0x6b"))
        .collect();
    assert_eq!(recs.len(), DUMP_LAST);
    assert!(recs[0].contains("t_ns=1136 "), "{}", recs[0]);
    assert!(
        recs[DUMP_LAST - 1].contains("t_ns=1199 "),
        "{}",
        recs[DUMP_LAST - 1]
    );
    assert!(recs[0].contains("seen slot=136"), "{}", recs[0]);
}

#[test]
fn dump_matches_the_xid_chid_on_the_low_16_bits() {
    let r = recorder();
    born(r, 0x1_006b, SP);
    born(r, 0x6c, SP2);
    chan(r, 0x1_006b).lock().unwrap().entry_seen(5, 1, 1);
    chan(r, 0x6c).lock().unwrap().entry_seen(6, 2, 2);
    let d = r.dump("xid", 100, Some(0x6b), None);
    assert!(
        d.iter().any(|l| l.contains("GATELOG CHAN token=0x1006b")),
        "{d:#?}"
    );
    assert!(d.iter().all(|l| !l.contains("token=0x6c ")), "{d:#?}");
    let d = r.dump("xid", 100, Some(0x77), None);
    assert!(
        d.iter()
            .any(|l| l.contains("GATELOG CHAN none tracked with chid=0x77")),
        "{d:#?}"
    );
    let d = r.dump("shutdown", 100, None, None);
    assert!(
        d.iter().all(|l| !l.contains("GATELOG REC")),
        "no channel asked for: no records"
    );
}

#[test]
fn dump_header_carries_the_vahist_clock() {
    let r = recorder();
    let d = r.dump("xid", 3_330_063_000_000, Some(0x6b), Some(0x2_d75e_5000));
    assert!(
        d[0].starts_with("kf3: GATELOG DUMP #1 why=xid t_ms=3330063 t_ns=3330063000000 "),
        "{}",
        d[0]
    );
    assert!(d[0].contains("t0_unix_ms=1792876069894"), "{}", d[0]);
    assert!(d[0].contains("chid=0x6b va=0x2d75e5000"), "{}", d[0]);
    assert!(d.last().unwrap().starts_with("kf3: GATELOG END #1 lines="));
    assert_eq!(r.t0_unix_ms(), T0_UNIX_MS);
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.entry_seen(3_330_050_999_999, 1, 1);
    let line = kf_mem::gatelog::rec_text(0x6b, &l.last(1)[0]);
    assert!(
        line.contains("t_ms=3330050 "),
        "ms floor like vahist: {line}"
    );
}

#[test]
fn dumps_per_process_are_capped() {
    let r = recorder();
    for i in 0..DUMPS_MAX {
        let d = r.dump("xid", u64::from(i), None, None);
        assert!(d[0].contains(&format!("DUMP #{}", i + 1)), "{}", d[0]);
    }
    let d = r.dump("shutdown", 99, None, None);
    assert_eq!(d.len(), 1);
    assert!(
        d[0].contains("capped") && d[0].starts_with("kf3: GATELOG "),
        "{}",
        d[0]
    );
}

#[test]
fn dump_lists_the_unmaps_that_cover_the_fault_va() {
    // Run 5227's shape: MAP walk 306599, UNMAP walk 306600, the fault at 0x2d75e5000.
    let r = recorder();
    born(r, 0x6b, SP);
    let tk = r.begin_apply_at(
        3_330_050_000_000,
        SP,
        306_599,
        &[run(false, 0x2_d75e_0000, 0x1_0000)],
    );
    r.end_apply_at(3_330_050_900_000, tk);
    {
        let c = chan(r, 0x6b);
        let mut l = c.lock().unwrap();
        l.put_seen(3_330_050_950_000, 1); // the copy's doorbell, seen while the gate holds
        l.gate(3_330_050_950_100, gate(Point::Put));
    }
    let tk = r.begin_apply_at(
        3_330_051_000_000,
        SP,
        306_600,
        &[run(true, 0x2_d75e_0000, 0x1_0000)],
    );
    r.end_apply_at(3_330_051_400_000, tk);
    // an unrelated unmap elsewhere in the same space, and one in another space
    let tk = r.begin_apply_at(
        3_330_052_000_000,
        SP,
        306_601,
        &[run(true, 0x7_0000_0000, 0x1000)],
    );
    r.end_apply_at(3_330_052_100_000, tk);
    let tk = r.begin_apply_at(
        3_330_052_200_000,
        SP2,
        306_602,
        &[run(true, 0x2_d75e_0000, 0x1_0000)],
    );
    r.end_apply_at(3_330_052_300_000, tk);
    let d = r.dump("xid", 3_330_063_000_000, Some(0x6b), Some(0x2_d75e_5000));
    let un: Vec<&String> = d.iter().filter(|l| l.contains("GATELOG UNMAP")).collect();
    assert_eq!(
        un.len(),
        1,
        "only the unmap that covers the VA, in the channel's space: {un:#?}"
    );
    let u = un[0];
    assert!(
        u.contains("walk#306600") && u.contains("space=(0xc1d0027e,0xff000850)"),
        "{u}"
    );
    assert!(u.contains("range=0x2d75e0000..0x2d75f0000"), "{u}");
    assert!(
        u.contains("this_chan: pending=1 oldest_age_ns=50000 in_hold=1 hold_age_ns=49900"),
        "{u}"
    );
    // without a VA every unmap of the space is listed, newest first
    let d = r.dump("xid", 3_330_063_000_001, Some(0x6b), None);
    let un: Vec<&String> = d.iter().filter(|l| l.contains("GATELOG UNMAP")).collect();
    assert_eq!(un.len(), 2);
    assert!(
        un[0].contains("walk#306601") && un[1].contains("walk#306600"),
        "{un:#?}"
    );
}

#[test]
fn dump_counts_held_across_unmap_per_space() {
    let r = recorder();
    born(r, 0x6b, SP);
    born(r, 0x70, SP2);
    chan(r, 0x6b).lock().unwrap().put_seen(1, 1);
    for (i, held) in [(1u64, true), (2, true), (3, false)] {
        if !held {
            chan(r, 0x6b).lock().unwrap().reconcile(10, 1);
        }
        let tk = r.begin_apply_at(10 * i, SP, i, &[run(true, 0x1000 * i, 0x1000)]);
        r.end_apply_at(10 * i + 1, tk);
    }
    chan(r, 0x70).lock().unwrap().put_seen(1, 2);
    let tk = r.begin_apply_at(50, SP2, 9, &[run(true, 0x1000, 0x1000)]);
    r.end_apply_at(51, tk);
    let d = r.dump("shutdown", 100, None, None);
    assert!(
        d.iter()
            .any(|l| l.contains("GATELOG SPACES spaces=2")
                && l.contains("held_across_unmap_total=3")),
        "{d:#?}"
    );
    let sp: Vec<&String> = d.iter().filter(|l| l.contains("GATELOG SPACE (")).collect();
    assert_eq!(sp.len(), 2);
    assert!(
        sp[0].contains(
            "(0xc1d0027e,0xff000850) unmaps=3 held_across_unmap=2 chan_events=2 entries=2"
        ),
        "most held first: {}",
        sp[0]
    );
    assert!(
        sp[1].contains(
            "(0xc1d00300,0xff000001) unmaps=1 held_across_unmap=1 chan_events=1 entries=2"
        ),
        "{}",
        sp[1]
    );
}

#[test]
fn dump_reports_hold_totals_and_the_top_channels() {
    let r = recorder();
    born(r, 0x6b, SP);
    born(r, 0x6c, SP2);
    born(r, 0x6d, SP2);
    {
        let c = chan(r, 0x6b);
        let mut l = c.lock().unwrap();
        l.gate(0, gate(Point::Put));
        l.pass(5_000_000, 1, &none(), None); // 5 ms, unrelated
    }
    {
        let c = chan(r, 0x6c);
        let mut l = c.lock().unwrap();
        l.gate(0, gate(Point::Put));
        l.pass(20_000_000, 1, &touched(1), None); // 20 ms, related
    }
    let d = r.dump("shutdown", 100_000_000, None, None);
    let f = d.iter().find(|l| l.contains("GATELOG FLEET")).unwrap();
    assert!(f.contains("holds=2 "), "{f}");
    assert!(
        f.contains("hold_ms=25 unrelated_holds=1 unrelated_hold_ms=5 longest_hold_ms=20"),
        "{f}"
    );
    let top: Vec<&String> = d.iter().filter(|l| l.contains("GATELOG HOLDTOP")).collect();
    assert_eq!(top.len(), 2, "channels that were never held are not listed");
    assert!(
        top[0].contains("token=0x6c") && top[0].contains("hold_ms=20"),
        "{}",
        top[0]
    );
    assert!(
        top[1].contains("token=0x6b") && top[1].contains("unrelated_hold_ms=5"),
        "{}",
        top[1]
    );
}

#[test]
fn fleet_totals_survive_an_evicted_channel() {
    let r = recorder();
    born(r, 0x6b, SP);
    {
        let c = chan(r, 0x6b);
        let mut l = c.lock().unwrap();
        l.gate(0, gate(Point::Put));
        l.pass(7_000_000, 1, &none(), None);
    }
    assert!(
        r.chan_birth(9, 0x6b, 0xc1d0_0002, 9, SP2, 256),
        "token reuse"
    );
    let d = r.dump("shutdown", 10, None, None);
    let f = d.iter().find(|l| l.contains("GATELOG FLEET")).unwrap();
    assert!(f.contains("holds=1 ") && f.contains("hold_ms=7 "), "{f}");
}

#[test]
fn dump_is_bounded_and_every_line_capped() {
    let r = recorder();
    // many channels, many spaces, every ring full, hostile values everywhere
    for i in 0..MAX_CHANS as u32 {
        let vas = (u64::from(0xffff_ff00 + i) << 32) | u64::from(u32::MAX - i);
        born(r, 0xffff_0000 + i, vas);
        let c = chan(r, 0xffff_0000 + i);
        let mut l = c.lock().unwrap();
        for k in 0..(RING_N as u64 + 10) {
            l.entry_seen(k, u32::MAX, u64::MAX);
        }
        l.put_seen(1, 255);
        l.gate(2, gate(Point::Publish));
    }
    for s in 0..300u64 {
        let tk = r.begin_apply_at(s, (s << 32) | s, s, &[run(true, u64::MAX - 5, u64::MAX)]);
        r.end_apply_at(s, tk);
    }
    for i in 0..DUMPS_MAX {
        let d = r.dump("xid", u64::MAX, Some(0xffff_u32.min(i)), Some(u64::MAX));
        assert!(d.len() <= DUMP_LINES_MAX, "{} lines", d.len());
        for l in &d {
            assert!(l.starts_with("kf3: GATELOG "), "{l}");
            assert!(l.len() <= LINE_MAX, "{} bytes: {l}", l.len());
            assert!(!l.contains('\n'));
        }
    }
    let d = r.dump("xid", 1, Some(0), Some(0));
    assert_eq!(d.len(), 1, "past the dump cap: one line");
}

#[test]
fn space_counters_are_bounded() {
    let r = recorder();
    for s in 0..(kf_mem::gatelog::SPACES_MAX as u64 + 40) {
        let tk = r.begin_apply_at(s, 0x1_0000_0000 + s, s, &[run(true, 0x1000, 0x1000)]);
        r.end_apply_at(s, tk);
    }
    let d = r.dump("shutdown", 1, None, None);
    let spaces = d.iter().find(|l| l.contains("GATELOG SPACES")).unwrap();
    assert!(
        spaces.contains(&format!(
            "spaces={} untracked=40",
            kf_mem::gatelog::SPACES_MAX
        )),
        "{spaces}"
    );
    assert!(
        d.iter().filter(|l| l.contains("GATELOG SPACE (")).count() <= kf_mem::gatelog::DUMP_SPACES
    );
}

// ---- the hooks' handle, on the real clock (M4, M5) ------------------------------------------

#[test]
fn handle_flow_closes_the_hold_on_publish() {
    let r = recorder();
    born(r, 0x6b, SP);
    let h = r.handle(0x6b).expect("handle");
    h.put_seen(1);
    h.gate(WHY_VA_BUSY, Point::Put);
    h.gate(WHY_VA_BUSY, Point::Put);
    assert!(chan(r, 0x6b).lock().unwrap().in_hold());
    h.published();
    let c = chan(r, 0x6b);
    let l = c.lock().unwrap();
    assert!(
        !l.in_hold(),
        "a publication proves the gate let the channel through"
    );
    let s = l.stats();
    assert_eq!(
        (s.holds, s.regates, s.published, s.published_held),
        (1, 1, 1, 1)
    );
    assert_eq!(
        kinds(&l),
        vec![
            Kind::PutAdvance,
            Kind::GateStart,
            Kind::GateEnd,
            Kind::Published
        ]
    );
    assert!(r.handle(0x99).is_none());
}

#[test]
fn handle_pass_classifies_by_the_walks_applied() {
    let r = recorder();
    born(r, 0x6b, SP);
    let h = r.handle(0x6b).expect("handle");
    // hold 1: a diff in ANOTHER space lands meanwhile
    h.gate(WHY_VA_BUSY, Point::Put);
    let tk = r.begin_apply(SP2, 1, &[run(false, 0x1000, 0x1000)]);
    r.end_apply(tk);
    h.pass();
    // hold 2: a diff in THIS space lands meanwhile
    h.gate(WHY_VA_BUSY, Point::Put);
    let tk = r.begin_apply(SP, 2, &[run(true, 0x1000, 0x1000)]);
    r.end_apply(tk);
    h.pass();
    // hold 3: nothing lands
    h.gate(WHY_VA_BUSY, Point::Put);
    h.pass();
    h.pass(); // no hold open: nothing happens
    let codes: Vec<Overlap> = chan(r, 0x6b)
        .lock()
        .unwrap()
        .last(RING_N)
        .iter()
        .filter(|x| x.kind == Kind::GateEnd)
        .map(|x| Overlap::from_code(x.a))
        .collect();
    assert_eq!(
        codes,
        vec![
            Overlap::SpaceUntouched,
            Overlap::SpaceTouched,
            Overlap::NoWalk
        ]
    );
    assert_eq!(r.last_walk(), 2);
}

#[test]
fn handle_gate_records_the_space_being_applied() {
    let r = recorder();
    born(r, 0x6b, SP);
    let h = r.handle(0x6b).expect("handle");
    let tk = r.begin_apply(SP2, 41, &[run(false, 0x1000, 0x1000)]);
    h.gate(WHY_VA_BUSY | WHY_SPLIT_QUEUED, Point::Publish); // while the VA thread applies SP2
    r.end_apply(tk);
    let c = chan(r, 0x6b);
    let g = c.lock().unwrap().hold_gate().unwrap();
    assert_eq!(
        (g.space, g.reason, g.point),
        (SP2, WHY_VA_BUSY | WHY_SPLIT_QUEUED, Point::Publish)
    );
    assert_eq!(g.walk, 0, "the last applied diff before the hold");
    h.gate(WHY_VA_BUSY, Point::Put);
    h.pass();
    h.gate(WHY_VA_BUSY, Point::Put);
    let g = c.lock().unwrap().hold_gate().unwrap();
    assert_eq!(
        (g.space, g.walk),
        (0, 41),
        "idle VA thread: no space; the last diff is 41"
    );
}

// ---- the flag (M9) ---------------------------------------------------------------------------

#[test]
fn flag_is_exactly_one() {
    assert!(flag_on(Some("1")));
    for v in [
        None,
        Some(""),
        Some("0"),
        Some("yes"),
        Some("true"),
        Some("11"),
        Some(" 1"),
        Some("1 "),
        Some("lookup"),
    ] {
        assert!(!flag_on(v), "{v:?}");
    }
}

#[test]
fn the_flag_is_off_by_default() {
    // the test environment does not set it: the default path records nothing
    assert!(std::env::var_os("KF3_DIAG_GATELOG").is_none());
    assert!(!kf_mem::gatelog::enabled());
}

// ---- second killers, added for the mutants one test alone caught -----------------------------

#[test]
fn classify_adjacent_extents_are_disjoint() {
    let changed = [(0x1000u64, 0x1000u64)];
    assert_eq!(
        classify_overlap(1, true, Some(&[(0x2000, 0x1000)]), &changed),
        Overlap::VaDisjoint,
        "the next page is not the changed page"
    );
    assert_eq!(
        classify_overlap(1, true, Some(&[(0x0, 0x1000)]), &changed),
        Overlap::VaDisjoint
    );
    assert_eq!(
        classify_overlap(1, true, Some(&[(0x1fff, 0x1000)]), &changed),
        Overlap::VaOverlap,
        "one byte in"
    );
}

#[test]
fn the_unmap_chan_list_holds_exactly_unmap_chans() {
    let r = recorder();
    let n = UNMAP_CHANS as u32 + 1;
    for i in 0..n {
        born(r, 0x300 + i, SP);
        chan(r, 0x300 + i).lock().unwrap().put_seen(1, 1);
    }
    let tk = r.begin_apply_at(10, SP, 1, &[run(true, 0x1000, 0x1000)]);
    r.end_apply_at(11, tk);
    let listed = r.dump(
        "xid",
        20,
        Some(0x300 + UNMAP_CHANS as u32 - 1),
        Some(0x1000),
    );
    let u = listed.iter().find(|l| l.contains("GATELOG UNMAP")).unwrap();
    assert!(u.contains(&format!("held_chans={n}")), "{u}");
    assert!(
        u.contains("this_chan: pending=1"),
        "the last listed channel: {u}"
    );
    let past = r.dump("xid", 21, Some(0x300 + UNMAP_CHANS as u32), Some(0x1000));
    let u = past.iter().find(|l| l.contains("GATELOG UNMAP")).unwrap();
    assert!(
        u.contains("unknown(list truncated)"),
        "the first one past it: {u}"
    );
}

#[test]
fn untracked_channels_are_counted_not_tracked() {
    let r = recorder();
    for i in 0..(MAX_CHANS as u32 + 5) {
        let _ = r.chan_birth(0, 0x2000 + i, 1, 0x2000 + i, SP + u64::from(i), 256);
    }
    let d = r.dump("shutdown", 1, None, None);
    assert!(
        d[0].contains(&format!("chans={MAX_CHANS} untracked=5")),
        "{}",
        d[0]
    );
    assert!(r.chan_ref(0x2000 + MAX_CHANS as u32).is_none());
    assert!(r.chan_ref(0x2000).is_some());
}

#[test]
fn bound_lines_keeps_the_head_and_leaves_room_for_the_end_line() {
    let mut v: Vec<String> = (0..1000).map(|i| i.to_string()).collect();
    bound_lines(&mut v);
    assert_eq!(v.len(), DUMP_LINES_MAX - 1);
    assert_eq!(v[0], "0");
    assert_eq!(v[DUMP_LINES_MAX - 2], (DUMP_LINES_MAX - 2).to_string());
    let mut short = vec!["a".to_string(), "b".to_string()];
    bound_lines(&mut short);
    assert_eq!(short, vec!["a", "b"]);
}

#[test]
fn the_dump_line_budget_fits_under_the_cap() {
    use kf_mem::gatelog::{DUMP_SPACES, DUMP_TOP, DUMP_UNMAPS};
    // header, fleet, two channels (a line + the records each), the unmaps for each, the spaces
    // header and lines, the top channels, the none-tracked line, the end line
    let worst = 2 + 2 * (1 + DUMP_LAST) + 2 * DUMP_UNMAPS + 1 + DUMP_SPACES + DUMP_TOP + 1;
    assert!(worst <= DUMP_LINES_MAX, "{worst} lines");
}

#[test]
fn the_space_table_keeps_counting_the_spaces_it_has_when_full() {
    use kf_mem::gatelog::SPACES_MAX;
    let r = recorder();
    for s in 0..(SPACES_MAX as u64 + 3) {
        let tk = r.begin_apply_at(s, 0x1_0000_0000 + s, s, &[run(true, 0x1000, 0x1000)]);
        r.end_apply_at(s, tk);
    }
    // an already tracked space still counts, an untracked one does not appear
    let tk = r.begin_apply_at(900, 0x1_0000_0000, 900, &[run(true, 0x1000, 0x1000)]);
    r.end_apply_at(901, tk);
    let d = r.dump("shutdown", 1000, None, None);
    let h = d.iter().find(|l| l.contains("GATELOG SPACES")).unwrap();
    assert!(
        h.contains(&format!("spaces={SPACES_MAX} untracked=3")),
        "{h}"
    );
    assert!(
        d.iter().any(|l| l.contains("SPACE (0x1,0x0) unmaps=2 ")),
        "{d:#?}"
    );
}

#[test]
fn dump_chid_match_takes_every_token_with_those_low_bits() {
    let r = recorder();
    born(r, 0x71, SP);
    born(r, 0x2_0071, SP2);
    born(r, 0x72, SP2);
    let d = r.dump("xid", 5, Some(0x71), None);
    let chans: Vec<&String> = d
        .iter()
        .filter(|l| l.contains("GATELOG CHAN token="))
        .collect();
    assert_eq!(chans.len(), 2, "{chans:#?}");
    assert!(chans.iter().any(|l| l.contains("token=0x71 ")));
    assert!(chans.iter().any(|l| l.contains("token=0x20071 ")));
    assert!(chans.iter().all(|l| !l.contains("token=0x72 ")));
}

#[test]
fn the_shutdown_dump_carries_the_clock_too() {
    let r = recorder();
    let d = r.dump("shutdown", 7_000_000_000, None, None);
    assert!(
        d[0].starts_with("kf3: GATELOG DUMP #1 why=shutdown t_ms=7000 "),
        "{}",
        d[0]
    );
    assert!(d[0].contains("t0_unix_ms=1792876069894"), "{}", d[0]);
    assert!(d[0].contains("chid=none va=none"), "{}", d[0]);
}

#[test]
fn a_short_ring_is_printed_whole_and_a_long_one_by_exactly_64() {
    let r = recorder();
    born(r, 0x6b, SP);
    born(r, 0x6c, SP2);
    {
        let c = chan(r, 0x6b);
        let mut l = c.lock().unwrap();
        for i in 0..(DUMP_LAST as u64 + 1) {
            l.entry_seen(i, 0, i);
        }
    }
    {
        let c = chan(r, 0x6c);
        let mut l = c.lock().unwrap();
        for i in 0..10u64 {
            l.entry_seen(i, 0, i);
        }
    }
    let long = r.dump("xid", 100, Some(0x6b), None);
    assert_eq!(
        long.iter().filter(|l| l.contains("GATELOG REC")).count(),
        DUMP_LAST
    );
    assert!(
        long.iter()
            .any(|l| l.contains("GATELOG REC token=0x6b t_ns=1 ")),
        "the oldest one is dropped"
    );
    assert!(
        long.iter()
            .all(|l| !l.contains("GATELOG REC token=0x6b t_ns=0 "))
    );
    let short = r.dump("xid", 100, Some(0x6c), None);
    assert_eq!(
        short.iter().filter(|l| l.contains("GATELOG REC")).count(),
        10
    );
}

#[test]
fn spaces_are_listed_most_held_first_then_by_key() {
    let r = recorder();
    let (a, b, c) = (0x1_0000_0001u64, 0x2_0000_0001u64, 0x3_0000_0001u64);
    for (i, (space, token, held)) in [(a, 0x40u32, 1u32), (b, 0x41, 3), (c, 0x42, 3)]
        .into_iter()
        .enumerate()
    {
        born(r, token, space);
        chan(r, token).lock().unwrap().put_seen(1, 1);
        for k in 0..held {
            let w = (i as u64) * 10 + u64::from(k);
            let tk = r.begin_apply_at(10 + w, space, w + 1, &[run(true, 0x1000, 0x1000)]);
            r.end_apply_at(11 + w, tk);
        }
    }
    let d = r.dump("shutdown", 100, None, None);
    let order: Vec<&str> = d
        .iter()
        .filter(|l| l.contains("GATELOG SPACE ("))
        .map(|l| l.split(' ').nth(3).unwrap())
        .collect();
    assert_eq!(order, vec!["(0x2,0x1)", "(0x3,0x1)", "(0x1,0x1)"], "{d:#?}");
}

#[test]
fn flag_rejects_near_misses() {
    for v in ["on", "ON", "1\n", "01", "1,1", "\u{ff11}", "one"] {
        assert!(!flag_on(Some(v)), "{v:?}");
    }
}

#[test]
fn the_record_text_names_the_point_and_the_reason() {
    use kf_mem::gatelog::rec_text;
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.gate(
        5,
        Gate {
            reason: WHY_VA_BUSY,
            point: Point::Put,
            space: 0,
            walk: 3,
        },
    );
    let t = rec_text(0x6b, &l.last(1)[0]);
    assert!(
        t.contains("gate_start why=0x1 point=put applying_space=none last_walk#=3"),
        "{t}"
    );
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.gate(
        5,
        Gate {
            reason: WHY_SPLIT_QUEUED,
            point: Point::Publish,
            space: SP,
            walk: 4,
        },
    );
    let t = rec_text(0x6b, &l.last(1)[0]);
    assert!(
        t.contains("why=0x4 point=publish applying_space=(0xc1d0027e,0xff000850) last_walk#=4"),
        "{t}"
    );
}

#[test]
fn the_oldest_age_is_the_front_chunks_not_the_newest() {
    let mut l = ChanLog::new(0x6b, 1, 2, SP, 256, 0);
    l.put_seen(100, 2);
    l.put_seen(300, 7);
    let h = l.held_now(400);
    assert_eq!((h.count, h.oldest_age_ns), (7, 300));
    l.published(400);
    l.published(400);
    let h = l.held_now(400);
    assert_eq!(
        (h.count, h.oldest_age_ns),
        (5, 100),
        "now the second chunk is the oldest"
    );
}

#[test]
fn bound_lines_boundary() {
    let mk = |n: usize| (0..n).map(|i| i.to_string()).collect::<Vec<_>>();
    let mut v = mk(DUMP_LINES_MAX - 1);
    bound_lines(&mut v);
    assert_eq!(
        v.len(),
        DUMP_LINES_MAX - 1,
        "room for the end line: unchanged"
    );
    let mut v = mk(DUMP_LINES_MAX);
    bound_lines(&mut v);
    assert_eq!(v.len(), DUMP_LINES_MAX - 1);
}

#[test]
fn the_space_with_more_held_unmaps_comes_first_even_with_the_larger_key() {
    let r = recorder();
    let (low, high) = (0x1_0000_0001u64, 0x9_0000_0001u64);
    for (space, token, unmaps) in [(low, 0x50u32, 1u64), (high, 0x51, 4)] {
        born(r, token, space);
        chan(r, token).lock().unwrap().put_seen(1, 1);
        for k in 0..unmaps {
            let tk = r.begin_apply_at(
                10 + k,
                space,
                100 * u64::from(token) + k,
                &[run(true, 0x1000, 0x1000)],
            );
            r.end_apply_at(11 + k, tk);
        }
    }
    let d = r.dump("shutdown", 100, None, None);
    let first = d.iter().find(|l| l.contains("GATELOG SPACE (")).unwrap();
    assert!(
        first.contains("(0x9,0x1) unmaps=4 held_across_unmap=4"),
        "{first}"
    );
}

#[test]
fn an_unmap_reports_the_oldest_held_entry_not_the_newest() {
    let r = recorder();
    born(r, 0x6b, SP);
    {
        let c = chan(r, 0x6b);
        let mut l = c.lock().unwrap();
        l.put_seen(1_000, 1);
        l.put_seen(5_000, 4);
    }
    let tk = r.begin_apply_at(9_000, SP, 1, &[run(true, 0x1000, 0x1000)]);
    r.end_apply_at(9_001, tk);
    let d = r.dump("xid", 10_000, Some(0x6b), Some(0x1000));
    let u = d.iter().find(|l| l.contains("GATELOG UNMAP")).unwrap();
    assert!(
        u.contains("this_chan: pending=4 oldest_age_ns=8000 "),
        "{u}"
    );
    let rec = chan(r, 0x6b).lock().unwrap().last(1)[0];
    assert_eq!((rec.kind, rec.a, rec.b), (Kind::Unmap, 4, 8_000));
}

#[test]
fn the_shutdown_dump_is_printed_once_whichever_exit_path_asks_first() {
    let r = recorder();
    born(r, 0x6b, SP);
    let first = r.dump_shutdown(5_000_000_000);
    assert!(
        first[0].starts_with("kf3: GATELOG DUMP #1 why=shutdown t_ms=5000 "),
        "{first:#?}"
    );
    assert!(first.last().unwrap().starts_with("kf3: GATELOG END #1 "));
    assert!(
        r.dump_shutdown(6_000_000_000).is_empty(),
        "the second path prints nothing"
    );
}

#[test]
fn the_shutdown_dump_still_fits_after_xid_dumps() {
    let r = recorder();
    for i in 0..(DUMPS_MAX - 1) {
        let _ = r.dump("xid", u64::from(i), None, None);
    }
    let d = r.dump_shutdown(1);
    assert!(
        d[0].contains("why=shutdown"),
        "the last slot is the shutdown dump's: {}",
        d[0]
    );
    assert!(r.dump("xid", 2, None, None)[0].contains("capped"));
}
