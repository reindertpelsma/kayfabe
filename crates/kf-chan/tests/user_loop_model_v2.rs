//! 2026-10-20 -- **Exhaustive interleaving model v2 of the Windows USER-ring relay's worker loop**: the REAL per-entry
//! turn (`kf_chan::user_turn::Relay` through `lifecycle_support::Relay`, lazy observe with the production defaults, the
//! real `Lifecycle` in Pointer mode) under a worker/token/wake/peek model, several channels, cross-channel semaphores,
//! the disable/enable act window and the private doorbell. GPU-free, deterministic, no wall-clock: a timer is a
//! scheduler step a configuration may forbid (`backstop = false` is the default of every liveness test).
//!
//! ## 1. WHAT THE CODE DOES (CODE READ at f8f9d1a5f: `userpump.rs`, `worker.rs`, `kf-core/plane.rs`, `chan.rs`)
//!
//! State a turn reads / writes (`ChanPlane::user_ring_serve` -> `Pump::turn` -> `user_turn::Relay::step`):
//!  * guest memory: USERD `GP_PUT` (read; fresh, or the PUT cache of rule A while cursor < cached), the GP entry at the
//!    cursor (one 8-byte snapshot), the pushbuffer (Scan/Window only), USERD `GP_GET` (WRITTEN: the guest's view);
//!  * private storage (vidmem): `GP_GET` and `REFERENCE` (read; every turn in Scan/Window, every 16th turn / after a
//!    wait / low credit / open segment / ring-full in Pointer: rule B), private GPFIFO + markers + `GP_PUT` (WRITTEN by
//!    `publish_guarded`, under the `pt` try-lock and the USERD admission), then the NATIVE DOORBELL (`rm.doorbell`,
//!    AFTER the pt lock is released);
//!  * relay state: cursor, generation, `Fetch` (issued/fetched/receipts), `Marks` (issued/seen), `SyncGate`, `Scan`,
//!    `Lifecycle`, `Lazy` (put cache, wait latch, interval counter), `need`, `again`, `unnotified`, `marker_event`;
//!  * record state outside the lock: `Cell::retry` (demand), `Cell::waiting` (mirror of `Pump::wait`),
//!    `Cell::contended` (consecutive contended turns), `Cell::canceled`; pt state `disabled` / `stopped`.
//!
//! Wake sources of a relay and what guarantees progress for each:
//!  1. DOORBELL (guest store -> token word RUNG -> bitmap + `worker_wake.bump`): `try_park(seen)` refuses when the
//!     sequence moved; the claim clears the flag BEFORE `GP_PUT` is read; a ring during BUSY is BUSY_RUNG and re-served
//!     (REACT_ROUNDS, then RepublishAndMoveOn). Guarantee: the last doorbell is followed by a turn that reads `GP_PUT`
//!     fresh (the PUT cache is used only while cursor < cached, so "nothing to do" is decided on a fresh read).
//!  2. `Need::Again` (progress): `user_ring_arm` -> `ring_internal` (BUSY_RUNG), same guarantees as 1.
//!  3. COMPLETION / ENGINE EVENT (the marker's NON_STALL_INTERRUPT on the worker's epoll): `dispatch` -> `peek(Event)` of
//!     every relay: reads the PRIVATE `GP_GET`/`REFERENCE` fresh and rings the satisfied. Guarantee: only if a marker was
//!     published (not in Pointer mode away from a GP_PUT boundary or with markers latched off) and the words are
//!     visible when the event is handled (`event_without_progress` counts the other case).
//!  4. SPIN (<= 64 `peek(Spin)` while some relay `waiting || pending`): an optimisation; liveness is proven with 0.
//!  5. BACKSTOP (50 ms `epoll` timeout): `peek(Backstop)`. Counted; DISABLED in every liveness test here (P6).
//!  6. ACT / lock release (`user_ring_wake`: `Cell::request` sets `retry`, `wake_if` rings if the record try-lock is free,
//!     `wake.signal()`): after birth/5080/ctx acts and after `DISABLE_CHANNELS(bDisable=false)` (chan.rs 4392). NOT after
//!     `disable_usermode(false)` (chan.rs 4181-4259: host verb only, no flag, no wake) and NOT after a `pt` /
//!     USERD-admission release (no wake exists: spin + backstop only, `Wait::Contended`).
//!  7. `serve` returns early WITHOUT a turn when the record is busy (retry stays set), pt is busy (retry stays set),
//!     `t.disabled || t.stopped` (retry is CLEARED; the enable's `user_ring_wake` re-requests), retired/dead (cancel).
//!
//! ## 2. WHAT IS MODELLED / NOT MODELLED
//!  Modelled: everything above, 1-4 channels, global semaphores (`rel`/`acq` of a segment; CPU `Release`), PBDMA with
//!  `GP_GET` prefetch (measured), loaded/unloaded channel state and the private doorbell as its own step (lost on a
//!  host-disabled channel under `hw_drops_db_when_off`), visibility lag, interrupt loss, record/pt lock windows, the
//!  disable/enable acts (`Channels` = chan.rs 4261-4411 with `disabled` set AFTER the host verb; `Usermode` = 4181-4259),
//!  an ORACLE (no relay: the hardware reads the guest ring after each doorbell) to attribute a stall to the relay (P7).
//!  NOT modelled: GPU time-slicing between twin TSGs, WC/posted-write ordering between the BAR1 vidmem stores and the
//!  BAR0 doorbell (a GPU-side race, not expressible GPU-free), the shared software object / registry across the channels
//!  of one client, `segment_turn` triggers (Window/ring growth; covered by `user_lifecycle_model`), pt/USERD-admission
//!  busy at the `GP_GET` write.
//!
//! ## 3. PROPERTIES
//!  Safety: STALE, ORDER, UNBOUNDED, PARKED_WITH_WORK, MARKER_IN_PACKET, BEYOND_PUT (I8 and P9: a cached `GP_PUT` is never
//!  beyond the guest's real one), PUT_BOUND, TORN, backstop counters. Liveness at terminal states (timers that never fire):
//!  P6 every entry covered by a doorbell is relayed AND executed; P7 (a stall that the oracle does not have = a
//!  relay-introduced circular wait); P8 an enable act leaves no PUT-ahead-unread twin; P9 (stale cache); P10 no published
//!  private entry stays unknown to the hardware (`latched < log.len()`) at a terminal state.
#![allow(
    clippy::too_many_lines,
    clippy::needless_range_loop,
    clippy::type_complexity,
    dead_code,
    missing_docs,
    clippy::collapsible_if
)]

#[path = "lifecycle_support/mod.rs"]
mod support;

use kf_chan::user_gate::MARK_TAG;
use kf_chan::user_lazy::LazyConfig;
use kf_chan::user_lifecycle::{Lifecycle, LifecycleMode, Mode};
use kf_chan::user_loop::{self as ul, Need, Wait};
use kf_chan::user_scan::Scan;
use kf_chan::user_turn::Phase;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use support::*;

// ------------------------------------------------------------------------------------------------
// vocabulary

/// `kf_trap::REACT_ROUNDS` is 8; 2 keeps the model small (the rule is the same).
const REACT: u8 = 2;

const V_STALE: u32 = 1 << 0;
const V_MARKER_IN_PACKET: u32 = 1 << 1;
const V_ORDER: u32 = 1 << 2;
const V_UNBOUNDED: u32 = 1 << 3;
const V_PARKED_WITH_WORK: u32 = 1 << 4;
const V_BEYOND_PUT: u32 = 1 << 5;
const V_BACKSTOP_FOUND_WORK: u32 = 1 << 6;
const V_BACKSTOP_CARRIED: u32 = 1 << 7;
const V_BACKSTOP_CONTENDED: u32 = 1 << 8;
const V_PUT_BOUND: u32 = 1 << 9;
const V_TORN: u32 = 1 << 10;
const V_ENTRY_REFUSED: u32 = 1 << 11;
const V_REFUSED: u32 = 1 << 12;

fn viol_names(v: u32) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (bit, name) in [
        (
            V_STALE,
            "STALE_READ (pushbuffer read before the guest wrote it)",
        ),
        (
            V_MARKER_IN_PACKET,
            "MARKER_IN_PACKET (a host marker inside an open guest packet)",
        ),
        (
            V_ORDER,
            "ORDER (the PBDMA executed something other than the guest's entries in order)",
        ),
        (
            V_UNBOUNDED,
            "UNBOUNDED (published more than a lap beyond the guest's entries)",
        ),
        (
            V_PARKED_WITH_WORK,
            "PARKED_WITH_WORK (blocked while a doorbell flag was pending)",
        ),
        (
            V_BEYOND_PUT,
            "BEYOND_PUT (I8 / P9: read at or beyond the loaded GP_PUT, or a cached GP_PUT ahead of the guest's)",
        ),
        (
            V_BACKSTOP_FOUND_WORK,
            "BACKSTOP_FOUND_WORK (the safety net was needed)",
        ),
        (
            V_BACKSTOP_CARRIED,
            "BACKSTOP_CARRIED (counted apart: no interrupt exists for this wait)",
        ),
        (
            V_BACKSTOP_CONTENDED,
            "BACKSTOP_CONTENDED (counted apart: a lock release nobody announced)",
        ),
        (V_PUT_BOUND, "PUT_BOUND"),
        (V_TORN, "TORN (value stored != value tested)"),
        (V_ENTRY_REFUSED, "ENTRY_REFUSED"),
        (V_REFUSED, "REFUSED (scan refused for good)"),
    ] {
        if v & bit != 0 {
            out.push(name);
        }
    }
    out
}

// reach bits (monotone facts the exploration saw; the tests assert the races were reached)
const R_POINTER: u32 = 1 << 0;
const R_SKIPPED_OBS: u32 = 1 << 1;
const R_CACHE_HIT: u32 = 1 << 2;
const R_DISABLED_SERVE: u32 = 1 << 3;
const R_DOORBELL_LOST: u32 = 1 << 4;
const R_LOCK_BUSY: u32 = 1 << 5;
const R_EVENT_NO_PROGRESS: u32 = 1 << 6;
const R_WAIT_MARKER: u32 = 1 << 7;
const R_WAIT_POPPED: u32 = 1 << 8;
const R_PUBLISH_ON_OFF_CHANNEL: u32 = 1 << 9;
const R_GATE: u32 = 1 << 10;
const R_CARRIED: u32 = 1 << 11;
const R_SEM_BLOCKED: u32 = 1 << 12;
const R_PARK_REFUSED: u32 = 1 << 13;
const R_ACT_WAKE_NO_RING: u32 = 1 << 14;
const R_ABORTED_TURN: u32 = 1 << 15;

#[derive(Clone, Debug)]
enum Op {
    Push(Ge),
    /// `n` pushes as one atomic guest step (needs room for all of them).
    Many(Vec<Ge>),
    SetPut,
    Ring,
    Fill(u8),
    /// A CPU-side semaphore release.
    Release(u8),
    /// `Many` + `SetPut` + `Ring` as ONE guest step (cross-channel scenarios: keeps the product small; the
    /// push/put/ring races are covered by the single-channel scenarios).
    Submit(Vec<Ge>),
}

