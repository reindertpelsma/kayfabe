//! 2026-10-20 -- **Adversarial model v3: the Windows USER-ring relay in POINTER mode** (the production default since
//! 2026-10-16/17: lifecycle Scan -> Pointer, lazy observe interval 16 + PUT cache, Pointer markers at observed `GP_PUT`
//! boundaries or none when the VM-wide latch is set). Breaker brief: find interleavings that strand or DELAY a client.
//!
//! Every property is checked twice: LIVENESS at terminal states (no step enabled: every guest entry relayed and executed)
//! and a BOUND: an obligation (a satisfied wait not rung, demand left behind with its lock free, a guest blocked on ring
//! space although the hardware has freed slots) is discharged within `K` worker ticks (a tick = one pass, or one wake from
//! the park). The 50 ms safety net is the park TIMEOUT: a scheduler step that is enabled only while the worker is parked
//! and no fd is ready, so an adversary that keeps an fd ready (a busy neighbour's engine interrupts) keeps it from firing.
//!
//! CODE READ (branch base 82830cd5e), transliterated here and pinned by source text at the bottom:
//!  * worker loop `kf_chan::worker::run_with_relay` (worker.rs 213-323): pass; busy -> `LoopPolicy::after_pass` (Look every
//!    `LOOK_EVERY` busy passes: poll, `peek(Event)` on an engine event, then `peek(Spin)`); idle -> spin while
//!    `waiting()` (`peek(Spin)`), then `try_park`, `epoll_wait(50 ms)`; a doorbell-only wake does NOT peek; an engine event
//!    peeks with `Peek::Event`; ONLY a timeout with nothing ready peeks with `Peek::Backstop` (worker.rs 317-320).
//!  * `ChanPlane::user_ring_serve` (userpump.rs 2724-2804): `record.enter()` (sets `retry`) / `pt.try_lock()` busy ->
//!    `contended_note`, no turn, `retry` stays set, `Pump::wait` UNCHANGED; else `retry.swap(false)`, the turn, the arm.
//!  * `user_ring_arm` (2810-2832): `wait = wait_of(need)`; `Again` rings; `Contended` notes; others clear the run.
//!  * `user_ring_peek_one` (2865-2982): `look_needed`; `retry_ok = may_retry(contended_run, why)`; record busy ->
//!    `retry_ok && ring`; a wait: satisfied from the PRIVATE words read fresh; `!satisfied && pops && Marker` ->
//!    pop write-back ring (2919-2926: ONLY for `Wait::Marker`); `satisfied && (retry_ok || !Contended)` -> clear + ring;
//!    else `demand && retry_ok` -> ring.
//!  * the turn: the REAL `kf_chan::user_turn::Relay` (through `lifecycle_support::Relay`), micro-step by micro-step, so the
//!    guest, the PBDMA and the lock holder act between any two micro-steps.
//!
//! MODELLED: one target channel with the real relay; one worker; an optional HOT neighbour on the same worker (its token is
//! rung after every serve: every pass is busy); optional PEER engine interrupts (any time; a busy neighbour's batch markers);
//! a lock (the `pt` map / USERD admission: busy at `serve` and at the `GP_GET` write-back; released with no announcement)
//! that the adversary takes and releases a bounded number of times; the guest (pushes that need ring space by the
//! `GP_GET` it SEES, `GP_PUT` writes, doorbells, coalesced or not, CPU semaphore releases); the PBDMA (pops with a
//! prefetch window over processing, acquires that block processing, markers that write `REFERENCE` and raise the engine
//! interrupt); the hardware truth for the guest ring (the `GP_GET` the guest would see without the relay).
//! NOT MODELLED: two workers (an event consumed by the other worker during the turn), interrupt-before-REFERENCE
//! visibility lag (v2 `p6_get_visibility_lag_*`), Window / ring growth (`user_lifecycle_model`), the VA thread's walk
//! gate (`Io::walk_busy`; see the report), real time (a tick is not a duration).
#![allow(
    clippy::too_many_lines,
    clippy::type_complexity,
    dead_code,
    missing_docs,
    clippy::collapsible_if,
    clippy::struct_excessive_bools
)]

#[path = "lifecycle_support/mod.rs"]
mod support;

use kf_chan::user_gate::Marks;
use kf_chan::user_lazy::LazyConfig;
use kf_chan::user_lifecycle::{Lifecycle, LifecycleMode, Mode};
use kf_chan::user_loop::{self as ul, Need, Peek, Wait};
use kf_chan::user_scan::Scan;
use std::collections::{HashMap, VecDeque};
use support::*;

/// The bound, in worker ticks (passes and park wakes). A few multiples of `LOOK_EVERY` (4).
const K: u8 = 12;
/// Spin iterations in the model (`user_loop::SPIN` is 64; the rule is the same).
const SPIN_M: u8 = 1;

const V_STALL: u32 = 1; // terminal state with work left (liveness)
const V_WAIT: u32 = 2; // a satisfied wait not rung within K ticks
const V_DEMAND: u32 = 4; // demand (a doorbell, a contended turn) with its lock free not re-rung within K ticks
const V_SPACE: u32 = 8; // the guest is blocked on ring space the hardware has freed for more than K ticks
const V_I8: u32 = 16; // the relay read beyond GP_PUT / a refusal (safety, from the support layer)
const V_DEAD: u32 = 32; // the relay refused something

fn names(v: u32) -> Vec<&'static str> {
    let mut o = Vec::new();
    for (b, n) in [
        (
            V_STALL,
            "STALL: no step is enabled and guest work is left (liveness)",
        ),
        (
            V_WAIT,
            "WAIT: a satisfied wait of the relay was not rung within K worker ticks",
        ),
        (
            V_DEMAND,
            "DEMAND: demand left behind (doorbell / contended turn) with the lock FREE was not re-rung within K ticks",
        ),
        (
            V_SPACE,
            "SPACE: the guest was blocked on ring space that the hardware had freed for more than K ticks",
        ),
        (V_I8, "I8: a read beyond GP_PUT"),
        (V_DEAD, "DEAD: the relay refused (channel reset)"),
    ] {
        if v & b != 0 {
            o.push(n);
        }
    }
    o
}

