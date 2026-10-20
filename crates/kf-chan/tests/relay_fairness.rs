//! 2026-10-20 -- **The worker loop over the USER-ring relays: bounded fairness, the lock holder's hand-off, and the sleep invariant**
//! (P11, P12, P13, P14, P15). Owner design, binding: no heuristics (no retry budget, no "every Nth pass", no timer as correctness); one big
//! loop that drains every fd, evaluates every relay's recorded state and serves with a per-visit bound; the worker blocks (`epoll_wait`
//! without a timeout) ONLY when every obligation any relay holds has a registered wake source that reaches every worker; a relay that loses a
//! `try_lock` registers with the holder BEFORE giving up and the holder rings it after releasing; the 50 ms park timeout may stay as a safety
//! net but a firing that finds work is a counted bug and everything here is checked as if it were infinite.
//!
//! Hardware showed STARVATION on the relay route (6-59 s for one client while another ran thousands of iterations; nothing stuck, the
//! safety-net counters 0). A finite-schedule liveness check cannot see that, so every property is a BOUND: whatever the schedule, a satisfied
//! wait, an engine event, a lost-and-released lock or a flagged token is attended to within `K` worker passes.
//!
//! CODE READ before the fix (`worker::run_with_relay`, `kf_core::Plane::worker_pass`): the loop looked at the world only when a pass served
//! NOTHING; a hot relay rings its own token after every turn, so one worker (or two workers handing the hot token to each other, the baton)
//! never looked. Contended relays were re-rung by looks on a budget (`CONTENTION_RETRIES`) and then only by the safety net.
//!
//! MODELLED here: W workers (1, 2, 3) each with its own poller (every wake source reaches every worker, level-triggered), the token layer
//! (`kf-trap` claim / release / republish, `REACT_ROUNDS`), hot relays that ring themselves, one cold relay with a wait (marker: announced by
//! the engine event; carried `Popped`: announced by nothing; visibility lag: the event comes first), a lock holder that takes and releases
//! the `pt` / admission lock and then runs the hand-off micro-steps (bump the epoch, look at the registry, ring), the loser's micro-steps
//! (read the epoch, try, register, re-check) and the park with its sleep invariant. The production pure functions drive it:
//! `user_loop::{look_needed, idle, may_block, wake_source, Wait::satisfied}`. The hand-off protocol itself is checked on the REAL
//! `Handoff` in `handoff_protocol.rs`.
#![allow(
    clippy::too_many_lines,
    clippy::needless_range_loop,
    dead_code,
    missing_docs
)]

use kf_chan::user_loop::{self as ul, Obligation, Peek, Wait};
use kf_trap::REACT_ROUNDS;
use std::collections::{HashMap, HashSet, VecDeque};

/// The bound, in worker passes (all workers' passes counted).
const K: u8 = 12;

const V_P11: u32 = 1; // a satisfied (visible) wait of the cold relay was not rung within K passes
const V_P12: u32 = 2; // an engine event was not dispatched within K passes
const V_P14: u32 = 4; // a relay that lost a try_lock was not re-rung within K passes after the lock was free and the holder done
const V_TOKEN: u32 = 8; // the cold relay's flagged token was not claimed within 2 passes
const V_SLEEP: u32 = 16; // a worker is blocked while an obligation has no wake source
const V_NET: u32 = 32; // the park timeout (safety net) fired and found work: a bug

