//! 2026-10-20 -- **The holder-handoff protocol** (`kf_chan::handoff::Handoff`), checked three ways: every interleaving of a loser and
//! a holder (the lost-registration race included), the same enumeration against a NAIVE loser that must be found stranded (the checker has
//! teeth), and real threads on a real mutex.
//!
//! The protocol: the loser reads the epoch BEFORE its attempt; if the attempt fails it registers and then asks whether a holder released since
//! (`changed_since`): if so it retries at once. The holder, after unlocking, bumps the epoch FIRST and only then looks at the waiter list and
//! rings it. So either the holder sees the registration, or the loser sees the epoch move: the registration cannot be lost.
#![allow(clippy::too_many_lines, missing_docs)]

use kf_chan::handoff::Handoff;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const WHO: u32 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Loser {
    Prepare,
    Try,
    Register,
    Recheck,
    Parked,
    Acquired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Holder {
    Unlock,
    Bump,
    Look,
    Ring,
    /// A second holder takes the lock again (enabled only while it is free), then releases it the same way.
    Relock,
    Done,
}

struct World {
    h: Handoff,
    lock: bool,
    rung: Vec<u32>,
    loser: Loser,
    seen: u64,
    holder: Holder,
    second: bool,
    retries: u32,
    naive: bool,
}

impl World {
    fn new(second: bool, naive: bool) -> Self {
        Self {
            h: Handoff::new(),
            lock: true, // the holder holds it when the loser starts
            rung: Vec::new(),
            loser: Loser::Prepare,
            seen: 0,
            holder: Holder::Unlock,
            second,
            retries: 0,
            naive,
        }
    }
    fn enabled(&self) -> Vec<char> {
        let mut v = Vec::new();
        if !matches!(self.loser, Loser::Parked | Loser::Acquired) {
            v.push('L');
        }
        let holder_ok = match self.holder {
            Holder::Done => false,
            Holder::Relock => !self.lock,
            _ => true,
        };
        if holder_ok {
            v.push('H');
        }
        v
    }
    fn step(&mut self, who: char) {
        if who == 'L' {
            match self.loser {
                Loser::Prepare => {
                    self.seen = self.h.prepare();
                    self.loser = Loser::Try;
                }
                Loser::Try => {
                    if self.lock {
                        self.loser = Loser::Register;
                    } else {
                        self.lock = true;
                        self.loser = Loser::Acquired;
                    }
                }
                Loser::Register => {
                    self.h.register(WHO);
                    self.loser = if self.naive {
                        Loser::Parked
                    } else {
                        Loser::Recheck
                    };
                }
                Loser::Recheck => {
                    if self.h.changed_since(self.seen) {
                        self.retries += 1;
                        self.loser = Loser::Prepare;
                    } else {
                        self.loser = Loser::Parked;
                    }
                }
                Loser::Parked | Loser::Acquired => unreachable!(),
            }
        } else {
            match self.holder {
                Holder::Unlock => {
                    self.lock = false;
                    self.holder = Holder::Bump;
                }
                Holder::Bump => {
                    self.h.bump();
                    self.holder = Holder::Look;
                }
                Holder::Look => {
                    self.holder = if self.h.waiters() != 0 {
                        Holder::Ring
                    } else {
                        self.after_release()
                    };
                }
                Holder::Ring => {
                    self.rung.extend(self.h.take());
                    self.holder = self.after_release();
                }
                Holder::Relock => {
                    self.lock = true;
                    self.holder = Holder::Unlock;
                }
                Holder::Done => unreachable!(),
            }
        }
    }
    fn after_release(&mut self) -> Holder {
        if self.second {
            self.second = false;
            Holder::Relock
        } else {
            Holder::Done
        }
    }
    /// A terminal state is bad when the loser is parked, unrung, and nobody holds the lock any more.
    fn stranded(&self) -> bool {
        self.loser == Loser::Parked
            && !self.lock
            && !self.rung.contains(&WHO)
            && self.holder == Holder::Done
    }
}

/// Run `schedule` (a sequence of actor letters) on a fresh world.
fn run(second: bool, naive: bool, schedule: &[char]) -> World {
    let mut w = World::new(second, naive);
    for &c in schedule {
        w.step(c);
    }
    w
}

/// Explore every schedule; returns the first stranded one.
fn explore(second: bool, naive: bool) -> (usize, Option<String>) {
    let mut leaves = 0usize;
    let mut bad: Option<String> = None;
    let mut stack: Vec<Vec<char>> = vec![Vec::new()];
    while let Some(sched) = stack.pop() {
        let w = run(second, naive, &sched);
        let en = w.enabled();
        if en.is_empty() {
            leaves += 1;
            if w.stranded() && bad.is_none() {
                bad = Some(describe(&sched));
            }
            continue;
        }
        assert!(sched.len() < 40, "runaway schedule");
        for c in en {
            let mut s = sched.clone();
            s.push(c);
            stack.push(s);
        }
    }
    (leaves, bad)
}

fn describe(sched: &[char]) -> String {
    let mut out = Vec::new();
    // re-run the schedule on a world of the same kind to print the steps
    let mut w2 = World::new(false, false);
    for &c in sched {
        let before = if c == 'L' {
            format!("loser {:?}", w2.loser)
        } else {
            format!("holder {:?}", w2.holder)
        };
        out.push(before);
        w2.step(c);
    }
    out.join(" -> ")
}

/// Every interleaving of a loser (attempt, register, re-check) and a holder (unlock, bump, look at the list, ring) leaves the loser either holding
/// the lock or rung -- including the lost-registration order: the loser checked, the holder released and looked at an EMPTY list, the loser
/// registered afterwards.
#[test]
fn no_interleaving_of_a_loser_and_a_holder_strands_the_loser() {
    for second in [false, true] {
        let (leaves, bad) = explore(second, false);
        assert!(leaves > 20, "{leaves} schedules");
        assert!(
            bad.is_none(),
            "second hold {second}: stranded by {}",
            bad.unwrap_or_default()
        );
    }
}

/// The lost-registration schedule, spelled out: it exists (the holder looks at an empty list before the loser registers), and the protocol
/// answers it with `changed_since`.
#[test]
fn the_lost_registration_schedule_is_answered_by_the_epoch() {
    // L: Prepare, Try (lock held) | H: Unlock, Bump, Look (empty) | L: Register, Recheck
    let s = ['L', 'L', 'H', 'H', 'H', 'L', 'L'];
    let w = run(false, false, &s);
    assert_eq!(
        w.h.waiters(),
        1,
        "the registration came after the holder's look"
    );
    assert!(w.rung.is_empty(), "the holder rang nobody");
    assert_eq!(
        w.retries, 1,
        "the loser saw the epoch move and retried at once"
    );
    let w = run(false, false, &[s.as_slice(), &['L', 'L']].concat());
    assert_eq!(w.loser, Loser::Acquired, "the retry finds the lock free");
}

/// The checker has teeth: a NAIVE loser (register after the failed attempt and give up, no epoch re-check) is stranded by exactly that schedule.
#[test]
fn a_naive_loser_is_stranded_by_the_lost_registration_schedule() {
    let (_, bad) = explore(false, true);
    let bad = bad.expect("the naive protocol must be found stranded");
    eprintln!("naive loser stranded by: {bad}");
}

/// The composed calls are the micro-steps: `lost` = register then `changed_since`; `released` = bump, then ring the registered.
#[test]
fn lost_and_released_compose_the_micro_steps() {
    let h = Handoff::new();
    let seen = h.prepare();
    assert!(
        !h.lost(1, seen),
        "no holder released since the attempt began: stay registered"
    );
    assert_eq!(h.waiters(), 1);
    let mut rung = Vec::new();
    h.released(|w| rung.push(w));
    assert_eq!(rung, vec![1], "the holder rings the registered waiter");
    assert_eq!(h.waiters(), 0);
    // a holder that released BEFORE the registration: the loser is told to retry
    let seen = h.prepare();
    h.released(|_| {});
    assert!(
        h.lost(2, seen),
        "the epoch moved between the attempt and the registration: retry now"
    );
    // idempotent registration
    h.register(2);
    h.register(2);
    assert_eq!(h.waiters(), 1);
}

/// Real threads, real mutex: losers wait to be rung, every unlock is followed by `released`; nobody may wait for ever.
#[test]
fn real_threads_never_lose_a_wake() {
    const THREADS: u32 = 4;
    const ITERS: u32 = 4000;
    let h = Arc::new(Handoff::new());
    let m = Arc::new(Mutex::new(0u64));
    let flags: Arc<Vec<AtomicBool>> =
        Arc::new((0..THREADS).map(|_| AtomicBool::new(false)).collect());
    let mut hs = Vec::new();
    for id in 0..THREADS {
        let (h, m, flags) = (Arc::clone(&h), Arc::clone(&m), Arc::clone(&flags));
        hs.push(std::thread::spawn(move || {
            let ring = |who: u32| flags[who as usize].store(true, Ordering::SeqCst);
            for i in 0..ITERS {
                loop {
                    let seen = h.prepare();
                    match m.try_lock() {
                        Ok(mut g) => {
                            *g += 1;
                            if i % 7 == 0 {
                                std::thread::yield_now();
                            }
                            drop(g);
                            h.released(ring);
                            break;
                        }
                        Err(_) => {
                            if h.lost(id, seen) {
                                continue;
                            }
                            let t0 = Instant::now();
                            while !flags[id as usize].swap(false, Ordering::SeqCst) {
                                assert!(
                                    t0.elapsed() < Duration::from_secs(20),
                                    "thread {id} was never rung: a lost wake"
                                );
                                std::hint::spin_loop();
                            }
                        }
                    }
                }
            }
        }));
    }
    for t in hs {
        t.join().expect("no thread stranded");
    }
    assert_eq!(*m.lock().unwrap(), u64::from(THREADS * ITERS));
}

/// `lost` answers "retry now" exactly when a holder released since the attempt began, and counts that retry (the production summary).
#[test]
fn lost_retries_exactly_when_a_release_intervened_and_counts_it() {
    let h = Handoff::new();
    for round in 0..3u32 {
        let seen = h.prepare();
        assert!(
            !h.lost(7, seen),
            "round {round}: no release since the attempt"
        );
        h.bump();
        assert!(
            h.lost(7, seen),
            "round {round}: a release intervened: retry"
        );
        let fresh = h.prepare();
        assert!(
            !h.lost(7, fresh),
            "round {round}: the retry's own attempt starts after the release"
        );
    }
    assert!(h.summary().contains("retried=3"), "{}", h.summary());
}
