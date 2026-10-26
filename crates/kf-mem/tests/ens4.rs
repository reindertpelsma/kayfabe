//! ENS4 operator bounds and conditional latency endpoint; GPU-free virtual time.
use kf_mem::{
    applydelay::Config,
    latency_gate::{Bound, model},
};
#[test]
fn default_off_even_when_threshold_is_supplied() {
    assert_eq!(
        Config::parse(None, Some("1")).unwrap(),
        Config { ms: 0, min_runs: 1 }
    );
}
#[test]
fn delay_applies_only_to_large_attempted_diffs() {
    let c = Config::parse(Some("500"), None).unwrap();
    assert_eq!(c.for_runs(999), 0);
    assert_eq!(c.for_runs(1000), 500);
    assert_eq!(c.for_runs(4174), 500);
}
#[test]
fn upper_delay_boundary_is_inclusive_and_cannot_wrap() {
    assert_eq!(
        Config::parse(Some("5000"), None).unwrap().for_runs(4174),
        5000
    );
    for value in ["5001", "18446744073709551616", "-1", "+1", "1ms", ""] {
        assert!(Config::parse(Some(value), None).is_err(), "{value}");
    }
}
#[test]
fn threshold_is_bounded_and_never_zero() {
    for value in ["0", "1000001", "-1", ""] {
        assert!(Config::parse(Some("500"), Some(value)).is_err(), "{value}");
    }
}
#[test]
fn custom_threshold_has_no_guest_dependent_duration() {
    let c = Config::parse(Some("3000"), Some("4174")).unwrap();
    assert_eq!(c.for_runs(4173), 0);
    assert_eq!(c.for_runs(4174), 3000);
    assert_eq!(c.for_runs(usize::MAX), 3000);
}
#[test]
fn sleeper_runs_on_callers_thread_once_per_large_apply() {
    let c = Config::parse(Some("1000"), None).unwrap();
    let id = std::thread::current().id();
    let mut calls = Vec::new();
    c.run(4174, |ms| {
        assert_eq!(std::thread::current().id(), id);
        calls.push(ms);
    });
    c.run(999, |ms| calls.push(ms));
    assert_eq!(calls, vec![1000]);
}
#[test]
fn zero_delay_never_calls_the_sleeper() {
    Config::parse(Some("0"), None)
        .unwrap()
        .run(100000, |_| panic!("off"));
}
fn baseline() -> Bound {
    Bound::calibrate(&vec![100_000; 10000]).unwrap()
}
#[test]
fn clean_calibration_freezes_four_times_nearest_rank_p999() {
    let mut samples = vec![100; 10000];
    samples[9990..].fill(200);
    let b = Bound::calibrate(&samples).unwrap();
    assert_eq!(b.p999_ns, 100);
    assert_eq!(b.limit_ns, 400);
    assert!(b.passes(Some(400)));
    assert!(!b.passes(Some(401)));
}
#[test]
fn calibration_rejects_invalid_or_insufficient_samples() {
    for samples in [vec![], vec![1; 9999], vec![0; 10000]] {
        assert!(Bound::calibrate(&samples).is_err());
    }
}
#[test]
fn no_completion_is_a_failure_even_if_get_or_irq_progresses() {
    assert!(!baseline().passes(None));
}
#[test]
fn clean_no_hold_control_passes() {
    let t = model(1_000_000, 0, 0, 100_000);
    assert_eq!(t.published, 1_000_000);
    assert!(baseline().passes(Some(t.completed - t.announced)));
}
#[test]
fn retained_77ms_apply_causes_a_deliberate_conditional_latency_failure() {
    let t = model(1_000_000, 0, 77_000_000, 100_000);
    assert_eq!(t.published, 77_000_000);
    assert!(t.apply_start >= t.gate_start && t.published >= t.apply_end);
    assert!(!baseline().passes(Some(t.completed - t.announced)));
}
#[test]
fn injection_can_fail_a_previously_clean_virtual_time_control() {
    let b = baseline();
    let clean = model(1_000_000, 0, 500_000, 100_000);
    let injected = model(1_000_000, 0, 500_500_000, 100_000);
    assert!(b.passes(Some(clean.completed - clean.announced)));
    assert!(!b.passes(Some(injected.completed - injected.announced)));
}
#[test]
fn a_packet_published_before_the_apply_is_not_delayed_retroactively() {
    let t = model(0, 100_000_000, 3_000_000_000, 100_000);
    assert_eq!(t.published, 0);
    assert_eq!(t.completed, 100_000);
}
#[test]
fn overlapping_gate_and_apply_are_counted_once_not_summed() {
    let t = model(10, 10, 77, 3);
    assert_eq!(t.completed - t.announced, 80);
}