// ------------------------------------------------------------------------------------------------
// configuration

#[derive(Clone, Debug)]
enum Op {
    /// Write these entries into the guest ring (needs space by the `GP_GET` the guest SEES); `GP_PUT` is not moved.
    Push(Vec<Ge>),
    /// Write `GP_PUT` = everything pushed so far.
    SetPut,
    /// The doorbell (`ChanPlane::user_ring_wake`).
    Ring,
    /// The guest CPU releases semaphore `s` (an acquire on it blocks the PBDMA until then).
    Release(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LockKind {
    /// The `pt` map / USERD admission: busy at `serve` (`pt.try_lock`) and at the `GP_GET` write-back; the release rings
    /// nothing (userpump.rs 2736-2740, user_turn.rs 562-569).
    Pt,
    /// The record mutex held by an act thread: `record.enter()` fails, the peek's `try_lock` fails; the act's completion
    /// rings (`user_ring_wake`), like the birth / 5080 / ctx acts.
    Record,
}

#[derive(Clone, Debug)]
struct Cfg {
    guest_entries: u32,
    priv_entries: u32,
    mode: LifecycleMode,
    lazy: LazyConfig,
    /// PBDMA GP fetch window over processing (`popped <= processed + prefetch`).
    prefetch: usize,
    script: Vec<Op>,
    /// A hot neighbour shares the worker: every pass serves it (busy).
    hot: bool,
    /// Peer engine interrupts may arrive at any time (a busy neighbour's markers).
    peer_events: bool,
    /// The 50 ms park timeout exists (the safety net).
    backstop: bool,
    lock: LockKind,
    /// How many times the adversary may take the lock.
    lock_holds: u8,
    /// The lock is held when the exploration starts.
    lock_at_start: bool,
    /// Exploration cap (a test that hits it fails loudly).
    max_states: usize,
    /// The retired behaviour: the pt holder does not hand off (no registration, no ring).
    no_handoff: bool,
}

impl Cfg {
    fn new(script: Vec<Op>) -> Self {
        Self {
            guest_entries: 8,
            priv_entries: 8,
            mode: LifecycleMode::Default,
            lazy: LazyConfig::ON,
            prefetch: 1,
            script,
            hot: false,
            peer_events: false,
            backstop: false,
            lock: LockKind::Pt,
            lock_holds: 0,
            lock_at_start: false,
            max_states: 3_000_000,
            no_handoff: false,
        }
    }
}

// ------------------------------------------------------------------------------------------------
// state

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Item {
    Guest { id: u8 },
    Barrier,
    Marker(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pc {
    /// About to run a pass.
    Top,
    /// Inside the target relay's turn (micro-steps); `served` of the pass so far.
    InTurn {
        served: u8,
    },
    /// The pass is over: the after-pass decision.
    After {
        served: u8,
    },
    Parked,
}

/// The guest and the GPU (the support layer's `Env`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct G {
    e: u32,
    p: u32,
    ring: Vec<u64>,
    put: u32,
    wpos: u32,
    pushed_total: u64,
    /// The guest's view of `GP_GET` (what the relay wrote back).
    seen_get: u32,
    items: Vec<Item>,
    popped: usize,
    processed: usize,
    reference: u32,
    sems: u8,
    /// The lock (pt / admission) is held by someone else.
    pt_held: bool,
    flags: u32,
    ids: Vec<Option<Ge>>,
}

impl G {
    fn guest_consumed(&self) -> u64 {
        self.items[..self.popped]
            .iter()
            .filter(|i| matches!(i, Item::Guest { .. }))
            .count() as u64
    }
    /// The guest ring's free slots by a `GP_GET` value.
    fn free(&self, get: u32) -> u32 {
        (get + self.e - self.wpos - 1) % self.e
    }
    /// The `GP_GET` the hardware would show the guest without the relay (entries the PBDMA has popped).
    fn true_get(&self) -> u32 {
        (self.guest_consumed() % u64::from(self.e)) as u32
    }
}

impl Env for G {
    fn src_entries(&self) -> u32 {
        self.e
    }
    fn gp_put(&mut self) -> u32 {
        self.put
    }
    fn gp_dword(&mut self, slot: u32, hi: bool) -> u32 {
        let v = self.ring[slot as usize];
        if hi { (v >> 32) as u32 } else { v as u32 }
    }
    fn registry_epoch(&mut self) -> u64 {
        0
    }
    fn registry_live(&mut self) -> u32 {
        0
    }
    fn hw_get(&mut self) -> u32 {
        (self.popped % self.p as usize) as u32
    }
    fn hw_reference(&mut self) -> u32 {
        self.reference
    }
    fn read_pb(&mut self, raw: u64) -> Vec<u32> {
        entry_id(raw)
            .and_then(|id| self.ids.get(id as usize).copied().flatten())
            .map_or_else(|| vec![POISON], |g| g.words())
    }
    fn publish(&mut self, p: Pub) {
        self.items.push(match p {
            Pub::Guest { stored, .. } | Pub::Staged { tested: stored } => Item::Guest {
                id: entry_id(stored).unwrap_or(0xff),
            },
            Pub::Barrier => Item::Barrier,
            Pub::Marker(n) => Item::Marker(n),
        });
    }
    fn write_guest_get(&mut self, v: u32) {
        self.seen_get = v;
    }
    fn trigger_migrates(&mut self, _raw: u64) -> bool {
        false
    }
    fn executed(&mut self, _migrate: bool) {}
    fn event(&mut self, e: Ev) {
        match e {
            Ev::BeyondPut | Ev::PutBound => self.flags |= V_I8,
            Ev::EntryRefused | Ev::ScanRefused => self.flags |= V_DEAD,
            _ => {}
        }
    }
    fn write_guest_get_busy(&mut self) -> bool {
        self.pt_held
    }
    fn cached_put_ok(&mut self, put: u32, cursor: u32) -> bool {
        let n = self.e;
        let ahead = put.wrapping_sub(cursor) & (n - 1);
        let real = self.put.wrapping_sub(cursor) & (n - 1);
        ahead != 0 && ahead <= real
    }
}

#[derive(Clone, Debug)]
struct S {
    relay: Relay,
    g: G,
    pc_guest: usize,
    script: std::rc::Rc<Vec<Op>>,
    // the record (`userrelay::Cell`) and `Pump::wait`
    retry: bool,
    waiting: bool,
    /// Registered with the pt holder after losing the lock (`kf_chan::handoff`).
    reg: bool,
    /// An interrupt came and the words did not show the progress yet: the worker may not block.
    lagging: bool,
    wait: Option<Wait>,
    rec_held: bool,
    holds_left: u8,
    // tokens and fds
    t_rung: bool,
    efd: bool,
    engine_event: bool,
    // the worker
    pc: Pc,
    // bounded-wait counters (ticks since the obligation began; capped)
    c_wait: u8,
    c_demand: u8,
    c_space: u8,
    flags: u32,
    backstops: u8,
}

impl S {
    fn key(&self) -> String {
        format!(
            "{}|{:?}|{}|{}{}{}{}|{:?}|{}{}|{}{}{}|{:?}|{},{},{}|{:x}|{}",
            self.relay.key(),
            (
                &self.g.ring,
                self.g.put,
                self.g.wpos,
                self.g.seen_get,
                &self.g.items,
                self.g.popped,
                self.g.processed,
                self.g.reference,
                self.g.sems,
                self.g.pt_held,
                self.g.flags
            ),
            self.pc_guest,
            u8::from(self.retry),
            u8::from(self.waiting),
            u32::from(self.reg) * 2 + u32::from(self.lagging),
            u8::from(self.rec_held),
            self.wait,
            self.holds_left,
            u8::from(self.t_rung),
            u8::from(self.efd),
            u8::from(self.engine_event),
            u8::from(false),
            self.pc,
            self.c_wait,
            self.c_demand,
            self.c_space,
            self.flags,
            self.backstops.min(1),
        )
    }

    fn lock_free(&self) -> bool {
        !self.g.pt_held && !self.rec_held
    }

    /// `ChanPlane::user_ring_waiting`: a recorded private-word wait.
    fn waiting_any(&self) -> bool {
        self.waiting && !self.relay.dead
    }

    /// `ChanPlane::user_ring_unannounced`: an obligation no wake source announces (a carried wait, a spent hint).
    fn unannounced(&self) -> bool {
        !self.relay.dead
            && (matches!(self.wait, Some(Wait::Popped { .. }))
                || self.lagging
                // the guest's ring looked full at the last `GP_PUT` read and pops are still owed to it (they carry no event)
                || (self.relay.core.put_full
                    && self.relay.core.fetch.fetched() < self.relay.core.fetch.issued()))
    }

    /// `ring(self)` in the peek: `plane.ring_internal` + `wake.signal()`.
    fn ring(&mut self) {
        self.t_rung = true;
        self.efd = true;
    }

    /// `ChanPlane::user_ring_peek_one`, transliterated. Returns whether it rang. Contention is the lock holder's business: no look acts on
    /// it (`look_needed` ignores demand).
    fn peek(&mut self, why: Peek) -> bool {
        if self.relay.dead {
            return false;
        }
        let backstop = why == Peek::Backstop;
        if !ul::look_needed(why, false, self.waiting) {
            return false;
        }
        if self.rec_held || matches!(self.pc, Pc::InTurn { .. }) {
            // record held elsewhere: its completion rings (`user_ring_wake`) / the arm picks the demand up
            return false;
        }
        if let Some(w) = self.wait {
            let get = self.g.hw_get();
            let reference = self.g.hw_reference();
            let mut pops = false;
            let satisfied = match self.relay.core.fetch.peek(get) {
                Ok(popped) => {
                    pops = popped > self.relay.core.fetch.fetched();
                    w.satisfied(self.relay.core.marks.peek(reference), popped)
                }
                Err(_) => true,
            };
            if !satisfied && pops && matches!(w, Wait::Marker { .. } | Wait::Popped { .. }) {
                self.ring();
                return true;
            }
            if satisfied && !matches!(w, Wait::Contended) {
                if backstop {
                    self.backstops = self.backstops.saturating_add(1);
                }
                self.wait = None;
                self.waiting = false;
                self.lagging = false;
                self.ring();
                return true;
            }
            if why == Peek::Event {
                self.lagging = true;
            }
        }
        false
    }

    /// `ChanPlane::user_ring_arm`.
    fn arm(&mut self, no_handoff: bool) {
        // `Pump::turn`: a turn that made progress ends `Again` whatever else it said
        if self.relay.core.again && !self.relay.core.dead {
            self.relay.core.need = Need::Again;
        }
        let need = self.relay.core.need;
        self.wait = ul::wait_of(need);
        self.waiting = matches!(self.wait, Some(Wait::Marker { .. } | Wait::Popped { .. }));
        if !self.waiting {
            self.lagging = false;
        }
        match need {
            Need::Again => self.ring(),
            Need::Contended => {
                // register with the holder BEFORE giving up; a lock already free means a holder released since the attempt: serve again
                if self.g.pt_held {
                    self.reg = !no_handoff;
                } else {
                    self.ring();
                }
                self.wait = None;
            }
            Need::Idle | Need::Marker { .. } | Need::Popped { .. } => {}
        }
        // demand that arrived while the turn ran (an act's `user_ring_wake` found the record held and could only request): serve again
        if self.retry {
            self.ring();
        }
    }

    /// Is the target's recorded wait satisfied by the hardware words right now (the obligation of `V_WAIT`)?
    fn wait_satisfied_now(&self) -> bool {
        let Some(w) = self.wait else { return false };
        if matches!(w, Wait::Contended) {
            return false; // counted by the demand obligation
        }
        let get = (self.g.popped % self.g.p as usize) as u32;
        match self.relay.core.fetch.peek(get) {
            Ok(popped) => w.satisfied(self.relay.core.marks.peek(self.g.reference), popped),
            Err(_) => true,
        }
    }

    fn guest_blocked_on_space(&self) -> Option<bool> {
        match self.script.get(self.pc_guest) {
            Some(Op::Push(v)) => {
                let n = v.len() as u32;
                if self.g.free(self.g.seen_get) < n {
                    Some(self.g.free(self.g.true_get()) >= n)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------------------------------
// the transition relation

fn in_turn(s: &S) -> bool {
    matches!(s.pc, Pc::InTurn { .. })
}

/// One worker tick: every unmet obligation ages by one (and is checked against `K`).
fn tick(s: &mut S) {
    let rung = s.t_rung || in_turn(s);
    let wait = s.wait_satisfied_now() && !rung;
    let demand = s.retry && s.lock_free() && !rung && !s.relay.dead;
    let space = s.guest_blocked_on_space() == Some(true);
    s.c_wait = if wait { s.c_wait.saturating_add(1) } else { 0 };
    s.c_demand = if demand {
        s.c_demand.saturating_add(1)
    } else {
        0
    };
    s.c_space = if space {
        s.c_space.saturating_add(1)
    } else {
        0
    };
    if s.c_wait > K {
        s.flags |= V_WAIT;
    }
    if s.c_demand > K {
        s.flags |= V_DEMAND;
    }
    if s.c_space > K {
        s.flags |= V_SPACE;
    }
    s.c_wait = s.c_wait.min(K + 1);
    s.c_demand = s.c_demand.min(K + 1);
    s.c_space = s.c_space.min(K + 1);
}

/// The worker's park attempt (`try_park(seen)`: refused when a ring happened since the pass).
fn park(s: &mut S) {
    s.pc = if s.t_rung { Pc::Top } else { Pc::Parked };
}

/// `dispatch` of the ready fds: the doorbell eventfd is drained (no peek); an engine event peeks with `Peek::Event`.
fn dispatch(s: &mut S) {
    s.efd = false;
    if s.engine_event {
        s.engine_event = false;
        let _ = s.peek(Peek::Event);
    }
}

fn guest_step(cfg: &Cfg, s: &S, out: &mut Vec<(String, S)>, limit_prelude: Option<usize>) {
    let Some(op) = s.script.get(s.pc_guest) else {
        return;
    };
    if limit_prelude.is_some_and(|p| s.pc_guest >= p) {
        return;
    }
    let mut n = s.clone();
    n.pc_guest += 1;
    let label = match op {
        Op::Push(v) => {
            if s.g.free(s.g.seen_get) < v.len() as u32 {
                return; // blocked on ring space (by the GP_GET the guest sees)
            }
            for ge in v {
                let slot = n.g.wpos as usize;
                n.g.ring[slot] = ge.raw();
                n.g.ids[ge.id as usize] = Some(*ge);
                n.g.wpos = (n.g.wpos + 1) % n.g.e;
                n.g.pushed_total += 1;
            }
            format!(
                "guest pushes {:?} (sees GP_GET={})",
                v.iter().map(|g| g.id).collect::<Vec<_>>(),
                s.g.seen_get
            )
        }
        Op::SetPut => {
            n.g.put = n.g.wpos;
            format!("guest writes GP_PUT={}", n.g.put)
        }
        Op::Ring => {
            // `user_ring_wake`: `request()` then `wake_if` (needs the record try-lock) then `wake.signal()`
            n.retry = true;
            if !n.rec_held && !in_turn(&n) {
                n.t_rung = true;
            }
            n.efd = true;
            "guest rings the doorbell".into()
        }
        Op::Release(sem) => {
            n.g.sems |= 1 << sem;
            format!("guest CPU releases semaphore {sem}")
        }
    };
    let _ = cfg;
    out.push((label, n));
}

fn gpu_steps(cfg: &Cfg, s: &S, out: &mut Vec<(String, S)>) {
    if s.g.popped < s.g.items.len() && s.g.popped < s.g.processed + cfg.prefetch {
        let mut n = s.clone();
        n.g.popped += 1;
        out.push((
            format!(
                "PBDMA pops private entry #{} ({:?})",
                s.g.popped, s.g.items[s.g.popped]
            ),
            n,
        ));
    }
    if s.g.processed < s.g.popped {
        let item = s.g.items[s.g.processed];
        let blocked = match item {
            Item::Guest { id } => {
                s.g.ids
                    .get(id as usize)
                    .copied()
                    .flatten()
                    .and_then(|g| g.acq)
                    .is_some_and(|sem| s.g.sems & (1 << sem) == 0)
            }
            _ => false,
        };
        if !blocked {
            let mut n = s.clone();
            n.g.processed += 1;
            if let Item::Marker(c) = item {
                n.g.reference = Marks::value(c);
                n.engine_event = true;
            }
            out.push((format!("GPU processes #{} ({item:?})", s.g.processed), n));
        }
    }
}

fn adversary_steps(cfg: &Cfg, s: &S, out: &mut Vec<(String, S)>) {
    if s.holds_left > 0 && s.lock_free() && !(cfg.lock == LockKind::Record && in_turn(s)) {
        let mut n = s.clone();
        n.holds_left -= 1;
        match cfg.lock {
            LockKind::Pt => n.g.pt_held = true,
            LockKind::Record => n.rec_held = true,
        }
        out.push((format!("another thread takes the {:?} lock", cfg.lock), n));
    }
    if !s.lock_free() {
        let mut n = s.clone();
        match cfg.lock {
            LockKind::Pt => {
                n.g.pt_held = false;
                // ★ the holder's hand-off: after releasing, ring the relay that registered
                if n.reg && !cfg.no_handoff {
                    n.reg = false;
                    if !in_turn(&n) {
                        n.t_rung = true;
                    }
                    n.efd = true;
                }
            }
            LockKind::Record => {
                n.rec_held = false;
                // the act's completion: `user_ring_wake`
                n.retry = true;
                if !in_turn(&n) {
                    n.t_rung = true;
                }
                n.efd = true;
            }
        }
        out.push((
            format!(
                "the {:?} lock is released (pt: the holder rings the registered relay)",
                cfg.lock
            ),
            n,
        ));
    }
    if cfg.peer_events && !s.engine_event && !in_turn(s) {
        let mut n = s.clone();
        n.engine_event = true;
        out.push(("a PEER channel's engine interrupt becomes ready".into(), n));
    }
}

fn worker_steps(cfg: &Cfg, s: &S, out: &mut Vec<(String, S)>, timeouts: bool) {
    let mut n = s.clone();
    let label: String = match s.pc {
        Pc::Top => {
            tick(&mut n);
            if n.t_rung && !n.relay.dead {
                n.t_rung = false;
                n.retry = true; // `record.enter()` -> `request()`
                if n.rec_held {
                    n.pc = Pc::After {
                        served: 1 + u8::from(cfg.hot),
                    };
                    "pass: serve -> record busy (the act's completion rings)".into()
                } else if n.g.pt_held {
                    // register with the holder BEFORE giving up (the lock IS held at this step)
                    n.reg = !cfg.no_handoff;
                    n.pc = Pc::After {
                        served: 1 + u8::from(cfg.hot),
                    };
                    "pass: serve -> pt busy (registered with the holder)".into()
                } else {
                    n.retry = false;
                    n.relay.claim();
                    n.pc = Pc::InTurn { served: 1 };
                    "pass: serve -> turn begins".into()
                }
            } else {
                n.pc = Pc::After {
                    served: u8::from(cfg.hot),
                };
                if cfg.hot {
                    "pass: serves only the HOT neighbour".into()
                } else {
                    "pass: nothing to serve".into()
                }
            }
        }
        Pc::InTurn { served } => {
            let before = n.relay.core.phase;
            n.relay.step(&mut n.g);
            if n.relay.core.dead {
                n.flags |= V_DEAD;
            }
            if n.relay.idle() {
                n.arm(cfg.no_handoff);
                n.pc = Pc::After {
                    served: served + u8::from(cfg.hot),
                };
                format!(
                    "turn ends ({before:?}): need={:?} cursor={} wrote GP_GET={}",
                    n.relay.core.need, n.relay.core.cursor, n.g.seen_get
                )
            } else {
                format!("turn step {before:?}")
            }
        }
        Pc::After { served } => {
            // ★ the ONE evaluation per iteration: drain the ready fds (an engine event peeks with Peek::Event), evaluate every recorded wait
            dispatch(&mut n);
            let _ = n.peek(Peek::Spin);
            if served > 0 {
                n.pc = Pc::Top;
                "after a busy pass: evaluate, next pass".into()
            } else if ul::idle(n.unannounced()) == ul::Idle::Spin {
                n.pc = Pc::Top;
                "idle with an obligation no wake source announces: evaluate again (epoll timeout 0)"
                    .into()
            } else {
                park(&mut n);
                "idle, every obligation announced: park".into()
            }
        }
        Pc::Parked => {
            if n.efd || n.engine_event {
                tick(&mut n);
                let ev = n.engine_event;
                dispatch(&mut n);
                n.pc = Pc::Top;
                if ev {
                    "park wakes on an ENGINE event: peek(Event)".into()
                } else {
                    "park wakes on the doorbell eventfd (no peek)".into()
                }
            } else if timeouts && cfg.backstop {
                tick(&mut n);
                let _ = n.peek(Peek::Backstop);
                n.pc = Pc::Top;
                "park TIMES OUT (50 ms): peek(Backstop)".into()
            } else {
                return;
            }
        }
    };
    out.push((label, n));
}

fn successors(cfg: &Cfg, s: &S) -> Vec<(String, S)> {
    let mut out = Vec::new();
    guest_step(cfg, s, &mut out, None);
    gpu_steps(cfg, s, &mut out);
    adversary_steps(cfg, s, &mut out);
    worker_steps(cfg, s, &mut out, true);
    out
}

fn work_left(s: &S) -> bool {
    s.pc_guest < s.script.len() || s.relay.core.cursor != s.g.put || s.g.processed < s.g.items.len()
}

// ------------------------------------------------------------------------------------------------
// initial state, prelude, exploration

const PRELUDE: usize = 4;

fn prelude() -> Vec<Op> {
    vec![
        Op::Push(vec![ge(0, Kind::SetObj), ge(9, Kind::Plain)]),
        Op::SetPut,
        Op::Ring,
    ]
}

fn plain(id: u8) -> Ge {
    ge(id, Kind::Plain)
}

fn script(rest: Vec<Op>) -> Vec<Op> {
    let mut v = prelude();
    v.push(Op::Release(7)); // no-op marker of the prelude's end (semaphore 7 is never acquired)
    v.extend(rest);
    v
}

fn initial(cfg: &Cfg) -> S {
    let life = Lifecycle::new().with_mode(cfg.mode);
    let scan = Scan::for_subdevice(1).expect("scan");
    let relay = Relay::new(cfg.priv_entries, life, scan, Glue::default()).with_lazy(cfg.lazy);
    S {
        relay,
        g: G {
            e: cfg.guest_entries,
            p: cfg.priv_entries,
            ring: vec![0; cfg.guest_entries as usize],
            put: 0,
            wpos: 0,
            pushed_total: 0,
            seen_get: 0,
            items: Vec::new(),
            popped: 0,
            processed: 0,
            reference: 0,
            sems: 0,
            pt_held: false,
            flags: 0,
            ids: vec![None; 64],
        },
        pc_guest: 0,
        script: std::rc::Rc::new(cfg.script.clone()),
        retry: false,
        waiting: false,
        reg: false,
        lagging: false,
        wait: None,
        rec_held: false,
        holds_left: cfg.lock_holds,
        t_rung: false,
        efd: false,
        engine_event: false,
        pc: Pc::Parked,
        c_wait: 0,
        c_demand: 0,
        c_space: 0,
        flags: 0,
        backstops: 0,
    }
}

/// Run the prelude (Scan -> Pointer) deterministically: GPU first, then the worker, then the guest; no adversary, no
/// timeout. Ends parked, quiescent, in Pointer mode.
fn settle(cfg: &Cfg) -> S {
    let quiet = Cfg {
        hot: false,
        ..cfg.clone()
    };
    let cfg = &quiet;
    let mut s = initial(cfg);
    for _ in 0..10_000 {
        let mut out = Vec::new();
        gpu_steps(cfg, &s, &mut out);
        if out.is_empty() {
            worker_steps(cfg, &s, &mut out, false);
        }
        if out.is_empty() {
            guest_step(cfg, &s, &mut out, Some(PRELUDE));
        }
        let Some((_, n)) = out.into_iter().next() else {
            break;
        };
        s = n;
    }
    assert_eq!(s.pc_guest, PRELUDE, "the prelude ran");
    assert_eq!(
        s.relay.life.mode(),
        Mode::Pointer,
        "the prelude ends in Pointer mode"
    );
    assert!(
        !work_left(&S {
            pc_guest: s.script.len(),
            ..s.clone()
        }),
        "the prelude is quiescent"
    );
    assert_eq!(s.pc, Pc::Parked);
    s.flags = 0;
    s.g.flags = 0;
    if cfg.lock_at_start {
        match cfg.lock {
            LockKind::Pt => s.g.pt_held = true,
            LockKind::Record => s.rec_held = true,
        }
    }
    s
}

struct Found {
    flags: u32,
    trace: Vec<String>,
    states: usize,
}

fn explore(cfg: &Cfg) -> Result<usize, Found> {
    let s0 = settle(cfg);
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut nodes: Vec<(usize, String)> = vec![(
        usize::MAX,
        "(after the prelude: Pointer mode, parked)".into(),
    )];
    let mut q: VecDeque<(S, usize)> = VecDeque::new();
    seen.insert(s0.key(), 0);
    q.push_back((s0, 0));
    let path = |nodes: &Vec<(usize, String)>, mut i: usize| {
        let mut t = Vec::new();
        while i != usize::MAX {
            t.push(nodes[i].1.clone());
            i = nodes[i].0;
        }
        t.reverse();
        t
    };
    while let Some((s, i)) = q.pop_front() {
        let flags = s.flags | s.g.flags;
        if flags != 0 {
            return Err(Found {
                flags,
                trace: path(&nodes, i),
                states: seen.len(),
            });
        }
        let next = successors(cfg, &s);
        if next.is_empty() && work_left(&s) {
            return Err(Found {
                flags: V_STALL,
                trace: path(&nodes, i),
                states: seen.len(),
            });
        }
        for (label, n) in next {
            let k = n.key();
            if seen.contains_key(&k) {
                continue;
            }
            nodes.push((i, label));
            let j = nodes.len() - 1;
            seen.insert(k, j);
            assert!(
                seen.len() <= cfg.max_states,
                "state space larger than {}",
                cfg.max_states
            );
            q.push_back((n, j));
        }
    }
    Ok(seen.len())
}

fn holds(name: &str, cfg: &Cfg) -> usize {
    match explore(cfg) {
        Ok(n) => {
            eprintln!("{name}: holds over {n} states");
            n
        }
        Err(f) => panic!(
            "{name}: VIOLATION {:?} after {} states; schedule:\n  {}",
            names(f.flags),
            f.states,
            f.trace.join("\n  ")
        ),
    }
}

/// Explore; the configuration must reach a violation containing `want`. Prints the minimal (BFS-shortest) schedule.
fn breaks(name: &str, cfg: &Cfg, want: u32) -> Found {
    match explore(cfg) {
        Ok(n) => panic!(
            "{name}: expected a violation {:?}, but every one of {n} states holds",
            names(want)
        ),
        Err(f) => {
            eprintln!(
                "{name}: {:?} after {} states; schedule:\n  {}",
                names(f.flags),
                f.states,
                f.trace.join("\n  ")
            );
            assert!(
                f.flags & want != 0,
                "{name}: expected {:?}, found {:?}",
                names(want),
                names(f.flags)
            );
            f
        }
    }
}

// ------------------------------------------------------------------------------------------------
// calibration and the attacks that HOLD (what was tried)

#[test]
fn calibration_the_prelude_reaches_pointer_mode_and_a_batch_is_live_and_bounded() {
    let cfg = Cfg::new(script(vec![
        Op::Push(vec![plain(1), plain(2)]),
        Op::SetPut,
        Op::Ring,
    ]));
    holds("calibration", &cfg);
}

/// Coalesced doorbells with the PUT cache (rule A): two `GP_PUT` writes, one ring, then a third write and ring. The cache
/// is filled at the first `GP_PUT` the relay reads; the cursor reaching it forces a fresh read.
#[test]
fn coalesced_doorbells_with_a_stale_put_cache_are_live_and_bounded() {
    let cfg = Cfg::new(script(vec![
        Op::Push(vec![plain(1)]),
        Op::SetPut,
        Op::Push(vec![plain(2)]),
        Op::SetPut,
        Op::Ring,
        Op::Push(vec![plain(3)]),
        Op::SetPut,
        Op::Ring,
    ]));
    holds("coalesced", &cfg);
}

/// Markers latched off (`NoPointerMarkers` = the VM-wide latch): every batch end is a CARRIED `Popped` wait. With a hot
/// neighbour on the worker, the busy Look (fix 1, LOOK_EVERY) finds the satisfied wait within the bound.
#[test]
fn a_carried_popped_wait_satisfied_while_the_worker_is_busy_is_found_within_k_ticks() {
    let mut cfg = Cfg::new(script(vec![
        Op::Push(vec![plain(1), plain(2)]),
        Op::SetPut,
        Op::Ring,
    ]));
    cfg.mode = LifecycleMode::NoPointerMarkers;
    cfg.hot = true;
    holds("popped-busy", &cfg);
}

/// The guest fills a small ring (4 entries, lazy interval min(16, 4/2) = 2) in Pointer mode with markers ON: the D6
/// guard and the marker wait's pop write-back keep the guest's `GP_GET` moving.
#[test]
fn a_ring_full_guest_with_markers_on_is_live_and_bounded() {
    let mut cfg = Cfg::new(script(vec![
        Op::Push(vec![plain(1), plain(2), plain(3)]),
        Op::SetPut,
        Op::Ring,
        Op::Push(vec![plain(4), plain(5)]),
        Op::SetPut,
        Op::Ring,
    ]));
    cfg.guest_entries = 4;
    holds("ring-full-markers", &cfg);
}

/// A pt-lock window (one hold) across a doorbell, no convoy before it: the spin re-rings within the retry budget.
#[test]
fn one_pt_window_across_a_doorbell_is_re_rung_within_k_ticks() {
    let mut cfg = Cfg::new(script(vec![Op::Push(vec![plain(1)]), Op::SetPut, Op::Ring]));
    cfg.lock_holds = 1;
    holds("pt-window", &cfg);
}

// ------------------------------------------------------------------------------------------------
// BREAK 1: a contended relay past CONTENTION_RETRIES is stranded while ANY engine interrupt keeps the park from timing out

/// One doorbell; the `pt` lock (or USERD admission) is held across it by another thread. The relay registers with the holder before giving
/// up and the holder rings it after releasing (owner design, `kf_chan::handoff`).
fn convoy() -> Cfg {
    let mut cfg = Cfg::new(script(vec![Op::Push(vec![plain(1)]), Op::SetPut, Op::Ring]));
    cfg.lock = LockKind::Pt;
    cfg.lock_at_start = true;
    cfg
}

/// Control: the same convoy on an IDLE worker with nothing else happening is rescued by the 50 ms safety net (counted
/// `backstop_found(contended)`), within the bound.
#[test]
fn break1_control_a_convoy_past_the_retry_budget_on_a_quiet_worker_is_rescued_by_the_safety_net() {
    holds("break1-control", &convoy());
}

/// ★ EXPECTED RED (break 1, NOT the busy-Look case: the worker is IDLE). After `CONTENTION_RETRIES` consecutive
/// contended serves, `may_retry` lets only `Peek::Backstop` re-ring the relay (user_loop.rs 123-125, userpump.rs 2882,
/// 2975-2980); the backstop runs only when the 50 ms park TIMES OUT with no fd ready (worker.rs 317-320). A peer channel
/// whose batch markers raise an engine interrupt more often than every 50 ms makes every park end on an event:
/// `peek(Event)` refuses (`may_retry` false), the spin refuses, and the relay -- whose doorbell is covered and whose lock
/// is FREE -- is not served for as long as the peer keeps running. Hardware signature: the stall ends at the first quiet
/// 50 ms of the peer, counted as `backstop_found(contended)`.
#[test]
fn break1_expected_red_a_convoy_past_the_retry_budget_is_stranded_by_a_peers_interrupt_stream() {
    let mut cfg = convoy();
    cfg.peer_events = true;
    holds("break1", &cfg);
}

/// ★ EXPECTED RED (break 1b): the convoy is built from 0 inside the model -- one `pt` hold, the spin and the peer
/// interrupts re-ring the contended relay until the budget is spent, then the same stranding. Shows the budget is spent
/// by ONE lock hold that outlasts 64 looks (microseconds on a spinning worker).
#[test]
fn break1b_expected_red_one_long_pt_hold_spends_the_budget_and_strands_the_relay() {
    let mut cfg = convoy();
    cfg.peer_events = true;
    holds("break1b", &cfg);
}

/// The known case (the fairness review's P13, a fix is being implemented elsewhere): a HOT neighbour keeps the worker
/// busy, so it never parks and the backstop never runs; the busy Look peeks with `Peek::Spin`. Kept here as the
/// cross-check of this model against `relay_fairness.rs` (EXPECTED RED until that fix lands).
#[test]
fn known_p13_expected_red_a_convoy_past_the_retry_budget_beside_a_hot_neighbour() {
    let mut cfg = convoy();
    cfg.hot = true;
    holds("known-p13", &cfg);
}

// ------------------------------------------------------------------------------------------------
// BREAK 2: a CARRIED (`Popped`) wait never writes the guest's GP_GET back for partial pops

/// The guest ring has 4 slots. After the prelude the guest pushes three entries, the second ending with a semaphore
/// ACQUIRE that the guest's own CPU releases only after its NEXT submission found ring space (a single submitting thread:
/// "submit A (waits on F), submit B, signal F"). On hardware without the relay the PBDMA pops the first two entries
/// (fetch runs ahead of processing), the guest sees space, submits B, signals F.
fn cyclic_ring_full(mode: LifecycleMode) -> Cfg {
    let mut cfg = Cfg::new(script(vec![
        Op::Push(vec![plain(1), plain(2).acq(0), plain(3)]),
        Op::SetPut,
        Op::Ring,
        Op::Push(vec![plain(4)]),
        Op::SetPut,
        Op::Ring,
        Op::Release(0),
    ]));
    cfg.guest_entries = 4;
    cfg.mode = mode;
    cfg
}

/// Control: with Pointer markers ON the batch-end marker waits behind the acquire, and the peek's pop write-back
/// (userpump.rs 2919-2926, `Wait::Marker` only) rings the relay for the pops: the guest gets its space.
#[test]
fn break2_control_with_pointer_markers_on_the_marker_waits_pop_write_back_frees_the_guest() {
    holds("break2-control", &cyclic_ring_full(LifecycleMode::Default));
}

/// ★ EXPECTED RED (break 2). With Pointer markers OFF (the VM-wide latch `USER_RING_POINTER_MARKERS_LATCHED_OFF`, or
/// `KF3_LIFECYCLE_MODE=no_pointer_markers`) the batch ends in `Need::Popped { target: issued }` (user_turn.rs 897-910). The
/// PBDMA pops the first entries and blocks on the acquire before popping the last; `Popped` is not satisfied, and the
/// peek's pop write-back exists only for `Wait::Marker` (userpump.rs 2919). No turn runs, so the guest's `GP_GET` stays
/// where the last observation left it: the guest sees a FULL ring that the hardware has already freed, never submits B,
/// never signals F. Neither the spin, an event nor the 50 ms backstop helps (the wait is not satisfied). Same shape in
/// Scan / Window when a packet is open at the batch end (`marker_possible` false) and for `end_batch`'s second branch.
#[test]
fn break2_expected_red_a_carried_wait_never_writes_back_partial_pops_and_the_guest_deadlocks_on_ring_space()
 {
    holds("break2", &cyclic_ring_full(LifecycleMode::NoPointerMarkers));
}

/// ★ EXPECTED RED (break 2, the non-cyclic DELAY form): no semaphore cycle, the guest only needs one slot; markers off. The
/// PBDMA pops the batch slowly (prefetch 1, processing gated by nothing): the guest's space waits for the WHOLE batch to
/// be popped and then for a look (spin / event / backstop), not for the first freed slot. Bounded by the GPU, but the
/// bound is the batch, not one entry.
#[test]
fn break2b_expected_red_a_carried_wait_delays_the_guests_ring_space_until_the_whole_batch_is_popped()
 {
    let mut cfg = Cfg::new(script(vec![
        Op::Push(vec![plain(1), plain(2), plain(3)]),
        Op::SetPut,
        Op::Ring,
        Op::Push(vec![plain(4)]),
        Op::SetPut,
        Op::Ring,
    ]));
    cfg.guest_entries = 4;
    cfg.mode = LifecycleMode::NoPointerMarkers;
    cfg.peer_events = true;
    holds("break2b", &cfg);
}

/// Control of break 1: an act thread holding the RECORD lock rings the relay at its completion (`user_ring_wake`), so the
/// same convoy and the same peer interrupt stream do not strand it. Only an UNANNOUNCED release (pt / USERD admission /
/// the `GP_GET` write-back's admission) does.
#[test]
fn break1_control_a_record_lock_convoy_is_rung_by_the_acts_completion_even_with_peer_interrupts() {
    let mut cfg = convoy();
    cfg.lock = LockKind::Record;
    cfg.peer_events = true;
    holds("break1-control-record", &cfg);
}

// ------------------------------------------------------------------------------------------------
// candidate fixes, MODEL ONLY (production is unchanged on this branch): each turns its break green

/// The checker has teeth: remove the hand-off (the holder tells nobody) and the SAME schedules strand the relay, with a peer's interrupt
/// stream or beside a hot neighbour -- the red tests of the adversary, red again by construction.
#[test]
fn without_the_hand_off_break1_strands_the_relay_again() {
    let mut cfg = convoy();
    cfg.peer_events = true;
    cfg.no_handoff = true;
    breaks("no-handoff/break1", &cfg, V_DEMAND | V_STALL);
}

#[test]
fn without_the_hand_off_the_hot_neighbour_case_strands_the_relay_again() {
    let mut cfg = convoy();
    cfg.hot = true;
    cfg.no_handoff = true;
    breaks("no-handoff/known-p13", &cfg, V_DEMAND | V_STALL);
}

/// The sleep invariant on this model: blocked (parked) means no obligation without a wake source.
#[test]
fn the_parked_worker_holds_no_unannounced_obligation() {
    for cfg in [convoy(), cyclic_ring_full(LifecycleMode::NoPointerMarkers)] {
        let s0 = settle(&cfg);
        let mut stack = vec![s0];
        let mut seen = std::collections::HashSet::new();
        while let Some(s) = stack.pop() {
            if !seen.insert(s.key()) {
                continue;
            }
            assert!(
                !(s.pc == Pc::Parked && s.unannounced()),
                "parked with an unannounced obligation"
            );
            for (_, n) in successors(&cfg, &s) {
                stack.push(n);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------
// pins: the transliterated production lines this model rests on (a change there must revisit the model)

fn src(path: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .expect(path)
}

#[test]
fn pin_nothing_is_budgeted_and_no_look_rings_for_contention() {
    let l = src("src/user_loop.rs");
    assert!(
        !l.contains("CONTENTION_RETRIES") && !l.contains("fn may_retry"),
        "the budget is back"
    );
    assert!(
        l.contains("pub const fn look_needed(_why: Peek, _demand: bool, waiting: bool) -> bool")
    );
    let w = src("src/worker.rs");
    assert!(
        w.contains("if matches!(got, Ok(0)) && !owed {")
            && w.contains("relay.peek(Peek::Backstop)"),
        "the backstop is a park TIMEOUT only (a firing that finds work is a counted bug)"
    );
}

#[test]
fn pin_the_peeks_pop_write_back_covers_marker_and_popped_waits() {
    let p = src("../kf-qemu/src/userpump.rs");
    let flat: String = p.split_whitespace().collect();
    assert!(
        flat.contains(
            &"kf_chan::user_loop::Wait::Marker { .. } | kf_chan::user_loop::Wait::Popped { .. }"
                .split_whitespace()
                .collect::<String>()
        ),
        "the pop write-back condition changed: revisit break 2"
    );
    assert!(p.contains("look_needed(why, false, record.is_waiting())"));
}

#[test]
fn pin_a_pointer_batch_without_a_marker_waits_for_every_issued_pop() {
    let t = src("src/user_turn.rs");
    assert!(
        t.contains(
            "self.set_need(Need::Popped {\n                    target: self.fetch.issued(),"
        ),
        "end_batch's carried wait changed: revisit break 2"
    );
}