#[derive(Clone, Debug)]
struct ChanSpec {
    script: Vec<Op>,
    /// The ids the PBDMA must execute, in order.
    expected: Vec<u8>,
    /// At a terminal state at least this many leading ids were executed.
    must_exec: usize,
    /// `(entry id, semaphore)`: the segment releases it when it begins.
    rel: Vec<(u8, u8)>,
    /// Leading ops run greedily before the exploration (to reach Pointer mode cheaply).
    prelude: usize,
    may_block: bool,
}
impl ChanSpec {
    fn new(script: Vec<Op>, expected: Vec<u8>) -> Self {
        let n = expected.len();
        Self {
            script,
            expected,
            must_exec: n,
            rel: Vec::new(),
            prelude: 0,
            may_block: false,
        }
    }
    fn rel(mut self, id: u8, sem: u8) -> Self {
        self.rel.push((id, sem));
        self
    }
    fn prelude(mut self, n: usize) -> Self {
        self.prelude = n;
        self
    }
    fn must(mut self, n: usize) -> Self {
        self.must_exec = n;
        self
    }
    fn may_block(mut self) -> Self {
        self.may_block = true;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    None,
    /// `disable_channels` (chan.rs 4261-4411): host disable, then `disabled = true`; host enable, `disabled = false`,
    /// `user_ring_wake`. Targets are channel indices.
    Channels(u8),
    /// `disable_usermode` (chan.rs 4181-4259): host disable / host enable only.
    Usermode(u8),
}

/// Deliberately broken variants of the transliterated parts (`worker.rs`, `userpump.rs`, `chan.rs`): the checker must
/// find each one, with at least two scenarios.
#[derive(Clone, Copy, Debug, Default)]
struct Mutant {
    /// `try_park` without the sequence comparison.
    no_seq_check: bool,
    /// The token release forgets a ring that arrived while BUSY.
    drop_busy_rung: bool,
    /// The enable act does not call `user_ring_wake` (chan.rs 4392).
    enable_skips_wake: bool,
    /// The act's lock release does not call `user_ring_wake`.
    unlock_no_wake: bool,
    /// The worker ignores engine events (no `peek(Event)`).
    event_deaf: bool,
    /// A serve that finds the record busy drops the durable demand.
    busy_drops_demand: bool,
    /// A serve that finds the twin disabled does not clear `retry` AND is also not re-requested (control: benign).
    /// `peek` never rings a satisfied wait.
    peek_never_rings: bool,
    /// The look does not write the guest's `GP_GET` back for pops under a draining marker (pop_writebacks).
    no_pop_writeback_ring: bool,
}

#[derive(Clone)]
struct Cfg {
    chans: Vec<ChanSpec>,
    lazy: LazyConfig,
    mode: LifecycleMode,
    src_entries: u32,
    priv_entries: u32,
    pop_depth: usize,
    irq: bool,
    lag: bool,
    spin: u8,
    timers: bool,
    backstop: bool,
    lock_windows: u8,
    pt_windows: u8,
    act: Act,
    hw_drops_db_when_off: bool,
    hw_reload_on_enable: bool,
    /// INFERRED hardware ordering: the BAR0 doorbell can reach the Host unit before the BAR1 stores of the private
    /// GPFIFO entries / GP_PUT are visible in video memory (no read-back separates them in `publish_guarded`).
    hw_db_races_stores: bool,
    /// The proposed fix: a read-back of the last stored word before the doorbell.
    readback_before_doorbell: bool,
    oracle: bool,
    liveness: bool,
    m: Mutant,
    max_states: usize,
}
impl Cfg {
    fn new(chans: Vec<ChanSpec>) -> Self {
        Self {
            chans,
            lazy: LazyConfig::ON,
            mode: LifecycleMode::Default,
            src_entries: 8,
            priv_entries: 128,
            pop_depth: usize::MAX,
            irq: true,
            lag: false,
            spin: 1,
            timers: false,
            backstop: false,
            lock_windows: 0,
            pt_windows: 0,
            act: Act::None,
            hw_drops_db_when_off: false,
            hw_reload_on_enable: false,
            hw_db_races_stores: false,
            readback_before_doorbell: false,
            oracle: false,
            liveness: true,
            m: Mutant::default(),
            max_states: 1_500_000,
        }
    }
    fn lazy(mut self, l: LazyConfig) -> Self {
        self.lazy = l;
        self
    }
    fn src(mut self, n: u32) -> Self {
        self.src_entries = n;
        self
    }
    fn privr(mut self, n: u32) -> Self {
        self.priv_entries = n;
        self
    }
    fn act(mut self, a: Act) -> Self {
        self.act = a;
        self
    }
    fn locks(mut self, n: u8) -> Self {
        self.lock_windows = n;
        self
    }
    fn oracle(mut self) -> Self {
        self.oracle = true;
        self
    }
    fn spin(mut self, n: u8) -> Self {
        self.spin = n;
        self
    }
    fn pop_depth(mut self, n: usize) -> Self {
        self.pop_depth = n;
        self
    }
    fn lag(mut self) -> Self {
        self.lag = true;
        self
    }
    fn no_irq(mut self) -> Self {
        self.irq = false;
        self
    }
    fn backstop(mut self) -> Self {
        self.timers = true;
        self.backstop = true;
        self
    }
    fn db_races_stores(mut self) -> Self {
        self.hw_db_races_stores = true;
        self
    }
    fn readback(mut self) -> Self {
        self.readback_before_doorbell = true;
        self
    }
    fn adversarial_hw(mut self) -> Self {
        self.hw_drops_db_when_off = true;
        self
    }
    fn mutant(mut self, f: impl FnOnce(&mut Mutant)) -> Self {
        f(&mut self.m);
        self
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

/// One channel's memory and hardware, and the `Env` the real relay reads and writes.
#[derive(Clone)]
struct CEnv {
    ges: Rc<Vec<Option<Ge>>>,
    rels: Rc<Vec<Option<u8>>>,
    e: u32,
    priv_entries: u32,
    ring: Vec<(u32, u32)>,
    put: u32,
    guest_get: u32,
    fill: u64,
    log: Vec<Pub>,
    popped: usize,
    hw_get: usize,
    markers_done: u64,
    vis_popped: usize,
    vis_refs: u64,
    hw_wait: Option<u8>,
    executed: Vec<u8>,
    ev: bool,
    viol: u32,
    hw_scan: Scan,
    db_pending: bool,
    host_off: bool,
    loaded: bool,
    latched: usize,
    hw_pending: bool,
    /// Private entries (and the private `GP_PUT`) whose stores are visible to the GPU.
    landed: usize,
    observed: bool,
    cache_hit: bool,
    carried: bool,
    /// The pt / USERD admission is busy right now (the guest `GP_GET` write answers `Busy`).
    busy: bool,
}

impl Env for CEnv {
    fn src_entries(&self) -> u32 {
        self.e
    }
    fn gp_put(&mut self) -> u32 {
        self.put
    }
    fn gp_dword(&mut self, slot: u32, hi: bool) -> u32 {
        let (lo, h) = self.ring[slot as usize];
        if hi { h } else { lo }
    }
    fn registry_epoch(&mut self) -> u64 {
        0
    }
    fn registry_live(&mut self) -> u32 {
        0
    }
    fn hw_get(&mut self) -> u32 {
        self.observed = true;
        (self.vis_popped as u32) & (self.priv_entries - 1)
    }
    fn hw_reference(&mut self) -> u32 {
        if self.vis_refs == 0 {
            0
        } else {
            MARK_TAG | (self.vis_refs as u32 & 0xffff)
        }
    }
    fn read_pb(&mut self, raw: u64) -> Vec<u32> {
        let Some(id) = entry_id(raw) else {
            self.viol |= V_STALE;
            return vec![POISON];
        };
        let g = self.ges[usize::from(id)].expect("known id");
        let words = g.words();
        if g.late && self.fill & (1 << id) == 0 {
            self.viol |= V_STALE;
            return vec![POISON; words.len()];
        }
        words
    }
    fn publish(&mut self, p: Pub) {
        let hw_words = |env: &Self, raw: u64| -> Vec<u32> {
            entry_id(raw)
                .and_then(|id| env.ges.get(usize::from(id)).copied().flatten())
                .map(|g| g.words())
                .unwrap_or_default()
        };
        match p {
            Pub::Guest { stored, .. } | Pub::Staged { tested: stored } => {
                let words = hw_words(self, stored);
                let mut at = 0usize;
                loop {
                    match self.hw_scan.segment(words.get(at..).unwrap_or(&[]), sw) {
                        Ok(Some(t)) => {
                            at += t.end;
                            if at >= words.len() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(_) => {
                            self.hw_scan = Scan::default();
                            break;
                        }
                    }
                }
            }
            Pub::Marker(_) => {
                if self.hw_scan.carried().is_some() {
                    self.viol |= V_MARKER_IN_PACKET;
                }
            }
            Pub::Barrier => {}
        }
        if let Pub::Guest { tested, stored, .. } = p {
            if tested != stored {
                self.viol |= V_TORN;
            }
        }
        self.log.push(p);
        // publish_guarded: the private doorbell is rung AFTER the pt lock is released
        self.db_pending = true;
    }
    fn write_guest_get(&mut self, v: u32) {
        self.guest_get = v;
    }
    fn write_guest_get_busy(&mut self) -> bool {
        self.busy
    }
    fn trigger_migrates(&mut self, _raw: u64) -> bool {
        false
    }
    fn executed(&mut self, _migrate: bool) {}
    fn event(&mut self, e: Ev) {
        match e {
            Ev::PutBound => self.viol |= V_PUT_BOUND,
            Ev::EntryRefused => self.viol |= V_ENTRY_REFUSED,
            Ev::ScanRefused => self.viol |= V_REFUSED,
            Ev::BeyondPut => self.viol |= V_BEYOND_PUT,
            Ev::CarriedUngated => self.carried = true,
            _ => {}
        }
    }
    /// P9: a PUT the relay used WITHOUT reading `GP_PUT` this turn must not be beyond the guest's real `GP_PUT`.
    fn cached_put_ok(&mut self, put: u32, cursor: u32) -> bool {
        self.cache_hit = true;
        let m = self.e - 1;
        put.wrapping_sub(cursor) & m <= self.put.wrapping_sub(cursor) & m
    }
}

#[derive(Clone)]
struct Ch {
    env: CEnv,
    relay: Relay,
    // guest
    gpc: usize,
    put_local: u32,
    since_put: usize,
    visible: usize,
    covered: usize,
    raws: Vec<u64>,
    oracle_pub: usize,
    // token and record
    tok: Tok,
    retry: bool,
    waiting_flag: bool,
    p_wait: Option<Wait>,
    contended_run: u8,
    canceled: bool,
    act_lock: bool,
    serving: bool,
    pt_held: bool,
    disabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pc {
    Top0,
    Top1,
    Spin,
    Top2,
    Parked,
    Event,
    S(u8),
}
const S_REQ: u8 = 0;
const S_LOCK: u8 = 1;
const S_CHK: u8 = 2;
const S_SWAP: u8 = 3;
const S_RUN: u8 = 4;

#[derive(Clone)]
struct World {
    ch: Vec<Ch>,
    pc: Pc,
    cur: usize,
    rounds: u8,
    moved: bool,
    parked: bool,
    efd: bool,
    spin_left: u8,
    locks_left: u8,
    pts_left: u8,
    timers_left: u8,
    act: u8,
    sem: u32,
    viol: u32,
    reach: u32,
    limit: bool,
}

impl Ch {
    fn alive(&self) -> bool {
        !self.canceled && self.tok != Tok::Dead
    }
    fn pending(&self) -> bool {
        !self.canceled && self.retry
    }
}

impl World {
    fn new(cfg: &Cfg) -> Self {
        let ch = cfg
            .chans
            .iter()
            .map(|spec| {
                let mut ges: Vec<Option<Ge>> = vec![None; 64];
                for op in &spec.script {
                    match op {
                        Op::Push(g) => ges[usize::from(g.id)] = Some(*g),
                        Op::Many(gs) | Op::Submit(gs) => {
                            for g in gs {
                                ges[usize::from(g.id)] = Some(*g);
                            }
                        }
                        _ => {}
                    }
                }
                let mut rels = vec![None; 64];
                for (id, s) in &spec.rel {
                    rels[usize::from(*id)] = Some(*s);
                }
                let life = Lifecycle::new().with_mode(cfg.mode);
                let scan = Scan::for_subdevice(1).expect("sub-device");
                let relay =
                    Relay::new(cfg.priv_entries, life, scan, Glue::default()).with_lazy(cfg.lazy);
                Ch {
                    env: CEnv {
                        ges: Rc::new(ges),
                        rels: Rc::new(rels),
                        e: cfg.src_entries,
                        priv_entries: cfg.priv_entries,
                        ring: vec![(0, 0); cfg.src_entries as usize],
                        put: 0,
                        guest_get: 0,
                        fill: 0,
                        log: Vec::new(),
                        popped: 0,
                        hw_get: 0,
                        markers_done: 0,
                        vis_popped: 0,
                        vis_refs: 0,
                        hw_wait: None,
                        executed: Vec::new(),
                        ev: false,
                        viol: 0,
                        hw_scan: Scan::default(),
                        db_pending: false,
                        host_off: false,
                        loaded: false,
                        latched: 0,
                        hw_pending: false,
                        landed: 0,
                        observed: false,
                        cache_hit: false,
                        carried: false,
                        busy: false,
                    },
                    relay,
                    gpc: 0,
                    put_local: 0,
                    since_put: 0,
                    visible: 0,
                    covered: 0,
                    raws: Vec::new(),
                    oracle_pub: 0,
                    tok: Tok::Idle,
                    retry: false,
                    waiting_flag: false,
                    p_wait: None,
                    contended_run: 0,
                    canceled: false,
                    act_lock: false,
                    serving: false,
                    pt_held: false,
                    disabled: false,
                }
            })
            .collect();
        Self {
            ch,
            pc: if cfg.oracle { Pc::Parked } else { Pc::Top0 },
            cur: 0,
            rounds: 0,
            moved: false,
            parked: cfg.oracle,
            efd: false,
            spin_left: cfg.spin,
            locks_left: cfg.lock_windows,
            pts_left: cfg.pt_windows,
            timers_left: if cfg.backstop { 2 } else { 0 },
            act: 0,
            sem: 0,
            viol: 0,
            reach: 0,
            limit: false,
        }
    }

    /// `Plane::ring_internal`: IDLE -> RUNG (+ bump, + eventfd when parked); a ring while BUSY is BUSY_RUNG.
    fn ring(&mut self, c: usize) {
        match self.ch[c].tok {
            Tok::Idle => {
                self.ch[c].tok = Tok::Rung;
                self.moved = true;
                if self.parked {
                    self.efd = true;
                }
            }
            Tok::Busy => self.ch[c].tok = Tok::BusyRung,
            Tok::Rung | Tok::BusyRung | Tok::Dead => {}
        }
    }

    /// `ChanPlane::user_ring_wake`: `request()`, ring if the record try-lock is free, `wake.signal()`.
    fn user_ring_wake(&mut self, c: usize) {
        if self.ch[c].canceled {
            return;
        }
        self.ch[c].retry = true;
        if !self.ch[c].act_lock && !self.ch[c].serving {
            self.ring(c);
        } else {
            self.reach |= R_ACT_WAKE_NO_RING;
        }
        self.efd = true;
    }

    /// Feed everything that decides behaviour (none of the counters) to a hasher.
    fn feed<H: Hasher>(&self, h: &mut H) {
        (
            self.pc,
            self.cur,
            self.rounds,
            self.moved,
            self.parked,
            self.efd,
            self.spin_left,
            self.locks_left,
            self.pts_left,
            self.timers_left,
            self.act,
        )
            .hash(h);
        (self.sem, self.viol, self.limit).hash(h);
        for c in &self.ch {
            let e = &c.env;
            (
                c.gpc,
                c.put_local,
                c.since_put,
                c.visible,
                c.covered,
                c.tok,
                c.retry,
                c.waiting_flag,
            )
                .hash(h);
            (
                c.contended_run,
                c.canceled,
                c.act_lock,
                c.serving,
                c.pt_held,
                c.disabled,
                c.oracle_pub,
            )
                .hash(h);
            format!("{:?}", c.p_wait).hash(h);
            (
                &e.ring,
                e.put,
                e.guest_get,
                e.fill,
                e.popped,
                e.hw_get,
                e.markers_done,
                e.hw_wait,
            )
                .hash(h);
            (e.vis_popped, e.vis_refs, e.ev, e.viol, e.db_pending).hash(h);
            (
                e.host_off,
                e.loaded,
                e.latched,
                e.hw_pending,
                e.busy,
                e.landed,
                e.log.len(),
            )
                .hash(h);
            e.log[e.hw_get.min(e.log.len())..].hash(h);
            e.executed.hash(h);
            e.hw_scan.carried().is_some().hash(h);
            c.relay.key().hash(h);
        }
    }

    fn key(&self) -> u128 {
        use std::collections::hash_map::DefaultHasher;
        let mut a = DefaultHasher::new();
        self.feed(&mut a);
        let mut b = DefaultHasher::new();
        0x9e37_79b9_7f4a_7c15_u64.hash(&mut b);
        self.feed(&mut b);
        (u128::from(a.finish()) << 64) | u128::from(b.finish())
    }
}

// ------------------------------------------------------------------------------------------------
// steps

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
        let spec = &cfg.chans[c];
        if w.limit && ch.gpc >= spec.prelude {
            continue;
        }
        let Some(op) = spec.script.get(ch.gpc).cloned() else {
            continue;
        };
        let mut n = w.clone();
        let e = cfg.src_entries;
        let m = &mut n.ch[c];
        match &op {
            Op::Push(g) => {
                if (m.put_local + 1) % e == m.env.guest_get {
                    continue;
                }
                let raw = g.raw();
                m.env.ring[m.put_local as usize] = (raw as u32, (raw >> 32) as u32);
                m.put_local = (m.put_local + 1) % e;
                m.since_put += 1;
                m.raws.push(raw);
                if !g.late {
                    m.env.fill |= 1 << g.id;
                }
            }
            Op::Many(gs) => {
                let used = (m.put_local + e - m.env.guest_get) % e;
                if (used as usize) + gs.len() >= e as usize {
                    continue;
                }
                for g in gs {
                    let raw = g.raw();
                    m.env.ring[m.put_local as usize] = (raw as u32, (raw >> 32) as u32);
                    m.put_local = (m.put_local + 1) % e;
                    m.since_put += 1;
                    m.raws.push(raw);
                    if !g.late {
                        m.env.fill |= 1 << g.id;
                    }
                }
            }
            Op::SetPut => {
                m.env.put = m.put_local;
                m.visible += m.since_put;
                m.since_put = 0;
            }
            Op::Ring => {
                m.covered = m.visible;
                if cfg.oracle {
                    while m.oracle_pub < m.visible {
                        let raw = m.raws[m.oracle_pub];
                        m.env.log.push(Pub::Guest {
                            tested: raw,
                            stored: raw,
                            scanned: false,
                        });
                        m.oracle_pub += 1;
                    }
                    m.env.loaded = true;
                    m.env.latched = m.env.log.len();
                } else {
                    n.ring(c);
                }
            }
            Op::Fill(id) => m.env.fill |= 1 << id,
            Op::Release(s) => n.sem |= 1 << s,
            Op::Submit(gs) => {
                let used = (m.put_local + e - m.env.guest_get) % e;
                if (used as usize) + gs.len() >= e as usize {
                    continue;
                }
                for g in gs {
                    let raw = g.raw();
                    m.env.ring[m.put_local as usize] = (raw as u32, (raw >> 32) as u32);
                    m.put_local = (m.put_local + 1) % e;
                    m.raws.push(raw);
                    if !g.late {
                        m.env.fill |= 1 << g.id;
                    }
                }
                m.env.put = m.put_local;
                m.visible += gs.len();
                m.covered = m.visible;
                if cfg.oracle {
                    while m.oracle_pub < m.visible {
                        let raw = m.raws[m.oracle_pub];
                        m.env.log.push(Pub::Guest {
                            tested: raw,
                            stored: raw,
                            scanned: false,
                        });
                        m.oracle_pub += 1;
                    }
                    m.env.loaded = true;
                    m.env.latched = m.env.log.len();
                } else {
                    n.ring(c);
                }
            }
        }
        n.ch[c].gpc += 1;
        out.push(step(format!("g{c}:{:?}", short(&op)), n));
    }
}

fn short(op: &Op) -> String {
    match op {
        Op::Push(g) => format!("Push#{}", g.id),
        Op::Many(gs) => format!("Many[{}]", gs.len()),
        Op::Submit(gs) => format!("Submit[{}]", gs.len()),
        Op::SetPut => "SetPut".into(),
        Op::Ring => "Ring".into(),
        Op::Fill(i) => format!("Fill#{i}"),
        Op::Release(s) => format!("Release(sem{s})"),
    }
}

fn hw_steps(cfg: &Cfg, w: &World, out: &mut Vec<Step>) {
    for c in 0..w.ch.len() {
        let ch = &w.ch[c];
        let e = &ch.env;
        let blocked = e.hw_wait.is_some_and(|s| w.sem & (1 << s) == 0);
        if !e.host_off && e.hw_get < e.popped.min(e.log.len()) && !blocked {
            let mut n = w.clone();
            let sem_now = n.sem;
            let m = &mut n.ch[c].env;
            m.hw_wait = None;
            let p = m.log[m.hw_get];
            m.hw_get += 1;
            let mut label = format!("hw{c}:exec");
            match p {
                Pub::Guest { stored, .. } | Pub::Staged { tested: stored } => {
                    if let Some(id) = entry_id(stored) {
                        if let Some(g) = m.ges[usize::from(id)] {
                            m.hw_wait = g.acq;
                        }
                        if let Some(Some(s)) = m.rels.get(usize::from(id)) {
                            n.sem |= 1 << s;
                        }
                        let m = &mut n.ch[c].env;
                        m.executed.push(id);
                        let _ = write!(label, " #{id}");
                    } else {
                        n.ch[c].env.viol |= V_TORN;
                    }
                }
                Pub::Barrier => label.push_str(" barrier"),
                Pub::Marker(_) => {
                    m.markers_done += 1;
                    if !cfg.lag {
                        m.vis_refs = m.markers_done;
                    }
                    if cfg.irq {
                        m.ev = true;
                    }
                    label.push_str(" marker");
                }
            }
            let _ = sem_now;
            let exp = &cfg.chans[c].expected;
            let ex = &n.ch[c].env.executed;
            if ex.len() > exp.len() || ex[..] != exp[..ex.len()] {
                n.ch[c].env.viol |= V_ORDER;
            }
            out.push(step(label, n));
        }
        if cfg.hw_db_races_stores && e.landed < e.log.len() {
            let mut n = w.clone();
            n.ch[c].env.landed = e.log.len();
            out.push(step(
                format!("hw{c}:private stores land in video memory"),
                n,
            ));
        }
        if cfg.lag && (e.vis_popped != e.popped || e.vis_refs != e.markers_done) {
            let mut n = w.clone();
            n.ch[c].env.vis_popped = e.popped;
            n.ch[c].env.vis_refs = e.markers_done;
            out.push(step(format!("hw{c}:GP_GET/REFERENCE visible"), n));
        }
    }
}

fn env_steps(cfg: &Cfg, w: &World, out: &mut Vec<Step>) {
    if w.limit {
        return;
    }
    for c in 0..w.ch.len() {
        if !w.ch[c].act_lock && w.locks_left > 0 && w.ch[c].alive() {
            let mut n = w.clone();
            n.ch[c].act_lock = true;
            n.locks_left -= 1;
            out.push(step(format!("env{c}:act holds the record"), n));
        }
        if w.ch[c].act_lock {
            let mut n = w.clone();
            n.ch[c].act_lock = false;
            if !cfg.m.unlock_no_wake {
                n.user_ring_wake(c);
            }
            out.push(step(
                format!("env{c}:act releases the record + user_ring_wake"),
                n,
            ));
        }
        if !w.ch[c].pt_held && w.pts_left > 0 && w.ch[c].alive() {
            let mut n = w.clone();
            n.ch[c].pt_held = true;
            n.pts_left -= 1;
            out.push(step(format!("env{c}:pt held"), n));
        }
        if w.ch[c].pt_held {
            let mut n = w.clone();
            n.ch[c].pt_held = false;
            out.push(step(format!("env{c}:pt released (nobody rings)"), n));
        }
    }
    // the disable/enable act, one stage per step so that every other actor runs between any two
    let (kind, mask) = match cfg.act {
        Act::None => return,
        Act::Channels(m) => (true, m),
        Act::Usermode(m) => (false, m),
    };
    let targets: Vec<usize> = (0..w.ch.len()).filter(|c| mask & (1 << c) != 0).collect();
    let mut n = w.clone();
    match (kind, w.act) {
        (_, 0) => {
            // host DISABLE_CHANNELS: the channel is preempted (unloaded) and not scheduled
            for &c in &targets {
                n.ch[c].env.host_off = true;
                n.ch[c].env.loaded = false;
            }
            n.act = 1;
            out.push(step("act:host disable returned", n));
        }
        (true, 1) => {
            // chan.rs 4364: `v.disabled = disable` -- only AFTER the host verb returned
            for &c in &targets {
                n.ch[c].disabled = true;
            }
            n.act = 2;
            out.push(step("act:pt.disabled = true", n));
        }
        (true, 2) | (false, 1) => {
            for &c in &targets {
                enable_host(cfg, &mut n, c);
            }
            n.act = if kind { 3 } else { 9 };
            out.push(step("act:host enable returned", n));
        }
        (true, 3) => {
            for &c in &targets {
                n.ch[c].disabled = false;
            }
            n.act = 4;
            out.push(step("act:pt.disabled = false", n));
        }
        (true, 4) => {
            if !cfg.m.enable_skips_wake {
                for &c in &targets {
                    n.user_ring_wake(c);
                }
            }
            n.act = 9;
            out.push(step("act:user_ring_wake of the enabled twins", n));
        }
        _ => {}
    }
}

fn enable_host(cfg: &Cfg, n: &mut World, c: usize) {
    let e = &mut n.ch[c].env;
    e.host_off = false;
    if (e.hw_pending || cfg.hw_reload_on_enable) && e.log.len() > e.hw_get {
        e.loaded = true;
        e.latched = e.landed.max(if cfg.hw_db_races_stores {
            0
        } else {
            e.log.len()
        });
    }
    e.hw_pending = false;
}

/// End of a claimed turn's record work: the token release (`TokenWord::release`).
fn finish(cfg: &Cfg, n: &mut World) {
    let c = n.cur;
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
                n.pc = Pc::S(S_REQ);
            } else {
                ch.tok = Tok::Rung;
                n.moved = true;
                n.pc = Pc::Top0;
            }
        }
        Tok::Dead => n.pc = Pc::Top0,
        t => panic!("release in token state {t:?}"),
    }
}

