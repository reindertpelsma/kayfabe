//! Runtime file tests use an explicitly marked diagnostic OS clock.
#![cfg(feature = "prof-inject")]
use kf_prof::{
    runtime::Session,
    tables::{Kind, Site, State},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn directory() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "kf-prof-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&p).unwrap();
    p
}
#[test]
fn mmap_jsonl_and_completed_span_survive_shutdown() {
    let p = directory();
    let s = Session::create_diagnostic(&p).unwrap();
    let binding = s.bind_current(1).unwrap();
    {
        let _g = kf_prof::site(Site::HostRmIoctl, State::HostIoctl, 19);
        std::thread::sleep(Duration::from_millis(2));
    }
    let row = s.snapshot();
    let thread = &row["threads"][0];
    assert_eq!(thread["states"][0]["state"], State::HostIoctl as u16);
    assert_eq!(thread["states"][0]["count"], 1);
    assert!(thread["states"][0]["max_ns"].as_u64().unwrap() >= 1_000_000);
    drop(binding);
    s.stop().unwrap();
    let b = std::fs::read(p.join("prof.mmap")).unwrap();
    assert_eq!(b.len(), kf_prof::layout::FILE_BYTES);
    let off = kf_prof::layout::RECORD_BASE;
    let seq = u64::from_le_bytes(b[off..off + 8].try_into().unwrap());
    assert!(seq > 0 && seq % 2 == 0);
    assert_eq!(u64::from_le_bytes(b[40..48].try_into().unwrap()), 1);
    assert!(kf_prof::layout::Header::decode(&b).is_ok());
    assert!(u64::from_le_bytes(b[56..64].try_into().unwrap()) > 0);
    let lines = std::fs::read_to_string(p.join("prof.jsonl")).unwrap();
    assert!(lines.contains("\"schema\":1"));
    std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn live_site_and_since_are_visible_before_a_span_finishes() {
    let p = directory();
    let s = Session::create_diagnostic(&p).unwrap();
    std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        let worker_session = &s;
        scope.spawn(move || {
            let _binding = worker_session.bind_current(2).unwrap();
            let _g = kf_prof::site(Site::BrokerReceive, State::Receive, 7);
            tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(40));
        });
        rx.recv().unwrap();
        let row = s.snapshot();
        assert_eq!(row["threads"][0]["site"], Site::BrokerReceive as u16);
        assert!(row["threads"][0]["since_ns"].as_u64().unwrap() > 0);
    });
    s.stop().unwrap();
    std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn ledger_pending_is_not_closed_by_a_snapshot() {
    let p = directory();
    let s = Session::create_diagnostic(&p).unwrap();
    let b = s.bind_current(1).unwrap();
    let t = kf_prof::ledger_open(Kind::HeldReply, 42).unwrap();
    std::thread::sleep(Duration::from_millis(14));
    let row = s.snapshot();
    assert_eq!(row["ledger"]["oldest"][0]["id"], 42);
    assert_eq!(row["ledger"]["oldest"][0]["overdue"], true);
    assert!(kf_prof::ledger_close(t));
    assert!(!kf_prof::ledger_close(t));
    assert_eq!(
        s.snapshot()["ledger"]["oldest"].as_array().unwrap().len(),
        0
    );
    drop(b);
    s.stop().unwrap();
    std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn runtime_refuses_to_replace_existing_measurements() {
    let p = directory();
    std::fs::write(p.join("prof.mmap"), b"keep").unwrap();
    assert!(Session::create_diagnostic(&p).is_err());
    assert_eq!(std::fs::read(p.join("prof.mmap")).unwrap(), b"keep");
    std::fs::remove_dir_all(p).unwrap();
}

#[test]
fn live_mmap_state_is_published_before_completion() {
    let p = directory();
    let s = Session::create_diagnostic(&p).unwrap();
    let b = s.bind_current(1).unwrap();
    let g = kf_prof::site(Site::WorkerPark, State::EventWait, 55);
    let bytes = std::fs::read(p.join("prof.mmap")).unwrap();
    let base = kf_prof::layout::THREAD_BASE;
    let word = |off| u64::from_le_bytes(bytes[base + off..base + off + 8].try_into().unwrap());
    assert!(word(0) > 0);
    assert_eq!(word(0) % 2, 0);
    assert_eq!(word(24), State::EventWait as u64);
    assert_eq!(word(48), Site::WorkerPark as u64);
    drop(g);
    drop(b);
    s.stop().unwrap();
    std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn final_mmap_record_and_jsonl_have_matching_committed_endpoints() {
    let p = directory();
    let s = Session::create_diagnostic(&p).unwrap();
    s.stop().unwrap();
    let b = std::fs::read(p.join("prof.mmap")).unwrap();
    assert_eq!(u64::from_le_bytes(b[40..48].try_into().unwrap()), 1);
    let base = kf_prof::layout::RECORD_BASE;
    let word = |off| u64::from_le_bytes(b[base + off..base + off + 8].try_into().unwrap());
    assert!(word(0) > 0);
    assert_eq!(word(0) % 2, 0);
    let lines = std::fs::read_to_string(p.join("prof.jsonl")).unwrap();
    let row: serde_json::Value = serde_json::from_str(lines.lines().last().unwrap()).unwrap();
    assert_eq!(row["end_ns"].as_u64(), Some(word(16)));
    let payload: serde_json::Value =
        serde_json::from_slice(&b[base + 64..base + 64 + word(24) as usize]).unwrap();
    assert_eq!(payload, row);
    std::fs::remove_dir_all(p).unwrap();
}

#[test]
fn production_clock_gate_does_not_use_the_diagnostic_fixture() {
    if kf_linux_raw::prof_clock::counter_available() {
        return;
    }
    let p = directory();
    assert!(Session::create(&p).is_err());
    assert_eq!(std::fs::read_dir(&p).unwrap().count(), 0);
    std::fs::remove_dir_all(p).unwrap();
}

#[test]
fn completed_spans_reconcile_histogram_and_site_counts() {
    let p = directory();
    let s = Session::create_diagnostic(&p).unwrap();
    let b = s.bind_current(1).unwrap();
    for arg in [7, 8] {
        let _g = kf_prof::site(Site::WorkerPark, State::Mutex, arg);
        std::thread::sleep(Duration::from_millis(1));
    }
    let row = s.snapshot();
    let state = row["threads"][0]["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["state"] == State::Mutex as u16)
        .unwrap();
    assert_eq!(state["count"], 2);
    assert_eq!(
        state["histogram"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap())
            .sum::<u64>(),
        2
    );
    assert_eq!(row["threads"][0]["sites"][0]["count"], 2);
    assert!(state["total_ns"].as_u64().unwrap() >= 2_000_000);
    drop(b);
    s.stop().unwrap();
    std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn ledger_slot_reuse_keeps_only_the_new_lifetime_open() {
    let p = directory();
    let s = Session::create_diagnostic(&p).unwrap();
    let b = s.bind_current(1).unwrap();
    let old = kf_prof::ledger_open(Kind::ForwardedRm, 4).unwrap();
    assert!(kf_prof::ledger_close(old));
    let new = kf_prof::ledger_open(Kind::ForwardedRm, 5).unwrap();
    assert!(!kf_prof::ledger_close(old));
    let row = s.snapshot();
    assert_eq!(row["ledger"]["oldest"].as_array().unwrap().len(), 1);
    assert_eq!(row["ledger"]["oldest"][0]["id"], 5);
    assert!(kf_prof::ledger_close(new));
    drop(b);
    s.stop().unwrap();
    std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn unavailable_production_clock_reports_its_refusal_explicitly() {
    if kf_linux_raw::prof_clock::counter_available() {
        return;
    }
    let p = directory();
    let e = Session::create(&p)
        .err()
        .expect("unsupported production clock must refuse");
    assert!(e.to_string().contains("invariant TSC unavailable"));
    std::fs::remove_dir_all(p).unwrap();
}

#[test]
fn unfinished_state_residence_is_exact_without_sampling_estimates() {
    let p = directory();
    let s = Session::create_diagnostic(&p).unwrap();
    let b = s.bind_current(1).unwrap();
    let g = kf_prof::site(Site::BrokerReceive, State::Receive, 3);
    std::thread::sleep(Duration::from_millis(2));
    let row = s.snapshot();
    let t = &row["threads"][0];
    let state = t["states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["state"] == State::Receive as u16)
        .unwrap();
    assert_eq!(
        state["exclusive_ns"].as_u64().unwrap(),
        t["observed_ns"].as_u64().unwrap() - t["since_ns"].as_u64().unwrap()
    );
    assert_eq!(state["count"], 0);
    drop(g);
    drop(b);
    s.stop().unwrap();
    std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn resumed_parent_residence_excludes_the_completed_child() {
    let p = directory();
    let s = Session::create_diagnostic(&p).unwrap();
    let b = s.bind_current(1).unwrap();
    let parent = kf_prof::site(Site::TrapDispatch, State::Running, 3);
    {
        let _child = kf_prof::site(Site::WorkerPark, State::EventWait, 4);
        std::thread::sleep(Duration::from_millis(2));
    }
    let row = s.snapshot();
    let t = &row["threads"][0];
    let states = t["states"].as_array().unwrap();
    let running = states
        .iter()
        .find(|x| x["state"] == State::Running as u16)
        .unwrap();
    let child = states
        .iter()
        .find(|x| x["state"] == State::EventWait as u16)
        .unwrap();
    assert_eq!(
        running["exclusive_ns"].as_u64().unwrap(),
        row["end_ns"].as_u64().unwrap()
            - t["since_ns"].as_u64().unwrap()
            - child["total_ns"].as_u64().unwrap()
    );
    drop(parent);
    drop(b);
    s.stop().unwrap();
    std::fs::remove_dir_all(p).unwrap();
}

#[test]
fn concurrent_thread_snapshots_keep_histogram_counts_coherent() {
    let p=directory();let s=Session::create_diagnostic(&p).unwrap();
    std::thread::scope(|scope| {let session=&s;let child=scope.spawn(move || {let _b=session.bind_current(1).unwrap();for i in 0..1000 {let _g=kf_prof::site(Site::WorkerPark,State::Mutex,i);}});
        for _ in 0..100 {let row=s.snapshot();for t in row["threads"].as_array().unwrap() {
            assert!(t["observed_ns"].as_u64().unwrap()<=row["end_ns"].as_u64().unwrap());
            for h in t["states"].as_array().unwrap() {assert_eq!(h["histogram"].as_array().unwrap().iter().map(|n|n.as_u64().unwrap()).sum::<u64>(),h["count"].as_u64().unwrap());}
        }}child.join().unwrap();
    });s.stop().unwrap();std::fs::remove_dir_all(p).unwrap();
}

fn paused_publication(s:&Session) {
    let deadline=std::time::Instant::now()+Duration::from_secs(2);
    while !s.publication_paused() {assert!(std::time::Instant::now()<deadline,"bounded publication fixture");std::thread::yield_now();}
}
#[test]
fn publishing_thread_is_not_sampled_halfway_through_a_histogram_update() {
    let p=directory();let s=Session::create_diagnostic(&p).unwrap();s.inject_publication_delay_ms(200).unwrap();
    std::thread::scope(|scope| {let session=&s;scope.spawn(move || {let _b=session.bind_current(1).unwrap();let _g=kf_prof::site(Site::WorkerPark,State::Mutex,1);});
        paused_publication(&s);assert!(s.snapshot()["threads"].as_array().unwrap().is_empty());
    });s.stop().unwrap();std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn mapped_thread_sequence_stays_odd_until_the_whole_update_is_committed() {
    let p=directory();let s=Session::create_diagnostic(&p).unwrap();s.inject_publication_delay_ms(200).unwrap();
    std::thread::scope(|scope| {let session=&s;scope.spawn(move || {let _b=session.bind_current(1).unwrap();let _g=kf_prof::site(Site::WorkerPark,State::Mutex,1);});
        paused_publication(&s);let bytes=std::fs::read(p.join("prof.mmap")).unwrap();let base=kf_prof::layout::THREAD_BASE;
        assert_eq!(u64::from_le_bytes(bytes[base..base+8].try_into().unwrap())%2,1);
    });s.stop().unwrap();std::fs::remove_dir_all(p).unwrap();
}
