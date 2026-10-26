//! ENS3 observable contracts, GPU-free.
use kf_mem::apply::DiffRun;
use kf_mem::gatelog::ens3::{self, Poller, Progress, Selection, Timing};
use kf_mem::gatelog::{Gate, Point, Recorder, WHY_VA_BUSY};
use std::time::{Duration, Instant};

fn fixture() -> Recorder {
    let r = Recorder::with_clock(Instant::now(), 1234);
    assert!(r.chan_birth(1, 0x10067, 1, 2, 0x100000003, 64));
    assert!(r.chan_birth(2, 0x10068, 1, 4, 0x100000005, 64));
    r
}
fn runs() -> Vec<DiffRun> {
    vec![
        DiffRun {
            va: 0x20000,
            len: 0x1000,
            at: 0,
            ap: 0,
            kind: 0,
            held: false,
            perm: kf_host::MapPerm::READ_WRITE,
            privileged: false,
            leaf: 4096,
            unmap: true,
        },
        DiffRun {
            va: 0x30000,
            len: 0x1000,
            at: 0x50000,
            ap: 0,
            kind: 0,
            held: false,
            perm: kf_host::MapPerm::READ_WRITE,
            privileged: false,
            leaf: 4096,
            unmap: false,
        },
    ]
}
fn text(r: &Recorder, s: Selection) -> String {
    r.snapshot_153(s, 100).join("\n")
}

#[test]
fn no_va_trigger_accepts_unidentified_first_153() {
    assert_eq!(ens3::parse(b"event=153"), Some(Selection::default()));
}
#[test]
fn selected_trigger_preserves_exact_space_and_channel() {
    assert_eq!(
        ens3::parse(b"event=153 chid=0x67 space=0x100000003"),
        Some(Selection {
            chid: Some(0x67),
            space: Some(0x100000003)
        })
    );
}
#[test]
fn non_153_text_never_becomes_a_snapshot() {
    assert_eq!(ens3::parse(b"event=153 event=154"), None);
    assert_eq!(ens3::parse(b"event=4101"), None);
}
#[test]
fn malformed_and_oversized_triggers_are_rejected() {
    for s in [
        "event=154",
        "event=153 va=0x1",
        "event=153 chid=0x10000",
        "event=153 chid=0x67 chid=0x68",
        "event=153 space=0x10000000000000000",
    ] {
        assert!(ens3::parse(s.as_bytes()).is_none(), "{s}");
    }
    assert!(ens3::parse(&vec![b' '; ens3::TRIGGER_BYTES + 1]).is_none());
}
#[test]
fn unselected_snapshot_keeps_both_rings_without_xid() {
    let r = fixture();
    let s = text(&r, Selection::default());
    assert!(s.contains("token=0x10067"));
    assert!(s.contains("token=0x10068"));
    assert!(s.contains("t0_unix_ms=1234"));
}
#[test]
fn selector_does_not_leak_unrelated_channel_or_space() {
    let r = fixture();
    let s = text(
        &r,
        Selection {
            chid: Some(0x67),
            space: Some(0x100000003),
        },
    );
    assert!(s.contains("token=0x10067"));
    assert!(!s.contains("token=0x10068"));
    assert!(!s.contains("space=0x100000005"));
}
#[test]
fn space_only_selects_its_rings_and_applies() {
    let r = fixture();
    r.ens3_begin(0x100000005, 77, &runs());
    let s = text(
        &r,
        Selection {
            chid: None,
            space: Some(0x100000003),
        },
    );
    assert!(s.contains("token=0x10067"));
    assert!(!s.contains("token=0x10068"));
    assert!(!s.contains("walk=77"));
}
#[test]
fn snapshot_includes_map_unmap_and_apply_phase_costs() {
    let r = fixture();
    r.ens3_begin(0x100000003, 7, &runs());
    r.ens3_end(
        0x100000003,
        7,
        Timing {
            wall_ns: 1000,
            walk_ns: 2000,
            gpu_us: 1,
            ..Timing::default()
        },
    );
    let s = text(&r, Selection::default());
    assert!(s.contains("UNMAP va=0x20000"));
    assert!(s.contains("MAP va=0x30000"));
    assert!(s.contains("wall_ns=1000"));
    assert!(s.contains("walk_ns=2000"));
}
#[test]
fn in_flight_apply_is_visible_before_it_finishes() {
    let r = fixture();
    r.ens3_begin(0x100000003, 8, &runs());
    let s = text(&r, Selection::default());
    assert!(s.contains("walk=8"));
    assert!(s.contains("finished=false"));
}
#[test]
fn newest_publication_and_observed_marker_survive_ring_wrap() {
    let r = fixture();
    r.ens3_progress(
        0x10067,
        Progress {
            publish_ns: 50,
            put: 123,
            marker: 99,
            observe_ns: 60,
            get: 7,
            reference: 0xabcd1234,
        },
    );
    let c = r.chan_ref(0x10067).unwrap();
    for i in 1..5000 {
        c.lock().unwrap().put_seen(i, i as u32 % 64);
    }
    let s = text(&r, Selection::default());
    assert!(s.contains("put=123 marker=99"));
    assert!(s.contains("reference=0xabcd1234"));
}
#[test]
fn latest_publication_is_distinct_from_last_hardware_observation() {
    let r = fixture();
    r.ens3_progress(
        0x10067,
        Progress {
            publish_ns: 20,
            put: 44,
            marker: 31,
            observe_ns: 10,
            get: 2,
            reference: 0xabcd0001,
        },
    );
    let s = text(&r, Selection::default());
    assert!(s.contains("publish_ns=20 put=44 marker=31"));
    assert!(s.contains("observe_ns=10 get=2 reference=0xabcd0001"));
}

