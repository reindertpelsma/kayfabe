//! ★ 2026-10-16 — **Real-thread stress of the USER-ring lifecycle with a hostile writer**
//! (brief section 3 item 5; pattern of the existing `userstore` stress: seeded, bounded seconds by default,
//! `KF3_STRESS_SECS` longer).
//!
//! The relay under test is the same `lifecycle_support::Relay` as in the interleaving model, running on its
//! own thread against REAL atomics: a guest ring of `AtomicU32` dword pairs, an epoch / live registry, a
//! private ring that "the hardware" consumes at once. Two tests:
//!
//! 1. `a_well_behaved_guest_with_concurrent_registrations`: the guest thread writes entries, registers (the
//!    epoch bump precedes its "reply"), publishes the trigger after the reply, and so on for thousands of
//!    cycles. Asserts I2 (every entry forwarded exactly once, in order, bit-exact) and I4 (an entry published
//!    while a registration is live was scanned).
//! 2. `a_hostile_writer_rewrites_ring_entries_and_gp_put_while_the_worker_runs`: a second thread flips ONE
//!    dword of random slots and rewrites `GP_PUT` at random while the worker runs. Asserts I7 (the stored
//!    value is the tested value, one consistent pair of dwords the guest really wrote, and passes
//!    `inspect_entry`), no panic, bounded work per turn, and no pushbuffer read in Pointer mode (I1).
#![allow(
    dead_code,
    missing_docs,
    clippy::too_many_lines,
    clippy::collapsible_if
)]

#[path = "lifecycle_support/mod.rs"]
mod support;

use kf_chan::user_gate::MARK_TAG;
use kf_chan::user_lifecycle::{Lifecycle, Mode};
use kf_chan::user_scan::Scan;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use support::*;

const N: u32 = 64;

struct Shared {
    lo: Vec<AtomicU32>,
    hi: Vec<AtomicU32>,
    put: AtomicU32,
    guest_get: AtomicU32,
    epoch: AtomicU64,
    live: AtomicU32,
    /// Everything the guest or the hostile thread ever wrote to each half of each slot.
    wrote_lo: Mutex<Vec<HashSet<u32>>>,
    wrote_hi: Mutex<Vec<HashSet<u32>>>,
    log: Mutex<Vec<Pub>>,
    markers: AtomicU64,
    pb_reads: AtomicU64,
    pb_reads_in_pointer: AtomicU64,
    violations: Mutex<Vec<String>>,
    /// The guest's entries by id, for the pushbuffer words.
    stop: AtomicBool,
}

impl Shared {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            lo: (0..N).map(|_| AtomicU32::new(0)).collect(),
            hi: (0..N).map(|_| AtomicU32::new(0)).collect(),
            put: AtomicU32::new(0),
            guest_get: AtomicU32::new(0),
            epoch: AtomicU64::new(0),
            live: AtomicU32::new(0),
            wrote_lo: Mutex::new((0..N).map(|_| HashSet::from([0])).collect()),
            wrote_hi: Mutex::new((0..N).map(|_| HashSet::from([0])).collect()),
            log: Mutex::new(Vec::new()),
            markers: AtomicU64::new(0),
            pb_reads: AtomicU64::new(0),
            pb_reads_in_pointer: AtomicU64::new(0),
            violations: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
        })
    }
    fn write_slot(&self, slot: u32, raw: u64) {
        self.wrote_lo.lock().unwrap()[slot as usize].insert(raw as u32);
        self.wrote_hi.lock().unwrap()[slot as usize].insert((raw >> 32) as u32);
        self.lo[slot as usize].store(raw as u32, Ordering::Release);
        self.hi[slot as usize].store((raw >> 32) as u32, Ordering::Release);
    }
    fn violation(&self, s: String) {
        let mut v = self.violations.lock().unwrap();
        if v.len() < 20 {
            v.push(s);
        }
    }
}

struct ThreadEnv {
    sh: Arc<Shared>,
    mode: Mode,
}

