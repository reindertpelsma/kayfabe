//! 2026-10-20 -- **Grader group G8: lock / wake protocol interleavings** (`docs/design/V3_RELAY_GRADER_20261020.md`).
//!
//! An exhaustive interleaving explorer (own, std only: loom 0.7.2 is in the local cargo cache but not a dependency of the
//! workspace; adding it would change `Cargo.lock` for every crate, so the explorer is local) over every two- and three-thread
//! interleaving of:
//! * the REAL wake word (`kf_trap::WakeWord`: `seen` / `try_park` / `bump` / `unpark`) and the REAL token word
//!   (`kf_trap::TokenWord`: `ring` / `claim` / `release`) -- the doorbell (vCPU) vs worker park protocol and the claim / again
//!   protocol, each step one real atomic operation;
//! * the relay's lost-lock protocol (record / `pt` / storage / admission) as the LINEAGE implements it (`grade_adapter.rs`:
//!   `look`, `unsourced`, the budget), against a holder that rings on release (the act thread) or rings nobody (`pt`, storage);
//! * NEGATIVE CONTROLS that the explorer must find red (its own validity): the naive waiter registration (the LOST
//!   REGISTRATION interleaving), a park that ignores the sequence, a ring that is dropped while the token is busy.
#![allow(dead_code, missing_docs, clippy::too_many_lines)]

#[path = "grade_adapter.rs"]
mod adapter;

use adapter::{AWait, Verdict, View, Why};
use kf_trap::{Claim, Release, Route, State, TokenWord, Wake, WakeWord};
use std::collections::{HashSet, VecDeque};
use std::hash::Hash;

trait Proto {
    type S: Clone + Eq + Hash + std::fmt::Debug;
    fn init(&self) -> Self::S;
    fn threads(&self) -> usize;
    fn step(&self, s: &Self::S, t: usize) -> Option<Self::S>;
    /// A violation visible in one state.
    fn bad(&self, s: &Self::S) -> Option<String>;
    /// A violation of a terminal state (nothing can step).
    fn stuck(&self, s: &Self::S) -> Option<String>;
}

/// Every interleaving (BFS over hashed states). Returns the first violation and the state count.
fn explore<P: Proto>(p: &P, cap: usize) -> (Option<String>, usize) {
    let s0 = p.init();
    let mut seen: HashSet<P::S> = HashSet::from([s0.clone()]);
    let mut q = VecDeque::from([s0]);
    while let Some(s) = q.pop_front() {
        assert!(seen.len() <= cap, "state cap {cap}");
        if let Some(b) = p.bad(&s) {
            return (Some(b), seen.len());
        }
        let mut any = false;
        for t in 0..p.threads() {
            if let Some(n) = p.step(&s, t) {
                any = true;
                if seen.insert(n.clone()) {
                    q.push_back(n);
                }
            }
        }
        if !any {
            if let Some(b) = p.stuck(&s) {
                return (Some(b), seen.len());
            }
        }
    }
    (None, seen.len())
}

// ----------------------------------------------------------------------------------------------------------------------------
// 1. the doorbell (vCPU) vs worker park protocol and the claim / again protocol, on the REAL atomics

fn token_in(state: State) -> TokenWord {
    let w = TokenWord::new();
    assert!(w.allocate_fresh(Route::Translated, 1) || w.allocate(Route::Translated, 1));
    match state {
        State::Idle => {}
        State::Rung => {
            w.ring(0);
        }
        State::Busy => {
            w.ring(0);
            assert!(matches!(w.claim(), Claim::Won(_)));
        }
        State::BusyRung => {
            w.ring(0);
            assert!(matches!(w.claim(), Claim::Won(_)));
            w.ring(0);
        }
        State::Dead => unreachable!(),
    }
    assert_eq!(w.load().state, state);
    w
}