#[test]
fn observing_private_words_does_not_erase_last_successful_publication() {
    let r = fixture();
    r.ens3_progress(
        0x10067,
        Progress {
            publish_ns: 20,
            put: 44,
            marker: 31,
            ..Progress::default()
        },
    );
    r.ens3_progress(
        0x10067,
        Progress {
            observe_ns: 30,
            get: 8,
            reference: 0xabcd0003,
            ..Progress::default()
        },
    );
    let s = text(&r, Selection::default());
    assert!(s.contains("publish_ns=20 put=44 marker=31"));
    assert!(s.contains("observe_ns=30 get=8 reference=0xabcd0003"));
}

#[test]
fn newest_put_does_not_erase_last_hardware_reference() {
    let r = fixture();
    r.ens3_progress(
        0x10067,
        Progress {
            observe_ns: 10,
            get: 2,
            reference: 0xabcd0001,
            ..Progress::default()
        },
    );
    r.ens3_progress(
        0x10067,
        Progress {
            publish_ns: 20,
            put: 44,
            marker: 31,
            ..Progress::default()
        },
    );
    let s = text(&r, Selection::default());
    assert!(s.contains("publish_ns=20 put=44 marker=31"));
    assert!(s.contains("observe_ns=10 get=2 reference=0xabcd0001"));
}

#[test]
fn large_diff_keeps_only_bounded_run_samples_and_counts_omissions() {
    let r = fixture();
    r.ens3_begin(0x100000003, 7, &vec![runs()[0]; 1000]);
    let s = text(&r, Selection::default());
    assert!(s.contains("runs=1000 omitted=992"));
    assert_eq!(s.lines().filter(|s| s.contains("ENS3 RUN ")).count(), 8);
}

#[test]
fn every_diff_has_its_own_fixed_run_budget() {
    let r = fixture();
    for i in 1..=2 {
        r.ens3_begin(0x100000003, i, &vec![runs()[1]; 64]);
    }
    let s = text(&r, Selection::default());
    assert_eq!(s.lines().filter(|s| s.contains("ENS3 RUN ")).count(), 16);
    assert_eq!(s.matches("runs=64 omitted=56").count(), 2);
}
#[test]
fn open_gate_and_recent_gate_interval_are_visible() {
    let r = fixture();
    let c = r.chan_ref(0x10067).unwrap();
    c.lock().unwrap().gate(
        20,
        Gate {
            reason: WHY_VA_BUSY,
            point: Point::Put,
            space: 0,
            walk: 0,
        },
    );
    let s = text(&r, Selection::default());
    assert!(s.contains("gate_start"));
    assert!(s.contains("hold_age_ns=80"));
}
#[test]
fn apply_history_and_dump_are_bounded_and_report_loss() {
    let r = fixture();
    for i in 0..ens3::APPLY_N + 5 {
        r.ens3_begin(0x100000003, i as u64, &runs());
    }
    let lines = r.snapshot_153(Selection::default(), 100);
    assert!(!lines.is_empty());
    assert!(lines.len() <= ens3::LINES_MAX);
    assert!(lines.iter().all(|s| s.len() <= 512));
    assert!(lines.join("\n").contains("apply_overflow=5"));
}
#[test]
fn first_valid_trigger_is_consumed_once() {
    let p = std::env::temp_dir().join(format!("ens3-{}-first", std::process::id()));
    std::fs::write(&p, b"event=153").unwrap();
    let mut poll = Poller::default();
    let now = Instant::now();
    assert_eq!(poll.poll(&p, now), Some(Selection::default()));
    assert_eq!(poll.poll(&p, now + Duration::from_secs(1)), None);
    std::fs::remove_file(p).unwrap();
}
#[test]
fn changing_the_file_does_not_rearm_first_153() {
    let p = std::env::temp_dir().join(format!("ens3-{}-changed", std::process::id()));
    std::fs::write(&p, b"event=153").unwrap();
    let mut poll = Poller::default();
    let now = Instant::now();
    assert!(poll.poll(&p, now).is_some());
    std::fs::write(&p, b"event=153 chid=0x68").unwrap();
    assert!(poll.poll(&p, now + Duration::from_secs(1)).is_none());
    std::fs::remove_file(p).unwrap();
}
#[test]
fn oversized_file_cannot_hide_a_valid_trigger_prefix() {
    let p = std::env::temp_dir().join(format!("ens3-{}-large", std::process::id()));
    let mut b = b"event=153".to_vec();
    b.resize(ens3::TRIGGER_BYTES + 1, b' ');
    std::fs::write(&p, b).unwrap();
    let mut poll = Poller::default();
    let now = Instant::now();
    assert!(poll.poll(&p, now).is_none());
    std::fs::write(&p, b"event=153").unwrap();
    assert!(poll.poll(&p, now + Duration::from_secs(1)).is_some());
    std::fs::remove_file(p).unwrap();
}
#[test]
fn invalid_trigger_can_be_corrected_without_losing_first_snapshot() {
    let p = std::env::temp_dir().join(format!("ens3-{}-corrected", std::process::id()));
    std::fs::write(&p, b"event=154").unwrap();
    let mut poll = Poller::default();
    let now = Instant::now();
    assert_eq!(poll.poll(&p, now), None);
    std::fs::write(&p, b"event=153 chid=0x67").unwrap();
    assert!(poll.poll(&p, now + Duration::from_secs(1)).is_some());
    std::fs::remove_file(p).unwrap();
}
