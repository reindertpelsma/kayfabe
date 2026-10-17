//! ★ 2026-10-14 — **Exhaustive interleaving model of the USER-ring relay's worker loop**
//! (`docs/design/V3_USER_RING_EVENT_LOOP_20261014.md`). GPU-free, deterministic, NO wall-clock time
//! anywhere: a "timer" is just a scheduler step that the configuration may forbid.
//!
//! WHAT IS MODELLED (every item is a separate ATOMIC step, so the explorer interleaves the guest,
//! the PBDMA, the environment and the worker between any two of them):
//!
//! * the **guest**, a nondeterministic writer running a script of `Fill` (late pushbuffer bytes),
//!   `Push` (a GP entry), `SetPut` (`GP_PUT`), `Ring` (the doorbell), `Release` (the semaphore a
//!   segment's acquire waits for), `UseNew` (switch to the migrated ring), `PutRaw` (hostile PUT);
//! * the **doorbell token** word (`kf_trap::token`: IDLE/RUNG/BUSY/BUSY_RUNG — a ring while BUSY is
//!   picked up by the claim's own re-check), and the **wake word** (`kf_trap::wake`: `seen` before the
//!   scan, `try_park` fails when the sequence moved — the lost-wakeup rule);
//! * the **worker**: `Top0 seen=seq`, `Top1 scan+claim`, `Top2 try_park`, `Parked` (woken by the
//!   eventfd, a completion event, or — only when the configuration lets it fire — a timeout), and a
//!   claimed **turn** transliterated from `Pump::turn` (`crates/kf-qemu/src/userpump.rs:1170`):
//!   observe `GP_GET` and write the guest's back, read `GP_PUT` AFTER the claim, read the entry, gate
//!   a `SYNC_WAIT` entry with the REAL [`kf_chan::user_gate::SyncGate`], read the pushbuffer, publish;
//! * the **PBDMA** consuming the private ring in order (`GP_GET` passes an entry once it BEGAN; a
//!   segment's trailing acquire blocks the next entry; a witness/notify raises the completion
//!   interrupt) and the optional visibility lag of `GP_GET`;
//! * the **policies**: [`Policy::Today`] = the loop as committed (`worker.rs` park/`TICK_TAG`,
//!   `userpump.rs` `tick`/`retry`, `user_gate::Poll`), [`Policy::Design`] = the event loop the doc
//!   proposes (doorbell, completion event, bounded spin, backstop with counters).
//!
//! HOW THE CHECKS ARE DERIVED (not from today's behaviour): the owner's rules in the task text and the
//! public hardware text (`dev_pbdma.ref` `GP_ENTRY1_SYNC`, `GP_GET`, `LENGTH == 0`):
//!   1. liveness with timers that NEVER fire: after a doorbell the work it covers is relayed;
//!   2. no turn is ever caused by a timer; the backstop never finds work when no interrupt is lost;
//!   3. the relay never reads a pushbuffer behind a guest `SYNC_WAIT` before the drain completes, and
//!      never publishes a method byte inside an open packet;
//!   4. the PBDMA executes exactly the guest's entries, in order, none dropped or replayed;
//!   5. the worker is never parked with a doorbell flag or an undelivered completion pending.
//!
//! The tests of [`Policy::Today`] are expected to FAIL (they are the gap list); the tests of
//! [`Policy::Design`] and of the deliberately broken mutants validate that the checker is neither
//! vacuous nor unsatisfiable.
#![allow(clippy::too_many_lines, clippy::needless_range_loop)]

use kf_chan::user_gate::{Gate, SyncGate, Witness, witness};
use kf_chan::user_loop::{self as ul, Need, Wait};
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

// ------------------------------------------------------------------------------------------------
// vocabulary

