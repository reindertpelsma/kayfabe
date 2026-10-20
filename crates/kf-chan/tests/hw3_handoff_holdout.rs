//! Independent HW3 holdout of M's real Handoff. No GPU or production changes.
//! FALSIFIER: any failed try_lock is neither immediately retried nor rung after
//! every holder has released. Both forced loss and release races must execute.
#![allow(missing_docs)]
use kf_chan::handoff::Handoff;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::thread;

fn run(workers: u32, seed: u64) {
    let h = Arc::new(Handoff::new());
    let lock = Arc::new(Mutex::new(()));
    let rung = Arc::new(AtomicU64::new(0));
    let mut lost_count = 0;
    for episode in 0..128 {
        rung.store(0, Ordering::SeqCst);
        let guard = lock.lock().unwrap();
        let start = Arc::new(Barrier::new(workers as usize + 1));
        let (tx, rx) = mpsc::channel();
        let mut joins = Vec::new();
        for worker in 0..workers {
            let (h, lock, rung, start, tx) = (
                Arc::clone(&h),
                Arc::clone(&lock),
                Arc::clone(&rung),
                Arc::clone(&start),
                tx.clone(),
            );
            joins.push(thread::spawn(move || {
                let seen = h.prepare();
                start.wait();
                let bits = seed.rotate_left(worker + (episode % 17)) ^ u64::from(episode);
                if bits & 1 != 0 {
                    thread::yield_now();
                }
                let acquired = match lock.try_lock() {
                    Ok(g) => {
                        drop(g);
                        h.released(|who| {
                            rung.fetch_or(1 << who, Ordering::SeqCst);
                        });
                        true
                    }
                    Err(_) => {
                        if h.lost(worker, seen) {
                            rung.fetch_or(1 << worker, Ordering::SeqCst);
                        }
                        false
                    }
                };
                tx.send(()).unwrap();
                acquired
            }));
        }
        drop(tx);
        start.wait();
        if episode % 2 == 0 {
            // All workers have lost the attempt before the release. This is a
            // positive control for the randomized race schedule next episode.
            for _ in 0..workers {
                rx.recv().unwrap();
            }
        }
        drop(guard);
        h.released(|who| {
            rung.fetch_or(1 << who, Ordering::SeqCst);
        });
        for (worker, join) in joins.into_iter().enumerate() {
            if !join.join().unwrap() {
                lost_count += 1;
                assert_ne!(
                    rung.load(Ordering::SeqCst) & (1 << worker),
                    0,
                    "seed={seed} workers={workers} episode={episode} worker={worker}"
                );
            }
        }
        h.released(|who| {
            rung.fetch_or(1 << who, Ordering::SeqCst);
        });
    }
    assert!(
        lost_count >= 64 * workers,
        "forced losses must have executed"
    );
}

#[test]
fn m_holdout_one_worker() {
    run(1, 0x7f4a_7c15_d39b_2201);
}
#[test]
fn m_holdout_two_workers() {
    run(2, 0x9e37_79b9_ae11_0023);
}
#[test]
fn m_holdout_three_workers() {
    run(3, 0xd1b5_4a32_4837_00a7);
}
