//! 2026-10-20 -- **Bounded fairness of the worker loop over the USER-ring relays** (P11, P12, P13).
//!
//! Hardware showed STARVATION on the relay route (Linux with `KF3_DEBUG_FORCE_MIGRATABLE=1`, Windows): one client starved for
//! 6-59 s while the other ran thousands of iterations, nothing stuck on the ring, the safety-net counters 0. A finite-schedule
//! liveness check cannot see that ("eventually served" holds after 59 s), so this model checks a BOUND: whatever the schedule, a
//! relay's satisfied wait, an engine event or a flagged token is attended to within `K` worker passes.
//!
//! CODE READ (`kf_chan::worker::run_with_relay`, `kf_core::Plane::worker_pass`, `ChanPlane::user_ring_serve/arm/peek_one`):
//!  * a pass serves every flagged token (`RungBitmap::scan`, limit 64, rotating start; claim `RUNG -> BUSY`; `ActAgain` up to
//!    `REACT_ROUNDS` rounds; then `RepublishAndMoveOn`). A relay that ended its turn with `Need::Again` rings its own token, so a hot
//!    relay is flagged again at once;
//!  * `worker_pass` returns the number of serves (a serve that did nothing, a contended one included, counts);
//!  * the loop looks at the world (`poller.wait`: engine NON_STALL events, completions, the doorbell eventfd; `relay.peek`: the
//!    waiting relays' private words, demand left by a busy lock) ONLY when a pass served NOTHING (`if n > 0 { continue }`).
//!    A wait is woken only by such a look (marker interrupt -> peek(Event); `Popped` / pt-busy / disabled Retry -> peek(Spin));
//!    the 50 ms safety net is a park timeout and never runs while the worker is busy.
//!
//! So: with as many hot relays as workers (or fewer, when the workers hand one hot token to each other: the baton) no look ever happens; a cold relay's wait, the engine's guest interrupts and a busy-lock
//! retry all wait for the hot neighbours to pause. The model drives the production `worker::LoopPolicy::after_pass` and the
//! production `user_loop::{idle, look_needed, may_retry, Wait}`; the token/bitmap layer is the production state machine
//! transliterated (`kf-trap` `TokenWord`, `RungBitmap::scan`: fairness of FINDING is separately tested below with the real bitmap).
#![allow(
    clippy::too_many_lines,
    clippy::needless_range_loop,
    dead_code,
    missing_docs
)]

use kf_chan::user_loop::{self as ul, Peek, Wait};
use kf_chan::worker::{AfterPass, LoopPolicy};
use kf_trap::REACT_ROUNDS;
use std::collections::{HashMap, HashSet, VecDeque};

/// The bound, in worker passes (both workers' passes counted): a few multiples of `LOOK_EVERY` (4), independent of the hot relays.
const K: u8 = 16;
/// Spin iterations in the model (`user_loop::SPIN` is 64; the rule is the same).
const SPIN: u8 = 1;
/// Scheduler fairness: no worker runs more than `DELTA` steps ahead of the slowest one (real threads both run; an adversary that
/// never schedules the idle worker would be an unfair OS, not a defect of the loop).
const DELTA: u8 = 14;

const V_P11: u32 = 1; // a satisfied wait of the cold relay was not rung within K passes
const V_P12: u32 = 2; // an engine event was not dispatched within K passes
const V_P13: u32 = 4; // left-behind demand (busy lock) was not re-rung within K passes
const V_TOKEN: u32 = 8; // the cold relay's flagged token was not claimed within 2 passes

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
            V_P13,
            "P13 demand left behind by a busy lock was not re-rung within K passes",
        ),
        (
            V_TOKEN,
            "TOKEN the cold relay's flagged token was not claimed within 2 passes",
        ),
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
    After,
    Look,
    Spin(u8),
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
    policy: LoopPolicy,
}