fn wake_in(seq: u64, pollers: u64) -> WakeWord {
    let w = WakeWord::new();
    for _ in 0..seq {
        w.bump();
    }
    for _ in 0..pollers {
        assert!(w.try_park(seq));
    }
    w
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum WPc {
    Seen,
    Scan,
    Serve,
    Release,
    Decide,
    TryPark,
    Parked,
    Unpark,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct DbState {
    tok: u8,
    seq: u64,
    pollers: u64,
    bit: bool,
    efd: bool,
    w: Vec<(WPc, u64, bool, u32)>, // pc, seen, served-this-pass, rounds
    rings_left: u8,
    v_pc: u8,
    rings_done: u8,
    serves_after_last_ring: bool,
}

fn st_of(n: u8) -> State {
    [State::Idle, State::Rung, State::Busy, State::BusyRung][n as usize]
}
fn n_of(s: State) -> u8 {
    match s {
        State::Idle => 0,
        State::Rung => 1,
        State::Busy => 2,
        State::BusyRung => 3,
        State::Dead => 9,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DbVariant {
    Real,
    /// NEGATIVE CONTROL: the worker parks without comparing the sequence it saw before its scan.
    ParkIgnoresSeq,
    /// NEGATIVE CONTROL: a ring on a BUSY token is dropped (no `BusyRung`, no `ActAgain` / republish).
    RingDroppedWhileBusy,
}

struct Doorbell {
    workers: usize,
    rings: u8,
    k: u32,
    v: DbVariant,
}

impl Proto for Doorbell {
    type S = DbState;
    fn init(&self) -> DbState {
        DbState {
            tok: 0,
            seq: 0,
            pollers: 0,
            bit: false,
            efd: false,
            w: vec![(WPc::Seen, 0, false, 0); self.workers],
            rings_left: self.rings,
            v_pc: 0,
            rings_done: 0,
            serves_after_last_ring: true,
        }
    }
    fn threads(&self) -> usize {
        self.workers + 1
    }
    fn step(&self, s: &DbState, t: usize) -> Option<DbState> {
        let mut n = s.clone();
        let tok = token_in(st_of(s.tok));
        let wake = wake_in(s.seq, s.pollers);
        if t == self.workers {
            // the vCPU: GP_PUT, then the doorbell trap: ring (real), publish, bump (real), signal when told
            match s.v_pc {
                0 if s.rings_left > 0 => {
                    let newly = if self.v == DbVariant::RingDroppedWhileBusy && matches!(st_of(s.tok), State::Busy | State::BusyRung) {
                        false
                    } else {
                        tok.ring(0)
                    };
                    n.tok = n_of(tok.load().state);
                    n.rings_left -= 1;
                    n.rings_done += 1;
                    n.serves_after_last_ring = false;
                    n.v_pc = if newly { 1 } else { 0 };
                }
                1 => {
                    n.bit = true;
                    n.v_pc = 2;
                }
                2 => {
                    if wake.bump() == Wake::SignalOne {
                        n.efd = true;
                    }
                    n.seq = wake.seen();
                    n.v_pc = 0;
                }
                _ => return None,
            }
            return Some(n);
        }
        let (pc, seen, served, rounds) = s.w[t];
        match pc {
            WPc::Seen => {
                n.w[t] = (WPc::Scan, wake.seen(), false, 0);
            }
            WPc::Scan => {
                if s.bit {
                    n.bit = false;
                    match tok.claim() {
                        Claim::Won(_) => {
                            n.tok = n_of(tok.load().state);
                            n.w[t] = (WPc::Serve, seen, true, 0);
                        }
                        Claim::NotOurs => n.w[t] = (WPc::Decide, seen, served, 0),
                    }
                } else {
                    n.w[t] = (WPc::Decide, seen, served, 0);
                }
            }
            WPc::Serve => {
                // the turn reads every GP_PUT written before the doorbell it serves
                n.serves_after_last_ring = true;
                n.w[t] = (WPc::Release, seen, true, rounds + 1);
            }
            WPc::Release => {
                let r = tok.release(rounds, self.k);
                n.tok = n_of(tok.load().state);
                n.w[t] = match r {
                    Release::Idled => (WPc::Decide, seen, true, 0),
                    Release::ActAgain => (WPc::Serve, seen, true, rounds),
                    Release::RepublishAndMoveOn => {
                        n.bit = true;
                        (WPc::Decide, seen, true, 0)
                    }
                };
            }
            WPc::Decide => {
                n.w[t] = if served {
                    (WPc::Seen, 0, false, 0)
                } else {
                    (WPc::TryPark, seen, false, 0)
                };
            }
            WPc::TryPark => {
                let ok = if self.v == DbVariant::ParkIgnoresSeq {
                    wake.try_park(wake.seen())
                } else {
                    wake.try_park(seen)
                };
                n.pollers = wake.pollers();
                n.w[t] = if ok { (WPc::Parked, seen, false, 0) } else { (WPc::Seen, 0, false, 0) };
            }
            WPc::Parked => {
                if !s.efd {
                    return None;
                }
                n.efd = false;
                n.w[t] = (WPc::Unpark, seen, false, 0);
            }
            WPc::Unpark => {
                wake.unpark();
                n.pollers = wake.pollers();
                n.w[t] = (WPc::Seen, 0, false, 0);
            }
        }
        Some(n)
    }
    fn bad(&self, _s: &DbState) -> Option<String> {
        None
    }
    fn stuck(&self, s: &DbState) -> Option<String> {
        // terminal: every doorbell rung; the token must be idle (served) and nothing pending
        let st = st_of(s.tok);
        if st != State::Idle || s.bit {
            return Some(format!("a rung token stranded with every worker parked: {s:?}"));
        }
        if !s.serves_after_last_ring {
            return Some(format!("the last doorbell was never served: {s:?}"));
        }
        None
    }
}

/// G8: the real wake word and token word: no doorbell is lost against 1 or 2 parking workers, with 1..3 doorbells, k = 1, 2.
#[test]
fn g8_lock_protocol_real_wake_and_token_words_lose_no_doorbell() {
    for workers in [1, 2] {
        for rings in [1, 2, 3] {
            for k in [1, 2] {
                let (bad, n) = explore(
                    &Doorbell {
                        workers,
                        rings,
                        k,
                        v: DbVariant::Real,
                    },
                    2_000_000,
                );
                println!("doorbell W={workers} rings={rings} k={k}: {n} states");
                assert!(bad.is_none(), "W={workers} rings={rings} k={k}: {bad:?}");
            }
        }
    }
}

/// G8 explorer validity: a park that ignores the sequence and a ring dropped while busy are both found.
#[test]
fn g8_lock_protocol_negative_controls_are_red() {
    for v in [DbVariant::ParkIgnoresSeq, DbVariant::RingDroppedWhileBusy] {
        let (bad, _) = explore(
            &Doorbell {
                workers: 1,
                rings: 2,
                k: 1,
                v,
            },
            2_000_000,
        );
        assert!(bad.is_some(), "the explorer must find the defect of the negative control");
    }
}

// ----------------------------------------------------------------------------------------------------------------------------
// 2. waiter registration: the LOST REGISTRATION interleaving

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct RegState {
    lock: bool,
    epoch: u8,
    registered: bool,
    rung: bool,
    loser_pc: u8,
    loser_seen: u8,
    holder_pc: u8,
    loser_parked: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RegVariant {
    /// NEGATIVE CONTROL: check the lock, register, park; the holder releases then reads the (maybe empty) list.
    Naive,
    /// The event-count fix: the loser reads the epoch BEFORE its attempt and re-checks it after registering; the holder bumps
    /// the epoch BEFORE it reads the list.
    EventCount,
}

struct Registration {
    v: RegVariant,
}

impl Proto for Registration {
    type S = RegState;
    fn init(&self) -> RegState {
        RegState {
            lock: true,
            epoch: 0,
            registered: false,
            rung: false,
            loser_pc: 0,
            loser_seen: 0,
            holder_pc: 0,
            loser_parked: false,
        }
    }
    fn threads(&self) -> usize {
        2
    }
    fn step(&self, s: &RegState, t: usize) -> Option<RegState> {
        let mut n = s.clone();
        if t == 1 {
            // the holder: release; (EventCount: bump the epoch); read the list and ring
            match s.holder_pc {
                0 => {
                    n.lock = false;
                    n.holder_pc = 1;
                }
                1 => {
                    if self.v == RegVariant::EventCount {
                        n.epoch += 1;
                    }
                    n.holder_pc = 2;
                }
                2 => {
                    if s.registered {
                        n.rung = true;
                    }
                    n.holder_pc = 3;
                }
                _ => return None,
            }
            return Some(n);
        }
        match s.loser_pc {
            0 => {
                n.loser_seen = s.epoch;
                n.loser_pc = 1;
            }
            1 => {
                // the attempt
                if s.lock {
                    n.loser_pc = 2;
                } else {
                    n.loser_pc = 9; // won
                }
            }
            2 => {
                n.registered = true;
                n.loser_pc = 3;
            }
            3 => {
                if self.v == RegVariant::EventCount && s.epoch != s.loser_seen {
                    n.loser_pc = 0; // retry at once
                } else {
                    n.loser_parked = true;
                    n.loser_pc = 4;
                }
            }
            4 => {
                if !s.rung {
                    return None;
                }
                n.rung = false;
                n.loser_parked = false;
                n.registered = false;
                n.loser_pc = 0;
            }
            _ => return None,
        }
        Some(n)
    }
    fn bad(&self, _s: &RegState) -> Option<String> {
        None
    }
    fn stuck(&self, s: &RegState) -> Option<String> {
        (s.loser_pc != 9).then(|| format!("LOST REGISTRATION: the loser sleeps on a free lock nobody will announce: {s:?}"))
    }
}

/// G8: the naive waiter registration (loser checks the lock, holder releases and reads the empty list, loser registers after)
/// MUST be red -- the explorer's validity on the interleaving the owner named.
#[test]
fn g8_lost_registration_is_red_for_the_naive_protocol() {
    let (bad, _) = explore(&Registration { v: RegVariant::Naive }, 10_000);
    assert!(
        bad.as_deref().is_some_and(|b| b.contains("LOST REGISTRATION")),
        "the naive registration must strand: {bad:?}"
    );
    let (ok, _) = explore(&Registration { v: RegVariant::EventCount }, 10_000);
    assert!(ok.is_none(), "the event-count registration closes it: {ok:?}");
}

// ----------------------------------------------------------------------------------------------------------------------------
// 3. the relay's lost-lock protocol, as the lineage implements it

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum Holds {
    /// The act thread on the record: rings on release (`user_ring_wake`).
    RecordAct,
    /// `pt` / storage / admission / a peer: rings nobody.
    Silent,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct LockState {
    lock_held_by_holder: bool,
    holds_left: u8,
    demand: bool,
    wait: Option<AWait>,
    waiting: bool,
    crun: u8,
    rung: bool,
    efd: bool,
    served: bool,
    /// per worker: 0 attempt, 1 look, 2 decide, 3 parked
    w: Vec<u8>,
    w_rang: Vec<bool>,
    seq: u8,
    w_seen: Vec<u8>,
}

struct LostLock {
    workers: usize,
    holds: u8,
    kind: Holds,
}

impl LostLock {
    fn crun_cap() -> u8 {
        adapter::BUDGET.map_or(0, |b| u8::try_from(b + 1).unwrap_or(u8::MAX))
    }
    fn ring(n: &mut LockState) {
        if !n.rung {
            n.rung = true;
            n.efd = true;
            n.seq = (n.seq + 1) % 4;
        }
    }
}

impl Proto for LostLock {
    type S = LockState;
    fn init(&self) -> LockState {
        LockState {
            lock_held_by_holder: true,
            holds_left: self.holds,
            demand: true,
            wait: None,
            waiting: false,
            crun: 0,
            rung: true,
            efd: true,
            served: false,
            w: vec![0; self.workers],
            w_rang: vec![false; self.workers],
            seq: 1,
            w_seen: vec![0; self.workers],
        }
    }
    fn threads(&self) -> usize {
        self.workers + 1
    }
    fn step(&self, s: &LockState, t: usize) -> Option<LockState> {
        let mut n = s.clone();
        if t == self.workers {
            if s.lock_held_by_holder {
                n.lock_held_by_holder = false;
                n.holds_left -= 1;
                if self.kind == Holds::RecordAct {
                    n.demand = true;
                    Self::ring(&mut n);
                    n.efd = true;
                }
            } else if s.holds_left > 0 && !s.served {
                n.lock_held_by_holder = true;
            } else {
                return None;
            }
            return Some(n);
        }
        match s.w[t] {
            0 => {
                // a pass: claim the rung token and serve: enter() = request(), then the lock attempt
                n.w_seen[t] = s.seq;
                n.w_rang[t] = false;
                if s.rung {
                    n.rung = false;
                    n.demand = true;
                    if s.lock_held_by_holder {
                        n.crun = (s.crun + 1).min(Self::crun_cap());
                        // the old arm: Wait::Contended; the simple arm: demand stays
                        if let Some(w) = adapter::contended_wait() {
                            n.wait = Some(w);
                            n.waiting = true;
                        }
                        if !adapter::contended_leaves_demand() {
                            n.demand = true; // enter() set it; the serve returned before the swap
                        }
                    } else {
                        n.demand = false;
                        n.served = true;
                        n.wait = None;
                        n.waiting = false;
                        n.crun = 0;
                    }
                    n.w_rang[t] = true; // served something: the loop goes round
                }
                n.w[t] = 1;
            }
            1 => {
                let v = View {
                    why: Why::Spin,
                    demand: s.demand,
                    record_busy: false,
                    waiting_flag: s.waiting,
                    wait: s.wait,
                    satisfied: false,
                    storage_busy: false,
                    crun: u32::from(s.crun),
                };
                if let Verdict::Ring { clear_wait } = adapter::look(v) {
                    if clear_wait {
                        n.wait = None;
                        n.waiting = false;
                    }
                    Self::ring(&mut n);
                    n.w_rang[t] = true;
                }
                n.w[t] = 2;
            }
            2 => {
                let spin = s.w_rang[t] || adapter::unsourced(s.demand, s.waiting, false, false);
                n.w[t] = if spin {
                    n.efd = false;
                    0
                } else if s.seq != s.w_seen[t] {
                    0
                } else {
                    3
                };
            }
            3 => {
                if !s.efd {
                    return None;
                }
                n.efd = false;
                n.w[t] = 0;
            }
            _ => unreachable!(),
        }
        Some(n)
    }
    fn bad(&self, _s: &LockState) -> Option<String> {
        None
    }
    fn stuck(&self, s: &LockState) -> Option<String> {
        (!s.served).then(|| format!("{}: the relay that lost its lock is stranded after the holder left: {s:?}", adapter::LINEAGE))
    }
}

/// G8: the lineage's lost-lock protocol (record / pt / storage / admission; act-thread holder and silent holders), 1 and 2
/// workers plus the holder (two- and three-thread interleavings), a hold long enough to exhaust any retry budget: the relay is
/// served after the holder leaves, without a timer.
#[test]
fn g8_lineage_protocol_never_strands() {
    let mut bad = Vec::new();
    for workers in [1, 2] {
        for kind in [Holds::RecordAct, Holds::Silent] {
            for holds in [1, 2] {
                let (b, n) = explore(&LostLock { workers, holds, kind }, 4_000_000);
                println!("{} lost-lock W={workers} {kind:?} holds={holds}: {n} states", adapter::LINEAGE);
                if let Some(b) = b {
                    bad.push(format!("W={workers} {kind:?} holds={holds}: {b}"));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