#[test]
fn in_flight_dump_has_requested_injection_before_the_sleeper_finishes() {
    let r = kf_mem::gatelog::Recorder::with_clock(std::time::Instant::now(), 123);
    r.ens3_begin(1, 1, &[]);
    r.ens4_delay_requested(1, 1, 3000);
    let text = r.snapshot_153(Default::default(), 100).join("\n");
    assert!(text.contains("injected_ms=3000"));
    assert!(text.contains("finished=false"));
}
#[test]
fn finished_dump_keeps_actual_injection_inside_whole_apply_wall_time() {
    let r = kf_mem::gatelog::Recorder::with_clock(std::time::Instant::now(), 123);
    r.ens3_begin(1, 1, &[]);
    r.ens4_delay_requested(1, 1, 500);
    r.ens3_end(
        1,
        1,
        kf_mem::gatelog::ens3::Timing {
            wall_ns: 577_000_000,
            injected_ns: 500_000_000,
            ..Default::default()
        },
    );
    let text = r.snapshot_153(Default::default(), 600_000_000).join("\n");
    assert!(text.contains("injected_ms=500"));
    assert!(text.contains("injected_wall_ns=500000000"));
    assert!(text.contains("wall_ns=577000000"));
}
#[test]
fn a_clean_threefold_latency_passes_but_a_fivefold_latency_fails() {
    let b = baseline();
    assert!(b.passes(Some(300_000)));
    assert!(!b.passes(Some(500_000)));
}
#[test]
fn a_missing_fence_cannot_be_calibrated_or_counted_as_a_fast_packet() {
    assert!(!baseline().passes(None));
    let mut samples = vec![100_000; 10000];
    samples[123] = 0;
    assert!(Bound::calibrate(&samples).is_err());
}
#[test]
fn absent_operator_delay_cannot_inject_even_on_a_huge_apply() {
    let c = Config::parse(None, None).unwrap();
    assert_eq!(c.for_runs(usize::MAX), 0);
    c.run(usize::MAX, |_| panic!("default must remain OFF"));
}
#[test]
fn out_of_policy_doses_are_rejected_rather_than_slept_or_clamped() {
    for value in ["6000", "10000", "999999"] {
        assert!(Config::parse(Some(value), Some("4174")).is_err());
    }
}
#[test]
fn zero_or_extreme_threshold_cannot_turn_all_tiny_applies_into_treatments() {
    assert!(Config::parse(Some("500"), Some("0")).is_err());
    assert!(Config::parse(Some("500"), Some("2000000")).is_err());
}
#[test]
fn every_qualifying_apply_sleeps_once_without_doubling_or_background_workers() {
    let c = Config::parse(Some("500"), None).unwrap();
    let mut calls = Vec::new();
    for runs in [4174, 20, 1000, 8000] {
        c.run(runs, |ms| calls.push(ms));
    }
    assert_eq!(calls, vec![500, 500, 500]);
}
#[test]
fn sparse_tail_just_over_point_one_percent_controls_the_quantile() {
    let mut samples = vec![100; 10000];
    samples[9989..].fill(200);
    let b = Bound::calibrate(&samples).unwrap();
    assert_eq!(b.p999_ns, 200);
    assert_eq!(b.limit_ns, 800);
}
#[test]
fn a_larger_population_keeps_the_same_nearest_rank_tail_rule() {
    let mut samples = vec![100; 20000];
    samples[19979..].fill(500);
    assert_eq!(Bound::calibrate(&samples).unwrap().p999_ns, 500);
}
#[test]
fn the_frozen_limit_includes_exact_equality() {
    assert!(baseline().passes(Some(400_000)));
}
#[test]
fn too_few_clean_samples_cannot_produce_a_plausible_bound() {
    assert!(Bound::calibrate(&vec![100; 9999]).is_err());
    assert!(Bound::calibrate(&vec![u64::MAX; 10000]).is_err());
}
#[test]
fn dump_records_actual_sleep_wall_time_not_only_the_requested_duration() {
    let r = kf_mem::gatelog::Recorder::with_clock(std::time::Instant::now(), 123);
    r.ens3_begin(1, 1, &[]);
    r.ens4_delay_requested(1, 1, 500);
    r.ens3_end(
        1,
        1,
        kf_mem::gatelog::ens3::Timing {
            wall_ns: 600_000_000,
            injected_ns: 500_123_000,
            ..Default::default()
        },
    );
    let text = r.snapshot_153(Default::default(), 600_000_000).join("\n");
    assert!(text.contains("injected_wall_ns=500123000"));
}
#[test]
fn a_later_apply_does_not_pause_an_already_executing_packet() {
    let t = model(0, 100_000, 77_000_000, 1_000_000);
    assert_eq!(t.published, 0);
    assert_eq!(t.completed, 1_000_000);
}
#[test]
fn plus_prefixed_delay_is_not_strict_unsigned_decimal_configuration() {
    assert!(Config::parse(Some("+500"), None).is_err());
}
