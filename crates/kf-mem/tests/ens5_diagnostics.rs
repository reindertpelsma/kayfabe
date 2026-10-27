//! ENS5 diagnostic bounds, recent-tail visibility, threshold and one-shot tests.
use kf_mem::recent::{Slow, retain};
#[test]
fn first_four_and_last_twelve_survive_a_filled_log() {
    let mut rows = vec![];
    for i in 0..100 {
        retain(&mut rows, i);
    }
    assert_eq!(&rows[..4], &[0, 1, 2, 3]);
    assert_eq!(&rows[4..], &(88..100).collect::<Vec<_>>());
}
#[test]
fn a_later_boot_phase_cannot_be_hidden_by_early_refusals() {
    let mut rows = vec![];
    for i in 0..1000 {
        retain(&mut rows, i);
    }
    assert_eq!(rows.len(), 16);
    assert_eq!(rows.last(), Some(&999));
}
#[test]
fn apply_max_includes_fast_and_slow_values() {
    let mut s = Slow::default();
    s.observe(50, [0; 4]);
    s.observe(10, [0; 4]);
    assert_eq!(s.apply_ns_max, 50);
}
#[test]
fn the_last_fast_apply_does_not_erase_a_slow_peak() {
    let mut s = Slow::default();
    s.observe(300_000_000, [0; 4]);
    s.observe(1, [0; 4]);
    assert_eq!(s.apply_ns_max, 300_000_000);
    assert_eq!(s.slow, 1);
}
#[test]
fn threshold_is_inclusive_at_200ms() {
    let mut s = Slow::default();
    assert_eq!(s.observe(199_999_999, [0; 4]), None);
    assert_eq!(s.observe(200_000_000, [4, 3, 2, 1]), Some("admit"));
}
#[test]
fn a_slow_sparse_admit_produces_a_named_first_snapshot() {
    let mut s = Slow::default();
    assert_eq!(s.observe(250_000_000, [200, 10, 0, 1]), Some("admit"));
}
#[test]
fn slow_apply_snapshot_is_once_while_counter_keeps_increasing() {
    let mut s = Slow::default();
    s.observe(300_000_000, [0; 4]);
    assert_eq!(s.observe(400_000_000, [0; 4]), None);
    assert_eq!(s.slow, 2);
}
#[test]
fn largest_timer_label_is_host_rm() {
    assert_eq!(
        Slow::default().observe(300_000_000, [1, 10, 2, 3]),
        Some("host_rm")
    );
}
#[test]
fn largest_timer_label_is_walk() {
    assert_eq!(
        Slow::default().observe(300_000_000, [1, 2, 10, 3]),
        Some("walk")
    );
}
#[test]
fn largest_timer_label_is_invalidate() {
    assert_eq!(
        Slow::default().observe(300_000_000, [1, 2, 3, 10]),
        Some("invalidate")
    );
}
#[test]
fn a_full_ring_preserves_early_refusals_and_the_latest_failure() {
    let mut rows = vec![];
    for i in 0..10000 {
        retain(&mut rows, i);
    }
    assert_eq!(&rows[..4], &[0, 1, 2, 3]);
    assert_eq!(rows.last(), Some(&9999));
}
#[test]
fn exact_threshold_after_fast_work_still_counts_and_snapshots() {
    let mut s = Slow::default();
    s.observe(1, [0; 4]);
    assert_eq!(s.observe(200_000_000, [0, 0, 0, 5]), Some("invalidate"));
    assert_eq!(s.slow, 1);
}
#[test]
fn changing_the_dominant_timer_does_not_repeat_the_one_shot() {
    let mut s = Slow::default();
    assert!(s.observe(200_000_000, [5, 0, 0, 0]).is_some());
    assert_eq!(s.observe(300_000_000, [0, 8, 0, 0]), None);
    assert_eq!(s.slow, 2);
}
