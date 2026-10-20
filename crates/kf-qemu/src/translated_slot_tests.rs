//! GPU-free tests of the production Translated cell. The ring/plane/GPU are
//! fakes; the cell, mutex and handoff (when fixed) are production code. No timer
//! rings a token. Timeouts below bound a failed test, not modeled progress.
use super::{Cell, Identity};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

const BOUND: Duration = Duration::from_secs(2);

fn fixture(value: u32) -> (Arc<Cell<u32>>, Arc<AtomicU64>) {
    let rung = Arc::new(AtomicU64::new(0));
    let r = Arc::clone(&rung);
    let cell = Arc::new(Cell::new(
        value,
        Identity {
            idx: 7,
            tsg: Some(9),
        },
        move |idx| {
            assert_eq!(idx, 7);
            r.fetch_add(1, Ordering::SeqCst);
        },
    ));
    (cell, rung)
}

#[test]
fn translated_lost_worker_attempt_is_rung_on_holder_release() {
    let (cell, rung) = fixture(0);
    let hold = cell.lock().unwrap();
    let c = Arc::clone(&cell);
    thread::spawn(move || assert!(c.enter().is_err()))
        .join()
        .unwrap();
    assert_eq!(rung.load(Ordering::SeqCst), 0, "no ring before release");
    drop(hold);
    assert_eq!(
        rung.load(Ordering::SeqCst),
        1,
        "final doorbell was consumed: the holder owes a ring"
    );
}

#[test]
fn translated_lost_attempt_under_probe_guard_is_rung_too() {
    let (cell, rung) = fixture(0);
    let hold = cell.try_lock().unwrap();
    let c = Arc::clone(&cell);
    thread::spawn(move || assert!(c.enter().is_err()))
        .join()
        .unwrap();
    drop(hold);
    assert_eq!(
        rung.load(Ordering::SeqCst),
        1,
        "diagnostic holders must hand off too"
    );
}

#[test]
fn translated_release_rings_after_unlock() {
    let target = Arc::new(std::sync::OnceLock::<Arc<Cell<u32>>>::new());
    let checked = Arc::new(AtomicU64::new(0));
    let (t, n) = (Arc::clone(&target), Arc::clone(&checked));
    let cell = Arc::new(Cell::new(0, Identity { idx: 3, tsg: None }, move |_| {
        assert!(
            t.get().unwrap().try_lock().is_ok(),
            "ring while still locked"
        );
        n.fetch_add(1, Ordering::SeqCst);
    }));
    assert!(target.set(Arc::clone(&cell)).is_ok());
    let hold = cell.lock().unwrap();
    assert!(cell.enter().is_err());
    drop(hold);
    assert_eq!(checked.load(Ordering::SeqCst), 1, "no handoff callback");
}

fn metadata_during_hold(group: bool) {
    let (cell, _) = fixture(0);
    let hold = cell.lock().unwrap();
    let (tx, rx) = mpsc::channel();
    let c = Arc::clone(&cell);
    let drainer = thread::spawn(move || {
        let id = c.identity().unwrap();
        tx.send(if group { id.tsg.unwrap() } else { id.idx })
            .unwrap();
    });
    let result = rx.recv_timeout(BOUND);
    // Always release/join, including EXPECTED RED, so a failure leaks no thread.
    drop(hold);
    drainer.join().unwrap();
    assert_eq!(
        result.unwrap_or(0),
        if group { 9 } else { 7 },
        "drainer blocked behind pump"
    );
}

#[test]
fn translated_token_query_does_not_wait_for_the_pump() {
    metadata_during_hold(false);
}

#[test]
fn translated_group_query_does_not_wait_for_the_pump() {
    metadata_during_hold(true);
}

fn edit_during_hold(value: u32) {
    let (cell, _) = fixture(0);
    let hold = cell.lock().unwrap();
    let (tx, rx) = mpsc::channel();
    let c = Arc::clone(&cell);
    let drainer = thread::spawn(move || {
        tx.send(c.edit(|v| *v = value).is_none()).unwrap();
    });
    let result = rx.recv_timeout(BOUND);
    drop(hold);
    drainer.join().unwrap();
    assert_eq!(
        result,
        Ok(true),
        "busy schedule must defer and leave the drainer free"
    );
    assert_eq!(
        *cell.lock().unwrap(),
        0,
        "busy edit must not run inline later"
    );
    // The fake act thread resolves the deferred operation after the holder ends.
    let c = Arc::clone(&cell);
    thread::spawn(move || *c.lock().unwrap() = value)
        .join()
        .unwrap();
    assert_eq!(*cell.lock().unwrap(), value);
}

#[test]
fn translated_busy_schedule_enable_defers_without_blocking() {
    edit_during_hold(1);
}

#[test]
fn translated_busy_schedule_disable_defers_without_blocking() {
    edit_during_hold(2);
}

#[test]
fn translated_quiet_slot_does_not_ring_itself_forever() {
    let (cell, rung) = fixture(0);
    for _ in 0..100 {
        drop(cell.enter().unwrap());
        drop(cell.try_lock().unwrap());
        assert_eq!(cell.edit(|v| *v += 1), Some(()));
    }
    assert_eq!(rung.load(Ordering::SeqCst), 0, "no outstanding obligation");
}