#[derive(Clone, Copy, Debug, Default)]
struct Mutant {
    /// The look does not dispatch engine events.
    look_skips_events: bool,
    /// The look does not peek the waiting relays.
    look_skips_peek: bool,
    /// The look is replaced by `Continue` (the retired behaviour): no look after a busy pass.
    policy_continue_only: bool,
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
    wait: WaitKind,
    /// How many of the cold relay's serves may fail on a busy lock (the lock-convoy adversary).
    fails: u8,
    /// Doorbells the cold guest rings (cold-but-active).
    cold_rings: u8,
    irq: bool,
    m: Mutant,
    /// Which of P11/P12/P13 are tracked (the counters multiply the state space).
    track: u32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct S {
    tok: Vec<T>,
    bit: Vec<bool>,
    wait: Option<Wait>,
    sat: bool,
    demand: bool,
    run: u8,
    fails_left: u8,
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
    S {
        tok,
        bit,
        // the cold relay has just published a batch and waits for its marker (or its pops)
        wait: Some(match cfg.wait {
            WaitKind::Marker => Wait::Marker { target: 1 },
            WaitKind::Popped => Wait::Popped { target: 1 },
        }),
        sat: false,
        demand: false,
        run: 0,
        fails_left: cfg.fails,
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
                policy: LoopPolicy::default(),
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
    fn waiting(&self) -> bool {
        self.wait.is_some() || self.demand
    }
    /// `ChanPlane::user_ring_peek_one` for the cold relay (the hot ones keep no wait and no demand): the production
    /// `look_needed`, `may_retry` and `Wait::satisfied` decide.
    fn peek(&mut self, cold: usize, why: Peek) -> bool {
        if !ul::look_needed(why, self.demand, self.wait.is_some()) {
            return false;
        }
        let retry_ok = ul::may_retry(u32::from(self.run), why);
        if let Some(w) = self.wait {
            let (marks, popped) = if self.sat { (1, 1) } else { (0, 0) };
            if w.satisfied(marks, popped) && (retry_ok || !matches!(w, Wait::Contended)) {
                self.wait = None;
                self.ring(cold);
                // review 2026-10-20: P13 is a bound PER demand episode; the counter was never reset (cumulative budget)
                self.since_demand = 0;
                return true;
            }
        }
        if self.demand && retry_ok {
            self.ring(cold);
            self.since_demand = 0;
            return true;
        }
        false
    }
    fn dispatch_event(&mut self, cfg: &Cfg, cold: usize) {
        if cfg.m.look_skips_events {
            return;
        }
        self.ev = false;
        self.since_ev = 0;
        self.peek(cold, Peek::Event);
    }
}

fn successors(cfg: &Cfg, s: &S) -> Vec<(String, S)> {
    let cold = cfg.hot;
    let mut out: Vec<(String, S)> = Vec::new();
    // ---- environment
    if s.wait.is_some() && !s.sat {
        let mut n = s.clone();
        n.sat = true;
        if matches!(cfg.wait, WaitKind::Marker) && cfg.irq {
            n.ev = true;
        }
        out.push((
            "env: the cold relay's wait is satisfied on the hardware".into(),
            n,
        ));
    }
    if s.rings_left > 0 {
        let mut n = s.clone();
        n.rings_left -= 1;
        n.ring(cold);
        out.push(("guest: the cold client rings its doorbell".into(), n));
    }
    // ---- workers
    for (i, w) in s.wk.iter().enumerate() {
        if s.lead[i] >= DELTA {
            continue;
        }
        let first = out.len();
        let me = |t: &str| format!("w{i}: {t}");
        match w.pc {
            Pc::Scan => {
                let mut n = s.clone();
                n.moved = false;
                // passes since something became attendable
                if cfg.track & V_P11 != 0 && n.wait.is_some() && n.sat {
                    n.since_sat = n.since_sat.saturating_add(1);
                }
                if cfg.track & V_P12 != 0 && n.ev {
                    n.since_ev = n.since_ev.saturating_add(1);
                }
                if cfg.track & V_P13 != 0 && n.demand {
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
                    n.wk[i].pc = Pc::After;
                    out.push((me("pass ends"), n));
                } else {
                    let r = n.wk[i].queue.remove(0) as usize;
                    if n.tok[r] == T::Rung {
                        n.tok[r] = T::Busy;
                        n.wk[i].cur = r as u8;
                        n.wk[i].round = 0;
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
                let mut variants: Vec<(String, S)> = Vec::new();
                let mut base = s.clone();
                base.wk[i].n += 1;
                if r == cold {
                    if base.fails_left > 0 {
                        let mut f = base.clone();
                        f.fails_left -= 1;
                        f.demand = true;
                        f.run = f.run.saturating_add(1);
                        variants.push((me("serve the cold relay: try_lock lost against the neighbour, demand stays"), f));
                    }
                    let mut ok = base;
                    ok.demand = false;
                    ok.run = 0;
                    variants.push((me("serve the cold relay"), ok));
                } else {
                    // a hot relay: every turn makes progress (`Need::Again`) -> it rings itself, so `worker_pass` serves it
                    // `REACT_ROUNDS + 1` times (nothing else touches a BUSY token meanwhile: the rounds are one step) and then
                    // republishes it RUNG (`RepublishAndMoveOn`)
                    let mut h = base;
                    h.wk[i].n += REACT_ROUNDS;
                    h.tok[r] = T::Rung;
                    h.bit[r] = true;
                    h.moved = true;
                    h.wk[i].pc = Pc::Claim;
                    out.push((me(&format!("serve hot relay {r} {} times (it rings itself again every time), republish it", REACT_ROUNDS + 1)), h));
                    continue;
                }
                for (label, mut n) in variants {
                    // release
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
                    out.push((label, n));
                }
            }
            Pc::After => {
                let mut n = s.clone();
                let served = n.wk[i].n as usize;
                let mut next = n.wk[i].policy.after_pass(served);
                if cfg.m.policy_continue_only && next == AfterPass::Look {
                    next = AfterPass::Continue;
                }
                n.wk[i].pc = match next {
                    AfterPass::Continue => Pc::Scan,
                    AfterPass::Look => Pc::Look,
                    AfterPass::Idle => Pc::Spin(0),
                };
                out.push((
                    me(&format!("after a pass that served {served}: {next:?}")),
                    n,
                ));
            }
            Pc::Look => {
                let mut n = s.clone();
                // poll(0): an engine event is dispatched, then the waiting relays are peeked
                if n.ev {
                    n.dispatch_event(cfg, cold);
                }
                if n.waiting() && !cfg.m.look_skips_peek {
                    n.peek(cold, Peek::Spin);
                }
                n.wk[i].pc = Pc::Scan;
                out.push((me("LOOK between busy passes: poll(0) + peek"), n));
            }
            Pc::Spin(k) => {
                let mut n = s.clone();
                if n.ev {
                    n.dispatch_event(cfg, cold);
                    n.wk[i].pc = Pc::Scan;
                    out.push((me("idle spin: event"), n));
                } else if ul::idle(n.waiting(), u32::from(SPIN - k.min(SPIN))) == ul::Idle::Spin {
                    let rang = n.peek(cold, Peek::Spin);
                    n.wk[i].pc = if rang { Pc::Scan } else { Pc::Spin(k + 1) };
                    out.push((me("idle spin peek"), n));
                } else {
                    n.wk[i].pc = Pc::Park;
                    out.push((me("idle: to try_park"), n));
                }
            }
            Pc::Park => {
                let mut n = s.clone();
                if n.moved {
                    n.wk[i].pc = Pc::Scan;
                    out.push((me("try_park refused"), n));
                } else {
                    n.wk[i].pc = Pc::Parked;
                    out.push((me("parked"), n));
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
            }
        }
        for (_, n) in &mut out[first..] {
            n.lead[i] += 1;
            let min = *n.lead.iter().min().unwrap_or(&0);
            for l in &mut n.lead {
                *l -= min;
            }
        }
    }
    out
}

fn invariant(s: &S) -> u32 {
    let mut v = 0;
    if s.since_sat > K {
        v |= V_P11;
    }
    if s.since_ev > K {
        v |= V_P12;
    }
    if s.since_demand > K {
        v |= V_P13;
    }
    if s.since_rung > 2 {
        v |= V_TOKEN;
    }
    v
}

struct Counterexample {
    why: Vec<&'static str>,
    trace: Vec<String>,
    states: usize,
}

fn explore(cfg: &Cfg) -> Result<usize, Counterexample> {
    let start = new_state(cfg);
    let mut seen: HashMap<S, (Option<S>, String)> = HashMap::new();
    let mut q = VecDeque::new();
    seen.insert(start.clone(), (None, String::new()));
    q.push_back(start);
    let mut states = 0usize;
    while let Some(s) = q.pop_front() {
        states += 1;
        assert!(states < 8_000_000, "state space exceeded");
        let v = invariant(&s);
        if v != 0 {
            let mut trace = Vec::new();
            let mut cur = s.clone();
            while let Some((Some(p), l)) = seen.get(&cur) {
                trace.push(l.clone());
                cur = p.clone();
            }
            trace.reverse();
            return Err(Counterexample {
                why: names(v),
                trace,
                states,
            });
        }
        for (label, n) in successors(cfg, &s) {
            if !seen.contains_key(&n) {
                seen.insert(n.clone(), (Some(s.clone()), label));
                q.push_back(n);
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
    match explore(cfg) {
        Err(e) if e.why.iter().any(|w| w.starts_with(needle)) => e,
        Err(e) => panic!("{name}: found {:?}, expected {needle}", e.why),
        Ok(n) => panic!("{name}: expected a violation {needle}, none in {n} states"),
    }
}

fn base(workers: usize, hot: usize, wait: WaitKind) -> Cfg {
    Cfg {
        workers,
        hot,
        wait,
        fails: 0,
        cold_rings: 1,
        irq: true,
        m: Mutant::default(),
        track: V_P11 | V_P12 | V_P13 | V_TOKEN,
    }
}

// ------------------------------------------------------------------------------------------------
// P11 / P12 / P13 -- the production loop must bound the delay (RED while `after_pass` never asks for a look)

/// P11 + P12: one worker, one hot relay, the cold relay waits for a marker whose interrupt has fired.
#[test]
fn p11_p12_one_worker_one_hot_relay_the_cold_wait_and_the_engine_event_are_attended_within_k_passes()
 {
    holds("W=1 hot=1 marker", &base(1, 1, WaitKind::Marker));
}

/// P13: the same with a CARRIED wait (`Wait::Popped`: nothing announces it, only a look finds it).
#[test]
fn p13_one_worker_one_hot_relay_a_carried_wait_is_found_within_k_passes_without_the_safety_net() {
    holds("W=1 hot=1 popped", &base(1, 1, WaitKind::Popped));
}

/// P13: a try_lock lost against the neighbour (pt / record lock) leaves demand behind; only a look re-rings it.
#[test]
fn p13_one_worker_a_lost_try_lock_is_re_rung_within_k_passes() {
    let mut c = base(1, 1, WaitKind::Popped);
    c.fails = 2;
    holds("W=1 hot=1 lock convoy", &c);
}

/// ⚠ EXPECTED RED (review 2026-10-20, claude/relay-fairness-review-20261020). The convoy above has `fails = 2`, far below
/// `user_loop::CONTENTION_RETRIES` (64), so `may_retry` is always true in the model and the budget never runs out. CODE READ:
/// once a relay has `CONTENTION_RETRIES` consecutive contended turns, only `Peek::Backstop` may re-ring it
/// (`user_loop::may_retry`, userpump.rs `user_ring_peek_one` lines 2882/2891/2927/2975); the busy Look peeks with `Spin` and
/// `Event` only (worker.rs 239/242); and `Backstop` happens only after a park TIMEOUT (worker.rs 317), which a worker beside a
/// hot relay never reaches (the hot relay's ring bumps the wake word, so `try_park(seen)` refuses, worker.rs 282). The
/// contended run is reached with no holder ring for a `pt` / storage `try_lock`, a USERD `AccessError::Busy` or a
/// `userread` `Busy`/`Changed` (v2 model, section 1 item 6: "no wake exists: spin + backstop only"). Here: 64 lost try_locks,
/// then the lock is free, and nothing rings the cold relay while the hot neighbour runs (W = 1: the worker is never idle, so
/// a missing park timeout is not a modelling artefact).
#[test]
fn p13_expected_red_a_convoy_past_contention_retries_is_still_re_rung_within_k_passes_beside_a_hot_relay() {
    let mut c = base(1, 1, WaitKind::Popped);
    c.track = V_P13;
    c.cold_rings = 0;
    c.fails = u8::try_from(ul::CONTENTION_RETRIES).expect("small");
    holds("W=1 hot=1 convoy past CONTENTION_RETRIES", &c);
}

/// Control for the red test above: one lost try_lock fewer than the budget, and the busy Look re-rings every episode within K.
#[test]
fn p13_control_a_convoy_just_below_contention_retries_is_re_rung_within_k_passes() {
    let mut c = base(1, 1, WaitKind::Popped);
    c.track = V_P13;
    c.cold_rings = 0;
    c.fails = u8::try_from(ul::CONTENTION_RETRIES - 1).expect("small");
    holds("W=1 hot=1 convoy below CONTENTION_RETRIES", &c);
}

/// The BATON: with two workers and ONE hot relay the workers hand the hot token to each other. The worker that finishes its
/// `REACT_ROUNDS` republishes the token RUNG and its next scan (or the other worker's) claims it again; a worker finds nothing
/// -- and so looks -- only when it scans in the window in which the token is BUSY (timing sensitive: about one run in three on
/// hardware).
#[test]
fn p11_two_workers_one_hot_relay_the_baton_does_not_starve_the_cold_wait() {
    let mut c = base(2, 1, WaitKind::Marker);
    c.track = V_P11;
    holds("W=2 hot=1 baton", &c);
}

/// The baton also starves the ENGINE EVENT (the guest's completion interrupt is raised from the same dispatch).
#[test]
fn p12_two_workers_one_hot_relay_the_baton_does_not_delay_the_engine_event() {
    let mut c = base(2, 1, WaitKind::Marker);
    c.track = V_P12;
    holds("W=2 hot=1 baton event", &c);
}

/// Control: the token path (finding a flagged cold token) is fair today: the scan visits every summary word and takes all flagged
/// tokens of a pass, so the cold relay's doorbell is claimed within 2 passes however hot the neighbours are.
#[test]
fn control_a_flagged_cold_token_is_claimed_within_two_passes() {
    let mut c = base(1, 2, WaitKind::Popped);
    c.wait = WaitKind::Popped;
    // the wait is never satisfied here: only the token bound is under test (P11 would fire otherwise), so the cold relay
    // starts with no wait
    let mut s_cfg = c.clone();
    s_cfg.cold_rings = 2;
    let r = explore_with(&s_cfg, |s| {
        s.wait = None;
        s.sat = false;
    });
    assert!(r.is_ok(), "token path: {:?}", r.err().map(|e| e.trace));
}

fn explore_with(cfg: &Cfg, init: impl Fn(&mut S)) -> Result<usize, Counterexample> {
    let mut start = new_state(cfg);
    init(&mut start);
    let mut seen: HashSet<S> = HashSet::new();
    let mut q = VecDeque::new();
    seen.insert(start.clone());
    q.push_back(start);
    let mut states = 0;
    while let Some(s) = q.pop_front() {
        states += 1;
        let v = invariant(&s) & V_TOKEN;
        if v != 0 {
            return Err(Counterexample {
                why: names(v),
                trace: vec![],
                states,
            });
        }
        for (_, n) in successors(cfg, &s) {
            if seen.insert(n.clone()) {
                q.push_back(n);
            }
        }
    }
    Ok(states)
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
        // every hot token is flagged again at once
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
// the checker has teeth: mutants of the loop's look, each found by at least two scenarios

fn w1(wait: WaitKind, track: u32, m: Mutant) -> Cfg {
    let mut c = base(1, 1, wait);
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
    mutant_no_look_after_busy_passes_is_found_by_a_marker_wait:
        w1(WaitKind::Marker, V_P11, Mutant { policy_continue_only: true, ..Mutant::default() }), "P11";
    mutant_no_look_after_busy_passes_is_found_by_a_carried_wait:
        w1(WaitKind::Popped, V_P11, Mutant { policy_continue_only: true, ..Mutant::default() }), "P11";
    mutant_no_look_after_busy_passes_is_found_by_the_engine_event:
        w1(WaitKind::Marker, V_P12, Mutant { policy_continue_only: true, ..Mutant::default() }), "P12";
    mutant_look_without_event_dispatch_is_found_by_the_engine_event:
        w1(WaitKind::Marker, V_P12, Mutant { look_skips_events: true, ..Mutant::default() }), "P12";
    mutant_look_without_event_dispatch_is_found_by_a_marker_wait_that_only_the_event_peeks:
        Cfg { cold_rings: 0, ..w1(WaitKind::Marker, V_P12, Mutant { look_skips_events: true, ..Mutant::default() }) }, "P12";
    mutant_look_without_peek_is_found_by_a_carried_wait:
        w1(WaitKind::Popped, V_P11, Mutant { look_skips_peek: true, ..Mutant::default() }), "P11";
    mutant_look_without_peek_is_found_by_a_lost_try_lock:
        Cfg { fails: 2, ..w1(WaitKind::Popped, V_P13, Mutant { look_skips_peek: true, ..Mutant::default() }) }, "P13";
}

// ------------------------------------------------------------------------------------------------
// the production loop really does what the model's Look does

const WORKER: &str = include_str!("../src/worker.rs");

fn look_arm() -> &'static str {
    let a = WORKER
        .split("AfterPass::Look => {")
        .nth(1)
        .expect("pin: the Look arm of run_with_relay is gone");
    a.split("continue;").next().expect("pin")
}

#[test]
fn pin_the_busy_look_dispatches_events_and_completions() {
    let l = look_arm();
    assert!(
        l.contains("PollTimeout::Immediate")
            && l.contains("dispatch(&ready)")
            && l.contains("relay.peek(Peek::Event)"),
        "{l}"
    );
}

#[test]
fn pin_the_busy_look_peeks_the_waiting_relays() {
    let l = look_arm();
    assert!(
        l.contains("relay.waiting()") && l.contains("relay.peek(Peek::Spin)"),
        "{l}"
    );
}

#[test]
fn pin_a_busy_pass_asks_the_policy() {
    assert!(
        WORKER.contains("policy.after_pass(n)"),
        "pin: run_with_relay no longer asks LoopPolicy after a busy pass"
    );
    assert!(WORKER.contains("pub const LOOK_EVERY: u32 = 4;"));
}

#[test]
fn pin_the_busy_look_order_is_poll_dispatch_event_peek_then_spin_peek() {
    let l = look_arm();
    let order = [
        "PollTimeout::Immediate",
        "dispatch(&ready)",
        "relay.peek(Peek::Event)",
        "relay.waiting()",
        "relay.peek(Peek::Spin)",
    ];
    let mut at = 0;
    for o in order {
        let i = l[at..]
            .find(o)
            .unwrap_or_else(|| panic!("pin: {o:?} missing or out of order in the Look arm"));
        at += i + o.len();
    }
}