/// `user_ring_arm`.
fn arm(n: &mut World, c: usize, need: Need) {
    let ch = &mut n.ch[c];
    ch.p_wait = ul::wait_of(need);
    ch.waiting_flag = ch.p_wait.is_some();
    match need {
        Need::Again => {
            ch.contended_run = 0;
            n.ring(c);
        }
        Need::Contended => ch.contended_run = ch.contended_run.saturating_add(1),
        Need::Idle => ch.contended_run = 0,
        Need::Marker { .. } => {
            ch.contended_run = 0;
            n.reach |= R_WAIT_MARKER;
        }
        Need::Popped { .. } => {
            ch.contended_run = 0;
            n.reach |= R_WAIT_POPPED;
        }
    }
}

/// The production `ChanPlane::user_ring_peek_one` (userpump.rs 2865-2982), transliterated (the decision functions are the
/// production ones: `look_needed`, `may_retry`, `Wait::satisfied`, `found`).
fn peek_one(cfg: &Cfg, n: &mut World, c: usize, why: ul::Peek) -> bool {
    let ch = &n.ch[c];
    if ch.canceled {
        return false;
    }
    let backstop = why == ul::Peek::Backstop;
    let demand = ch.pending();
    if !ul::look_needed(why, demand, ch.waiting_flag) {
        return false;
    }
    let retry_ok = ul::may_retry(u32::from(ch.contended_run), why);
    if ch.act_lock {
        // `record.try_lock()` failed: ring, the turn handles the contention itself
        if retry_ok {
            n.ring(c);
            return true;
        }
        return false;
    }
    if let Some(w) = ch.p_wait {
        let e = &ch.env;
        let get = (e.vis_popped as u32) & (e.priv_entries - 1);
        let reference = if e.vis_refs == 0 {
            0
        } else {
            MARK_TAG | (e.vis_refs as u32 & 0xffff)
        };
        let (satisfied, pops) = match ch.relay.core.fetch.peek(get) {
            Ok(popped) => (
                w.satisfied(ch.relay.core.marks.peek(reference), popped),
                popped > ch.relay.core.fetch.fetched(),
            ),
            Err(_) => (true, false),
        };
        if !satisfied && pops && matches!(w, Wait::Marker { .. }) && !cfg.m.no_pop_writeback_ring {
            n.ring(c);
            return true;
        }
        if satisfied && (retry_ok || !matches!(w, Wait::Contended)) && !cfg.m.peek_never_rings {
            if backstop {
                n.viol |= match ul::found(w) {
                    ul::Found::Work => V_BACKSTOP_FOUND_WORK,
                    ul::Found::Carried => V_BACKSTOP_CARRIED,
                    ul::Found::Contended => V_BACKSTOP_CONTENDED,
                };
            }
            n.ch[c].p_wait = None;
            n.ch[c].waiting_flag = false;
            n.ring(c);
            return true;
        }
        if why == ul::Peek::Event {
            n.reach |= R_EVENT_NO_PROGRESS;
        }
    }
    if demand && retry_ok {
        if backstop {
            n.viol |= V_BACKSTOP_CONTENDED;
        }
        n.ring(c);
        return true;
    }
    false
}