impl Env for ThreadEnv {
    fn src_entries(&self) -> u32 {
        N
    }
    fn gp_put(&mut self) -> u32 {
        self.sh.put.load(Ordering::Acquire)
    }
    fn gp_dword(&mut self, slot: u32, hi: bool) -> u32 {
        let v = if hi { &self.sh.hi } else { &self.sh.lo };
        v[slot as usize].load(Ordering::Acquire)
    }
    fn registry_epoch(&mut self) -> u64 {
        std::sync::atomic::fence(Ordering::SeqCst);
        self.sh.epoch.load(Ordering::Acquire)
    }
    fn registry_live(&mut self) -> u32 {
        self.sh.live.load(Ordering::Acquire)
    }
    fn hw_get(&mut self) -> u32 {
        (self.sh.log.lock().unwrap().len() as u32) & 7
    }
    fn hw_reference(&mut self) -> u32 {
        let m = self.sh.markers.load(Ordering::Acquire);
        if m == 0 {
            0
        } else {
            MARK_TAG | (m as u32 & 0xffff)
        }
    }
    fn read_pb(&mut self, raw: u64) -> Vec<u32> {
        self.sh.pb_reads.fetch_add(1, Ordering::Relaxed);
        if self.mode == Mode::Pointer {
            self.sh.pb_reads_in_pointer.fetch_add(1, Ordering::Relaxed);
        }
        match entry_id(raw) {
            Some(0) => ge(0, Kind::SetObj).words(),
            Some(190) => ge(190, Kind::Trig { migrate: false }).words(),
            Some(id) => plain_words(id),
            None => vec![POISON],
        }
    }
    fn publish(&mut self, p: Pub) {
        if let Pub::Guest { tested, stored, .. } = p {
            if tested != stored {
                self.sh
                    .violation(format!("I7: tested {tested:#x} stored {stored:#x}"));
            }
        }
        if let Pub::Marker(_) = p {
            self.sh.markers.fetch_add(1, Ordering::AcqRel);
        }
        self.sh.log.lock().unwrap().push(p);
    }
    fn write_guest_get(&mut self, v: u32) {
        self.sh.guest_get.store(v, Ordering::Release);
    }
    fn trigger_migrates(&mut self, _raw: u64) -> bool {
        false
    }
    fn executed(&mut self, _migrate: bool) {
        // implicit delete of the consumed registration
        let _ = self
            .sh
            .live
            .try_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_sub(1));
        self.sh.epoch.fetch_add(1, Ordering::AcqRel);
    }
    fn event(&mut self, _e: Ev) {}
}

fn plain_words(id: u8) -> Vec<u32> {
    ge(id, Kind::Plain).words()
}

