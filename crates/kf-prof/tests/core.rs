//! K1 core contracts, authored before the implementation.
use kf_prof::{clock::Calibration, histogram::Histogram, layout::*, ledger::Ledger,
    publication::{AtomicWords, ReadError, WordStorage, read_record, write_record}, ring::{Event, Ring}};
use std::sync::atomic::Ordering;
use std::sync::Mutex;

#[test]
fn header_exact_offsets_and_zero_reserved_bytes() {
    let h = Header { pid: 42, flags: 7, start_ns: 900, revision: [b'a';40] };
    let b = h.encode();
    assert_eq!(&b[..8], b"KFPROF01");
    assert_eq!(u64::from_le_bytes(b[32..40].try_into().unwrap()),42);
    assert_eq!(u64::from_le_bytes(b[48..56].try_into().unwrap()),900);
    assert_eq!(&b[128..168], &[b'a';40]);
    assert!(b[200..].iter().all(|&b| b == 0));
    assert_eq!(Header::decode(&b), Ok(h));
}
#[test]
fn layout_regions_are_bounded_aligned_and_disjoint() {
    assert_eq!(THREAD_BASE, HEADER_BYTES);
    assert_eq!(THREAD_BASE + THREADS * THREAD_BYTES, LEDGER_BASE);
    assert_eq!(LEDGER_BASE + LEDGER_BYTES, RECORD_BASE);
    assert!(RECORD_BASE + RECORDS * RECORD_BYTES <= FILE_BYTES);
    for x in [THREAD_BASE, THREAD_BYTES, LEDGER_BASE, RECORD_BASE, RECORD_BYTES] { assert_eq!(x % 64, 0); }
    assert_eq!(FILE_BYTES, 16 * 1024 * 1024);
}
#[test]
fn header_rejects_truncated_wrong_version_and_wrong_geometry() {
    let b = Header::default().encode();
    for n in 0..HEADER_BYTES { assert!(Header::decode(&b[..n]).is_err()); }
    for offset in [0,8,12,16] { let mut bad=b; bad[offset] ^= 0xff; assert!(Header::decode(&bad).is_err()); }
}
#[test]
fn header_hostile_bytes_never_panic() {
    let mut seed=17u64;
    for _ in 0..1000 {
        let mut b=[0;HEADER_BYTES];
        for c in &mut b { seed=seed.wrapping_mul(6364136223846793005).wrapping_add(1); *c=(seed>>32) as u8; }
        assert!(Header::decode(&b).is_err());
    }
}

#[derive(Debug,PartialEq)]
enum Op { Load(usize,Ordering), Store(usize,u64,Ordering), Fence(Ordering) }
struct Spy { words: AtomicWords<8>, ops: Mutex<Vec<Op>> }
impl Spy { fn new()->Self {Self { words:AtomicWords::new(),ops:Mutex::new(Vec::new()) }} }
impl WordStorage for Spy {
    fn len(&self)->usize { 8 }
    fn load(&self,i:usize,o:Ordering)->u64 { self.ops.lock().unwrap().push(Op::Load(i,o));self.words.load(i,o) }
    fn store(&self,i:usize,v:u64,o:Ordering) {self.ops.lock().unwrap().push(Op::Store(i,v,o));self.words.store(i,v,o);}
    fn fence(&self,o:Ordering) {self.ops.lock().unwrap().push(Op::Fence(o));std::sync::atomic::fence(o);}
}
#[test]
fn seqlock_writer_orders_odd_before_payload_and_release_commit() {
    let s=Spy::new(); let mut seq=0;
    write_record(&s,0,&[10,20],&mut seq).unwrap();
    assert_eq!(*s.ops.lock().unwrap(), vec![Op::Store(0,1,Ordering::Relaxed),Op::Fence(Ordering::Release),
        Op::Store(1,10,Ordering::Relaxed),Op::Store(2,20,Ordering::Relaxed),Op::Store(0,2,Ordering::Release)]);
    assert_eq!(seq,2);
}
#[test]
fn seqlock_reader_orders_acquire_validation_after_payload() {
    let s=Spy::new();let mut seq=0;write_record(&s,0,&[1,2],&mut seq).unwrap();s.ops.lock().unwrap().clear();
    assert_eq!(read_record::<2>(&s,0),Ok([1,2]));
    assert_eq!(*s.ops.lock().unwrap(),vec![Op::Load(0,Ordering::Acquire),Op::Load(1,Ordering::Relaxed),
        Op::Load(2,Ordering::Relaxed),Op::Fence(Ordering::Acquire),Op::Load(0,Ordering::Relaxed)]);
}
#[test]
fn seqlock_interrupted_writer_is_busy_without_waiting() {
    let s=AtomicWords::<8>::new();s.store(0,1,Ordering::Relaxed);
    assert_eq!(read_record::<2>(&s,0),Err(ReadError::Busy));
}
#[test]
fn seqlock_bounds_and_generation_exhaustion_make_no_stores() {
    let s=Spy::new();let mut seq=u64::MAX-1;
    assert!(write_record(&s,0,&[1],&mut seq).is_err());
    assert!(s.ops.lock().unwrap().is_empty());
    assert!(write_record(&s,usize::MAX,&[1],&mut 0).is_err());
    assert!(s.ops.lock().unwrap().is_empty());
    assert_eq!(read_record::<8>(&s,0),Err(ReadError::Bounds));
}
#[test]
fn seqlock_concurrent_snapshots_never_mix_generations() {
    let s=AtomicWords::<4>::new();
    std::thread::scope(|scope| {
        scope.spawn(|| {let mut seq=0;for i in 1..100_000u64 {write_record(&s,0,&[i,!i,i^0x1234],&mut seq).unwrap();}});
        for _ in 0..100_000 {if let Ok([x,y,z])=read_record::<3>(&s,0) {if x!=0 {assert_eq!(y,!x);assert_eq!(z,x^0x1234);}}}
    });
}

