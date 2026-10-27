//! ENS8: production recovery/token transitions under a controllable adversarial scheduler.
//! No timer, GPU, native RM, or guessed completion is part of the model.
use super::{HostIdx, reowe};
use kf_trap::{Claim, REACT_ROUNDS, Release, Route, State, TokenWord};
use std::collections::{HashSet, VecDeque};

const GUEST: [u32; 2] = [1, 2];
const HOST: [u32; 2] = [41, 73];
const DRAIN: usize = 128;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pc {
    Scan,
    Claim(u8),
    Serve { channel: u8, rest: u8, round: u8 },
    Release { channel: u8, rest: u8, round: u8 },
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Model {
    words: [u8; 2],
    bits: u8,
    pending: u8,
    observed: u8,
    future: u8,
    hints: u8,
    held: bool,
    live: u8,
    map: [u32; 2],
    retire_peer: bool,
    workers: Vec<Pc>,
}
fn word(state: u8) -> TokenWord {
    let w = TokenWord::new();
    assert!(w.allocate_fresh(Route::Translated, HOST[0]));
    match state {
        x if x == State::Idle as u8 => {}
        x if x == State::Dead as u8 => {
            assert!(w.retire());
        }
        x => {
            w.ring(0);
            if x != State::Rung as u8 {
                assert!(matches!(w.claim(), Claim::Won(_)));
                if x == State::BusyRung as u8 {
                    w.ring(0);
                }
            }
        }
    }
    assert_eq!(w.load().state as u8, state);
    w
}
impl Model {
    fn new(workers: usize, peer: bool) -> Self {
        Self {
            words: [State::Idle as u8; 2],
            bits: 0,
            pending: 0,
            observed: 0,
            future: if peer { 3 } else { 1 },
            hints: if peer { 3 } else { 1 },
            held: true,
            live: 3,
            map: GUEST,
            retire_peer: peer,
            workers: vec![Pc::Scan; workers],
        }
    }
    fn index(&self) -> HostIdx {
        let index = HostIdx::default();
        for (i, &g) in self.map.iter().enumerate() {
            if g != 0 {
                index.insert(HOST[i], g);
            }
        }
        index
    }
    fn ring(&mut self, c: usize) {
        let w = word(self.words[c]);
        if w.ring(0) {
            self.bits |= 1 << c;
        }
        self.words[c] = w.load().state as u8;
    }
    fn edge(&mut self, c: usize) {
        self.future &= !(1 << c);
        if self.live & (1 << c) != 0 {
            self.pending |= 1 << c;
            self.ring(c);
        }
    }
    fn retire(&mut self) {
        let index = self.index();
        index.remove(HOST[1]);
        self.map = [
            index.get(HOST[0]).unwrap_or(0),
            index.get(HOST[1]).unwrap_or(0),
        ];
        self.retire_peer = false;
        // Logical retirement cancels only this channel's unobserved obligations.
        self.live &= !2;
        self.pending &= !2;
        let w = word(self.words[1]);
        let _ = w.retire(); // BUSY is left for its owner to release before physical retirement.
        self.words[1] = w.load().state as u8;
    }
    fn step_worker(&mut self, id: usize) {
        match self.workers[id] {
            Pc::Scan => {
                let queue = self.bits;
                self.bits = 0;
                self.workers[id] = Pc::Claim(queue);
            }
            Pc::Claim(0) => self.workers[id] = Pc::Scan,
            Pc::Claim(queue) => {
                let c = queue.trailing_zeros() as usize;
                let rest = queue & !(1 << c);
                let w = word(self.words[c]);
                self.workers[id] = if matches!(w.claim(), Claim::Won(_)) {
                    Pc::Serve {
                        channel: c as u8,
                        rest,
                        round: 0,
                    }
                } else {
                    Pc::Claim(rest)
                };
                self.words[c] = w.load().state as u8;
            }
            Pc::Serve {
                channel,
                rest,
                round,
            } => {
                let c = channel as usize;
                if self.live & (1 << c) != 0 {
                    if c == 0 && self.held {
                        // Exactly the native contention recovery, with the actual HostIdx lookup.
                        reowe(self.index().get(HOST[c]), |g| {
                            if let Some(peer) = GUEST.iter().position(|&t| t == g) {
                                self.ring(peer);
                            }
                        });
                    } else {
                        assert_eq!(self.observed & self.pending & (1 << c), 0, "double effect");
                        self.observed |= self.pending & (1 << c);
                        self.pending &= !(1 << c);
                    }
                }
                self.workers[id] = Pc::Release {
                    channel,
                    rest,
                    round,
                };
            }
            Pc::Release {
                channel,
                rest,
                round,
            } => {
                let c = channel as usize;
                let w = word(self.words[c]);
                self.workers[id] = match w.release(u32::from(round), REACT_ROUNDS) {
                    Release::Idled => Pc::Claim(rest),
                    Release::ActAgain => Pc::Serve {
                        channel,
                        rest,
                        round: round + 1,
                    },
                    Release::RepublishAndMoveOn => {
                        self.bits |= 1 << c;
                        Pc::Claim(rest)
                    }
                };
                self.words[c] = w.load().state as u8;
            }
        }
    }
    fn check(&self) -> Result<(), String> {
        for c in 0..2 {
            let owners = self
                .workers
                .iter()
                .filter(|p| {
                    matches!(p,
                Pc::Serve { channel, .. } | Pc::Release { channel, .. } if *channel as usize == c)
                })
                .count();
            if owners > 1 {
                return Err(format!("double-pump ownership on channel{c}"));
            }
            if self.live & self.pending & (1 << c) != 0 {
                let queued = self.workers.iter().any(|p| {
                    matches!(p, Pc::Claim(q) if q & (1 << c) != 0)
                        || matches!(p, Pc::Serve { rest, .. } | Pc::Release { rest, .. }
                        if rest & (1 << c) != 0)
                });
                if owners == 0 && !queued && self.bits & (1 << c) == 0 {
                    return Err(format!(
                        "wake lost: channel{c} pending with no owner, queue or bit"
                    ));
                }
            }
        }
        Ok(())
    }
    fn stable(&self) -> bool {
        self.pending == 0
            && self.bits == 0
            && self.workers.iter().all(|p| *p == Pc::Scan)
            && self
                .words
                .iter()
                .all(|&s| s == State::Idle as u8 || s == State::Dead as u8)
    }
    fn successors(&self) -> Vec<(String, Self)> {
        let mut next = Vec::new();
        if self.held {
            let mut s = self.clone();
            s.held = false;
            next.push(("slot holder releases".into(), s));
        }
        for c in 0..2 {
            if self.future & (1 << c) != 0 {
                let mut s = self.clone();
                s.edge(c);
                next.push((format!("doorbell/completion edge{c}"), s));
            }
            if self.hints & (1 << c) != 0 {
                let mut s = self.clone();
                s.hints &= !(1 << c);
                s.bits |= 1 << c;
                next.push((format!("stale bitmap hint{c}"), s));
            }
        }
        if self.retire_peer {
            let mut s = self.clone();
            s.retire();
            next.push(("retire peer slot".into(), s));
        }
        for w in 0..self.workers.len() {
            if self.workers[w] == Pc::Scan && self.bits == 0 {
                continue;
            }
            let mut s = self.clone();
            s.step_worker(w);
            next.push((format!("worker{w} {:?}", self.workers[w]), s));
        }
        next
    }
    fn drain(mut self) -> Result<(), String> {
        // Fair suffix after every finite adversarial prefix. No timer or later external wake.
        self.held = false;
        for c in 0..2 {
            if self.future & (1 << c) != 0 {
                self.edge(c);
            }
        }
        self.hints = 0;
        for step in 0..DRAIN {
            self.check()?;
            if self.stable() {
                return Ok(());
            }
            let worker = step % self.workers.len();
            self.step_worker(worker);
        }
        Err(format!(
            "not quiescent after{DRAIN} fair microsteps: {self:?}"
        ))
    }
}
fn contended(kind: &str) {
    let mut s = Model::new(1, false);
    s.future = 0;
    s.hints = 0;
    // Host fence was released (or PUT advanced), then exactly one edge, no future pump source.
    s.pending = 1;
    s.ring(0);
    s.step_worker(0); // scan consumes the only bitmap bit
    s.step_worker(0); // token becomes BUSY
    s.step_worker(0); // try_lock contends; must re-owe before leaving
    s.step_worker(0); // release must preserve the ring
    s.check().unwrap_or_else(|e| {
        panic!("{kind}: edge -> scan -> claim -> contended serve -> release: {e}")
    });
    s.drain().unwrap_or_else(|e| panic!("{kind}: {e}"));
}
#[test]
fn contended_try_lock_completion_edge_with_no_later_pump() {
    contended("completion");
}
#[test]
fn contended_try_lock_doorbell_edge_with_no_later_pump() {
    contended("doorbell");
}

fn explore(workers: usize, peer: bool) -> usize {
    let start = Model::new(workers, peer);
    let mut visited = HashSet::from([start.clone()]);
    let mut queue = VecDeque::from([(start, Vec::<String>::new())]);
    while let Some((s, trace)) = queue.pop_front() {
        s.check()
            .unwrap_or_else(|e| panic!("{e}; shortest trace={trace:?}"));
        s.clone()
            .drain()
            .unwrap_or_else(|e| panic!("{e}; prefix={trace:?}"));
        for (step, next) in s.successors() {
            if visited.insert(next.clone()) {
                let mut path = trace.clone();
                path.push(step);
                queue.push_back((next, path));
            }
        }
        assert!(
            visited.len() <= 1_000_000,
            "inconclusive: model state budget exhausted"
        );
    }
    eprintln!(
        "ENS8 exhaustive workers={workers} peer={peer} states={}",
        visited.len()
    );
    visited.len()
}
#[test]
fn exhaustive_one_worker_releases_every_finite_contention_without_a_timer() {
    assert!(explore(1, false) > 20);
}
#[test]
fn exhaustive_two_workers_preserve_peer_wakes_through_slot_teardown() {
    assert!(explore(2, true) > 100);
}
fn seeded(seed: u64) -> Vec<String> {
    let mut rng = seed;
    let mut s = Model::new(2, true);
    let mut trace = Vec::new();
    for _ in 0..256 {
        s.check()
            .unwrap_or_else(|e| panic!("seed={seed} {e}: {trace:?}"));
        let next = s.successors();
        if next.is_empty() {
            break;
        }
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let (step, n) = next[(rng % next.len() as u64) as usize].clone();
        trace.push(step);
        s = n;
    }
    s.drain()
        .unwrap_or_else(|e| panic!("seed={seed} fair suffix: {e}; {trace:?}"));
    trace
}
#[test]
fn seeds_1_through_512_replay_identically_with_two_workers_and_peer_retirement() {
    for seed in 1..=512 {
        assert_eq!(seeded(seed), seeded(seed));
    }
}
fn peer_retirement(kind: &str) {
    let mut s = Model::new(1, true);
    s.future = 0;
    s.hints = 0;
    s.pending = 1;
    s.ring(0);
    s.step_worker(0);
    s.step_worker(0);
    s.retire(); // removes the OTHER host slot before our contended serve resolves its identity
    s.step_worker(0);
    s.step_worker(0);
    s.check()
        .unwrap_or_else(|e| panic!("{kind}: peer teardown lost a live wake: {e}"));
    s.drain().unwrap();
}
#[test]
fn peer_slot_teardown_cannot_lose_a_live_completion_ring() {
    peer_retirement("completion");
}
#[test]
fn peer_slot_teardown_cannot_lose_a_live_doorbell_ring() {
    peer_retirement("doorbell");
}
#[test]
fn a_busy_token_excludes_a_second_pump_owner() {
    let w = word(State::Busy as u8);
    assert_eq!(w.claim(), Claim::NotOurs);
}
#[test]
fn a_busy_rung_token_excludes_a_second_pump_owner() {
    let w = word(State::BusyRung as u8);
    assert_eq!(w.claim(), Claim::NotOurs);
}
#[test]
fn a_retired_slot_cannot_reowe_by_its_old_identity() {
    let map = HostIdx::default();
    map.insert(HOST[0], GUEST[0]);
    map.remove(HOST[0]);
    reowe(map.get(HOST[0]), |_| {
        panic!("retired host must not ring anything")
    });
}
#[test]
fn a_reused_host_slot_reowes_only_the_new_guest_identity() {
    let map = HostIdx::default();
    map.insert(HOST[0], GUEST[0]);
    map.remove(HOST[0]);
    map.insert(HOST[0], GUEST[1]);
    let mut called = None;
    reowe(map.get(HOST[0]), |g| called = Some(g));
    assert_eq!(called, Some(GUEST[1]));
    assert_eq!(map.0.read().unwrap().len(), 1);
}
#[test]
fn hostile_unknown_host_tokens_cannot_grow_the_owed_set() {
    let map = HostIdx::default();
    map.insert(HOST[0], GUEST[0]);
    let w = word(State::Busy as u8);
    for h in 1000..11000 {
        reowe(map.get(h), |_| panic!("unknown host"));
    }
    for _ in 0..10000 {
        reowe(map.get(HOST[0]), |_| {
            w.ring(0);
        });
    }
    assert_eq!(map.0.read().unwrap().len(), 1);
    assert_eq!(w.load().state, State::BusyRung);
    assert_eq!(
        w.release(REACT_ROUNDS, REACT_ROUNDS),
        Release::RepublishAndMoveOn
    );
}

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
struct RealWorker<'a, 'v> {
    plane: &'a kf_core::Plane<'v>,
    index: HostIdx,
    slot: std::sync::Mutex<()>,
    pending: AtomicBool,
    pumps: AtomicUsize,
    effects: AtomicUsize,
    contention: AtomicUsize,
}
impl kf_core::HostOps for RealWorker<'_, '_> {
    fn run_translated(&self, host: u32, _: u64) -> bool {
        assert_eq!(host, HOST[0]);
        let Ok(_slot) = self.slot.try_lock() else {
            self.contention.fetch_add(1, Ordering::Relaxed);
            reowe(self.index.get(host), |g| {
                let _ = self.plane.ring_internal(g);
            });
            return false;
        };
        self.pumps.fetch_add(1, Ordering::Relaxed);
        if self.pending.swap(false, Ordering::AcqRel) {
            self.effects.fetch_add(1, Ordering::Relaxed);
        }
        false
    }
    fn operands_translatable(&self, _: u32, _: u64) -> kf_core::Translatable {
        kf_core::Translatable::Yes
    }
    fn ring_host(&self, _: u32) {
        panic!("not passthrough");
    }
    fn run_emulated(&self, _: u32, _: u64) {
        panic!("not emulated");
    }
    fn apply_register(&self, _: u8, _: u32, _: u64, _: u8) {
        panic!("no registers");
    }
    fn forge_completion(&self, _: u32) {
        panic!("translated completion is never forged");
    }
    fn refuse_and_poison(&self, _: u32) {
        panic!("no refusal");
    }
    fn fault_channel(&self, _: u32) {
        panic!("no fault");
    }
    fn map_guest_slice(&self, _: kf_core::HostSlice) {
        panic!("no mapping");
    }
    fn teardown_step(&self, _: kf_core::Step) {
        panic!("no device teardown");
    }
}
fn actual_worker(completion: bool) {
    for runlist in [false, true] {
        actual_worker_indexed(completion, runlist);
    }
}
fn actual_worker_indexed(completion: bool, runlist: bool) {
    let vmm = kf_core::Vmm::new();
    let plane = if runlist {
        kf_core::Plane::for_device(&vmm, 8, 7, kf_chip::Family::Ampere, 16 << 20)
    } else {
        kf_core::Plane::new(&vmm, 8, 7)
    };
    let guest = if runlist { 0x809 } else { GUEST[0] };
    let doorbell = if runlist { (1 << 16) | 9 } else { guest };
    let mut caps = kf_core::VmCaps::from_declared(8, 8, 8, 8);
    plane
        .allocate_channel(
            &mut caps,
            guest,
            Route::Translated,
            HOST[0],
            kf_core::Owner::User,
        )
        .unwrap();
    let host = RealWorker {
        plane: &plane,
        index: HostIdx::default(),
        slot: std::sync::Mutex::new(()),
        pending: AtomicBool::new(true),
        pumps: AtomicUsize::new(0),
        effects: AtomicUsize::new(0),
        contention: AtomicUsize::new(0),
    };
    host.index.insert(HOST[0], guest);
    let held = host.slot.lock().unwrap();
    if completion {
        let _ = plane.ring_internal(guest);
    } else {
        let _ = plane.trap_write(kf_trap::Class::Doorbell, 0, 0x90, u64::from(doorbell), 4);
    }
    // The act-side slot lock stays held while a real worker thread must return its bounded pass.
    let (tx, rx) = std::sync::mpsc::channel();
    let result = std::thread::scope(|scope| {
        let child = scope.spawn(|| {
            let n = plane.worker_pass(&host, &mut Vec::new(), 8);
            tx.send((n, plane.tokens[guest as usize].load().state))
                .unwrap();
        });
        let result = rx.recv_timeout(std::time::Duration::from_secs(2));
        drop(held); // always release before joining, including on a broken/blocking mutant
        child.join().unwrap();
        result
    })
    .expect("worker blocked on another thread's slot lock");
    assert_eq!(result.0, (REACT_ROUNDS + 1) as usize);
    assert_eq!(
        result.1,
        State::Rung,
        "the owed bit must survive the bounded visit"
    );
    assert_eq!(host.effects.load(Ordering::Relaxed), 0);
    // No further doorbell/completion event. Re-published debt alone must pump exactly once.
    plane.worker_pass(&host, &mut Vec::new(), 8);
    assert_eq!(host.pumps.load(Ordering::Relaxed), 1);
    assert_eq!(host.effects.load(Ordering::Relaxed), 1);
    assert_eq!(
        plane.worker_pass(&host, &mut Vec::new(), 8),
        0,
        "no perpetual re-owe spin"
    );
    assert!(plane.free_channel(&mut caps, guest));
}
#[test]
fn real_worker_completion_survives_a_slot_held_by_another_thread() {
    actual_worker(true);
}
#[test]
fn real_worker_doorbell_survives_a_slot_held_by_another_thread() {
    actual_worker(false);
}