fn names(v: u32) -> Vec<&'static str> {
    let mut o = Vec::new();
    for (b, n) in [
        (
            V_P11,
            "P11 a satisfied wait of the cold relay waited more than K passes",
        ),
        (
            V_P12,
            "P12 an engine event was not dispatched within K passes (the guest's completion interrupt is late)",
        ),
        (
            V_P14,
            "P14 a relay that lost a try_lock was not re-rung within K passes of the lock being free",
        ),
        (
            V_TOKEN,
            "TOKEN the cold relay's flagged token was not claimed within 2 passes",
        ),
        (
            V_SLEEP,
            "SLEEP a worker blocked while an obligation had no wake source",
        ),
        (V_NET, "NET the safety net fired and found work"),
    ] {
        if v & b != 0 {
            o.push(n);
        }
    }
    o
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum T {
    Idle,
    Rung,
    Busy,
    BusyRung,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pc {
    Scan,
    Claim,
    Serve,
    After { served: u8 },
    Park,
    Parked,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Wk {
    pc: Pc,
    queue: Vec<u8>,
    cur: u8,
    round: u32,
    n: u32,
    /// The cold relay's serve micro-step (0 read the epoch, 1 try, 2 register, 3 re-check).
    cs: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct Mutant {
    /// The evaluation does not dispatch engine events.
    eval_skips_events: bool,
    /// The evaluation does not peek the waiting relays.
    eval_skips_peek: bool,
    /// The RETIRED loop: evaluate only after a pass that served nothing.
    eval_only_when_idle: bool,
    /// The worker blocks even though an obligation has no wake source.
    park_ignores_unannounced: bool,
    /// An event that finds the words not yet visible does not mark the relay (the spent hint is forgotten).
    forgets_lagging: bool,
}

/// The hand-off protocols the model can run (the real one is `kf_chan::handoff::Handoff`, checked on its own in `handoff_protocol.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handoff {
    /// The retired behaviour: the holder releases and tells nobody.
    None,
    /// NAIVE: the loser registers AFTER its failed attempt and gives up; loses the registration that arrives after the holder's look.
    Naive,
    /// The real protocol: the loser reads the epoch before the attempt, registers, then re-checks the epoch (retry at once if it moved).
    Epoch,
}

#[derive(Clone, Copy, Debug)]
enum WaitKind {
    /// Announced by the marker's engine interrupt.
    Marker,
    /// Carried: nothing announces it (pops, `Wait::Popped`).
    Popped,
}

#[derive(Clone, Debug)]
struct Cfg {
    workers: usize,
    /// Number of hot relays (token indices `0..hot`); the cold relay is index `hot`.
    hot: usize,
    /// The cold relay's recorded wait (`None`: nothing outstanding).
    wait: Option<WaitKind>,
    /// The engine interrupt comes BEFORE the words are visible (the hint arrives first).
    lag: bool,
    /// How many times the adversary (another thread) takes the lock the cold relay needs (`pt` / USERD admission).
    holds: u8,
    /// The lock is held when the exploration starts (the cold relay's first serve meets it).
    lock_at_start: bool,
    handoff: Handoff,
    /// Doorbells the cold guest rings.
    cold_rings: u8,
    /// The cold relay starts flagged (a doorbell is pending).
    cold_flagged: bool,
    irq: bool,
    m: Mutant,
    /// Which of the bounds are tracked (the counters multiply the state space).
    track: u32,
    /// The parked worker's 50 ms park timeout exists (the safety net). Off by default: everything is checked as if it were infinite.
    timer: bool,
    delta: u8,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct S {
    tok: Vec<T>,
    bit: Vec<bool>,
    wait: Option<Wait>,
    sat: bool,
    vis: bool,
    lagging: bool,
    demand: bool,
    held: bool,
    holds_left: u8,
    epoch: u8,
    seen: u8,
    reg: bool,
    hpc: u8,
    rings_left: u8,
    ev: bool,
    efd: bool,
    moved: bool,
    wk: Vec<Wk>,
    lead: Vec<u8>,
    since_sat: u8,
    since_ev: u8,
    since_demand: u8,
    since_rung: u8,
    viol: u32,
}

fn new_state(cfg: &Cfg) -> S {
    let n = cfg.hot + 1;
    let mut tok = vec![T::Idle; n];
    let mut bit = vec![false; n];
    for h in 0..cfg.hot {
        tok[h] = T::Rung;
        bit[h] = true;
    }
    if cfg.cold_flagged {
        tok[cfg.hot] = T::Rung;
        bit[cfg.hot] = true;
    }
    S {
        tok,
        bit,
        wait: cfg.wait.map(|w| match w {
            WaitKind::Marker => Wait::Marker { target: 1 },
            WaitKind::Popped => Wait::Popped { target: 1 },
        }),
        sat: false,
        vis: false,
        lagging: false,
        demand: false,
        held: cfg.lock_at_start,
        holds_left: cfg.holds,
        epoch: 0,
        seen: 0,
        reg: false,
        hpc: 0,
        rings_left: cfg.cold_rings,
        ev: false,
        efd: false,
        moved: false,
        wk: (0..cfg.workers)
            .map(|_| Wk {
                pc: Pc::Scan,
                queue: Vec::new(),
                cur: 0,
                round: 0,
                n: 0,
                cs: 0,
            })
            .collect(),
        lead: vec![0; cfg.workers],
        since_sat: 0,
        since_ev: 0,
        since_demand: 0,
        since_rung: 0,
        viol: 0,
    }
}

impl S {
    fn ring(&mut self, r: usize) {
        match self.tok[r] {
            T::Idle => {
                self.tok[r] = T::Rung;
                self.bit[r] = true;
                self.moved = true;
                if self.wk.iter().any(|w| w.pc == Pc::Parked) {
                    self.efd = true;
                }
            }
            T::Busy => self.tok[r] = T::BusyRung,
            T::Rung | T::BusyRung => {}
        }
    }
    /// What the cold relay holds that the worker must not sleep through (the production [`Obligation`]).
    fn obligations(&self) -> Vec<Obligation> {
        let mut o = Vec::new();
        match self.wait {
            Some(Wait::Marker { .. }) => o.push(if self.lagging {
                Obligation::Lagging
            } else {
                Obligation::Marker
            }),
            Some(Wait::Popped { .. }) => o.push(Obligation::Popped),
            _ => {}
        }
        if self.demand && self.tok[self.tok.len() - 1] == T::Idle {
            // registered with the holder: the holder rings; not registered: nothing will. (Once the relay's token is flagged again the
            // token path carries it.)
            o.push(if self.reg {
                Obligation::Contended
            } else {
                Obligation::Lagging
            });
        }
        o
    }
    fn unannounced(&self) -> bool {
        !ul::may_block(self.obligations())
    }
    /// `ChanPlane::user_ring_peek_one` for the cold relay (the hot ones keep no wait): the production `look_needed` and `Wait::satisfied`
    /// decide. Contention is the lock holder's business: no look rings for it.
    fn peek(&mut self, cfg: &Cfg, cold: usize, why: Peek) -> bool {
        if !ul::look_needed(why, self.demand, self.wait.is_some()) {
            return false;
        }
        if let Some(w) = self.wait {
            let (marks, popped) = if self.vis { (1, 1) } else { (0, 0) };
            if w.satisfied(marks, popped) {
                self.wait = None;
                self.lagging = false;
                self.ring(cold);
                self.since_sat = 0;
                if why == Peek::Backstop {
                    self.viol |= V_NET;
                }
                return true;
            }
            if why == Peek::Event && !cfg.m.forgets_lagging {
                // the interrupt (a hint) came and the words do not show the progress yet: the state is not final, keep evaluating
                self.lagging = true;
            }
        }
        false
    }
    fn dispatch_event(&mut self, cfg: &Cfg, cold: usize) {
        if cfg.m.eval_skips_events {
            return;
        }
        self.ev = false;
        self.since_ev = 0;
        self.peek(cfg, cold, Peek::Event);
    }
}

/// `TokenWord::release` after a serve (`worker_pass`).
fn release(n: &mut S, i: usize, r: usize) {
    match n.tok[r] {
        T::Busy => {
            n.tok[r] = T::Idle;
            n.wk[i].pc = Pc::Claim;
        }
        T::BusyRung if n.wk[i].round < REACT_ROUNDS => {
            n.tok[r] = T::Busy;
            n.wk[i].round += 1;
        }
        T::BusyRung => {
            n.tok[r] = T::Rung;
            n.bit[r] = true;
            n.moved = true;
            n.wk[i].pc = Pc::Claim;
        }
        t => panic!("release in {t:?}"),
    }
}

fn successors(cfg: &Cfg, s: &S) -> Vec<(String, S)> {
    let cold = cfg.hot;
    let mut out: Vec<(String, S)> = Vec::new();
    // ---- environment
    if s.wait.is_some() && !s.sat {
        let mut n = s.clone();
        n.sat = true;
        n.vis = !cfg.lag;
        if matches!(cfg.wait, Some(WaitKind::Marker)) && cfg.irq {
            n.ev = true;
        }
        out.push((
            "env: the cold relay's wait is satisfied on the hardware".into(),
            n,
        ));
    }
    if cfg.lag && s.sat && !s.vis {
        let mut n = s.clone();
        n.vis = true;
        out.push(("env: the private words become visible to the CPU".into(), n));
    }
    if s.rings_left > 0 {
        let mut n = s.clone();
        n.rings_left -= 1;
        n.ring(cold);
        out.push(("guest: the cold client rings its doorbell".into(), n));
    }
    // ---- the lock holder (another thread: `pt` / USERD admission): take, release, then -- the hand-off -- bump the epoch, look at the
    // registry, ring the registered relay. Each is a step, so the loser's steps interleave anywhere (the lost-registration race included).
    if s.holds_left > 0 && !s.held && s.hpc == 0 {
        let mut n = s.clone();
        n.holds_left -= 1;
        n.held = true;
        out.push(("holder: takes the lock".into(), n));
    }
    if s.held {
        let mut n = s.clone();
        n.held = false;
        n.hpc = u8::from(cfg.handoff != Handoff::None);
        out.push(("holder: releases the lock".into(), n));
    }
    match s.hpc {
        1 => {
            let mut n = s.clone();
            n.epoch = n.epoch.wrapping_add(1);
            n.hpc = 2;
            out.push(("holder: bumps the epoch".into(), n));
        }
        2 => {
            let mut n = s.clone();
            n.hpc = if n.reg { 3 } else { 0 };
            out.push((
                if s.reg {
                    "holder: looks at the registry: a waiter".into()
                } else {
                    "holder: looks at the registry: EMPTY".into()
                },
                n,
            ));
        }
        3 => {
            let mut n = s.clone();
            n.reg = false;
            n.hpc = 0;
            n.ring(cold);
            n.since_demand = 0;
            out.push(("holder: rings the registered relay".into(), n));
        }
        _ => {}
    }
    // ---- workers
    for (i, w) in s.wk.iter().enumerate() {
        if s.lead[i] >= cfg.delta && w.pc != Pc::Parked {
            continue;
        }
        let first = out.len();
        let me = |t: &str| format!("w{i}: {t}");
        match w.pc {
            Pc::Scan => {
                let mut n = s.clone();
                n.moved = false;
                // passes since something became attendable
                if cfg.track & V_P11 != 0 && n.wait.is_some() && n.vis {
                    n.since_sat = n.since_sat.saturating_add(1);
                }
                if cfg.track & V_P12 != 0 && n.ev {
                    n.since_ev = n.since_ev.saturating_add(1);
                }
                if cfg.track & V_P14 != 0
                    && n.demand
                    && !n.held
                    && n.hpc == 0
                    && n.tok[cold] == T::Idle
                {
                    n.since_demand = n.since_demand.saturating_add(1);
                }
                if cfg.track & V_TOKEN != 0 && n.bit[cold] {
                    n.since_rung = n.since_rung.saturating_add(1);
                }
                let mut q = Vec::new();
                for r in 0..n.tok.len() {
                    if n.bit[r] {
                        n.bit[r] = false;
                        q.push(r as u8);
                    }
                }
                n.wk[i].queue = q;
                n.wk[i].n = 0;
                n.wk[i].pc = Pc::Claim;
                out.push((me("pass: scan the flagged tokens"), n));
            }
            Pc::Claim => {
                let mut n = s.clone();
                if n.wk[i].queue.is_empty() {
                    let served = u8::try_from(n.wk[i].n.min(255)).unwrap_or(255);
                    n.wk[i].pc = Pc::After { served };
                    out.push((me("pass ends"), n));
                } else {
                    let r = n.wk[i].queue.remove(0) as usize;
                    if n.tok[r] == T::Rung {
                        n.tok[r] = T::Busy;
                        n.wk[i].cur = r as u8;
                        n.wk[i].round = 0;
                        n.wk[i].cs = 0;
                        n.wk[i].pc = Pc::Serve;
                        if r == cold {
                            n.since_rung = 0;
                        }
                        out.push((me(&format!("claim token {r}")), n));
                    } else {
                        out.push((me(&format!("token {r} no longer rung: skip")), n));
                    }
                }
            }
            Pc::Serve => {
                let r = usize::from(w.cur);
                if r != cold {
                    // a hot relay: every turn makes progress (`Need::Again`) -> it rings itself, so `worker_pass` serves it
                    // `REACT_ROUNDS + 1` times (nothing else touches a BUSY token meanwhile: the rounds are one step) and then
                    // republishes it RUNG (`RepublishAndMoveOn`)
                    let mut h = s.clone();
                    h.wk[i].n += 1 + REACT_ROUNDS;
                    h.tok[r] = T::Rung;
                    h.bit[r] = true;
                    h.moved = true;
                    h.wk[i].pc = Pc::Claim;
                    out.push((me(&format!("serve hot relay {r} {} times (it rings itself again every time), republish it", REACT_ROUNDS + 1)), h));
                } else {
                    // the cold relay, micro-step by micro-step so that the lock holder acts between any two
                    let mut n = s.clone();
                    match w.cs {
                        0 => {
                            // `let e0 = self.handoff.prepare()` BEFORE the first attempt
                            n.wk[i].n += 1;
                            n.seen = n.epoch;
                            n.wk[i].cs = 1;
                            out.push((me("serve the cold relay: read the hand-off epoch"), n));
                        }
                        1 => {
                            if n.held {
                                // `pt.try_lock()` failed: the relay now holds a contended turn
                                n.demand = true;
                                n.wk[i].cs = 2;
                                out.push((
                                    me("serve the cold relay: try_lock LOST against the neighbour"),
                                    n,
                                ));
                            } else {
                                n.demand = false;
                                n.since_demand = 0;
                                n.wk[i].cs = 0;
                                release(&mut n, i, r);
                                out.push((me("serve the cold relay"), n));
                            }
                        }
                        2 => {
                            // register with the holder BEFORE giving up (HandoffHub::lost)
                            if cfg.handoff != Handoff::None {
                                n.reg = true;
                            }
                            if cfg.handoff == Handoff::Epoch {
                                n.wk[i].cs = 3;
                            } else {
                                n.wk[i].cs = 0;
                                release(&mut n, i, r);
                            }
                            out.push((me("register with the lock holder"), n));
                        }
                        _ => {
                            // the epoch re-check: a holder released since the attempt began -> serve again at once (the ring is the BUSY
                            // token's BusyRung, the release below turns it into `ActAgain`)
                            let again = n.epoch != n.seen;
                            if again {
                                n.ring(r);
                            }
                            n.wk[i].cs = 0;
                            release(&mut n, i, r);
                            let label = if again {
                                "re-check: a holder released since the attempt began, serve again"
                            } else {
                                "re-check: no release yet, wait for the holder's ring"
                            };
                            out.push((me(label), n));
                        }
                    }
                }
            }
            Pc::After { served } => {
                // THE ONE EVALUATION per iteration: drain the ready fds, evaluate every relay's wait; then decide
                let mut n = s.clone();
                if !(cfg.m.eval_only_when_idle && served > 0) {
                    if n.ev {
                        n.dispatch_event(cfg, cold);
                    }
                    if n.wait.is_some() && !cfg.m.eval_skips_peek {
                        n.peek(cfg, cold, Peek::Spin);
                    }
                }
                let unannounced = n.unannounced() && !cfg.m.park_ignores_unannounced;
                let (pc, what) = if served > 0 {
                    (Pc::Scan, "busy: next pass")
                } else if ul::idle(unannounced) == ul::Idle::Spin {
                    (
                        Pc::Scan,
                        "idle with an obligation no wake source announces: keep evaluating (epoll timeout 0)",
                    )
                } else {
                    (Pc::Park, "idle, every obligation announced: to try_park")
                };
                n.wk[i].pc = pc;
                out.push((
                    me(&format!(
                        "after a pass that served {served}: evaluate, then {what}"
                    )),
                    n,
                ));
            }
            Pc::Park => {
                let mut n = s.clone();
                if n.moved {
                    n.wk[i].pc = Pc::Scan;
                    out.push((me("try_park refused (seq moved)"), n));
                } else {
                    n.wk[i].pc = Pc::Parked;
                    out.push((me("blocks in epoll_wait (no timeout)"), n));
                }
            }
            Pc::Parked => {
                if s.efd {
                    let mut n = s.clone();
                    n.efd = false;
                    n.wk[i].pc = Pc::Scan;
                    out.push((me("woken by the doorbell eventfd"), n));
                }
                if s.ev {
                    let mut n = s.clone();
                    n.dispatch_event(cfg, cold);
                    n.wk[i].pc = Pc::Scan;
                    out.push((me("woken by an engine event"), n));
                }
                if cfg.timer && !s.efd && !s.ev {
                    // the park timeout (safety net): a firing that finds work is a bug (V_NET)
                    let mut n = s.clone();
                    n.peek(cfg, cold, Peek::Backstop);
                    n.wk[i].pc = Pc::Scan;
                    out.push((me("park timeout (50 ms): safety-net look"), n));
                }
            }
        }
        for (_, n) in &mut out[first..] {
            n.lead[i] += 1;
            // a blocked worker takes no steps and is not "behind": the scheduler-fairness gap is measured among the awake ones
            let awake: Vec<usize> = (0..n.wk.len())
                .filter(|&k| n.wk[k].pc != Pc::Parked)
                .collect();
            let min = awake.iter().map(|&k| n.lead[k]).min().unwrap_or(0);
            for k in 0..n.lead.len() {
                n.lead[k] = if awake.contains(&k) {
                    n.lead[k] - min
                } else {
                    0
                };
            }
        }
    }
    out
}

fn invariant(cfg: &Cfg, s: &S) -> u32 {
    let mut v = s.viol;
    if s.since_sat > K {
        v |= V_P11;
    }
    if s.since_ev > K {
        v |= V_P12;
    }
    if s.since_demand > K {
        v |= V_P14;
    }
    if s.since_rung > 2 {
        v |= V_TOKEN;
    }
    // ★ the sleep invariant, per worker: blocked in epoll_wait without a timeout => every obligation has a registered wake source
    // (a worker decides to block only when `may_block` holds -- the After step -- and an obligation that appears while it sleeps is
    // owned by the AWAKE worker whose turn created it, which keeps evaluating; so the state to forbid is: EVERY worker blocked)
    if s.wk.iter().all(|w| w.pc == Pc::Parked) && !ul::may_block(s.obligations()) {
        v |= V_SLEEP;
    }
    let _ = cfg;
    v
}

struct Counterexample {
    why: Vec<&'static str>,
    trace: Vec<String>,
    states: usize,
}

fn explore(cfg: &Cfg) -> Result<usize, Counterexample> {
    let start = new_state(cfg);
    // each state is stored ONCE (the key of `seen`); the parent chain is an index into `parents` (memory: tens of millions of states)
    let mut seen: HashMap<S, u32> = HashMap::new();
    let mut parents: Vec<(u32, u32)> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let mut label_ids: HashMap<String, u32> = HashMap::new();
    let mut q: VecDeque<(S, u32)> = VecDeque::new();
    seen.insert(start.clone(), 0);
    parents.push((u32::MAX, 0));
    q.push_back((start, 0));
    let mut states = 0usize;
    while let Some((s, idx)) = q.pop_front() {
        states += 1;
        assert!(states < 24_000_000, "state space exceeded");
        let v = invariant(cfg, &s) & (cfg.track | V_SLEEP | V_NET);
        if v != 0 {
            let mut trace = Vec::new();
            let mut cur = idx;
            while parents[cur as usize].0 != u32::MAX {
                trace.push(labels[parents[cur as usize].1 as usize].clone());
                cur = parents[cur as usize].0;
            }
            trace.reverse();
            return Err(Counterexample {
                why: names(v),
                trace,
                states,
            });
        }
        let succ = successors(cfg, &s);
        if succ.is_empty() {
            // nothing can ever run again: a satisfied, visible wait left unrung, or a lost lock left unannounced, is a strand
            let mut v = 0;
            if s.wait.is_some() && s.vis {
                v |= V_P11;
            }
            if s.demand && !s.held {
                v |= V_P14;
            }
            if v & cfg.track != 0 {
                let mut trace = Vec::new();
                let mut cur = idx;
                while parents[cur as usize].0 != u32::MAX {
                    trace.push(labels[parents[cur as usize].1 as usize].clone());
                    cur = parents[cur as usize].0;
                }
                trace.reverse();
                trace.push("(nothing can run any more)".into());
                return Err(Counterexample {
                    why: names(v & cfg.track),
                    trace,
                    states,
                });
            }
        }
        for (label, n) in succ {
            if !seen.contains_key(&n) {
                let lid = *label_ids.entry(label.clone()).or_insert_with(|| {
                    labels.push(label);
                    (labels.len() - 1) as u32
                });
                let id = parents.len() as u32;
                parents.push((idx, lid));
                seen.insert(n.clone(), id);
                q.push_back((n, id));
            }
        }
    }
    Ok(states)
}

fn holds(name: &str, cfg: &Cfg) -> usize {
    match explore(cfg) {
        Ok(n) => {
            eprintln!("{name}: {n} states, no violation");
            n
        }
        Err(e) => panic!(
            "{name}: {} after {} states\n  minimal schedule ({} steps):\n    {}",
            e.why.join("; "),
            e.states,
            e.trace.len(),
            e.trace.join("\n    ")
        ),
    }
}

fn finds(name: &str, cfg: &Cfg, needle: &str) -> Counterexample {
    // `needle` may name several acceptable properties ("P14|SLEEP")
    match explore(cfg) {
        Err(e)
            if e.why
                .iter()
                .any(|w| needle.split('|').any(|n| w.starts_with(n))) =>
        {
            e
        }
        Err(e) => panic!("{name}: found {:?}, expected {needle}", e.why),
        Ok(n) => panic!("{name}: expected a violation {needle}, none in {n} states"),
    }
}

fn base(workers: usize, hot: usize, wait: Option<WaitKind>) -> Cfg {
    Cfg {
        workers,
        hot,
        wait,
        lag: false,
        holds: 0,
        lock_at_start: false,
        handoff: Handoff::Epoch,
        cold_rings: if wait.is_some() { 1 } else { 0 },
        cold_flagged: false,
        irq: true,
        m: Mutant::default(),
        track: V_P11 | V_P12 | V_P14 | V_TOKEN,
        timer: false,
        delta: 14,
    }
}

/// A convoy: the cold relay is flagged and the lock is held when it is first served; the adversary may take the lock again.
fn convoy(workers: usize, hot: usize, handoff: Handoff, holds: u8) -> Cfg {
    let mut c = base(workers, hot, None);
    c.cold_flagged = true;
    c.lock_at_start = true;
    c.holds = holds;
    c.handoff = handoff;
    c.track = V_P14;
    c
}

// ------------------------------------------------------------------------------------------------
// P11 / P12 / P13: bounded attendance, 1, 2 and 3 workers

#[test]
fn p11_p12_one_worker_one_hot_relay_the_cold_wait_and_the_engine_event_are_attended_within_k_passes()
 {
    holds("W=1 hot=1 marker", &base(1, 1, Some(WaitKind::Marker)));
}

#[test]
fn p13_one_worker_one_hot_relay_a_carried_wait_is_found_within_k_passes_without_the_safety_net() {
    let mut c = base(1, 1, Some(WaitKind::Popped));
    c.track = V_P11;
    holds("W=1 hot=1 popped", &c);
}

#[test]
fn p11_two_workers_one_hot_relay_the_baton_does_not_starve_the_cold_wait() {
    let mut c = base(2, 1, Some(WaitKind::Marker));
    c.track = V_P11;
    holds("W=2 hot=1 baton", &c);
}

#[test]
fn p12_two_workers_one_hot_relay_the_baton_does_not_delay_the_engine_event() {
    let mut c = base(2, 1, Some(WaitKind::Marker));
    c.track = V_P12;
    holds("W=2 hot=1 baton event", &c);
}

#[test]
fn p11_three_workers_one_hot_relay_a_carried_wait_is_attended_within_k_passes() {
    let mut c = base(3, 1, Some(WaitKind::Popped));
    c.track = V_P11;
    c.delta = 8;
    holds("W=3 hot=1 popped", &c);
}

#[test]
fn control_a_flagged_cold_token_is_claimed_within_two_passes() {
    let mut c = base(1, 2, None);
    c.cold_rings = 2;
    c.track = V_TOKEN;
    holds("token path", &c);
}

// ------------------------------------------------------------------------------------------------
// P14: a relay that lost a try_lock is re-rung by the lock HOLDER, with no budget, 1, 2 and 3 workers

#[test]
fn p14_one_worker_a_lost_try_lock_is_re_rung_by_the_holder_beside_a_hot_relay() {
    holds("W=1 hot=1 convoy", &convoy(1, 1, Handoff::Epoch, 2));
}

#[test]
fn p14_two_workers_a_lost_try_lock_is_re_rung_by_the_holder_beside_a_hot_relay() {
    holds("W=2 hot=1 convoy", &convoy(2, 1, Handoff::Epoch, 1));
}

#[test]
fn p14_three_workers_a_lost_try_lock_is_re_rung_by_the_holder() {
    let mut c = convoy(3, 0, Handoff::Epoch, 1);
    c.delta = 6;
    holds("W=3 hot=0 convoy", &c);
}

/// The retired behaviour (the holder tells nobody; looks and the net were the only rescue): stranded.
#[test]
fn p14_without_a_hand_off_a_lost_try_lock_is_stranded_one_worker() {
    finds(
        "W=1 no hand-off",
        &convoy(1, 1, Handoff::None, 1),
        "P14|SLEEP",
    );
}

#[test]
fn p14_without_a_hand_off_a_lost_try_lock_is_stranded_two_workers() {
    finds(
        "W=2 no hand-off",
        &convoy(2, 1, Handoff::None, 1),
        "P14|SLEEP",
    );
}

/// ★ The lost-registration interleaving as an explicit case: the loser tried, the holder released and looked at an EMPTY registry, the loser
/// registered afterwards. A NAIVE hand-off (register after the failed attempt, no epoch re-check) is stranded by it; the real protocol is not.
#[test]
fn p14_a_naive_registration_loses_the_race_one_worker() {
    let e = finds("W=1 naive", &convoy(1, 0, Handoff::Naive, 1), "P14|SLEEP");
    let t = e.trace.join(" | ");
    assert!(
        t.find("looks at the registry: EMPTY")
            .zip(t.find("register with the lock holder"))
            .is_some_and(|(a, b)| a < b),
        "the minimal schedule is the lost registration: {t}"
    );
}

#[test]
fn p14_a_naive_registration_loses_the_race_two_workers() {
    finds("W=2 naive", &convoy(2, 1, Handoff::Naive, 1), "P14|SLEEP");
}

// ------------------------------------------------------------------------------------------------
// P15: the sleep invariant -- blocking needs a wake source for every obligation; an idle system blocks with zero wakeups

#[test]
fn sleep_a_spent_hint_keeps_the_worker_evaluating_until_the_words_are_visible() {
    for w in 1..=2 {
        let mut c = base(w, 1, Some(WaitKind::Marker));
        c.lag = true;
        c.track = V_P11;
        holds(&format!("lag W={w}"), &c);
    }
}

#[test]
fn sleep_a_carried_wait_never_lets_a_worker_block() {
    for w in 1..=2 {
        let mut c = base(w, 0, Some(WaitKind::Popped));
        c.track = V_P11;
        holds(&format!("popped W={w} (no hot relay)"), &c);
    }
}

/// The inverse: with no obligation at all every worker blocks and nothing is left to run -- no spinning, no wakeups.
#[test]
fn an_idle_worker_blocks_with_zero_wakeups() {
    for w in 1..=3 {
        let c = base(w, 0, None);
        let n = holds(&format!("idle W={w}"), &c);
        assert!(
            n < 60 * w,
            "{n} states for {w} workers: an idle system must reach its blocked state and stop"
        );
        // from the all-blocked state no worker step is enabled
        let mut s = new_state(&c);
        let mut guard = 0;
        loop {
            let next = successors(&c, &s).into_iter().next();
            match next {
                Some((_, n)) => s = n,
                None => break,
            }
            guard += 1;
            assert!(guard < 200, "the idle system never settled");
        }
        assert!(
            s.wk.iter().all(|k| k.pc == Pc::Parked),
            "every worker is blocked: {:?}",
            s.wk.iter().map(|k| k.pc).collect::<Vec<_>>()
        );
    }
}

/// With the park timeout present in the model the net never fires on work: nothing depends on it.
#[test]
fn the_safety_net_is_never_needed_with_one_and_two_workers() {
    for w in 1..=2 {
        let mut c = base(w, 0, Some(WaitKind::Marker));
        c.timer = true;
        c.track = V_P11;
        holds(&format!("net W={w}"), &c);
        let mut c = convoy(w, 0, Handoff::Epoch, 1);
        c.timer = true;
        holds(&format!("net convoy W={w}"), &c);
    }
}

// ------------------------------------------------------------------------------------------------
// the checker has teeth

fn mutant(workers: usize, hot: usize, wait: Option<WaitKind>, track: u32, m: Mutant) -> Cfg {
    let mut c = base(workers, hot, wait);
    c.track = track;
    c.m = m;
    c
}

macro_rules! mutants {
    ($( $name:ident : $cfg:expr, $needle:expr; )*) => {
        $( #[test] fn $name() { let c: Cfg = $cfg; holds(concat!(stringify!($name), "/control"), &Cfg { m: Mutant::default(), ..c.clone() }); finds(stringify!($name), &c, $needle); } )*
    };
}

mutants! {
    mutant_the_retired_loop_evaluates_only_when_idle_marker_wait:
        mutant(1, 1, Some(WaitKind::Marker), V_P11, Mutant { eval_only_when_idle: true, ..Mutant::default() }), "P11";
    mutant_the_retired_loop_evaluates_only_when_idle_carried_wait:
        mutant(1, 1, Some(WaitKind::Popped), V_P11, Mutant { eval_only_when_idle: true, ..Mutant::default() }), "P11";
    mutant_the_retired_loop_evaluates_only_when_idle_engine_event:
        mutant(1, 1, Some(WaitKind::Marker), V_P12, Mutant { eval_only_when_idle: true, ..Mutant::default() }), "P12";
    mutant_evaluation_without_event_dispatch_engine_event:
        mutant(1, 1, Some(WaitKind::Marker), V_P12, Mutant { eval_skips_events: true, ..Mutant::default() }), "P12";
    mutant_evaluation_without_event_dispatch_marker_wait:
        mutant(1, 0, Some(WaitKind::Marker), V_P12, Mutant { eval_skips_events: true, ..Mutant::default() }), "P12";
    mutant_evaluation_without_peek_carried_wait:
        mutant(1, 1, Some(WaitKind::Popped), V_P11, Mutant { eval_skips_peek: true, ..Mutant::default() }), "P11";
    mutant_evaluation_without_peek_marker_wait_with_a_spent_hint:
        Cfg { lag: true, ..mutant(1, 0, Some(WaitKind::Marker), V_P11, Mutant { eval_skips_peek: true, ..Mutant::default() }) }, "P11|SLEEP";
    mutant_a_worker_that_blocks_with_a_carried_wait:
        mutant(1, 0, Some(WaitKind::Popped), V_P11, Mutant { park_ignores_unannounced: true, ..Mutant::default() }), "SLEEP";
    mutant_a_worker_that_blocks_with_a_carried_wait_two_workers:
        mutant(2, 0, Some(WaitKind::Popped), V_P11, Mutant { park_ignores_unannounced: true, ..Mutant::default() }), "SLEEP";
    mutant_a_spent_hint_that_is_forgotten:
        Cfg { lag: true, ..mutant(1, 0, Some(WaitKind::Marker), V_P11, Mutant { forgets_lagging: true, ..Mutant::default() }) }, "SLEEP|P11";
    mutant_a_spent_hint_that_is_forgotten_two_workers:
        Cfg { lag: true, ..mutant(2, 0, Some(WaitKind::Marker), V_P11, Mutant { forgets_lagging: true, ..Mutant::default() }) }, "SLEEP|P11";
}

// ------------------------------------------------------------------------------------------------
// the real bitmap: the FINDING of flagged tokens is fair (rotating start)

#[test]
fn the_real_rung_bitmap_reaches_a_cold_token_behind_a_crowd_of_hot_ones() {
    use kf_trap::RungBitmap;
    let bm = RungBitmap::new();
    let hot: Vec<u32> = (0..200).collect();
    let cold = 100_000u32;
    for t in &hot {
        bm.publish(*t);
    }
    bm.publish(cold);
    let mut out = Vec::new();
    let mut served_cold_at = None;
    for pass in 0..8 {
        bm.scan(&mut out, 64);
        if out.contains(&cold) {
            served_cold_at = Some(pass);
            break;
        }
        for t in &hot {
            bm.publish(*t);
        }
    }
    assert!(
        served_cold_at.is_some_and(|p| p < 8),
        "the cold token was never scanned in 8 passes"
    );
}

// ------------------------------------------------------------------------------------------------
// the production loop really is the model's loop (source pins)

const WORKER: &str = include_str!("../src/worker.rs");
const USER_LOOP: &str = include_str!("../src/user_loop.rs");
const USERPUMP: &str = include_str!("../../kf-qemu/src/userpump.rs");

fn loop_body() -> &'static str {
    WORKER
        .split("while !stop.load(Ordering::Acquire) {")
        .nth(1)
        .expect("pin: the worker loop")
        .split("\n#[cfg(test)]")
        .next()
        .expect("pin")
}

#[test]
fn pin_the_loop_evaluates_after_every_pass_and_before_it_decides_to_block() {
    let l = loop_body();
    let order = [
        "plane.worker_pass(",
        "PollTimeout::Immediate",
        "dispatch(&ready)",
        "relay.peek(Peek::Event)",
        "relay.peek(Peek::Spin)",
        "if n > 0 {",
        "relay.unannounced()",
        "try_park(seen)",
        "park_timeout(owed, infinite)",
    ];
    let mut at = 0;
    for o in order {
        let i = l[at..]
            .find(o)
            .unwrap_or_else(|| panic!("pin: {o:?} missing or out of order in the worker loop"));
        at += i + o.len();
    }
}

#[test]
fn pin_nothing_is_budgeted_or_counted_in_passes() {
    for forbidden in [
        "LOOK_EVERY",
        "CONTENTION_RETRIES",
        "may_retry",
        "LoopPolicy",
        "AfterPass",
        "user_loop::SPIN",
    ] {
        assert!(
            !WORKER.contains(forbidden)
                && !USER_LOOP.contains(&format!("pub const {forbidden}"))
                && !USER_LOOP.contains(&format!("fn {forbidden}")),
            "{forbidden} is back"
        );
    }
}

#[test]
fn pin_a_contended_relay_registers_before_giving_up_and_no_look_rings_for_contention() {
    let serve = USERPUMP
        .split("pub(super) fn user_ring_serve(&self, ht: u32)")
        .nth(1)
        .expect("pin")
        .split("fn user_ring_arm")
        .next()
        .unwrap();
    let e0 = serve
        .find("self.handoff.prepare()")
        .expect("pin: the epoch is read before the first attempt");
    let enter = serve.find("record.enter()").expect("pin");
    let lost = serve
        .find("self.handoff.lost(record.idx, e0)")
        .expect("pin: the loser registers");
    assert!(e0 < enter && enter < lost);
    let peek = USERPUMP
        .split("fn user_ring_peek_one(")
        .nth(1)
        .expect("pin")
        .split("pub(crate) fn user_ring_poll_due")
        .next()
        .unwrap();
    assert!(
        !peek.contains("record.pending()") && !peek.contains("is_contended"),
        "a look must not act on demand / contention"
    );
    assert!(peek.contains("look_needed(why, false, record.is_waiting())"));
}

#[test]
fn pin_every_pt_release_hands_off() {
    let pt = include_str!("../../kf-qemu/src/ptlock.rs");
    let drop_at = pt.find("impl<T> Drop for PtGuard").expect("pin");
    let tail = &pt[drop_at..];
    let release = tail
        .find("drop(self.g.take());")
        .expect("pin: the lock is released first");
    let tell = tail
        .find("self.hub.released();")
        .expect("pin: then the hub is told");
    assert!(release < tell);
}

// ---- the knob and the unannounced check, as the loop uses them (second killers beside the unit tests / pins in worker.rs)

#[test]
fn the_park_timeout_knob_makes_the_wait_infinite_and_nothing_else_does() {
    use kf_chan::user_loop::BACKSTOP_MS as PARK_MS;
    use kf_chan::worker::park_timeout;
    use kf_linux_raw::PollTimeout;
    assert!(matches!(park_timeout(false, true), PollTimeout::Blocking));
    assert!(matches!(
        park_timeout(false, false),
        PollTimeout::Millis(PARK_MS)
    ));
    // the paced relay's trailing raise is a timer's business, whatever the knob says
    assert!(!matches!(park_timeout(true, true), PollTimeout::Blocking));
}

#[test]
fn pin_the_worker_spins_on_an_unannounced_obligation_before_it_may_park() {
    let l = loop_body();
    let flat: String = l.split_whitespace().collect();
    let order = [
        "user_loop::idle(relay.unannounced())==user_loop::Idle::Spin",
        "unannounced_spins",
        "continue;",
        "try_park(seen)",
    ];
    let mut at = 0;
    for o in order {
        let i = flat[at..]
            .find(o)
            .unwrap_or_else(|| panic!("pin: {o:?} missing or out of order before the park"));
        at += i + o.len();
    }
}