fn waiting_any(w: &World) -> bool {
    w.ch.iter()
        .any(|c| c.alive() && (c.waiting_flag || c.pending()))
}

fn worker_steps(cfg: &Cfg, w: &World, out: &mut Vec<Step>) {
    match w.pc {
        Pc::Top0 => {
            let mut n = w.clone();
            n.moved = false;
            n.spin_left = cfg.spin;
            n.pc = Pc::Top1;
            out.push(step("w:seen=seq", n));
        }
        Pc::Top1 => {
            let mut n = w.clone();
            if let Some(c) = (0..w.ch.len()).find(|&c| w.ch[c].tok == Tok::Rung) {
                n.ch[c].tok = Tok::Busy;
                n.cur = c;
                n.rounds = 0;
                n.pc = Pc::S(S_REQ);
                out.push(step(format!("w:claim c{c}"), n));
            } else if ul::idle(waiting_any(w), u32::from(w.spin_left)) == ul::Idle::Spin {
                n.pc = Pc::Spin;
                out.push(step("w:to spin", n));
            } else {
                n.pc = Pc::Top2;
                out.push(step("w:to try_park", n));
            }
        }
        Pc::Spin => {
            let mut n = w.clone();
            n.spin_left -= 1;
            if n.ch.iter().any(|c| c.env.ev) {
                for c in &mut n.ch {
                    c.env.ev = false;
                }
                if !cfg.m.event_deaf {
                    for c in 0..n.ch.len() {
                        peek_one(cfg, &mut n, c, ul::Peek::Event);
                    }
                }
                n.pc = Pc::Top0;
                out.push(step("w:spin: event", n));
                return;
            }
            let mut any = false;
            for c in 0..n.ch.len() {
                any |= peek_one(cfg, &mut n, c, ul::Peek::Spin);
            }
            n.pc = if any { Pc::Top0 } else { Pc::Top1 };
            out.push(step("w:spin peek", n));
        }
        Pc::Top2 => {
            let mut n = w.clone();
            if w.moved && !cfg.m.no_seq_check {
                n.reach |= R_PARK_REFUSED;
                n.pc = Pc::Top0;
                out.push(step("w:try_park refused (seq moved)", n));
            } else {
                n.parked = true;
                n.pc = Pc::Parked;
                out.push(step("w:parked", n));
            }
        }
        Pc::Parked => {
            if w.efd {
                let mut n = w.clone();
                n.efd = false;
                n.parked = false;
                n.pc = Pc::Top0;
                out.push(step("w:wake(efd)", n));
            }
            if w.ch.iter().any(|c| c.env.ev) {
                let mut n = w.clone();
                n.parked = false;
                n.pc = Pc::Event;
                out.push(step("w:wake(event)", n));
            }
            // the safety net: it can only matter while some relay waits or has demand, and a budget of 2 firings makes
            // the graph acyclic so that LIVENESS can be checked WITH the net (a firing that finds nothing owed is a no-op)
            if cfg.timers
                && cfg.backstop
                && !w.limit
                && w.timers_left > 0
                && waiting_any(w)
                && !w.ch.iter().any(|c| c.act_lock || c.pt_held)
                && !w.efd
                && !w.ch.iter().any(|c| c.env.ev)
            {
                let mut n = w.clone();
                n.timers_left -= 1;
                n.parked = false;
                for c in 0..n.ch.len() {
                    peek_one(cfg, &mut n, c, ul::Peek::Backstop);
                }
                n.pc = Pc::Top0;
                out.push(step("w:wake(backstop)", n));
            }
        }
        Pc::Event => {
            let mut n = w.clone();
            for c in &mut n.ch {
                c.env.ev = false;
            }
            if !cfg.m.event_deaf {
                for c in 0..n.ch.len() {
                    peek_one(cfg, &mut n, c, ul::Peek::Event);
                }
            }
            n.pc = Pc::Top0;
            out.push(step("w:handle event", n));
        }
        Pc::S(stage) => serve_step(cfg, w, stage, out),
    }
}

fn serve_step(cfg: &Cfg, w: &World, stage: u8, out: &mut Vec<Step>) {
    let c = w.cur;
    let mut n = w.clone();
    let ch = &w.ch[c];
    match stage {
        S_REQ => {
            if ch.canceled || ch.tok == Tok::Dead {
                finish(cfg, &mut n);
                out.push(step("w:serve: canceled", n));
            } else {
                n.ch[c].retry = true; // `record.enter()` -> `request()`
                n.pc = Pc::S(S_LOCK);
                out.push(step("w:serve: request()", n));
            }
        }
        S_LOCK => {
            if ch.act_lock {
                // `Some(false)`: `retry` stays set; `record.contended_note()`
                n.reach |= R_LOCK_BUSY;
                n.ch[c].contended_run = ch.contended_run.saturating_add(1);
                if cfg.m.busy_drops_demand {
                    n.ch[c].retry = false;
                }
                finish(cfg, &mut n);
                out.push(step("w:serve: record busy", n));
            } else {
                n.ch[c].serving = true;
                n.pc = Pc::S(S_CHK);
                out.push(step("w:serve: record locked", n));
            }
        }
        S_CHK => {
            if ch.pt_held {
                n.ch[c].serving = false;
                n.ch[c].contended_run = ch.contended_run.saturating_add(1);
                n.reach |= R_LOCK_BUSY;
                finish(cfg, &mut n);
                out.push(step("w:serve: pt busy (retry stays set)", n));
            } else if ch.disabled {
                // `if t.stopped || t.disabled { record.retry.store(false); return Some(false) }`
                n.ch[c].serving = false;
                n.ch[c].retry = false;
                n.reach |= R_DISABLED_SERVE;
                finish(cfg, &mut n);
                out.push(step("w:serve: twin disabled, demand dropped", n));
            } else {
                n.pc = Pc::S(S_SWAP);
                out.push(step("w:serve: pt checked", n));
            }
        }
        S_SWAP => {
            n.ch[c].retry = false; // `record.retry.swap(false)`
            n.ch[c].env.observed = false;
            n.ch[c].env.cache_hit = false;
            n.ch[c].relay.claim();
            n.pc = Pc::S(S_RUN);
            out.push(step("w:turn: claim", n));
        }
        S_RUN => {
            if ch.env.db_pending {
                // publish_guarded: `rm.doorbell(token)` AFTER the pt lock was released
                let m = &mut n.ch[c].env;
                m.db_pending = false;
                if m.host_off {
                    if cfg.hw_drops_db_when_off {
                        n.reach |= R_DOORBELL_LOST;
                    } else {
                        m.hw_pending = true;
                    }
                    n.reach |= R_PUBLISH_ON_OFF_CHANNEL;
                } else {
                    if cfg.readback_before_doorbell {
                        m.landed = m.log.len();
                    }
                    m.loaded = true;
                    m.latched = m.landed.max(if cfg.hw_db_races_stores {
                        0
                    } else {
                        m.log.len()
                    });
                }
                if n.ch[c].relay.idle() {
                    end_turn(cfg, &mut n, false);
                }
                out.push(step("w:private doorbell", n));
                return;
            }
            let publishing = matches!(
                ch.relay.core.phase,
                Phase::PubPointer | Phase::PubNop | Phase::Gate | Phase::Put
            );
            if ch.disabled && publishing {
                // publish_guarded: `if t.stopped || t.disabled { return Err(Failure::Retry) }`
                n.ch[c].relay.core.phase = Phase::Idle;
                n.ch[c].relay.core.again = false;
                n.reach |= R_ABORTED_TURN;
                end_turn(cfg, &mut n, true);
                out.push(step("w:turn aborted: twin disabled (Retry)", n));
                return;
            }
            let before = ch.relay.core.phase;
            {
                let m = &mut n.ch[c];
                m.env.busy = m.pt_held;
                m.relay.step(&mut m.env);
            }
            if n.ch[c].relay.life.mode() == Mode::Pointer {
                n.reach |= R_POINTER;
            }
            if n.ch[c].env.carried {
                n.reach |= R_CARRIED;
            }
            if n.ch[c].relay.sync.waiting() {
                n.reach |= R_GATE;
            }
            if n.ch[c].relay.idle() && !n.ch[c].env.db_pending {
                end_turn(cfg, &mut n, false);
            } else if n.ch[c].relay.idle() {
                // the doorbell step comes first; the turn ends after it
            }
            out.push(step(format!("w:turn c{c} {before:?}"), n));
        }
        _ => unreachable!(),
    }
}

