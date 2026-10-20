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
//! ★ 2026-10-20 (simple relay, commit e49f0c225): TRANSLITERATED TO THE ONE-LOOP WORKER. The park timeout (the 50 ms net) is
//! GONE from the model: a parked worker is woken by the doorbell eventfd or an engine event and by nothing else (the production
//! net is a last resort the design does not rely on; `worker::set_park_net(false)`). The budget (`CONTENTION_RETRIES`,
//! `may_retry`, `Wait::Contended`), the bounded pre-park spin and every backstop look are deleted; the properties below (liveness,
//! the bound `K`, the V_* bits) are UNCHANGED, and a new SLEEP-SAFETY bit (`V_SLEEP`) is checked at every state.
//!
//! CODE READ (branch `claude/relay-simple-20261020`, commit e49f0c225 + Change A 41d42ee98), transliterated here and pinned by
//! source text at the bottom:
//!  * worker loop `kf_chan::worker::run_with_relay` is ONE loop: serve (`worker_pass`); `after = policy.after_pass(n)`; unless
//!    `Continue` (the negative control) `rang = relay.peek(Peek::Spin)` -- a LEVEL evaluation of all relays, busy or idle;
//!    `spin = n > 0 || rang || relay.unsourced()`; spin -> `epoll_wait(0)`, else `try_park(seen)` (refused -> next iteration) and
//!    a BLOCKING wait; then `dispatch` (doorbell drained, engine event -> `on_other`, NO peek).
//!  * `ChanPlane::user_ring_serve`: `record.enter()` (sets `retry`) / `pt.try_lock()` busy -> `contended_note`, no turn, `retry`
//!    stays set; else `retry.swap(false)`, the turn, the arm.
//!  * `user_ring_arm`: `wait = wait_of(need)`; `Again` rings; `Contended` -> `record.request()` (demand again, no wait);
//!    `carried` = the wait is `Wait::Popped`.
//!  * `user_ring_peek_one`: `look_needed`, then THE pure decision `user_loop::look` (called here, not copied).
//!  * `user_ring_unsourced`: any relay with demand (`pending()`) or (`is_waiting() && carried`).
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
//! WORKER COUNT: this model has ONE worker (no worker-count parameter; `relay_fairness.rs` models 1, 2 and 3).
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
use kf_chan::user_loop::{self as ul, Look, LookIn, Need, Peek, Wait};
use kf_chan::user_scan::Scan;
use kf_chan::worker::{AfterPass, LoopPolicy};
use std::collections::{HashMap, VecDeque};
use support::*;

/// The bound, in worker ticks (passes and park wakes).
const K: u8 = 12;

