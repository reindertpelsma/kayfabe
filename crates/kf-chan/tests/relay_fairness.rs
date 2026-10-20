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
    /// The negative control: the production policy `LoopPolicy::without_busy_look()` (`KF3_NEGCTL_NO_BUSY_LOOK=1`).
    negctl: bool,
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
    /// The LAST STEP evaluated the cold relay's wait (a look that was entitled to act): the progress detector's `last_peek` stamp.
    peeked_now: bool,
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
                policy: if cfg.negctl {
                    LoopPolicy::without_busy_look()
                } else {
                    LoopPolicy::default()
                },
            })
            .collect(),
        lead: vec![0; cfg.workers],
        since_sat: 0,
        since_ev: 0,
        since_demand: 0,
        since_rung: 0,
        viol: 0,
        peeked_now: false,
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
            // `ChanPlane::user_ring_peek_one` stamps the relay when the look was entitled to act on its wait
            if retry_ok || !matches!(w, Wait::Contended) {
                self.peeked_now = true;
            }
            let (marks, popped) = if self.sat { (1, 1) } else { (0, 0) };
            if w.satisfied(marks, popped) && (retry_ok || !matches!(w, Wait::Contended)) {
                self.wait = None;
                self.ring(cold);
                return true;
            }
        }
        if self.demand && retry_ok {
            self.ring(cold);
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
    // the stamp describes one step only
    let cleared = S {
        peeked_now: false,
        ..s.clone()
    };
    let s = &cleared;
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
    /// The states along the minimal schedule, the violating one last (the start state first).
    path: Vec<S>,
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
            let mut path = vec![s.clone()];
            let mut cur = s.clone();
            while let Some((Some(p), l)) = seen.get(&cur) {
                trace.push(l.clone());
                path.push(p.clone());
                cur = p.clone();
            }
            trace.reverse();
            path.reverse();
            return Err(Counterexample {
                why: names(v),
                trace,
                path,
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
        negctl: false,
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
                path: vec![],
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

// ------------------------------------------------------------------------------------------------
// 2026-10-20 -- the NEGATIVE CONTROL and the PROGRESS DETECTOR tied to the model's property (P11)
//
// `KF3_NEGCTL_NO_BUSY_LOOK=1` selects the production policy `LoopPolicy::without_busy_look()` (the pre-fix loop); the runtime
// detector `kf_chan::progress::Tracker` (`KF3_DIAG_PROGRESS`) is driven here along the model's schedules with a FAKE CLOCK (one
// worker pass = one tick, threshold = K ticks). The property: the detector's condition "wait satisfied and not looked at for longer
// than the threshold" holds in a model state EXACTLY when the model flags P11, so (a) the failing schedule the model produces for
// the control triggers the detector at its last step and (b) no schedule of the fixed loop ever triggers it.

use kf_chan::progress::{Tracker, WaitKey};

const TICK_NS: u64 = 1_000_000;
const THRESHOLD_NS: u64 = K as u64 * TICK_NS;
const COLD_ID: u64 = 1;

/// The detector's view of the model along a path: returns, per state, whether the tracker's open interval exceeds the threshold,
/// and the index of the first state at which `observe` reported.
struct Replay {
    first_report: Option<usize>,
    /// per state: the open interval is longer than the threshold
    over: Vec<bool>,
}

fn replay(path: &[S]) -> Replay {
    let mut t = Tracker::new();
    let (mut clock, mut last_peek) = (TICK_NS, 0u64);
    let mut out = Replay {
        first_report: None,
        over: Vec::new(),
    };
    for (i, cur) in path.iter().enumerate() {
        if i > 0 {
            let prev = &path[i - 1];
            // a worker pass = a Scan step
            if prev
                .wk
                .iter()
                .zip(&cur.wk)
                .any(|(a, b)| a.pc == Pc::Scan && b.pc == Pc::Claim)
            {
                clock += TICK_NS;
            }
        }
        if cur.peeked_now {
            last_peek = clock;
        }
        let wait = cur.wait.map(WaitKey::of);
        let satisfied = cur.wait.is_some() && cur.sat;
        let reported = t.observe(clock, COLD_ID, wait, satisfied, last_peek, THRESHOLD_NS);
        if reported.is_some() && out.first_report.is_none() {
            out.first_report = Some(i);
        }
        out.over.push(
            satisfied
                && t.open_age_ns(COLD_ID, clock)
                    .is_some_and(|a| a > THRESHOLD_NS),
        );
    }
    out
}

fn negctl(workers: usize, hot: usize, wait: WaitKind, track: u32) -> Cfg {
    Cfg {
        negctl: true,
        track,
        ..base(workers, hot, wait)
    }
}

/// With the look off the model's P11 failing schedule makes the detector report, at the very state where the model flags P11.
#[test]
fn the_control_s_p11_schedule_triggers_the_detector_at_the_violating_state() {
    for (name, cfg) in [
        ("W=1 marker", negctl(1, 1, WaitKind::Marker, V_P11)),
        ("W=1 popped", negctl(1, 1, WaitKind::Popped, V_P11)),
        ("W=2 baton", negctl(2, 1, WaitKind::Marker, V_P11)),
    ] {
        let e = finds(name, &cfg, "P11");
        let r = replay(&e.path);
        assert_eq!(
            r.first_report,
            Some(e.path.len() - 1),
            "{name}: the detector must report exactly when the model flags P11 (schedule: {:#?})",
            e.trace
        );
        // and never earlier: the condition is false in every state before the violating one
        assert!(r.over[..e.path.len() - 1].iter().all(|o| !o), "{name}");
        assert!(r.over[e.path.len() - 1], "{name}");
    }
}

/// The control IS the retired behaviour: the same minimal failing schedule as the `policy_continue_only` mutant.
#[test]
fn the_control_policy_is_the_continue_only_mutant() {
    for wait in [WaitKind::Marker, WaitKind::Popped] {
        let ctl = negctl(1, 1, wait, V_P11);
        let mutant = w1(
            wait,
            V_P11,
            Mutant {
                policy_continue_only: true,
                ..Mutant::default()
            },
        );
        let (a, b) = (finds("ctl", &ctl, "P11"), finds("mutant", &mutant, "P11"));
        assert_eq!(a.trace.len(), b.trace.len());
    }
}

/// Deterministic random walks over the model's successors.
fn walks(cfg: &Cfg, n: usize, len: usize, mut f: impl FnMut(&[S])) {
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut rnd = move |m: usize| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((seed >> 33) as usize) % m
    };
    for _ in 0..n {
        let mut path = vec![new_state(cfg)];
        for _ in 0..len {
            let succ = successors(cfg, path.last().unwrap());
            if succ.is_empty() {
                break;
            }
            let pick = rnd(succ.len());
            let (_, next) = succ.into_iter().nth(pick).unwrap();
            let bad = invariant(&next) & V_P11 != 0;
            path.push(next);
            if bad {
                break;
            }
        }
        f(&path);
    }
}

/// With the look ON no schedule reaches the detector's condition, and the model's bound holds (K passes) on every walk.
#[test]
fn the_fixed_loop_never_triggers_the_detector_on_any_walk() {
    for (name, cfg) in [
        (
            "W=1 marker",
            Cfg {
                track: V_P11,
                ..base(1, 1, WaitKind::Marker)
            },
        ),
        (
            "W=1 popped",
            Cfg {
                track: V_P11,
                ..base(1, 1, WaitKind::Popped)
            },
        ),
        (
            "W=2 baton",
            Cfg {
                track: V_P11,
                ..base(2, 1, WaitKind::Marker)
            },
        ),
        (
            "W=1 lock convoy",
            Cfg {
                track: V_P11,
                fails: 2,
                ..base(1, 1, WaitKind::Popped)
            },
        ),
    ] {
        let mut total = 0usize;
        walks(&cfg, 300, 120, |path| {
            let r = replay(path);
            assert_eq!(
                r.first_report, None,
                "{name}: the fixed loop must never trigger the detector"
            );
            assert!(r.over.iter().all(|o| !o), "{name}");
            assert_eq!(invariant(path.last().unwrap()) & V_P11, 0, "{name}");
            total += path.len();
        });
        assert!(
            total > 300 * 20,
            "{name}: walks too short to mean anything ({total} states)"
        );
    }
}

/// Exactness on random schedules of the control: the detector's condition holds in a state iff the model flags P11 there; and the
/// walks are not vacuous (some of them do reach the violation).
#[test]
fn on_the_controls_walks_the_detector_condition_equals_the_models_p11() {
    for (name, cfg) in [
        ("W=1 marker", negctl(1, 1, WaitKind::Marker, V_P11)),
        ("W=1 popped", negctl(1, 1, WaitKind::Popped, V_P11)),
        ("W=2 baton", negctl(2, 1, WaitKind::Marker, V_P11)),
    ] {
        let (mut violations, mut walks_n) = (0, 0);
        walks(&cfg, 300, 120, |path| {
            walks_n += 1;
            let r = replay(path);
            for (i, s) in path.iter().enumerate() {
                assert_eq!(
                    r.over[i],
                    invariant(s) & V_P11 != 0,
                    "{name}: state {i} of a {}-state walk",
                    path.len()
                );
            }
            if r.first_report.is_some() {
                violations += 1;
                assert_eq!(
                    r.first_report,
                    Some(path.len() - 1),
                    "{name}: reported at the violating state"
                );
            }
        });
        assert!(
            violations > 0,
            "{name}: no walk of {walks_n} reached the starvation; the check is vacuous"
        );
    }
}

/// The control's engine-event starvation (P12) is the same cause: with the look off the event waits; with it on it does not.
#[test]
fn the_control_starves_the_engine_event_too_and_the_fix_does_not() {
    finds("ctl P12", &negctl(1, 1, WaitKind::Marker, V_P12), "P12");
    holds(
        "fix P12",
        &Cfg {
            track: V_P12,
            ..base(1, 1, WaitKind::Marker)
        },
    );
}

// ---- the production loop carries its hooks where the model's steps are (source pins)

fn run_body() -> &'static str {
    WORKER
        .split("pub fn run_with_relay(")
        .nth(1)
        .expect("pin")
        .split("#[cfg(test)]")
        .next()
        .expect("pin")
}

/// Every hook is one line `if let Some(t) = &tap { t.NAME(..); }`: no clock, lock or allocation outside the `Some` branch.
#[test]
fn pin_every_progress_hook_is_a_single_guarded_line_and_the_inventory_is_exact() {
    let body = run_body();
    let mut names: Vec<String> = Vec::new();
    for line in body.lines().filter(|l| l.contains("&tap {")) {
        let l = line.trim();
        assert!(
            l.starts_with("if let Some(t) = &tap { t.") && l.ends_with("; }"),
            "hook not in the guarded one-line form: {l}"
        );
        let name = l["if let Some(t) = &tap { t.".len()..]
            .split('(')
            .next()
            .unwrap()
            .to_string();
        names.push(name);
    }
    names.sort();
    assert_eq!(
        names,
        [
            "begin_pass",
            "dispatch_completions",
            "dispatch_other",
            "doorbell_drain",
            "end_pass",
            "look",
            "park_begin",
            "park_end",
            "spin",
        ],
        "hook inventory"
    );
    assert!(
        !body.contains("progress::now_ns"),
        "the worker never reads the clock itself"
    );
    assert!(
        body.contains("Tap::register()"),
        "registered once, before the loop"
    );
    assert_eq!(body.matches("Tap::register()").count(), 1);
}

#[test]
fn pin_the_hooks_sit_in_the_arm_they_describe() {
    let l = look_arm();
    assert!(l.contains("t.look()"), "the Look arm announces itself: {l}");
    let body = run_body();
    let spin = body
        .split("for _ in 0..user_loop::SPIN {")
        .nth(1)
        .expect("pin")
        .split("std::hint::spin_loop();")
        .next()
        .expect("pin");
    assert!(spin.contains("t.spin()"), "{spin}");
    let dispatch = body
        .split("let dispatch =")
        .nth(1)
        .expect("pin")
        .split("let mut scratch")
        .next()
        .expect("pin");
    for h in [
        "t.dispatch_other()",
        "t.dispatch_completions()",
        "t.doorbell_drain()",
    ] {
        assert!(dispatch.contains(h), "{h} missing from dispatch");
    }
    // the pass hooks bracket worker_pass
    let b = body.find("t.begin_pass()").expect("pin");
    let p = body.find("plane.worker_pass(").expect("pin");
    let e = body.find("t.end_pass(n)").expect("pin");
    assert!(b < p && p < e, "begin_pass < worker_pass < end_pass");
    // the park hooks bracket the park wait
    let pb = body.find("t.park_begin()").expect("pin");
    let w = body
        .find("poller.wait(&mut ready, PollTimeout::Millis(park))")
        .expect("pin");
    let pe = body.find("t.park_end()").expect("pin");
    assert!(pb < w && w < pe);
}

/// The relay's peeks are stamped by a wrapper that shadows `relay`, so the call sites (and the pins above) are unchanged.
#[test]
fn pin_peeks_are_stamped_by_a_wrapper_not_at_each_call() {
    let body = run_body();
    assert!(body.contains("struct Watched") || WORKER.contains("struct Watched"));
    assert!(body.contains("let relay: &dyn RelayLoop ="));
    assert_eq!(body.matches("relay.peek(Peek::Event)").count(), 3);
    assert_eq!(body.matches("relay.peek(Peek::Spin)").count(), 2);
    assert_eq!(body.matches("relay.peek(Peek::Backstop)").count(), 1);
    assert!(WORKER.contains("self.tap.peek(why)"));
}