fn end_turn(cfg: &Cfg, n: &mut World, retry: bool) {
    let c = n.cur;
    {
        let ch = &mut n.ch[c];
        ch.serving = false;
        if !ch.env.observed && !retry {
            n.reach |= R_SKIPPED_OBS;
        }
        if ch.env.cache_hit {
            n.reach |= R_CACHE_HIT;
        }
    }
    if n.ch[c].relay.core.dead {
        let ch = &mut n.ch[c];
        ch.canceled = true;
        ch.retry = false;
        ch.waiting_flag = false;
        ch.p_wait = None;
        finish(cfg, n);
        return;
    }
    let need = if retry {
        Need::Contended
    } else if n.ch[c].relay.core.again {
        Need::Again
    } else {
        n.ch[c].relay.core.need
    };
    arm(n, c, need);
    finish(cfg, n);
}

/// The PBDMA's view: GP_GET prefetch (measured: it runs ahead of processing), the autosync of a loaded channel.
fn normalize(cfg: &Cfg, w: &mut World) {
    for c in 0..w.ch.len() {
        let sem = w.sem;
        let e = &mut w.ch[c].env;
        if e.host_off {
            continue;
        }
        if !cfg.hw_db_races_stores {
            e.landed = e.log.len();
        }
        if e.loaded {
            e.latched = e.landed;
        }
        let to = e.latched.min(e.hw_get.saturating_add(cfg.pop_depth));
        if e.popped < to {
            e.popped = to;
            if !cfg.lag {
                e.vis_popped = e.popped;
            }
        }
        let blocked = e.hw_wait.is_some_and(|s| sem & (1 << s) == 0);
        if e.loaded && e.hw_get == e.latched && !blocked && !e.db_pending {
            e.loaded = false;
        }
        if cfg.oracle {
            w.ch[c].env.guest_get = (w.ch[c].env.popped as u32) % cfg.src_entries;
        }
    }
}

fn successors(cfg: &Cfg, w: &World) -> Vec<Step> {
    let mut out = Vec::new();
    guest_steps(cfg, w, &mut out);
    hw_steps(cfg, w, &mut out);
    env_steps(cfg, w, &mut out);
    if !cfg.oracle {
        worker_steps(cfg, w, &mut out);
    }
    for s in &mut out {
        normalize(cfg, &mut s.w);
    }
    let me = w.key();
    out.retain(|s| s.w.key() != me);
    out
}

// ------------------------------------------------------------------------------------------------
// invariants and goals

fn invariant(cfg: &Cfg, w: &World) -> u32 {
    let mut v = w.viol;
    for (c, ch) in w.ch.iter().enumerate() {
        v |= ch.env.viol;
        let exp = &cfg.chans[c].expected;
        if ch.env.log.len() > 3 * exp.len() + 4 * cfg.priv_entries as usize + 8 {
            v |= V_UNBOUNDED;
        }
    }
    if w.pc == Pc::Parked
        && !cfg.oracle
        && w.ch
            .iter()
            .any(|c| c.tok == Tok::Rung || c.tok == Tok::BusyRung)
        && !w.efd
    {
        v |= V_PARKED_WITH_WORK;
    }
    v
}

fn goal_failure(cfg: &Cfg, w: &World) -> Option<String> {
    if !cfg.oracle && !matches!(w.pc, Pc::Parked) {
        return Some(format!("worker stuck at {:?}", w.pc));
    }
    for (c, ch) in w.ch.iter().enumerate() {
        let spec = &cfg.chans[c];
        let e = &ch.env;
        if e.latched < e.log.len() && !e.host_off && !e.loaded {
            return Some(format!(
                "P10 chan {c}: the PBDMA is parked with {} published private entr{} it was never told about (GP_PUT {} vs latched {}; GET {}): the covered work is unpopped forever. relay cursor {} put {} wait {:?} retry {} disabled {}",
                e.log.len() - e.latched,
                if e.log.len() - e.latched == 1 {
                    "y"
                } else {
                    "ies"
                },
                e.log.len(),
                e.latched,
                e.popped,
                ch.relay.core.cursor,
                e.put,
                ch.p_wait,
                ch.retry,
                ch.disabled
            ));
        }
        if e.executed.len() < spec.must_exec && !ch.canceled {
            return Some(format!(
                "P6/P7/P8 chan {c}: the PBDMA executed {:?} but {} entries ({:?}) were owed; relay cursor {} relayed {} put {} wait {:?} waiting_flag {} retry {} tok {:?} disabled {} host_off {} hw_wait {:?} sem {:#b} log {:?}",
                e.executed,
                spec.must_exec,
                &spec.expected[..spec.must_exec.min(spec.expected.len())],
                ch.relay.core.cursor,
                ch.relay.core.relayed,
                e.put,
                ch.p_wait,
                ch.waiting_flag,
                ch.retry,
                ch.tok,
                ch.disabled,
                e.host_off,
                e.hw_wait,
                w.sem,
                e.log
            ));
        }
        if ch.gpc < spec.script.len() && !spec.may_block {
            return Some(format!(
                "chan {c}: the guest is stuck at op {} ({:?}) -- ring full: the relay never wrote GP_GET back? (guest_get {} put_local {})",
                ch.gpc,
                short(&spec.script[ch.gpc]),
                e.guest_get,
                ch.put_local
            ));
        }
    }
    None
}

#[derive(Debug)]
struct Explored {
    states: usize,
    labels: HashSet<String>,
    reach: u32,
}
#[derive(Debug)]
struct Counterexample {
    why: String,
    trace: Vec<String>,
    states: usize,
}

fn key(w: &World) -> u128 {
    w.key()
}

/// Run the prelude (the leading ops of every script) with a deterministic schedule until quiescent.
fn settle(cfg: &Cfg) -> World {
    let mut w = World::new(cfg);
    if cfg.chans.iter().all(|c| c.prelude == 0) {
        return w;
    }
    w.limit = true;
    loop {
        let s = successors(cfg, &w);
        let Some(first) = s.into_iter().next() else {
            break;
        };
        w = first.w;
    }
    assert_eq!(
        invariant(cfg, &w),
        0,
        "the prelude itself violated something"
    );
    w.limit = false;
    w
}

/// Reach bits that are properties of a state (transition-set bits live in `World::reach`).
fn state_reach(w: &World) -> u32 {
    let mut r = 0;
    for ch in &w.ch {
        let e = &ch.env;
        if ch.relay.life.mode() == Mode::Pointer {
            r |= R_POINTER;
        }
        if ch.relay.sync.waiting() {
            r |= R_GATE;
        }
        if e.hw_wait.is_some_and(|s| w.sem & (1 << s) == 0) && e.hw_get < e.popped.min(e.log.len())
        {
            r |= R_SEM_BLOCKED;
        }
        if matches!(ch.p_wait, Some(Wait::Marker { .. })) {
            r |= R_WAIT_MARKER;
        }
        if matches!(ch.p_wait, Some(Wait::Popped { .. })) {
            r |= R_WAIT_POPPED;
        }
        if e.carried {
            r |= R_CARRIED;
        }
    }
    r
}

fn explore(cfg: &Cfg, allow: u32) -> Result<Explored, Counterexample> {
    let start = settle(cfg);
    let mut seen: HashMap<u128, (u128, String)> = HashMap::new();
    let mut q: VecDeque<World> = VecDeque::new();
    seen.insert(key(&start), (0, String::new()));
    q.push_back(start);
    let mut states = 0usize;
    let mut labels = HashSet::new();
    let mut reach = 0u32;
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
            states < cfg.max_states,
            "state space exceeded {}: shrink the scenario",
            cfg.max_states
        );
        if states.is_multiple_of(5_000) && std::env::var("KF_V2_PROGRESS").is_ok() {
            eprintln!(
                "  .. {states} states, queue {}, seen {}",
                q.len(),
                seen.len()
            );
        }
        reach |= w.reach | state_reach(&w);
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
            reach |= s.w.reach;
            let nk = key(&s.w);
            if let std::collections::hash_map::Entry::Vacant(e) = seen.entry(nk) {
                labels.insert(s.label.clone());
                e.insert((k, s.label));
                q.push_back(s.w);
            }
        }
    }
    Ok(Explored {
        states,
        labels,
        reach,
    })
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

fn holds(name: &str, cfg: &Cfg, allow: u32) -> Explored {
    let r = explore(cfg, allow);
    if r.is_err() && !cfg.oracle {
        // attribution (P7): does the same scenario complete without the relay?
        let o = explore(&cfg.clone().oracle(), 0);
        eprintln!(
            "{} ; ORACLE (no relay): {}",
            report(name, &r),
            report("oracle", &o).lines().next().unwrap_or("")
        );
    }
    assert!(r.is_ok(), "{}", report(name, &r));
    r.unwrap()
}

fn finds(name: &str, cfg: &Cfg, allow: u32, needles: &[&str]) -> Counterexample {
    match explore(cfg, allow) {
        Err(e) if needles.iter().any(|n| e.why.contains(n)) => e,
        other => panic!(
            "{name}: expected a violation containing one of {needles:?}, got:\n{}",
            report(name, &other)
        ),
    }
}

// ------------------------------------------------------------------------------------------------
// scenario helpers

fn plain(id: u8) -> Ge {
    ge(id, Kind::Plain)
}
fn setobj() -> Ge {
    ge(0, Kind::SetObj)
}
/// The three leading ops that take a channel into Pointer mode (run greedily as the prelude).
fn to_pointer() -> Vec<Op> {
    vec![Op::Many(vec![setobj(), plain(63)]), Op::SetPut, Op::Ring]
}
fn pointer_chan(rest: Vec<Op>, ids: Vec<u8>) -> ChanSpec {
    let mut script = to_pointer();
    script.extend(rest);
    let mut expected = vec![0, 63];
    expected.extend(ids);
    ChanSpec::new(script, expected).prelude(3)
}
fn batch(ids: &[u8]) -> Vec<Op> {
    // the pushes are one guest step: the relay reads nothing before `SetPut`, so interleaving them one by one adds
    // states, not behaviours (the ring-full check is made for the whole batch)
    vec![
        Op::Many(ids.iter().map(|&i| plain(i)).collect()),
        Op::SetPut,
        Op::Ring,
    ]
}

