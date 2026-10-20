//! 2026-10-20 -- **Bounded fairness of the worker loop over the USER-ring relays** (P11, P12, P13).
//!
//! Hardware showed STARVATION on the relay route (Linux with `KF3_DEBUG_FORCE_MIGRATABLE=1`, Windows): one client starved for
//! 6-59 s while the other ran thousands of iterations, nothing stuck on the ring, the safety-net counters 0. A finite-schedule
//! liveness check cannot see that ("eventually served" holds after 59 s), so this model checks a BOUND: whatever the schedule, a
//! relay's satisfied wait, an engine event or a flagged token is attended to within `K` worker passes.
//!
//! ★ 2026-10-20 (simple relay, commit e49f0c225): TRANSLITERATED TO THE ONE-LOOP WORKER. The properties (P11, P12, P13, TOKEN, the
//! bound `K`, the configs) are unchanged; the model's loop, peek and cell mirror the new production, the park is INFINITE (no
//! timeout step exists), and a sleep-safety bit (`V_SLEEP`) is checked at every state.
//!
//! CODE READ (`kf_chan::worker::run_with_relay`, `kf_core::Plane::worker_pass`, `ChanPlane::user_ring_serve/arm/peek_one/unsourced`):
//!  * a pass serves every flagged token (`RungBitmap::scan`, limit 64, rotating start; claim `RUNG -> BUSY`; `ActAgain` up to
//!    `REACT_ROUNDS` rounds; then `RepublishAndMoveOn`). A relay that ended its turn with `Need::Again` rings its own token, so a hot
//!    relay is flagged again at once;
//!  * `worker_pass` returns the number of serves (a serve that did nothing, a contended one included, counts);
//!  * ONE loop: `after = policy.after_pass(n)`; unless `Continue` (the negative control) `rang = relay.peek(Peek::Spin)` -- a level
//!    evaluation of ALL relays after EVERY pass, busy or idle (`user_loop::look`: demand always re-rung, a satisfied wait cleared and
//!    rung); `spin = n > 0 || rang || relay.unsourced()` (demand, or a `Wait::Popped` no interrupt announces); spin -> `epoll_wait(0)`,
//!    else `try_park(seen)` and a BLOCKING wait; then `dispatch` (doorbell drained, engine event -> `on_other`; no peek).
//!
//! The model drives the production `worker::LoopPolicy::after_pass` and the production `user_loop::{look, look_needed, Wait}`; the
//! token/bitmap layer is the production state machine transliterated (`kf-trap` `TokenWord`, `RungBitmap::scan`: fairness of FINDING
//! is separately tested below with the real bitmap). Worker counts 1, 2 and 3 are explored (see the tests).
#![allow(
    clippy::too_many_lines,
    clippy::needless_range_loop,
    dead_code,
    missing_docs
)]

use kf_chan::user_loop::{self as ul, Look, LookIn, Peek, Wait};
use kf_chan::worker::{AfterPass, LoopPolicy};
use kf_trap::REACT_ROUNDS;
use std::collections::{HashMap, HashSet, VecDeque};

/// The bound, in worker passes (all workers' passes counted), independent of the hot relays.
const K: u8 = 16;
/// Scheduler fairness: no worker runs more than `DELTA` steps ahead of the slowest one (real threads both run; an adversary that
/// never schedules the idle worker would be an unfair OS, not a defect of the loop).
const DELTA: u8 = 14;