fn event(n:u64)->Event {Event {ts_ns:n,tag:1,site:2,arg:n,value:!n,scope:0,clock:1}}
#[test]
fn ring_wrap_preserves_fifo_over_many_laps() {
    let mut r=Ring::<4>::new().unwrap();let (mut w,mut rd)=r.split();
    for lap in 0..1000 {for i in 0..4 {assert!(w.push(event(lap*4+i)));} for i in 0..4 {assert_eq!(rd.pop(),Some(event(lap*4+i)));}}
    assert_eq!(rd.pop(),None);
}
#[test]
fn ring_wrapped_partial_batches_match_reference_queue() {
    use std::collections::VecDeque;
    let mut r=Ring::<8>::new().unwrap();let (mut w,mut rd)=r.split();let mut q=VecDeque::new();let mut seed=3u64;
    for i in 0..20_000 {seed=seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        if seed>>63==0 {let accepted=w.push(event(i));assert_eq!(accepted,q.len()<8);if accepted {q.push_back(event(i));}}
        else {assert_eq!(rd.pop(),q.pop_front());}}
    while let Some(e)=q.pop_front() {assert_eq!(rd.pop(),Some(e));}
}
#[test]
fn ring_overflow_is_counted_without_claiming_a_slot() {
    let mut r=Ring::<2>::new().unwrap();let (mut w,mut rd)=r.split();
    assert!(w.push(event(1)));assert!(w.push(event(2)));
    for _ in 0..3 {assert!(!w.push(event(99)));} assert_eq!(w.dropped(),3);
    assert_eq!(rd.pop(),Some(event(1)));assert!(w.push(event(3)));
    assert_eq!(rd.pop(),Some(event(2)));assert_eq!(rd.pop(),Some(event(3)));assert_eq!(rd.pop(),None);
}
#[test]
fn ring_repeated_full_laps_count_all_drops() {
    let mut r=Ring::<2>::new().unwrap();let (mut w,mut rd)=r.split();
    for lap in 0..100 {assert!(w.push(event(lap)));assert!(w.push(event(lap)));for _ in 0..7 {assert!(!w.push(event(9)));}rd.pop();rd.pop();}
    assert_eq!(w.dropped(),700);
}
#[test]
fn ring_invalid_capacity_is_rejected() {assert!(Ring::<0>::new().is_err());assert!(Ring::<3>::new().is_err());}
#[test]
fn ring_concurrent_stress_has_no_loss_among_accepted_records() {
    let mut r=Ring::<64>::new().unwrap();let (mut w,mut rd)=r.split();
    std::thread::scope(|s| {let t=s.spawn(move || {let mut accepted=Vec::new();for n in 1..50_000 {if w.push(event(n)) {accepted.push(n);}} accepted});
        let mut got=Vec::new();while !t.is_finished() {if let Some(e)=rd.pop() {assert_eq!(e.value,!e.arg);got.push(e.arg);}else {std::thread::yield_now();}}
        while let Some(e)=rd.pop() {got.push(e.arg);}assert_eq!(got,t.join().unwrap());});
}

