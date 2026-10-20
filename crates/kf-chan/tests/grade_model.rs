//! 2026-10-20 -- **Grader groups G1 / G2 / G4 / G5 / G6 at the MODEL level** (`docs/design/V3_RELAY_GRADER_20261020.md`).
//!
//! An exhaustive explicit-state explorer of the worker loop(s) and one real relay ("cold") beside 0..2 self-re-ringing hot
//! relays, a lock holder (act thread / silent record holder / `pt` holder / in-turn admission) and the hardware (marker
//! processed -> `REFERENCE`, then the engine interrupt; or pops with no interrupt). Every DECISION is the implementation's own,
//! asked through `grade_adapter.rs` (`look`, `after_pass`, the loop shape, `unsourced`, the contended arm), so the same
//! tests grade every lineage. The park is INFINITE (no timer transition exists).
//!
//! Checked on the full reachable graph:
//! * **strand** (G1/G2): a terminal state (nothing can step, every worker blocked) while the cold relay is not done;
//! * **sleep safety** (G2): a worker enters a blocking wait while the cold relay holds an obligation with no registered wake
//!   source (demand / the retry set, a satisfied wait nobody rang, a pop wait) and no fd is ready;
//! * **starvation** (G1/G6): a fair cycle (every worker that is runnable throughout steps in it) in which the cold relay is
//!   serviceable and makes no progress; and the longest serviceable path without progress, in steps, `<= K`;
//! * **spin reference** (G4): the same configuration with the loop forced never to block must not be better than the blocking one;
//! * **no hidden spin** (G5): a worker takes a timeout-0 wait although nothing was served, nothing rang and no obligation without
//!   a wake source exists.
//!
//! Abstractions (INFERRED, stated): one atomic step per loop phase; the old loop's pre-park spin is capped at 4 looks (with an
//! infinite park the length of a bounded spin cannot fix a strand); a hot relay's serve rings its own token while busy.
#![allow(dead_code, missing_docs, clippy::too_many_lines)]

#[path = "grade_adapter.rs"]
mod adapter;

use adapter::{AWait, After, Shape, Verdict, View, Why};
use std::collections::{HashMap, VecDeque};

const MODEL_SPIN_CAP: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum HolderKind {
    None,
    /// The act thread: holds the record lock, on release `user_ring_wake` (request, ring if free, always wake the worker).
    RecordAct,
    /// A record holder that announces nothing (a peer's look/serve stand-in).
    RecordSilent,
    /// The `pt` map lock (checked by the serve before the turn; rings nobody).
    Pt,
    /// An admission / storage lock lost INSIDE the turn (`Failure::Retry` -> `Need::Contended`).
    Admission,
}