const V_STALL: u32 = 1; // terminal state with work left (liveness)
const V_WAIT: u32 = 2; // a satisfied wait not rung within K ticks
const V_DEMAND: u32 = 4; // demand (a doorbell, a contended turn) with its lock free not re-rung within K ticks
const V_SPACE: u32 = 8; // the guest is blocked on ring space the hardware has freed for more than K ticks
const V_I8: u32 = 16; // the relay read beyond GP_PUT / a refusal (safety, from the support layer)
const V_DEAD: u32 = 32; // the relay refused something
const V_SLEEP: u32 = 64; // the worker is in a BLOCKING park while an obligation without a registered wake source exists

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
        (
            V_SLEEP,
            "SLEEP: the worker blocks (no timeout) while an obligation has no registered wake source",
        ),
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
    lock: LockKind,
    /// How many times the adversary may take the lock.
    lock_holds: u8,
    /// The lock is held when the exploration starts.
    lock_at_start: bool,
    /// Exploration cap (a test that hits it fails loudly).
    max_states: usize,
    /// MODEL-ONLY MUTANT: `unsourced()` forgets recorded `Wait::Popped` waits (the `carried` leg).
    mutant_unsourced_ignores_carried: bool,
    /// MODEL-ONLY MUTANT: `unsourced()` forgets durable demand (the retry-set leg).
    mutant_unsourced_ignores_demand: bool,
    /// The negative control `LoopPolicy::without_busy_look()` (`KF3_NEGCTL_NO_BUSY_LOOK=1`): a busy pass never looks.
    no_busy_look: bool,
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
            lock: LockKind::Pt,
            lock_holds: 0,
            lock_at_start: false,
            max_states: 3_000_000,
            mutant_unsourced_ignores_carried: false,
            mutant_unsourced_ignores_demand: false,
            no_busy_look: false,
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
    InTurn { served: u8 },
    /// The pass is over: the after-pass decision.
    After { served: u8 },
    /// The peek is done; `spin` = `n > 0 || rang`. Next: `|| unsourced()` decides poll(0) or park.
    Decide { spin: bool },
    /// `epoll_wait(timeout 0)` then dispatch.
    Poll,
    /// In the BLOCKING `epoll_wait` (no timeout).
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
    /// `Cell::carried`: the recorded wait is `Wait::Popped` (set in the arm).
    carried: bool,
    wait: Option<Wait>,
    rec_held: bool,
    holds_left: u8,
    // tokens and fds
    t_rung: bool,
    efd: bool,
    engine_event: bool,
    // the worker
    pc: Pc,
    policy: LoopPolicy,
    // bounded-wait counters (ticks since the obligation began; capped)
    c_wait: u8,
    c_demand: u8,
    c_space: u8,
    flags: u32,
    /// Model-only mutants of `unsourced()` (applied after the prelude).
    mut_carried: bool,
    mut_demand: bool,
}

impl S {
    fn key(&self) -> String {
        format!(
            "{}|{:?}|{}|{}{}{}{}|{:?}|{}{}|{}{}{}|{:?}|{:?}|{},{},{}|{:x}",
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
            u8::from(self.carried),
            u8::from(self.rec_held),
            self.wait,
            self.holds_left,
            u8::from(self.t_rung),
            u8::from(self.efd),
            u8::from(self.engine_event),
            u8::from(false),
            self.pc,
            self.policy,
            self.c_wait,
            self.c_demand,
            self.c_space,
            self.flags,
        )
    }

    fn lock_free(&self) -> bool {
        !self.g.pt_held && !self.rec_held
    }

    /// `ChanPlane::user_ring_waiting`.
    fn waiting_any(&self) -> bool {
        (self.waiting || self.retry) && !self.relay.dead
    }

    /// `ChanPlane::user_ring_unsourced` (+ the model-only mutants): demand left behind, or a wait no interrupt announces.
    fn unsourced(&self) -> bool {
        !self.relay.dead
            && ((self.retry && !self.mut_demand)
                || (self.waiting && self.carried && !self.mut_carried))
    }

    /// `ring(self)` in the peek: `plane.ring_internal` + `wake.signal()`.
    fn ring(&mut self) {
        self.t_rung = true;
        self.efd = true;
    }

