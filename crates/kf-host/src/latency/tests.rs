use super::core::*;
const MS: u64 = 1_000_000;
fn sample(stream: u64, kind: Kind, marker: Option<u64>) -> Sample {
    Sample {
        stream,
        token: 7,
        kind,
        observed: 10 * MS,
        published: 20 * MS,
        marker,
        wrap: false,
        advanced: None,
        gpu: None,
        vector: Some(19),
        raised: None,
        source: None,
        waits: vec![],
        worker: 1,
        cookie: 0,
        saved: false,
        work: vec![],
        context_checked: false,
    }
}
fn complete(gpu: Option<u64>, observed: u64, raised: u64) -> Sample {
    let mut s = sample(1, Kind::User, Some(1));
    s.advanced = Some(observed);
    s.gpu = gpu;
    s.raised = Some(raised);
    s
}
#[test]
fn ens11_stage_sum_is_end_to_end_on_host_observations() {
    let s = complete(None, 30 * MS, 90 * MS);
    let stages = s
        .stages()
        .expect("the recorder must decompose native observations");
    assert_eq!(stages, [10 * MS, 10 * MS, 60 * MS]);
    assert_eq!(stages.iter().sum::<u64>(), s.raised.unwrap() - s.observed);
    assert_eq!(
        s.observation_lag(),
        None,
        "no GPU timestamp is not zero lag"
    );
}
#[test]
fn ens11_stage_sum_with_real_gpu_timestamp_retains_observation_lag() {
    let s = complete(Some(25 * MS), 80 * MS, 90 * MS);
    assert_eq!(s.stages(), Some([10 * MS, 5 * MS, 65 * MS]));
    assert_eq!(s.observation_lag(), Some(55 * MS));
}
#[test]
fn ens11_worker_held_in_rm_after_gpu_completion_is_in_marker_to_raise() {
    let s = complete(Some(25 * MS), 180 * MS, 190 * MS);
    assert_eq!(s.stages(), Some([10 * MS, 5 * MS, 165 * MS]));
    assert_eq!(s.observation_lag(), Some(155 * MS));
}
#[test]
fn ens11_worker_lock_delay_after_gpu_completion_is_not_publication_delay() {
    let s = complete(Some(25 * MS), 280 * MS, 290 * MS);
    assert_eq!(s.stages(), Some([10 * MS, 5 * MS, 265 * MS]));
}
#[test]
fn ens11_own_gate_delay_changes_only_observe_to_publish() {
    let first = complete(None, 30 * MS, 40 * MS);
    let mut held = first.clone();
    held.published += 200 * MS;
    held.advanced = Some(230 * MS);
    held.raised = Some(240 * MS);
    held.waits.push(Wait {
        cause: Cause::OwnWalk,
        start: 10 * MS,
        end: 210 * MS,
        owner: 2,
    });
    assert_eq!(first.stages(), Some([10 * MS, 10 * MS, 10 * MS]));
    assert_eq!(held.stages(), Some([210 * MS, 10 * MS, 10 * MS]));
}
#[test]
fn ens11_scan_budget_delay_changes_only_the_first_stage() {
    let mut s = complete(None, 130 * MS, 140 * MS);
    s.published = 120 * MS;
    s.waits.push(Wait {
        cause: Cause::ScanBudget,
        start: 10 * MS,
        end: 110 * MS,
        owner: 1,
    });
    assert_eq!(s.stages(), Some([110 * MS, 10 * MS, 10 * MS]));
}
#[test]
fn ens11_inverted_host_observation_is_explicit_not_clamped() {
    let s = complete(None, 80 * MS, 30 * MS);
    assert_eq!(s.stages(), None);
    let mut st = State::default();
    st.publish(sample(1, Kind::User, Some(1)));
    st.notify(19, 30 * MS, true);
    st.marker(1, 1, 80 * MS, None);
    assert_eq!(st.early, 1);
}
#[test]
fn ens11_negative_gpu_observation_lag_is_not_zero() {
    let s = complete(Some(90 * MS), 80 * MS, 100 * MS);
    assert_eq!(s.observation_lag(), None);
    assert_eq!(s.stages(), None);
}
#[test]
fn ens11_failed_notification_does_not_retire_a_user_sample() {
    let mut st = State::default();
    st.publish(sample(1, Kind::User, Some(1)));
    st.marker(1, 1, 30 * MS, None);
    st.notify(19, 90 * MS, false);
    assert_eq!(st.pending.len(), 1);
    assert_eq!(st.gauges[2].count, 0);
    st.notify(19, 100 * MS, true);
    assert!(st.pending.is_empty());
}
#[test]
fn ens11_wrong_vector_does_not_retire_translated_work() {
    let mut st = State::default();
    st.publish(sample(1, Kind::Translated, Some(1)));
    st.marker(1, 1, 30 * MS, None);
    st.notify(20, 90 * MS, true);
    assert_eq!(st.pending.len(), 1);
    st.notify(19, 100 * MS, true);
    assert!(st.pending.is_empty());
}
#[test]
fn ens11_uncovered_open_packet_cannot_gain_a_marker_receipt() {
    let mut st = State::default();
    st.publish(sample(1, Kind::User, None));
    st.marker(1, 1, 30 * MS, None);
    st.notify(19, 90 * MS, true);
    assert_eq!(st.pending.front().unwrap().advanced, None);
}
#[test]
fn ens11_covering_marker_only_covers_its_own_native_stream() {
    let mut st = State::default();
    st.publish(sample(1, Kind::User, None));
    st.publish(sample(2, Kind::User, None));
    st.cover(1, 4, false);
    st.marker(1, 4, 90 * MS, None);
    assert_eq!(st.pending[0].advanced, Some(90 * MS));
    assert_eq!(st.pending[1].advanced, None);
}
#[test]
fn ens11_new_life_and_reused_token_cannot_observe_an_old_stream() {
    let mut st = State::default();
    st.publish(sample(1, Kind::User, Some(1)));
    st.cancel(1);
    st.publish(sample(2, Kind::User, Some(1)));
    st.marker(1, 1, 90 * MS, None);
    assert_eq!(st.canceled, 1);
    assert_eq!(st.pending[0].advanced, None);
}
#[test]
fn ens11_private_wrapping_fence_receipt_survives_sequence_wrap() {
    let mut st = State::default();
    let mut s = sample(1, Kind::Private, Some(u64::from(u32::MAX)));
    s.wrap = true;
    s.vector = None;
    st.publish(s);
    st.marker(1, 0, 90 * MS, None);
    assert_eq!(st.gauges[1].count, 1);
    assert!(st.pending.is_empty());
}
#[test]
fn ens11_unreached_wrapping_fence_never_retires() {
    let mut st = State::default();
    let mut s = sample(1, Kind::Translated, Some(2));
    s.wrap = true;
    st.publish(s);
    st.marker(1, u64::from(u32::MAX), 90 * MS, None);
    assert_eq!(st.pending[0].advanced, None);
}
#[test]
fn ens11_pending_and_slow_history_are_hostile_guest_bounded() {
    let mut st = State::default();
    for stream in 0..1000 {
        st.publish(sample(stream, Kind::User, Some(1)));
    }
    assert_eq!(st.pending.len(), 512);
    assert_eq!(st.overflow, 488);
    st.expire(1000 * MS);
    assert!(st.first_dump);
    assert_eq!(st.slow.len(), 64);
}
#[test]
fn ens11_stage_counts_include_partial_samples_without_notifications() {
    let mut st = State::default();
    st.publish(sample(1, Kind::User, Some(1)));
    assert_eq!(st.gauges[0].count, 1);
    st.marker(1, 1, 90 * MS, None);
    assert_eq!(st.gauges[1].count, 1);
    assert_eq!(st.gauges[2].count, 0);
    assert_eq!(st.gauges[1].max, 70 * MS);
}
#[test]
fn ens11_missing_gpu_time_stays_unknown_on_private_ring() {
    let mut st = State::default();
    let mut s = sample(1, Kind::Private, Some(1));
    s.vector = None;
    st.publish(s);
    st.marker(1, 1, 90 * MS, None);
    assert_eq!(st.marker_unknown, 1);
    assert_eq!(st.gauges[3].count, 0);
}

#[test]
fn ens11_normal_notification_source_is_preserved_in_a_slow_packet() {
    let mut st = State::default();
    st.publish(sample(1, Kind::User, Some(1)));
    st.marker(1, 1, 30 * MS, None);
    st.notify_source(19, 90 * MS, true, NotificationSource::Normal);
    assert_eq!(
        st.slow.back().unwrap().source,
        Some(NotificationSource::Normal)
    );
}
#[test]
fn ens11_irqstorm_notification_is_identified_without_claiming_packet_causality() {
    let mut st = State::default();
    st.publish(sample(1, Kind::Translated, Some(1)));
    st.marker(1, 1, 30 * MS, None);
    st.notify_source(19, 90 * MS, true, NotificationSource::Storm);
    assert_eq!(
        st.slow.back().unwrap().source,
        Some(NotificationSource::Storm)
    );
}