// ------------------------------------------------------------------------------------------------
// tests: calibration

#[test]
fn calibration_the_prelude_reaches_pointer_mode_and_one_channel_is_live() {
    let cfg = Cfg::new(vec![pointer_chan(batch(&[1, 2]), vec![1, 2])]);
    let s = settle(&cfg);
    assert_eq!(
        s.ch[0].relay.life.mode(),
        Mode::Pointer,
        "the prelude must end in Pointer mode"
    );
    let r = holds("calibration", &cfg, 0);
    eprintln!("calibration: {} states", r.states);
    assert!(r.reach & R_POINTER != 0);
}

fn submit(ids: &[u8]) -> Op {
    Op::Submit(ids.iter().map(|&i| plain(i)).collect())
}
fn submit_g(gs: Vec<Ge>) -> Op {
    Op::Submit(gs)
}

// ------------------------------------------------------------------------------------------------
// lazy observe + PUT cache with the production defaults; Pointer mode (P6, P9)

/// Production lazy defaults (`LazyConfig::ON`: PUT cache, observe interval 16) on a 32-entry guest ring, so that the
/// effective interval is really 16 (`min(16, E/2)`): 18 entries relayed in Pointer mode, one batch.
#[test]
fn p6_production_interval_16_on_a_32_entry_ring_is_live_with_the_safety_net_disabled() {
    let ids: Vec<u8> = (1..=18).collect();
    let gs: Vec<Ge> = ids.iter().map(|&i| plain(i)).collect();
    let spec = pointer_chan(vec![Op::Many(gs), Op::SetPut, Op::Ring], ids);
    let cfg = Cfg::new(vec![spec]).src(32).privr(128);
    let r = holds("lazy/interval16/E32", &cfg, 0);
    eprintln!("interval16: {} states reach {:#x}", r.states, r.reach);
    assert!(
        r.reach & R_POINTER != 0 && r.reach & R_SKIPPED_OBS != 0,
        "lazy observe skipped looks"
    );
    assert!(r.reach & R_CACHE_HIT != 0, "the PUT cache was used");
}

/// The same with an 8-entry guest ring (K = 4) and three batches whose rings interleave with the worker.
#[test]
fn p9_a_stale_put_cache_never_hides_a_covered_doorbell() {
    let mut rest = batch(&[1, 2, 3]);
    rest.extend(batch(&[4, 5]));
    rest.extend(batch(&[6]));
    let cfg = Cfg::new(vec![pointer_chan(rest, vec![1, 2, 3, 4, 5, 6])])
        .src(16)
        .privr(128);
    let r = holds("lazy/p9/three-batches", &cfg, 0);
    eprintln!("p9: {} states reach {:#x}", r.states, r.reach);
    assert!(r.reach & R_CACHE_HIT != 0, "a cached GP_PUT was used");
}

/// PUT ahead of the last doorbell is owed nothing, and a later SetPut + Ring covers it (the cache holds the older PUT).
#[test]
fn p9_put_written_before_the_ring_and_a_second_put_before_the_second_ring() {
    let rest = vec![
        Op::Push(plain(1)),
        Op::Push(plain(2)),
        Op::SetPut,
        Op::Push(plain(3)),
        Op::Ring,
        Op::SetPut,
        Op::Ring,
    ];
    let cfg = Cfg::new(vec![pointer_chan(rest, vec![1, 2, 3])])
        .src(16)
        .privr(128);
    holds("lazy/p9/put-ahead", &cfg, 0);
}

/// Lazy observe OFF and interval 2: the same property (the configurations are all live).
#[test]
fn p6_lazy_configurations_are_all_live() {
    let mut rest = batch(&[1, 2, 3]);
    rest.extend(batch(&[4, 5]));
    for lz in [
        LazyConfig::OFF,
        LazyConfig::ON.with_interval(2),
        LazyConfig::ON.with_put_cache(false),
    ] {
        let cfg = Cfg::new(vec![pointer_chan(rest.clone(), vec![1, 2, 3, 4, 5])])
            .src(8)
            .privr(128)
            .lazy(lz);
        holds(&format!("lazy/{lz:?}"), &cfg, 0);
    }
}

/// A guest ring of 8 slots lapped by 12 entries: the guest blocks on the relay's `GP_GET` write-back (D6).
#[test]
fn p6_a_lapped_small_guest_ring_is_written_back_with_lazy_observe_on() {
    let ids: Vec<u8> = (1..=10).collect();
    let mut rest = Vec::new();
    for chunk in ids.chunks(3) {
        rest.extend(batch(chunk));
    }
    let cfg = Cfg::new(vec![pointer_chan(rest, ids)])
        .src(8)
        .privr(128)
        .spin(0);
    let r = holds("lazy/lap", &cfg, 0);
    eprintln!("lap: {} states reach {:#x}", r.states, r.reach);
}

// ------------------------------------------------------------------------------------------------
// Pointer-mode marker accounting

/// Markers latched off (`NoPointerMarkers`): every wait is a `Popped` wait with no interrupt. Pops are visible at once
/// (no lag) and the worker looks after every turn, so the lapped ring is live with the spin; the model asserts WHICH
/// wake carries it (no event ever arrives: no marker exists).
#[test]
fn pointer_without_markers_is_carried_by_the_spin_only() {
    let ids: Vec<u8> = (1..=6).collect();
    let mut rest = Vec::new();
    for chunk in ids.chunks(3) {
        rest.extend(batch(chunk));
    }
    let cfg = Cfg {
        mode: LifecycleMode::NoPointerMarkers,
        ..Cfg::new(vec![pointer_chan(rest, ids)])
            .src(4)
            .privr(128)
            .spin(1)
    };
    let r = holds("pointer/no-markers/spin1", &cfg, 0);
    assert!(
        r.reach & R_WAIT_POPPED != 0,
        "a Popped wait (no interrupt) was reached"
    );
}

/// ... and with the spin at 0 and no safety net, a guest that needs the write-back DEADLOCKS: the Popped wait has no
/// wake source at all. DOCUMENTED (the backstop is the declared wake of a carried wait, `backstop_carried` counts it).
#[test]
fn pointer_without_markers_needs_the_spin_or_the_counted_backstop() {
    let ids: Vec<u8> = (1..=6).collect();
    let mut rest = Vec::new();
    for chunk in ids.chunks(3) {
        rest.extend(batch(chunk));
    }
    let cfg = Cfg {
        mode: LifecycleMode::NoPointerMarkers,
        ..Cfg::new(vec![pointer_chan(rest, ids)])
            .src(4)
            .privr(128)
            .spin(0)
    };
    finds("pointer/no-markers/spin0", &cfg, 0, &["DEADLOCK"]);
    // with the safety net armed it is covered and COUNTED
    let cfg = cfg.backstop();
    holds("pointer/no-markers/backstop", &cfg, V_BACKSTOP_CARRIED);
}

#[test]
fn pointer_without_markers_is_carried_by_the_spin_with_the_put_cache_off() {
    let ids: Vec<u8> = (1..=6).collect();
    let mut rest = Vec::new();
    for chunk in ids.chunks(3) {
        rest.extend(batch(chunk));
    }
    let cfg = Cfg {
        mode: LifecycleMode::NoPointerMarkers,
        ..Cfg::new(vec![pointer_chan(rest, ids)])
            .src(4)
            .spin(1)
            .lazy(LazyConfig::ON.with_put_cache(false))
    };
    holds("pointer/no-markers/spin1/nocache", &cfg, 0);
}

// ------------------------------------------------------------------------------------------------
// the act windows (P8) and the private doorbell

fn two_batches() -> Vec<ChanSpec> {
    // batch 1 keeps the push / SetPut / Ring races; batch 2 is one guest step (its doorbell may land anywhere in the act)
    let mut rest = batch(&[1]);
    rest.push(submit(&[2]));
    vec![pointer_chan(rest, vec![1, 2])]
}

#[test]
fn p8_a_doorbell_inside_the_disable_enable_window_is_relayed_after_the_enable() {
    let cfg = Cfg::new(two_batches()).act(Act::Channels(1));
    let r = holds("act/channels", &cfg, 0);
    eprintln!("act/channels: {} states reach {:#x}", r.states, r.reach);
    assert!(
        r.reach & R_DISABLED_SERVE != 0,
        "a serve met the disabled twin and dropped its demand"
    );
    assert!(r.reach & R_ABORTED_TURN != 0 || r.reach & R_PUBLISH_ON_OFF_CHANNEL != 0);
}

#[test]
fn p8_the_same_with_the_record_lock_held_by_an_act_around_the_enable() {
    let cfg = Cfg::new(vec![pointer_chan(batch(&[1]), vec![1])])
        .act(Act::Channels(1))
        .locks(1);
    holds("act/channels+lock", &cfg, 0);
}

/// A publish + private doorbell that land on a host-DISABLED channel (host verb returned, `v.disabled` not yet set):
/// benign hardware (the doorbell is latched pending and the enable schedules the channel) is live.
#[test]
fn p10_a_publish_on_a_host_disabled_channel_is_live_if_the_hardware_latches_the_doorbell() {
    for act in [Act::Channels(1), Act::Usermode(1)] {
        let cfg = Cfg::new(two_batches()).act(act);
        let r = holds(&format!("p10/benign/{act:?}"), &cfg, 0);
        assert!(
            r.reach & R_PUBLISH_ON_OFF_CHANNEL != 0,
            "{act:?}: a publish landed on a host-disabled channel"
        );
    }
}

/// ADVERSARIAL hardware (INFERRED, not measured: a doorbell to a disabled channel is dropped and the enable does not
/// re-read GP_PUT): the model leaves a covered private entry unpopped FOREVER, with the relay idle and nothing outstanding.
/// `disable_channels` and `disable_usermode` both have the window; neither re-rings the NATIVE doorbell after the enable.
#[test]
fn p10_adversarial_hardware_leaves_published_entries_unpopped_after_a_disable_window() {
    for act in [Act::Channels(1), Act::Usermode(1)] {
        let cfg = Cfg::new(two_batches()).act(act).adversarial_hw();
        let e = finds(&format!("p10/adversarial/{act:?}"), &cfg, 0, &["P10"]);
        eprintln!(
            "p10/adversarial/{act:?}: {}\n    {}",
            e.why,
            e.trace.join("\n    ")
        );
    }
}

/// The hardware assumption is the whole story: a re-read of GP_PUT at the enable makes both live again.
#[test]
fn p10_hardware_that_rereads_gp_put_on_enable_is_live_even_if_the_doorbell_was_dropped() {
    for act in [Act::Channels(1), Act::Usermode(1)] {
        let mut cfg = Cfg::new(two_batches()).act(act).adversarial_hw();
        cfg.hw_reload_on_enable = true;
        holds(&format!("p10/reload/{act:?}"), &cfg, 0);
    }
}

// ------------------------------------------------------------------------------------------------
// P7: cross-channel semaphore dependencies (the relay must never be the only thing between an acquire and its release)

/// Main + subcontext channel of one client: the main channel's segment acquires what the sub channel's segment
/// releases; the second main entry is SYNC_WAIT-flagged.
fn x_two() -> Vec<ChanSpec> {
    vec![
        pointer_chan(
            vec![submit_g(vec![plain(1).acq(1), plain(2).sync()])],
            vec![1, 2],
        ),
        pointer_chan(vec![submit_g(vec![plain(1)])], vec![1]).rel(1, 1),
    ]
}