/// Guest GP ring slots (a power of two; one slot is always empty).
const R: usize = 4;
/// Bounded `ActAgain` rounds per claim (`kf_trap::REACT_ROUNDS` is 8; 2 keeps the model small).
const REACT: u8 = 2;
/// A GP slot the guest never wrote (all zero = `LENGTH == 0`, `OPCODE == NOP`).
const ZERO_ID: u8 = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Seg,
    Nop,
    /// A segment whose last packet is the migration trigger (the guest's `FIFO_UPDATE_CHANNEL_INFO`).
    Trigger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Ge {
    id: u8,
    kind: Kind,
    sync: bool,
    /// The segment ends with a semaphore acquire on this semaphore (the PBDMA blocks until released).
    acq: Option<u8>,
    /// The segment ends inside a method packet (its arguments continue in a later entry).
    open: bool,
    /// The pushbuffer bytes are written by an explicit `Fill`, not at `Push`.
    late: bool,
}
fn seg(id: u8) -> Ge {
    Ge {
        id,
        kind: Kind::Seg,
        sync: false,
        acq: None,
        open: false,
        late: false,
    }
}
fn nop(id: u8) -> Ge {
    Ge {
        kind: Kind::Nop,
        ..seg(id)
    }
}
fn trigger(id: u8) -> Ge {
    Ge {
        kind: Kind::Trigger,
        ..seg(id)
    }
}
impl Ge {
    fn sync(self) -> Self {
        Self { sync: true, ..self }
    }
    fn acq(self, s: u8) -> Self {
        Self {
            acq: Some(s),
            ..self
        }
    }
    fn open(self) -> Self {
        Self { open: true, ..self }
    }
    fn late(self) -> Self {
        Self { late: true, ..self }
    }
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Fill(u8),
    Push(Ge),
    SetPut,
    Ring,
    Release(u8),
    UseNew,
    PutRaw(u32),
    /// The act thread retires the channel (guest free / device reset); waits out a BUSY token.
    Retire,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Policy {
    /// The loop as committed at 77eaf7620.
    Today,
    /// The loop of `V3_USER_RING_EVENT_LOOP_20261014.md` §3.
    Design,
}

/// Deliberately broken variants of the design: the checker must find each one.
#[derive(Clone, Copy, Debug, Default)]
struct Mutant {
    /// `try_park` without the sequence comparison (a lost wakeup).
    no_seq_check: bool,
    /// The token release forgets a ring that arrived while BUSY.
    drop_busy_rung: bool,
    /// The gate always authors the byte witness.
    bytes_witness: bool,
    /// No gate at all: the pushbuffer is read when `GP_PUT` names it.
    no_gate: bool,
    /// The holder of a busy lock does not wake the worker on release.
    no_lock_wake: bool,
    /// The worker parks without ever arming a completion interrupt.
    no_notify: bool,
    /// The RETIRED proof: the gate's marker is "processed" when `GP_GET` has popped past it.
    get_proof: bool,
    /// The RETIRED idle backstop: read `GP_PUT` and ring the relay when it is ahead of the cursor.
    backstop_polls_put: bool,
}

#[derive(Clone, Debug)]
struct ChanSpec {
    script: Vec<Op>,
    /// The ids the PBDMA must execute, in order (all of them if the guest keeps its contract).
    expected: Vec<u8>,
    /// At terminal states at least this many leading ids must have executed (those covered by a ring).
    must_relay: usize,
    /// The script may legitimately end blocked (a semaphore nobody releases, a dead channel).
    may_block: bool,
}

#[derive(Clone, Debug)]
struct Cfg {
    policy: Policy,
    m: Mutant,
    chans: Vec<ChanSpec>,
    /// May timeouts / elapsed time ever fire? (`false` = timers that never fire.)
    timers: bool,
    /// Does the PBDMA's `GP_GET` run ahead of processing (measured: yes)? `false` is the sequential
    /// reading the retired gate assumed.
    prefetch: bool,
    /// How many entries `GP_GET` may run ahead of processing (measured >= 3; the limit is unknown):
    /// `usize::MAX` = every published entry at once, small values make the private ring's credit
    /// waits reachable.
    pop_depth: usize,
    /// Does a completion interrupt reach the worker's epoll? (`false` = lost / never armed.)
    irq: bool,
    /// May the visible `GP_GET` lag the PBDMA's by an arbitrary time?
    lag: bool,
    /// Design: bounded spin iterations before blocking.
    spin: u8,
    /// Design: may the backstop wake fire (only when parked)?
    backstop: bool,
    /// Private ring entries (a power of two >= 4).
    priv_entries: usize,
    /// How many times may an act-thread lock window block the worker's record lock?
    lock_windows: u8,
    /// How many times may the pt / USERD admission lock be busy at the guest `GP_GET` write?
    pt_windows: u8,
    /// Check terminal states (needs a finite graph: timers off or no cycles).
    liveness: bool,
}
impl Cfg {
    fn new(policy: Policy, chans: Vec<ChanSpec>) -> Self {
        Self {
            policy,
            m: Mutant::default(),
            chans,
            timers: false,
            prefetch: true,
            pop_depth: usize::MAX,
            irq: true,
            lag: false,
            spin: 1,
            backstop: false,
            priv_entries: 8,
            lock_windows: 0,
            pt_windows: 0,
            liveness: true,
        }
    }
}

// ------------------------------------------------------------------------------------------------
// state

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Tok {
    Idle,
    Rung,
    Busy,
    BusyRung,
    Dead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Cause {
    Doorbell,
    Again,
    /// Today: `TICK_TAG` -> `user_ring_refresh` re-rings a pump (a timer-driven turn).
    Tick,
    Completion,
    Spin,
    Backstop,
    LockRelease,
}

/// One entry of the private ring, as the PBDMA sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum P {
    Seg {
        id: u8,
        acq: Option<u8>,
    },
    GuestNop {
        id: u8,
    },
    /// Host barrier: a `LENGTH == 0` SYNC_WAIT control entry.
    Barrier,
    /// Host witness after the barrier: `bytes` = a zero DMA_NOP word, else a control entry.
    Witness {
        bytes: bool,
    },
    /// Design only: a marker segment that raises the completion interrupt.
    Notify,
}

/// Wrapper so the real `SyncGate` (Copy + Debug + Eq, no Hash) can sit in a hashed state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Gt(SyncGate);
impl Hash for Gt {
    fn hash<H: Hasher>(&self, h: &mut H) {
        format!("{:?}", self.0).hash(h);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Chan {
    // ---- guest
    gpc: usize,
    gring: usize,
    ring: [[Option<Ge>; R]; 2],
    put_local: [u32; 2],
    put: [u32; 2],
    guest_get: [u32; 2],
    pb: u64,
    sem: u64,
    pushed_since_put: usize,
    visible: usize,
    covered: usize,
    // ---- token
    tok: Tok,
    cause: Cause,
    // ---- PBDMA / private ring
    log: Vec<P>,
    /// The PBDMA's PROCESSING pointer: the next private entry to execute.
    hw_get: usize,
    /// The entries the PBDMA has POPPED from the GPFIFO: what its `GP_GET` word reports. MEASURED
    /// (bare metal, RTX 4070, 595.91.07, `kf-syncwait-stress --families barrier`): it runs AHEAD of
    /// processing -- with an ACQUIRE stalling entry 0 it already read 3 of 3 entries while the segment
    /// behind a SYNC_WAIT barrier had not executed.
    popped: usize,
    userd_get: usize,
    /// `USERD.REFERENCE` as a count of reference-bearing markers PROCESSED (in order, after every
    /// earlier method including acquires).
    reference: u64,
    hw_wait: Option<u8>,
    executed: Vec<u8>,
    ev: bool,
    // ---- relay
    src: usize,
    cursor: u32,
    gener: u8,
    gate: Gt,
    issued: u64,
    fetched: u64,
    /// Guest-ring receipt `(generation, next cursor)` of each issued, not yet fetched entry.
    pend: VecDeque<Option<(u8, u32)>>,
    wb: Option<(u8, u32)>,
    open: bool,
    unnotified: bool,
    /// Reference-bearing markers published / observed processed by the relay.
    refs_issued: u64,
    ref_seen: u64,
    /// The marker count the outstanding gate waits for.
    gate_target: u64,
    /// `USERD.REFERENCE` as the CPU sees it (follows `reference`; lags like `GP_GET` if `lag`).
    ref_visible: u64,
    /// The most recent marker the relay published carries a completion interrupt.
    marker_event: bool,
    relayed: Vec<u8>,
    dead: bool,
    lock_held: bool,
    pt_held: bool,
    /// Consecutive contended turns (`Cell::contended`, `user_loop::may_retry`).
    contended_run: u8,
    // ---- Today's policy state (`Pump::tick`, `Cell::retry`, `Poll`)
    tick: bool,
    retry: bool,
    poll_active: bool,
    poll_due: bool,
    // ---- Design's policy state
    waiting: Option<Wait>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pc {
    Top0,
    Top1,
    Top2,
    Spin,
    Parked,
    /// Today: after a wake, `on_other(TICK_TAG)` -> `user_ring_refresh`.
    TickRefresh,
    /// Design: after a completion-event wake.
    Event,
    /// Inside a claimed turn on `cur`: stage.
    T(u8),
}
const T0: u8 = 0; // serve entry: request()
const T0B: u8 = 7; // serve entry: try_lock
const T1: u8 = 1; // observe GP_GET
const T1B: u8 = 2; // write the guest's GP_GET
const T2: u8 = 3; // read GP_PUT
const T3: u8 = 4; // read the GP entry
const T4: u8 = 5; // gate
const T5: u8 = 6; // read pushbuffer + publish

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct World {
    ch: Vec<Chan>,
    pc: Pc,
    cur: usize,
    rounds: u8,
    /// `WakeWord`: the sequence moved since the worker's `seen` load (kept as a flag so a cycle of
    /// timer wakes is a cycle of states, not an ever-growing counter).
    moved: bool,
    spin_left: u8,
    parked: bool,
    efd: bool,
    locks_left: u8,
    pts_left: u8,
    viol: u32,
}

const V_STALE: u32 = 1 << 0;
const V_BYTES_IN_OPEN_PACKET: u32 = 1 << 1;
const V_TIMER_TURN: u32 = 1 << 2;
const V_BACKSTOP_FOUND_WORK: u32 = 1 << 3;
const V_BACKSTOP_CARRIED: u32 = 1 << 4;
const V_ORDER: u32 = 1 << 5;
const V_PARKED_WITH_WORK: u32 = 1 << 6;
const V_PUT_BOUND: u32 = 1 << 8;
const V_PUT_POLL: u32 = 1 << 10;
const V_BACKSTOP_CONTENDED: u32 = 1 << 11;
const V_UNBOUNDED: u32 = 1 << 9;

fn viol_names(v: u32) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (bit, name) in [
        (
            V_STALE,
            "STALE_READ (pushbuffer read before the guest wrote it / before the drain)",
        ),
        (
            V_BYTES_IN_OPEN_PACKET,
            "BYTES_IN_OPEN_PACKET (a method byte published inside an open packet)",
        ),
        (V_TIMER_TURN, "TIMER_TURN (a turn caused by a timer tick)"),
        (
            V_BACKSTOP_FOUND_WORK,
            "BACKSTOP_FOUND_WORK (the safety net was needed)",
        ),
        (
            V_BACKSTOP_CARRIED,
            "BACKSTOP_CARRIED (counted separately: control-witness gate)",
        ),
        (
            V_ORDER,
            "ORDER (the PBDMA executed something other than the guest's entries in order)",
        ),
        (
            V_PARKED_WITH_WORK,
            "PARKED_WITH_WORK (blocked while a doorbell flag / event was pending)",
        ),
        (V_PUT_BOUND, "PUT_BOUND (hostile GP_PUT refused)"),
        (
            V_BACKSTOP_CONTENDED,
            "BACKSTOP_CONTENDED (counted apart: the safety net re-rang a relay whose lock was busy)",
        ),
        (
            V_PUT_POLL,
            "PUT_POLL (a timer read GP_PUT and rang the relay: the doorbell contract forbids any GP_PUT poll)",
        ),
        (
            V_UNBOUNDED,
            "UNBOUNDED (the relay published more than one ring lap beyond the guest's entries)",
        ),
    ] {
        if v & bit != 0 {
            out.push(name);
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------
// helpers

impl Chan {
    fn new() -> Self {
        Self {
            gpc: 0,
            gring: 0,
            ring: [[None; R]; 2],
            put_local: [0; 2],
            put: [0; 2],
            guest_get: [0; 2],
            pb: 0,
            sem: 0,
            pushed_since_put: 0,
            visible: 0,
            covered: 0,
            tok: Tok::Idle,
            cause: Cause::Doorbell,
            log: Vec::new(),
            hw_get: 0,
            popped: 0,
            userd_get: 0,
            reference: 0,
            hw_wait: None,
            executed: Vec::new(),
            ev: false,
            src: 0,
            cursor: 0,
            gener: 1,
            gate: Gt(SyncGate::default()),
            issued: 0,
            fetched: 0,
            pend: VecDeque::new(),
            wb: None,
            open: false,
            unnotified: false,
            refs_issued: 0,
            ref_seen: 0,
            gate_target: 0,
            ref_visible: 0,
            marker_event: false,
            relayed: Vec::new(),
            dead: false,
            lock_held: false,
            pt_held: false,
            contended_run: 0,
            tick: false,
            retry: false,
            poll_active: false,
            poll_due: false,
            waiting: None,
        }
    }
    fn avail(&self, entries: usize) -> usize {
        entries - 1 - self.pend.len()
    }
    fn alive(&self) -> bool {
        !self.dead && self.tok != Tok::Dead
    }
    fn publish(&mut self, p: P, receipt: Option<(u8, u32)>) {
        self.log.push(p);
        self.pend.push_back(receipt);
        self.issued += 1;
    }
}

impl World {
    fn new(cfg: &Cfg) -> Self {
        Self {
            ch: cfg.chans.iter().map(|_| Chan::new()).collect(),
            pc: Pc::Top0,
            cur: 0,
            rounds: 0,
            moved: false,
            spin_left: cfg.spin,
            parked: false,
            efd: false,
            locks_left: cfg.lock_windows,
            pts_left: cfg.pt_windows,
            viol: 0,
        }
    }

    /// `Plane::ring_internal` / the trap: IDLE -> RUNG (+ bump, + eventfd when parked); a ring while
    /// BUSY is recorded in the same word and picked up at release.
    fn ring(&mut self, c: usize, cause: Cause) {
        match self.ch[c].tok {
            Tok::Idle => {
                self.ch[c].tok = Tok::Rung;
                self.ch[c].cause = cause;
                self.moved = true;
                if self.parked {
                    self.efd = true;
                }
            }
            Tok::Busy => {
                self.ch[c].tok = Tok::BusyRung;
                self.ch[c].cause = cause;
            }
            Tok::Rung | Tok::BusyRung | Tok::Dead => {}
        }
    }
}

// ------------------------------------------------------------------------------------------------
// successors

struct Step {
    label: String,
    w: World,
}
fn step(label: impl Into<String>, w: World) -> Step {
    Step {
        label: label.into(),
        w,
    }
}

fn guest_steps(cfg: &Cfg, w: &World, out: &mut Vec<Step>) {
    for c in 0..w.ch.len() {
        let ch = &w.ch[c];
        let Some(op) = cfg.chans[c].script.get(ch.gpc).copied() else {
            continue;
        };
        let mut n = w.clone();
        let g = ch.gring;
        match op {
            Op::Fill(id) => n.ch[c].pb |= 1 << id,
            Op::Push(e) => {
                let next = (ch.put_local[g] + 1) % R as u32;
                if next == ch.guest_get[g] {
                    continue; // ring full by the guest's own view (GP_GET is the relay's write-back)
                }
                n.ch[c].ring[g][ch.put_local[g] as usize] = Some(e);
                n.ch[c].put_local[g] = next;
                n.ch[c].pushed_since_put += 1;
                if !e.late && e.kind != Kind::Nop {
                    n.ch[c].pb |= 1 << e.id;
                }
            }
            Op::SetPut => {
                n.ch[c].put[g] = ch.put_local[g];
                n.ch[c].visible += ch.pushed_since_put;
                n.ch[c].pushed_since_put = 0;
            }
            Op::Ring => {
                n.ch[c].covered = ch.visible;
                n.ring(c, Cause::Doorbell);
            }
            Op::Release(s) => n.ch[c].sem |= 1 << s,
            Op::UseNew => n.ch[c].gring = 1,
            Op::PutRaw(v) => n.ch[c].put[g] = v,
            Op::Retire => {
                if matches!(ch.tok, Tok::Busy | Tok::BusyRung) {
                    continue; // retire() waits out BUSY
                }
                n.ch[c].tok = Tok::Dead;
                n.ch[c].dead = true;
                n.ch[c].waiting = None;
                n.ch[c].tick = false;
                n.ch[c].retry = false;
                n.ch[c].poll_active = false;
            }
        }
        n.ch[c].gpc += 1;
        out.push(step(format!("g{c}:{op:?}"), n));
    }
}

fn hw_steps(cfg: &Cfg, w: &World, out: &mut Vec<Step>) {
    for c in 0..w.ch.len() {
        let ch = &w.ch[c];
        // the next entry may begin only when the segment before it is processed (its acquire done)
        let blocked = ch.hw_wait.is_some_and(|s| ch.sem & (1 << s) == 0);
        if ch.hw_get < ch.log.len() && !blocked {
            let mut n = w.clone();
            let m = &mut n.ch[c];
            match m.log[ch.hw_get] {
                P::Seg { id, acq } => {
                    assert!(
                        m.pb & (1 << id) != 0,
                        "INVALID SCRIPT: the PBDMA fetched segment {id} before the guest wrote it"
                    );
                    m.executed.push(id);
                    m.hw_wait = acq;
                }
                P::GuestNop { id } => {
                    if id != ZERO_ID {
                        m.executed.push(id);
                    }
                    m.hw_wait = None;
                }
                P::Barrier => m.hw_wait = None,
                P::Witness { bytes } => {
                    m.hw_wait = None;
                    // Today's byte witness is a zero DMA_NOP word: no reference, no interrupt. Only the
                    // design's marker writes USERD.REFERENCE and carries the non-stall interrupt method.
                    if bytes && cfg.policy == Policy::Design {
                        m.reference += 1;
                        if !cfg.lag {
                            m.ref_visible = m.reference;
                        }
                        if cfg.irq {
                            m.ev = true;
                        }
                    }
                }
                P::Notify => {
                    m.hw_wait = None;
                    m.reference += 1;
                    if !cfg.lag {
                        m.ref_visible = m.reference;
                    }
                    if cfg.irq {
                        m.ev = true;
                    }
                }
            }
            m.hw_get += 1;
            m.popped = m.popped.max(m.hw_get);
            if !cfg.lag {
                m.userd_get = m.popped;
            }
            out.push(step(format!("hw{c}:begin#{}", ch.hw_get), n));
        }
        if cfg.lag && (ch.userd_get != ch.popped || ch.ref_visible != ch.reference) {
            let mut n = w.clone();
            n.ch[c].userd_get = ch.popped;
            n.ch[c].ref_visible = ch.reference;
            out.push(step(format!("hw{c}:GP_GET/REFERENCE visible"), n));
        }
    }
}

fn env_steps(cfg: &Cfg, w: &World, out: &mut Vec<Step>) {
    // act-thread lock windows on a record (the worker's `try_lock` fails inside one)
    for c in 0..w.ch.len() {
        if !w.ch[c].lock_held && w.locks_left > 0 && w.ch[c].alive() {
            let mut n = w.clone();
            n.ch[c].lock_held = true;
            n.locks_left -= 1;
            out.push(step(format!("env{c}:lock"), n));
        }
        if w.ch[c].lock_held {
            let mut n = w.clone();
            n.ch[c].lock_held = false;
            // Design: the holder of the record lock checks the durable demand (`Cell::retry`, set
            // by `enter()` BEFORE its `try_lock`) after releasing, and rings the token.
            if cfg.policy == Policy::Design && !cfg.m.no_lock_wake && n.ch[c].retry {
                n.ch[c].retry = false;
                n.ring(c, Cause::LockRelease);
            }
            out.push(step(format!("env{c}:unlock"), n));
        }
    }
    // windows of the pt / USERD admission lock: nobody rings on its release
    for c in 0..w.ch.len() {
        if !w.ch[c].pt_held && w.pts_left > 0 && w.ch[c].alive() {
            let mut n = w.clone();
            n.ch[c].pt_held = true;
            n.pts_left -= 1;
            out.push(step(format!("env{c}:pt lock"), n));
        }
        if w.ch[c].pt_held {
            let mut n = w.clone();
            n.ch[c].pt_held = false;
            out.push(step(format!("env{c}:pt unlock"), n));
        }
    }
    if cfg.timers {
        // time passes: Today's 250 ms active window ends
        if w.ch.iter().any(|c| c.poll_active) {
            let mut n = w.clone();
            for c in &mut n.ch {
                c.poll_active = false;
            }
            out.push(step("env:250ms elapse", n));
        }
        // a timeout can only win when nothing else is ready (epoll returns the ready fd at once)
        if w.parked && w.pc == Pc::Parked && !w.efd && !w.ch.iter().any(|c| c.ev) {
            match cfg.policy {
                Policy::Today => {
                    let mut n = w.clone();
                    n.parked = false;
                    n.efd = false;
                    for c in &mut n.ch {
                        c.poll_due = true;
                    }
                    n.pc = Pc::TickRefresh;
                    out.push(step("env:park timeout", n));
                }
                Policy::Design if cfg.backstop => {
                    // handled in worker_steps (a parked worker's own wake)
                }
                Policy::Design => {}
            }
        }
    }
}

/// End of a claimed turn: policy bookkeeping, then the token release (`TokenWord::release`).
fn finish(cfg: &Cfg, n: &mut World, need: Need) {
    let c = n.cur;
    match cfg.policy {
        Policy::Today => {
            // userpump.rs:1263 `self.tick = fetched != issued || poll.active || sync.waiting || ...`;
            // :1696 `self.tick = true` after a publish; Gate::Wait / no credit / Busy -> `tick = true`;
            // `again` -> `ring_internal(r.idx)` + `wake.signal()` (userpump.rs:2262).
            let ch = &mut n.ch[c];
            ch.tick = ch.issued != ch.fetched
                || ch.poll_active
                || ch.gate.0.waiting()
                || ch.retry
                || matches!(
                    need,
                    Need::Marker { .. } | Need::Popped { .. } | Need::Contended
                );
            if matches!(need, Need::Again) {
                n.ring(c, Cause::Again);
            }
        }
        Policy::Design => {
            let ch = &mut n.ch[c];
            ch.waiting = ul::wait_of(need);
            if matches!(need, Need::Again) {
                n.ring(c, Cause::Again);
            }
        }
    }
    // `user_ring_arm`: contended turns are counted, any other ending clears the run
    {
        let ch = &mut n.ch[c];
        if matches!(need, Need::Contended) {
            ch.contended_run = ch.contended_run.saturating_add(1);
        } else if !matches!(need, Need::Idle) || !ch.retry {
            ch.contended_run = 0;
        }
    }
    // release
    let ch = &mut n.ch[c];
    match ch.tok {
        Tok::Busy => {
            ch.tok = Tok::Idle;
            n.pc = Pc::Top0;
        }
        Tok::BusyRung => {
            if cfg.m.drop_busy_rung {
                ch.tok = Tok::Idle;
                n.pc = Pc::Top0;
            } else if n.rounds < REACT {
                ch.tok = Tok::Busy;
                n.rounds += 1;
                n.pc = Pc::T(T0);
            } else {
                ch.tok = Tok::Rung; // RepublishAndMoveOn
                n.moved = true;
                n.pc = Pc::Top0;
            }
        }
        Tok::Dead => n.pc = Pc::Top0,
        t => panic!("release in token state {t:?}"),
    }
    n.spin_left = cfg.spin;
}

/// Design: a wait the worker cannot satisfy right now; publish the batch's marker out of the
/// reserved slot first when a packet boundary allows it, so SOME interrupt will announce progress.
fn credit_wait(cfg: &Cfg, n: &mut World) {
    let c = n.cur;
    let entries = cfg.priv_entries;
    if cfg.policy == Policy::Design && !cfg.m.no_notify {
        let ch = &mut n.ch[c];
        if ch.unnotified {
            if !ch.open && ch.avail(entries) >= 1 {
                ch.publish(P::Notify, None);
                ch.refs_issued += 1;
                ch.marker_event = true;
            } else {
                ch.marker_event = false; // no marker possible: the wake is the spin / the safety net
            }
            ch.unnotified = false;
        }
    }
    let need = if n.ch[c].marker_event {
        Need::Marker { target: n.ch[c].refs_issued }
    } else {
        Need::Popped { target: n.ch[c].fetched + 1 }
    };
    finish(cfg, n, need);
}

/// Production `ChanPlane::user_ring_peek_one` (`userpump.rs`): look at one relay's not-yet-fd
/// state and ring it if its wait is satisfied, if its lock was busy, or if demand is pending.
/// The classification of what a BACKSTOP look finds is `kf_chan::user_loop::found`.
fn peek(n: &mut World, c: usize, why: ul::Peek, spin: u8, polls_put: bool) -> bool {
    let ch = &n.ch[c];
    if !ch.alive() {
        return false;
    }
    let backstop = why == ul::Peek::Backstop;
    let demand = ch.retry;
    // the production decision (`kf_chan::user_loop::look_needed`); the retired idle poll is the mutant
    if !ul::look_needed(why, demand, ch.waiting.is_some()) && !(backstop && polls_put) {
        return false;
    }
    let cause = match why {
        ul::Peek::Spin => Cause::Spin,
        ul::Peek::Event => Cause::Completion,
        ul::Peek::Backstop => Cause::Backstop,
    };
    // a relay that keeps finding a busy lock is re-rung by the spin / event looks only a bounded
    // number of times in a row; the safety net always may (`user_loop::may_retry`)
    let retry_ok = backstop || u32::from(ch.contended_run) < u32::from(spin);
    if ch.lock_held {
        // `try_lock` failed: ring, the turn handles the contention itself
        if retry_ok {
            n.ring(c, cause);
            return true;
        }
        return false;
    }
    if let Some(w) = ch.waiting {
        let pops = (ch.userd_get as u64) > ch.fetched;
        if !w.satisfied(ch.ref_visible, ch.userd_get as u64) && pops && matches!(w, Wait::Marker { .. }) {
            // still draining, but pops the guest's GP_GET has not shown yet: write them back
            n.ring(c, cause);
            return true;
        }
        if w.satisfied(ch.ref_visible, ch.userd_get as u64)
            && (retry_ok || !matches!(w, Wait::Contended))
        {
            if backstop {
                n.viol |= match ul::found(w) {
                    ul::Found::Work => V_BACKSTOP_FOUND_WORK,
                    ul::Found::Carried => V_BACKSTOP_CARRIED,
                    ul::Found::Contended => V_BACKSTOP_CONTENDED,
                };
            }
            n.ch[c].waiting = None;
            n.ring(c, cause);
            return true;
        }
    } else if backstop && polls_put && ch.put[ch.src] != ch.cursor {
        // MUTANT ONLY: the retired idle GP_PUT poll. GP_PUT ahead of the last doorbell is NORMAL;
        // only a doorbell announces it, so a timer that reads it and rings is a contract violation.
        n.viol |= V_PUT_POLL;
        n.ring(c, cause);
        return true;
    }
    if demand && retry_ok {
        if backstop {
            n.viol |= V_BACKSTOP_CONTENDED;
        }
        n.ring(c, cause);
        return true;
    }
    false
}

fn worker_steps(cfg: &Cfg, w: &World, out: &mut Vec<Step>) {
    let design = cfg.policy == Policy::Design;
    match w.pc {
        Pc::Top0 => {
            let mut n = w.clone();
            n.moved = false; // seen = seq
            n.pc = Pc::Top1;
            out.push(step("w:seen=seq", n));
        }
        Pc::Top1 => {
            let mut n = w.clone();
            if let Some(c) = (0..w.ch.len()).find(|&c| w.ch[c].tok == Tok::Rung) {
                // worker_pass: claim RUNG -> BUSY. The flag is cleared HERE, before any PUT read.
                n.ch[c].tok = Tok::Busy;
                n.cur = c;
                n.rounds = 0;
                if n.ch[c].cause == Cause::Tick {
                    n.viol |= V_TIMER_TURN;
                }
                n.pc = Pc::T(T0);
                out.push(step(format!("w:claim c{c} ({:?})", w.ch[c].cause), n));
            } else if design
                && ul::idle(
                    w.ch.iter()
                        .any(|c| c.alive() && (c.waiting.is_some() || c.retry)),
                    u32::from(w.spin_left),
                ) == ul::Idle::Spin
            {
                n.pc = Pc::Spin;
                out.push(step("w:to spin", n));
            } else {
                n.pc = Pc::Top2;
                out.push(step("w:to try_park", n));
            }
        }
        Pc::Spin => {
            // epoll(timeout=0) + a peek of the not-yet-fd words (GP_GET); BOUNDED by `spin_left`.
            let mut n = w.clone();
            n.spin_left -= 1;
            let mut any = false;
            for c in 0..n.ch.len() {
                any |= peek(&mut n, c, ul::Peek::Spin, cfg.spin, cfg.m.backstop_polls_put);
            }
            if any {
                n.spin_left = cfg.spin;
            }
            n.pc = if any { Pc::Top0 } else { Pc::Top1 };
            out.push(step("w:spin peek", n));
        }
        Pc::Top2 => {
            let mut n = w.clone();
            if w.moved && !cfg.m.no_seq_check {
                n.pc = Pc::Top0; // work arrived during the scan: rescan, never sleep
                out.push(step("w:try_park refused (seq moved)", n));
            } else {
                n.parked = true;
                n.pc = Pc::Parked;
                out.push(step("w:parked", n));
            }
        }
        Pc::Parked => {
            // doorbell eventfd
            if w.efd {
                let mut n = w.clone();
                n.efd = false;
                n.parked = false;
                n.pc = if design { Pc::Top0 } else { Pc::TickRefresh };
                out.push(step("w:wake(efd)", n));
            }
            // completion interrupt on the worker's epoll
            if w.ch.iter().any(|c| c.ev) {
                let mut n = w.clone();
                n.parked = false;
                n.pc = if design { Pc::Event } else { Pc::TickRefresh };
                if !design {
                    // Today: the engine event is consumed for the guest's interrupt; it wakes nothing in the relay
                    for c in &mut n.ch {
                        c.ev = false;
                    }
                }
                out.push(step("w:wake(event)", n));
            }
            // Design: the backstop (only if the configuration lets time pass at all)
            if design && cfg.backstop && cfg.timers && !w.efd && !w.ch.iter().any(|c| c.ev) {
                let mut n = w.clone();
                n.parked = false;
                n.efd = false;
                for c in 0..n.ch.len() {
                    peek(&mut n, c, ul::Peek::Backstop, cfg.spin, cfg.m.backstop_polls_put);
                }
                n.pc = Pc::Top0;
                out.push(step("w:wake(backstop)", n));
            }
        }
        Pc::TickRefresh => {
            // Today: `on_other(TICK_TAG)` -> `user_ring_refresh`: re-ring every pump with
            // `tick`, `retry` (pending) or a due `Poll` (userpump.rs:2323).
            let mut n = w.clone();
            for c in 0..n.ch.len() {
                let ch = &n.ch[c];
                if ch.alive() && (ch.retry || ch.tick || ch.poll_active || ch.poll_due) {
                    n.ch[c].poll_due = false;
                    n.ring(c, Cause::Tick);
                }
            }
            n.pc = Pc::Top0;
            out.push(step("w:tick refresh", n));
        }
        Pc::Event => {
            // Design: clear the event BEFORE peeking (`dispatch` ran `on_other` first), then look at
            // every relay (production: any engine event peeks them all).
            let mut n = w.clone();
            for c in &mut n.ch {
                c.ev = false;
            }
            for c in 0..n.ch.len() {
                peek(&mut n, c, ul::Peek::Event, cfg.spin, cfg.m.backstop_polls_put);
            }
            n.spin_left = cfg.spin;
            n.pc = Pc::Top0;
            out.push(step("w:handle event", n));
        }
        Pc::T(stage) => turn_step(cfg, w, stage, out),
    }
}

fn turn_step(cfg: &Cfg, w: &World, stage: u8, out: &mut Vec<Step>) {
    let design = cfg.policy == Policy::Design;
    let c = w.cur;
    let mut n = w.clone();
    let ch = &w.ch[c];
    let entries = cfg.priv_entries;
    match stage {
        T0 => {
            // userpump.rs:2229 `record.enter()` = `request()` (retry=true) THEN `try_lock`
            if ch.dead || ch.tok == Tok::Dead {
                finish(cfg, &mut n, Need::Idle);
                out.push(step("w:T0 dead", n));
            } else {
                n.ch[c].retry = true;
                n.pc = Pc::T(T0B);
                out.push(step("w:T0 request()", n));
            }
        }
        T0B => {
            if ch.lock_held {
                // `Some(false)`: `retry` stays set -- the durable demand. Who rings: the holder's
                // completion (`user_ring_wake`, Design), the look at pending demand (spin /
                // safety net, Design), the tick (Today).
                finish(cfg, &mut n, Need::Idle);
                out.push(step("w:T0 record busy", n));
            } else {
                n.ch[c].retry = false; // `record.retry.swap(false)`
                n.pc = Pc::T(T1);
                out.push(step("w:T0 entered", n));
            }
        }
        T1 => {
            // observe the private GP_GET and REFERENCE (userpump.rs: the turn's first read)
            if design {
                n.ch[c].ref_seen = ch.ref_seen.max(ch.ref_visible);
            }
            let ug = ch.userd_get as u64;
            let delta = (ug - ch.fetched) as usize;
            let m = &mut n.ch[c];
            for _ in 0..delta {
                if let Some(Some((g, get))) = m.pend.pop_front() {
                    if g == m.gener {
                        m.wb = Some((g, get));
                    }
                }
            }
            m.fetched = ug;
            n.pc = Pc::T(if m.wb.is_some() { T1B } else { T2 });
            out.push(step("w:T1 observe GP_GET", n));
        }
        T1B => {
            // userpump.rs:1207 the guest's GP_GET write; `AccessError::Busy` -> tick, return
            if ch.pt_held {
                // `AccessError::Busy` from the USERD admission / pt lock: nothing announces its release
                finish(cfg, &mut n, Need::Contended);
                out.push(step("w:T1b GP_GET write busy", n));
            } else {
                let (_, get) = ch.wb.unwrap();
                n.ch[c].guest_get[ch.src] = get;
                n.ch[c].wb = None;
                n.pc = Pc::T(T2);
                out.push(step("w:T1b write guest GP_GET", n));
            }
        }
        T2 => {
            // userpump.rs:1262 read GP_PUT -- AFTER the claim cleared the flag
            let put = ch.put[ch.src];
            if put as usize >= R {
                // USER_RING_PUT_BOUND: a bounded refusal; the record is cancelled, nothing re-arms
                n.viol |= V_PUT_BOUND;
                n.ch[c].dead = true;
                n.ch[c].waiting = None;
                finish(cfg, &mut n, Need::Idle);
                out.push(step("w:T2 PUT out of bounds -> refused", n));
            } else if put == ch.cursor {
                if design && !cfg.m.no_notify && ch.unnotified {
                    // a batch with no marker yet: end it with one when a packet boundary allows
                    if ch.open {
                        n.ch[c].unnotified = false;
                        n.ch[c].marker_event = false;
                        let t = ch.issued;
                        finish(cfg, &mut n, Need::Popped { target: t });
                        out.push(step("w:T2 idle, packet open: no marker possible", n));
                    } else if ch.avail(entries) >= 1 {
                        n.ch[c].publish(P::Notify, None);
                        n.ch[c].refs_issued += 1;
                        n.ch[c].unnotified = false;
                        n.ch[c].marker_event = true;
                        let t = n.ch[c].refs_issued;
                        finish(cfg, &mut n, Need::Marker { target: t });
                        out.push(step("w:T2 idle: publish marker", n));
                    } else {
                        credit_wait(cfg, &mut n);
                        out.push(step("w:T2 idle: no credit for the marker", n));
                    }
                } else if design && ch.fetched < ch.issued {
                    // published work in flight: the last marker's processing frees the guest's ring
                    let need = if ch.marker_event {
                        Need::Marker { target: ch.refs_issued }
                    } else {
                        Need::Popped { target: ch.issued }
                    };
                    finish(cfg, &mut n, need);
                    out.push(step("w:T2 idle (work in flight)", n));
                } else {
                    finish(cfg, &mut n, Need::Idle);
                    out.push(step("w:T2 idle", n));
                }
            } else {
                n.ch[c].poll_active = true; // Poll::turned(now, new_work)
                n.pc = Pc::T(T3);
                out.push(step("w:T2 PUT ahead of cursor", n));
            }
        }
        T3 => {
            n.pc = Pc::T(T4);
            out.push(step("w:T3 read GP entry", n));
        }
        T4 => {
            let e = ch.ring[ch.src][ch.cursor as usize].unwrap_or(Ge {
                id: ZERO_ID,
                ..nop(ZERO_ID)
            });
            let raw = if e.sync { 1u64 << 63 } else { 0 };
            // The proof the gate waits on. Design: markers PROCESSED (USERD.REFERENCE). Today: GP_GET
            // (`fetch.fetched()`), the retired proof -- and the `get_proof` mutant of the design.
            let proof = if design && !cfg.m.get_proof { ch.ref_seen } else { ch.fetched };
            // a SYNC_WAIT entry behind an open packet cannot be gated (no witness fits): Design reads it at once
            let ungated = design && e.sync && ch.open && !cfg.m.bytes_witness;
            let mut gate = ch.gate.0;
            let verdict = if cfg.m.no_gate || ungated {
                Gate::Proceed
            } else {
                gate.entry(raw, u64::from(ch.gener), ch.cursor, proof)
            };
            n.ch[c].gate = Gt(gate);
            match verdict {
                Gate::Proceed => {
                    n.pc = Pc::T(T5);
                    out.push(step("w:T4 gate: proceed", n));
                }
                Gate::Wait => {
                    // the marker's processing writes the reference; its interrupt announces it
                    let t = ch.gate_target;
                    finish(cfg, &mut n, Need::Marker { target: t });
                    out.push(step("w:T4 gate: wait", n));
                }
                Gate::PublishBarrier => {
                    if ch.avail(entries) < 2 {
                        credit_wait(cfg, &mut n);
                        out.push(step("w:T4 gate: no credit for barrier", n));
                    } else {
                        if ch.open {
                            // only reachable by the `bytes_witness` mutant: a marker inside an open packet
                            n.viol |= V_BYTES_IN_OPEN_PACKET;
                        }
                        let m = &mut n.ch[c];
                        m.publish(P::Barrier, None);
                        m.publish(P::Witness { bytes: true }, None);
                        m.refs_issued += 1;
                        m.gate.0.published(if cfg.policy == Policy::Design && !cfg.m.get_proof {
                            m.refs_issued
                        } else {
                            m.issued
                        });
                        m.gate_target = m.refs_issued;
                        m.unnotified = false;
                        m.marker_event = true;
                        let t = if cfg.policy == Policy::Design { m.gate_target } else { m.issued };
                        finish(cfg, &mut n, Need::Marker { target: t });
                        out.push(step("w:T4 gate: barrier+marker", n));
                    }
                }
            }
        }
        T5 => {
            let e = ch.ring[ch.src][ch.cursor as usize].unwrap_or(Ge {
                id: ZERO_ID,
                ..nop(ZERO_ID)
            });
            // Design keeps one slot back for the batch's marker
            let need_credit = if design && !cfg.m.no_notify { 2 } else { 1 };
            if ch.avail(entries) < need_credit {
                credit_wait(cfg, &mut n);
                out.push(step("w:T5 no credit", n));
                return;
            }
            let next = (ch.cursor + 1) % R as u32;
            let receipt = Some((ch.gener, next));
            match e.kind {
                Kind::Nop => n.ch[c].publish(P::GuestNop { id: e.id }, receipt),
                Kind::Seg | Kind::Trigger => {
                    // read the guest pushbuffer
                    if ch.pb & (1 << e.id) == 0 {
                        n.viol |= V_STALE;
                        n.ch[c].pb |= 1 << e.id; // report once; the model reads on
                    }
                    n.ch[c].publish(
                        P::Seg {
                            id: e.id,
                            acq: e.acq,
                        },
                        receipt,
                    );
                    n.ch[c].open = e.open;
                }
            }
            let m = &mut n.ch[c];
            m.unnotified = true;
            if e.id != ZERO_ID {
                m.relayed.push(e.id);
            }
            if e.kind == Kind::Trigger {
                // migrate(): new source, cursor 0, generation + 1, the old write-back dropped;
                // the private ring (and its credits) continue
                m.src = 1;
                m.cursor = 0;
                m.gener += 1;
                m.wb = None;
            } else {
                m.cursor = next;
            }
            finish(cfg, &mut n, Need::Again);
            out.push(step(format!("w:T5 published {}", e.id), n));
        }
        _ => unreachable!(),
    }
}

/// ★ MEASURED (bare metal, RTX 4070 / 595.91.07): `GP_GET` runs AHEAD of processing -- with an ACQUIRE
/// stalling entry 0 it already covered all three published entries. The model takes the worst case:
/// every published entry is popped at once (its visibility to the CPU still lags if `lag`).
fn normalize(cfg: &Cfg, w: &mut World) {
    if !cfg.prefetch {
        return;
    }
    for ch in &mut w.ch {
        let to = ch.log.len().min(ch.hw_get.saturating_add(cfg.pop_depth));
        if ch.popped < to {
            ch.popped = to;
            if !cfg.lag {
                ch.userd_get = ch.popped;
            }
        }
    }
}

fn successors(cfg: &Cfg, w: &World) -> Vec<Step> {
    let mut out = Vec::new();
    guest_steps(cfg, w, &mut out);
    hw_steps(cfg, w, &mut out);
    env_steps(cfg, w, &mut out);
    worker_steps(cfg, w, &mut out);
    for s in &mut out {
        normalize(cfg, &mut s.w);
    }
    out.retain(|s| s.w != *w);
    out
}

// ------------------------------------------------------------------------------------------------
// invariants and the goal

fn invariant(cfg: &Cfg, w: &World) -> u32 {
    let mut v = w.viol;
    for (c, ch) in w.ch.iter().enumerate() {
        let exp = &cfg.chans[c].expected;
        if !(ch.executed.len() <= exp.len() && ch.executed[..] == exp[..ch.executed.len()]) {
            v |= V_ORDER;
        }
        if ch.log.len() > 3 * exp.len() + 4 * R {
            v |= V_UNBOUNDED;
        }
    }
    if w.pc == Pc::Parked {
        // the worker must never be blocked with a doorbell flag or a pending resume
        if w.ch
            .iter()
            .any(|c| c.tok == Tok::Rung || c.tok == Tok::BusyRung)
            && !w.efd
        {
            v |= V_PARKED_WITH_WORK;
        }
    }
    v
}

fn goal_failure(cfg: &Cfg, w: &World) -> Option<String> {
    if !matches!(w.pc, Pc::Parked) {
        return Some(format!("worker stuck at {:?}", w.pc));
    }
    for (c, ch) in w.ch.iter().enumerate() {
        let spec = &cfg.chans[c];
        if ch.executed.len() < spec.must_relay && !ch.dead {
            return Some(format!(
                "chan {c}: the PBDMA executed {:?} but a doorbell covered {} entries ({:?}); relay cursor {} src {} put {:?} waiting {:?} gate_waiting {} tok {:?}",
                ch.executed,
                spec.must_relay,
                &spec.expected[..spec.must_relay],
                ch.cursor,
                ch.src,
                ch.put,
                ch.waiting,
                ch.gate.0.waiting(),
                ch.tok
            ));
        }
        if ch.gpc < spec.script.len() && !spec.may_block {
            return Some(format!(
                "chan {c}: the guest is stuck at op {} ({:?}) -- ring full: the relay never wrote GP_GET back?",
                ch.gpc, spec.script[ch.gpc]
            ));
        }
    }
    None
}

/// What an exhaustive exploration visited.
#[derive(Debug)]
struct Explored {
    states: usize,
    /// Every distinct step label taken (coverage: the races the tests claim to cover were reached).
    labels: std::collections::HashSet<String>,
}

#[derive(Debug)]
struct Counterexample {
    why: String,
    trace: Vec<String>,
    states: usize,
}

fn key(w: &World) -> u128 {
    use std::collections::hash_map::DefaultHasher;
    let s = format!("{w:?}");
    let mut a = DefaultHasher::new();
    s.hash(&mut a);
    let mut b = DefaultHasher::new();
    (&s, 0x9e37_79b9_7f4a_7c15_u64).hash(&mut b);
    (u128::from(a.finish()) << 64) | u128::from(b.finish())
}

/// Breadth-first over every interleaving. `allow` = violation bits that are counted, not errors.
fn explore(cfg: &Cfg, allow: u32) -> Result<Explored, Counterexample> {
    let start = World::new(cfg);
    let mut seen: HashMap<u128, (u128, String)> = HashMap::new();
    let mut q: VecDeque<World> = VecDeque::new();
    seen.insert(key(&start), (0, String::new()));
    q.push_back(start);
    let mut states = 0usize;
    let mut labels = std::collections::HashSet::new();
    let trace_of = |seen: &HashMap<u128, (u128, String)>, mut k: u128| {
        let mut t = Vec::new();
        while let Some((p, l)) = seen.get(&k) {
            if l.is_empty() {
                break;
            }
            t.push(l.clone());
            k = *p;
        }
        t.reverse();
        t
    };
    while let Some(w) = q.pop_front() {
        states += 1;
        assert!(
            states < 3_000_000,
            "state space exceeded 3M: shrink the scenario"
        );
        let k = key(&w);
        let v = invariant(cfg, &w) & !allow;
        if v != 0 {
            return Err(Counterexample {
                why: viol_names(v).join("; "),
                trace: trace_of(&seen, k),
                states,
            });
        }
        let succ = successors(cfg, &w);
        if succ.is_empty() && cfg.liveness {
            if let Some(why) = goal_failure(cfg, &w) {
                return Err(Counterexample {
                    why: format!("DEADLOCK: {why}"),
                    trace: trace_of(&seen, k),
                    states,
                });
            }
        }
        for s in succ {
            if !labels.contains(&s.label) {
                labels.insert(s.label.clone());
            }
            let nk = key(&s.w);
            if !seen.contains_key(&nk) {
                seen.insert(nk, (k, s.label));
                q.push_back(s.w);
            }
        }
    }
    Ok(Explored { states, labels })
}

fn report(name: &str, r: &Result<Explored, Counterexample>) -> String {
    match r {
        Ok(e) => format!("{name}: {} states, no violation", e.states),
        Err(e) => format!(
            "{name}: {} after {} states\n  counterexample ({} steps):\n    {}",
            e.why,
            e.states,
            e.trace.len(),
            e.trace.join("\n    ")
        ),
    }
}

/// Asserts the property holds; on failure prints the shortest counterexample.
fn holds(name: &str, cfg: &Cfg, allow: u32) {
    let r = explore(cfg, allow);
    assert!(r.is_ok(), "{}", report(name, &r));
}
/// Asserts the checker FINDS a violation (it must not be vacuous).
fn finds(name: &str, cfg: &Cfg, allow: u32, needles: &[&str]) {
    let r = explore(cfg, allow);
    match &r {
        Err(e) if needles.iter().any(|n| e.why.contains(n)) => {}
        other => panic!(
            "{name}: expected a violation containing one of {needles:?}, got:\n{}",
            report(name, other)
        ),
    }
}

// ------------------------------------------------------------------------------------------------
// scenarios

fn spec(script: Vec<Op>, expected: Vec<u8>, must_relay: usize) -> ChanSpec {
    ChanSpec {
        script,
        expected,
        must_relay,
        may_block: false,
    }
}
use Op::*;

/// write GP_PUT, then ring.
fn s_put_then_ring() -> Vec<ChanSpec> {
    vec![spec(vec![Push(seg(1)), SetPut, Ring], vec![1], 1)]
}
/// several writes, ONE ring at the end.
fn s_batched() -> Vec<ChanSpec> {
    vec![spec(
        vec![Push(seg(1)), Push(seg(2)), Push(seg(3)), SetPut, Ring],
        vec![1, 2, 3],
        3,
    )]
}
/// GP_PUT runs ahead of the last doorbell (normal): the next doorbell covers it.
fn s_put_ahead() -> Vec<ChanSpec> {
    vec![spec(
        vec![
            Push(seg(1)),
            SetPut,
            Ring,
            Push(seg(2)),
            SetPut,
            Push(seg(3)),
            SetPut,
            Ring,
        ],
        vec![1, 2, 3],
        3,
    )]
}
/// GP_PUT ahead of the LAST doorbell: entry 2 is not covered by any ring (optional).
fn s_put_ahead_unrung() -> Vec<ChanSpec> {
    vec![spec(
        vec![Push(seg(1)), SetPut, Ring, Push(seg(2)), SetPut],
        vec![1, 2],
        1,
    )]
}
/// many rings for one PUT (coalescing).
fn s_coalesce() -> Vec<ChanSpec> {
    vec![spec(
        vec![
            Push(seg(1)),
            SetPut,
            Ring,
            Ring,
            Ring,
            Push(seg(2)),
            SetPut,
            Ring,
        ],
        vec![1, 2],
        2,
    )]
}
/// A SYNC_WAIT entry whose pushbuffer is written LATE, gated by an acquire in the preceding
/// segment; a second doorbell lands while the gate drains; the semaphore is released last.
fn s_gate_late_pb() -> Vec<ChanSpec> {
    vec![spec(
        vec![
            Push(seg(1).acq(0)),
            Push(seg(2).sync().late()),
            SetPut,
            Ring,
            Push(seg(3)),
            SetPut,
            Ring,
            Fill(2),
            Release(0),
        ],
        vec![1, 2, 3],
        3,
    )]
}
/// the same with the packet of segment 1 left OPEN across the SYNC_WAIT entry: the witness must
/// carry no byte, so no completion interrupt exists for the gate.
fn s_gate_carried() -> Vec<ChanSpec> {
    vec![spec(
        vec![
            Push(seg(1).acq(0).open()),
            Push(seg(2).sync().late()),
            SetPut,
            Ring,
            Fill(2),
            Release(0),
        ],
        vec![1, 2],
        2,
    )]
}
/// a LENGTH==0 SYNC_WAIT entry in the middle (first, middle, last, back to back).
fn s_zero_length() -> Vec<ChanSpec> {
    vec![spec(
        vec![
            Push(nop(1).sync()),
            Push(seg(2).acq(0)),
            Push(nop(3).sync()),
            SetPut,
            Ring,
            Release(0),
            Push(nop(4).sync()),
            Push(seg(5).sync()),
            SetPut,
            Ring,
        ],
        vec![1, 2, 3, 4, 5],
        5,
    )]
}
/// more entries than ring slots: the guest waits for the relay's GP_GET write-back.
fn s_wrap() -> Vec<ChanSpec> {
    vec![spec(
        vec![
            Push(seg(1)),
            Push(seg(2)),
            Push(seg(3)),
            SetPut,
            Ring,
            Push(seg(4)),
            Push(seg(5)),
            SetPut,
            Ring,
            Push(seg(6)),
            SetPut,
            Ring,
        ],
        vec![1, 2, 3, 4, 5, 6],
        6,
    )]
}
/// the migration: the trigger entry, then the new ring (the guest may ring for it before the
/// relay has executed the trigger).
fn s_migration() -> Vec<ChanSpec> {
    vec![spec(
        vec![
            Push(seg(1)),
            Push(trigger(2)),
            SetPut,
            Ring,
            UseNew,
            Push(seg(3)),
            SetPut,
            Ring,
            Push(seg(4)),
            SetPut,
            Ring,
        ],
        vec![1, 2, 3, 4],
        4,
    )]
}
/// the SAME slot is a SYNC_WAIT entry twice, one ring lap apart, with plain entries between (the
/// 4-slot ring wraps): the second one's pushbuffer is written late, behind the acquire of the entry
/// before it, and must not be read before the drain.
fn s_lap_sync() -> Vec<ChanSpec> {
    vec![spec(
        vec![
            Push(seg(1).acq(0)),
            Push(seg(2).sync().late()),
            Push(seg(3)),
            SetPut,
            Ring,
            Fill(2),
            Release(0),
            Push(seg(4).acq(1)),
            Push(seg(5)),
            Push(seg(6).sync().late()),
            SetPut,
            Ring,
            Fill(6),
            Release(1),
        ],
        vec![1, 2, 3, 4, 5, 6],
        6,
    )]
}
/// migration right behind a gated entry.
fn s_migration_gated() -> Vec<ChanSpec> {
    vec![spec(
        vec![
            Push(seg(1).acq(0)),
            Push(trigger(2).sync().late()),
            SetPut,
            Ring,
            UseNew,
            Push(seg(3)),
            SetPut,
            Ring,
            Fill(2),
            Release(0),
        ],
        vec![1, 2, 3],
        3,
    )]
}
/// a gate nobody releases (the guest's semaphore never comes) beside a normal channel.
fn s_two_channels_one_gated_forever() -> Vec<ChanSpec> {
    vec![
        ChanSpec {
            script: vec![
                Push(seg(1).acq(0)),
                Push(seg(2).sync().late()),
                SetPut,
                Ring,
            ],
            expected: vec![1, 2],
            must_relay: 1,
            may_block: true,
        },
        spec(
            vec![Push(seg(11)), SetPut, Ring, Push(seg(12)), SetPut, Ring],
            vec![11, 12],
            2,
        ),
    ]
}

// ------------------------------------------------------------------------------------------------
// (a) liveness with timers that NEVER fire, safety, no timer-caused turns

/// Every scenario of the doorbell contract, checked for a policy.
fn contract_scenarios() -> Vec<(&'static str, Vec<ChanSpec>)> {
    vec![
        ("put_then_ring", s_put_then_ring()),
        ("batched_one_ring", s_batched()),
        ("put_ahead_of_ring", s_put_ahead()),
        ("coalesced_rings", s_coalesce()),
        ("gate_late_pushbuffer", s_gate_late_pb()),
        ("zero_length_entries", s_zero_length()),
        ("ring_wrap", s_wrap()),
        ("migration", s_migration()),
        ("migration_behind_gate", s_migration_gated()),
        (
            "two_channels_one_gated_forever",
            s_two_channels_one_gated_forever(),
        ),
    ]
}

fn run_contract(
    policy: Policy,
    name_prefix: &str,
    allow: u32,
    mut tweak: impl FnMut(&mut Cfg),
) -> Vec<(&'static str, String)> {
    let mut failures = Vec::new();
    for (name, chans) in contract_scenarios() {
        let mut cfg = Cfg::new(policy, chans);
        tweak(&mut cfg);
        let r = explore(&cfg, allow);
        if let Ok(e) = &r {
            eprintln!("{name_prefix}/{name}: {} states", e.states);
        }
        if r.is_err() {
            failures.push((name, report(&format!("{name_prefix}/{name}"), &r)));
        }
    }
    failures
}

fn no_failures(f: &[(&'static str, String)]) {
    assert!(
        f.is_empty(),
        "{} scenario(s) fail:\n{}",
        f.len(),
        f.iter()
            .map(|(_, r)| r.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The RETIRED loop (`Policy::Today`: `worker.rs` park/`TICK_TAG` refresh, `Pump::tick`, `Poll`,
/// as committed at 77eaf7620) is rejected by the same checker, exactly at the scenarios whose
/// waits it polled: `failing` names them, `why` is the violation each one reports.
fn rejected(f: &[(&'static str, String)], failing: &[(&str, &str)]) {
    let got: Vec<(&str, &str)> = f
        .iter()
        .map(|(n, r)| {
            let why = failing.iter().find(|(m, _)| m == n).map_or("", |(_, w)| *w);
            assert!(why.is_empty() || r.contains(why), "{n}: expected {why:?} in\n{r}");
            (*n, why)
        })
        .collect();
    assert_eq!(got, failing, "the retired loop is rejected at exactly these scenarios");
}

/// ★ RULE 1: correctness must not depend on timing. With timers that never fire (no tick, no park
/// timeout, no backstop), every doorbell-covered entry is relayed, nothing is read early, nothing
/// is dropped or replayed. The retired loop FAILED this at 77eaf7620 (the gate/credit/lock waits
/// and the guest `GP_GET` write-back were polled by the 1 ms tick): the checker still rejects it.
#[test]
fn the_retired_loop_deadlocks_without_timers_exactly_where_it_polled() {
    let f = run_contract(Policy::Today, "retired/no-timers", V_TIMER_TURN, |c| {
        c.timers = false;
        c.priv_entries = 8;
    });
    // With GP_GET running ahead of processing (measured), the retired GP_GET gate proof fails on the
    // late pushbuffers (STALE_READ) before liveness is the question; the scenarios without late bytes
    // still show the tick-polled wait (DEADLOCK).
    rejected(
        &f,
        &[
            ("gate_late_pushbuffer", "STALE_READ"),
            ("zero_length_entries", "DEADLOCK"),
            ("migration_behind_gate", "STALE_READ"),
            ("two_channels_one_gated_forever", "STALE_READ"),
        ],
    );
}

/// The design satisfies the same contract (so the property is satisfiable and the test above is
/// not shaped by an impossible requirement).
#[test]
fn design_a_relay_is_live_without_any_timer() {
    let f = run_contract(Policy::Design, "design/no-timers", 0, |c| {
        c.timers = false;
        c.spin = 1;
    });
    no_failures(&f);
}

/// ★ RULE 2: no turn is caused by a timer. With time allowed to pass, the retired loop's `TICK_TAG`
/// refresh rang a pump nobody rang (the 1 ms poll, the 250 ms active window, the 20 ms idle poll)
/// in EVERY scenario. The design's only timer is the backstop, and with delivered interrupts it
/// never finds work.
#[test]
fn the_retired_loop_ran_turns_nobody_rang_when_time_passed() {
    let f = run_contract(Policy::Today, "retired/timers", 0, |c| {
        c.timers = true;
        c.liveness = false;
    });
    rejected(
        &f,
        &[
            ("put_then_ring", "TIMER_TURN"),
            ("batched_one_ring", "TIMER_TURN"),
            ("put_ahead_of_ring", "TIMER_TURN"),
            ("coalesced_rings", "TIMER_TURN"),
            ("gate_late_pushbuffer", "TIMER_TURN"),
            ("zero_length_entries", "TIMER_TURN"),
            ("ring_wrap", "TIMER_TURN"),
            ("migration", "TIMER_TURN"),
            ("migration_behind_gate", "TIMER_TURN"),
            ("two_channels_one_gated_forever", "TIMER_TURN"),
        ],
    );
}

#[test]
fn design_b_no_turn_is_caused_by_a_timer_and_the_backstop_finds_nothing() {
    let f = run_contract(Policy::Design, "design/timers+backstop", 0, |c| {
        c.timers = true;
        c.backstop = true;
        c.liveness = false;
    });
    no_failures(&f);
}

/// ★ THE MEASURED DEFECT as a model result. The retired gate proved "the barrier's wait finished" by
/// `GP_GET` passing the witness. On the hardware `GP_GET` runs AHEAD of processing (`kf-syncwait-stress
/// --families barrier`: with an ACQUIRE stalling entry 0, `GP_GET` covered all 3 entries, the segment
/// behind the barrier had not executed and `USERD.REFERENCE` was 0), so the relay read pushbuffers the
/// guest had not yet written.
#[test]
fn the_retired_gp_get_proof_read_a_pushbuffer_before_the_drain() {
    let mut cfg = Cfg::new(Policy::Today, s_gate_late_pb());
    cfg.timers = true; // so the retired loop can make progress at all; only the safety bits matter
    cfg.liveness = false;
    finds("retired/gp-get-proof", &cfg, V_TIMER_TURN, &["STALE_READ"]);
    // the same proof inside the design (a mutant) is found too
    let mut cfg = Cfg::new(Policy::Design, s_gate_late_pb());
    cfg.m.get_proof = true;
    cfg.liveness = false;
    finds("design/gp-get-proof-mutant", &cfg, 0, &["STALE_READ"]);
    // ... and with a sequential GP_GET (the reading the retired gate assumed) it was fine
    let mut cfg = Cfg::new(Policy::Design, s_gate_late_pb());
    cfg.m.get_proof = true;
    cfg.prefetch = false;
    cfg.liveness = false;
    holds("design/gp-get-proof-sequential-get", &cfg, 0);
}

/// ★ Packet open across a SYNC_WAIT entry: no witness fits inside the packet and a control entry proves
/// nothing, so the entry is not gated -- the relay reads it at once. With the bytes present (the
/// hardware does not delay such a segment either) that is correct and needs no wake at all.
#[test]
fn design_a_sync_wait_behind_an_open_packet_is_read_ungated_and_needs_no_wake() {
    let chans = vec![spec(
        vec![Push(seg(1).acq(0).open()), Push(seg(2).sync()), SetPut, Ring, Release(0)],
        vec![1, 2],
        2,
    )];
    let mut cfg = Cfg::new(Policy::Design, chans);
    cfg.timers = false;
    cfg.spin = 0;
    holds("design/carried-ungated", &cfg, 0);
}

/// The DOCUMENTED LIMITATION: if the guest writes the bytes of such an entry LATE the relay reads them
/// early (no proof exists inside an open packet). Pinned so that it is a known, named gap -- the
/// counter `carried_ungated` shows whether it is ever reached -- and not a silent one.
#[test]
fn design_a_sync_wait_behind_an_open_packet_cannot_protect_late_bytes() {
    let mut cfg = Cfg::new(Policy::Design, s_gate_carried());
    cfg.liveness = false;
    finds("design/carried-late", &cfg, 0, &["STALE_READ"]);
    // the mutant that puts a marker inside the open packet is the original Xid-32 PBENTRY
    cfg.m.bytes_witness = true;
    finds("design/carried-marker-in-packet", &cfg, 0, &["BYTES_IN_OPEN_PACKET", "STALE_READ"]);
}

/// ★ A lost / never-armed interrupt: progress comes only from the backstop, and every firing that
/// finds work is the loud counter.
#[test]
fn design_a_lost_interrupt_is_covered_by_the_backstop_and_counted() {
    let mut cfg = Cfg::new(Policy::Design, s_gate_late_pb());
    cfg.irq = false;
    cfg.timers = false;
    finds("design/lost-irq/no-backstop", &cfg, 0, &["DEADLOCK"]);
    cfg.timers = true;
    cfg.backstop = true;
    holds("design/lost-irq/backstop", &cfg, V_BACKSTOP_FOUND_WORK);
    finds(
        "design/lost-irq/counter-reached",
        &cfg,
        0,
        &["BACKSTOP_FOUND_WORK"],
    );
}

/// GP_GET becoming visible AFTER its interrupt (an inferred hazard, not measured): the event is a
/// hint, the peek is the proof; progress then needs the spin or the backstop.
#[test]
fn design_get_visibility_lag_after_the_interrupt_needs_the_backstop() {
    let mut cfg = Cfg::new(Policy::Design, s_gate_late_pb());
    cfg.lag = true;
    cfg.spin = 1;
    finds("design/lag/no-backstop", &cfg, 0, &["DEADLOCK"]);
    cfg.timers = true;
    cfg.backstop = true;
    holds(
        "design/lag/backstop",
        &cfg,
        V_BACKSTOP_FOUND_WORK | V_BACKSTOP_CARRIED,
    );
}

/// Without timers the spin is not what makes the design live: with spin 0 every non-carried
/// scenario still completes (the spin is a latency optimisation only).
#[test]
fn design_never_depends_on_the_spin() {
    let f = run_contract(Policy::Design, "design/spin=0", 0, |c| {
        c.timers = false;
        c.spin = 0;
    });
    no_failures(&f);
}

/// Private-ring credit shortage (3 credits): the relay waits for hardware progress by an event.
#[test]
fn design_credit_waits_are_event_driven() {
    let f = run_contract(Policy::Design, "design/priv=4", 0, |c| {
        c.priv_entries = 4;
        c.timers = false;
    });
    no_failures(&f);
}
#[test]
fn the_retired_loop_polled_private_ring_credit_too() {
    let f = run_contract(Policy::Today, "retired/priv=4", V_TIMER_TURN, |c| {
        c.priv_entries = 4;
        c.timers = false;
    });
    rejected(
        &f,
        &[
            ("gate_late_pushbuffer", "STALE_READ"),
            ("zero_length_entries", "DEADLOCK"),
            ("migration_behind_gate", "STALE_READ"),
            ("two_channels_one_gated_forever", "STALE_READ"),
        ],
    );
}

/// A busy record lock (the act thread birthing/freeing/migrating) defers the turn; whoever holds it
/// wakes the worker on release. The retired loop: the deferred doorbell waited for the tick.
#[test]
fn the_retired_loop_waited_for_the_tick_after_a_busy_record_lock() {
    let mut cfg = Cfg::new(Policy::Today, s_batched());
    cfg.lock_windows = 1;
    cfg.timers = false;
    finds("retired/lock", &cfg, 0, &["DEADLOCK"]);
}
#[test]
fn design_a_turn_deferred_by_a_busy_lock_is_resumed_by_an_event() {
    let mut cfg = Cfg::new(Policy::Design, s_batched());
    cfg.lock_windows = 1;
    cfg.timers = false;
    cfg.spin = 0; // the holder's wake alone (the spin is only a latency optimisation)
    holds("design/lock", &cfg, 0);
    cfg.m.no_lock_wake = true;
    finds("design/lock/mutant", &cfg, 0, &["DEADLOCK"]);
    // with the spin the durable demand (`Cell::retry`) is looked at and rung even without the wake
    cfg.spin = 1;
    holds("design/lock/spin", &cfg, 0);
}

/// The pt / USERD admission lock is busy at the guest's `GP_GET` write-back and nothing announces
/// its release: the relay waits `Wait::Contended` -- satisfied by looking again -- so liveness
/// needs the bounded spin to coincide with the release or else the safety net, whose firing is the
/// separate `BACKSTOP_CONTENDED` counter. Documented, counted, and the only place left where a
/// release is not an event.
#[test]
fn design_pt_admission_contention_is_spin_then_the_counted_backstop() {
    let mut cfg = Cfg::new(Policy::Design, s_batched());
    cfg.pt_windows = 1;
    cfg.timers = false;
    cfg.spin = 1;
    finds("design/pt/no-backstop", &cfg, 0, &["DEADLOCK"]);
    cfg.timers = true;
    cfg.backstop = true;
    holds("design/pt/backstop", &cfg, V_BACKSTOP_CONTENDED);
    finds(
        "design/pt/counter-reached",
        &cfg,
        0,
        &["BACKSTOP_CONTENDED"],
    );
}

// ------------------------------------------------------------------------------------------------
// the checker has teeth: every broken variant of the design is found

#[test]
fn mutant_lost_wakeup_is_found() {
    let mut cfg = Cfg::new(Policy::Design, s_put_then_ring());
    cfg.m.no_seq_check = true;
    finds(
        "mutant/no_seq_check",
        &cfg,
        0,
        &["DEADLOCK", "PARKED_WITH_WORK"],
    );
}
#[test]
fn mutant_a_ring_during_a_turn_is_lost_is_found() {
    let mut cfg = Cfg::new(Policy::Design, s_coalesce());
    cfg.m.drop_busy_rung = true;
    finds("mutant/drop_busy_rung", &cfg, 0, &["DEADLOCK"]);
}
#[test]
fn mutant_byte_witness_inside_an_open_packet_is_found() {
    let mut cfg = Cfg::new(Policy::Design, s_gate_carried());
    cfg.m.bytes_witness = true;
    cfg.timers = true;
    cfg.backstop = true;
    cfg.liveness = false;
    finds(
        "mutant/bytes_witness",
        &cfg,
        V_BACKSTOP_CARRIED,
        &["BYTES_IN_OPEN_PACKET"],
    );
}
#[test]
fn mutant_no_gate_reads_the_pushbuffer_early_is_found() {
    let mut cfg = Cfg::new(Policy::Design, s_gate_late_pb());
    cfg.m.no_gate = true;
    cfg.liveness = false;
    finds("mutant/no_gate", &cfg, 0, &["STALE_READ"]);
}
#[test]
fn mutant_no_completion_event_is_found() {
    let mut cfg = Cfg::new(Policy::Design, s_gate_late_pb());
    cfg.m.no_notify = true;
    cfg.irq = false;
    finds("mutant/no_notify", &cfg, 0, &["DEADLOCK"]);
}

/// ★ FAILED at 77eaf7620, in the PRODUCTION primitive (`SyncGate::cleared` is never forgotten,
/// see `user_gate_rules.rs`): the design's loop uses the real `SyncGate`, so it reads the late
/// pushbuffer of the lap-later SYNC_WAIT entry before the drain. Safety only (timers allowed so
/// liveness is not the subject).
#[test]
fn design_a_sync_wait_in_the_same_slot_one_lap_later_is_not_read_early() {
    let mut cfg = Cfg::new(Policy::Design, s_lap_sync());
    cfg.timers = false;
    holds("design/lap-sync", &cfg, 0);
}

// ------------------------------------------------------------------------------------------------
// (d) edge cases of the state machine

/// Hostile GP_PUT (garbage beyond the ring): one bounded refusal, the channel dies, the worker
/// parks (no spin, no ring storm), and a later doorbell on the dead channel is absorbed.
#[test]
fn hostile_put_out_of_range_is_refused_once_and_never_spins() {
    for policy in [Policy::Today, Policy::Design] {
        let chans = vec![ChanSpec {
            script: vec![Push(seg(1)), SetPut, Ring, PutRaw(0x7fff_0000), Ring, Ring],
            expected: vec![1],
            must_relay: 0,
            may_block: false,
        }];
        let mut cfg = Cfg::new(policy, chans);
        cfg.timers = false;
        holds(&format!("{policy:?}/hostile-put"), &cfg, V_PUT_BOUND);
    }
}

/// PUT moving BACKWARDS (a legal wrap pointing behind the cursor, or garbage inside the ring): the
/// relay relays at most one ring lap of whatever the slots hold (zero slots are LENGTH==0 NOPs) and
/// stops at the guest's PUT; it neither loops forever nor invents pushbuffer reads.
#[test]
fn put_moving_backwards_relays_at_most_one_lap() {
    for policy in [Policy::Today, Policy::Design] {
        let chans = vec![ChanSpec {
            script: vec![Push(seg(1)), Push(seg(2)), SetPut, Ring, PutRaw(1), Ring],
            expected: vec![1, 2],
            must_relay: 0,
            may_block: false,
        }];
        let mut cfg = Cfg::new(policy, chans);
        cfg.timers = false;
        // PUT=1 after the cursor reached 2 makes the relay walk 2 -> 3 -> 0 -> 1: two ZERO entries and
        // a replay of slot 0 (entry 1) -- exactly what the hardware would do with that PUT, bounded
        // by the ring. The replay is the guest's own doing (reported as ORDER, allowed here); what
        // is pinned: the exploration terminates, the relay publishes at most a ring lap beyond the
        // guest's entries (UNBOUNDED is NOT allowed), it reaches the end of the lap and parks.
        holds(&format!("{policy:?}/put-backwards"), &cfg, V_ORDER);
    }
}

/// Doorbell storm: 6 rings for one PUT coalesce into bounded worker work, and the final state is
/// the same as for one ring.
#[test]
fn doorbell_storm_coalesces() {
    for policy in [Policy::Today, Policy::Design] {
        let chans = vec![spec(
            vec![Push(seg(1)), SetPut, Ring, Ring, Ring, Ring, Ring, Ring],
            vec![1],
            1,
        )];
        let mut cfg = Cfg::new(policy, chans);
        cfg.timers = false;
        holds(&format!("{policy:?}/storm"), &cfg, 0);
    }
}

/// A gate is pending when the channel is retired (guest free / device reset): the retired channel
/// leaves nothing armed (no waiting wake, no tick), the worker parks, and the other channel is
/// unaffected. (Today's live-channel idle poll is isolated by allowing TIMER_TURN here; it is the
/// subject of the tests above.)
fn retire_in_gate(policy: Policy) {
    let chans = vec![
        ChanSpec {
            script: vec![
                Push(seg(1).acq(0)),
                Push(seg(2).sync().late()),
                SetPut,
                Ring,
                Retire,
            ],
            expected: vec![1, 2],
            must_relay: 0,
            may_block: true,
        },
        spec(vec![Push(seg(11)), SetPut, Ring], vec![11], 1),
    ];
    let mut cfg = Cfg::new(policy, chans);
    cfg.timers = true;
    cfg.backstop = true;
    cfg.liveness = false;
    holds(
        &format!("{policy:?}/retire-in-gate"),
        &cfg,
        V_TIMER_TURN | V_STALE | V_BACKSTOP_FOUND_WORK | V_BACKSTOP_CARRIED,
    );
}
#[test]
fn the_retired_loop_left_nothing_armed_for_a_channel_retired_in_a_gate() {
    retire_in_gate(Policy::Today);
}
#[test]
fn design_a_gate_pending_when_the_channel_is_retired_leaves_nothing_armed() {
    retire_in_gate(Policy::Design);
}

/// With the channel retired during its gate and NO timer at all, the neighbour still completes and
/// the worker parks (terminal state reached: no livelock on the dead channel).
#[test]
fn retire_in_gate_without_timers_terminates_for_both_policies() {
    for policy in [Policy::Today, Policy::Design] {
        let chans = vec![
            ChanSpec {
                script: vec![
                    Push(seg(1).acq(0)),
                    Push(seg(2).sync().late()),
                    SetPut,
                    Ring,
                    Retire,
                ],
                expected: vec![1, 2],
                must_relay: 0,
                may_block: true,
            },
            spec(vec![Push(seg(11)), SetPut, Ring], vec![11], 1),
        ];
        let mut cfg = Cfg::new(policy, chans);
        cfg.timers = false;
        holds(&format!("{policy:?}/retire-no-timers"), &cfg, V_TIMER_TURN | V_STALE);
    }
}

/// Two channels share the one worker. A channel whose gate is never released must not cost the
/// worker one wakeup: no timer turns (Today polls it every millisecond for as long as it is
/// stuck), and the other channel's work completes.
fn stuck_gate(policy: Policy, backstop: bool) {
    let mut cfg = Cfg::new(policy, s_two_channels_one_gated_forever());
    cfg.timers = true;
    cfg.backstop = backstop;
    cfg.liveness = false;
    holds(
        &format!("{policy:?}/stuck-gate-wakeups"),
        &cfg,
        V_BACKSTOP_FOUND_WORK | V_BACKSTOP_CARRIED,
    );
}
#[test]
fn the_retired_loop_polled_a_gate_nobody_releases_forever() {
    let mut cfg = Cfg::new(Policy::Today, s_two_channels_one_gated_forever());
    cfg.timers = true;
    cfg.liveness = false;
    finds(
        "retired/stuck-gate",
        &cfg,
        V_BACKSTOP_FOUND_WORK | V_BACKSTOP_CARRIED,
        &["TIMER_TURN"],
    );
}
#[test]
fn design_a_channel_stuck_in_a_gate_costs_no_wakeups_beyond_the_backstop() {
    stuck_gate(Policy::Design, true);
}
/// ... and the neighbour completes while the other channel is stuck, with no timer at all.
#[test]
fn a_stuck_gate_does_not_starve_a_neighbouring_channel() {
    for policy in [Policy::Today, Policy::Design] {
        let mut cfg = Cfg::new(policy, s_two_channels_one_gated_forever());
        cfg.timers = false;
        holds(&format!("{policy:?}/neighbour"), &cfg, V_TIMER_TURN | V_STALE);
    }
}

/// Ring wrap: full/empty are told apart by the one empty slot; the guest's view of GP_GET is the
/// relay's write-back, so a stuck write-back stalls the guest (liveness above).
#[test]
fn ring_full_and_empty_are_distinguishable_at_the_wrap_point() {
    let mut cfg = Cfg::new(Policy::Design, s_wrap());
    cfg.timers = false;
    holds("design/wrap", &cfg, 0);
}

// ------------------------------------------------------------------------------------------------
// (c) migration: nothing dropped, nothing replayed, the new ring is read after the switch

#[test]
fn migration_executes_each_entry_exactly_once_in_order() {
    for (gated, scenario) in [(false, s_migration()), (true, s_migration_gated())] {
        for lag in [false, true] {
            if gated && lag {
                continue; // the gated migration with a lagging GP_GET/REFERENCE exceeds the explorer's bound
            }
            // (the lagging variant runs a smaller migration: timers + backstop multiply the states)
            let scenario = if lag {
                vec![spec(
                    vec![Push(seg(1)), Push(trigger(2)), SetPut, Ring, UseNew, Push(seg(3)), SetPut, Ring],
                    vec![1, 2, 3],
                    3,
                )]
            } else {
                scenario.clone()
            };
            let mut cfg = Cfg::new(Policy::Design, scenario);
            cfg.timers = lag;
            cfg.backstop = lag;
            cfg.lag = lag;
            holds(
                &format!("design/migration/lag={lag}"),
                &cfg,
                if lag {
                    V_BACKSTOP_FOUND_WORK | V_BACKSTOP_CARRIED
                } else {
                    0
                },
            );
        }
    }
}

#[test]
fn the_retired_loop_migrated_without_timers_only_when_no_gate_was_involved() {
    let mut cfg = Cfg::new(Policy::Today, s_migration());
    cfg.timers = false;
    holds("retired/migration", &cfg, 0);
    let mut cfg = Cfg::new(Policy::Today, s_migration_gated());
    cfg.timers = false;
    finds("retired/migration-gated", &cfg, 0, &["DEADLOCK", "STALE_READ"]);
}

// ------------------------------------------------------------------------------------------------
// the model is not vacuous: the races the tests above claim to cover are actually reached

#[test]
fn the_design_exploration_reaches_every_race_it_claims_to_cover() {
    let mut all: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (_, chans) in contract_scenarios() {
        for (pe, locks, depth) in [(8usize, 0u8, usize::MAX), (4, 1, 1)] {
            let mut cfg = Cfg::new(Policy::Design, chans.clone());
            cfg.priv_entries = pe;
            cfg.lock_windows = locks;
            cfg.pop_depth = depth;
            cfg.timers = false;
            all.extend(explore(&cfg, 0).expect("design holds").labels);
        }
    }
    for needle in [
        "w:try_park refused (seq moved)", // a doorbell between the scan and the park (the lost-wakeup window)
        "w:claim c0 (Again)",             // the worker re-rings itself after progress
        "w:wake(efd)",                    // doorbell wake
        "w:wake(event)",                  // completion-interrupt wake
        "w:spin peek",                    // the bounded spin
        "w:T4 gate: wait",                // a turn that finds the gate still draining
        "w:T4 gate: barrier+marker",
        "w:T5 no credit",                     // private-ring credit shortage
        "w:T2 idle: publish marker",          // the batch's completion marker
        "w:T0 record busy",                   // the act thread holds the record
        "w:T1b write guest GP_GET",           // the guest's GP_GET write-back
        "env0:unlock",
    ] {
        assert!(all.contains(needle), "never reached: {needle:?}");
    }
    // the backstop wake itself is reached (a lost interrupt)
    let mut cfg = Cfg::new(Policy::Design, s_gate_late_pb());
    cfg.irq = false;
    cfg.timers = true;
    cfg.backstop = true;
    cfg.liveness = false;
    let lost = explore(&cfg, V_BACKSTOP_FOUND_WORK).expect("a lost interrupt is covered");
    assert!(lost.labels.contains("w:wake(backstop)"));
}

/// ★ DOORBELL CONTRACT (owner, 2026-10-17). `GP_PUT` ahead of the LAST doorbell is normal and
/// nothing is owed for it: only a doorbell announces `GP_PUT`. With timers that fire and the
/// backstop armed, the safety net never reads `GP_PUT` and never rings an idle relay (no
/// `PUT_POLL`), and the design stays live.
#[test]
fn put_ahead_of_the_last_doorbell_is_owed_nothing_and_no_timer_reads_it() {
    for chans in [s_put_ahead_unrung(), s_put_ahead()] {
        let mut cfg = Cfg::new(Policy::Design, chans);
        cfg.timers = false;
        holds("design/put-ahead-unrung", &cfg, 0);
        cfg.timers = true;
        cfg.backstop = true;
        holds("design/put-ahead-unrung/backstop", &cfg, 0);
        // the backstop wake itself is reached: it fires, finds nothing owed, and does nothing
        let r = explore(&cfg, 0).expect("holds");
        assert!(r.labels.contains("w:wake(backstop)"));
    }
}

/// The checker has teeth for it: the RETIRED idle poll (the backstop reads `GP_PUT` and rings the
/// relay when it is ahead of the cursor) is found.
#[test]
fn mutant_the_backstop_polls_gp_put_is_found() {
    for chans in [s_put_ahead_unrung(), s_put_ahead()] {
        let mut cfg = Cfg::new(Policy::Design, chans);
        cfg.m.backstop_polls_put = true;
        cfg.timers = true;
        cfg.backstop = true;
        finds("mutant/backstop_polls_put", &cfg, 0, &["PUT_POLL"]);
    }
}

/// Private-ring credit waits when `GP_GET` may NOT run far ahead of processing (depth 1, 3 credits):
/// the relay's wait for free slots is announced by its markers' interrupts, not by a tick.
#[test]
fn design_credit_waits_with_a_shallow_pop_window_are_event_driven() {
    let f = run_contract(Policy::Design, "design/priv=4/depth=1", 0, |c| {
        c.priv_entries = 4;
        c.pop_depth = 1;
        c.timers = false;
        c.spin = 0;
    });
    no_failures(&f);
}

/// KNOWN LIMITATION, pinned: the guest-visible `GP_GET` is the RELAY's write-back of the entries it has
/// relayed, and a gated entry is relayed only after its gate. A guest whose GPFIFO is FULL behind its own
/// gated entry -- it needs `GP_GET` past that entry to push, and releases the gate only after pushing --
/// therefore deadlocks, by construction, with or without timers; on the hardware `GP_GET` passes the
/// gated entry at once (it runs ahead of processing, measured). It takes a ring full of entries behind a
/// gate (32767 outstanding on a 32768-entry Windows ring), so it is rare, but it is a fidelity gap of the
/// relay, not of the loop. (A bound on how far past a gate the relay may forward bare GP entries is the
/// fix to design; none is attempted here.)
#[test]
fn a_guest_with_a_full_ring_behind_its_own_gate_deadlocks_by_construction() {
    let chans = vec![spec(
        vec![
            Push(seg(1).acq(0)),
            Push(seg(2).sync().late()),
            Push(seg(3)),
            SetPut,
            Ring,
            Push(seg(4)), // needs GP_GET >= 1: entry 1 was relayed and written back
            Push(seg(5)), // needs GP_GET >= 2: entry 2 is gated -- and the gate opens only below
            SetPut,
            Ring,
            Fill(2),
            Release(0),
        ],
        vec![1, 2, 3, 4, 5],
        5,
    )];
    let mut cfg = Cfg::new(Policy::Design, chans);
    cfg.timers = false;
    finds("design/full-ring-behind-own-gate", &cfg, 0, &["DEADLOCK"]);
    // the control: the same script without the fifth push (nothing waits for GP_GET past the gate)
    let chans = vec![spec(
        vec![
            Push(seg(1).acq(0)),
            Push(seg(2).sync().late()),
            Push(seg(3)),
            SetPut,
            Ring,
            Push(seg(4)),
            SetPut,
            Ring,
            Fill(2),
            Release(0),
        ],
        vec![1, 2, 3, 4],
        4,
    )];
    let mut cfg = Cfg::new(Policy::Design, chans);
    cfg.timers = false;
    holds("design/full-ring-control", &cfg, 0);
}