const V_P11: u32 = 1; // a satisfied wait of the cold relay was not rung within K passes
const V_P12: u32 = 2; // an engine event was not dispatched within K passes
const V_P13: u32 = 4; // left-behind demand (busy lock) was not re-rung within K passes
const V_TOKEN: u32 = 8; // the cold relay's flagged token was not claimed within 2 passes
const V_SLEEP: u32 = 16; // every worker is in a BLOCKING park while an obligation has no registered wake source

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
        (
            V_SLEEP,
            "SLEEP every worker blocks (no timeout) while an obligation has no registered wake source",
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
    /// The look is done; `spin` = `n > 0 || rang`; `|| unsourced()` decides poll(0) or park.
    Decide {
        spin: bool,
    },
    /// `epoll_wait(0)` + dispatch.
    Poll,
    /// `try_park(seen)`.
    Park,
    /// In the BLOCKING `epoll_wait` (no timeout).
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
    /// The RETIRED pre-fix loop's busy pass: after a pass that served something go straight to the next pass, with no poll(0) and no
    /// dispatch (the production loop polls every iteration, `Continue` included).
    busy_pass_skips_poll: bool,
    /// The look does not re-ring demand that has no wait behind it (a weaker look: used to show the demand leg of `unsourced()` is
    /// the safety net of the look, and that `V_SLEEP` sees its deletion).
    look_skips_demand: bool,
    /// `unsourced()` forgets recorded `Wait::Popped` waits (the carried leg).
    unsourced_ignores_carried: bool,
    /// `unsourced()` forgets durable demand (the retry-set leg).
    unsourced_ignores_demand: bool,
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
    /// Scheduler fairness bound (`DELTA` by default); smaller for the 3-worker runs to keep the state space explorable.
    delta: u8,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct S {
    tok: Vec<T>,
    bit: Vec<bool>,
    wait: Option<Wait>,
    sat: bool,
    demand: bool,
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
    /// `ChanPlane::user_ring_unsourced` (+ the mutants): demand, or a recorded wait no interrupt announces. (The hot relays keep
    /// neither.)
    fn unsourced(&self, m: Mutant) -> bool {
        (self.demand && !m.unsourced_ignores_demand)
            || (self.wait.is_some_and(Wait::carried) && !m.unsourced_ignores_carried)
    }
    /// `ChanPlane::user_ring_peek_one` for the cold relay (the hot ones keep no wait and no demand): the production
    /// `look_needed` gate and THE pure decision `user_loop::look`, on the level words.
    fn peek(&mut self, cold: usize, why: Peek, m: Mutant) -> bool {
        if !ul::look_needed(why, self.demand, self.wait.is_some()) {
            return false;
        }
        if m.look_skips_demand && self.wait.is_none() {
            return false;
        }
        let wait = self.wait;
        if wait.is_some() {
            // `ChanPlane::user_ring_peek_one` stamps the relay when the look evaluated its wait
            self.peeked_now = true;
        }
        let i = LookIn {
            demand: self.demand,
            record_busy: false,
            wait,
            words: wait.map(|_| if self.sat { (1, 1) } else { (0, 0) }),
            fetched: 0,
            get_follows_pops: kf_chan::user_turn::guest_get_follows_pops(),
        };
        match ul::look(i) {
            Look::Leave => false,
            Look::Ring { clear_wait, .. } => {
                if clear_wait {
                    self.wait = None;
                }
                self.ring(cold);
                // review 2026-10-20: P13 is a bound PER demand episode
                self.since_demand = 0;
                true
            }
        }
    }
    /// An engine event is dispatched (`on_other`); NO peek: the next iteration's level look covers it.
    fn dispatch_event(&mut self, cfg: &Cfg) {
        if cfg.m.look_skips_events {
            return;
        }
        self.ev = false;
        self.since_ev = 0;
    }
    /// Sleep safety: EVERY worker is in the blocking wait while an obligation lacks a registered wake source (the doorbell eventfd
    /// and the engine event are the sources): demand left behind, a recorded `Wait::Popped`, a recorded wait the words already
    /// satisfy, or a rung token without the eventfd. (A single parked worker beside an awake one is not a sleep: the awake worker
    /// evaluates the level state at its next look and cannot park while the obligation is unsourced.)
    fn sleep_unsafe(&self) -> bool {
        self.wk.iter().all(|w| w.pc == Pc::Parked)
            && !self.efd
            && !self.ev
            && (self.demand
                || self.wait.is_some_and(Wait::carried)
                || (self.wait.is_some() && self.sat)
                || self.bit.iter().any(|b| *b)
                || self.tok.iter().any(|t| matches!(t, T::Rung | T::BusyRung)))
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
        if s.lead[i] >= cfg.delta {
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
                        variants.push((me("serve the cold relay: try_lock lost against the neighbour, demand stays"), f));
                    }
                    let mut ok = base;
                    ok.demand = false;
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
                // unless `Continue`: the level look (`relay.peek(Peek::Spin)`), busy or idle
                let skip = next == AfterPass::Continue || (cfg.m.look_skips_peek && served > 0);
                let rang = if skip { false } else { n.peek(cold, Peek::Spin, cfg.m) };
                n.wk[i].pc = Pc::Decide {
                    spin: served > 0 || rang,
                };
                out.push((
                    me(&format!(
                        "after a pass that served {served}: {next:?}, level look rang={rang}"
                    )),
                    n,
                ));
            }
            Pc::Decide { spin } => {
                let mut n = s.clone();
                if cfg.m.busy_pass_skips_poll && n.wk[i].n > 0 {
                    n.wk[i].pc = Pc::Scan;
                    out.push((me("RETIRED loop: a busy pass goes straight on (no poll)"), n));
                } else if spin || n.unsourced(cfg.m) {
                    n.wk[i].pc = Pc::Poll;
                    out.push((
                        me("spin = n>0 || rang || unsourced(): epoll_wait(0)"),
                        n,
                    ));
                } else {
                    n.wk[i].pc = Pc::Park;
                    out.push((me("nothing to spin on: to try_park"), n));
                }
            }
            Pc::Poll => {
                let mut n = s.clone();
                // epoll_wait(0) returns every ready fd: the doorbell is drained, an engine event goes to `on_other`
                n.efd = false;
                if n.ev {
                    n.dispatch_event(cfg);
                }
                n.wk[i].pc = Pc::Scan;
                out.push((me("poll(0) + dispatch (no peek)"), n));
            }
            Pc::Park => {
                let mut n = s.clone();
                if n.moved {
                    n.wk[i].pc = Pc::Scan;
                    out.push((me("try_park refused"), n));
                } else {
                    n.wk[i].pc = Pc::Parked;
                    out.push((me("parked (blocking, no timeout)"), n));
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
                    n.dispatch_event(cfg);
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
    if s.sleep_unsafe() {
        v |= V_SLEEP;
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
        delta: DELTA,
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

/// ⚠ WAS EXPECTED RED (review 2026-10-20); NOW GREEN since the budget is deleted (simple relay, commit e49f0c225: the look
/// re-rings demand on every look, `CONTENTION_RETRIES` / `may_retry` / `Peek::Backstop` no longer exist; `fails = 64` is the old
/// budget as a literal). The text below describes the OLD defect. The convoy above has `fails = 2`, far below
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
    c.fails = 64; // the retired `CONTENTION_RETRIES`
    holds("W=1 hot=1 convoy past the retired budget (64)", &c);
}

/// Control for the red test above: one lost try_lock fewer than the budget, and the busy Look re-rings every episode within K.
#[test]
fn p13_control_a_convoy_just_below_contention_retries_is_re_rung_within_k_passes() {
    let mut c = base(1, 1, WaitKind::Popped);
    c.track = V_P13;
    c.cold_rings = 0;
    c.fails = 63;
    holds("W=1 hot=1 convoy just below the retired budget (63)", &c);
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
    mutant_no_look_and_no_poll_after_busy_passes_is_found_by_the_engine_event:
        w1(WaitKind::Marker, V_P12, Mutant { policy_continue_only: true, busy_pass_skips_poll: true, ..Mutant::default() }), "P12";
    mutant_busy_pass_without_poll_is_found_by_the_engine_event:
        w1(WaitKind::Marker, V_P12, Mutant { busy_pass_skips_poll: true, ..Mutant::default() }), "P12";
    mutant_look_without_event_dispatch_is_found_by_the_engine_event:
        w1(WaitKind::Marker, V_P12, Mutant { look_skips_events: true, ..Mutant::default() }), "P12";
    mutant_look_without_event_dispatch_is_found_by_a_marker_wait_with_no_doorbell:
        Cfg { cold_rings: 0, ..w1(WaitKind::Marker, V_P12, Mutant { look_skips_events: true, ..Mutant::default() }) }, "P12";
    mutant_look_without_peek_is_found_by_a_carried_wait:
        w1(WaitKind::Popped, V_P11, Mutant { look_skips_peek: true, ..Mutant::default() }), "P11";
    mutant_look_without_peek_is_found_by_a_lost_try_lock:
        Cfg { fails: 2, ..w1(WaitKind::Popped, V_P13, Mutant { look_skips_peek: true, ..Mutant::default() }) }, "P13";
}

// ------------------------------------------------------------------------------------------------
// SLEEP SAFETY (V_SLEEP): every worker blocks only when every obligation has a registered wake source. Worker counts 1, 2, 3.

/// A cold relay with no hot neighbour: the workers park whenever the loop lets them.
fn quiet(workers: usize, wait: WaitKind, m: Mutant) -> Cfg {
    Cfg {
        track: V_SLEEP,
        m,
        ..base(workers, 0, wait)
    }
}

#[test]
fn sleep_safety_holds_with_one_worker() {
    for (name, c) in [
        ("popped", quiet(1, WaitKind::Popped, Mutant::default())),
        ("marker", quiet(1, WaitKind::Marker, Mutant::default())),
        (
            "popped+convoy",
            Cfg {
                fails: 3,
                ..quiet(1, WaitKind::Popped, Mutant::default())
            },
        ),
    ] {
        holds(&format!("sleep W=1 {name}"), &c);
    }
}

#[test]
fn sleep_safety_holds_with_two_workers() {
    for (name, c) in [
        ("popped", quiet(2, WaitKind::Popped, Mutant::default())),
        ("marker", quiet(2, WaitKind::Marker, Mutant::default())),
        (
            "popped+convoy",
            Cfg {
                fails: 2,
                ..quiet(2, WaitKind::Popped, Mutant::default())
            },
        ),
        (
            "marker+two doorbells",
            Cfg {
                cold_rings: 2,
                ..quiet(2, WaitKind::Marker, Mutant::default())
            },
        ),
    ] {
        holds(&format!("sleep W=2 {name}"), &c);
    }
}

#[test]
fn sleep_safety_holds_with_three_workers() {
    for (name, c) in [
        ("popped", quiet(3, WaitKind::Popped, Mutant::default())),
        ("marker", quiet(3, WaitKind::Marker, Mutant::default())),
        (
            "popped+convoy (delta 4)",
            Cfg {
                fails: 1,
                delta: 4,
                ..quiet(3, WaitKind::Popped, Mutant::default())
            },
        ),
    ] {
        let n = holds(&format!("sleep W=3 {name}"), &c);
        eprintln!("W=3 {name}: {n} states");
    }
}

fn carried_mutant() -> Mutant {
    Mutant {
        unsourced_ignores_carried: true,
        ..Mutant::default()
    }
}
fn demand_mutant() -> Mutant {
    Mutant {
        unsourced_ignores_demand: true,
        ..Mutant::default()
    }
}

macro_rules! sleep_mutants {
    ($( $name:ident : $cfg:expr; )*) => {
        $( #[test] fn $name() {
            let c: Cfg = $cfg;
            holds(concat!(stringify!($name), "/control"), &Cfg { m: Mutant::default(), ..c.clone() });
            finds(stringify!($name), &c, "SLEEP");
        } )*
    };
}

sleep_mutants! {
    mutant_unsourced_ignores_carried_is_found_with_one_worker:
        quiet(1, WaitKind::Popped, carried_mutant());
    mutant_unsourced_ignores_carried_is_found_with_two_workers:
        quiet(2, WaitKind::Popped, carried_mutant());
    mutant_unsourced_ignores_carried_is_found_with_three_workers:
        quiet(3, WaitKind::Popped, carried_mutant());
    mutant_unsourced_ignores_carried_is_found_with_a_lock_convoy:
        Cfg { fails: 1, ..quiet(2, WaitKind::Popped, carried_mutant()) };
}

/// ★ `unsourced_ignores_demand` is EQUIVALENT in this model (and in the one-worker pointer model): the look that precedes the park
/// decision rings every demand it sees (`user_loop::look`), the ring marks the token rung (`try_park` is refused) or wakes the
/// parked workers by the eventfd, and a failed serve is followed by that same look. Four exhaustive scenarios (W = 1, 2, 3) hold
/// with the leg deleted. The leg is belt and braces: it is load-bearing only if the look may skip a demand.
/// The demand leg is load-bearing exactly when the look may skip a demand: with a look that skips demand-only relays, `unsourced()`
/// keeps the worker SPINNING (no sleep, though P13 would be violated: nobody re-rings), and deleting the leg too makes the worker
/// BLOCK with the demand unsourced -- found by `V_SLEEP`, with one and with two workers.
fn weak_look_demand_case(name: &str, workers: usize, fails: u8) {
    let weak = Mutant {
        look_skips_demand: true,
        ..Mutant::default()
    };
    let both = Mutant {
        unsourced_ignores_demand: true,
        ..weak
    };
    holds(
        &format!("weak look, leg kept {name}"),
        &Cfg {
            fails,
            ..quiet(workers, WaitKind::Popped, weak)
        },
    );
    finds(
        &format!("weak look, leg deleted {name}"),
        &Cfg {
            fails,
            ..quiet(workers, WaitKind::Popped, both)
        },
        "SLEEP",
    );
}

#[test]
fn mutant_unsourced_ignores_demand_is_found_once_the_look_skips_demand_with_one_worker() {
    weak_look_demand_case("W=1", 1, 2);
}

#[test]
fn mutant_unsourced_ignores_demand_is_found_once_the_look_skips_demand_with_two_workers() {
    weak_look_demand_case("W=2", 2, 1);
}

#[test]
fn mutant_unsourced_ignores_demand_is_equivalent_with_one_and_two_workers() {
    for (name, c) in [
        (
            "W=1 convoy",
            Cfg {
                fails: 3,
                ..quiet(1, WaitKind::Popped, demand_mutant())
            },
        ),
        (
            "W=2 convoy",
            Cfg {
                fails: 2,
                ..quiet(2, WaitKind::Popped, demand_mutant())
            },
        ),
    ] {
        holds(&format!("equiv {name}"), &c);
    }
}

#[test]
fn mutant_unsourced_ignores_demand_is_equivalent_with_three_workers_and_with_two_doorbells() {
    for (name, c) in [
        (
            "W=3 convoy (delta 4)",
            Cfg {
                fails: 1,
                delta: 4,
                ..quiet(3, WaitKind::Popped, demand_mutant())
            },
        ),
        (
            "W=2 marker two doorbells + convoy",
            Cfg {
                fails: 1,
                cold_rings: 2,
                ..quiet(2, WaitKind::Marker, demand_mutant())
            },
        ),
    ] {
        holds(&format!("equiv {name}"), &c);
    }
}

// ------------------------------------------------------------------------------------------------
// worker counts 1, 2 and 3 for the bounded properties

#[test]
fn p11_p12_p13_three_workers_one_hot_relay() {
    let mut c = base(3, 1, WaitKind::Marker);
    c.track = V_P11 | V_P12;
    c.cold_rings = 0;
    c.delta = 4;
    let n = holds("W=3 hot=1 marker P11+P12 (delta 4)", &c);
    eprintln!("W=3 hot=1 marker: {n} states");
}

// ------------------------------------------------------------------------------------------------
// the production loop really does what the model does (source pins)

const WORKER: &str = include_str!("../src/worker.rs");

#[test]
fn pin_the_deleted_mechanisms_are_gone() {
    let prod = WORKER.split("#[cfg(test)]").next().unwrap();
    for gone in ["const LOOK_EVERY", "Peek::Backstop", "user_loop::idle", "user_loop::SPIN"] {
        assert!(!prod.contains(gone), "{gone} is back: revisit the model");
    }
}

/// Replaces `pin_the_busy_look_dispatches_events_and_completions` / `..._peeks_the_waiting_relays` / `pin_a_busy_pass_asks_the_policy`
/// (LOOK_EVERY pin) / `pin_the_busy_look_order_is_poll_dispatch_event_peek_then_spin_peek`: the Look arm and the idle spin are gone,
/// the loop is ONE sequence and its ORDER is the model's: pass, policy, level look, spin decision, poll(0) or park, dispatch.
#[test]
fn pin_the_one_loop_order_is_pass_policy_level_look_spin_decision_wait_dispatch() {
    let b = run_body();
    let order = [
        "plane.worker_pass(",
        "policy.after_pass(n)",
        "relay.peek(Peek::Spin)",
        "let spin = n > 0 || rang || relay.unsourced();",
        "PollTimeout::Immediate",
        "try_park(seen)",
        "PollTimeout::Blocking",
        "dispatch(&ready)",
    ];
    let mut at = 0;
    for o in order {
        let i = b[at..]
            .find(o)
            .unwrap_or_else(|| panic!("pin: {o:?} missing or out of order in run_with_relay"));
        at += i + o.len();
    }
    // dispatch makes no peek of its own (the model's `Poll` / `Parked` steps do none)
    let dispatch = b
        .split("let dispatch =")
        .nth(1)
        .expect("pin")
        .split("let mut scratch")
        .next()
        .expect("pin");
    assert!(!dispatch.contains("relay.peek"), "dispatch peeks again: revisit the model");
    // the negative control skips ONLY the look
    assert!(b.contains("if after == AfterPass::Continue {"));
    assert_eq!(b.matches("relay.peek(").count(), 1, "one look site");
}

#[test]
fn pin_the_busy_policy_is_look_every_pass() {
    assert!(WORKER.contains("policy.after_pass(n)"));
    assert!(
        WORKER.contains("let mut p = LoopPolicy::default();")
            && WORKER.contains("a_busy_worker_looks_after_every_pass_and_an_empty_pass_idles"),
        "the production test of the every-pass look changed"
    );
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
    // the baton (W=2) needs both workers to stay busy on the one hot token in lock step: a random walk rarely does, so its
    // exactness is checked on the walks that do reach it and its reachability by the exhaustive test above
    for (name, cfg, must_reach) in [
        ("W=1 marker", negctl(1, 1, WaitKind::Marker, V_P11), true),
        ("W=1 popped", negctl(1, 1, WaitKind::Popped, V_P11), true),
        ("W=2 baton", negctl(2, 1, WaitKind::Marker, V_P11), false),
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
            violations > 0 || !must_reach,
            "{name}: no walk of {walks_n} reached the starvation; the check is vacuous"
        );
    }
}

/// REPLACES `the_control_starves_the_engine_event_too_and_the_fix_does_not`. The one-loop worker polls (epoll timeout 0) and
/// dispatches in EVERY iteration, busy or idle, the control included, so the control no longer starves the engine event (P12): it
/// withholds only the level look. The retired loop (the control AND no poll after a busy pass) did starve it, and the fix does not.
#[test]
fn the_control_no_longer_starves_the_engine_event_the_retired_loop_did_and_the_fix_does_not() {
    holds("ctl P12 (the control still polls)", &negctl(1, 1, WaitKind::Marker, V_P12));
    finds(
        "retired loop P12",
        &Cfg {
            m: Mutant {
                busy_pass_skips_poll: true,
                ..Mutant::default()
            },
            ..negctl(1, 1, WaitKind::Marker, V_P12)
        },
        "P12",
    );
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

/// Every hook is one `hook!(tap, name(..));` line (a call on the registered `Tap`, nothing when the diagnostic is off): no clock, lock
/// or allocation outside the `Some` branch.
#[test]
fn pin_every_progress_hook_is_a_single_guarded_line_and_the_inventory_is_exact() {
    let body = run_body();
    let mut names: Vec<String> = Vec::new();
    for line in body.lines().filter(|l| l.contains("hook!(")) {
        let l = line.trim();
        assert!(
            l.starts_with("hook!(tap, ") && l.ends_with(");"),
            "hook not in the one-line form: {l}"
        );
        let name = l["hook!(tap, ".len()..]
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
            "peek",
            "spin",
        ],
        "hook inventory"
    );
    assert!(
        !body.contains("&tap {"),
        "no open-coded hook: every hook is the macro"
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
    let mac = WORKER
        .split("macro_rules! hook {")
        .nth(1)
        .expect("pin: the hook macro")
        .split("/// The relay seen")
        .next()
        .unwrap();
    assert!(mac.contains("if let Some(t) = &$tap {"), "{mac}");
}

#[test]
fn pin_the_hooks_sit_in_the_arm_they_describe() {
    let body = run_body();
    // the look hook is the busy pass's: right before the level look
    let l = body.find("hook!(tap, look())").expect("pin: look hook");
    let pk = body.find("relay.peek(Peek::Spin)").expect("pin");
    assert!(l < pk, "the look announces itself before the peek");
    // the spin hook is in the poll(0) branch
    let sp = body.find("hook!(tap, spin())").expect("pin: spin hook");
    let imm = body.find("PollTimeout::Immediate").expect("pin");
    assert!(sp < imm, "the spin hook precedes poll(0)");
    let dispatch = body
        .split("let dispatch =")
        .nth(1)
        .expect("pin")
        .split("let mut scratch")
        .next()
        .expect("pin");
    for h in [
        "hook!(tap, dispatch_other())",
        "hook!(tap, dispatch_completions())",
        "hook!(tap, doorbell_drain())",
    ] {
        assert!(dispatch.contains(h), "{h} missing from dispatch");
    }
    // the pass hooks bracket worker_pass
    let b = body.find("hook!(tap, begin_pass())").expect("pin");
    let p = body.find("plane.worker_pass(").expect("pin");
    let e = body.find("hook!(tap, end_pass(n))").expect("pin");
    assert!(b < p && p < e, "begin_pass < worker_pass < end_pass");
    // the park hooks bracket the park wait
    let pb = body.find("hook!(tap, park_begin())").expect("pin");
    let w = body
        .find("poller.wait(&mut ready, timeout)")
        .expect("pin");
    let pe = body.find("hook!(tap, park_end())").expect("pin");
    assert!(pb < w && w < pe);
}

/// The relay's peeks are stamped by a wrapper that shadows `relay`, so the call sites (and the pins above) are unchanged.
#[test]
fn pin_peeks_are_stamped_by_a_wrapper_not_at_each_call() {
    let body = run_body();
    assert!(body.contains("struct Watched") || WORKER.contains("struct Watched"));
    assert!(body.contains("let relay: &dyn RelayLoop ="));
    assert_eq!(body.matches("relay.peek(Peek::Event)").count(), 0);
    assert_eq!(body.matches("relay.peek(Peek::Spin)").count(), 1);
    assert_eq!(body.matches("relay.peek(Peek::Backstop)").count(), 0);
    assert!(WORKER.contains("self.tap.peek(why)"));
}
