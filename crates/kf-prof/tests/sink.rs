//! Bounded file retention without replacing another writer's data.
#![cfg(feature="prof")]
use std::{path::PathBuf,sync::atomic::{AtomicU64,Ordering}};
use kf_prof::runtime::JsonSink;
static NEXT:AtomicU64=AtomicU64::new(0);
fn dir()->PathBuf {let p=std::env::temp_dir().join(format!("kf-prof-sink-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));std::fs::create_dir(&p).unwrap();p}
#[test]
fn rotation_retains_only_current_and_previous_owned_chunks() {
    let p=dir();let path=p.join("prof.jsonl");let mut s=JsonSink::create(&path,8).unwrap();
    for line in [b"1111111\n",b"2222222\n",b"3333333\n"] {s.append(line).unwrap();}
    assert_eq!(std::fs::read(&path).unwrap(),b"3333333\n");assert_eq!(std::fs::read(p.join("prof.jsonl.previous")).unwrap(),b"2222222\n");
    drop(s);assert_eq!(std::fs::read_dir(&p).unwrap().count(),2);std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn existing_previous_chunk_is_never_replaced_or_deleted() {
    let p=dir();let path=p.join("prof.jsonl");std::fs::write(p.join("prof.jsonl.previous"),b"foreign").unwrap();
    let mut s=JsonSink::create(&path,8).unwrap();s.append(b"1111111\n").unwrap();assert!(s.append(b"2222222\n").is_err());
    assert_eq!(std::fs::read(p.join("prof.jsonl.previous")).unwrap(),b"foreign");assert_eq!(std::fs::read(path).unwrap(),b"1111111\n");
    drop(s);std::fs::remove_dir_all(p).unwrap();
}
#[test]
fn zero_limit_and_oversized_record_are_refused_without_partial_emit() {
    let p=dir();let path=p.join("prof.jsonl");assert!(JsonSink::create(&path,0).is_err());
    assert!(!path.exists());let mut s=JsonSink::create(&path,8).unwrap();assert!(s.append(b"123456789").is_err());assert_eq!(std::fs::read(path).unwrap().len(),0);
    drop(s);std::fs::remove_dir_all(p).unwrap();
}