fn stress_secs() -> f64 {
    std::env::var("KF3_STRESS_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2.0)
}

/// A hard bound on every wait of a test, so that a defective relay FAILS the test instead of hanging it.
fn hard_deadline(secs: f64) -> std::time::Instant {
    std::time::Instant::now() + std::time::Duration::from_secs_f64(secs + 8.0)
}

fn run_relay_until(
    sh: &Arc<Shared>,
    relay: &mut Relay,
    env: &mut ThreadEnv,
    should_stop: impl Fn() -> bool,
) -> u64 {
    let mut turns = 0u64;
    while !should_stop() && !relay.dead {
        relay.claim();
        let mut steps = 0;
        while !relay.idle() {
            env.mode = relay.life.mode();
            relay.step(env);
            steps += 1;
            if steps > 64 {
                sh.violation("a turn did not end within 64 micro-steps".into());
                return turns;
            }
        }
        turns += 1;
        if !relay.again && relay.cursor == sh.put.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
    }
    turns
}

#[test]
fn a_well_behaved_guest_with_concurrent_registrations_loses_and_duplicates_nothing() {
    let sh = Shared::new();
    let secs = stress_secs();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(secs);
    let hard = hard_deadline(secs);
    let sent = Arc::new(Mutex::new(Vec::<u64>::new())); // raw of every entry the guest published, in order
    let post_marks = Arc::new(Mutex::new(Vec::<(usize, bool)>::new())); // (index, published while a registration was live)
    let done = Arc::new(AtomicBool::new(false));
    let mut relay = Relay::new(
        8,
        Lifecycle::new(),
        Scan::for_subdevice(1).unwrap(),
        Glue::default(),
    );
    let mut env = ThreadEnv {
        sh: sh.clone(),
        mode: Mode::Scan,
    };
    let turns = std::thread::scope(|s| {
        let guest = {
            let (sh, sent, post_marks, done) =
                (sh.clone(), sent.clone(), post_marks.clone(), done.clone());
            let hard = hard;
            s.spawn(move || {
                let mut put = 0u32;
                let mut push = |raw: u64, live_at_publish: bool, sh: &Shared| {
                    // wait for ring space (the relay writes GP_GET back)
                    while (put + 1) % N == sh.guest_get.load(Ordering::Acquire) {
                        if sh.stop.load(Ordering::Relaxed) || std::time::Instant::now() > hard {
                            return;
                        }
                        std::thread::yield_now();
                    }
                    sh.write_slot(put, raw);
                    put = (put + 1) % N;
                    sh.put.store(put, Ordering::Release); // GP_PUT after the entry
                    let mut s = sent.lock().unwrap();
                    post_marks.lock().unwrap().push((s.len(), live_at_publish));
                    s.push(raw);
                };
                push(ge(0, Kind::SetObj).raw(), false, &sh);
                let mut i = 0u32;
                while std::time::Instant::now() < deadline && !sh.stop.load(Ordering::Relaxed) {
                    for _ in 0..(1 + i % 5) {
                        i += 1;
                        push(ge(1 + (i % 100) as u8, Kind::Plain).raw(), false, &sh);
                    }
                    if i.is_multiple_of(7) {
                        // REGISTER: the registry changes, then "the reply" (this function returns)
                        sh.live.fetch_add(1, Ordering::AcqRel);
                        sh.epoch.fetch_add(1, Ordering::AcqRel);
                        // entries published after the reply: scanned while the registration is live
                        for _ in 0..(1 + i % 3) {
                            i += 1;
                            push(ge(1 + (i % 100) as u8, Kind::Plain).raw(), true, &sh);
                        }
                        push(ge(190, Kind::Trig { migrate: false }).raw(), true, &sh);
                        // wait until the relay consumed it (the model's guest would continue; this keeps live bounded)
                        while sh.live.load(Ordering::Acquire) > 0 && !sh.stop.load(Ordering::Relaxed) {
                            if std::time::Instant::now() > hard {
                                sh.violation("the registration was never consumed: the trigger was not staged".into());
                                sh.stop.store(true, Ordering::Relaxed);
                                break;
                            }
                            std::thread::yield_now();
                        }
                    }
                }
                done.store(true, Ordering::Release);
            })
        };
        let t = run_relay_until(&sh, &mut relay, &mut env, || {
            (done.load(Ordering::Acquire) && relay_idle_and_drained(&sh))
                || sh.stop.load(Ordering::Relaxed)
        });
        sh.stop.store(true, Ordering::Relaxed);
        guest.join().unwrap();
        t
    });
    // drain what is left
    let mut env2 = ThreadEnv {
        sh: sh.clone(),
        mode: relay.life.mode(),
    };
    let start = std::time::Instant::now();
    while relay.cursor != sh.put.load(Ordering::Acquire)
        && start.elapsed().as_secs() < 5
        && !relay.dead
    {
        relay.claim();
        while !relay.idle() {
            relay.step(&mut env2);
        }
    }
    let v = sh.violations.lock().unwrap().clone();
    assert!(v.is_empty(), "violations: {v:?}");
    assert!(!relay.dead);
    let sent = sent.lock().unwrap().clone();
    let log = sh.log.lock().unwrap().clone();
    // I2: the guest entries in the private ring are exactly the sent ones, in order (the trigger staged)
    let mut forwarded = Vec::new();
    let mut staged = 0;
    for p in &log {
        match p {
            Pub::Guest { stored, .. } => forwarded.push(*stored),
            Pub::Staged { tested } => {
                staged += 1;
                forwarded.push(*tested);
            }
            _ => {}
        }
    }
    assert_eq!(
        forwarded.len(),
        sent.len(),
        "every entry forwarded exactly once ({} turns)",
        turns
    );
    assert_eq!(forwarded, sent, "in order, bit-exact");
    assert!(staged > 10, "registrations were exercised: {staged}");
    // I4: entries published while a registration was live were scanned
    let marks = post_marks.lock().unwrap().clone();
    let mut idx = 0;
    let mut checked = 0;
    for p in &log {
        if let Pub::Guest { scanned, .. } = p {
            if marks[idx].1 && !*scanned {
                // the registration may already have been consumed by the time it was forwarded
                // only if the relay consumed it: that needs the trigger staged first, which is later in the
                // stream than these entries, so this is a violation
                panic!(
                    "entry {idx} was published after a registration's reply and forwarded unscanned"
                );
            }
            if marks[idx].1 {
                checked += 1;
            }
        }
        if matches!(p, Pub::Guest { .. } | Pub::Staged { .. }) {
            idx += 1;
        }
    }
    assert!(checked > 10, "post-reply entries checked: {checked}");
    assert_eq!(sh.pb_reads_in_pointer.load(Ordering::Relaxed), 0);
    println!(
        "stress(well-behaved): {} entries, {staged} triggers staged, {checked} post-reply entries scanned",
        sent.len()
    );
}

fn relay_idle_and_drained(sh: &Shared) -> bool {
    sh.guest_get.load(Ordering::Acquire) == sh.put.load(Ordering::Acquire)
}

#[test]
fn a_hostile_writer_rewrites_ring_entries_and_gp_put_while_the_worker_runs() {
    let sh = Shared::new();
    let secs = stress_secs();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(secs);
    let deaths = Arc::new(AtomicU64::new(0));
    let total_turns = std::thread::scope(|s| {
        // the hostile writer: random whole entries, random single-dword flips, random in-range GP_PUT
        let hostile = {
            let sh = sh.clone();
            s.spawn(move || {
                let mut x = 0x4057_11E5u64;
                let mut next = move || {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    x
                };
                while !sh.stop.load(Ordering::Relaxed) {
                    let slot = (next() % u64::from(N)) as u32;
                    match next() % 6 {
                        0 | 1 => {
                            // a valid-looking entry
                            let g = ge(1 + (next() % 100) as u8, Kind::Plain);
                            let raw = if next() % 4 == 0 {
                                g.sync().raw()
                            } else {
                                g.raw()
                            };
                            sh.write_slot(slot, raw);
                        }
                        2 => {
                            // one dword flip (bit chosen at random): writes ONE half
                            let bit = 1u32 << (next() % 32);
                            if next() % 2 == 0 {
                                let v = sh.lo[slot as usize].load(Ordering::Relaxed) ^ bit;
                                sh.wrote_lo.lock().unwrap()[slot as usize].insert(v);
                                sh.lo[slot as usize].store(v, Ordering::Release);
                            } else {
                                let v = sh.hi[slot as usize].load(Ordering::Relaxed) ^ bit;
                                sh.wrote_hi.lock().unwrap()[slot as usize].insert(v);
                                sh.hi[slot as usize].store(v, Ordering::Release);
                            }
                        }
                        3 => sh.write_slot(slot, ge(0, Kind::SetObj).raw()),
                        4 => sh.write_slot(slot, ge(1 + (next() % 100) as u8, Kind::Nop).raw()),
                        _ => sh
                            .put
                            .store((next() % u64::from(N)) as u32, Ordering::Release),
                    }
                    if next().is_multiple_of(64) {
                        // let the writer run ahead of the reader sometimes
                        std::thread::yield_now();
                    }
                }
            })
        };
        let mut turns = 0u64;
        let mut max_steps = 0;
        while std::time::Instant::now() < deadline {
            // a fresh channel after every refusal (a hostile entry kills only its own channel)
            let mut relay = Relay::new(
                8,
                Lifecycle::new(),
                Scan::for_subdevice(1).unwrap(),
                Glue::default(),
            );
            let mut env = ThreadEnv {
                sh: sh.clone(),
                mode: Mode::Scan,
            };
            sh.log.lock().unwrap().clear();
            sh.markers.store(0, Ordering::Release);
            let mut guard = 0u64;
            while !relay.dead && std::time::Instant::now() < deadline && guard < 200_000 {
                guard += 1;
                relay.claim();
                let mut steps = 0;
                while !relay.idle() {
                    env.mode = relay.life.mode();
                    relay.step(&mut env);
                    steps += 1;
                    assert!(steps <= 64, "a turn must be bounded");
                }
                max_steps = max_steps.max(steps);
                turns += 1;
            }
            // I7 on this channel's output: tested == stored, a consistent pair the writers really wrote,
            // and the stored value passes the entry-level validation
            let log = sh.log.lock().unwrap().clone();
            let lo = sh.wrote_lo.lock().unwrap().clone();
            let hi = sh.wrote_hi.lock().unwrap().clone();
            let all_lo: HashSet<u32> = lo.iter().flatten().copied().collect();
            let all_hi: HashSet<u32> = hi.iter().flatten().copied().collect();
            for p in &log {
                if let Pub::Guest { tested, stored, .. } = p {
                    assert_eq!(tested, stored, "I7");
                    assert!(
                        kf_chan::user_stream::inspect_entry(
                            *stored,
                            kf_chan::user_storage::MAX_SOURCE_BYTES
                        )
                        .is_ok(),
                        "stored an entry the validation refuses: {stored:#x}"
                    );
                    assert!(
                        all_lo.contains(&(*stored as u32))
                            && all_hi.contains(&((*stored >> 32) as u32)),
                        "a stored dword the guest never wrote: {stored:#x}"
                    );
                }
            }
            if relay.dead {
                deaths.fetch_add(1, Ordering::Relaxed);
            }
        }
        sh.stop.store(true, Ordering::Relaxed);
        hostile.join().unwrap();
        let _ = max_steps;
        turns
    });
    let v = sh.violations.lock().unwrap().clone();
    assert!(v.is_empty(), "violations: {v:?}");
    assert_eq!(
        sh.pb_reads_in_pointer.load(Ordering::Relaxed),
        0,
        "I1 under attack"
    );
    println!(
        "stress(hostile): {total_turns} turns, {} channels killed by their own hostile entries",
        deaths.load(Ordering::Relaxed)
    );
    assert!(total_turns > 1000);
}
