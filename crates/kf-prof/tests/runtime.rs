#![cfg(feature = "prof")]
use kf_prof::{runtime::Session, tables::{State, Site, Kind}};
use std::{path::PathBuf, sync::atomic::{AtomicU64, Ordering}, time::Duration};
static NEXT:AtomicU64=AtomicU64::new(0);
fn directory()->PathBuf {let p=std::env::temp_dir().join(format!("kf-prof-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));std::fs::create_dir(&p).unwrap();p}
#[test]
fn mmap_jsonl_and_completed_span_survive_shutdown() {
    let p=directory();let s=Session::create(&p).unwrap();let binding=s.bind_current(1).unwrap();
    {let _g=kf_prof::site(Site::HostRmIoctl,State::HostIoctl,19);std::thread::sleep(Duration::from_millis(2));}
    let row=s.snapshot();let thread=&row["threads"][0];
    assert_eq!(thread["states"][0]["state"],State::HostIoctl as u16);
    assert_eq!(thread["states"][0]["count"],1);assert!(thread["states"][0]["max_ns"].as_u64().unwrap()>=1_000_000);
    drop(binding);s.stop().unwrap();
    let b=std::fs::read(p.join("prof.mmap")).unwrap();assert_eq!(b.len(),kf_prof::layout::FILE_BYTES);
    assert!(kf_prof::layout::Header::decode(&b).is_ok());
    assert!(u64::from_le_bytes(b[56..64].try_into().unwrap())>0);
    let lines=std::fs::read_to_string(p.join("prof.jsonl")).unwrap();assert!(lines.contains("\"schema\":1"));
    std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn live_site_and_since_are_visible_before_a_span_finishes() {
    let p=directory();let s=Session::create(&p).unwrap();
    std::thread::scope(|scope| {let (tx,rx)=std::sync::mpsc::channel();scope.spawn(|| {
        let _binding=s.bind_current(2).unwrap();let _g=kf_prof::site(Site::BrokerReceive,State::Receive,7);tx.send(()).unwrap();std::thread::sleep(Duration::from_millis(40));});
        rx.recv().unwrap();let row=s.snapshot();assert_eq!(row["threads"][0]["site"],Site::BrokerReceive as u16);
        assert!(row["threads"][0]["since_ns"].as_u64().unwrap()>0);
    });s.stop().unwrap();std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn ledger_pending_is_not_closed_by_a_snapshot() {
    let p=directory();let s=Session::create(&p).unwrap();let b=s.bind_current(1).unwrap();
    let t=kf_prof::ledger_open(Kind::HeldReply,42).unwrap();
    std::thread::sleep(Duration::from_millis(14));let row=s.snapshot();assert_eq!(row["ledger"]["oldest"][0]["id"],42);
    assert_eq!(row["ledger"]["oldest"][0]["overdue"],true);
    assert!(kf_prof::ledger_close(t));assert!(!kf_prof::ledger_close(t));assert_eq!(s.snapshot()["ledger"]["oldest"].as_array().unwrap().len(),0);
    drop(b);s.stop().unwrap();std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn runtime_refuses_to_replace_existing_measurements() {
    let p=directory();std::fs::write(p.join("prof.mmap"),b"keep").unwrap();
    assert!(Session::create(&p).is_err());assert_eq!(std::fs::read(p.join("prof.mmap")).unwrap(),b"keep");std::fs::remove_dir_all(p).unwrap();
}
