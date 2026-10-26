//! SET_REF observes engine-idle completion; GET/IRQ are not completion receipts.
use kf_mem::gatelog::{
    Gate, Point, Recorder, WHY_VA_BUSY,
    ens4::{Exposure, Stats},
};
use std::time::Instant;
fn known(x: &mut Exposure, start: u64, pub_ns: u64, marker: u64) {
    x.gate(start + 1);
    x.pass();
    x.published(pub_ns, Some(start), true);
    x.marker(marker, pub_ns + 1);
}
#[test]
fn known_gated_entry_matches_the_later_engine_idle_marker() {
    let mut x = Exposure::new(4);
    known(&mut x, 10, 100, 1);
    x.observed(1, 110);
    assert_eq!(x.stats().known_ns, Some(100));
    assert_eq!(x.stats().completed, 1);
}
#[test]
fn absent_completion_stays_unavailable_not_zero() {
    let mut x = Exposure::new(4);
    known(&mut x, 10, 100, 1);
    assert_eq!(x.stats().worst(), None);
    assert_eq!(x.stats().pending, 1);
    assert!(x.stats().summary().contains("complete_ns=NA"));
}
#[test]
fn a_hold_before_put_observation_is_labelled_as_an_upper_bound() {
    let mut x = Exposure::new(4);
    x.gate(10);
    x.pass();
    x.published(100, Some(90), false);
    x.marker(1, 101);
    x.observed(1, 110);
    assert_eq!(x.stats().upper_ns, Some(100));
    assert_eq!(x.stats().known_ns, None);
}
#[test]
fn previously_published_work_is_not_retroactively_gated() {
    let mut x = Exposure::new(4);
    x.published(10, Some(0), false);
    x.marker(1, 11);
    x.gate(20);
    x.observed(1, 30);
    assert_eq!(x.stats().worst(), None);
}
#[test]
fn an_old_marker_cannot_complete_a_new_post_hold_publication() {
    let mut x = Exposure::new(4);
    x.marker(1, 1);
    x.gate(10);
    x.pass();
    x.published(20, Some(15), false);
    x.marker(2, 21);
    x.observed(1, 30);
    assert_eq!(x.stats().worst(), None);
    assert_eq!(x.stats().pending, 1);
    x.observed(2, 40);
    assert_eq!(x.stats().upper_ns, Some(30));
}
#[test]
fn an_unpublished_future_marker_is_never_a_completion_receipt() {
    let mut x = Exposure::new(4);
    known(&mut x, 10, 20, 1);
    x.observed(99, 30);
    assert_eq!(x.stats().worst(), None);
    assert!(x.stats().lost > 0);
}
#[test]
fn stale_and_duplicate_observations_do_not_inflate_the_maximum() {
    let mut x = Exposure::new(4);
    known(&mut x, 10, 20, 1);
    x.observed(1, 30);
    x.observed(0, 100);
    x.observed(1, 100);
    assert_eq!(x.stats().known_ns, Some(20));
    assert_eq!(x.stats().completed, 1);
}
#[test]
fn marker_publication_before_entry_cannot_cover_that_entry() {
    let mut x = Exposure::new(4);
    x.marker(1, 0);
    x.gate(10);
    x.pass();
    x.published(20, Some(0), true);
    x.observed(1, 30);
    assert_eq!(x.stats().worst(), None);
    x.marker(2, 40);
    x.observed(2, 50);
    assert_eq!(x.stats().known_ns, Some(50));
}
#[test]
fn bounded_pending_receipts_record_loss_instead_of_growing() {
    let mut x = Exposure::new(2);
    for i in 0..3 {
        known(&mut x, i * 10, i * 10 + 2, i + 1);
    }
    assert_eq!(x.stats().pending, 2);
    assert_eq!(x.stats().lost, 1);
    x.observed(3, 100);
    assert_eq!(x.stats().completed, 2);
    assert_eq!(x.stats().known_ns, Some(90));
}
#[test]
fn retiring_a_channel_cannot_complete_its_pending_future_work() {
    let mut x = Exposure::new(4);
    known(&mut x, 10, 20, 1);
    x.retire();
    x.observed(1, 30);
    assert_eq!(x.stats().worst(), None);
    assert_eq!(x.stats().pending, 0);
    assert!(x.stats().lost > 0);
}
#[test]
fn an_idle_hold_is_not_assigned_to_work_arriving_much_later() {
    let mut x = Exposure::new(4);
    x.gate(0);
    x.pass();
    x.idle();
    x.published(1000, Some(999), false);
    x.marker(1, 1001);
    x.observed(1, 1010);
    assert_eq!(x.stats().worst(), None);
}
#[test]
fn idle_does_not_discard_a_real_publication_waiting_for_its_marker() {
    let mut x = Exposure::new(4);
    x.gate(0);
    x.pass();
    x.published(10, Some(1), true);
    x.idle();
    x.marker(1, 11);
    x.observed(1, 20);
    assert_eq!(x.stats().known_ns, Some(19));
}
#[test]
fn multiple_held_entries_use_the_oldest_announcement_in_one_batch() {
    let mut x = Exposure::new(4);
    x.gate(10);
    x.pass();
    x.published(20, Some(1), true);
    x.published(21, Some(5), true);
    x.marker(1, 22);
    x.observed(1, 30);
    assert_eq!(x.stats().known_ns, Some(29));
    assert_eq!(x.stats().completed, 1);
}
#[test]
fn overlapping_apply_inside_hold_is_not_added_a_second_time() {
    let mut x = Exposure::new(4);
    x.gate(10);
    x.pass();
    x.published(87, Some(10), true);
    x.marker(1, 88);
    x.observed(1, 90);
    assert_eq!(x.stats().known_ns, Some(80));
}
#[test]
fn folding_completed_channels_preserves_the_boot_maximum() {
    let mut total = Stats::default();
    total.fold(Stats {
        known_ns: Some(100),
        completed: 1,
        ..Default::default()
    });
    total.fold(Stats {
        upper_ns: Some(200),
        completed: 1,
        ..Default::default()
    });
    assert_eq!(total.worst(), Some(200));
    assert_eq!(total.completed, 2);
}
fn recorder() -> Recorder {
    let r = Recorder::with_clock(Instant::now(), 123);
    assert!(r.chan_birth(1, 1, 2, 3, 4, 64));
    r
}
fn recorded(r: &Recorder) {
    let c = r.chan_ref(1).unwrap();
    let mut c = c.lock().unwrap();
    c.put_seen(10, 1);
    c.gate(
        11,
        Gate {
            reason: WHY_VA_BUSY,
            point: Point::Publish,
            space: 4,
            walk: 1,
        },
    );
    c.pass(20, 1, &Default::default(), None);
    c.published(21);
    drop(c);
    r.ens4_turn(1, 1, 0, 0, 22);
    r.ens4_turn(1, 1, 1, 30, 31);
}
#[test]
fn production_chanlog_and_recorder_feed_the_existing_status_endpoint() {
    let r = recorder();
    recorded(&r);
    assert!(
        r.ens4_summary()
            .contains("ens4_worst_gate_apply_complete_ns=20")
    );
}
#[test]
fn free_and_native_token_reuse_preserve_the_per_boot_completed_maximum() {
    let r = recorder();
    recorded(&r);
    r.chan_free(32, 2, 3);
    assert!(r.chan_birth(33, 1, 5, 6, 7, 64));
    assert!(
        r.ens4_summary()
            .contains("ens4_worst_gate_apply_complete_ns=20")
    );
}
