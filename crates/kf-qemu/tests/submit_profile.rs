//! GPU-free controls for direct USER-doorbell/native-submit measurements.
#[path = "../src/prof.rs"]
mod prof;
#[path = "../src/submitprof.rs"]
mod submitprof;

use submitprof::{Kind, SubmitProfile};

fn submit(p: &SubmitProfile, idx: u32, kind: Kind, end: u64, ok: bool) {
    let ticket = p.begin(idx, kind);
    p.finish(ticket, end, ok);
}

#[test]
fn pending_doorbell_measures_actual_native_completion() {
    let p = SubmitProfile::new(true, 2);
    p.doorbell(0, 1_000);
    submit(&p, 0, Kind::Translated, 11_000, true);
    let s = p.snapshot(Kind::Translated);
    assert_eq!((s.calls, s.paired, s.unpaired), (1, 1, 0));
    assert!(s.latency.contains("avg_us=10.0"), "{}", s.latency);
}

#[test]
fn coalescing_retains_first_pending_stamp() {
    let p = SubmitProfile::new(true, 2);
    p.doorbell(0, 1_000);
    p.doorbell(0, 8_000);
    submit(&p, 0, Kind::UserRelay, 11_000, true);
    let s = p.snapshot(Kind::UserRelay);
    assert_eq!(p.coalesced(), 1);
    assert!(s.latency.contains("avg_us=10.0"));
}

#[test]
fn a_doorbell_is_not_reused_for_multiple_native_submits() {
    let p = SubmitProfile::new(true, 1);
    p.doorbell(0, 1_000);
    submit(&p, 0, Kind::UserRelay, 11_000, true);
    submit(&p, 0, Kind::UserRelay, 21_000, true);
    let s = p.snapshot(Kind::UserRelay);
    assert_eq!((s.calls, s.paired, s.unpaired), (2, 1, 1));
}

#[test]
fn no_doorbell_is_explicitly_unpaired_not_a_zero_latency_sample() {
    let p = SubmitProfile::new(true, 1);
    submit(&p, 0, Kind::Passthrough, 9_000, true);
    let s = p.snapshot(Kind::Passthrough);
    assert_eq!((s.paired, s.unpaired), (0, 1));
    assert!(s.latency.starts_with("n=0 "));
}

#[test]
fn failed_native_calls_never_become_success_latency() {
    let p = SubmitProfile::new(true, 1);
    p.doorbell(0, 1_000);
    submit(&p, 0, Kind::Passthrough, 9_000, false);
    let s = p.snapshot(Kind::Passthrough);
    assert_eq!((s.calls, s.failed, s.paired), (1, 1, 0));
}

#[test]
fn route_histograms_are_independent() {
    let p = SubmitProfile::new(true, 3);
    for (idx, kind) in [Kind::Translated, Kind::UserRelay, Kind::Passthrough]
        .into_iter()
        .enumerate()
    {
        p.doorbell(idx as u32, 1_000);
        submit(&p, idx as u32, kind, 11_000, true);
        assert_eq!(p.snapshot(kind).paired, 1);
    }
    assert_eq!(p.snapshot(Kind::Translated).calls, 1);
}

#[test]
fn disabled_profile_records_nothing_and_allocates_no_token_slots() {
    let p = SubmitProfile::new(false, 8);
    p.doorbell(0, 1_000);
    submit(&p, 0, Kind::UserRelay, 9_000, true);
    assert_eq!(p.slot_count(), 0);
    assert_eq!(p.snapshot(Kind::UserRelay).calls, 0);
}

#[test]
fn token_reuse_discards_stamps_from_the_previous_birth() {
    let p = SubmitProfile::new(true, 1);
    p.doorbell(0, 1_000);
    p.reset(0, 5_000);
    // A late old-life store is also rejected by its timestamp.
    p.doorbell(0, 2_000);
    submit(&p, 0, Kind::UserRelay, 9_000, true);
    assert_eq!(p.snapshot(Kind::UserRelay).paired, 0);
    p.doorbell(0, 10_000);
    submit(&p, 0, Kind::UserRelay, 12_000, true);
    assert_eq!(p.snapshot(Kind::UserRelay).paired, 1);
}

#[test]
fn reset_during_a_native_call_discards_its_old_life_ticket() {
    let p = SubmitProfile::new(true, 1);
    p.doorbell(0, 1_000);
    let ticket = p.begin(0, Kind::Translated);
    p.reset(0, 5_000);
    p.finish(ticket, 9_000, true);
    let s = p.snapshot(Kind::Translated);
    assert_eq!((s.paired, s.discarded), (0, 1));
}

#[test]
fn a_doorbell_during_a_native_call_belongs_to_the_next_call() {
    let p = SubmitProfile::new(true, 1);
    p.doorbell(0, 1_000);
    let first = p.begin(0, Kind::Passthrough);
    p.doorbell(0, 8_000);
    p.finish(first, 9_000, true);
    submit(&p, 0, Kind::Passthrough, 10_000, true);
    assert_eq!(p.snapshot(Kind::Passthrough).paired, 2);
}

#[test]
fn invalid_indices_and_backwards_clocks_do_not_create_samples() {
    let p = SubmitProfile::new(true, 1);
    p.doorbell(8, 1_000);
    submit(&p, 8, Kind::UserRelay, 9_000, true);
    p.doorbell(0, 10_000);
    submit(&p, 0, Kind::UserRelay, 9_000, true);
    let s = p.snapshot(Kind::UserRelay);
    assert_eq!(s.paired, 0);
    assert_eq!(s.discarded, 2);
}