    /// `ChanPlane::user_ring_peek_one`, transliterated: the gate (`look_needed`), the words read fresh, then THE pure decision
    /// `user_loop::look`. Returns whether it rang.
    fn peek(&mut self, why: Peek) -> bool {
        if self.relay.dead {
            return false;
        }
        let demand = self.retry;
        if !ul::look_needed(why, demand, self.waiting) {
            return false;
        }
        if self.rec_held || matches!(self.pc, Pc::InTurn { .. }) {
            // `record.try_lock()` failed: production passes `Some(_)` when the waiting flag is set (the wait itself is unreadable)
            let i = LookIn {
                demand,
                record_busy: true,
                wait: self.waiting.then_some(Wait::Marker { target: u64::MAX }),
                words: None,
                fetched: 0,
                get_follows_pops: false,
            };
            return if matches!(ul::look(i), Look::Ring { .. }) {
                self.ring();
                true
            } else {
                false
            };
        }
        let wait = self.wait;
        let words = if wait.is_some() {
            let get = self.g.hw_get();
            let reference = self.g.hw_reference();
            let marks = &mut self.relay.core.marks;
            self.relay
                .core
                .fetch
                .peek(get)
                .ok()
                .map(|popped| (marks.peek(reference), popped))
        } else {
            None
        };
        let i = LookIn {
            demand,
            record_busy: false,
            wait,
            words,
            fetched: self.relay.core.fetch.fetched(),
            get_follows_pops: kf_chan::user_turn::guest_get_follows_pops(),
        };
        match ul::look(i) {
            Look::Leave => false,
            Look::Ring { clear_wait, .. } => {
                if clear_wait {
                    // production clears `Pump::wait` and `waiting`; `carried` stays and is gated by `waiting`
                    self.wait = None;
                    self.waiting = false;
                }
                self.ring();
                true
            }
        }
    }

    /// `ChanPlane::user_ring_arm`.
    fn arm(&mut self) {
        // `Pump::turn`: a turn that made progress ends `Again` whatever else it said
        if self.relay.core.again && !self.relay.core.dead {
            self.relay.core.need = Need::Again;
        }
        let need = self.relay.core.need;
        self.wait = ul::wait_of(need);
        self.waiting = self.wait.is_some();
        match need {
            Need::Again => self.ring(),
            // the turn lost a lock: back into the retry set (the durable demand flag), no wait
            Need::Contended => self.retry = true,
            Need::Idle | Need::Marker { .. } | Need::Popped { .. } => {}
        }
        self.carried = self.relay.core.wait_unannounced(self.wait);
    }

    /// Is the target's recorded wait satisfied by the hardware words right now (the obligation of `V_WAIT`)?
    fn wait_satisfied_now(&self) -> bool {
        let Some(w) = self.wait else { return false };
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

/// `dispatch` of the ready fds: the doorbell eventfd is drained, an engine event goes to `on_other`. NO peek (the next iteration's
/// level look covers it).
fn dispatch(s: &mut S) {
    s.efd = false;
    s.engine_event = false;
}

/// Sleep safety: the worker is in a BLOCKING park (no timeout exists) while an obligation lacking a registered wake source
/// exists: demand with the token not rung, a recorded `Wait::Popped`, a recorded wait the words already satisfy and nobody rang, or
/// the token rung without the eventfd. The eventfd and the engine event are the registered sources.
fn sleep_unsafe(s: &S) -> bool {
    // A RECORD-lock holder (an act thread) rings the relay at its completion (`user_ring_wake`): a registered wake source. (The
    // `pt` / admission holder rings nobody: that is the retry set's case.)
    if s.pc != Pc::Parked || s.efd || s.engine_event || s.relay.dead || s.rec_held {
        return false;
    }
    s.t_rung || s.retry || (s.waiting && s.carried) || (s.wait.is_some() && s.wait_satisfied_now())
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
            LockKind::Pt => n.g.pt_held = false,
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
            format!("the {:?} lock is released (pt: nobody is told)", cfg.lock),
            n,
        ));
    }
    if cfg.peer_events && !s.engine_event && !in_turn(s) {
        let mut n = s.clone();
        n.engine_event = true;
        out.push(("a PEER channel's engine interrupt becomes ready".into(), n));
    }
}