#[derive(Clone, Copy, Debug)]
struct Cfg {
    workers: usize,
    hot: usize,
    holder: HolderKind,
    holds: u8,
    /// The wait the cold relay's first turn leaves.
    kind: AWait,
    /// The interrupt becomes visible BEFORE `REFERENCE` (the hazard row of the catalogue).
    irq_first: bool,
    /// G4 reference: the loop never blocks.
    force_spin: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pc {
    Top,
    Serve,
    ColdTurn,
    AfterPass,
    Peek,
    Decide,
    Poll,
    BusyLook,
    Spin(u8),
    TryPark,
    Parked,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Wk {
    pc: Pc,
    seen: u8,
    served: u8,
    rang: bool,
    queue: u8,
    cur: u8,
    pol: adapter::Policy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
struct Tok {
    rung: bool,
    busy: bool,
    again: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Rec {
    Free,
    Worker(u8),
    Holder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Hw {
    Idle,
    Pending,
    Half,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct St {
    w: Vec<Wk>,
    tok: Vec<Tok>,
    efd: bool,
    evfd: bool,
    seq: u8,
    // the cold relay (token index = hot count)
    demand: bool,
    waiting: bool,
    carried: bool,
    wait: Option<AWait>,
    crun: u8,
    rec: Rec,
    phase: u8,
    sat: bool,
    hw: Hw,
    // actors
    guest_done: bool,
    holding: bool,
    holds_left: u8,
}

struct Sys {
    c: Cfg,
    cold: usize,
}

#[derive(Default, Debug)]
struct Verdicts {
    states: usize,
    strands: Vec<String>,
    unsafe_sleeps: Vec<String>,
    hidden_spins: Vec<String>,
    starvation: Vec<String>,
    longest: usize,
    /// G5: quiet states (nothing owed) from which the workers can never all block.
    never_settles: usize,
    /// G5: cycles among quiet states that serve the cold relay again and again (the phantom-demand ring).
    phantom_cycles: usize,
}

impl Verdicts {
    fn ok(&self, k: usize) -> bool {
        self.strands.is_empty()
            && self.unsafe_sleeps.is_empty()
            && self.starvation.is_empty()
            && self.longest <= k
    }
    fn line(&self) -> String {
        format!(
            "states={} strands={} unsafe_sleeps={} hidden_spins={} never_settles={} phantom_cycles={} starvation={} longest_serviceable_path={} first: {:?} {:?} {:?} {:?}",
            self.states,
            self.strands.len(),
            self.unsafe_sleeps.len(),
            self.hidden_spins.len(),
            self.never_settles,
            self.phantom_cycles,
            self.starvation.len(),
            self.longest,
            self.strands.first(),
            self.unsafe_sleeps.first(),
            self.starvation.first(),
            self.hidden_spins.first()
        )
    }
}

impl Sys {
    fn new(c: Cfg) -> Self {
        Self { c, cold: c.hot }
    }

    fn init(&self) -> St {
        let mut tok = vec![Tok::default(); self.c.hot + 1];
        for t in tok.iter_mut().take(self.c.hot) {
            t.rung = true;
        }
        St {
            w: (0..self.c.workers)
                .map(|_| Wk {
                    pc: Pc::Top,
                    seen: 0,
                    served: 0,
                    rang: false,
                    queue: 0,
                    cur: 0,
                    pol: adapter::policy(),
                })
                .collect(),
            tok,
            efd: self.c.hot > 0,
            evfd: false,
            seq: 0,
            demand: false,
            waiting: false,
            carried: false,
            wait: None,
            crun: 0,
            rec: Rec::Free,
            phase: 0,
            sat: false,
            hw: Hw::Idle,
            guest_done: false,
            holding: false,
            holds_left: if self.c.holder == HolderKind::None { 0 } else { self.c.holds },
        }
    }

    fn crun_cap(&self) -> u8 {
        adapter::BUDGET.map_or(0, |b| u8::try_from(b + 1).unwrap_or(u8::MAX))
    }

    /// `Plane::ring_internal` + the worker eventfd signal.
    fn ring(s: &mut St, t: usize) {
        let k = &mut s.tok[t];
        if k.busy {
            k.again = true;
        } else if !k.rung {
            k.rung = true;
            s.efd = true;
            s.seq = s.seq.wrapping_add(1) % 4;
        }
    }

    fn done(&self, s: &St) -> bool {
        s.phase >= 2 && !s.tok[self.cold].rung && !s.tok[self.cold].busy && !s.demand
    }

    fn holder_blocks(&self, s: &St) -> bool {
        s.holding && matches!(self.c.holder, HolderKind::RecordAct | HolderKind::RecordSilent | HolderKind::Pt)
    }

    /// The cold relay has an obligation the system can act on now.
    fn serviceable(&self, s: &St) -> bool {
        if s.phase >= 2 {
            // the client's batch is answered; what remains is the relay's own bookkeeping (see `phantom`)
            return false;
        }
        if self.holder_blocks(s) {
            return false;
        }
        let t = s.tok[self.cold];
        t.rung || t.busy || s.demand || (s.wait.is_some() && s.sat) || s.wait == Some(AWait::Contended)
    }

    /// An obligation of the cold relay that no registered wake source will announce.
    fn unsourced_truth(&self, s: &St) -> Option<&'static str> {
        if s.tok[self.cold].rung || s.tok[self.cold].busy {
            return None;
        }
        if s.demand && !self.holder_blocks(s) {
            return Some("demand (retry set) with the lock free");
        }
        if s.demand && self.holder_blocks(s) && self.c.holder != HolderKind::RecordAct {
            return Some("demand behind a holder that rings nobody");
        }
        if s.wait.is_some() && s.sat && !s.evfd && s.hw != Hw::Half {
            return Some("a satisfied wait nobody rang");
        }
        if s.wait == Some(AWait::Popped) && s.hw != Hw::Done {
            return Some("a pop wait (no interrupt)");
        }
        if s.wait == Some(AWait::Contended) {
            return Some("a contended wait");
        }
        None
    }

    fn view(&self, s: &St, why: Why) -> View {
        View {
            why,
            demand: s.demand,
            record_busy: s.rec != Rec::Free,
            waiting_flag: s.waiting,
            wait: s.wait,
            satisfied: s.sat,
            storage_busy: false,
            crun: u32::from(s.crun),
        }
    }

    /// One look at every relay (hot relays have no cell). Returns whether it rang.
    fn look(&self, s: &mut St, why: Why) -> bool {
        match adapter::look(self.view(s, why)) {
            Verdict::Leave => false,
            Verdict::Ring { clear_wait } => {
                if clear_wait && s.rec == Rec::Free {
                    s.wait = None;
                    s.waiting = false;
                    s.carried = false;
                }
                Self::ring(s, self.cold);
                true
            }
        }
    }

    fn relays_waiting(s: &St) -> bool {
        s.waiting || s.demand
    }

    fn dispatch(s: &mut St) -> (bool, bool) {
        let r = (s.efd, s.evfd);
        s.efd = false;
        s.evfd = false;
        r
    }

    fn finish_token(s: &mut St, t: usize) {
        let k = &mut s.tok[t];
        k.busy = false;
        if k.again {
            k.again = false;
            k.rung = true;
        }
    }

    fn arm(&self, s: &mut St, need: Option<AWait>, contended: bool) {
        if contended {
            s.crun = (s.crun + 1).min(self.crun_cap());
            if adapter::contended_leaves_demand() {
                s.demand = true;
            }
            s.wait = adapter::contended_wait();
            s.waiting = s.wait.is_some();
            s.carried = false;
            return;
        }
        s.crun = 0;
        s.wait = need;
        s.waiting = need.is_some();
        s.carried = need == Some(AWait::Popped);
    }

    /// The successor of worker `i` (None = not runnable).
    fn step_worker(&self, s: &St, i: usize) -> Option<(St, bool, Option<String>)> {
        let mut n = s.clone();
        let mut progress = false;
        let mut note = None;
        let wk = n.w[i].clone();
        match wk.pc {
            Pc::Top => {
                n.w[i].seen = n.seq;
                let mut q = 0u8;
                for (t, k) in n.tok.iter_mut().enumerate() {
                    if k.rung {
                        k.rung = false;
                        k.busy = true;
                        q |= 1 << t;
                    }
                }
                n.w[i].queue = q;
                n.w[i].served = 0;
                n.w[i].rang = false;
                n.w[i].pc = Pc::Serve;
            }
            Pc::Serve => {
                let q = wk.queue;
                if q == 0 {
                    n.w[i].pc = Pc::AfterPass;
                } else {
                    let t = q.trailing_zeros() as usize;
                    n.w[i].queue &= !(1 << t);
                    n.w[i].served = (wk.served + 1).min(2);
                    if t < self.cold {
                        Self::ring(&mut n, t);
                        Self::finish_token(&mut n, t);
                    } else {
                        // `user_ring_serve`: `record.enter()` (2026-10-20): try_lock; ONLY on a loss request() and try again
                        if n.rec != Rec::Free {
                            n.demand = true;
                        }
                        if n.rec == Rec::Free {
                            n.rec = Rec::Worker(i as u8);
                            n.w[i].cur = t as u8;
                            n.w[i].pc = Pc::ColdTurn;
                        } else {
                            n.crun = (n.crun + 1).min(self.crun_cap());
                            Self::finish_token(&mut n, t);
                        }
                    }
                }
            }
            Pc::ColdTurn => {
                let t = wk.cur as usize;
                if n.holding && self.c.holder == HolderKind::Pt {
                    // the pt try_lock lost before the turn: contended_note, and the demand is made durable (`record.request()`)
                    n.crun = (n.crun + 1).min(self.crun_cap());
                    n.demand = true;
                } else {
                    n.demand = false; // retry.swap(false)
                    let owes_work = n.phase == 0 || (n.phase == 1 && n.sat);
                    if owes_work && n.holding && self.c.holder == HolderKind::Admission {
                        self.arm(&mut n, None, true);
                    } else if n.phase == 0 {
                        n.phase = 1;
                        n.hw = Hw::Pending;
                        progress = true;
                        self.arm(&mut n, Some(self.c.kind), false);
                    } else if n.phase == 1 && n.sat {
                        n.phase = 2;
                        progress = true;
                        self.arm(&mut n, None, false);
                    } else if n.phase == 1 {
                        self.arm(&mut n, Some(self.c.kind), false);
                    } else {
                        self.arm(&mut n, None, false);
                    }
                }
                n.rec = Rec::Free;
                Self::finish_token(&mut n, t);
                n.w[i].pc = Pc::Serve;
            }
            Pc::AfterPass => {
                let mut pol = wk.pol.clone();
                let a = adapter::after_pass(&mut pol, usize::from(wk.served));
                n.w[i].pol = pol;
                n.w[i].pc = match (adapter::loop_shape(), a) {
                    (_, After::Continue) => Pc::Top,
                    (Shape::OneLoop, _) => Pc::Peek,
                    (Shape::SpinThenPark { .. }, After::Look) => Pc::BusyLook,
                    (Shape::SpinThenPark { spin }, After::Idle) => {
                        if Self::relays_waiting(&n) {
                            Pc::Spin(u8::try_from(spin).unwrap_or(u8::MAX).min(MODEL_SPIN_CAP))
                        } else {
                            Pc::TryPark
                        }
                    }
                };
            }
            Pc::Peek => {
                n.w[i].rang = self.look(&mut n, Why::Spin);
                n.w[i].pc = Pc::Decide;
            }
            Pc::Decide => {
                let uns = adapter::unsourced(n.demand, n.waiting, n.carried, n.holding && self.c.holder == HolderKind::RecordAct);
                let spin = wk.served > 0 || wk.rang || uns || self.c.force_spin;
                let mid_turn = n.tok.iter().any(|t| t.busy) || matches!(n.rec, Rec::Worker(_)) || n.tok.iter().any(|t| t.rung) || n.efd || n.evfd;
                if spin && wk.served == 0 && !wk.rang && !mid_turn && self.unsourced_truth(&n).is_none() && !self.c.force_spin {
                    note = Some(format!("hidden spin: timeout 0 with nothing outstanding uncovered ({n:?})"));
                }
                n.w[i].pc = if spin { Pc::Poll } else { Pc::TryPark };
            }
            Pc::Poll => {
                let _ = Self::dispatch(&mut n);
                n.w[i].pc = Pc::Top;
            }
            Pc::BusyLook => {
                let (_db, ev) = Self::dispatch(&mut n);
                if ev {
                    self.look(&mut n, Why::Event);
                }
                if Self::relays_waiting(&n) {
                    self.look(&mut n, Why::Spin);
                }
                n.w[i].pc = Pc::Top;
            }
            Pc::Spin(left) => {
                if n.efd || n.evfd {
                    let (_db, ev) = Self::dispatch(&mut n);
                    if ev {
                        self.look(&mut n, Why::Event);
                    }
                    n.w[i].pc = Pc::Top;
                } else if self.look(&mut n, Why::Spin) {
                    n.w[i].pc = Pc::Top;
                } else if !Self::relays_waiting(&n) || left <= 1 {
                    if self.unsourced_truth(&n).is_none() && left > 1 {
                        // fine
                    }
                    n.w[i].pc = Pc::TryPark;
                } else {
                    let mid_turn = n.tok.iter().any(|t| t.busy) || matches!(n.rec, Rec::Worker(_)) || n.tok.iter().any(|t| t.rung) || n.efd || n.evfd;
                    if self.unsourced_truth(&n).is_none() && !mid_turn {
                        note = Some(format!("hidden spin: the pre-park spin looks with every obligation covered ({n:?})"));
                    }
                    n.w[i].pc = Pc::Spin(left - 1);
                }
            }
            Pc::TryPark => {
                if self.c.force_spin {
                    n.w[i].pc = Pc::Top;
                } else if n.seq != wk.seen {
                    n.w[i].pc = Pc::Top;
                } else {
                    n.w[i].pc = Pc::Parked;
                }
            }
            Pc::Parked => {
                if !(n.efd || n.evfd) {
                    return None;
                }
                let (_db, ev) = Self::dispatch(&mut n);
                if ev && matches!(adapter::loop_shape(), Shape::SpinThenPark { .. }) {
                    self.look(&mut n, Why::Event);
                }
                n.w[i].pc = Pc::Top;
            }
        }
        Some((n, progress, note))
    }

    fn step_hw(&self, s: &St) -> Option<St> {
        let mut n = s.clone();
        let marker = self.c.kind == AWait::Marker;
        match (s.hw, self.c.irq_first) {
            (Hw::Pending, false) => {
                n.sat = true;
                n.hw = if marker { Hw::Half } else { Hw::Done };
            }
            (Hw::Half, false) => {
                n.evfd = true;
                n.hw = Hw::Done;
            }
            (Hw::Pending, true) => {
                if marker {
                    n.evfd = true;
                    n.hw = Hw::Half;
                } else {
                    n.sat = true;
                    n.hw = Hw::Done;
                }
            }
            (Hw::Half, true) => {
                n.sat = true;
                n.hw = Hw::Done;
            }
            _ => return None,
        }
        Some(n)
    }

    fn step_holder(&self, s: &St) -> Option<St> {
        if self.c.holder == HolderKind::None {
            return None;
        }
        let mut n = s.clone();
        let record = matches!(self.c.holder, HolderKind::RecordAct | HolderKind::RecordSilent);
        if s.holding {
            n.holding = false;
            n.holds_left -= 1;
            if record {
                n.rec = Rec::Free;
            }
            if self.c.holder == HolderKind::RecordAct {
                // MIRROR of `user_ring_wake`: request, ring when the record is free, ALWAYS wake the worker
                n.demand = true;
                Self::ring(&mut n, self.cold);
                n.efd = true;
                n.seq = n.seq.wrapping_add(1) % 4;
            }
            return Some(n);
        }
        if s.holds_left == 0 {
            return None;
        }
        if record {
            if s.rec != Rec::Free {
                return None;
            }
            n.rec = Rec::Holder;
        }
        n.holding = true;
        Some(n)
    }

    fn step_guest(&self, s: &St) -> Option<St> {
        if s.guest_done {
            return None;
        }
        let mut n = s.clone();
        n.guest_done = true;
        // GP_PUT written, then the doorbell: trap -> ring the token -> the worker eventfd
        Self::ring(&mut n, self.cold);
        n.efd = true;
        Some(n)
    }

    /// Explore every reachable state; check the properties.
    fn explore(&self, cap: usize) -> Verdicts {
        let mut v = Verdicts::default();
        let mut idx: HashMap<St, usize> = HashMap::new();
        let mut states: Vec<St> = Vec::new();
        // edges: (to, actor, progress)
        let mut edges: Vec<Vec<(usize, u8, bool)>> = Vec::new();
        let s0 = self.init();
        idx.insert(s0.clone(), 0);
        states.push(s0);
        edges.push(Vec::new());
        let mut q = VecDeque::from([0usize]);
        while let Some(si) = q.pop_front() {
            assert!(states.len() <= cap, "state cap {cap} exceeded ({:?})", self.c);
            let s = states[si].clone();
            let mut succ: Vec<(St, u8, bool)> = Vec::new();
            for i in 0..self.c.workers {
                if let Some((n, prog, note)) = self.step_worker(&s, i) {
                    if s.w[i].pc == Pc::TryPark && n.w[i].pc == Pc::Parked && !(n.efd || n.evfd) && n.w.iter().all(|w| w.pc == Pc::Parked) {
                        if let Some(why) = self.unsourced_truth(&n) {
                            if v.unsafe_sleeps.len() < 4 {
                                v.unsafe_sleeps.push(format!("worker {i} blocks while {why}: {n:?}"));
                            } else {
                                v.unsafe_sleeps.push(String::new());
                            }
                        }
                    }
                    if let Some(m) = note {
                        if v.hidden_spins.len() < 4 {
                            v.hidden_spins.push(m);
                        } else {
                            v.hidden_spins.push(String::new());
                        }
                    }
                    succ.push((n, i as u8, prog));
                }
            }
            if let Some(n) = self.step_hw(&s) {
                succ.push((n, 100, false));
            }
            if let Some(n) = self.step_holder(&s) {
                succ.push((n, 101, false));
            }
            if let Some(n) = self.step_guest(&s) {
                succ.push((n, 102, false));
            }
            if succ.is_empty() && !self.done(&s) {
                if v.strands.len() < 4 {
                    v.strands.push(format!("terminal with the cold relay not done: {s:?}"));
                } else {
                    v.strands.push(String::new());
                }
            }
            for (n, a, p) in succ {
                let ni = if let Some(&j) = idx.get(&n) {
                    j
                } else {
                    let j = states.len();
                    idx.insert(n.clone(), j);
                    states.push(n);
                    edges.push(Vec::new());
                    q.push_back(j);
                    j
                };
                edges[si].push((ni, a, p));
            }
        }
        v.states = states.len();
        self.starvation(&states, &edges, &mut v);
        if self.c.hot == 0 {
            self.phantom(&states, &edges, &mut v);
        }
        v
    }

    /// G5: a cycle of worker steps after the client's batch was answered, with no guest work left, the hardware and the holder
    /// done: the workers keep serving / looking at a relay that owes nothing (a self-sustaining ring between workers).
    fn phantom(&self, states: &[St], edges: &[Vec<(usize, u8, bool)>], v: &mut Verdicts) {
        let quiet: Vec<bool> = states
            .iter()
            .map(|s| s.phase >= 2 && s.guest_done && s.hw == Hw::Done && !s.holding && s.holds_left == 0)
            .collect();
        // a quiet state from which no blocking state is reachable inside the quiet sub-graph = the workers never settle
        let n = states.len();
        let mut settles = vec![false; n];
        let mut rev: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut q = VecDeque::new();
        for u in 0..n {
            if !quiet[u] {
                continue;
            }
            for &(t, _, _) in &edges[u] {
                if quiet[t] {
                    rev[t].push(u);
                }
            }
            if states[u].w.iter().all(|w| w.pc == Pc::Parked) && !(states[u].efd || states[u].evfd) {
                settles[u] = true;
                q.push_back(u);
            }
        }
        while let Some(u) = q.pop_front() {
            for &p in &rev[u] {
                if !settles[p] {
                    settles[p] = true;
                    q.push_back(p);
                }
            }
        }
        // a quiet state that cannot settle is a hidden spin for ever; one that can but also cycles is a possible livelock
        let mut cycles = 0usize;
        for u in 0..n {
            if quiet[u] && !settles[u] {
                if v.hidden_spins.len() < 4 {
                    v.hidden_spins.push(format!("quiet state that never settles: {:?}", states[u]));
                }
                v.never_settles += 1;
            }
            if quiet[u] && edges[u].iter().any(|&(t, a, _)| quiet[t] && usize::from(a) < self.c.workers && t == u) {
                cycles += 1;
            }
        }
        let _ = cycles;
        // the phantom ring: a fair cycle among quiet states in which some worker serves the cold token
        let inside = &quiet;
        let mut onstack = vec![0u8; n];
        let mut found = 0usize;
        for root in 0..n {
            if !inside[root] || onstack[root] != 0 || found > 0 {
                continue;
            }
            // iterative DFS for a back edge through a cold serve (Serve -> ColdTurn)
            let mut st: Vec<(usize, usize)> = vec![(root, 0)];
            onstack[root] = 1;
            while let Some(&mut (u, ref mut k)) = st.last_mut() {
                if *k < edges[u].len() {
                    let (t, a, _) = edges[u][*k];
                    *k += 1;
                    if !inside[t] || usize::from(a) >= self.c.workers {
                        continue;
                    }
                    if onstack[t] == 1 {
                        let serves_cold = st.iter().any(|&(x, _)| states[x].rec != Rec::Free);
                        if serves_cold {
                            found += 1;
                        }
                    } else if onstack[t] == 0 {
                        onstack[t] = 1;
                        st.push((t, 0));
                    }
                } else {
                    onstack[u] = 2;
                    st.pop();
                }
            }
        }
        v.phantom_cycles = found;
    }

    /// Fair cycles without progress inside the serviceable sub-graph, and its longest acyclic path (worker steps).
    fn starvation(&self, states: &[St], edges: &[Vec<(usize, u8, bool)>], v: &mut Verdicts) {
        let n = states.len();
        let inside: Vec<bool> = states.iter().map(|s| self.serviceable(s)).collect();
        let sub = |u: usize| -> Vec<(usize, u8)> {
            edges[u]
                .iter()
                .filter(|(t, _, p)| !p && inside[*t])
                .map(|(t, a, _)| (*t, *a))
                .collect()
        };
        // Tarjan (iterative)
        let mut index = vec![usize::MAX; n];
        let mut low = vec![0usize; n];
        let mut on = vec![false; n];
        let mut stack = Vec::new();
        let mut comp = vec![usize::MAX; n];
        let mut ncomp = 0;
        let mut counter = 0;
        for root in 0..n {
            if !inside[root] || index[root] != usize::MAX {
                continue;
            }
            let mut call: Vec<(usize, usize)> = vec![(root, 0)];
            index[root] = counter;
            low[root] = counter;
            counter += 1;
            stack.push(root);
            on[root] = true;
            while let Some(&mut (u, ref mut k)) = call.last_mut() {
                let su = sub(u);
                if *k < su.len() {
                    let (w, _) = su[*k];
                    *k += 1;
                    if index[w] == usize::MAX {
                        index[w] = counter;
                        low[w] = counter;
                        counter += 1;
                        stack.push(w);
                        on[w] = true;
                        call.push((w, 0));
                    } else if on[w] {
                        low[u] = low[u].min(index[w]);
                    }
                } else {
                    call.pop();
                    if let Some(&(p, _)) = call.last() {
                        low[p] = low[p].min(low[u]);
                    }
                    if low[u] == index[u] {
                        while let Some(w) = stack.pop() {
                            on[w] = false;
                            comp[w] = ncomp;
                            if w == u {
                                break;
                            }
                        }
                        ncomp += 1;
                    }
                }
            }
        }
        // a non-trivial SCC is a fair starvation witness when every worker either steps inside it or is blocked somewhere in it
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); ncomp];
        for u in 0..n {
            if comp[u] != usize::MAX {
                members[comp[u]].push(u);
            }
        }
        let mut cyclic = vec![false; ncomp];
        for (c, m) in members.iter().enumerate() {
            let mut stepping = vec![false; self.c.workers];
            let mut blocked = vec![false; self.c.workers];
            let mut internal = false;
            for &u in m {
                for (t, a) in sub(u) {
                    if comp[t] == c {
                        internal = true;
                        if usize::from(a) < self.c.workers {
                            stepping[usize::from(a)] = true;
                        }
                    }
                }
                for (i, wk) in states[u].w.iter().enumerate() {
                    if wk.pc == Pc::Parked && !(states[u].efd || states[u].evfd) {
                        blocked[i] = true;
                    }
                }
            }
            let mut fair = (0..self.c.workers).all(|i| stepping[i] || blocked[i]);
            // weak fairness of the hardware, the holder's release and the guest: an actor enabled throughout must step
            let s0 = &states[m[0]];
            let mut actor_steps = [false; 3];
            for &u in m {
                for (t, a) in sub(u) {
                    if comp[t] == c && a >= 100 {
                        actor_steps[usize::from(a - 100)] = true;
                    }
                }
            }
            if matches!(s0.hw, Hw::Pending | Hw::Half) && !actor_steps[0] {
                fair = false;
            }
            if s0.holding && !actor_steps[1] {
                fair = false;
            }
            if !s0.guest_done && !actor_steps[2] {
                fair = false;
            }
            cyclic[c] = internal && fair;
            if internal && fair && stepping.iter().any(|&x| x) {
                if v.starvation.len() < 2 {
                    let s = &states[m[0]];
                    v.starvation.push(format!(
                        "a fair cycle of {} states where the cold relay is serviceable and never progresses, e.g. {s:?}",
                        m.len()
                    ));
                } else {
                    v.starvation.push(String::new());
                }
            }
        }
        // longest path (in worker steps) through the condensation, cycles counted once
        let mut memo: Vec<Option<usize>> = vec![None; ncomp];
        let mut order: Vec<usize> = (0..ncomp).collect();
        // Tarjan numbers SCCs in reverse topological order: successors first
        order.sort_unstable();
        let mut cedges: Vec<Vec<(usize, usize)>> = vec![Vec::new(); ncomp];
        for u in 0..n {
            if comp[u] == usize::MAX {
                continue;
            }
            for (t, a) in sub(u) {
                if comp[t] != comp[u] {
                    cedges[comp[u]].push((comp[t], usize::from(usize::from(a) < self.c.workers)));
                }
            }
        }
        for c in order {
            let inner = if cyclic[c] { members[c].len() } else { 0 };
            let best = cedges[c]
                .iter()
                .map(|&(t, w)| memo[t].unwrap_or(0) + w)
                .max()
                .unwrap_or(0);
            memo[c] = Some(best + inner);
            v.longest = v.longest.max(best + inner);
        }
    }
}

fn cfg(workers: usize, hot: usize, holder: HolderKind, kind: AWait) -> Cfg {
    Cfg {
        workers,
        hot,
        holder,
        holds: 1,
        kind,
        irq_first: false,
        force_spin: false,
    }
}

const CAP: usize = 6_000_000;
/// The bounded-wait bound K: worker steps the cold relay may wait while serviceable (all workers' steps counted).
fn k_bound(c: &Cfg) -> usize {
    60 * c.workers + 20 * c.hot + 40
}

fn check(c: Cfg) -> Verdicts {
    let v = Sys::new(c).explore(CAP);
    println!("{} {:?}: {}", adapter::LINEAGE, c, v.line());
    v
}

fn assert_green(c: Cfg) {
    let v = check(c);
    assert!(v.ok(k_bound(&c)), "{}: {c:?}: {}", adapter::LINEAGE, v.line());
}

const HOLDERS: [HolderKind; 5] = [
    HolderKind::None,
    HolderKind::RecordAct,
    HolderKind::RecordSilent,
    HolderKind::Pt,
    HolderKind::Admission,
];

fn exhaustive(workers: usize) {
    let mut bad = Vec::new();
    // three workers: the state space of hot relays x every holder exceeds the cap; W=3 runs hot 0 with every holder
    let hots: &[usize] = if workers >= 3 { &[0] } else { &[0, 1] };
    for &hot in hots {
        for holder in HOLDERS {
            if workers >= 3 && matches!(holder, HolderKind::RecordAct | HolderKind::RecordSilent) {
                continue; // > 6 M states; the record holders run with 1 and 2 workers
            }
            for kind in [AWait::Marker, AWait::Popped] {
                let c = cfg(workers, hot, holder, kind);
                let v = check(c);
                if !v.ok(k_bound(&c)) {
                    bad.push(format!("{c:?}: {}", v.line()));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{}: {} configurations fail:\n{}", adapter::LINEAGE, bad.len(), bad.join("\n"));
}

// ---------------------------------------------------------------------------------------------------------------- G1

/// G1: every configuration of 1 worker (hot 0/1, every holder kind, marker and pop waits): no strand, no unsafe sleep, no
/// starvation, bounded wait.
#[test]
fn g1_models_exhaustive_one_worker() {
    exhaustive(1);
}

#[test]
fn g1_models_exhaustive_two_workers() {
    exhaustive(2);
}

#[test]
fn g1_models_exhaustive_three_workers() {
    exhaustive(3);
}

/// G1 F1 (the convoy): the cold relay loses its locks past any retry budget (a silent record holder, then `pt`) beside a hot
/// relay, 1 and 2 workers, infinite park. Expected: served within K steps of the release, never stranded.
#[test]
fn g1_f1_convoy_past_budget_is_served() {
    for workers in [1, 2] {
        for holder in [HolderKind::RecordSilent, HolderKind::Pt, HolderKind::Admission] {
            for hot in [0, 1] {
                assert_green(cfg(workers, hot, holder, AWait::Marker));
            }
        }
    }
}

/// G1 F3: a worker kept busy by a self-re-ringing relay still looks: the cold relay's satisfied marker wait is rung within K.
#[test]
fn g1_f3_busy_worker_looks() {
    for hot in [1, 2] {
        assert_green(cfg(1, hot, HolderKind::None, AWait::Marker));
    }
}

/// G1 F5: two workers, the cold relay's record held by one worker's turn while the other dispatches the engine event and looks:
/// the look must not drop the consumed event.
#[test]
fn g1_f5_busy_record_look_keeps_the_event() {
    for hot in [0, 1] {
        assert_green(cfg(2, hot, HolderKind::None, AWait::Marker));
    }
}

/// G1: a pop wait (no interrupt can announce it: SYNC_WAIT behind an open packet, a credit wait) is never parked on.
#[test]
fn g1_popped_wait_is_spun_not_parked() {
    for workers in [1, 2, 3] {
        assert_green(cfg(workers, 0, HolderKind::None, AWait::Popped));
    }
}

/// G1: the act thread's release (`user_ring_wake`) reaches a relay whose serve lost the record to it.
#[test]
fn g1_act_thread_release_rings() {
    for workers in [1, 2] {
        let mut c = cfg(workers, 0, HolderKind::RecordAct, AWait::Marker);
        c.holds = 2;
        assert_green(c);
    }
}

/// G1 (OPEN hazard, catalogue): the engine interrupt visible BEFORE `REFERENCE`. With an infinite park no wake source covers the
/// late word. Not in the default suite: an open owner question (the expected behaviour is not decided), kept runnable.
#[test]
#[ignore = "OPEN: interrupt-before-REFERENCE hazard (catalogue); the expected behaviour awaits an owner decision"]
fn g1_open_irq_before_reference_hazard() {
    let mut c = cfg(1, 0, HolderKind::None, AWait::Marker);
    c.irq_first = true;
    assert_green(c);
}

// ---------------------------------------------------------------------------------------------------------------- G2

/// G2 (model): no worker ever enters a blocking wait while the cold relay holds an obligation without a registered wake source
/// (all holders, 1..3 workers, hot 0/1).
#[test]
fn g2_sleep_safety_model_never_blocks_with_an_unsourced_obligation() {
    let mut bad = Vec::new();
    for workers in [1, 2, 3] {
        for holder in HOLDERS {
            if workers >= 3 && matches!(holder, HolderKind::RecordAct | HolderKind::RecordSilent) {
                continue;
            }
            for kind in [AWait::Marker, AWait::Popped] {
                let v = check(cfg(workers, 0, holder, kind));
                if !v.unsafe_sleeps.is_empty() || !v.strands.is_empty() {
                    bad.push(format!("W={workers} {holder:?} {kind:?}: {}", v.line()));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{}:\n{}", adapter::LINEAGE, bad.join("\n"));
}

// ---------------------------------------------------------------------------------------------------------------- G4

/// G4: the blocking loop is never worse than the same loop forced never to block (the efficiency layer neither hides nor
/// creates a bug), and the spin reference itself is green.
#[test]
fn g4_spin_reference_model_blocking_equals_spinning() {
    let mut bad = Vec::new();
    for workers in [1, 2] {
        for hot in [0, 1] {
            for holder in HOLDERS {
                for kind in [AWait::Marker, AWait::Popped] {
                    let c = cfg(workers, hot, holder, kind);
                    let b = check(c);
                    let s = check(Cfg { force_spin: true, ..c });
                    let k = k_bound(&c);
                    if !s.ok(k) {
                        bad.push(format!("SPIN REFERENCE red {c:?}: {}", s.line()));
                    }
                    if !b.ok(k) && s.ok(k) {
                        bad.push(format!("blocking red where spinning is green {c:?}: {}", b.line()));
                    }
                }
            }
        }
    }
    assert!(bad.is_empty(), "{}:\n{}", adapter::LINEAGE, bad.join("\n"));
}

// ---------------------------------------------------------------------------------------------------------------- G5

/// G5 (model): with nothing outstanding every worker reaches a blocking wait and nothing steps afterwards; with only a covered
/// obligation (a marker wait whose interrupt is still to come) no worker spins.
#[test]
fn g5_efficiency_model_no_hidden_spin() {
    let mut bad = Vec::new();
    for workers in [1, 2, 3] {
        for holder in [HolderKind::None, HolderKind::RecordAct] {
            if workers >= 3 && holder == HolderKind::RecordAct {
                continue;
            }
            let v = check(cfg(workers, 0, holder, AWait::Marker));
            if !v.hidden_spins.is_empty() || v.never_settles > 0 || v.phantom_cycles > 0 {
                bad.push(format!("W={workers} {holder:?}: {}", v.line()));
            }
        }
    }
    assert!(bad.is_empty(), "{}:\n{}", adapter::LINEAGE, bad.join("\n"));
}

/// G5 (model): the idle system (no guest work, no relay obligation) ends with every worker parked and no fd ready.
#[test]
fn g5_efficiency_model_idle_workers_block() {
    for workers in [1, 2, 3] {
        let sys = Sys::new(cfg(workers, 0, HolderKind::None, AWait::Marker));
        let mut s = sys.init();
        s.guest_done = true;
        for _ in 0..64 {
            for i in 0..workers {
                if let Some((n, _, _)) = sys.step_worker(&s, i) {
                    s = n;
                }
            }
        }
        assert!(
            s.w.iter().all(|w| w.pc == Pc::Parked),
            "{}: W={workers}: idle workers must block: {s:?}",
            adapter::LINEAGE
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------- G6

/// G6 (model): two workers, two hot self-re-ringing relays and the cold sync client (the baton): the cold relay's wait is
/// bounded in worker steps (K) and no fair starvation cycle exists.
#[test]
fn g6_fairness_model_baton_bound_in_worker_steps() {
    for holder in [HolderKind::None, HolderKind::RecordAct] {
        let c = cfg(2, 2, holder, AWait::Marker);
        let v = check(c);
        println!("baton longest serviceable path {} steps (K = {})", v.longest, k_bound(&c));
        assert!(v.ok(k_bound(&c)), "{}: {}", adapter::LINEAGE, v.line());
    }
}

/// The explorer itself sees a strand: a worker that would park with demand left (the naive loop) -- self-check of the checker
/// through the hidden truth function, independent of the lineage.
#[test]
fn g1_models_checker_self_test_detects_a_lost_wake() {
    let sys = Sys::new(cfg(1, 0, HolderKind::Pt, AWait::Marker));
    let mut s = sys.init();
    s.demand = true;
    assert!(sys.unsourced_truth(&s).is_some());
    s.demand = false;
    s.wait = Some(AWait::Marker);
    s.sat = true;
    assert_eq!(sys.unsourced_truth(&s), Some("a satisfied wait nobody rang"));
    s.evfd = true;
    assert_eq!(sys.unsourced_truth(&s), None);
}