#[test]
fn histogram_all_octave_edges_and_zero_are_exact_buckets() {
    assert_eq!(Histogram::bucket(0),0);
    for bit in 0..64 {let lo=1u64<<bit;assert_eq!(Histogram::bucket(lo),bit+1);if bit>0 {assert_eq!(Histogram::bucket(lo-1),bit);}}
    assert_eq!(Histogram::bucket(u64::MAX),64);
}
#[test]
fn histogram_conserves_samples_and_arg_of_first_max() {
    let mut h=Histogram::new();for (ns,arg) in [(0,1),(1,2),(2,3),(3,4),(3,5),(8,6)] {h.record(ns,arg);}
    assert_eq!(h.count,6);assert_eq!(h.total_ns,17);assert_eq!(h.max_ns,8);assert_eq!(h.max_arg,6);
    assert_eq!(h.buckets[0],1);assert_eq!(h.buckets[1],1);assert_eq!(h.buckets[2],3);assert_eq!(h.buckets[4],1);
    let mut tie=Histogram::new();tie.record(5,10);tie.record(5,11);assert_eq!(tie.max_arg,10);
}
#[test]
fn histogram_saturates_instead_of_wrapping() {
    let mut h=Histogram::new();h.record(u64::MAX,1);h.record(1,2);assert_eq!(h.total_ns,u64::MAX);assert!(h.saturated);
}
#[test]
fn histogram_seeded_property_matches_log2_reference() {
    let mut s=5u64;let mut h=Histogram::new();let mut buckets=[0u64;65];
    for _ in 0..5000 {s=s.wrapping_mul(6364136223846793005).wrapping_add(1);let n=s>>32;let mut q=n;let mut b=0;while q>0 {b+=1;q>>=1;}buckets[b]+=1;h.record(n,0);}
    assert_eq!(h.buckets,buckets);assert_eq!(h.buckets.iter().sum::<u64>(),h.count);
}

#[test]
fn clock_calibration_converts_frequency_and_anchor() {
    let c=Calibration::new(100,5000,3_000_000_000,1,10).unwrap();
    assert_eq!(c.to_ns(100),Ok(5000));assert_eq!(c.to_ns(3100),Ok(6000));assert_eq!(c.to_ns(3_000_000_100),Ok(1_000_005_000));
}
#[test]
fn clock_rational_frequency_and_large_integers_do_not_use_floats() {
    let c=Calibration::new(0,1,5_000_000_000,2,1).unwrap();assert_eq!(c.to_ns(25),Ok(11));
    let c=Calibration::new(0,0,1_000_000_000,1,0).unwrap();let t=(1u64<<60)+37;assert_eq!(c.to_ns(t),Ok(t));
}
#[test]
fn clock_invalid_backwards_and_overflow_are_explicit() {
    assert!(Calibration::new(0,0,0,1,0).is_err());assert!(Calibration::new(0,0,1,0,0).is_err());
    let c=Calibration::new(10,u64::MAX,1,1,0).unwrap();assert!(c.to_ns(9).is_err());assert!(c.to_ns(11).is_err());
    let c=Calibration::new(0,0,1,u64::MAX,0).unwrap();assert!(c.to_ns(u64::MAX).is_err());
}
#[test]
fn clock_pair_estimator_matches_observed_interval() {
    let c=Calibration::from_pair(100,5000,3100,6000,10).unwrap();assert_eq!(c.to_ns(1600),Ok(5500));
    assert!(Calibration::from_pair(100,5000,99,6000,0).is_err());assert!(Calibration::from_pair(100,5000,200,5000,0).is_err());
}