fn worker_steps(cfg: &Cfg, s: &S, out: &mut Vec<(String, S)>) {
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
                    "pass: serve -> record busy (contended_note, no turn)".into()
                } else if n.g.pt_held {
                    n.pc = Pc::After {
                        served: 1 + u8::from(cfg.hot),
                    };
                    "pass: serve -> pt busy (contended_note, no turn)".into()
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
                n.arm();
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
            // `after = policy.after_pass(n)`; unless `Continue`: `rang = relay.peek(Peek::Spin)` (level evaluation, busy or idle)
            let after = n.policy.after_pass(served as usize);
            let rang = if after == AfterPass::Continue {
                false
            } else {
                n.peek(Peek::Spin)
            };
            n.pc = Pc::Decide {
                spin: served > 0 || rang,
            };
            format!("{after:?}: level look (peek Spin) rang={rang}")
        }
        Pc::Decide { spin } => {
            if spin || n.unsourced() {
                n.pc = Pc::Poll;
                "spin = n>0 || rang || unsourced(): epoll_wait(0)".into()
            } else {
                park(&mut n);
                "nothing to spin on: try_park, then the BLOCKING wait".into()
            }
        }
        Pc::Poll => {
            dispatch(&mut n);
            n.pc = Pc::Top;
            "epoll_wait(0) + dispatch (doorbell drained, engine event to on_other)".into()
        }
        Pc::Parked => {
            if n.efd || n.engine_event {
                tick(&mut n);
                let ev = n.engine_event;
                dispatch(&mut n);
                n.pc = Pc::Top;
                if ev {
                    "park wakes on an ENGINE event (dispatch, no peek)".into()
                } else {
                    "park wakes on the doorbell eventfd (no peek)".into()
                }
            } else {
                return; // infinite park: no timeout step exists
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
    worker_steps(cfg, s, &mut out);
    out
}

/// The steps of everything but the worker.
fn external_steps(cfg: &Cfg, s: &S) -> Vec<(String, S)> {
    let mut out = Vec::new();
    guest_step(cfg, s, &mut out, None);
    gpu_steps(cfg, s, &mut out);
    adversary_steps(cfg, s, &mut out);
    out
}

/// LIVENESS terminal condition. With a worker that may SPIN (the retry set, a carried wait) "no step is enabled" is never true,
/// so: the guest / GPU / adversary have no step and no schedule of the worker alone (its closure, bounded) finishes the work or
/// enables one. A parked worker with nothing to wake it is the degenerate case (the closure is the one state).
fn stalled(cfg: &Cfg, s: &S) -> bool {
    if !work_left(s) || !external_steps(cfg, s).is_empty() {
        return false;
    }
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut q = VecDeque::new();
    seen.insert(s.key(), ());
    q.push_back(s.clone());
    while let Some(t) = q.pop_front() {
        if !work_left(&t) || !external_steps(cfg, &t).is_empty() {
            return false;
        }
        let mut out = Vec::new();
        worker_steps(cfg, &t, &mut out);
        for (_, n) in out {
            if seen.insert(n.key(), ()).is_none() {
                assert!(seen.len() < 100_000, "worker-only closure too large");
                q.push_back(n);
            }
        }
    }
    true
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
        carried: false,
        wait: None,
        rec_held: false,
        holds_left: cfg.lock_holds,
        t_rung: false,
        efd: false,
        engine_event: false,
        pc: Pc::Parked,
        policy: LoopPolicy::default(),
        c_wait: 0,
        c_demand: 0,
        c_space: 0,
        flags: 0,
        mut_carried: false,
        mut_demand: false,
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
            worker_steps(cfg, &s, &mut out);
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
    s.policy = if cfg.no_busy_look {
        LoopPolicy::without_busy_look()
    } else {
        LoopPolicy::default()
    };
    s.mut_carried = cfg.mutant_unsourced_ignores_carried;
    s.mut_demand = cfg.mutant_unsourced_ignores_demand;
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
        let flags = s.flags | s.g.flags | if sleep_unsafe(&s) { V_SLEEP } else { 0 };
        if flags != 0 {
            return Err(Found {
                flags,
                trace: path(&nodes, i),
                states: seen.len(),
            });
        }
        let next = successors(cfg, &s);
        if stalled(cfg, &s) {
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
/// neighbour on the worker, the busy pass's level look (every pass, no period) finds the satisfied wait within the bound.
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

/// A pt-lock window (one hold) across a doorbell: the retry set re-rings it (no budget).
#[test]
fn one_pt_window_across_a_doorbell_is_re_rung_within_k_ticks() {
    let mut cfg = Cfg::new(script(vec![Op::Push(vec![plain(1)]), Op::SetPut, Op::Ring]));
    cfg.lock_holds = 1;
    holds("pt-window", &cfg);
}

// ------------------------------------------------------------------------------------------------
// BREAK 1: a contended relay was stranded while ANY engine interrupt kept the park from timing out (the budget is DELETED
// 2026-10-20: the retry set re-rings demand on every look; these configs are unchanged and now hold)

/// One doorbell; the `pt` lock (or USERD admission) is held across it by another thread and released with no announcement.
/// (`contended_at_start` was the pre-spent budget; there is no budget any more, so `convoy()` is the only convoy.)
fn convoy() -> Cfg {
    let mut cfg = Cfg::new(script(vec![Op::Push(vec![plain(1)]), Op::SetPut, Op::Ring]));
    cfg.lock = LockKind::Pt;
    cfg.lock_at_start = true;
    cfg
}

/// Control: the same convoy on an IDLE worker with nothing else happening. REPLACES
/// `break1_control_a_convoy_past_the_retry_budget_on_a_quiet_worker_is_rescued_by_the_safety_net` (there is no safety-net
/// rescue to test: the worker re-rings the demand itself, with the park infinite): the relay is re-rung WITHOUT ANY TIMEOUT.
#[test]
fn break1_control_a_convoy_on_a_quiet_worker_is_re_rung_without_any_timeout() {
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
// the candidate-fix flags are GONE (their behaviour is production now). REPLACED:
//  * `fix1_candidate_an_event_look_may_re_ring_a_contended_relay_closes_break1_and_1b` -> the two break1 tests above (same
//    configs, no flag: the event no longer needs a special look, every look re-rings demand) plus the tests below;
//  * `fix2_candidate_a_pop_write_back_for_popped_waits_closes_break2` -> `break2_expected_red_*` above (no flag: `user_loop::look`
//    covers every wait kind while the guest's GP_GET follows pops, and with Change A the guest's GP_GET is the read index).

/// Replaces fix1's "closes break 1 and 1b": the convoy stays live with the lock taken again and again, a hot neighbour AND a
/// peer interrupt stream, whatever the order.
#[test]
fn simple_relay_a_repeated_pt_convoy_beside_a_hot_neighbour_and_peer_interrupts_is_live_and_bounded()
 {
    let mut cfg = convoy();
    cfg.hot = true;
    cfg.peer_events = true;
    cfg.lock_holds = 2;
    holds("simple/convoy-hot-peer", &cfg);
}

/// Replaces fix2's "closes break 2": markers off, the ring-full cycle, plus peer interrupts, is live and the guest is never
/// blocked on space the hardware has freed for more than K ticks.
#[test]
fn simple_relay_a_carried_wait_with_a_full_guest_ring_is_live_and_bounded_with_peer_interrupts() {
    let mut cfg = cyclic_ring_full(LifecycleMode::NoPointerMarkers);
    cfg.peer_events = true;
    holds("simple/break2-peer", &cfg);
}

// ------------------------------------------------------------------------------------------------
// the NEGATIVE CONTROL (`LoopPolicy::without_busy_look`): the model can see the pre-fix starvation

/// With a hot neighbour and a carried wait, a worker that never looks after a busy pass strands the satisfied wait (V_WAIT).
#[test]
fn negctl_no_busy_look_strands_a_satisfied_carried_wait_beside_a_hot_neighbour() {
    let mut cfg = Cfg::new(script(vec![
        Op::Push(vec![plain(1), plain(2)]),
        Op::SetPut,
        Op::Ring,
    ]));
    cfg.mode = LifecycleMode::NoPointerMarkers;
    cfg.hot = true;
    holds("negctl-control", &cfg);
    cfg.no_busy_look = true;
    breaks("negctl", &cfg, V_WAIT | V_DEMAND | V_STALL | V_SPACE);
}

// ------------------------------------------------------------------------------------------------
// SLEEP SAFETY (V_SLEEP): the worker blocks only when every obligation has a registered wake source. The unmutated configs
// hold; each mutant of `unsourced()` is found by at least two scenarios.

/// A carried wait recorded when the worker would block: markers latched off, a batch, a ring-full cycle.
fn carried_scenarios() -> Vec<(&'static str, Cfg)> {
    let mut a = Cfg::new(script(vec![
        Op::Push(vec![plain(1), plain(2)]),
        Op::SetPut,
        Op::Ring,
    ]));
    a.mode = LifecycleMode::NoPointerMarkers;
    let b = cyclic_ring_full(LifecycleMode::NoPointerMarkers);
    let mut c = a.clone();
    c.peer_events = true;
    vec![
        ("carried/batch", a),
        ("carried/ring-full", b),
        ("carried/peer", c),
    ]
}

/// Demand left behind when the worker would block: a `pt` lock (nobody rings at its release) or a record-lock act held across a
/// doorbell (the doorbell sets the demand flag and the eventfd but rings no token).
fn demand_scenarios() -> Vec<(&'static str, Cfg)> {
    let mut a = convoy();
    a.lock = LockKind::Record;
    let mut b = Cfg::new(script(vec![Op::Push(vec![plain(1)]), Op::SetPut, Op::Ring]));
    b.lock = LockKind::Record;
    b.lock_holds = 1;
    let mut c = b.clone();
    c.lock = LockKind::Pt;
    let mut d = convoy();
    d.peer_events = true;
    d.lock_holds = 2;
    vec![
        ("demand/record-at-start", a),
        ("demand/record-window", b),
        ("demand/pt-window", c),
        ("demand/pt-at-start-peer-events", d),
    ]
}

#[test]
fn sleep_safety_holds_for_every_scenario_unmutated() {
    for (name, cfg) in carried_scenarios().into_iter().chain(demand_scenarios()) {
        holds(&format!("sleep/{name}"), &cfg);
    }
}

#[test]
fn mutant_unsourced_ignores_carried_is_found_by_a_batch_with_markers_latched_off() {
    let (name, mut cfg) = carried_scenarios().remove(0);
    cfg.mutant_unsourced_ignores_carried = true;
    breaks(name, &cfg, V_SLEEP);
}

#[test]
fn mutant_unsourced_ignores_carried_is_found_by_the_ring_full_cycle() {
    let (name, mut cfg) = carried_scenarios().remove(1);
    cfg.mutant_unsourced_ignores_carried = true;
    breaks(name, &cfg, V_SLEEP);
}

#[test]
fn mutant_unsourced_ignores_carried_is_found_with_peer_interrupts() {
    let (name, mut cfg) = carried_scenarios().remove(2);
    cfg.mutant_unsourced_ignores_carried = true;
    breaks(name, &cfg, V_SLEEP);
}

/// ★ `mutant_unsourced_ignores_demand` is EQUIVALENT in this one-worker model: the look that precedes the park decision rings every
/// demand it sees (`user_loop::look`: demand always rings), a ring leaves the token rung (so `try_park` is refused) and the
/// doorbell that arrives after the look leaves its eventfd readable (the park returns at once). The demand leg of `unsourced()` is
/// belt and braces here (it is load-bearing only if the look may skip a demand, e.g. under a mutant of `look`; see the next test).
/// Each scenario below holds with the leg deleted -- 4 scenarios, exhaustive -- and the pair of tests documents it.
#[test]
fn mutant_unsourced_ignores_demand_is_equivalent_here_scenarios_a_and_b() {
    for (name, mut cfg) in demand_scenarios().into_iter().take(2) {
        cfg.mutant_unsourced_ignores_demand = true;
        holds(&format!("equiv/{name}"), &cfg);
    }
}

#[test]
fn mutant_unsourced_ignores_demand_is_equivalent_here_scenarios_c_and_d() {
    for (name, mut cfg) in demand_scenarios().into_iter().skip(2) {
        cfg.mutant_unsourced_ignores_demand = true;
        holds(&format!("equiv/{name}"), &cfg);
    }
}

// ------------------------------------------------------------------------------------------------
// pins: the transliterated production lines this model rests on (a change there must revisit the model)

fn src(path: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .expect(path)
}

/// Replaces `pin_only_the_safety_net_may_re_ring_past_the_contention_budget` (the budget and `Peek::Backstop` are deleted).
#[test]
fn pin_the_budget_and_the_backstop_look_are_gone_and_the_loop_is_one_level_loop() {
    let l = src("src/user_loop.rs");
    let prod = l.split("#[cfg(test)]").next().unwrap();
    for gone in [
        "CONTENTION_RETRIES",
        "fn may_retry",
        "Peek::Backstop",
        "Backstop,",
        "Wait::Contended",
        "pub const SPIN",
        "pub fn idle",
    ] {
        assert!(!prod.contains(gone), "{gone} is back: revisit the model");
    }
    let w = src("src/worker.rs");
    let w = w.split("#[cfg(test)]").next().unwrap();
    for gone in ["const LOOK_EVERY", "Peek::Backstop"] {
        assert!(!w.contains(gone), "{gone} is back: revisit the model");
    }
    for order in [
        "plane.worker_pass(",
        "policy.after_pass(n)",
        "relay.peek(Peek::Spin)",
        "let spin = n > 0 || rang || relay.unsourced();",
        "PollTimeout::Immediate",
        "try_park(seen)",
        "PollTimeout::Blocking",
        "dispatch(&ready)",
    ] {
        assert!(
            w.contains(order),
            "pin: {order:?} missing from run_with_relay"
        );
    }
    let mut at = 0;
    for o in [
        "plane.worker_pass(",
        "policy.after_pass(n)",
        "relay.peek(Peek::Spin)",
        "let spin = n > 0 || rang || relay.unsourced();",
        "try_park(seen)",
        "dispatch(&ready)",
    ] {
        let i = w[at..]
            .find(o)
            .unwrap_or_else(|| panic!("{o:?} out of order"));
        at += i + o.len();
    }
}

/// Replaces `pin_the_peeks_pop_write_back_is_for_marker_waits_only` and the `may_retry` / `demand && retry_ok` pins: the peek calls
/// THE pure decision; the arm re-sets demand for `Contended`; `unsourced` is demand or a carried wait.
#[test]
fn pin_the_peek_calls_look_the_arm_re_requests_and_unsourced_is_demand_or_carried() {
    let p = src("../kf-qemu/src/userpump.rs");
    let peek = p.split("fn user_ring_peek_one(").nth(1).expect("peek_one");
    let peek = peek
        .split("pub(crate) fn user_ring_unsourced")
        .next()
        .unwrap();
    assert!(
        peek.contains("match look(i) {"),
        "the peek no longer calls user_loop::look"
    );
    assert!(!peek.contains("may_retry"));
    let arm = p.split("fn user_ring_arm(").nth(1).expect("arm");
    let arm = arm.split("pub(crate) fn user_ring_waiting").next().unwrap();
    assert!(
        arm.contains("Need::Contended => {") && arm.contains("let _ = record.request();"),
        "Contended must re-set the demand flag"
    );
    assert!(
        arm.contains("record.set_carried(p.core.wait_unannounced(p.wait));")
    );
    assert!(
        p.contains("record.pending() || (record.is_waiting() && record.carried())"),
        "unsourced() changed: revisit the mutants"
    );
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