#[test]
fn p7_main_and_sub_channel_of_one_client_complete_like_the_oracle() {
    let cfg = Cfg::new(x_two()).src(8).privr(128);
    let r = holds("p7/two", &cfg, 0);
    eprintln!("p7/two: {} states reach {:#x}", r.states, r.reach);
    assert!(
        r.reach & R_SEM_BLOCKED != 0,
        "the acquire really blocked the main channel"
    );
    holds("p7/two/oracle", &cfg.clone().oracle(), 0);
}

#[test]
fn p7_the_same_with_a_one_entry_pop_window_and_a_small_private_ring() {
    let cfg = Cfg::new(x_two()).src(8).privr(4).pop_depth(1);
    holds("p7/two/depth1", &cfg, 0);
}

/// Two clients A (Minecraft-like) and B (Heaven / compositor-like), each a main channel plus a subcontext channel:
/// A.main acquires what B.sub releases, B.main acquires what A.sub releases, and each main channel's second entry is
/// SYNC_WAIT-flagged. Hardware-acyclic (the oracle completes).
fn x_four() -> Vec<ChanSpec> {
    vec![
        // 0 = A.main
        pointer_chan(
            vec![submit_g(vec![plain(1).acq(1), plain(2).sync()])],
            vec![1, 2],
        ),
        // 1 = A.sub: releases sem 2 for B.main
        pointer_chan(vec![submit_g(vec![plain(1)])], vec![1]).rel(1, 2),
        // 2 = B.main
        pointer_chan(
            vec![submit_g(vec![plain(1).acq(2), plain(2).sync()])],
            vec![1, 2],
        ),
        // 3 = B.sub: releases sem 1 for A.main
        pointer_chan(vec![submit_g(vec![plain(1)])], vec![1]).rel(1, 1),
    ]
}

#[test]
fn p7_two_clients_each_with_a_main_and_a_subcontext_channel_complete_like_the_oracle() {
    let mut cfg = Cfg::new(x_four()).src(4).privr(128);
    cfg.max_states = 4_000_000;
    holds("p7/four/oracle", &cfg.clone().oracle(), 0);
    let r = holds("p7/four", &cfg, 0);
    eprintln!("p7/four: {} states reach {:#x}", r.states, r.reach);
    assert!(r.reach & R_SEM_BLOCKED != 0);
}

/// A relay parked on a wait while another channel's acquire depends on it: B.sub's relay is held by an act lock window
/// and the enable/disable act while A.main waits for B.sub's release.
#[test]
fn p7_a_dependency_on_a_channel_whose_relay_is_parked_by_an_act_window() {
    let cfg = Cfg::new(x_two()).src(8).privr(128).act(Act::Channels(2));
    holds("p7/parked/act", &cfg, 0);
    let cfg = Cfg::new(x_two()).src(8).privr(128).locks(1);
    holds("p7/parked/lock", &cfg, 0);
    let cfg = Cfg::new(x_two()).src(8).privr(128).act(Act::Usermode(2));
    holds("p7/parked/usermode", &cfg, 0);
}

/// Scan mode (no prelude): the SYNC_WAIT entries are GATED (barrier + marker) and the gate waits for an acquire that
/// the OTHER channel releases after its own gate. Hardware-acyclic; the relay holds the gated entries meanwhile.
fn x_gates() -> Vec<ChanSpec> {
    vec![
        // ch0: [1 acq(S1)] [2 sync]: the gate of entry 2 waits for the acquire of entry 1 (released by ch1 entry 2)
        ChanSpec::new(
            vec![submit_g(vec![plain(1).acq(1), plain(2).sync()])],
            vec![1, 2],
        ),
        // ch1: [1 acq(S0)] [2 sync, releases S1]: gated behind an acquire released by the CPU afterwards
        ChanSpec::new(
            vec![
                submit_g(vec![plain(1).acq(0), plain(2).sync()]),
                Op::Release(0),
            ],
            vec![1, 2],
        )
        .rel(2, 1),
    ]
}

#[test]
fn p7_gated_entries_in_scan_mode_that_release_each_other_complete_like_the_oracle() {
    let cfg = Cfg::new(x_gates()).src(8).privr(128);
    holds("p7/gates/oracle", &cfg.clone().oracle(), 0);
    let r = holds("p7/gates", &cfg, 0);
    eprintln!("p7/gates: {} states reach {:#x}", r.states, r.reach);
    assert!(
        r.reach & R_GATE != 0,
        "a SYNC_WAIT gate was draining while the other channel waited"
    );
}

/// The relay's single worker serves independent channels: a channel whose relay waits for a marker that depends on a
/// semaphore nobody releases (a guest that never signals) must not delay the OTHER channel (no head-of-line blocking).
#[test]
fn p7_a_stuck_channel_does_not_hold_up_its_neighbour() {
    let stuck = pointer_chan(vec![submit_g(vec![plain(1).acq(7), plain(2)])], vec![1, 2])
        .must(1)
        .may_block();
    let free = pointer_chan(
        vec![submit_g(vec![plain(1)]), submit_g(vec![plain(2)])],
        vec![1, 2],
    );
    let cfg = Cfg::new(vec![stuck, free]).src(8).privr(128);
    holds("p7/neighbour", &cfg, 0);
}

// ------------------------------------------------------------------------------------------------
// the explicit 2-entry stall: PUT - GET == 2 on the private ring, with the safety net disabled

/// A marker and a following entry stay unpopped behind a segment whose acquire is released only later by the other
/// channel, with a pop window of 1 (the PBDMA does not run ahead): the entries are popped once the release lands, in
/// every interleaving. The PBDMA is never left parked on them while the hardware is healthy.
#[test]
fn stall2_two_unpopped_private_entries_behind_an_acquire_are_popped_after_the_release() {
    let cfg = Cfg::new(x_two()).src(8).privr(8).pop_depth(1);
    let r = holds("stall2/healthy", &cfg, 0);
    assert!(r.reach & R_SEM_BLOCKED != 0);
}

/// ... and the same two entries are unpopped FOREVER when the doorbell of their publish was lost (adversarial hardware
/// plus a disable window): the minimal trace of a 2-entry PUT-GET stall with the relay idle and the host quiet.
#[test]
fn stall2_two_unpopped_private_entries_forever_need_a_lost_native_doorbell() {
    // one guest entry + its batch marker = exactly two private entries (PUT - GET == 2)
    let cfg = Cfg::new(vec![pointer_chan(vec![submit(&[1])], vec![1])])
        .src(8)
        .act(Act::Usermode(1))
        .adversarial_hw();
    let e = finds(
        "stall2/lost-doorbell",
        &cfg,
        0,
        &["parked with 2 published private entries"],
    );
    eprintln!("stall2: {}\n    {}", e.why, e.trace.join("\n    "));
    // the same with the channel-level act (flag set after the host verb) and a second client channel beside it
    let cfg = Cfg::new(x_two())
        .src(8)
        .act(Act::Usermode(1))
        .adversarial_hw();
    finds("stall2/lost-doorbell/two-channels", &cfg, 0, &["P10"]);
}

/// INFERRED hardware ordering, no disable act at all: the native doorbell (BAR0) overtakes the private GPFIFO / GP_PUT
/// stores (BAR1, video memory) because `publish_guarded` rings it without a read-back. An IDLE channel then latches the
/// old GP_PUT and the last entries (the batch's entry and its marker: PUT - GET == 2) stay unpopped until the NEXT
/// doorbell of that channel. Relay idle, host quiet, GPU idle: the freeze signature. Falsifier on hardware: add the
/// read-back (modelled by `readback`) and the freezes must vanish.
#[test]
fn p10_a_doorbell_that_overtakes_the_private_stores_strands_the_last_two_entries() {
    let cfg = Cfg::new(vec![pointer_chan(vec![submit(&[1])], vec![1])])
        .src(8)
        .db_races_stores();
    let e = finds(
        "p10/db-races-stores",
        &cfg,
        0,
        &["parked with 2 published private entries"],
    );
    eprintln!("db-races-stores: {}\n    {}", e.why, e.trace.join("\n    "));
}

#[test]
fn p10_the_race_strands_entries_on_each_channel_of_a_pair() {
    let cfg = Cfg::new(x_two()).src(8).db_races_stores();
    finds("p10/db-races-stores/pair", &cfg, 0, &["P10"]);
}

#[test]
fn p10_a_read_back_before_the_doorbell_closes_the_race() {
    let cfg = Cfg::new(vec![pointer_chan(vec![submit(&[1])], vec![1])])
        .src(8)
        .db_races_stores()
        .readback();
    holds("p10/readback", &cfg, 0);
    let cfg = Cfg::new(x_two()).src(8).db_races_stores().readback();
    holds("p10/readback/pair", &cfg, 0);
}

// ------------------------------------------------------------------------------------------------
// lock windows, GP_GET visibility lag, interrupt loss

#[test]
fn p6_an_act_holding_the_record_across_a_doorbell_is_resumed_by_its_release() {
    let cfg = Cfg::new(two_batches()).locks(1);
    let r = holds("locks", &cfg, 0);
    assert!(r.reach & R_LOCK_BUSY != 0 && r.reach & R_ACT_WAKE_NO_RING != 0);
}

#[test]
fn p6_a_pt_admission_window_is_only_covered_by_the_counted_backstop() {
    let mut cfg = Cfg::new(vec![pointer_chan(batch(&[1]), vec![1])]);
    cfg.pt_windows = 1;
    // nobody announces a pt release: with the spin exhausted and no safety net the doorbell can be stranded
    // (DOCUMENTED in V3_USER_RING_EVENT_LOOP_20261014 §3.1: Wait::Contended -> backstop_contended)
    let r = explore(&cfg, 0);
    eprintln!(
        "pt window, no backstop: {}",
        report("pt", &r).lines().next().unwrap_or("")
    );
    let cfg = cfg.backstop();
    holds("pt/backstop", &cfg, V_BACKSTOP_CONTENDED);
}

/// A pt window that opens INSIDE a turn: the `GP_GET` write-back answers `Busy`, the turn ends `Need::Contended` BEFORE it
/// read `GP_PUT`, and `retry` was already swapped to false -- only the recorded `Wait::Contended` (a look) can re-ring it.
/// The release of the lock is announced by nobody: the net is the declared wake (`backstop_contended`).
#[test]
fn p6_a_pt_window_inside_a_turn_leaves_only_a_contended_wait_and_the_net_finds_it() {
    let mut cfg = Cfg::new(vec![pointer_chan(batch(&[1]), vec![1])]).spin(0);
    cfg.pt_windows = 1;
    finds("pt-in-turn/no-net", &cfg, 0, &["DEADLOCK"]);
    holds(
        "pt-in-turn/net",
        &cfg.clone().backstop(),
        V_BACKSTOP_CONTENDED,
    );
}

#[test]
fn p6_a_pt_window_inside_a_turn_across_two_batches_is_found_by_the_net() {
    let mut cfg = Cfg::new(two_batches()).spin(0);
    cfg.pt_windows = 1;
    holds(
        "pt-in-turn/net/two-batches",
        &cfg.backstop(),
        V_BACKSTOP_CONTENDED,
    );
}

/// The safety net finds the stranded demand of a pt-admission window (counted `backstop_contended`) in a second shape.
#[test]
fn p6_a_pt_window_across_two_batches_is_covered_by_the_counted_backstop() {
    let mut cfg = Cfg::new(two_batches());
    cfg.pt_windows = 1;
    holds(
        "pt/backstop/two-batches",
        &cfg.backstop(),
        V_BACKSTOP_CONTENDED,
    );
}

#[test]
fn p6_get_visibility_lag_after_the_interrupt_is_covered_by_the_spin_or_the_counted_backstop() {
    let cfg = Cfg::new(vec![pointer_chan(batch(&[1, 2]), vec![1, 2])])
        .lag()
        .src(4)
        .privr(128);
    let r = explore(&cfg, 0);
    eprintln!(
        "lag, no backstop: {}",
        report("lag", &r).lines().next().unwrap_or("")
    );
    holds(
        "lag/backstop",
        &cfg.backstop(),
        V_BACKSTOP_FOUND_WORK | V_BACKSTOP_CARRIED,
    );
}

