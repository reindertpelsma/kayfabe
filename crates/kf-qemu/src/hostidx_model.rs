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
                let queued = self
                    .workers
                    .iter()
                    .any(|p| matches!(p, Pc::Claim(q) if q & (1 << c) != 0));
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