#[test]
fn ledger_reuse_rejects_stale_close() {
    let mut l=Ledger::<2,2>::new(1);let a=l.open(0,99,10).unwrap();assert!(l.close(a,20));
    let b=l.open(0,99,30).unwrap();assert!(!l.close(a,40));assert_eq!(l.entries().count(),1);assert!(l.close(b,50));
}
#[test]
fn ledger_same_id_on_reused_slot_has_new_generation() {
    let mut l=Ledger::<1,1>::new(1);let a=l.open(0,7,10).unwrap();assert!(l.close(a,20));let b=l.open(0,7,30).unwrap();
    assert_eq!(a.slot(),b.slot());assert_ne!(a.generation(),b.generation());assert!(!l.close(a,40));assert!(l.close(b,50));
}
#[test]
fn ledger_kind_and_owner_identity_cannot_cross_close() {
    let mut a=Ledger::<2,2>::new(1);let mut b=Ledger::<2,2>::new(2);
    let x=a.open(1,42,0).unwrap();let y=b.open(1,42,0).unwrap();assert!(!b.close(x,1));assert!(!a.close(y,1));assert!(a.close(x,1));assert!(b.close(y,1));
}
#[test]
fn ledger_overflow_does_not_overwrite_and_is_counted() {
    let mut l=Ledger::<2,1>::new(1);let a=l.open(0,1,10).unwrap();assert!(l.open(0,2,20).is_none());assert_eq!(l.dropped(),1);
    assert_eq!(l.entries().next().unwrap().id,1);assert!(l.close(a,30));assert!(l.open(0,3,40).is_some());
}
#[test]
fn ledger_overdue_is_advisory_and_boundary_is_strict() {
    let mut l=Ledger::<1,2>::new(1);let a=l.open(0,42,100).unwrap();
    let e=*l.entries().next().unwrap();assert_eq!(e.age_ns(90),0);assert!(!e.overdue(200,100));assert!(e.overdue(201,100));assert!(l.close(a,202));
}
#[test]
fn ledger_full_kinds_are_independent() {
    let mut l=Ledger::<3,1>::new(1);for k in 0..3 {assert!(l.open(k,k as u64,0).is_some());}for k in 0..3 {assert!(l.open(k,9,0).is_none());}
    assert_eq!(l.dropped(),3);assert_eq!(l.entries().count(),3);assert!(l.open(3,0,0).is_none());
}

#[test]
fn seqlock_model_requires_fences_on_both_sides() {
    // Observing a new payload word must carry the writer's odd sequence before
    // the reader's final validation. Assert the actual backend operation trace.
    let s=Spy::new();let mut seq=0;write_record(&s,0,&[1,2,3],&mut seq).unwrap();
    assert!(s.ops.lock().unwrap().contains(&Op::Fence(Ordering::Release)));
    s.ops.lock().unwrap().clear();read_record::<3>(&s,0).unwrap();
    assert!(s.ops.lock().unwrap().contains(&Op::Fence(Ordering::Acquire)));
}
#[test]
fn seqlock_every_writer_commit_is_release_and_every_reader_starts_acquire() {
    let s=Spy::new();let mut seq=0;
    for _ in 0..10 {s.ops.lock().unwrap().clear();write_record(&s,0,&[7],&mut seq).unwrap();
        assert_eq!(s.ops.lock().unwrap().last(),Some(&Op::Store(0,seq,Ordering::Release)));
        s.ops.lock().unwrap().clear();read_record::<1>(&s,0).unwrap();
        assert_eq!(s.ops.lock().unwrap().first(),Some(&Op::Load(0,Ordering::Acquire)));}
}
#[test]
fn ledger_seeded_model_preserves_every_live_obligation() {
    let mut l=Ledger::<2,4>::new(1);let mut live=Vec::new();let mut seed=11u64;
    for id in 0..10_000 {seed=seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        if seed&1==0 {if let Some(t)=l.open(((seed>>20)&1) as usize,id,id) {live.push((t,id));}}
        else if !live.is_empty() {let index=(seed as usize)%live.len();let (t,_)=live.swap_remove(index);assert!(l.close(t,id));assert!(!l.close(t,id));}
        let mut actual=l.entries().map(|e| e.id).collect::<Vec<_>>();let mut expected=live.iter().map(|e| e.1).collect::<Vec<_>>();actual.sort_unstable();expected.sort_unstable();assert_eq!(actual,expected);
    }
}