#[test]
fn translated_a_final_doorbell_makes_progress_within_one_pass_after_release() {
    let (cell, rung) = fixture(1);
    let hold = cell.lock().unwrap();
    // The plane consumes the ONLY doorbell while an act holds the slot.
    assert!(cell.enter().is_err());
    drop(hold);
    let mut passes = 0;
    for _ in 0..4 {
        passes += 1;
        if rung.swap(0, Ordering::SeqCst) != 0 {
            let mut g = cell.enter().unwrap();
            *g = 0; // fake GPU accepted the one queued entry
        }
        if *cell.lock().unwrap() == 0 {
            break;
        }
    }
    assert_eq!(
        *cell.lock().unwrap(),
        0,
        "stranded without another doorbell or edge"
    );
    assert_eq!(passes, 1, "release must publish, never rely on a timer");
}

#[test]
fn translated_multiple_losers_coalesce_but_keep_the_obligation() {
    let (cell, rung) = fixture(0);
    let hold = cell.lock().unwrap();
    thread::scope(|scope| {
        for _ in 0..3 {
            scope.spawn(|| assert!(cell.enter().is_err()));
        }
    });
    drop(hold);
    assert_eq!(rung.load(Ordering::SeqCst), 1);
}

struct FakeSlot {
    pending: VecDeque<u64>,
    next: u64,
}

fn chaos(seed: u64, workers: usize) {
    const N: u64 = 240;
    let rung = Arc::new(AtomicBool::new(true)); // birth doorbell
    let sent = Arc::new(Mutex::new(VecDeque::<u64>::new()));
    let (ring, gpu_in) = (Arc::clone(&rung), Arc::clone(&sent));
    let cell = Arc::new(Cell::new(
        FakeSlot {
            pending: (0..N).collect(),
            next: 0,
        },
        Identity {
            idx: 7,
            tsg: Some(9),
        },
        move |_| {
            ring.store(true, Ordering::SeqCst);
        },
    ));
    let stop = Arc::new(AtomicBool::new(false));
    let consumed = Arc::new(AtomicU64::new(0));
    let passes = Arc::new(AtomicU64::new(0));
    // Hold the pump slot while every worker consumes a claimed doorbell. There
    // are deliberately no GPU entries/engine edges yet to rescue the channel.
    let hold = cell.lock().unwrap();
    for _ in 0..workers {
        rung.store(true, Ordering::SeqCst);
        assert!(rung.swap(false, Ordering::SeqCst));
        let c = Arc::clone(&cell);
        thread::spawn(move || assert!(c.enter().is_err()))
            .join()
            .unwrap();
    }
    drop(hold);
    // A deterministic pass bound catches the defect before scheduler timing.
    assert!(
        rung.load(Ordering::SeqCst),
        "seed={seed} workers={workers}: no holder re-ring"
    );
    let mut threads = Vec::new();
    for worker in 0..workers {
        let (c, r, q, done, p) = (
            Arc::clone(&cell),
            Arc::clone(&rung),
            Arc::clone(&sent),
            Arc::clone(&stop),
            Arc::clone(&passes),
        );
        threads.push(thread::spawn(move || {
            let mut rng = seed ^ (worker as u64 + 1);
            while !done.load(Ordering::SeqCst) {
                p.fetch_add(1, Ordering::SeqCst);
                if r.swap(false, Ordering::SeqCst) {
                    if let Ok(mut g) = c.enter() {
                        if let Some(entry) = g.pending.pop_front() {
                            assert_eq!(entry, g.next);
                            g.next += 1;
                            q.lock().unwrap().push_back(entry);
                        }
                    }
                }
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                if rng & 3 == 0 {
                    thread::yield_now();
                }
            }
        }));
    }
    // Fake PBDMA consumes in order, real engine edges ring the same fake token.
    let (done, r, seen) = (Arc::clone(&stop), Arc::clone(&rung), Arc::clone(&consumed));
    threads.push(thread::spawn(move || {
        while !done.load(Ordering::SeqCst) {
            if let Some(entry) = gpu_in.lock().unwrap().pop_front() {
                let expected = seen.fetch_add(1, Ordering::SeqCst);
                assert_eq!(entry, expected, "fake GPU duplicate/reordered entry");
                r.store(true, Ordering::SeqCst);
            }
            thread::yield_now();
        }
    }));
    // The drainer never waits behind a worker's pump. Busy schedule edits are
    // queued for a separate fake act thread, which may take the blocking lock.
    let (c, done) = (Arc::clone(&cell), Arc::clone(&stop));
    let (act_tx, act_rx) = mpsc::sync_channel(8);
    let act_cell = Arc::clone(&cell);
    threads.push(thread::spawn(move || {
        while let Ok(()) = act_rx.recv() {
            drop(act_cell.lock().unwrap());
        }
    }));
    threads.push(thread::spawn(move || {
        while !done.load(Ordering::SeqCst) {
            assert_eq!(c.identity().unwrap().idx, 7);
            if c.edit(|_| ()).is_none() {
                let _ = act_tx.try_send(());
            }
            thread::yield_now();
        }
    }));
    let until = Instant::now() + BOUND;
    while consumed.load(Ordering::SeqCst) != N && Instant::now() < until {
        thread::yield_now();
    }
    stop.store(true, Ordering::SeqCst);
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(
        consumed.load(Ordering::SeqCst),
        N,
        "seed={seed} workers={workers}: stranded"
    );
    assert_eq!(cell.lock().unwrap().next, N);
    assert!(passes.load(Ordering::SeqCst) >= N);
}

#[test]
fn translated_real_threads_one_worker() {
    for seed in [17, 71, 337] {
        chaos(seed, 1);
    }
}
#[test]
fn translated_real_threads_two_workers() {
    for seed in [31, 127, 509] {
        chaos(seed, 2);
    }
}
#[test]
fn translated_real_threads_three_workers() {
    for seed in [47, 251, 1021] {
        chaos(seed, 3);
    }
}