#[test]
fn p6_a_lost_marker_interrupt_is_found_only_by_the_counted_backstop() {
    let cfg = lapped(4, 6).no_irq();
    finds("noirq/spin0", &cfg, 0, &["DEADLOCK"]);
    let small = Cfg::new(vec![pointer_chan(batch(&[1, 2]), vec![1, 2])])
        .no_irq()
        .src(4)
        .spin(0);
    holds(
        "noirq/backstop",
        &small.backstop(),
        V_BACKSTOP_FOUND_WORK | V_BACKSTOP_CARRIED,
    );
}

// ------------------------------------------------------------------------------------------------
// drift pins: the transliterated decisions are still the production ones

const USERPUMP: &str = include_str!("../../kf-qemu/src/userpump.rs");
const CHAN: &str = include_str!("../../kf-qemu/src/chan.rs");

fn after<'a>(hay: &'a str, needle: &str) -> &'a str {
    hay.split(needle)
        .nth(1)
        .unwrap_or_else(|| panic!("pin: {needle:?} is gone"))
}

#[test]
fn pin_user_ring_serve_order_is_the_models() {
    let serve = after(USERPUMP, "pub(super) fn user_ring_serve(&self, ht: u32)");
    let serve = serve.split("fn user_ring_arm").next().unwrap();
    let order = [
        "record.enter()",
        "self.pt.try_lock()",
        "t.stopped || t.disabled",
        "record.retry.store(false, Ordering::Release)",
        "record.retry.swap(false, Ordering::AcqRel)",
        "pump.turn(self, &mut r)",
        "self.user_ring_arm(&record, &mut r)",
    ];
    let mut at = 0;
    for o in order {
        let i = serve[at..]
            .find(o)
            .unwrap_or_else(|| panic!("pin: {o:?} missing or out of order in user_ring_serve"));
        at += i + o.len();
    }
}

#[test]
fn pin_the_arm_mapping_and_the_peek_decision_are_the_models() {
    let arm = after(USERPUMP, "fn user_ring_arm(&self")
        .split("pub(crate) fn user_ring_waiting")
        .next()
        .unwrap();
    for o in [
        "wait_of(need)",
        "record.set_waiting(p.wait.is_some())",
        "self.plane.ring_internal(idx)",
        "record.contended_note()",
        "record.contended_clear()",
    ] {
        assert!(arm.contains(o), "pin: {o:?} left user_ring_arm");
    }
    let peek = after(USERPUMP, "fn user_ring_peek_one(")
        .split("pub(crate) fn user_ring_poll_due")
        .next()
        .unwrap();
    for o in [
        "look_needed(why, demand, record.is_waiting())",
        "may_retry(record.contended_run(), why)",
        "w.satisfied(p.core.marks.peek(reference), popped)",
        "pops = popped > p.core.fetch.fetched()",
        "matches!(w, kf_chan::user_loop::Wait::Marker { .. })",
        "if demand && retry_ok",
    ] {
        assert!(peek.contains(o), "pin: {o:?} left user_ring_peek_one");
    }
}

#[test]
fn pin_the_disable_enable_acts_are_the_models() {
    let dc = after(CHAN, "fn disable_channels(\n")
        .split("fn preempt_group")
        .next()
        .unwrap();
    let host = dc.find("me.rm\n                    .disable_channels(&hosts, disable, only_scheduling, rewind)").expect("pin: host verb");
    let flag = dc
        .find("v.disabled = disable;")
        .expect("pin: pt.disabled assignment");
    let wake = dc
        .find("me.user_ring_wake(chan.token)")
        .expect("pin: user_ring_wake of the enabled twins");
    assert!(
        host < flag && flag < wake,
        "pin: the act order is host verb, then `disabled`, then user_ring_wake"
    );
    let um = after(CHAN, "fn disable_usermode(&self")
        .split("fn disable_channels(")
        .next()
        .unwrap();
    assert!(
        !um.contains("user_ring_wake") && !um.contains(".disabled ="),
        "pin: disable_usermode gained a flag or a wake: update the model's Act::Usermode"
    );
}

#[test]
fn pin_publish_rings_the_native_doorbell_after_the_pt_lock_is_released() {
    let p = after(USERPUMP, "fn publish_guarded(")
        .split("fn snapshot_with")
        .next()
        .unwrap();
    let admit = p
        .find(".map_err(|e| access_error(e, \"USER_RING_USERD_ADMISSION\"))??;")
        .expect("pin: admit end");
    let bell = p.find("me.rm\n        .doorbell(").expect("pin: doorbell");
    assert!(
        admit < bell,
        "pin: the doorbell is rung after the admission (pt guard) ended"
    );
    assert!(
        p.contains("if t.stopped || t.disabled {\n                return Err(Failure::Retry);"),
        "pin: publish refuses a disabled twin"
    );
}

// ------------------------------------------------------------------------------------------------
// the checker has teeth: harness mutants of the transliterated parts, each found by at least two scenarios

fn lapped(entries: u32, n: u8) -> Cfg {
    lapped_c(entries, n, 3)
}

fn lapped_c(entries: u32, n: u8, chunk_len: usize) -> Cfg {
    let ids: Vec<u8> = (1..=n).collect();
    let mut rest = Vec::new();
    for chunk in ids.chunks(chunk_len) {
        rest.extend(batch(chunk));
    }
    Cfg::new(vec![pointer_chan(rest, ids)])
        .src(entries)
        .privr(128)
        .spin(0)
}

macro_rules! mutant_tests {
    ($( $name:ident : $mutate:expr, $needle:expr, $cfg:expr; )*) => {
        $(
            #[test]
            fn $name() {
                let cfg: Cfg = ($cfg).mutant($mutate);
                // the unmutated configuration holds (so the mutant, not the scenario, is what is found)
                holds(concat!(stringify!($name), "/control"), &($cfg), 0);
                finds(stringify!($name), &cfg, 0, &$needle);
            }
        )*
    };
}

mutant_tests! {
    mutant_enable_skips_wake_is_found_by_the_channels_act:
        |m| m.enable_skips_wake = true, ["DEADLOCK"],
        Cfg::new(two_batches()).act(Act::Channels(1)).spin(0);
    mutant_enable_skips_wake_is_found_by_the_dependency_on_a_disabled_twin:
        |m| m.enable_skips_wake = true, ["DEADLOCK"],
        Cfg::new(x_two()).src(8).privr(128).act(Act::Channels(2)).spin(0);
    mutant_unlock_no_wake_is_found_with_one_lock_window:
        |m| m.unlock_no_wake = true, ["DEADLOCK"],
        Cfg::new(two_batches()).locks(1).spin(0);
    mutant_unlock_no_wake_is_found_on_the_dependent_pair:
        |m| m.unlock_no_wake = true, ["DEADLOCK"],
        Cfg::new(x_two()).src(8).privr(128).locks(1).spin(0);
    mutant_event_deaf_worker_is_found_on_a_lapped_ring_of_4:
        |m| m.event_deaf = true, ["DEADLOCK"],
        lapped(4, 6);
    mutant_event_deaf_worker_is_found_with_the_put_cache_off:
        |m| m.event_deaf = true, ["DEADLOCK"],
        lapped(4, 6).lazy(LazyConfig::ON.with_put_cache(false));
    mutant_peek_never_rings_is_found_on_a_lapped_ring_of_4:
        |m| m.peek_never_rings = true, ["DEADLOCK"],
        lapped(4, 6);
    mutant_peek_never_rings_is_found_with_the_put_cache_off:
        |m| m.peek_never_rings = true, ["DEADLOCK"],
        lapped(4, 6).lazy(LazyConfig::ON.with_put_cache(false));
    mutant_no_seq_check_is_found_on_two_batches:
        |m| m.no_seq_check = true, ["DEADLOCK", "PARKED_WITH_WORK"],
        Cfg::new(two_batches());
    mutant_no_seq_check_is_found_on_the_dependent_pair:
        |m| m.no_seq_check = true, ["DEADLOCK", "PARKED_WITH_WORK"],
        Cfg::new(x_two()).src(8).privr(128);
    mutant_drop_busy_rung_is_found_on_two_batches:
        |m| m.drop_busy_rung = true, ["DEADLOCK", "PARKED_WITH_WORK"],
        Cfg::new(two_batches());
    mutant_drop_busy_rung_is_found_on_the_lapped_ring:
        |m| m.drop_busy_rung = true, ["DEADLOCK", "PARKED_WITH_WORK"],
        lapped_c(8, 10, 5);
}

/// The reach test: the races the scenarios claim to cover are actually reached.
#[test]
fn the_exploration_reaches_every_race_it_claims_to_cover() {
    let mut reach = 0u32;
    let mut labels: HashSet<String> = HashSet::new();
    for cfg in [
        Cfg::new(two_batches()).act(Act::Channels(1)),
        Cfg::new(vec![pointer_chan(batch(&[1]), vec![1])]).locks(1),
        Cfg::new(x_two()).src(8).privr(128),
        Cfg::new(x_gates()).src(8).privr(128),
        lapped(4, 6),
    ] {
        let r = explore(&cfg, 0).expect("holds");
        reach |= r.reach;
        labels.extend(r.labels);
    }
    for (bit, name) in [
        (R_POINTER, "Pointer mode"),
        (R_SKIPPED_OBS, "a skipped observation (rule B)"),
        (R_CACHE_HIT, "a PUT cache hit (rule A)"),
        (R_DISABLED_SERVE, "a serve that met a disabled twin"),
        (R_LOCK_BUSY, "a busy record"),
        (R_WAIT_MARKER, "a Marker wait"),
        (R_SEM_BLOCKED, "a PBDMA blocked on a semaphore"),
        (R_PARK_REFUSED, "try_park refused (seq moved)"),
        (R_ACT_WAKE_NO_RING, "an act wake that found the record held"),
        (R_GATE, "a draining SYNC_WAIT gate"),
        (
            R_PUBLISH_ON_OFF_CHANNEL,
            "a publish + doorbell on a host-disabled channel",
        ),
    ] {
        assert!(reach & bit != 0, "never reached: {name}");
    }
    for needle in [
        "w:wake(efd)",
        "w:wake(event)",
        "w:spin peek",
        "w:private doorbell",
        "act:user_ring_wake of the enabled twins",
    ] {
        assert!(labels.contains(needle), "never reached: {needle:?}");
    }
}

/// EXPECTED RED (2026-10-20). P10 under the inferred hardware ordering (`hw_db_races_stores`, see the model tests above)
/// needs the pump to read a private word back between its video-memory stores and `rm.doorbell`. Today `publish_guarded`
/// (and `Storage::publish_put`) have release fences only: the entry and its marker (PUT - GET == 2) can be stranded until the
/// channel's next doorbell. The hardware ordering is NOT measured; this test pins the gap, the owner decides (docs section 9.2).
#[test]
#[ignore = "OPEN (EXPECTED RED since the previous commit): owner decision on a read-back before the private doorbell; docs/design/V3_USER_RING_EVENT_LOOP_20261014.md section 9.2"]
fn p10_publish_reads_back_before_the_native_doorbell() {
    let p = after(USERPUMP, "fn publish_guarded(")
        .split("fn snapshot_with")
        .next()
        .unwrap();
    let admit = p
        .find(".map_err(|e| access_error(e, \"USER_RING_USERD_ADMISSION\"))??;")
        .expect("pin: admit end");
    let bell = p.find("me.rm\n        .doorbell(").expect("pin: doorbell");
    let between = &p[admit..bell];
    assert!(
        between.contains("gpu_get()")
            || between.contains("read_back")
            || between.contains("load_u32"),
        "publish_guarded rings the native doorbell with no read-back of the private stores before it (P10)"
    );
}
