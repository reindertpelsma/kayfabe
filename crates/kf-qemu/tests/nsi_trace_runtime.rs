//! Runtime recorder requirements, registered before implementation. A production build does
//! not compile this module or its recorder. The separate wiring gates are EXPECTED RED first.
#![cfg(feature = "diag-nsi")]

use kf_chan::ptnsi::Verdict;
use kf_qemu::nsitrace::{CAPACITY, Source, Trace};

#[test]
fn unset_and_non_one_values_leave_the_trace_unallocated_and_silent() {
    for setting in [None, Some("0"), Some("on"), Some(""), Some("junk")] {
        let t = Trace::from_setting(setting);
        assert_eq!(t.capacity(), 0);
        t.note(
            10,
            Source::CompletionFd,
            Some(7),
            Some(3),
            Some(true),
            Verdict::Raise(3),
        );
        assert!(t.finish().is_none());
    }
}

#[test]
fn a_host_edge_keeps_source_time_vector_arming_and_decision() {
    let t = Trace::from_setting(Some("1"));
    t.note(
        10,
        Source::CompletionFd,
        Some(7),
        Some(3),
        Some(true),
        Verdict::Raise(3),
    );
    t.note(
        11,
        Source::Engine(9),
        Some(8),
        Some(4),
        Some(false),
        Verdict::NotArmed,
    );
    t.note(
        12,
        Source::Engine(9),
        Some(8),
        Some(4),
        Some(true),
        Verdict::Owed,
    );
    t.note(55, Source::Trailing, None, Some(4), None, Verdict::Raise(4));
    let s = t.finish().unwrap();
    assert_eq!((s.seen, s.overwritten, s.contended), (4, 0, 0));
    let r = &s.records;
    assert_eq!(r.len(), 4);
    assert_eq!(
        (
            r[0].seq,
            r[0].at_ns,
            r[0].source,
            r[0].slot,
            r[0].vector,
            r[0].armed,
            r[0].verdict
        ),
        (
            1,
            10,
            Source::CompletionFd,
            Some(7),
            Some(3),
            Some(true),
            Verdict::Raise(3)
        )
    );
    assert_eq!(
        (r[1].source, r[1].armed, r[1].verdict),
        (Source::Engine(9), Some(false), Verdict::NotArmed)
    );
    assert_eq!(r[2].verdict, Verdict::Owed);
    assert_eq!(
        (r[3].at_ns, r[3].source, r[3].armed),
        (55, Source::Trailing, None)
    );
}

#[test]
fn the_trace_retains_a_bounded_tail_instead_of_filling_during_boot() {
    let t = Trace::from_setting(Some("1"));
    assert_eq!(t.capacity(), CAPACITY);
    for n in 0..CAPACITY + 5 {
        t.note(
            n as u64,
            Source::Engine(1),
            Some(0),
            Some(0),
            Some(true),
            Verdict::Raise(0),
        );
    }
    assert_eq!(t.capacity(), CAPACITY);
    let s = t.finish().unwrap();
    assert_eq!(
        (s.seen, s.overwritten, s.contended),
        ((CAPACITY + 5) as u64, 5, 0)
    );
    assert_eq!(s.records.len(), CAPACITY);
    assert_eq!(s.records.first().unwrap().at_ns, 5);
    assert_eq!(s.records.last().unwrap().seq, (CAPACITY + 5) as u64);
}

#[test]
fn finishing_twice_does_not_duplicate_output_and_later_edges_do_not_grow_it() {
    let t = Trace::from_setting(Some("1"));
    t.note(
        1,
        Source::CompletionFd,
        None,
        None,
        Some(false),
        Verdict::NotArmed,
    );
    assert_eq!(t.finish().unwrap().records.len(), 1);
    t.note(
        2,
        Source::CompletionFd,
        None,
        None,
        Some(false),
        Verdict::NotArmed,
    );
    assert!(t.finish().is_none());
}

#[test]
fn concurrent_workers_account_for_every_record_or_explicit_trace_loss() {
    let t = Trace::from_setting(Some("1"));
    std::thread::scope(|scope| {
        for engine in 0..4 {
            let t = &t;
            scope.spawn(move || {
                for n in 0..2000 {
                    t.note(
                        n,
                        Source::Engine(engine),
                        None,
                        Some(engine),
                        Some(true),
                        Verdict::Raise(engine),
                    );
                }
            });
        }
    });
    let s = t.finish().unwrap();
    assert_eq!(s.seen, 8000);
    assert_eq!(s.records.len() as u64 + s.overwritten + s.contended, s.seen);
    let mut ids: Vec<_> = s.records.iter().map(|r| r.seq).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), s.records.len());
    assert!(
        s.records
            .iter()
            .all(|r| r.armed == Some(true) && matches!(r.verdict, Verdict::Raise(_)))
    );
}
